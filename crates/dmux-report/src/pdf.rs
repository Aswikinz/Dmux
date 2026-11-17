//! The PDF summary (PLAN.md 4.7, 1, 2.4 and 5).
//!
//! Generated entirely in Rust with `genpdf` — no browser printing.  Liberation
//! Sans (regular and bold) is embedded from the bundled files under `fonts/`;
//! their permissive licence is `fonts/OFL-1.1.txt`.
//!
//! Page 1 carries the overall verdict (a word, never colour alone), the coverage
//! line, every ignored item with its reason, failures first, the run identity
//! (run ID, tool version, mapping hash, results SHA-256), the "what a PASS does
//! not prove" paragraph, the md5 threat-model sentence (decision 10), the
//! tamper-evident-not-proof sentence, and a sign-off block.  Per-table sections
//! follow (verdict, level, counts, differences summary).

use std::path::Path;

use dmux_engine::results::RunResults;
use genpdf::elements::{LinearLayout, Paragraph};
use genpdf::fonts::{FontData, FontFamily};
use genpdf::style;
use genpdf::{Document, Element, SimplePageDecorator};

use crate::layout::{self, Layout, ReportOptions};
use crate::verify::HookEvidence;

const FONT_REGULAR: &[u8] = include_bytes!("../fonts/LiberationSans-Regular.ttf");
const FONT_BOLD: &[u8] = include_bytes!("../fonts/LiberationSans-Bold.ttf");

/// Errors rendering the PDF.
#[derive(Debug, thiserror::Error)]
pub enum PdfError {
    #[error("cannot write PDF: {0}")]
    Io(#[from] std::io::Error),
    #[error("cannot embed the bundled font: {0}")]
    Font(String),
    #[error("PDF render failed: {0}")]
    Render(String),
}

/// Write the PDF summary for verified results to `path`.
///
/// `results_sha256` is the verified digest of the results file (PLAN 2.4); it is
/// printed on page 1 so a later edit is detectable.  `hooks`, when present,
/// contributes the custom-check section and (via [`layout::build`]) folds into
/// the displayed verdict.
pub fn write_summary(
    results: &RunResults,
    hooks: Option<&HookEvidence>,
    results_sha256: &str,
    options: &ReportOptions,
    path: &Path,
) -> Result<(), PdfError> {
    let layout = layout::build(results, hooks, options);
    write_summary_layout(&layout, results_sha256, path)
}

/// Write a pre-built layout to disk (used by the shared `render_all` path so the
/// displayed-verdict fold happens once, in the caller).
pub fn write_summary_layout(
    layout: &Layout,
    results_sha256: &str,
    path: &Path,
) -> Result<(), PdfError> {
    let bytes = render_bytes(layout, results_sha256)?;
    std::fs::write(path, bytes)?;
    Ok(())
}

/// Render a [`Layout`] to in-memory PDF bytes (the test entry point).
///
/// The bytes have the Info-dictionary creation/modification dates neutralised so
/// a fixed results file renders byte-identically (PLAN 4.7 deterministic output;
/// the sign-off carries the run ID and SHA-256, not a wall-clock time).
pub fn render_bytes(layout: &Layout, results_sha256: &str) -> Result<Vec<u8>, PdfError> {
    let doc = build_document(layout, results_sha256)?;
    let mut buf = Vec::new();
    doc.render(&mut buf)
        .map_err(|e| PdfError::Render(e.to_string()))?;
    Ok(neutralise_volatile(buf))
}

/// Make the rendered bytes deterministic by overwriting the wall-clock values
/// printpdf writes — the Info dictionary's creation/modification dates and the
/// trailer's random document ID — with fixed same-length values.  Lengths are
/// preserved so the cross-reference offsets stay valid.  The date is not a
/// report claim; the run ID and results SHA-256 are the audit identity.
fn neutralise_volatile(mut pdf: Vec<u8>) -> Vec<u8> {
    // Fixed 23-byte date ("D:" + 14 digits + "+00'00'"), matching printpdf's form.
    let fixed_date: &[u8] = b"D:19700101000000+00'00'";
    for key in [b"/CreationDate(".as_slice(), b"/ModDate(".as_slice()] {
        overwrite_value(&mut pdf, key, fixed_date);
    }
    // The /ID holds two random 32-hex-char strings; overwrite both with zeros,
    // keeping the surrounding parentheses intact.
    let mut id_pos = 0;
    while let Some(id) = find_bytes(&pdf, b"/ID", id_pos) {
        id_pos = id + b"/ID".len();
        let mut pos = id;
        for _ in 0..2 {
            let Some(open) = find_bytes(&pdf, b"(", pos) else {
                break;
            };
            let Some(close) = find_bytes(&pdf, b")", open + 1) else {
                break;
            };
            for b in pdf.iter_mut().take(close).skip(open + 1) {
                *b = b'0';
            }
            pos = close + 1;
        }
    }
    pdf
}

/// Overwrite the `(...)` value that follows each occurrence of `key` (e.g.
/// `/CreationDate(`) with `fixed`, preserving the value's byte length by
/// padding/truncating — the cross-reference offsets must not move.
fn overwrite_value(pdf: &mut [u8], key: &[u8], fixed: &[u8]) {
    let mut start = 0;
    while let Some(p) = find_bytes(pdf, key, start) {
        let value_start = p + key.len();
        let Some(close) = find_bytes(pdf, b")", value_start) else {
            break;
        };
        let len = close - value_start;
        for (i, b) in pdf.iter_mut().take(close).skip(value_start).enumerate() {
            *b = if i < fixed.len() { fixed[i] } else { b'0' };
        }
        debug_assert_eq!(len, fixed.len(), "date length changed");
        start = close + 1;
    }
}

fn find_bytes(hay: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    hay[from..]
        .windows(needle.len())
        .position(|w| w == needle)
        .map(|p| from + p)
}

/// Build the `genpdf` document from a layout.
pub fn build_document(layout: &Layout, results_sha256: &str) -> Result<Document, PdfError> {
    let regular =
        FontData::new(FONT_REGULAR.to_vec(), None).map_err(|e| PdfError::Font(e.to_string()))?;
    let bold =
        FontData::new(FONT_BOLD.to_vec(), None).map_err(|e| PdfError::Font(e.to_string()))?;
    let sans = FontFamily {
        regular: regular.clone(),
        bold,
        italic: regular.clone(),
        bold_italic: regular,
    };

    let mut doc = Document::new(sans);
    doc.set_title("Dmux migration verification report");
    doc.set_font_size(11);
    // Minimal conformance drops the ICC profile and XMP metadata (which carries a
    // random document UUID and a wall-clock creation date), keeping the summary
    // deterministic for a given results file — which is what lets tests hash it
    // (PLAN 4.7 / the deterministic-output requirement).  It also cuts ~3 KiB.
    doc.set_minimal_conformance();
    let mut decorator = SimplePageDecorator::new();
    decorator.set_margins(18);
    doc.set_page_decorator(decorator);

    let mut body = LinearLayout::vertical();

    // --- Title ---
    body.push(
        Paragraph::new("Dmux migration verification report")
            .styled(style::Style::new().with_font_size(20).bold()),
    );

    // --- Page 1: overall verdict, a word + text, never colour alone ---
    body.push(heading("Overall verdict"));
    body.push(
        Paragraph::new(layout.verdict_word.clone())
            .styled(style::Style::new().with_font_size(16).bold()),
    );

    // Coverage line.
    body.push(heading("Coverage"));
    body.push(plain(&layout.coverage_line));

    // Failures first.
    body.push(heading("Failures and findings"));
    let failures: Vec<&layout::TableSummary> = layout
        .tables
        .iter()
        .filter(|t| t.verdict_word != "PASS")
        .collect();
    if failures.is_empty() {
        body.push(plain("None."));
    } else {
        for t in &failures {
            body.push(plain(&format!(
                "{} — {} ({}): {}",
                t.table,
                t.verdict_word,
                t.level,
                if t.reasons.is_empty() {
                    "no reason recorded".to_string()
                } else {
                    t.reasons.join("; ")
                }
            )));
        }
    }

    // Every ignored item with its reason.
    body.push(heading("Ignored items"));
    if layout.ignored_items.is_empty() {
        body.push(plain("None."));
    } else {
        for i in &layout.ignored_items {
            body.push(plain(&format!("{} — {}", i.path, i.reason)));
        }
    }

    // Structure (inventory) findings, after the mapping is applied.
    body.push(heading("Structure findings"));
    if layout.structure_findings.is_empty() {
        body.push(plain("None."));
    } else {
        for f in &layout.structure_findings {
            body.push(plain(&format!(
                "{} — {}: {}",
                f.object, f.verdict, f.message
            )));
        }
    }

    // Suggested matches for unmapped objects (a person must tick them).
    if !layout.suggestions.is_empty() {
        body.push(heading("Suggested matches"));
        for s in &layout.suggestions {
            body.push(plain(&format!(
                "{} -> {} (confidence {})",
                s.path, s.candidate, s.confidence
            )));
        }
    }

    // Run identity.
    body.push(heading("Run identity"));
    body.push(labeled("Run ID", &layout.run_id));
    body.push(labeled("Tool version", &layout.tool_version));
    body.push(labeled("Mapping hash", &layout.mapping_hash));
    body.push(labeled("Results SHA-256", results_sha256));
    body.push(labeled("Results schema version", &layout.results_version));
    if !layout.definition_hash.is_empty() {
        body.push(labeled("Definition hash", &layout.definition_hash));
    }
    if let Some(wls) = &layout.wal_start_lsn {
        body.push(labeled("Write-ahead log start", wls));
    }
    if let Some(wle) = &layout.wal_end_lsn {
        body.push(labeled("Write-ahead log end", wle));
    }

    // Mandatory prose.
    body.push(heading("What a PASS does not prove"));
    body.push(plain(layout::WHAT_PASS_DOES_NOT_PROVE));
    body.push(heading("Limit of the row hashes"));
    body.push(plain(layout::MD5_THREAT_MODEL));
    body.push(heading("Trust and audit"));
    body.push(plain(layout::TAMPER_EVIDENT_NOT_PROOF));

    // Per-table sections.
    body.push(heading("Per-table results"));
    for t in &layout.tables {
        body.push(
            Paragraph::new(format!("{} — {}", t.table, t.verdict_word))
                .styled(style::Style::new().with_font_size(12).bold()),
        );
        body.push(labeled("Level", &t.level));
        body.push(labeled("Old rows", t.old_rows.as_deref().unwrap_or("-")));
        body.push(labeled("New rows", t.new_rows.as_deref().unwrap_or("-")));
        body.push(labeled("Differing buckets", &t.differing_buckets));
        body.push(labeled("Missing buckets", &t.missing_buckets));
        body.push(labeled("Extra buckets", &t.extra_buckets));
        body.push(labeled("Missing rows", &t.missing_rows));
        body.push(labeled("Extra rows", &t.extra_rows));
        body.push(labeled("Changed rows", &t.changed_rows));
        if !t.keys.is_empty() {
            body.push(labeled("Keys / covered columns", &t.keys.join(", ")));
        }
        let reasons = if t.reasons.is_empty() {
            "none recorded".to_string()
        } else {
            t.reasons.join("; ")
        };
        body.push(labeled("Reasons", &reasons));
        if !t.changed_columns.is_empty() {
            body.push(labeled("Changed columns", &t.changed_columns.join(", ")));
        }
        for word in &t.plain_words {
            body.push(labeled("Note", word));
        }
        for ct in &t.control_totals {
            body.push(labeled("Control total", ct));
        }
        for key in &t.keyed_rows {
            body.push(labeled("Key", key));
        }
    }

    // Hook custom checks (PLAN 4.9), marked as such, with each script's hash.
    if !layout.custom_checks.is_empty() || !layout.hook_scripts.is_empty() {
        body.push(heading("Custom checks (Python hooks)"));
        for c in &layout.custom_checks {
            let scope = c
                .table
                .as_deref()
                .map(|t| format!(" ({t})"))
                .unwrap_or_default();
            body.push(plain(&format!(
                "[{}] {}{}: {}",
                c.id, c.severity, scope, c.message
            )));
        }
        for w in &layout.hook_warnings {
            body.push(plain(&format!("hook warning: {w}")));
        }
        for s in &layout.hook_scripts {
            body.push(plain(&format!(
                "script {} — SHA-256 {}",
                s.script, s.sha256
            )));
        }
    }

    // Sign-off block.
    body.push(heading("Sign-off"));
    for line in layout::sign_off_block() {
        body.push(plain(line));
    }

    doc.push(body);
    Ok(doc)
}

fn heading(title: &str) -> genpdf::elements::StyledElement<Paragraph> {
    Paragraph::new(title).styled(style::Style::new().with_font_size(14).bold())
}

fn plain(text: &str) -> Paragraph {
    Paragraph::new(text)
}

fn labeled(label: &str, value: &str) -> Paragraph {
    Paragraph::new(format!("{label}: {value}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::build;
    use dmux_engine::results::{
        finalise, ChangedRowExport, CheckLevel, RunResults, TableDetail, TableResult, Verdict,
        RESULTS_VERSION,
    };
    use std::collections::BTreeMap;

    const SENTINEL: &str = "SECRET-ROW-VALUE-999";

    fn fixture() -> RunResults {
        let mut tables = BTreeMap::new();
        tables.insert(
            "demo_bank.accounts".into(),
            TableResult {
                table: "demo_bank.accounts".into(),
                level: CheckLevel::FullValues,
                verdict: Verdict::Fail,
                old_rows: Some(100),
                new_rows: Some(99),
                columns_covered: Some(vec!["id".into()]),
                differing_buckets: Some(1),
                missing_buckets: Some(0),
                extra_buckets: Some(0),
                missing_rows: Some(1),
                extra_rows: Some(0),
                changed_rows: Some(0),
                reasons: vec!["row counts differ".into()],
            },
        );
        // Drill-down key sentinels live in the typed `details` field.
        let details = BTreeMap::from([(
            "demo_bank.accounts".into(),
            TableDetail {
                changed_columns: vec!["balance".into()],
                changed_rows: vec![ChangedRowExport {
                    key: Some(vec![Some(SENTINEL.into())]),
                    columns: vec!["balance".into()],
                }],
                missing_keys: None,
                extra_keys: None,
                control_totals: vec![],
                plain_words: vec!["a balance changed".into()],
            },
        )]);
        let r = RunResults {
            results_version: RESULTS_VERSION,
            run_id: "run-1".into(),
            tool_version: "0.1.0".into(),
            mapping_hash: "mh".into(),
            verdict: Verdict::Fail,
            coverage_line:
                "0 of 1 tables fully checked, 0 counts only, 0 structure only, 0 ignored".into(),
            fully_checked: 0,
            counts_only: 0,
            ignored: 0,
            tables,
            details,
            ..RunResults::default()
        };
        finalise(r)
    }

    /// Reconstruct the report's text from a rendered PDF: decode each font's
    /// ToUnicode CMap and every content stream's glyph codes.  One decode per
    /// font is returned; a sentinel must appear in none of them.
    fn extract_texts(pdf: &[u8]) -> Vec<String> {
        let streams = content_streams(pdf);
        to_unicode_maps(pdf)
            .into_iter()
            .map(|map| {
                let mut text = String::new();
                for stream in &streams {
                    text.push_str(&decode_content(stream, &map));
                    text.push('\n');
                }
                text
            })
            .collect()
    }

    /// Parse every font's ToUnicode CMap (`beginbfchar` blocks) into a
    /// glyph-id -> char map.  The streams are uncompressed (printpdf writes no
    /// FlateDecode), so the entries are plain text.
    fn to_unicode_maps(pdf: &[u8]) -> Vec<std::collections::HashMap<u16, char>> {
        let mut maps = Vec::new();
        // Each font's ToUnicode object: `/ToUnicode N 0 R`.
        let mut i = 0;
        while let Some(p) = find(pdf, b"/ToUnicode", i) {
            // Skip past `/ToUnicode` and any whitespace to the object number.
            let mut after = p + b"/ToUnicode".len();
            while after < pdf.len() && pdf[after].is_ascii_whitespace() {
                after += 1;
            }
            i = after;
            let num = read_number(&pdf[after..]).unwrap_or(u32::MAX);
            if num == u32::MAX {
                continue;
            }
            let needle = format!("{} 0 obj", num);
            let (_obj, stream_bytes) = stream_for_obj(pdf, needle.as_bytes());
            let mut map = std::collections::HashMap::new();
            parse_bfchar(stream_bytes, &mut map);
            if !map.is_empty() {
                maps.push(map);
            }
        }
        maps
    }

    fn stream_for_obj<'a>(pdf: &'a [u8], needle: &[u8]) -> (usize, &'a [u8]) {
        match find(pdf, needle, 0) {
            None => (0, &[]),
            Some(start) => {
                let s = find(pdf, b"stream", start).unwrap_or(start);
                let body_start = s + b"stream".len();
                let body_start = skip_crlf(pdf, body_start);
                let e = find(pdf, b"endstream", body_start).unwrap_or(pdf.len());
                (start, &pdf[body_start..e])
            }
        }
    }

    fn content_streams(pdf: &[u8]) -> Vec<&[u8]> {
        let mut out = Vec::new();
        // Page content streams are referenced from the page dict as
        // `/Contents N 0 R`; read each referenced object's stream directly,
        // rather than scanning every `stream` (the font binaries would be
        // misread as content otherwise).
        let mut i = 0;
        while let Some(p) = find(pdf, b"/Contents", i) {
            let mut after = p + b"/Contents".len();
            while after < pdf.len() && pdf[after].is_ascii_whitespace() {
                after += 1;
            }
            i = after;
            let num = read_number(&pdf[after..]).unwrap_or(u32::MAX);
            if num == u32::MAX {
                continue;
            }
            let needle = format!("{} 0 obj", num);
            let (_obj, stream) = stream_for_obj(pdf, needle.as_bytes());
            if !stream.is_empty() {
                out.push(stream);
            }
        }
        out
    }

    fn parse_bfchar(stream: &[u8], map: &mut std::collections::HashMap<u16, char>) {
        let mut idx = 0;
        while let Some(p) = find(stream, b"beginbfchar", idx) {
            idx = p + 1;
            // The count precedes `beginbfchar` (e.g. "100 beginbfchar"): skip
            // the separating whitespace, then read the digits backwards.
            let mut k = p;
            while k > 0 && stream[k - 1].is_ascii_whitespace() {
                k -= 1;
            }
            let mut n = 0usize;
            let mut mult = 1usize;
            let mut have = false;
            while k > 0 && stream[k - 1].is_ascii_digit() {
                have = true;
                n += (stream[k - 1] - b'0') as usize * mult;
                mult *= 10;
                k -= 1;
            }
            if !have {
                continue;
            }
            // Skip to after `beginbfchar`, then read n `<gid> <uni>` lines.
            let mut pos = p + b"beginbfchar".len();
            for _ in 0..n {
                let Some(gid_hex) = angle_hex(stream, &mut pos) else {
                    break;
                };
                let Some(uni_hex) = angle_hex(stream, &mut pos) else {
                    break;
                };
                let gid = u16::from_str_radix(&gid_hex, 16).unwrap_or(0);
                let uni = u32::from_str_radix(&uni_hex, 16).unwrap_or(0);
                if let Some(c) = char::from_u32(uni) {
                    map.insert(gid, c);
                }
            }
        }
    }

    /// Read the next `<hex>` token at/after `pos`, advancing `pos` past it.
    fn angle_hex(buf: &[u8], pos: &mut usize) -> Option<String> {
        let open = find(buf, b"<", *pos)?;
        let close = find(buf, b">", open + 1)?;
        let inner = &buf[open + 1..close];
        let hex: String = String::from_utf8_lossy(inner).trim().to_string();
        if hex.is_empty() || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
            *pos = close + 1;
            return None;
        }
        *pos = close + 1;
        Some(hex)
    }

    /// Decode one content stream against one font's glyph map.
    fn decode_content(stream: &[u8], map: &std::collections::HashMap<u16, char>) -> String {
        let mut out = String::new();
        let mut i = 0;
        while i < stream.len() {
            let b = stream[i];
            if b == b'(' {
                // Literal string: read to the matching ')', unescaping nothing.
                let Some(close) = find(stream, b")", i + 1) else {
                    break;
                };
                out.push_str(&String::from_utf8_lossy(&stream[i + 1..close]));
                i = close + 1;
            } else if b == b'<' && i + 1 < stream.len() && stream[i + 1] != b'<' {
                // Hex glyph code `<XXXX>`.
                let Some(close) = find(stream, b">", i + 1) else {
                    break;
                };
                let hex = &stream[i + 1..close];
                if let Ok(gid) = u16::from_str_radix(String::from_utf8_lossy(hex).trim(), 16) {
                    if let Some(c) = map.get(&gid) {
                        out.push(*c);
                    }
                }
                i = close + 1;
            } else {
                i += 1;
            }
        }
        out
    }

    fn find(hay: &[u8], needle: &[u8], from: usize) -> Option<usize> {
        hay[from..]
            .windows(needle.len())
            .position(|w| w == needle)
            .map(|p| from + p)
    }

    fn skip_crlf(buf: &[u8], mut i: usize) -> usize {
        while i < buf.len() && (buf[i] == b'\r' || buf[i] == b'\n') {
            i += 1;
        }
        i
    }

    fn read_number(buf: &[u8]) -> Option<u32> {
        let end = buf.iter().position(|b| !b.is_ascii_digit())?;
        if end == 0 {
            return None;
        }
        String::from_utf8_lossy(&buf[..end]).parse().ok()
    }

    fn any_contains(texts: &[String], needle: &str) -> bool {
        texts.iter().any(|t| t.contains(needle))
    }

    #[test]
    fn pdf_omits_drilldown_keys_by_default_and_includes_them_when_ticked() {
        let r = fixture();
        let without = build(&r, None, &ReportOptions::default());
        let with = build(&r, None, &ReportOptions { include_keys: true });

        let t_without = extract_texts(&render_bytes(&without, "abc").unwrap());
        let t_with = extract_texts(&render_bytes(&with, "abc").unwrap());

        assert!(
            !any_contains(&t_without, SENTINEL),
            "drill-down key must be absent by default ({t_without:?})"
        );
        assert!(
            any_contains(&t_with, SENTINEL),
            "drill-down key present when ticked ({t_with:?})"
        );
    }

    #[test]
    fn pdf_never_prints_a_row_value_sentinel() {
        let r = fixture();
        let layout = build(&r, None, &ReportOptions::default());
        let texts = extract_texts(&render_bytes(&layout, "abc").unwrap());
        assert!(
            !any_contains(&texts, SENTINEL),
            "sentinel leaked into {texts:?}"
        );
    }

    #[test]
    fn pdf_strips_bidi_controls_from_rendered_text() {
        let mut r = fixture();
        r.tables.get_mut("demo_bank.accounts").unwrap().reasons =
            vec!["\u{202E}benign".to_string()];
        let r = finalise(r);
        let layout = build(&r, None, &ReportOptions::default());
        let texts = extract_texts(&render_bytes(&layout, "abc").unwrap());
        let joined = texts.concat();
        assert!(
            !joined.contains('\u{202E}'),
            "bidi control leaked: {joined:?}"
        );
        assert!(
            joined.contains("benign"),
            "reason still present after strip"
        );
    }

    #[test]
    fn pdf_carries_mandatory_prose_and_signoff() {
        let layout = build(&fixture(), None, &ReportOptions::default());
        let texts = extract_texts(&render_bytes(&layout, "abc").unwrap());
        assert!(
            any_contains(&texts, "What a PASS does not prove"),
            "{texts:?}"
        );
        assert!(any_contains(&texts, "md5"));
        assert!(any_contains(&texts, "tamper-evident, not tamper-proof"));
        assert!(any_contains(&texts, "Sign-off"));
        assert!(any_contains(&texts, "Signature"));
        assert!(any_contains(&texts, "Coverage"));
        assert!(any_contains(&texts, "FAIL"));
    }

    #[test]
    fn pdf_renders_hook_custom_checks_and_script_hashes() {
        use crate::verify::{HookEvidence, HookFinding, HookScript};
        let hooks = HookEvidence {
            custom_checks: vec![HookFinding {
                id: "c1".into(),
                severity: "fail".into(),
                message: "a custom check failed".into(),
                table: Some("demo_bank.accounts".into()),
            }],
            scripts: vec![HookScript {
                script: "hook.py".into(),
                sha256: "ab".repeat(32),
            }],
            warnings: vec!["hook warning".into()],
        };
        let layout = build(&fixture(), Some(&hooks), &ReportOptions::default());
        let texts = extract_texts(&render_bytes(&layout, "abc").unwrap());
        assert!(
            any_contains(&texts, "Custom checks (Python hooks)"),
            "{texts:?}"
        );
        assert!(any_contains(&texts, "a custom check failed"));
        assert!(any_contains(&texts, "hook.py"), "{texts:?}");
        let joined = texts.concat();
        assert!(joined.contains(&"ab".repeat(32)), "script hash present");
    }
}
