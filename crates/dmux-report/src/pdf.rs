//! The PDF summary (PLAN.md 4.7, 1, 2.4 and 5).
//!
//! Generated entirely in Rust with `printpdf` — no browser printing.  Liberation
//! Sans (regular and bold) is embedded from the bundled files under `fonts/`;
//! their permissive licence is `fonts/OFL-1.1.txt`.
//!
//! Page 1 carries the overall verdict (a word, never colour alone), the coverage
//! line, every failure, every ignored item with its reason, structure findings,
//! the run identity (run ID, tool version, mapping hash, results SHA-256,
//! definition hash), the "what a PASS does not prove" paragraph, the md5
//! threat-model sentence (decision 10), the hash-integrity sentence (audit M1 /
//! decision 23),
//! the hook custom checks with script hashes, and a sign-off block.  Per-table
//! sections follow (verdict, level, counts, differences summary).  Text wraps at
//! word boundaries and pages automatically.
//!
//! The output is byte-deterministic for a given results file: the Info
//! dictionary's wall-clock dates and the trailer's random document ID are
//! neutralised, so the golden/hash tests remain stable.

use std::path::Path;

use dmux_engine::results::RunResults;
use printpdf::{
    Color, Mm, Op, ParsedFont, PdfDocument, PdfFontHandle, PdfFontParseWarning, PdfSaveOptions, Pt,
    Rgb, TextItem,
};

use crate::layout::{self, Layout, ReportOptions};
use crate::verify::HookEvidence;

const FONT_REGULAR: &[u8] = include_bytes!("../fonts/LiberationSans-Regular.ttf");
const FONT_BOLD: &[u8] = include_bytes!("../fonts/LiberationSans-Bold.ttf");

/// A4 in millimetres.
const PAGE_W_MM: f32 = 210.0;
const PAGE_H_MM: f32 = 297.0;
/// Body text size and leading.
const BODY_PT: f32 = 10.0;
const LEADING_PT: f32 = 14.0;
const MARGIN_MM: f32 = 18.0;
/// Em-based char-width estimate for wrapping (Liberation Sans ≈ 0.5 em average).
const CHAR_W_FACTOR: f32 = 0.5;

/// Errors rendering the PDF.
#[derive(Debug, thiserror::Error)]
pub enum PdfError {
    #[error("cannot write PDF: {0}")]
    Io(#[from] std::io::Error),
    #[error("cannot parse the bundled font")]
    Font,
}

/// Write the PDF summary for verified results to `path`.
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

/// Write a pre-built layout to disk.
pub fn write_summary_layout(
    layout: &Layout,
    results_sha256: &str,
    path: &Path,
) -> Result<(), PdfError> {
    let bytes = render_bytes(layout, results_sha256)?;
    crate::write_private(path, &bytes)?;
    Ok(())
}

/// Render a [`Layout`] to in-memory PDF bytes (the test entry point).
pub fn render_bytes(layout: &Layout, results_sha256: &str) -> Result<Vec<u8>, PdfError> {
    let doc = build_document(layout, results_sha256)?;
    Ok(neutralise_volatile(save_bytes(doc)?))
}

fn save_bytes(doc: PdfDocument) -> Result<Vec<u8>, PdfError> {
    let bytes = doc.save(&PdfSaveOptions::default(), &mut Vec::new());
    if bytes.is_empty() {
        return Err(PdfError::Font);
    }
    Ok(bytes)
}

/// A styled line of text, reduced to (font-kind, text) for the flow renderer.
enum Run {
    Heading(String),
    Body(String),
    Bold(String),
    Title(String),
}

/// Build the printpdf document from a layout.
pub fn build_document(layout: &Layout, results_sha256: &str) -> Result<PdfDocument, PdfError> {
    let mut warnings_w = Vec::<PdfFontParseWarning>::new();
    let regular = ParsedFont::from_bytes(FONT_REGULAR, 0, &mut warnings_w).ok_or(PdfError::Font)?;
    let bold = ParsedFont::from_bytes(FONT_BOLD, 0, &mut warnings_w).ok_or(PdfError::Font)?;

    let mut doc = PdfDocument::new("Dmux migration verification report");
    // Register the fonts under deterministic IDs (FontId::new() is random, which
    // would leak into /BaseFont and every content stream and defeat the
    // byte-determinism the golden/hash tests rely on).  FontId is a public newtype
    // and `resources.fonts.map` is public, so we insert directly.
    let regular_id = printpdf::FontId("FREGULAR".to_string());
    let bold_id = printpdf::FontId("FBOLD".to_string());
    doc.resources
        .fonts
        .map
        .insert(regular_id.clone(), printpdf::PdfFont::new(regular));
    doc.resources
        .fonts
        .map
        .insert(bold_id.clone(), printpdf::PdfFont::new(bold));

    let mut runs: Vec<Run> = Vec::new();
    build_runs(layout, results_sha256, &mut runs);

    let pages = flow(runs, &regular_id, &bold_id);
    doc.with_pages(pages);
    Ok(doc)
}

/// Lay the runs out into wrapped, paginated pages.
fn flow(
    runs: Vec<Run>,
    regular: &printpdf::FontId,
    bold: &printpdf::FontId,
) -> Vec<printpdf::PdfPage> {
    let page_w = Mm(PAGE_W_MM);
    let page_h = Mm(PAGE_H_MM);
    let text_w_mm = PAGE_W_MM - 2.0 * MARGIN_MM;
    // y tracks the top of the current text line, measured downward from the TOP
    // of the page.  printpdf's Point origin is the bottom-left, so the cursor's
    // y-from-bottom is PAGE_H_MM - y.
    let usable_h = PAGE_H_MM - 2.0 * MARGIN_MM;
    let pt_to_mm = |pt: f32| pt * 0.352_778;

    let mut pages: Vec<printpdf::PdfPage> = Vec::new();
    let mut ops: Vec<Op> = Vec::new();
    let mut y = MARGIN_MM; // distance from top

    for run in runs {
        let (font_kind, content) = match run {
            Run::Heading(s) => (1u8, s),
            Run::Body(s) => (0u8, s),
            Run::Bold(s) => (1u8, s),
            Run::Title(s) => (2u8, s),
        };
        let font_size = match font_kind {
            2 => 20.0,
            1 => 13.0,
            _ => BODY_PT,
        };
        let leading_pt = match font_kind {
            2 => 26.0,
            1 => 17.0,
            _ => LEADING_PT,
        };
        let leading_mm = pt_to_mm(leading_pt);
        let lines = wrap(&content, text_w_mm, font_size);
        let use_bold = font_kind == 1 || font_kind == 2;
        let font_handle = if use_bold {
            PdfFontHandle::External(bold.clone())
        } else {
            PdfFontHandle::External(regular.clone())
        };

        for line in lines {
            // Paginate before a line that would fall below the bottom margin.
            if y + leading_mm > MARGIN_MM + usable_h && !ops.is_empty() {
                pages.push(printpdf::PdfPage::new(
                    page_w,
                    page_h,
                    std::mem::take(&mut ops),
                ));
                y = MARGIN_MM;
            }
            ops.push(Op::StartTextSection);
            ops.push(Op::SetTextCursor {
                pos: printpdf::Point::new(Mm(MARGIN_MM), Mm(PAGE_H_MM - y)),
            });
            ops.push(Op::SetFont {
                font: font_handle.clone(),
                size: Pt(font_size),
            });
            ops.push(Op::SetLineHeight { lh: Pt(leading_pt) });
            ops.push(Op::SetFillColor {
                col: Color::Rgb(Rgb {
                    r: 0.0,
                    g: 0.0,
                    b: 0.0,
                    icc_profile: None,
                }),
            });
            ops.push(Op::ShowText {
                items: vec![TextItem::Text(line)],
            });
            ops.push(Op::EndTextSection);
            y += leading_mm;
        }

        // A small extra gap after headings.
        if font_kind == 1 {
            y += 2.0;
        }
    }

    if !ops.is_empty() {
        pages.push(printpdf::PdfPage::new(page_w, page_h, ops));
    }

    pages
}

/// Word-wrap text to `max_w_mm` at `font_size`, breaking on spaces.
fn wrap(text: &str, max_w_mm: f32, font_size: f32) -> Vec<String> {
    let char_w_mm = font_size * CHAR_W_FACTOR * 0.352778; // pt → mm
    let max_chars = ((max_w_mm / char_w_mm).floor() as usize).max(1);
    let mut out = Vec::new();
    for hard in text.split('\n') {
        if hard.is_empty() {
            out.push(String::new());
            continue;
        }
        let mut line = String::new();
        for word in hard.split_whitespace() {
            if line.is_empty() {
                line = word.to_string();
            } else if line.chars().count() + 1 + word.chars().count() <= max_chars {
                line.push(' ');
                line.push_str(word);
            } else {
                out.push(std::mem::take(&mut line));
                line = word.to_string();
            }
            // A single over-long word still overflows; hard-break it.
            while line.chars().count() > max_chars {
                let mut split_at = max_chars;
                while split_at > 0 && !line.is_char_boundary(split_at) {
                    split_at -= 1;
                }
                let (head, tail) = line.split_at(split_at);
                out.push(head.to_string());
                line = tail.to_string();
            }
        }
        if !line.is_empty() {
            out.push(line);
        }
    }
    out
}

/// Build the ordered list of runs (the report content).
fn build_runs(layout: &Layout, results_sha256: &str, runs: &mut Vec<Run>) {
    runs.push(Run::Title("Dmux migration verification report".to_string()));

    // Page 1: overall verdict, a word + text, never colour alone.
    runs.push(Run::Heading("Overall verdict".to_string()));
    runs.push(Run::Bold(layout::verdict_display(
        &layout.verdict_word,
        layout.options_applied,
    )));

    runs.push(Run::Heading("Coverage".to_string()));
    runs.push(Run::Body(layout.coverage_line.clone()));

    if !layout.options_applied_lines.is_empty() {
        runs.push(Run::Heading("Options applied".to_string()));
        for line in &layout.options_applied_lines {
            runs.push(Run::Body(line.clone()));
        }
    }

    runs.push(Run::Heading("Failures and findings".to_string()));
    let failures: Vec<&layout::TableSummary> = layout
        .tables
        .iter()
        .filter(|t| t.verdict_word != "PASS")
        .collect();
    if failures.is_empty() {
        runs.push(Run::Body("None.".to_string()));
    } else {
        for t in &failures {
            runs.push(Run::Body(format!(
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

    runs.push(Run::Heading("Ignored items".to_string()));
    if layout.ignored_items.is_empty() {
        runs.push(Run::Body("None.".to_string()));
    } else {
        for i in &layout.ignored_items {
            runs.push(Run::Body(format!("{} — {}", i.path, i.reason)));
        }
    }

    runs.push(Run::Heading("Structure findings".to_string()));
    if layout.structure_findings.is_empty() {
        runs.push(Run::Body("None.".to_string()));
    } else {
        for f in &layout.structure_findings {
            runs.push(Run::Body(format!(
                "{} — {}: {}",
                f.object, f.verdict, f.message
            )));
        }
    }

    if !layout.suggestions.is_empty() {
        runs.push(Run::Heading("Suggested matches".to_string()));
        for s in &layout.suggestions {
            runs.push(Run::Body(format!(
                "{} -> {} (confidence {})",
                s.path, s.candidate, s.confidence
            )));
        }
    }

    runs.push(Run::Heading("Run identity".to_string()));
    runs.push(labeled_run("Run ID", &layout.run_id));
    runs.push(labeled_run("Tool version", &layout.tool_version));
    runs.push(labeled_run("Mapping hash", &layout.mapping_hash));
    runs.push(labeled_run("Results SHA-256", results_sha256));
    runs.push(labeled_run(
        "Results schema version",
        &layout.results_version,
    ));
    if !layout.definition_hash.is_empty() {
        runs.push(labeled_run("Definition hash", &layout.definition_hash));
    }
    if let Some(old) = &layout.capture_sha256_old {
        runs.push(labeled_run("Old capture SHA-256", old));
    }
    if let Some(new) = &layout.capture_sha256_new {
        runs.push(labeled_run("New capture SHA-256", new));
    }
    if let Some(wls) = &layout.wal_start_lsn {
        runs.push(labeled_run("Write-ahead log start", wls));
    }
    if let Some(wle) = &layout.wal_end_lsn {
        runs.push(labeled_run("Write-ahead log end", wle));
    }

    if !layout.connections.is_empty() {
        runs.push(Run::Heading("Connections".to_string()));
        for line in &layout.connections {
            runs.push(Run::Body(line.clone()));
        }
    }

    runs.push(Run::Heading("What a PASS does not prove".to_string()));
    runs.push(Run::Body(layout::WHAT_PASS_DOES_NOT_PROVE.to_string()));
    runs.push(Run::Heading("Limit of the row hashes".to_string()));
    runs.push(Run::Body(layout::MD5_THREAT_MODEL.to_string()));
    runs.push(Run::Heading("Trust and audit".to_string()));
    runs.push(Run::Body(layout::TAMPER_EVIDENT_NOT_PROOF.to_string()));

    runs.push(Run::Heading("Per-table results".to_string()));
    for t in &layout.tables {
        runs.push(Run::Bold(format!("{} — {}", t.table, t.verdict_word)));
        runs.push(labeled_run("Level", &t.level));
        runs.push(labeled_run(
            "Old rows",
            t.old_rows.as_deref().unwrap_or("-"),
        ));
        runs.push(labeled_run(
            "New rows",
            t.new_rows.as_deref().unwrap_or("-"),
        ));
        runs.push(labeled_run("Differing buckets", &t.differing_buckets));
        runs.push(labeled_run("Missing buckets", &t.missing_buckets));
        runs.push(labeled_run("Extra buckets", &t.extra_buckets));
        runs.push(labeled_run("Missing rows", &t.missing_rows));
        runs.push(labeled_run("Extra rows", &t.extra_rows));
        runs.push(labeled_run("Changed rows", &t.changed_rows));
        if !t.keys.is_empty() {
            runs.push(labeled_run("Keys / covered columns", &t.keys.join(", ")));
        }
        let reasons = if t.reasons.is_empty() {
            "none recorded".to_string()
        } else {
            t.reasons.join("; ")
        };
        runs.push(labeled_run("Reasons", &reasons));
        if !t.changed_columns.is_empty() {
            runs.push(labeled_run(
                "Changed columns",
                &t.changed_columns.join(", "),
            ));
        }
        for word in &t.plain_words {
            runs.push(labeled_run("Note", word));
        }
        for note in &t.column_notes {
            runs.push(labeled_run("Column note", note));
        }
        for ct in &t.control_totals {
            runs.push(labeled_run("Control total", ct));
        }
        for key in &t.keyed_rows {
            runs.push(labeled_run("Key", key));
        }
    }

    // Hook custom checks, marked as such, with each script's hash.
    if !layout.custom_checks.is_empty() || !layout.hook_scripts.is_empty() {
        runs.push(Run::Heading("Custom checks (Python hooks)".to_string()));
        for c in &layout.custom_checks {
            let scope = c
                .table
                .as_deref()
                .map(|t| format!(" ({t})"))
                .unwrap_or_default();
            runs.push(Run::Body(format!(
                "[{}] {}{}: {}",
                c.id, c.severity, scope, c.message
            )));
        }
        for w in &layout.hook_warnings {
            runs.push(Run::Body(format!("hook warning: {w}")));
        }
        for s in &layout.hook_scripts {
            runs.push(Run::Body(format!(
                "script {} — SHA-256 {}",
                s.script, s.sha256
            )));
        }
    }

    runs.push(Run::Heading("Sign-off".to_string()));
    for line in layout::sign_off_block() {
        runs.push(Run::Body(line.to_string()));
    }
}

fn labeled_run(label: &str, value: &str) -> Run {
    Run::Body(format!("{label}: {value}"))
}

/// Make the rendered bytes deterministic by overwriting the wall-clock values
/// printpdf writes — the Info dictionary's creation/modification dates and the
/// trailer's random document ID — with fixed same-length values.
fn neutralise_volatile(mut pdf: Vec<u8>) -> Vec<u8> {
    let fixed_date: &[u8] = b"D:19700101000000+00'00'";
    for key in [b"/CreationDate(".as_slice(), b"/ModDate(".as_slice()] {
        overwrite_value(&mut pdf, key, fixed_date);
    }
    // The /ID holds two random 32-char literal strings; overwrite with zeros.
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

fn overwrite_value(pdf: &mut [u8], key: &[u8], fixed: &[u8]) {
    let mut start = 0;
    while let Some(p) = find_bytes(pdf, key, start) {
        let value_start = p + key.len();
        let Some(close) = find_bytes(pdf, b")", value_start) else {
            break;
        };
        for (i, b) in pdf.iter_mut().take(close).skip(value_start).enumerate() {
            *b = if i < fixed.len() { fixed[i] } else { b'0' };
        }
        start = close + 1;
    }
}

fn find_bytes(hay: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    hay[from..]
        .windows(needle.len())
        .position(|w| w == needle)
        .map(|p| from + p)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::build;
    use dmux_engine::results::{
        finalise, ChangedRowExport, CheckLevel, RunResults, TableDetail, TableResult, Verdict,
        RESULTS_VERSION,
    };
    use printpdf::PdfParseOptions;
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
                column_notes: vec![],
            },
        );
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

    /// Read all the text back out of a rendered PDF using printpdf's own parser,
    /// so the assertions match exactly what a real reader (and pdftotext) would
    /// recover from the ToUnicode CMaps.
    fn extract_texts(pdf: &[u8]) -> Vec<String> {
        let mut warnings = Vec::new();
        let doc = PdfDocument::parse(pdf, &PdfParseOptions::default(), &mut warnings)
            .expect("rendered PDF must parse");
        doc.extract_text()
            .into_iter()
            .map(|page| page.join("\n"))
            .collect()
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
        assert!(any_contains(&texts, "These hashes detect corruption"));
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

    #[test]
    fn pdf_output_is_deterministic() {
        let layout = build(&fixture(), None, &ReportOptions::default());
        let a = render_bytes(&layout, "abc").unwrap();
        let b = render_bytes(&layout, "abc").unwrap();
        assert_eq!(a, b, "PDF must be byte-deterministic");
    }

    #[test]
    fn pdf_shows_pass_with_options_and_the_options_list() {
        use dmux_engine::results::ColumnNote;
        let mut r = fixture();
        r.verdict = Verdict::Pass;
        for t in r.tables.values_mut() {
            t.verdict = Verdict::Pass;
            t.reasons.clear();
        }
        r.tables.get_mut("demo_bank.accounts").unwrap().column_notes = vec![ColumnNote {
            column: "balance".into(),
            kind: "option".into(),
            note: "round to 2 decimals".into(),
        }];
        r.options_applied = true;
        let r = finalise(r);
        let layout = build(&r, None, &ReportOptions::default());
        let texts = extract_texts(&render_bytes(&layout, "abc").unwrap());
        assert!(any_contains(&texts, "PASS (with options)"), "{texts:?}");
        assert!(any_contains(&texts, "Options applied"), "{texts:?}");
        assert!(any_contains(&texts, "round to 2 decimals"), "{texts:?}");
    }
}
