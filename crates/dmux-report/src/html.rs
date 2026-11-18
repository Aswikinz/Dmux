//! The interactive HTML report.
//!
//! One self-contained file a reviewer can open in any browser, offline: the
//! styles, the script and the Dmux logo are inline, and a Content-Security-
//! Policy forbids every network fetch.  It carries the same content as the PDF
//! summary (built from the same [`Layout`], so it is under the same rules: no
//! row values ever, keys only when the user ticked "include keys"), plus:
//!
//! - a search box, verdict and level filters and a "not passing only" switch
//!   over the per-table results, with a live "Showing N of M" count;
//! - sortable columns (click a header; numbers sort as numbers);
//! - expandable per-table detail, with "Expand all" and "Collapse all";
//! - a search and a verdict filter over the structure findings;
//! - light and dark themes from the system setting, and a print stylesheet.
//!
//! Every value is HTML-escaped, and the script only shows, hides and reorders
//! elements that are already in the page, so the report works without it:
//! with scripting off, everything is simply shown.  The output is deterministic
//! for a given layout.

use std::fmt::Write as _;
use std::path::Path;

use crate::layout::{self, Layout, TableSummary, Tone};
use dmux_engine::results::IssueKind;

const LOGO_PNG: &[u8] = include_bytes!("../assets/logo-128.png");

/// Errors writing the HTML report.
#[derive(Debug, thiserror::Error)]
pub enum HtmlError {
    #[error("cannot write HTML report: {0}")]
    Io(#[from] std::io::Error),
}

/// Write the HTML report for a pre-built layout (owner-only on Unix).
pub fn write_html_layout(
    layout: &Layout,
    results_sha256: &str,
    path: &Path,
) -> Result<(), HtmlError> {
    crate::write_private(path, render(layout, results_sha256).as_bytes())?;
    Ok(())
}

/// Escape text for HTML element content and quoted attribute values.
fn esc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

/// Standard base64 (RFC 4648) for the inline logo.
fn base64(bytes: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            chunk.get(1).copied().unwrap_or(0),
            chunk.get(2).copied().unwrap_or(0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        out.push(T[(n >> 18) as usize & 63] as char);
        out.push(T[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 {
            T[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            T[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

/// The CSS class for a verdict word's tone.
fn tone_class(word: &str) -> &'static str {
    match layout::verdict_tone(word) {
        Tone::Good => "good",
        Tone::Bad => "bad",
        Tone::Caution => "caution",
        Tone::Neutral => "neutral",
    }
}

/// A verdict pill: the word itself, coloured by tone (never colour alone).
fn pill(word: &str) -> String {
    format!(
        r#"<span class="pill {}">{}</span>"#,
        tone_class(word),
        esc(word)
    )
}

/// A count for sorting: digits only, or -1 for "-" / missing.
fn sort_num(v: &str) -> String {
    if !v.is_empty() && v.chars().all(|c| c.is_ascii_digit()) {
        v.to_string()
    } else {
        "-1".to_string()
    }
}

/// Render the whole report.
pub fn render(layout: &Layout, results_sha256: &str) -> String {
    let mut h = String::with_capacity(64 * 1024);
    let shown = layout::verdict_display(
        &layout.verdict_word,
        layout.options_applied,
        layout.options_recorded,
    );
    let title = format!("Dmux report: {} ({})", shown, layout.run_id);

    h.push_str("<!doctype html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n");
    h.push_str("<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n");
    // Nothing may be fetched: inline styles and script only, images from data:.
    h.push_str(
        "<meta http-equiv=\"Content-Security-Policy\" content=\"default-src 'none'; \
         img-src data:; style-src 'unsafe-inline'; script-src 'unsafe-inline'; \
         base-uri 'none'; form-action 'none'\">\n",
    );
    h.push_str("<meta name=\"referrer\" content=\"no-referrer\">\n");
    let _ = writeln!(h, "<title>{}</title>", esc(&title));
    let _ = writeln!(
        h,
        "<link rel=\"icon\" href=\"data:image/png;base64,{}\">",
        base64(LOGO_PNG)
    );
    h.push_str("<style>\n");
    h.push_str(CSS);
    h.push_str("</style>\n</head>\n<body>\n");

    header(&mut h, layout);
    h.push_str("<main>\n");
    hero(&mut h, layout, &shown);
    toolbar(&mut h, layout);
    record(&mut h, layout);
    failures(&mut h, layout);
    options(&mut h, layout);
    tables(&mut h, layout);
    structure(&mut h, layout);
    lists(&mut h, layout);
    custom_checks(&mut h, layout);
    integrity(&mut h, layout, results_sha256);
    limits(&mut h);
    sign_off(&mut h);
    h.push_str("</main>\n");

    let _ = write!(h, "<footer><span>Run {}</span>", esc(&layout.run_id));
    if let Some(g) = &layout.generated_at {
        let _ = write!(h, "<span>Generated {}</span>", esc(g));
    }
    h.push_str("<span>Dmux migration verification report</span></footer>\n");

    h.push_str("<script>\n");
    h.push_str(JS);
    h.push_str("</script>\n</body>\n</html>\n");
    h
}

fn header(h: &mut String, layout: &Layout) {
    let _ = write!(
        h,
        r#"<header class="top"><div class="brand"><img src="data:image/png;base64,{}" alt="Dmux logo" width="36" height="36"><div><strong>Dmux</strong><span>Migration verification report</span></div></div>"#,
        base64(LOGO_PNG)
    );
    let _ = write!(
        h,
        r#"<div class="meta"><span>Run {}</span>"#,
        esc(&layout.run_id)
    );
    if let Some(g) = &layout.generated_at {
        let _ = write!(h, "<span>Generated {}</span>", esc(g));
    }
    h.push_str(
        r#"<button type="button" class="btn js-only" id="print">Print or save as PDF</button></div></header>"#,
    );
    h.push('\n');
}

fn hero(h: &mut String, layout: &Layout, shown: &str) {
    h.push_str("<h1>Migration verification report</h1>\n");
    if let (Some(o), Some(n)) = (&layout.old_source, &layout.new_source) {
        let _ = writeln!(
            h,
            r#"<p class="sub">{} <span aria-hidden="true">→</span><span class="sr"> compared with </span> {}</p>"#,
            esc(o),
            esc(n)
        );
    }
    let _ = write!(
        h,
        r#"<section class="verdict {}" aria-label="Overall verdict"><div class="eyebrow">Overall verdict</div><div class="word">{}</div><p>{}</p><div class="cov"><span>Coverage</span><strong>{}</strong></div></section>"#,
        tone_class(&layout.verdict_word),
        esc(shown),
        esc(layout::verdict_meaning(&layout.verdict_word)),
        esc(&layout.coverage_line)
    );
    h.push('\n');

    // Counted before anything was left out, so the tiles never under-report.
    let data = layout.data_issue_tables;
    let other = layout.other_issue_tables;
    let mut tiles = vec![
        ("In scope", layout.in_scope, ""),
        ("Fully checked", layout.fully_checked, ""),
        ("Counts only", layout.counts_only, ""),
        ("Ignored", layout.ignored, ""),
    ];
    // The parts always add up to the tables in scope.
    if layout.not_checked > 0 {
        tiles.push(("Not checked", layout.not_checked, " caution-tile"));
    }
    tiles.push(("Data differences", data, if data > 0 { " bad" } else { "" }));
    tiles.push((
        "Other failures",
        other,
        if other > 0 { " caution-tile" } else { "" },
    ));
    h.push_str(r#"<div class="tiles">"#);
    for (label, n, class) in tiles {
        let _ = write!(
            h,
            r#"<div class="tile{class}"><strong>{n}</strong><span>{label}</span></div>"#
        );
    }
    h.push_str("</div>\n");
    if !layout.omitted.is_empty() {
        let _ = writeln!(
            h,
            r#"<p class="left-out" data-testid="left-out">Left out of this report when it was made: {}. The verdict, coverage and counts above include them; the run log and the results file list everything.</p>"#,
            esc(&layout.omitted.join(", "))
        );
    }
}

/// The search and filter bar for the whole report: one search box and boxes
/// for each kind of item, applied to the failure cards, the table list, the
/// structure findings, the ignored items and the warnings.  It only hides and
/// shows what is already on the page.
fn toolbar(h: &mut String, layout: &Layout) {
    let n_kind = |k: &str| layout.tables.iter().filter(|t| kind_key(t) == k).count();
    let warnings = layout
        .tables
        .iter()
        .filter(|t| t.verdict_word == "WARNING")
        .count()
        + layout
            .structure_findings
            .iter()
            .filter(|f| f.verdict == "WARNING")
            .count()
        + layout.warnings.len()
        + layout.changed_during_check.len();
    let boxes = [
        ("data", IssueKind::Data.label(), n_kind("data")),
        (
            "structure",
            IssueKind::Structure.label(),
            n_kind("structure") + layout.structure_findings.len(),
        ),
        (
            "not_mapped",
            IssueKind::NotMapped.label(),
            layout.not_mapped.len(),
        ),
        ("other", IssueKind::Other.label(), n_kind("other")),
        ("warning", "Warnings", warnings),
        ("pass", "Passed tables", n_kind("pass")),
    ];
    h.push_str(
        r#"<div class="toolbar js-only" role="search" aria-label="Search and filter the report">"#,
    );
    h.push_str(r#"<label class="search"><span class="sr">Search the whole report</span><input type="search" id="gq" placeholder="Search the whole report" autocomplete="off"></label>"#);
    h.push_str(r#"<fieldset class="chips"><legend class="sr">Show</legend>"#);
    for (key, label, n) in boxes {
        let _ = write!(
            h,
            r#"<label class="chip {}"><input type="checkbox" class="gk" value="{key}" checked> {} <span class="n">{n}</span></label>"#,
            if key == "data" { "bad" } else { "neutral" },
            esc(label)
        );
    }
    h.push_str("</fieldset>");
    h.push_str(r#"<button type="button" class="btn" id="gonly">Only data differences</button><button type="button" class="btn" id="greset">Clear</button><span class="count" id="gcount" aria-live="polite"></span></div>"#);
    h.push('\n');
}

fn record(h: &mut String, layout: &Layout) {
    h.push_str(r#"<section><h2>Check record</h2><dl class="kv">"#);
    for (k, v) in layout::record_rows(layout) {
        let _ = write!(h, "<dt>{}</dt><dd>{}</dd>", esc(k), esc(&v));
    }
    h.push_str("</dl></section>\n");
}

/// Shown in a section whose every card the search or filters hid.
const EMPTY_NOTE: &str =
    r#"<p class="muted gempty" hidden>Nothing here matches the search or filters.</p>"#;

/// One failure as a card.
fn failure_card(h: &mut String, t: &TableSummary) {
    let search = format!(
        "{} {} {} {}",
        t.table,
        t.verdict_word,
        t.level,
        t.reasons.join(" ")
    )
    .to_lowercase();
    let _ = write!(
        h,
        r##"<li class="f" data-kind="{}" data-verdict="{}" data-search="{}"><div class="card-head">{}<a href="#t-{}">{}</a><span class="muted">{}</span></div><ul>"##,
        kind_key(t),
        esc(&t.verdict_word),
        esc(&search),
        pill(&t.verdict_word),
        anchor(&t.table),
        esc(&t.table),
        esc(&t.level)
    );
    if t.reasons.is_empty() {
        h.push_str("<li>no reason recorded</li>");
    }
    for r in &t.reasons {
        let _ = write!(h, "<li>{}</li>", esc(r));
    }
    h.push_str("</ul></li>");
}

/// Data differences first (always in the report), then every other failure,
/// grouped by kind.
fn failures(h: &mut String, layout: &Layout) {
    let of = |k: IssueKind| -> Vec<&TableSummary> {
        layout.tables.iter().filter(|t| t.kind == Some(k)).collect()
    };
    h.push_str(r#"<section class="data-first" id="data-differences"><h2>Data differences</h2>"#);
    let data = of(IssueKind::Data);
    if data.is_empty() {
        h.push_str(r#"<p class="muted">None. No rows or values differ in the tables that were compared.</p>"#);
    } else {
        h.push_str(r#"<p class="muted">Tables where rows or values differ between the two databases.</p><ul class="cards">"#);
        for t in data {
            failure_card(h, t);
        }
        h.push_str("</ul>");
        h.push_str(EMPTY_NOTE);
    }
    h.push_str("</section>\n");

    not_mapped_section(h, layout);

    let structure = of(IssueKind::Structure);
    let other = of(IssueKind::Other);
    h.push_str("<section><h2>Other failures</h2>");
    if structure.is_empty() && other.is_empty() {
        h.push_str(if layout.omitted.is_empty() {
            r#"<p class="muted">None.</p>"#
        } else {
            r#"<p class="muted">None in this report (see what was left out above).</p>"#
        });
    }
    for (kind, list) in [(IssueKind::Structure, structure), (IssueKind::Other, other)] {
        if list.is_empty() {
            continue;
        }
        let _ = write!(h, r#"<h3>{}</h3><ul class="cards">"#, esc(kind.label()));
        for t in list {
            failure_card(h, t);
        }
        h.push_str("</ul>");
    }
    h.push_str(EMPTY_NOTE);
    h.push_str("</section>\n");
}

/// Tables that could not be paired, apart from everything that was compared.
/// Always in the report: a table missing from the new database is data that
/// did not arrive.
fn not_mapped_section(h: &mut String, layout: &Layout) {
    h.push_str(r#"<section id="not-mapped"><h2>Not mapped</h2>"#);
    if layout.not_mapped.is_empty() {
        h.push_str(r#"<p class="muted">None. Every table was paired with a table in the other database.</p>"#);
    } else {
        h.push_str(r#"<p class="muted">Tables with no partner in the other database, so nothing could be compared. A table missing from the new database fails the check.</p><ul class="cards">"#);
        for (table, why) in &layout.not_mapped {
            let _ = write!(
                h,
                r#"<li class="f" data-kind="not_mapped" data-verdict="" data-search="{}"><div class="card-head"><strong>{}</strong><span class="muted">{}</span></div></li>"#,
                esc(&format!("{table} {why}").to_lowercase()),
                esc(table),
                esc(why)
            );
        }
        h.push_str("</ul>");
        h.push_str(EMPTY_NOTE);
    }
    h.push_str("</section>\n");
}

/// The kind of a table for the filter: data, structure, other, or pass.
fn kind_key(t: &TableSummary) -> &'static str {
    match t.kind {
        Some(IssueKind::Data) => "data",
        Some(IssueKind::Structure) => "structure",
        Some(IssueKind::Other) => "other",
        Some(IssueKind::NotMapped) => "not_mapped",
        None => "pass",
    }
}

/// A stable id fragment for a table name (letters, digits, '-', '_', '.').
fn anchor(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.') {
                c
            } else {
                '-'
            }
        })
        .collect()
}

fn options(h: &mut String, layout: &Layout) {
    if !layout.options_recorded {
        let _ = writeln!(
            h,
            "<section><h2>Options applied</h2><p>{}</p></section>",
            esc(layout::OPTIONS_NOT_RECORDED)
        );
    } else if !layout.options_applied_lines.is_empty() {
        h.push_str("<section><h2>Options applied</h2><p class=\"muted\">These column options loosened the comparison. Each is part of what was signed off.</p><ul>");
        for l in &layout.options_applied_lines {
            let _ = write!(h, "<li>{}</li>", esc(l));
        }
        h.push_str("</ul></section>\n");
    }
}

fn tables(h: &mut String, layout: &Layout) {
    // Tables that were compared; the not-mapped ones have their own section.
    let listed: Vec<&TableSummary> = layout
        .tables
        .iter()
        .filter(|t| t.kind != Some(IssueKind::NotMapped))
        .collect();
    h.push_str(r#"<section id="tables"><h2>Per-table results</h2>"#);
    if listed.is_empty() {
        h.push_str(r#"<p class="muted">No tables were compared.</p></section>"#);
        return;
    }

    // Filters: the verdicts and levels actually present, in a stable order.
    let mut verdicts: Vec<(String, usize)> = Vec::new();
    let mut levels: Vec<String> = Vec::new();
    for t in &listed {
        match verdicts.iter_mut().find(|(w, _)| *w == t.verdict_word) {
            Some((_, n)) => *n += 1,
            None => verdicts.push((t.verdict_word.clone(), 1)),
        }
        if !levels.contains(&t.level) {
            levels.push(t.level.clone());
        }
    }
    verdicts.sort_by_key(|(w, _)| (verdict_rank(w), w.clone()));
    levels.sort();

    h.push_str(r#"<div class="controls js-only" role="group" aria-label="Filter tables">"#);
    h.push_str(r#"<label class="search"><span class="sr">Search tables</span><input type="search" id="tq" placeholder="Search tables, reasons, columns" autocomplete="off"></label>"#);
    // Kind of problem: data differences first.
    let mut kinds: Vec<(&str, &str, usize)> = Vec::new();
    for (key, label) in [
        ("data", IssueKind::Data.label()),
        ("structure", IssueKind::Structure.label()),
        ("other", IssueKind::Other.label()),
        ("pass", "Passed"),
    ] {
        let n = listed.iter().filter(|t| kind_key(t) == key).count();
        if n > 0 {
            kinds.push((key, label, n));
        }
    }
    h.push_str(r#"<fieldset class="chips"><legend class="sr">Kind of problem</legend>"#);
    for (key, label, n) in &kinds {
        let _ = write!(
            h,
            r#"<label class="chip {}"><input type="checkbox" class="tk" value="{key}" checked> {} <span class="n">{n}</span></label>"#,
            if *key == "data" { "bad" } else { "neutral" },
            esc(label)
        );
    }
    h.push_str("</fieldset>");
    h.push_str(r#"<fieldset class="chips"><legend class="sr">Verdict</legend>"#);
    for (w, n) in &verdicts {
        let _ = write!(
            h,
            r#"<label class="chip {}"><input type="checkbox" class="tv" value="{}" checked> {} <span class="n">{n}</span></label>"#,
            tone_class(w),
            esc(w),
            esc(w)
        );
    }
    h.push_str("</fieldset>");
    h.push_str(r#"<label class="sel">Level <select id="tl"><option value="">All levels</option>"#);
    for l in &levels {
        let _ = write!(h, r#"<option value="{}">{}</option>"#, esc(l), esc(l));
    }
    h.push_str("</select></label>");
    h.push_str(r#"<label class="switch"><input type="checkbox" id="tf"> Not passing only</label>"#);
    h.push_str(r#"<span class="grow"></span><button type="button" class="btn" id="expand">Expand all</button><button type="button" class="btn" id="collapse">Collapse all</button><button type="button" class="btn" id="reset">Reset filters</button>"#);
    h.push_str("</div>");
    let _ = write!(
        h,
        r#"<p class="count js-only" id="tcount" aria-live="polite">Showing {n} of {n} tables</p>"#,
        n = listed.len()
    );

    h.push_str(r#"<div class="scroll"><table class="data" id="tt"><thead><tr>"#);
    let heads: [(&str, &str, bool); 8] = [
        ("Table", "text", false),
        ("Level", "text", false),
        ("Verdict", "verdict", false),
        ("Old rows", "num", true),
        ("New rows", "num", true),
        ("Missing", "num", true),
        ("Extra", "num", true),
        ("Changed", "num", true),
    ];
    for (i, (label, kind, right)) in heads.iter().enumerate() {
        let _ = write!(
            h,
            r#"<th scope="col" aria-sort="none"{}><button type="button" class="sort" data-col="{i}" data-kind="{kind}">{label}<span class="arrow" aria-hidden="true"></span></button></th>"#,
            if *right { r#" class="r""# } else { "" }
        );
    }
    h.push_str("</tr></thead>\n");

    for t in &listed {
        let dash = |v: &Option<String>| v.clone().unwrap_or_else(|| "-".to_string());
        let old_rows = dash(&t.old_rows);
        let new_rows = dash(&t.new_rows);
        let mut search = vec![t.table.clone(), t.level.clone(), t.verdict_word.clone()];
        search.extend(t.reasons.iter().cloned());
        search.extend(t.changed_columns.iter().cloned());
        search.extend(t.plain_words.iter().cloned());
        search.extend(t.column_notes.iter().cloned());
        search.extend(t.keys.iter().cloned());
        let search = search.join(" ").to_lowercase();
        let _ = write!(
            h,
            r#"<tbody class="t f" id="t-{}" data-kind="{}" data-verdict="{}" data-level="{}" data-search="{}" data-rank="{}">"#,
            anchor(&t.table),
            kind_key(t),
            esc(&t.verdict_word),
            esc(&t.level),
            esc(&search),
            verdict_rank(&t.verdict_word)
        );
        let _ = write!(
            h,
            r#"<tr class="main"><td data-v="{}"><button type="button" class="toggle" aria-expanded="false"><span class="chev" aria-hidden="true"></span>{}</button></td><td data-v="{}">{}</td><td data-v="{}">{}</td>"#,
            esc(&t.table.to_lowercase()),
            esc(&t.table),
            esc(&t.level),
            esc(&t.level),
            verdict_rank(&t.verdict_word),
            pill(&t.verdict_word)
        );
        for v in [
            &old_rows,
            &new_rows,
            &t.missing_rows,
            &t.extra_rows,
            &t.changed_rows,
        ] {
            let _ = write!(
                h,
                r#"<td class="r" data-v="{}">{}</td>"#,
                sort_num(v),
                esc(v)
            );
        }
        h.push_str("</tr>");
        h.push_str(r#"<tr class="detail"><td colspan="8"><dl class="kv small">"#);
        for (k, v) in table_detail_rows(t) {
            let _ = write!(h, "<dt>{}</dt><dd>{}</dd>", esc(&k), esc(&v));
        }
        h.push_str("</dl></td></tr></tbody>\n");
    }
    h.push_str("</table></div>");
    h.push_str(r#"<p class="empty muted" id="tempty" hidden>No table matches these filters.</p>"#);
    h.push_str("</section>\n");
}

/// Sort rank for verdict words: gravest first.
fn verdict_rank(w: &str) -> u8 {
    match w {
        "TOOL FAULT" => 0,
        "FAIL" => 1,
        "WIDELY DIFFERENT" => 2,
        "CHANGED DURING CHECK" => 3,
        "NEEDS REVIEW" => 4,
        "WARNING" => 5,
        "PASS" => 6,
        _ => 7,
    }
}

/// The per-table detail list, the same fields the PDF's "Table details" shows.
fn table_detail_rows(t: &TableSummary) -> Vec<(String, String)> {
    let mut rows = vec![
        (
            "Buckets".to_string(),
            format!(
                "{} differing, {} missing, {} extra",
                t.differing_buckets, t.missing_buckets, t.extra_buckets
            ),
        ),
        (
            "Rows".to_string(),
            format!(
                "{} missing, {} extra, {} changed",
                t.missing_rows, t.extra_rows, t.changed_rows
            ),
        ),
        (
            "Reasons".to_string(),
            if t.reasons.is_empty() {
                "none recorded".to_string()
            } else {
                t.reasons.join("; ")
            },
        ),
    ];
    if !t.changed_columns.is_empty() {
        rows.push(("Changed columns".into(), t.changed_columns.join(", ")));
    }
    for w in &t.plain_words {
        rows.push(("Note".into(), w.clone()));
    }
    for n in &t.column_notes {
        rows.push(("Column note".into(), n.clone()));
    }
    for ct in &t.control_totals {
        rows.push(("Control total".into(), ct.clone()));
    }
    if !t.keys.is_empty() {
        rows.push(("Keys / covered columns".into(), t.keys.join(", ")));
    }
    if !t.keyed_rows.is_empty() {
        let label = if t.keyed_rows.len() == 1 {
            "Row key".to_string()
        } else {
            format!("Row keys ({})", t.keyed_rows.len())
        };
        rows.push((label, t.keyed_rows.join(";  ")));
    }
    rows
}

fn structure(h: &mut String, layout: &Layout) {
    h.push_str(r#"<section id="structure"><h2>Structure findings</h2>"#);
    if layout.structure_findings.is_empty() {
        h.push_str(r#"<p class="muted">None. The structure matched after the mapping was applied.</p></section>"#);
        h.push('\n');
        return;
    }
    let mut verdicts: Vec<String> = Vec::new();
    for f in &layout.structure_findings {
        if !verdicts.contains(&f.verdict) {
            verdicts.push(f.verdict.clone());
        }
    }
    verdicts.sort_by_key(|w| (verdict_rank(w), w.clone()));
    h.push_str(
        r#"<div class="controls js-only" role="group" aria-label="Filter structure findings">"#,
    );
    h.push_str(r#"<label class="search"><span class="sr">Search structure findings</span><input type="search" id="sq" placeholder="Search objects and findings" autocomplete="off"></label>"#);
    h.push_str(
        r#"<label class="sel">Verdict <select id="sv"><option value="">All verdicts</option>"#,
    );
    for w in &verdicts {
        let _ = write!(h, r#"<option value="{}">{}</option>"#, esc(w), esc(w));
    }
    h.push_str("</select></label></div>");
    let _ = write!(
        h,
        r#"<p class="count js-only" id="scount" aria-live="polite">Showing {n} of {n} findings</p>"#,
        n = layout.structure_findings.len()
    );
    h.push_str(r#"<div class="scroll"><table class="data" id="st"><thead><tr><th scope="col">Object</th><th scope="col">Verdict</th><th scope="col">Finding</th></tr></thead><tbody>"#);
    for f in &layout.structure_findings {
        let search = format!("{} {} {}", f.object, f.verdict, f.message).to_lowercase();
        let _ = write!(
            h,
            r#"<tr class="f" data-kind="structure" data-verdict="{}" data-search="{}"><td class="mono">{}</td><td>{}</td><td>{}</td></tr>"#,
            esc(&f.verdict),
            esc(&search),
            esc(&f.object),
            pill(&f.verdict),
            esc(&f.message)
        );
    }
    h.push_str("</tbody></table></div></section>\n");
}

fn lists(h: &mut String, layout: &Layout) {
    h.push_str("<section><h2>Ignored items</h2>");
    if !layout.not_in_run.is_empty() {
        let _ = write!(h, "<p>{}</p>", esc(&layout::not_in_run_line(layout)));
    }
    if layout.ignored_items.is_empty() {
        if layout.not_in_run.is_empty() {
            h.push_str(r#"<p class="muted">None. Nothing in scope was skipped.</p>"#);
        }
    } else {
        h.push_str(r#"<div class="scroll"><table class="data"><thead><tr><th scope="col">Item</th><th scope="col">Reason</th></tr></thead><tbody>"#);
        for i in &layout.ignored_items {
            let _ = write!(
                h,
                r#"<tr class="f" data-kind="ignored" data-verdict="" data-search="{}"><td class="mono">{}</td><td>{}</td></tr>"#,
                esc(&format!("{} {}", i.path, i.reason).to_lowercase()),
                esc(&i.path),
                esc(&i.reason)
            );
        }
        h.push_str("</tbody></table></div>");
    }
    h.push_str("</section>\n");

    let mut warnings = layout.warnings.clone();
    for t in &layout.changed_during_check {
        warnings.push(format!("{t} changed during the check"));
    }
    if !warnings.is_empty() {
        h.push_str("<section><h2>Warnings</h2><ul>");
        for w in &warnings {
            let _ = write!(
                h,
                r#"<li class="f" data-kind="note" data-verdict="WARNING" data-search="{}">{}</li>"#,
                esc(&w.to_lowercase()),
                esc(w)
            );
        }
        h.push_str("</ul></section>\n");
    }
    if !layout.suggestions.is_empty() {
        h.push_str(r#"<section><h2>Suggested matches</h2><div class="scroll"><table class="data"><thead><tr><th scope="col">Unmapped</th><th scope="col">Possible match</th><th scope="col" class="r">Confidence</th></tr></thead><tbody>"#);
        for s in &layout.suggestions {
            let _ = write!(
                h,
                r#"<tr><td class="mono">{}</td><td class="mono">{}</td><td class="r">{}</td></tr>"#,
                esc(&s.path),
                esc(&s.candidate),
                esc(&s.confidence)
            );
        }
        h.push_str("</tbody></table></div></section>\n");
    }
    if !layout.connections.is_empty() {
        h.push_str("<section><h2>Connections</h2><ul>");
        for c in &layout.connections {
            let _ = write!(h, "<li>{}</li>", esc(c));
        }
        h.push_str("</ul></section>\n");
    }
}

fn custom_checks(h: &mut String, layout: &Layout) {
    if layout.custom_checks.is_empty() && layout.hook_scripts.is_empty() {
        return;
    }
    h.push_str("<section><h2>Custom checks (Python hooks)</h2><p class=\"muted\">These checks are scripts your team supplied. Their results count toward the overall verdict.</p>");
    if !layout.custom_checks.is_empty() {
        h.push_str(r#"<div class="scroll"><table class="data"><thead><tr><th scope="col">Check</th><th scope="col">Result</th><th scope="col">Table</th><th scope="col">Message</th></tr></thead><tbody>"#);
        for c in &layout.custom_checks {
            let _ = write!(
                h,
                "<tr><td class=\"mono\">{}</td><td>{}</td><td class=\"mono\">{}</td><td>{}</td></tr>",
                esc(&c.id),
                esc(&c.severity.to_uppercase()),
                esc(c.table.as_deref().unwrap_or("-")),
                esc(&c.message)
            );
        }
        h.push_str("</tbody></table></div>");
    }
    if !layout.hook_warnings.is_empty() {
        h.push_str("<ul>");
        for w in &layout.hook_warnings {
            let _ = write!(h, "<li>hook warning: {}</li>", esc(w));
        }
        h.push_str("</ul>");
    }
    if !layout.hook_scripts.is_empty() {
        h.push_str(r#"<dl class="kv">"#);
        for s in &layout.hook_scripts {
            let _ = write!(
                h,
                r#"<dt>{}</dt><dd class="mono">SHA-256 {}</dd>"#,
                esc(&s.script),
                esc(&s.sha256)
            );
        }
        h.push_str("</dl>");
    }
    h.push_str("</section>\n");
}

fn integrity(h: &mut String, layout: &Layout, results_sha256: &str) {
    let mut rows: Vec<(&str, String)> = vec![
        ("Run ID", layout.run_id.clone()),
        ("Tool version", layout.tool_version.clone()),
        ("Results SHA-256", results_sha256.to_string()),
        ("Mapping hash", layout.mapping_hash.clone()),
        ("Results schema version", layout.results_version.clone()),
    ];
    if !layout.definition_hash.is_empty() {
        rows.push(("Definition hash", layout.definition_hash.clone()));
    }
    if let Some(v) = &layout.capture_sha256_old {
        rows.push(("Old capture SHA-256", v.clone()));
    }
    if let Some(v) = &layout.capture_sha256_new {
        rows.push(("New capture SHA-256", v.clone()));
    }
    if let Some(v) = &layout.wal_start_lsn {
        rows.push(("Write-ahead log start", v.clone()));
    }
    if let Some(v) = &layout.wal_end_lsn {
        rows.push(("Write-ahead log end", v.clone()));
    }
    h.push_str(r#"<section><h2>Run identity and integrity</h2><dl class="kv">"#);
    for (k, v) in rows {
        let _ = write!(h, r#"<dt>{}</dt><dd class="mono">{}</dd>"#, esc(k), esc(&v));
    }
    h.push_str("</dl></section>\n");
}

fn limits(h: &mut String) {
    for (title, body) in [
        (
            "What a PASS does not prove",
            layout::WHAT_PASS_DOES_NOT_PROVE,
        ),
        ("Limit of the row hashes", layout::MD5_THREAT_MODEL),
        ("Trust and audit", layout::TAMPER_EVIDENT_NOT_PROOF),
    ] {
        let _ = writeln!(
            h,
            "<section><h2>{}</h2><p>{}</p></section>",
            esc(title),
            esc(body)
        );
    }
}

fn sign_off(h: &mut String) {
    let block = layout::sign_off_block();
    let _ = write!(
        h,
        r#"<section class="signoff"><h2>{}</h2><p>{}</p><div class="fields">"#,
        esc(block[0]),
        esc(block[1])
    );
    for field in &block[2..] {
        let label = field.split(':').next().unwrap_or(field).trim();
        let _ = write!(h, r#"<div class="field"><span>{}</span></div>"#, esc(label));
    }
    h.push_str("</div></section>\n");
}

const CSS: &str = r#":root{--ink:#1d1d1f;--muted:#5f5f64;--faint:#8e8e93;--bg:#f5f5f7;--card:#ffffff;--rule:#dcdce2;--zebra:#f6f6f9;--head:#eeedfb;--accent:#4a3df0;--accent-ink:#2a2487;
--good:#0b6e4f;--good-bg:#e7f6ef;--bad:#b42318;--bad-bg:#fdecea;--caution:#8a4b00;--caution-bg:#fff3dc;--neutral:#3f3f46;--neutral-bg:#efeff2;color-scheme:light dark}
@media (prefers-color-scheme:dark){:root{--ink:#f5f5f7;--muted:#b0b0b8;--faint:#8e8e96;--bg:#161618;--card:#232326;--rule:#38383d;--zebra:#2a2a2e;--head:#2c2a4a;--accent:#857bff;--accent-ink:#cdc9ff;
--good:#5fd4a6;--good-bg:#12352a;--bad:#ff8a80;--bad-bg:#3d1916;--caution:#ffc266;--caution-bg:#3b2a10;--neutral:#d0d0d8;--neutral-bg:#2f2f34}}
*{box-sizing:border-box}
body{margin:0;background:var(--bg);color:var(--ink);font:15px/1.5 -apple-system,BlinkMacSystemFont,"Segoe UI",Roboto,"Liberation Sans",Arial,sans-serif}
main{max-width:1120px;margin:0 auto;padding:8px 24px 32px}
header.top{display:flex;align-items:center;justify-content:space-between;gap:16px;flex-wrap:wrap;max-width:1120px;margin:0 auto;padding:20px 24px 14px;border-bottom:1px solid var(--rule)}
.brand{display:flex;align-items:center;gap:10px}.brand img{border-radius:8px}.brand div{display:flex;flex-direction:column;line-height:1.2}.brand strong{font-size:17px}.brand span{font-size:12px;color:var(--muted)}
.meta{display:flex;align-items:center;gap:14px;flex-wrap:wrap;font-size:12.5px;color:var(--muted)}
h1{font-size:30px;line-height:1.2;margin:26px 0 4px}
h2{font-size:18px;margin:0 0 12px;padding-left:11px;border-left:3px solid var(--accent)}
.sub{color:var(--muted);margin:0 0 18px;word-break:break-word}
section{background:var(--card);border:1px solid var(--rule);border-radius:12px;padding:20px 22px;margin:16px 0}
.verdict{border-left:7px solid var(--tone);background:var(--tone-bg)}
.verdict .eyebrow{font-size:11px;font-weight:700;letter-spacing:.08em;text-transform:uppercase;color:var(--tone)}
.verdict .word{font-size:40px;font-weight:800;line-height:1.1;color:var(--tone);margin:4px 0 8px}
.verdict p{margin:0 0 14px;max-width:70ch}
.cov span{display:block;font-size:12px;color:var(--muted)}
.good{--tone:var(--good);--tone-bg:var(--good-bg)}.bad{--tone:var(--bad);--tone-bg:var(--bad-bg)}.caution{--tone:var(--caution);--tone-bg:var(--caution-bg)}.neutral{--tone:var(--neutral);--tone-bg:var(--neutral-bg)}
.tiles{display:grid;grid-template-columns:repeat(auto-fit,minmax(140px,1fr));gap:10px;margin:16px 0}
.tile{background:var(--card);border:1px solid var(--rule);border-radius:12px;padding:12px 14px;display:flex;flex-direction:column}
.tile strong{font-size:26px;line-height:1.1}.tile span{font-size:12.5px;color:var(--muted)}
.tile.bad{background:var(--bad-bg);border-color:transparent}.tile.bad strong{color:var(--bad)}
.tile.caution-tile{background:var(--caution-bg);border-color:transparent}.tile.caution-tile strong{color:var(--caution)}
.toolbar{position:sticky;top:0;z-index:5;display:none;flex-wrap:wrap;align-items:center;gap:10px;margin:12px 0;padding:10px 14px;border:1px solid var(--rule);border-radius:12px;background:color-mix(in srgb,var(--card) 92%,transparent);backdrop-filter:blur(8px)}
.js .toolbar.js-only{display:flex}
.toolbar .count{margin:0 0 0 auto}
.left-out{margin:0 0 16px;padding:12px 16px;border-radius:10px;background:var(--caution-bg);color:var(--caution);font-size:14px}
section.data-first{border-left:4px solid var(--bad)}
section h3{font-size:15px;margin:14px 0 8px;color:var(--muted)}
dl.kv{display:grid;grid-template-columns:minmax(150px,220px) 1fr;margin:0;border-radius:8px;overflow:hidden}
dl.kv dt,dl.kv dd{margin:0;padding:8px 12px;border-top:1px solid var(--rule)}
dl.kv dt:first-of-type,dl.kv dt:first-of-type+dd{border-top:0}
dl.kv dt{font-weight:600;color:var(--muted);font-size:13px}dl.kv dd{word-break:break-word}
dl.kv.small dt,dl.kv.small dd{padding:6px 10px;font-size:13px}
.mono,dd.mono{font-family:ui-monospace,SFMono-Regular,Menlo,Consolas,"Liberation Mono",monospace;font-size:13px;word-break:break-all}
.muted{color:var(--muted)}
li,p,dd{overflow-wrap:anywhere}
.pill{display:inline-block;padding:1px 9px;border-radius:999px;font-size:11.5px;font-weight:700;letter-spacing:.02em;white-space:nowrap;color:var(--tone);background:var(--tone-bg)}
ul.cards{list-style:none;margin:0;padding:0;display:grid;gap:10px}
ul.cards>li{border:1px solid var(--rule);border-radius:10px;padding:10px 14px}
.card-head{display:flex;align-items:center;gap:10px;flex-wrap:wrap}.card-head a{font-weight:700;color:var(--ink);text-decoration:none}.card-head a:hover{text-decoration:underline}.card-head .muted{margin-left:auto;font-size:13px}
ul.cards ul{margin:6px 0 0;padding-left:20px}
.controls{display:flex;flex-wrap:wrap;align-items:center;gap:10px;margin:0 0 10px}
.controls .grow{flex:1}
.search input{width:min(320px,80vw);padding:8px 12px;border:1px solid var(--rule);border-radius:8px;background:var(--bg);color:var(--ink);font:inherit}
.chips{display:flex;flex-wrap:wrap;gap:6px;border:0;margin:0;padding:0}
.chip{display:inline-flex;align-items:center;gap:6px;padding:4px 10px;border-radius:999px;background:var(--tone-bg);color:var(--tone);font-size:12.5px;font-weight:700;cursor:pointer}
.chip .n{font-weight:500;opacity:.85}
.chip input{accent-color:var(--tone);margin:0}
.sel select{margin-left:6px;padding:6px 8px;border:1px solid var(--rule);border-radius:8px;background:var(--bg);color:var(--ink);font:inherit}
.switch{display:inline-flex;align-items:center;gap:6px;font-size:14px}
.btn{padding:6px 12px;border:1px solid var(--rule);border-radius:8px;background:var(--card);color:var(--ink);font:inherit;font-size:13.5px;cursor:pointer}
.btn:hover{border-color:var(--accent)}
button:focus-visible,input:focus-visible,select:focus-visible,a:focus-visible{outline:3px solid color-mix(in srgb,var(--accent) 55%,transparent);outline-offset:2px}
.count{font-size:13px;color:var(--muted);margin:0 0 8px}
.scroll{overflow-x:auto}
table.data{width:100%;border-collapse:collapse;font-size:14px}
table.data th{background:var(--head);color:var(--accent-ink);text-align:left;font-size:12.5px;padding:0;white-space:nowrap}
table.data th:not(:has(button)){padding:8px 10px}
table.data td{padding:8px 10px;border-top:1px solid var(--rule);vertical-align:top}
table.data .r{text-align:right;font-variant-numeric:tabular-nums}
#tt tbody.t:nth-of-type(even) tr.main{background:var(--zebra)}
#st tbody tr:nth-child(even){background:var(--zebra)}
button.sort{all:unset;box-sizing:border-box;display:flex;align-items:center;gap:4px;width:100%;padding:8px 10px;cursor:pointer;font-weight:700}
th.r button.sort{justify-content:flex-end}
th[aria-sort=ascending] .arrow::after{content:"▲";font-size:9px}th[aria-sort=descending] .arrow::after{content:"▼";font-size:9px}
button.toggle{all:unset;cursor:pointer;display:inline-flex;align-items:flex-start;gap:6px;font-weight:600;word-break:break-word}
.chev::before{content:"▸";display:inline-block;width:10px;color:var(--muted)}
button.toggle[aria-expanded=true] .chev::before{content:"▾"}
tr.detail td{background:var(--bg);padding:10px 14px 14px}
.js tr.detail{display:none}.js tr.detail.open{display:table-row}
.js-only{display:none}.js .js-only{display:flex}.js p.js-only{display:block}.js button.js-only{display:inline-block}
.signoff .fields{display:grid;grid-template-columns:1fr 1fr;gap:28px 36px;margin-top:18px}
.field{border-bottom:1px solid var(--muted);padding-bottom:30px}.field span{font-size:12.5px;font-weight:700;color:var(--muted)}
footer{max-width:1120px;margin:0 auto;padding:14px 24px 30px;display:flex;flex-wrap:wrap;gap:8px 20px;color:var(--faint);font-size:12px;border-top:1px solid var(--rule)}
.sr{position:absolute;width:1px;height:1px;overflow:hidden;clip:rect(0 0 0 0);white-space:nowrap}
@media (max-width:640px){main{padding:4px 16px 24px}header.top{padding:16px}dl.kv{grid-template-columns:1fr}dl.kv dd{border-top:0;padding-top:0}.verdict .word{font-size:30px}.signoff .fields{grid-template-columns:1fr}}
@media print{:root{color-scheme:light}body{background:#fff;font-size:11px}.js-only,.controls,.count,#print{display:none!important}section{break-inside:avoid-page;border-color:#ccc}#tables,#structure{break-inside:auto}.tile{border-color:#ccc}main,header.top,footer{max-width:none}a{color:inherit}}
"#;

const JS: &str = r#"(function () {
  'use strict';
  var root = document.documentElement;
  root.classList.add('js');
  var $ = function (id) { return document.getElementById(id); };
  var all = function (sel, el) { return Array.prototype.slice.call((el || document).querySelectorAll(sel)); };

  // An item is shown only when the section's own filter (lh) and the global
  // filter (gh) both allow it.
  var show = function (el) { el.hidden = el.getAttribute('data-lh') === '1' || el.getAttribute('data-gh') === '1'; };
  var refresh = [];

  var printBtn = $('print');
  if (printBtn) printBtn.addEventListener('click', function () { window.print(); });

  // ---- per-table results ----
  var table = $('tt');
  if (table) {
    var bodies = all('tbody.t', table);
    var q = $('tq'), lvl = $('tl'), notPass = $('tf'), count = $('tcount'), empty = $('tempty');
    var chips = all('input.tv');
    var kindChips = all('input.tk');
    var setOpen = function (tb, open) {
      var d = tb.querySelector('tr.detail'), b = tb.querySelector('button.toggle');
      d.classList.toggle('open', open);
      b.setAttribute('aria-expanded', open ? 'true' : 'false');
    };
    bodies.forEach(function (tb) {
      tb.querySelector('button.toggle').addEventListener('click', function () {
        setOpen(tb, !tb.querySelector('tr.detail').classList.contains('open'));
      });
    });
    var apply = function () {
      var text = q.value.trim().toLowerCase();
      var allowed = {};
      chips.forEach(function (c) { if (c.checked) allowed[c.value] = true; });
      var kinds = {};
      kindChips.forEach(function (c) { if (c.checked) kinds[c.value] = true; });
      var shown = 0;
      bodies.forEach(function (tb) {
        var v = tb.getAttribute('data-verdict');
        var ok = !!allowed[v] && !!kinds[tb.getAttribute('data-kind')] &&
          (!lvl.value || tb.getAttribute('data-level') === lvl.value) &&
          (!notPass.checked || v !== 'PASS') &&
          (!text || tb.getAttribute('data-search').indexOf(text) !== -1);
        tb.setAttribute('data-lh', ok ? '' : '1');
        show(tb);
        if (!tb.hidden) shown++;
      });
      count.textContent = 'Showing ' + shown + ' of ' + bodies.length + ' tables';
      empty.hidden = shown !== 0;
    };
    [q, lvl, notPass].concat(chips, kindChips).forEach(function (el) {
      el.addEventListener('input', apply);
      el.addEventListener('change', apply);
    });
    $('expand').addEventListener('click', function () { bodies.forEach(function (tb) { if (!tb.hidden) setOpen(tb, true); }); });
    $('collapse').addEventListener('click', function () { bodies.forEach(function (tb) { setOpen(tb, false); }); });
    refresh.push(apply);
    $('reset').addEventListener('click', function () {
      q.value = ''; lvl.value = ''; notPass.checked = false;
      chips.concat(kindChips).forEach(function (c) { c.checked = true; });
      apply();
    });

    // Sorting: click a header; click again to reverse.  Ties keep the
    // original (gravest-first, then by name) order.
    bodies.forEach(function (tb, i) { tb.setAttribute('data-i', i); });
    var heads = all('button.sort', table);
    heads.forEach(function (btn) {
      btn.addEventListener('click', function () {
        var th = btn.parentNode;
        var dir = th.getAttribute('aria-sort') === 'ascending' ? 'descending' : 'ascending';
        heads.forEach(function (b) { b.parentNode.setAttribute('aria-sort', 'none'); });
        th.setAttribute('aria-sort', dir);
        var col = +btn.getAttribute('data-col'), kind = btn.getAttribute('data-kind');
        var key = function (tb) { return tb.querySelector('tr.main').children[col].getAttribute('data-v'); };
        var sorted = bodies.slice().sort(function (a, b) {
          var x = key(a), y = key(b), r;
          if (kind === 'num' || kind === 'verdict') r = (+x) - (+y);
          else r = x < y ? -1 : x > y ? 1 : 0;
          if (dir === 'descending') r = -r;
          return r || (+a.getAttribute('data-i')) - (+b.getAttribute('data-i'));
        });
        sorted.forEach(function (tb) { table.appendChild(tb); });
      });
    });
    apply();
  }

  // ---- structure findings ----
  var st = $('st');
  if (st) {
    var rows = all('tbody tr', st), sq = $('sq'), sv = $('sv'), sc = $('scount');
    var applyS = function () {
      var text = sq.value.trim().toLowerCase(), n = 0;
      rows.forEach(function (tr) {
        var ok = (!sv.value || tr.getAttribute('data-verdict') === sv.value) &&
          (!text || tr.getAttribute('data-search').indexOf(text) !== -1);
        tr.setAttribute('data-lh', ok ? '' : '1');
        show(tr);
        if (!tr.hidden) n++;
      });
      sc.textContent = 'Showing ' + n + ' of ' + rows.length + ' findings';
    };
    refresh.push(applyS);
    sq.addEventListener('input', applyS);
    sv.addEventListener('change', applyS);
  }

  // ---- the whole report ----
  var gq = $('gq');
  if (gq) {
    var items = all('.f'), boxes = all('input.gk'), gcount = $('gcount');
    var applyG = function () {
      var text = gq.value.trim().toLowerCase(), on = {}, n = 0;
      boxes.forEach(function (b) { if (b.checked) on[b.value] = true; });
      items.forEach(function (el) {
        var k = el.getAttribute('data-kind'), v = el.getAttribute('data-verdict');
        var kindOk = k === 'ignored' || k === 'note' || !!on[k];
        var ok = kindOk && (v !== 'WARNING' || !!on.warning) &&
          (!text || el.getAttribute('data-search').indexOf(text) !== -1);
        el.setAttribute('data-gh', ok ? '' : '1');
        show(el);
        if (!el.hidden) n++;
      });
      refresh.forEach(function (f) { f(); });
      // A group of cards with nothing left to show loses its heading, and a
      // section emptied by the filter says so.
      all('ul.cards').forEach(function (ul) {
        var any = all('li.f', ul).some(function (li) { return !li.hidden; });
        ul.hidden = !any;
        var head = ul.previousElementSibling;
        if (head && head.tagName === 'H3') head.hidden = !any;
      });
      all('p.gempty').forEach(function (p) {
        var sec = p.parentNode, cards = all('li.f', sec);
        p.hidden = cards.length === 0 || cards.some(function (li) { return !li.hidden; });
      });
      gcount.textContent = 'Showing ' + n + ' of ' + items.length + ' items';
    };
    gq.addEventListener('input', applyG);
    boxes.forEach(function (b) { b.addEventListener('change', applyG); });
    $('gonly').addEventListener('click', function () {
      boxes.forEach(function (b) { b.checked = b.value === 'data'; });
      applyG();
    });
    $('greset').addEventListener('click', function () {
      gq.value = '';
      boxes.forEach(function (b) { b.checked = true; });
      applyG();
    });
    applyG();
  }
})();
"#;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::{build, ReportOptions};
    use dmux_engine::results::{
        finalise, ChangedRowExport, CheckLevel, RunResults, StructureFinding, TableDetail,
        TableResult, Verdict, RESULTS_VERSION,
    };
    use std::collections::BTreeMap;

    const SENTINEL: &str = "SECRET-ROW-VALUE-4242";

    fn fixture() -> RunResults {
        let mut tables = BTreeMap::new();
        for (name, verdict) in [
            ("demo_bank.accounts", Verdict::Fail),
            ("demo_bank.loans", Verdict::Pass),
        ] {
            tables.insert(
                name.to_string(),
                TableResult {
                    table: name.into(),
                    level: CheckLevel::FullValues,
                    verdict,
                    old_rows: Some(100),
                    new_rows: Some(99),
                    columns_covered: Some(vec!["id".into()]),
                    differing_buckets: Some(1),
                    missing_buckets: Some(0),
                    extra_buckets: Some(0),
                    missing_rows: Some(1),
                    extra_rows: Some(0),
                    changed_rows: Some(0),
                    reasons: if verdict == Verdict::Fail {
                        vec!["row counts differ <script>alert(1)</script>".into()]
                    } else {
                        vec![]
                    },
                    column_notes: vec![],
                },
            );
        }
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
                plain_words: vec![],
            },
        )]);
        finalise(RunResults {
            results_version: RESULTS_VERSION,
            run_id: "run-html-1".into(),
            tool_version: "1.1.0".into(),
            mapping_hash: "mh".into(),
            verdict: Verdict::Fail,
            coverage_line: "2 of 2 tables fully checked".into(),
            fully_checked: 2,
            in_scope: 2,
            tables,
            details,
            structure_findings: vec![StructureFinding {
                object: "demo_bank.accounts.idx".into(),
                path: vec![],
                verdict: Verdict::Fail,
                message: "index missing in the new database".into(),
            }],
            started_at: Some("2026-11-18T09:00:00Z".into()),
            finished_at: Some("2026-11-18T09:02:05Z".into()),
            old_source: Some("bank on old-db:5432 (login dmux)".into()),
            new_source: Some("bank on new-db:5432 (login dmux)".into()),
            ..RunResults::default()
        })
    }

    fn html(include_keys: bool) -> String {
        let layout = build(
            &fixture(),
            None,
            &ReportOptions {
                include_keys,
                generated_at: Some("2026-11-18T10:00:00Z".into()),
                ..Default::default()
            },
        );
        render(&layout, "f00d")
    }

    #[test]
    fn carries_the_verdict_record_logo_and_mandatory_prose() {
        let h = html(false);
        assert!(h.starts_with("<!doctype html>"));
        assert!(
            h.contains(r#"<div class="word">FAIL</div>"#),
            "verdict word"
        );
        assert!(h.contains("Check started</dt><dd>18 Nov 2026, 09:00:00 UTC"));
        assert!(h.contains("Check finished</dt><dd>18 Nov 2026, 09:02:05 UTC"));
        assert!(h.contains("Duration</dt><dd>2 min 5 s"));
        assert!(h.contains("Report generated</dt><dd>18 Nov 2026, 10:00:00 UTC"));
        assert!(h.contains("bank on old-db:5432 (login dmux)"));
        assert!(
            h.contains("data:image/png;base64,iVBORw0KGgo"),
            "logo inline"
        );
        assert!(h.contains("What a PASS does not prove"));
        assert!(h.contains("md5"));
        assert!(h.contains("These hashes detect corruption"));
        assert!(h.contains("Signature"));
        assert!(h.contains("f00d"), "results SHA-256");
        assert!(h.contains("index missing in the new database"));
    }

    #[test]
    fn nothing_can_be_fetched_and_untrusted_text_is_escaped() {
        let h = html(false);
        assert!(h.contains("default-src 'none'"), "CSP present");
        assert!(!h.contains("http://") && !h.contains("https://"), "no URLs");
        assert!(!h.contains("<script>alert(1)"), "reason not escaped");
        assert!(h.contains("&lt;script&gt;alert(1)&lt;/script&gt;"));
        // The only script element is the report's own.
        assert_eq!(h.matches("<script>").count(), 1);
    }

    #[test]
    fn keys_appear_only_when_the_user_opts_in() {
        assert!(!html(false).contains(SENTINEL));
        assert!(html(true).contains(SENTINEL));
    }

    #[test]
    fn rows_carry_what_the_filters_need() {
        let h = html(false);
        assert!(h.contains(r#"data-verdict="FAIL" data-level="full values""#));
        assert!(h.contains(r#"data-verdict="PASS""#));
        assert!(
            h.contains(r#"class="tv" value="FAIL" checked"#),
            "verdict chip"
        );
        assert!(h.contains(r#"id="tq""#) && h.contains(r#"id="tf""#));
        assert!(h.contains("Showing 2 of 2 tables"));
        // Failing tables are listed before passing ones.
        let fail = h.find(r#"id="t-demo_bank.accounts""#).unwrap();
        let pass = h.find(r#"id="t-demo_bank.loans""#).unwrap();
        assert!(fail < pass);
    }

    #[test]
    fn output_is_deterministic_and_written_owner_only() {
        assert_eq!(html(false), html(false));
        let dir = std::env::temp_dir().join(format!("dmux-html-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("r.html");
        let layout = build(&fixture(), None, &ReportOptions::default());
        write_html_layout(&layout, "x", &path).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }
    }

    #[test]
    fn the_whole_report_can_be_searched_and_filtered() {
        let h = html(false);
        assert!(h.contains(r#"id="gq""#), "global search box");
        for key in ["data", "structure", "other", "warning", "pass"] {
            assert!(
                h.contains(&format!(r#"class="gk" value="{key}" checked"#)),
                "{key} box"
            );
        }
        assert!(h.contains(r#"id="gonly""#) && h.contains(r#"id="greset""#));
        // Every card, table and finding carries what the filter needs.
        assert!(h.contains(r#"<li class="f" data-kind="data" data-verdict="FAIL""#));
        assert!(h.contains(r#"<tbody class="t f" id="t-demo_bank.loans" data-kind="pass""#));
        assert!(h.contains(r#"<tr class="f" data-kind="structure" data-verdict="FAIL""#));
        // Data differences come before the other failures.
        assert!(
            h.find("<h2>Data differences</h2>").unwrap()
                < h.find("<h2>Other failures</h2>").unwrap()
        );
    }

    #[test]
    fn base64_matches_the_standard_alphabet() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
    }
}
