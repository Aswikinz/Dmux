//! The report layout: a pure, deterministic model built from the *typed* v2
//! [`RunResults`] plus optional hook evidence.
//!
//! Every piece of text the PDF, Excel and log artefacts emit flows through this
//! module.  It is the single place a results value is joined to prose, and it
//! renders **summary fields only** — counts, hashes, names, verdicts and
//! reasons.  It never renders row values (there are none in `RunResults`), and
//! it renders keys (the drill-down's [`ChangedRowExport::key`] and missing/extra
//! key lists) only when [`ReportOptions::include_keys`] is ticked.
//!
//! Building this model in isolation is what makes the leak tests possible: a
//! test plants sentinel strings into fixture fields, builds a layout, and scans
//! the resulting PDF/XLSX text for those sentinels.

use dmux_engine::results::{
    table_issue_kind, CheckLevel, ConnectionInfo, IssueKind, RunResults, TableDetail, TableResult,
    Verdict,
};

use crate::verify::HookEvidence;

/// Strip Unicode bidi/format control characters and C0/C1 controls that could be
/// used to spoof or confuse the reader of a report (PLAN 2.3, 2.4).  All
/// user-derived strings are untrusted input (mapping files, ignore reasons, hook
/// findings), so every one passes through this before reaching any artefact.
/// Ordinary text is unchanged; invisible/control code points are dropped.  A
/// newline inside a field is dropped here too — it is only meaningful as a
/// deliberate paragraph break in the fixed prose, never inside a name or reason.
fn sanitize(text: &str) -> String {
    text.chars().filter(|&c| !is_stripped(c)).collect()
}

/// A character dropped from untrusted text: C0/C1 controls and invisible
/// bidi/format code points.
fn is_stripped(c: char) -> bool {
    if c.is_control() {
        return true; // C0 + C1 controls (incl. \n, \t, \r)
    }
    matches!(
        c,
        '\u{200B}'..='\u{200F}' // zero-width space … right-to-left mark
            | '\u{202A}'..='\u{202E}' // LRE/RLE/PDF/LRO/RLO
            | '\u{2060}'..='\u{2064}' // word joiner, invisible operators
            | '\u{2066}'..='\u{206F}' // LRI/RLI/FSI/PDI and control set
            | '\u{061C}' // Arabic letter mark
            | '\u{FEFF}' // zero-width no-break / BOM
    )
}

/// Render options the caller ticks.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ReportOptions {
    /// Include key / covered-column names in the differences detail.  Default
    /// false: keys stay out of reports unless the user explicitly asks
    /// (PLAN.md 2.3, security rule 6).
    pub include_keys: bool,
    /// When this report was generated, in UTC RFC 3339 (see [`now_utc`]).
    /// Callers that write a real report pass the current time; `None` leaves
    /// the line out, which keeps the golden tests byte-deterministic.
    pub generated_at: Option<String>,
    /// What the user chose to leave out of this report.  Data differences can
    /// never be left out; whatever is left out is named on page 1.
    pub leave_out: LeaveOut,
}

/// Parts of a report the user may leave out, so the data differences stand
/// out.  All `false` (the default) keeps everything.  Data differences are not
/// listed here because they are always included.  The overall verdict, the
/// coverage line, the counts, the run log and the results file are never
/// filtered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct LeaveOut {
    /// Tables whose problem is a structure difference, and structure findings.
    pub structure: bool,
    /// Tables that failed for another reason (fault, moving data, review).
    pub other: bool,
    /// Anything whose verdict is WARNING, and the run-level warnings.
    pub warnings: bool,
    /// Tables that passed.
    pub passing: bool,
}

impl LeaveOut {
    /// Parse a comma-separated list: structure, other, warnings, passing.
    pub fn parse(list: &str) -> Result<LeaveOut, String> {
        let mut out = LeaveOut::default();
        for part in list.split(',').map(str::trim).filter(|p| !p.is_empty()) {
            match part {
                "structure" => out.structure = true,
                "other" => out.other = true,
                "warnings" => out.warnings = true,
                "passing" => out.passing = true,
                "data" => {
                    return Err("data differences are always in the report".to_string());
                }
                other => {
                    return Err(format!(
                        "unknown part {other:?}: use structure, other, warnings or passing"
                    ))
                }
            }
        }
        Ok(out)
    }

    /// Whether a table (by its kind and verdict word) stays in the report.
    fn keeps_table(&self, kind: Option<IssueKind>, verdict_word: &str) -> bool {
        match kind {
            // A table missing from the new database is data that did not
            // arrive: like data differences, never left out.
            Some(IssueKind::Data | IssueKind::NotMapped) => true,
            None => !self.passing,
            Some(k) => {
                let by_kind = match k {
                    IssueKind::Structure => !self.structure,
                    _ => !self.other,
                };
                by_kind && !(self.warnings && verdict_word == "WARNING")
            }
        }
    }
}

/// The current instant in UTC RFC 3339 with whole seconds, for
/// [`ReportOptions::generated_at`].
pub fn now_utc() -> String {
    jiff::Timestamp::now()
        .strftime("%Y-%m-%dT%H:%M:%SZ")
        .to_string()
}

/// An RFC 3339 instant in plain words: "18 Nov 2026, 11:06:40 UTC".  Text that
/// does not parse is shown as written (sanitised), never dropped.
pub fn display_time(rfc3339: &str) -> String {
    match rfc3339.parse::<jiff::Timestamp>() {
        Ok(t) => t.strftime("%-d %b %Y, %H:%M:%S UTC").to_string(),
        Err(_) => sanitize(rfc3339),
    }
}

/// How long the check took, in plain words ("2 min 5 s", "1 h 3 min").  `None`
/// when either end is missing or unreadable, or the end precedes the start.
pub fn display_duration(started: &str, finished: &str) -> Option<String> {
    let s = started.parse::<jiff::Timestamp>().ok()?;
    let f = finished.parse::<jiff::Timestamp>().ok()?;
    let secs = f.as_second() - s.as_second();
    if secs < 0 {
        return None;
    }
    Some(match secs {
        0..=59 => format!("{secs} s"),
        60..=3599 => format!("{} min {} s", secs / 60, secs % 60),
        _ => format!("{} h {} min", secs / 3600, (secs % 3600) / 60),
    })
}

/// Convenience alias kept for callers that spell the option as a bool.
pub type IncludeKeys = bool;

/// The overall verdict as words (PLAN.md 1: never colour alone; always a word).
pub fn verdict_word(v: Verdict) -> &'static str {
    match v {
        Verdict::Pass => "PASS",
        Verdict::Fail => "FAIL",
        Verdict::WidelyDifferent => "WIDELY DIFFERENT",
        Verdict::NeedsReview => "NEEDS REVIEW",
        Verdict::ChangedDuringCheck => "CHANGED DURING CHECK",
        Verdict::ToolFault => "TOOL FAULT",
        Verdict::Warning => "WARNING",
    }
}

/// The displayed overall verdict word.  A PASS is never bare when the results
/// file could not vouch for options (decision 23): "(with options)" when at
/// least one column option was applied, and "(options not recorded — older
/// results file)" when the results schema predates option recording.
pub fn verdict_display(word: &str, options_applied: bool, options_recorded: bool) -> String {
    if word != "PASS" {
        return word.to_string();
    }
    if !options_recorded {
        return "PASS (options not recorded — older results file)".to_string();
    }
    if options_applied {
        "PASS (with options)".to_string()
    } else {
        "PASS".to_string()
    }
}

/// What an overall verdict word means, in the same plain words as the app's
/// Results screen.  Unknown words get no sentence.
pub fn verdict_meaning(word: &str) -> &'static str {
    match word {
        "PASS" => "The two databases match at the levels you chose.",
        "FAIL" => "The two databases do not match. The findings below say what differs.",
        "WIDELY DIFFERENT" => {
            "Many rows differ, so Dmux stopped listing them one by one. The findings below show a sample."
        }
        "NEEDS REVIEW" => {
            "Dmux could not fully confirm a match. A person must review the items below before sign-off."
        }
        "CHANGED DURING CHECK" => {
            "Data changed while Dmux was reading it, so this result cannot be relied on. Run the check again on a database that is not being written to."
        }
        "TOOL FAULT" => {
            "Dmux could not complete a check. This says nothing about your data; the reasons are listed below."
        }
        "WARNING" => {
            "No difference was found, but something needs your attention. The warnings are listed below."
        }
        _ => "",
    }
}

/// The colour family a verdict word is shown in.  Colour is never the only
/// signal: every format prints the word itself next to it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Good,
    Bad,
    Caution,
    Neutral,
}

/// The tone for a verdict word (PASS green, FAIL red, review amber, fault grey).
pub fn verdict_tone(word: &str) -> Tone {
    match word {
        "PASS" => Tone::Good,
        "FAIL" | "WIDELY DIFFERENT" => Tone::Bad,
        "WARNING" | "NEEDS REVIEW" | "CHANGED DURING CHECK" => Tone::Caution,
        _ => Tone::Neutral,
    }
}

/// The "Options applied" body when options were never recorded: every report
/// format and the log show this rather than assume none were applied.
pub const OPTIONS_NOT_RECORDED: &str = "options not recorded (older results file)";

/// Plain-language connection-safety lines for one side (audit M5, H1).  An empty
/// `tls` means the side was not a live connection (a saved fingerprint file).
fn connection_lines(side: &str, c: &ConnectionInfo) -> Vec<String> {
    if c.tls.is_empty() {
        return vec![format!("{side}: not a live connection")];
    }
    let tls = match c.tls.as_str() {
        "pinned" => format!(
            "TLS pinned (fingerprint {})",
            c.tls_fingerprint.as_deref().unwrap_or("?")
        ),
        "plaintext" => format!(
            "plaintext (reason: {})",
            c.plaintext_reason
                .as_deref()
                .unwrap_or("no reason recorded")
        ),
        _ => "TLS verified".to_string(),
    };
    let mut lines = vec![format!(
        "{side}: {tls}; PostgreSQL {}, encoding {}",
        c.server_version, c.server_encoding
    )];
    if c.superuser {
        let why = c
            .superuser_reason
            .as_deref()
            .unwrap_or("no reason recorded");
        lines.push(format!("{side}: superuser login (override reason: {why})"));
    }
    if c.can_write {
        let reasons = if c.can_write_reasons.is_empty() {
            "at least one table".to_string()
        } else {
            c.can_write_reasons.join("; ")
        };
        lines.push(format!("{side}: the login can write ({reasons})"));
    }
    if !c.rls_tables.is_empty() && !c.rolbypassrls {
        lines.push(format!(
            "{side}: {} tables use row-level security; Dmux refused to read them filtered",
            c.rls_tables.len()
        ));
    }
    if c.pooler_suspected {
        lines.push(format!("{side}: a connection pooler is suspected"));
    }
    if !c.md5_available {
        lines.push(format!("{side}: md5 is not available"));
    }
    lines
}

/// "table.column: option" lines for page 1, sorted (H2 / decision 23).
fn option_lines(results: &RunResults) -> Vec<String> {
    let mut lines = Vec::new();
    for (table, t) in &results.tables {
        for n in &t.column_notes {
            if n.kind == "option" {
                lines.push(format!(
                    "{}.{}: {}",
                    sanitize(table),
                    sanitize(&n.column),
                    sanitize(&n.note)
                ));
            }
        }
    }
    lines.sort();
    lines
}

/// Connection-safety lines for both sides (audit M5), old then new.
fn connection_lines_for(results: &RunResults) -> Vec<String> {
    let mut lines = Vec::new();
    lines.extend(connection_lines("old", &results.connections.old));
    lines.extend(connection_lines("new", &results.connections.new));
    lines
}

/// The check level in the plan's spelling.
pub fn level_word(level: CheckLevel) -> &'static str {
    match level {
        CheckLevel::Structure => "structure",
        CheckLevel::CountsAndTotals => "counts and totals",
        CheckLevel::FullValues => "full values",
    }
}

/// Whether a verdict means the table failed (a finding) versus passed or was
/// merely flagged.  Used to order failures first (PLAN.md 4.7).
fn is_failure(v: Verdict) -> bool {
    matches!(
        v,
        Verdict::Fail
            | Verdict::WidelyDifferent
            | Verdict::ChangedDuringCheck
            | Verdict::ToolFault
            | Verdict::NeedsReview
    )
}

/// One table's summary section (PDF per-table body; the Tables sheet).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TableSummary {
    pub table: String,
    pub level: String,
    pub verdict: String,
    pub verdict_word: String,
    /// The kind of problem; `None` for a PASS.
    pub kind: Option<IssueKind>,
    pub old_rows: Option<String>,
    pub new_rows: Option<String>,
    pub differing_buckets: String,
    pub missing_buckets: String,
    pub extra_buckets: String,
    pub missing_rows: String,
    pub extra_rows: String,
    pub changed_rows: String,
    /// Covered-column names, ONLY populated when `include_keys` is true.
    pub keys: Vec<String>,
    pub reasons: Vec<String>,
    /// Drill-down detail: columns that differ, plain-language notes and
    /// control-total differences (aggregates, never row data).
    pub changed_columns: Vec<String>,
    pub plain_words: Vec<String>,
    pub control_totals: Vec<String>,
    /// Per-column notes (ticked options and type-change flags), rendered as
    /// "column: note" (H2 / decision 23).
    pub column_notes: Vec<String>,
    /// Key material from the drill-down (changed-row keys, missing/extra keys),
    /// ONLY when `include_keys` is true.
    pub keyed_rows: Vec<String>,
}

/// One ignored item with its required reason (first page; the Ignored sheet).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ignored {
    pub path: String,
    pub reason: String,
}

/// A single difference line (the Differences sheet).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Difference {
    pub table: String,
    pub missing_rows: String,
    pub extra_rows: String,
    pub changed_rows: String,
    /// Key/covered-column names, ONLY when `include_keys` is true.
    pub keys: Vec<String>,
    pub reasons: Vec<String>,
}

/// A structure (inventory) finding, rendered verbatim from the typed struct.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StructureFindingLine {
    pub object: String,
    pub verdict: String,
    pub message: String,
}

/// A suggested match, rendered verbatim.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SuggestionLine {
    pub path: String,
    pub candidate: String,
    pub confidence: String,
}

/// A hook finding (custom check), marked as such (PLAN 4.9).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CustomCheck {
    pub id: String,
    pub severity: String,
    pub message: String,
    pub table: Option<String>,
}

/// A hook script's name + SHA-256.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HookScriptLine {
    pub script: String,
    pub sha256: String,
}

/// The complete layout model.  Everything downstream renders this.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Layout {
    pub results_version: String,
    pub run_id: String,
    pub tool_version: String,
    pub mapping_hash: String,
    pub results_sha256: String,
    pub verdict_word: String,
    pub coverage_line: String,
    pub fully_checked: usize,
    pub counts_only: usize,
    pub ignored: usize,
    pub in_scope: usize,
    pub structure_only: usize,
    /// Tables in scope that were neither checked at any level nor ignored
    /// (missing on one side, only in the new database, unreadable).
    pub not_checked: usize,
    /// Items a person decided to ignore, each with its reason.
    pub ignored_items: Vec<Ignored>,
    /// Tables left out because the run checked only chosen tables (not
    /// selected, or not mapped outside the run).  Listed in their own report
    /// ("Tables not in this run"), not on the main report's first page, which
    /// names how many there are (decision 30).
    pub not_in_run: Vec<Ignored>,
    pub warnings: Vec<String>,
    pub changed_during_check: Vec<String>,
    pub wal_start_lsn: Option<String>,
    pub wal_end_lsn: Option<String>,
    /// Tables ordered failures-first, then PASS/flag tables, then by name.
    pub tables: Vec<TableSummary>,
    /// Differences only (tables with any missing/extra/changed row or a
    /// non-PASS verdict), failures first.
    pub differences: Vec<Difference>,
    pub structure_findings: Vec<StructureFindingLine>,
    pub suggestions: Vec<SuggestionLine>,
    pub definition_hash: String,
    /// Whether the results file recorded per-column options (results schema
    /// version >= 3).  When false, the reader says "options not recorded",
    /// never a bare PASS or "no options".
    pub options_recorded: bool,
    /// Whether any column option was applied (H2): page 1 says
    /// "PASS (with options)" when the overall verdict is PASS.
    pub options_applied: bool,
    /// "table.column: option" lines for page 1 (sorted).
    pub options_applied_lines: Vec<String>,
    /// Capture-file SHA-256s (present only for file inputs, audit M1).
    pub capture_sha256_old: Option<String>,
    pub capture_sha256_new: Option<String>,
    /// Connection-safety lines for page 1 (audit M5).
    pub connections: Vec<String>,
    /// Hook evidence (present only for an envelope).
    pub custom_checks: Vec<CustomCheck>,
    pub hook_scripts: Vec<HookScriptLine>,
    pub hook_warnings: Vec<String>,
    /// The compliance record, already in plain words: when the check started
    /// and finished, how long it took, when this report was generated, and
    /// what each side was.  `None` when the results file predates the record.
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
    pub duration: Option<String>,
    pub generated_at: Option<String>,
    pub old_source: Option<String>,
    pub new_source: Option<String>,
    /// What was left out of this report at the user's choice, in plain words
    /// ("23 structure findings").  Empty when nothing was left out.
    pub omitted: Vec<String>,
    /// Tables that could not be paired, with why, for their own section:
    /// ("core.client_notes", "missing from the new database").
    pub not_mapped: Vec<(String, String)>,
    /// Tables with data differences, and tables that failed for any other
    /// reason, counted before anything was left out.
    pub data_issue_tables: usize,
    pub other_issue_tables: usize,
}

/// Render an optional count as a place-holder-free string.
fn opt_count(v: Option<u64>) -> Option<String> {
    v.map(|n| n.to_string())
}

fn usize_or_dash(v: Option<usize>) -> String {
    v.map(|n| n.to_string()).unwrap_or_else(|| "-".to_string())
}

/// Build the full layout from a results payload, optional hook evidence and the
/// render options.  Everything renders only from the verified typed struct.
pub fn build(
    results: &RunResults,
    hooks: Option<&HookEvidence>,
    options: &ReportOptions,
) -> Layout {
    let include_keys = options.include_keys;

    // Data differences first, then structure differences, other reasons and
    // the tables that passed; by name within each (BTreeMap already sorted).
    let kind_of = |name: &String| table_issue_kind(name, &results.tables[name], results);
    let rank = |k: Option<IssueKind>| match k {
        Some(IssueKind::Data) => 0,
        Some(IssueKind::NotMapped) => 1,
        Some(IssueKind::Structure) => 2,
        Some(IssueKind::Other) => 3,
        None => 4,
    };
    let mut names: Vec<&String> = results.tables.keys().collect();
    names.sort_by(|a, b| {
        rank(kind_of(a))
            .cmp(&rank(kind_of(b)))
            .then_with(|| a.cmp(b))
    });

    let leave = options.leave_out;
    let all_tables: Vec<TableSummary> = names
        .iter()
        .map(|name| {
            let mut t = summarise(
                results.tables.get(*name).expect("present"),
                results.details.get(*name),
                include_keys,
            );
            t.kind = kind_of(name);
            t
        })
        .collect();
    let data_issue_tables = all_tables
        .iter()
        .filter(|t| t.kind == Some(IssueKind::Data))
        .count();
    let other_issue_tables = all_tables
        .iter()
        .filter(|t| matches!(t.kind, Some(IssueKind::Structure | IssueKind::Other)))
        .count();
    let (tables, dropped): (Vec<TableSummary>, Vec<TableSummary>) = all_tables
        .into_iter()
        .partition(|t| leave.keeps_table(t.kind, &t.verdict_word));
    let mut omitted: Vec<String> = Vec::new();
    let count = |n: usize, one: &str, many: &str| {
        if n == 1 {
            format!("1 {one}")
        } else {
            format!("{n} {many}")
        }
    };
    let dropped_of =
        |pred: &dyn Fn(&TableSummary) -> bool| dropped.iter().filter(|t| pred(t)).count();
    let n = dropped_of(&|t| t.kind == Some(IssueKind::Structure));
    if n > 0 {
        omitted.push(count(
            n,
            "table with a structure difference",
            "tables with structure differences",
        ));
    }
    let n = dropped_of(&|t| t.kind == Some(IssueKind::Other));
    if n > 0 {
        omitted.push(count(
            n,
            "table that failed for another reason",
            "tables that failed for other reasons",
        ));
    }
    let n = dropped_of(&|t| t.kind.is_none());
    if n > 0 {
        omitted.push(count(n, "table that passed", "tables that passed"));
    }

    let mut differences: Vec<Difference> = tables
        .iter()
        .filter(|t| {
            let verdict = results.tables[&t.table].verdict;
            let has_counts = results.tables[&t.table]
                .missing_rows
                .or(results.tables[&t.table].extra_rows)
                .or(results.tables[&t.table].changed_rows)
                .is_some();
            is_failure(verdict) || has_counts
        })
        .map(|t| {
            let tr = &results.tables[&t.table];
            Difference {
                table: t.table.clone(),
                missing_rows: usize_or_dash(tr.missing_rows),
                extra_rows: usize_or_dash(tr.extra_rows),
                changed_rows: usize_or_dash(tr.changed_rows),
                keys: t.keys.clone(),
                reasons: t.reasons.clone(),
            }
        })
        .collect();

    differences.sort_by(|a, b| {
        let va = results.tables[&a.table].verdict;
        let vb = results.tables[&b.table].verdict;
        let fa = is_failure(va);
        let fb = is_failure(vb);
        fb.cmp(&fa).then_with(|| a.table.cmp(&b.table))
    });

    Layout {
        results_version: results.results_version.to_string(),
        run_id: sanitize(&results.run_id),
        tool_version: sanitize(&results.tool_version),
        mapping_hash: sanitize(&results.mapping_hash),
        results_sha256: sanitize(&results.results_sha256),
        verdict_word: verdict_word(results.verdict).to_string(),
        coverage_line: sanitize(&results.coverage_line),
        fully_checked: results.fully_checked,
        counts_only: results.counts_only,
        ignored: results.ignored,
        in_scope: results.in_scope,
        not_checked: dmux_engine::results::not_checked(
            results.in_scope,
            results.fully_checked,
            results.counts_only,
            results.structure_only,
            results.ignored,
        ),
        structure_only: results.structure_only,
        ignored_items: results
            .ignored_items
            .iter()
            .filter(|i| !dmux_engine::run::is_table_outside_run(&i.path, &i.reason))
            .map(|i| Ignored {
                path: sanitize(&i.path),
                reason: sanitize(&i.reason),
            })
            .collect(),
        not_in_run: results
            .ignored_items
            .iter()
            .filter(|i| dmux_engine::run::is_table_outside_run(&i.path, &i.reason))
            .map(|i| Ignored {
                path: sanitize(&i.path),
                reason: sanitize(&i.reason),
            })
            .collect(),
        warnings: if leave.warnings {
            if !results.warnings.is_empty() {
                omitted.push(count(results.warnings.len(), "run warning", "run warnings"));
            }
            Vec::new()
        } else {
            results.warnings.iter().map(|w| sanitize(w)).collect()
        },
        changed_during_check: results
            .changed_during_check
            .iter()
            .map(|w| sanitize(w))
            .collect(),
        wal_start_lsn: results.wal_start_lsn.as_deref().map(sanitize),
        wal_end_lsn: results.wal_end_lsn.as_deref().map(sanitize),
        tables,
        differences,
        structure_findings: {
            // Gravest first, then in the engine's (deterministic) order.
            let all = results.structure_findings.len();
            let mut found: Vec<&dmux_engine::results::StructureFinding> = results
                .structure_findings
                .iter()
                .filter(|f| !(leave.structure || (leave.warnings && f.verdict == Verdict::Warning)))
                .collect();
            if found.len() < all {
                omitted.push(count(
                    all - found.len(),
                    "structure finding",
                    "structure findings",
                ));
            }
            found.sort_by_key(|f| std::cmp::Reverse(dmux_engine::results::severity(f.verdict)));
            found
                .into_iter()
                .map(|f| StructureFindingLine {
                    object: sanitize(&f.object),
                    verdict: verdict_word(f.verdict).to_string(),
                    message: sanitize(&f.message),
                })
                .collect()
        },
        suggestions: results
            .suggestions
            .iter()
            .map(|s| SuggestionLine {
                path: sanitize(&s.path),
                candidate: sanitize(&s.candidate),
                confidence: s.confidence.to_string(),
            })
            .collect(),
        definition_hash: sanitize(&results.definition_hash),
        custom_checks: hooks
            .map(|h| {
                h.custom_checks
                    .iter()
                    .map(|c| CustomCheck {
                        id: sanitize(&c.id),
                        severity: sanitize(&c.severity),
                        message: sanitize(&c.message),
                        table: c.table.as_deref().map(sanitize),
                    })
                    .collect()
            })
            .unwrap_or_default(),
        hook_scripts: hooks
            .map(|h| {
                h.scripts
                    .iter()
                    .map(|s| HookScriptLine {
                        script: sanitize(&s.script),
                        sha256: sanitize(&s.sha256),
                    })
                    .collect()
            })
            .unwrap_or_default(),
        hook_warnings: hooks
            .map(|h| h.warnings.iter().map(|w| sanitize(w)).collect())
            .unwrap_or_default(),
        options_recorded: dmux_engine::results::options_recorded(results),
        options_applied: results.options_applied,
        options_applied_lines: option_lines(results),
        capture_sha256_old: results.capture_sha256_old.as_deref().map(sanitize),
        capture_sha256_new: results.capture_sha256_new.as_deref().map(sanitize),
        connections: connection_lines_for(results),
        started_at: results.started_at.as_deref().map(display_time),
        finished_at: results.finished_at.as_deref().map(display_time),
        duration: match (&results.started_at, &results.finished_at) {
            (Some(s), Some(f)) => display_duration(s, f),
            _ => None,
        },
        generated_at: options.generated_at.as_deref().map(display_time),
        old_source: results.old_source.as_deref().map(sanitize),
        new_source: results.new_source.as_deref().map(sanitize),
        // Last, so it holds what the fields above left out.
        omitted,
        data_issue_tables,
        not_mapped: results
            .unmapped
            .iter()
            .map(|(table, why)| (sanitize(table), why.label().to_string()))
            .collect(),
        other_issue_tables,
    }
}

/// The main report's one line about the tables left out of a run of chosen
/// tables; they are listed in their own report.
pub fn not_in_run_line(layout: &Layout) -> String {
    let n = layout.not_in_run.len();
    format!(
        "{} not part of this run, which checked only the chosen tables. {} listed, each with its reason, in the separate report \"Tables not in this run\" and in the Excel workbook.",
        if n == 1 { "1 table was".to_string() } else { format!("{n} tables were") },
        if n == 1 { "It is" } else { "They are" }
    )
}

/// The compliance record as (label, value) rows, in reading order.  Every
/// format renders these, so a missing value says so rather than vanishing.
pub fn record_rows(layout: &Layout) -> Vec<(&'static str, String)> {
    let or_not = |v: &Option<String>| {
        v.clone()
            .unwrap_or_else(|| "not recorded (older results file)".to_string())
    };
    let mut rows = vec![
        ("Check started", or_not(&layout.started_at)),
        ("Check finished", or_not(&layout.finished_at)),
    ];
    if let Some(d) = &layout.duration {
        rows.push(("Duration", d.clone()));
    }
    if let Some(g) = &layout.generated_at {
        rows.push(("Report generated", g.clone()));
    }
    rows.push(("Old database", or_not(&layout.old_source)));
    rows.push(("New database", or_not(&layout.new_source)));
    rows.push(("Run ID", layout.run_id.clone()));
    rows.push(("Dmux version", layout.tool_version.clone()));
    rows
}

fn summarise(tr: &TableResult, detail: Option<&TableDetail>, include_keys: bool) -> TableSummary {
    let mut keyed_rows = Vec::new();
    if include_keys {
        if let Some(d) = detail {
            for row in &d.changed_rows {
                if let Some(key) = &row.key {
                    keyed_rows.push(format_key(key));
                }
            }
            if let Some(missing) = &d.missing_keys {
                for k in missing {
                    keyed_rows.push(format!("missing {}", format_key(k)));
                }
            }
            if let Some(extra) = &d.extra_keys {
                for k in extra {
                    keyed_rows.push(format!("extra {}", format_key(k)));
                }
            }
        }
    }

    TableSummary {
        table: sanitize(&tr.table),
        level: level_word(tr.level).to_string(),
        verdict: verdict_word(tr.verdict).to_string(),
        verdict_word: verdict_word(tr.verdict).to_string(),
        kind: None,
        old_rows: opt_count(tr.old_rows),
        new_rows: opt_count(tr.new_rows),
        differing_buckets: usize_or_dash(tr.differing_buckets),
        missing_buckets: usize_or_dash(tr.missing_buckets),
        extra_buckets: usize_or_dash(tr.extra_buckets),
        missing_rows: usize_or_dash(tr.missing_rows),
        extra_rows: usize_or_dash(tr.extra_rows),
        changed_rows: usize_or_dash(tr.changed_rows),
        keys: if include_keys {
            tr.columns_covered
                .clone()
                .unwrap_or_default()
                .iter()
                .map(|k| sanitize(k))
                .collect()
        } else {
            Vec::new()
        },
        reasons: tr.reasons.iter().map(|r| sanitize(r)).collect(),
        changed_columns: detail
            .map(|d| d.changed_columns.iter().map(|c| sanitize(c)).collect())
            .unwrap_or_default(),
        plain_words: detail
            .map(|d| d.plain_words.iter().map(|w| sanitize(w)).collect())
            .unwrap_or_default(),
        control_totals: detail
            .map(|d| {
                d.control_totals
                    .iter()
                    .map(|ct| {
                        format!(
                            "{}: {} -> {}",
                            sanitize(&ct.column),
                            sanitize(&ct.old),
                            sanitize(&ct.new)
                        )
                    })
                    .collect()
            })
            .unwrap_or_default(),
        column_notes: tr
            .column_notes
            .iter()
            .map(|n| format!("{}: {}", sanitize(&n.column), sanitize(&n.note)))
            .collect(),
        keyed_rows,
    }
}

fn format_key(key: &[Option<String>]) -> String {
    key.iter()
        .map(|k| k.as_deref().map(sanitize).unwrap_or_else(|| "NULL".into()))
        .collect::<Vec<_>>()
        .join(", ")
}

/// The plan-mandated fixed prose (PLAN.md 1, 2.4; decision 10).
pub const WHAT_PASS_DOES_NOT_PROVE: &str = "\
A PASS does not prove that the application works on the new database, that the \
new database is fast, or that data which was already wrong before the migration \
is now right. It does not cover tables, columns or rows that were ignored, \
unmapped or checked at a lower level.";

/// The md5 threat-model sentence every report must carry (decision 10).
pub const MD5_THREAT_MODEL: &str = "\
Row hashes use md5. The agreed threat model is accidental migration error, not \
a malicious operator: a deliberate attacker who can write to the new database \
could in principle craft rows that hash to the same code, and Dmux cannot \
detect that. An independent database administrator reviews the method before \
first real use.";

/// The hash-integrity sentence (audit M1 / decision 23): the hashes on page 1
/// detect corruption; they show tampering only when compared with the values
/// printed on the signed PDF.
pub const TAMPER_EVIDENT_NOT_PROOF: &str = "\
These hashes detect corruption. They show tampering only when compared with \
the values printed on the signed PDF.";

/// The plain sign-off block (name, role, date, signature lines), separated for
/// easy testing.  The date is left blank for a person to complete at signing.
pub fn sign_off_block() -> Vec<&'static str> {
    vec![
        "Sign-off",
        "I confirm I have read this report, including the coverage line, every \
         ignored item and its reason, and the limitations above.",
        "Name: ______________________________",
        "Role: ______________________________",
        "Date: ______________________________",
        "Signature: ______________________________",
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use dmux_engine::results::{
        CheckLevel, CheckLevel::FullValues, IgnoredItem, RunResults, StructureFinding, TableResult,
        Verdict, RESULTS_VERSION,
    };
    use std::collections::BTreeMap;

    const SENTINEL_VALUE: &str = "SECRET-ROW-VALUE-12345";

    #[test]
    fn times_and_durations_read_plainly() {
        assert_eq!(
            display_time("2026-11-18T09:05:07Z"),
            "18 Nov 2026, 09:05:07 UTC"
        );
        // Unreadable text is shown (sanitised), never dropped.
        assert_eq!(display_time("yesterday\u{202E}"), "yesterday");
        let d = |a, b| display_duration(a, b);
        assert_eq!(
            d("2026-11-18T09:00:00Z", "2026-11-18T09:00:52Z").as_deref(),
            Some("52 s")
        );
        assert_eq!(
            d("2026-11-18T09:00:00Z", "2026-11-18T09:02:05Z").as_deref(),
            Some("2 min 5 s")
        );
        assert_eq!(
            d("2026-11-18T09:00:00Z", "2026-11-18T10:03:00Z").as_deref(),
            Some("1 h 3 min")
        );
        assert_eq!(
            d("2026-11-18T09:00:00Z", "2026-11-18T08:00:00Z"),
            None,
            "end before start"
        );
        assert_eq!(d("nope", "2026-11-18T08:00:00Z"), None);
    }

    #[test]
    fn verdict_words_have_a_meaning_and_a_tone() {
        for w in [
            "PASS",
            "FAIL",
            "WIDELY DIFFERENT",
            "NEEDS REVIEW",
            "CHANGED DURING CHECK",
            "TOOL FAULT",
            "WARNING",
        ] {
            assert!(!verdict_meaning(w).is_empty(), "{w}");
        }
        assert_eq!(verdict_tone("PASS"), Tone::Good);
        assert_eq!(verdict_tone("WIDELY DIFFERENT"), Tone::Bad);
        assert_eq!(verdict_tone("CHANGED DURING CHECK"), Tone::Caution);
        assert_eq!(verdict_tone("TOOL FAULT"), Tone::Neutral);
    }

    fn table(name: &str, verdict: Verdict, level: CheckLevel) -> TableResult {
        TableResult {
            table: name.to_string(),
            level,
            verdict,
            old_rows: Some(100),
            new_rows: Some(99),
            columns_covered: Some(vec!["id".into(), "balance".into()]),
            differing_buckets: Some(1),
            missing_buckets: Some(0),
            extra_buckets: Some(1),
            missing_rows: Some(1),
            extra_rows: None,
            changed_rows: None,
            reasons: vec![],
            column_notes: vec![],
        }
    }

    fn fixture(verdict: Verdict) -> RunResults {
        let mut tables = BTreeMap::new();
        tables.insert(
            "demo_bank.accounts".into(),
            table("demo_bank.accounts", verdict, FullValues),
        );
        tables.insert(
            "demo_bank.transactions".into(),
            table("demo_bank.transactions", Verdict::Fail, FullValues),
        );
        RunResults {
            results_version: RESULTS_VERSION,
            run_id: "run-abc".into(),
            tool_version: "0.1.0".into(),
            mapping_hash: "mh-123".into(),
            verdict,
            coverage_line:
                "1 of 2 tables fully checked, 0 counts only, 0 structure only, 1 ignored".into(),
            fully_checked: 1,
            counts_only: 0,
            ignored: 1,
            tables,
            ignored_items: vec![IgnoredItem {
                path: "demo_bank.archived".into(),
                reason: "out of scope".into(),
            }],
            changed_during_check: vec![],
            warnings: vec!["old database is SQL_ASCII".into()],
            ..RunResults::default()
        }
    }

    #[test]
    fn verdict_and_coverage_and_ignored_are_present() {
        let layout = build(&fixture(Verdict::Fail), None, &ReportOptions::default());
        assert_eq!(layout.verdict_word, "FAIL");
        assert!(layout.coverage_line.contains("1 of 2"));
        assert_eq!(layout.ignored_items.len(), 1);
        assert_eq!(layout.ignored_items[0].path, "demo_bank.archived");
        assert_eq!(layout.ignored_items[0].reason, "out of scope");
    }

    #[test]
    fn failures_come_first() {
        let layout = build(&fixture(Verdict::Pass), None, &ReportOptions::default());
        assert_eq!(layout.tables[0].table, "demo_bank.transactions");
        assert_eq!(layout.tables[1].table, "demo_bank.accounts");
    }

    #[test]
    fn keys_are_absent_by_default_and_present_when_ticked() {
        let layout = build(&fixture(Verdict::Fail), None, &ReportOptions::default());
        assert!(layout.tables.iter().all(|t| t.keys.is_empty()));
        assert!(layout.differences.iter().all(|d| d.keys.is_empty()));

        let layout = build(
            &fixture(Verdict::Fail),
            None,
            &ReportOptions {
                include_keys: true,
                ..Default::default()
            },
        );
        assert!(layout.tables.iter().any(|t| !t.keys.is_empty()));
        assert!(layout.differences.iter().any(|d| !d.keys.is_empty()));
    }

    #[test]
    fn drilldown_keys_are_gated_on_include_keys() {
        // Plant sentinels in the drill-down key fields (the fields that must
        // never be printed without --include-keys).
        let mut r = fixture(Verdict::Fail);
        r.details.insert(
            "demo_bank.transactions".into(),
            dmux_engine::results::TableDetail {
                changed_columns: vec!["amount".into()],
                changed_rows: vec![dmux_engine::results::ChangedRowExport {
                    key: Some(vec![Some(SENTINEL_VALUE.into())]),
                    columns: vec!["amount".into()],
                }],
                missing_keys: Some(vec![vec![Some("MISSING-KEY-1".into())]]),
                extra_keys: Some(vec![vec![Some("EXTRA-KEY-2".into())]]),
                control_totals: vec![],
                plain_words: vec!["an amount changed".into()],
            },
        );

        let no_keys = build(&r, None, &ReportOptions::default());
        let t = no_keys
            .tables
            .iter()
            .find(|t| t.table == "demo_bank.transactions")
            .unwrap();
        assert!(t.keyed_rows.is_empty(), "keys must be absent by default");

        let with_keys = build(
            &r,
            None,
            &ReportOptions {
                include_keys: true,
                ..Default::default()
            },
        );
        let t = with_keys
            .tables
            .iter()
            .find(|t| t.table == "demo_bank.transactions")
            .unwrap();
        assert!(!t.keyed_rows.is_empty());
        let joined = t.keyed_rows.join(" ");
        assert!(joined.contains(SENTINEL_VALUE), "key present when ticked");
        assert!(joined.contains("EXTRA-KEY-2"));
    }

    #[test]
    fn sentinel_row_values_stay_out_of_the_layout() {
        let results = fixture(Verdict::Fail);
        let layout = build(&results, None, &ReportOptions::default());
        let flat = format!("{:?}", (layout.tables.clone(), layout.differences.clone()));
        assert!(!flat.contains(SENTINEL_VALUE));
        assert!(!serde_json::to_string(&results)
            .unwrap()
            .contains(SENTINEL_VALUE));
    }

    #[test]
    fn bidi_and_format_controls_are_stripped() {
        // A reason with a right-to-left override, an Arabic letter mark and a
        // zero-width no-break space is reduced to its visible characters.
        let mut results = fixture(Verdict::Fail);
        results
            .tables
            .get_mut("demo_bank.transactions")
            .unwrap()
            .reasons = vec!["\u{202E}evil\u{061C}\u{FEFF} text".to_string()];
        let layout = build(&results, None, &ReportOptions::default());
        let reason = &layout
            .tables
            .iter()
            .find(|t| t.table == "demo_bank.transactions")
            .unwrap()
            .reasons[0];
        assert_eq!(reason, "evil text");
        assert!(!reason.contains('\u{202E}'));
        assert!(!reason.contains('\u{061C}'));
        assert!(!reason.contains('\u{FEFF}'));
    }

    #[test]
    fn extra_format_and_bidi_ranges_are_stripped() {
        // U+061C, U+2060–U+2064, U+206A–U+206F, plus C0/C1, must all be dropped.
        let mut results = fixture(Verdict::Fail);
        results
            .tables
            .get_mut("demo_bank.transactions")
            .unwrap()
            .reasons = vec!["a\u{061C}\u{2060}\u{2064}\u{206A}\u{206F}\u{0000}b".to_string()];
        let layout = build(&results, None, &ReportOptions::default());
        let reason = &layout
            .tables
            .iter()
            .find(|t| t.table == "demo_bank.transactions")
            .unwrap()
            .reasons[0];
        assert_eq!(reason, "ab");
        for c in [
            '\u{061C}', '\u{2060}', '\u{2064}', '\u{206A}', '\u{206F}', '\u{0000}',
        ] {
            assert!(!reason.contains(c), "must strip {c:?}");
        }
    }

    #[test]
    fn c0_and_c1_controls_are_stripped() {
        let mut results = fixture(Verdict::Fail);
        results
            .tables
            .get_mut("demo_bank.transactions")
            .unwrap()
            .reasons = vec![format!("tab{}\u{000B}ret\r\nend", '\t')];
        let layout = build(&results, None, &ReportOptions::default());
        let reason = &layout
            .tables
            .iter()
            .find(|t| t.table == "demo_bank.transactions")
            .unwrap()
            .reasons[0];
        assert!(!reason.contains('\t'));
        assert!(!reason.contains('\u{000B}'));
        assert!(!reason.contains('\r'));
        assert!(!reason.contains('\n'));
    }

    #[test]
    fn hook_findings_flow_into_the_layout() {
        use crate::verify::{HookEvidence, HookFinding};
        let hooks = HookEvidence {
            custom_checks: vec![HookFinding {
                id: "c1".into(),
                severity: "fail".into(),
                message: "custom finding".into(),
                table: Some("s.t".into()),
            }],
            ..Default::default()
        };
        let layout = build(
            &fixture(Verdict::Pass),
            Some(&hooks),
            &ReportOptions::default(),
        );
        assert_eq!(layout.custom_checks.len(), 1);
        assert_eq!(layout.custom_checks[0].severity, "fail");
        assert_eq!(layout.custom_checks[0].message, "custom finding");
    }

    #[test]
    fn verdict_words_match_the_plan_vocabulary() {
        assert_eq!(verdict_word(Verdict::Pass), "PASS");
        assert_eq!(verdict_word(Verdict::Fail), "FAIL");
        assert_eq!(verdict_word(Verdict::WidelyDifferent), "WIDELY DIFFERENT");
        assert_eq!(verdict_word(Verdict::NeedsReview), "NEEDS REVIEW");
        assert_eq!(
            verdict_word(Verdict::ChangedDuringCheck),
            "CHANGED DURING CHECK"
        );
        assert_eq!(verdict_word(Verdict::ToolFault), "TOOL FAULT");
    }

    #[test]
    fn structure_findings_flow_in_typed() {
        let mut r = fixture(Verdict::Fail);
        r.structure_findings = vec![StructureFinding {
            object: "a.t.idx".into(),
            path: vec!["a".into(), "t".into(), "idx".into()],
            verdict: Verdict::Fail,
            message: "missing index".into(),
        }];
        let layout = build(&r, None, &ReportOptions::default());
        assert_eq!(layout.structure_findings.len(), 1);
        assert_eq!(layout.structure_findings[0].message, "missing index");
        assert_eq!(layout.structure_findings[0].verdict, "FAIL");
    }

    #[test]
    fn pass_with_options_and_connections_reach_the_layout() {
        use dmux_engine::results::{ColumnNote, ConnectionInfo, Connections};
        let mut r = fixture(Verdict::Pass);
        r.options_applied = true;
        r.tables.get_mut("demo_bank.accounts").unwrap().column_notes = vec![ColumnNote {
            column: "balance".into(),
            kind: "option".into(),
            note: "round to 2 decimals".into(),
        }];
        r.connections = Connections {
            old: ConnectionInfo {
                tls: "plaintext".into(),
                plaintext_reason: Some("test network".into()),
                superuser: true,
                superuser_reason: Some("migration needs it".into()),
                can_write: true,
                server_version: "17.0".into(),
                server_encoding: "UTF8".into(),
                ..ConnectionInfo::default()
            },
            new: ConnectionInfo::default(),
        };
        let layout = build(&r, None, &ReportOptions::default());
        assert!(layout.options_applied);
        assert!(layout.options_recorded);
        assert_eq!(
            verdict_display(
                &layout.verdict_word,
                layout.options_applied,
                layout.options_recorded
            ),
            "PASS (with options)"
        );
        assert_eq!(
            layout.options_applied_lines,
            vec!["demo_bank.accounts.balance: round to 2 decimals"]
        );
        assert!(layout
            .connections
            .iter()
            .any(|l| l.contains("plaintext") && l.contains("test network")));
        assert!(layout
            .connections
            .iter()
            .any(|l| l.contains("superuser") && l.contains("migration needs it")));
        let notes = &layout
            .tables
            .iter()
            .find(|t| t.table == "demo_bank.accounts")
            .unwrap()
            .column_notes;
        assert_eq!(notes, &vec!["balance: round to 2 decimals".to_string()]);
    }

    #[test]
    fn an_old_results_file_never_shows_a_bare_pass() {
        // A v2 results file never recorded options, so a PASS must be qualified
        // and the options line must say "not recorded", never "none" or a bare
        // PASS (H2 / decision 23 / fix round).
        let mut r = fixture(Verdict::Pass);
        r.results_version = 2;
        r.options_applied = false;
        let layout = build(&r, None, &ReportOptions::default());
        assert!(!layout.options_recorded);
        assert_eq!(
            verdict_display(
                &layout.verdict_word,
                layout.options_applied,
                layout.options_recorded
            ),
            "PASS (options not recorded — older results file)"
        );
        assert!(layout.options_applied_lines.is_empty());
    }

    #[test]
    fn rls_and_can_write_reasons_reach_the_connection_lines() {
        use dmux_engine::results::Connections;
        let mut r = fixture(Verdict::Fail);
        r.connections = Connections {
            old: ConnectionInfo {
                tls: "verified".into(),
                can_write: true,
                can_write_reasons: vec!["has INSERT on at least one table".into()],
                rolbypassrls: false,
                rls_tables: vec!["s.accounts".into(), "s.audit_log".into()],
                server_version: "17.0".into(),
                server_encoding: "UTF8".into(),
                ..ConnectionInfo::default()
            },
            new: ConnectionInfo::default(),
        };
        let layout = build(&r, None, &ReportOptions::default());
        assert!(layout
            .connections
            .iter()
            .any(|l| l.contains("has INSERT on at least one table")));
        assert!(layout
            .connections
            .iter()
            .any(|l| l.contains("2 tables use row-level security")));
    }
}
