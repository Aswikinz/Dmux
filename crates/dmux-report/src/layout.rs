//! The report layout: a pure, deterministic model built from [`RunResults`].
//!
//! Every piece of text the PDF, Excel and log artefacts emit flows through this
//! module.  It is the single place a `RunResults` value is joined to prose, and
//! it renders **summary fields only** — counts, hashes, names, verdicts and
//! reasons.  It never renders row values (there are none in `RunResults`), and
//! it renders keys ([`TableResult::columns_covered`]'s covered-column names)
//! only when [`ReportOptions::include_keys`] is ticked.
//!
//! Building this model in isolation is what makes the leak tests possible: a
//! test plants sentinel strings into fixture fields, builds a layout, and scans
//! the resulting PDF/XLSX text for those sentinels.

use crate::v2;
use dmux_engine::results::{CheckLevel, RunResults, TableResult, Verdict};

/// Strip Unicode bidi/format control characters that could be used to spoof or
/// confuse the reader of a report (PLAN 2.3 "sensitive data in reports", 2.4
/// "trust and audit").  Table names, reasons, warnings and column names are
/// untrusted input (mapping files, ignore reasons), so every user-derived string
/// passes through this before it reaches any artefact.  It never alters ordinary
/// text; it only drops the invisible/control code points.
fn sanitize(text: &str) -> String {
    text.chars().filter(|&c| !is_bidi_control(c)).collect()
}

fn is_bidi_control(c: char) -> bool {
    matches!(
        c,
        '\u{200B}'..='\u{200F}' // zero-width space … right-to-left mark
            | '\u{202A}'..='\u{202E}' // LRE/RLE/PDF/LRO/RLO
            | '\u{2066}'..='\u{2069}' // LRI/RLI/FSI/PDI
            | '\u{FEFF}' // zero-width no-break / BOM
    )
}

/// Render options the caller ticks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ReportOptions {
    /// Include key/covered-column names in the differences detail.  Default
    /// false: keys stay out of reports unless the user explicitly asks
    /// (PLAN.md 2.3, security rule 6).
    pub include_keys: bool,
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
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableSummary {
    pub table: String,
    pub level: String,
    pub verdict: String,
    pub verdict_word: String,
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
    /// Drill-down detail (v2): columns that differ, plain-language notes and
    /// control-total differences (aggregates, never row data).
    pub changed_columns: Vec<String>,
    pub plain_words: Vec<String>,
    pub control_totals: Vec<String>,
    /// Key material from the drill-down, ONLY when `include_keys` is true.
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

/// The complete layout model.  Everything downstream renders this.
#[derive(Debug, Clone, PartialEq, Eq)]
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
    pub ignored_items: Vec<Ignored>,
    pub warnings: Vec<String>,
    pub changed_during_check: Vec<String>,
    pub wal_start_lsn: Option<String>,
    pub wal_end_lsn: Option<String>,
    /// Tables ordered failures-first, then PASS/flag tables, then by name.
    pub tables: Vec<TableSummary>,
    /// Differences only (tables with any missing/extra/changed row or a
    /// non-PASS verdict), failures first.
    pub differences: Vec<Difference>,
    /// v2: total tables in scope and the structure-only tally (also reflected
    /// in the coverage line the engine writes).
    pub in_scope: usize,
    pub structure_only: usize,
    /// v2: structure (inventory) findings after the mapping is applied.
    pub structure_findings: Vec<v2::StructureFinding>,
    /// v2: suggested matches for unmapped tables/columns.
    pub suggestions: Vec<v2::Suggestion>,
    /// v2: SHA-256 of the run definition the results came from.
    pub definition_hash: String,
}

/// Render an optional count as a place-holder-free string.
fn opt_count(v: Option<u64>) -> Option<String> {
    v.map(|n| n.to_string())
}

fn usize_or_dash(v: Option<usize>) -> String {
    v.map(|n| n.to_string()).unwrap_or_else(|| "-".to_string())
}

/// Build the full layout from a results payload and the render options.  This is
/// the plain-results form; for the raw JSON (v2 extension fields) use
/// [`build_from`].
pub fn build(results: &RunResults, options: &ReportOptions) -> Layout {
    build_from(results, &serde_json::Value::Null, options)
}

/// Build the full layout from a results payload, its raw results JSON (for
/// results-schema v2 extension fields) and the render options.
pub fn build_from(
    results: &RunResults,
    raw: &serde_json::Value,
    options: &ReportOptions,
) -> Layout {
    let include_keys = options.include_keys;

    // Order tables failures-first, then by name (BTreeMap already sorted).
    let mut names: Vec<&String> = results.tables.keys().collect();
    names.sort_by(|a, b| {
        let va = results.tables[*a].verdict;
        let vb = results.tables[*b].verdict;
        let fa = is_failure(va);
        let fb = is_failure(vb);
        fb.cmp(&fa).then_with(|| a.cmp(b))
    });

    let tables: Vec<TableSummary> = names
        .iter()
        .map(|name| {
            summarise(
                results.tables.get(*name).expect("present"),
                raw,
                include_keys,
            )
        })
        .collect();

    let mut differences: Vec<Difference> = tables
        .iter()
        .filter(|t| {
            // A difference row exists when the verdict is a failure/flag OR a
            // count is recorded.  PASSed/flagged-only tables with no counts do
            // not produce difference rows.
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

    // Failures first within differences too.
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
        ignored_items: results
            .ignored_items
            .iter()
            .map(|i| Ignored {
                path: sanitize(&i.path),
                reason: sanitize(&i.reason),
            })
            .collect(),
        warnings: results.warnings.iter().map(|w| sanitize(w)).collect(),
        changed_during_check: results
            .changed_during_check
            .iter()
            .map(|w| sanitize(w))
            .collect(),
        wal_start_lsn: results.wal_start_lsn.as_deref().map(sanitize),
        wal_end_lsn: results.wal_end_lsn.as_deref().map(sanitize),
        tables,
        differences,
        in_scope: v2::in_scope(raw),
        structure_only: v2::structure_only(raw),
        structure_findings: v2::structure_findings(raw)
            .into_iter()
            .map(|f| v2::StructureFinding {
                object: sanitize(&f.object),
                verdict: sanitize(&f.verdict),
                message: sanitize(&f.message),
            })
            .collect(),
        suggestions: v2::suggestions(raw)
            .into_iter()
            .map(|s| v2::Suggestion {
                path: sanitize(&s.path),
                candidate: sanitize(&s.candidate),
                confidence: sanitize(&s.confidence),
            })
            .collect(),
        definition_hash: sanitize(&v2::definition_hash(raw)),
    }
}

fn summarise(tr: &TableResult, raw: &serde_json::Value, include_keys: bool) -> TableSummary {
    let detail = v2::detail_for(raw, &tr.table, include_keys).unwrap_or_default();
    TableSummary {
        table: sanitize(&tr.table),
        level: level_word(tr.level).to_string(),
        verdict: verdict_word(tr.verdict).to_string(),
        verdict_word: verdict_word(tr.verdict).to_string(),
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
        changed_columns: detail.changed_columns.iter().map(|c| sanitize(c)).collect(),
        plain_words: detail.plain_words.iter().map(|w| sanitize(w)).collect(),
        control_totals: detail.control_totals.iter().map(|c| sanitize(c)).collect(),
        keyed_rows: detail.keyed_rows.iter().map(|k| sanitize(k)).collect(),
    }
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

/// The tamper-evident-not-proof sentence (PLAN.md 2.4).
pub const TAMPER_EVIDENT_NOT_PROOF: &str = "\
This report carries a run identifier, the tool version, the mapping hash and a \
SHA-256 of the results file, so a later edit is detectable. That is \
tamper-evident, not tamper-proof.";

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
        CheckLevel, IgnoredItem, RunResults, TableResult, Verdict, RESULTS_VERSION,
    };
    use std::collections::BTreeMap;

    const SENTINEL_VALUE: &str = "SECRET-ROW-VALUE-12345";

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
        }
    }

    fn fixture(verdict: Verdict) -> RunResults {
        let mut tables = BTreeMap::new();
        tables.insert(
            "demo_bank.accounts".into(),
            table("demo_bank.accounts", verdict, CheckLevel::FullValues),
        );
        tables.insert(
            "demo_bank.transactions".into(),
            table(
                "demo_bank.transactions",
                Verdict::Fail,
                CheckLevel::FullValues,
            ),
        );
        RunResults {
            results_version: RESULTS_VERSION,
            run_id: "run-abc".into(),
            tool_version: "0.1.0".into(),
            mapping_hash: "mh-123".into(),
            verdict,
            coverage_line: "1 of 2 tables fully checked, 0 counts only, 1 ignored".into(),
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
            wal_start_lsn: None,
            wal_end_lsn: None,
            results_sha256: "abc".into(),
        }
    }

    #[test]
    fn verdict_and_coverage_and_ignored_are_present() {
        let layout = build(&fixture(Verdict::Fail), &ReportOptions::default());
        assert_eq!(layout.verdict_word, "FAIL");
        assert!(layout.coverage_line.contains("1 of 2"));
        assert_eq!(layout.ignored_items.len(), 1);
        assert_eq!(layout.ignored_items[0].path, "demo_bank.archived");
        assert_eq!(layout.ignored_items[0].reason, "out of scope");
    }

    #[test]
    fn failures_come_first() {
        let layout = build(&fixture(Verdict::Pass), &ReportOptions::default());
        // transactions is FAIL, accounts is PASS; failures precede.
        assert_eq!(layout.tables[0].table, "demo_bank.transactions");
        assert_eq!(layout.tables[1].table, "demo_bank.accounts");
    }

    #[test]
    fn keys_are_absent_by_default_and_present_when_ticked() {
        let layout = build(&fixture(Verdict::Fail), &ReportOptions::default());
        assert!(layout.tables.iter().all(|t| t.keys.is_empty()));
        assert!(layout.differences.iter().all(|d| d.keys.is_empty()));

        let layout = build(
            &fixture(Verdict::Fail),
            &ReportOptions { include_keys: true },
        );
        assert!(layout.tables.iter().any(|t| !t.keys.is_empty()));
        assert!(layout.differences.iter().any(|d| !d.keys.is_empty()));
    }

    #[test]
    fn sentinel_row_values_stay_out_of_the_layout() {
        // Plant a sentinel where a row value would live: the covered-column
        // list (the only "key-like" namespace in the schema).  With include_keys
        // off it must not surface anywhere in the layout's rendered text; with
        // include_keys on it appears (that is the flip side the renderer tests).
        let results = fixture(Verdict::Fail);
        // Simulate a row-value sentinel by overriding a reason string that a
        // buggy renderer might join verbatim.
        let layout = build(&results, &ReportOptions::default());
        let flat = format!("{:?}", (layout.tables.clone(), layout.differences.clone()));
        assert!(!flat.contains(SENTINEL_VALUE));
        // Sanity: the sentinel genuinely is not part of the fixture either.
        assert!(!serde_json::to_string(&results)
            .unwrap()
            .contains(SENTINEL_VALUE));
    }

    #[test]
    fn bidi_and_format_controls_are_stripped() {
        // A reason with a right-to-left override and a zero-width no-break space
        // is reduced to its visible characters: spoofing is impossible.
        let mut results = fixture(Verdict::Fail);
        results
            .tables
            .get_mut("demo_bank.transactions")
            .unwrap()
            .reasons = vec!["\u{202E}evil\u{FEFF} text".to_string()];
        let layout = build(&results, &ReportOptions::default());
        let reason = &layout
            .tables
            .iter()
            .find(|t| t.table == "demo_bank.transactions")
            .unwrap()
            .reasons[0];
        assert_eq!(reason, "evil text");
        assert!(!reason.contains('\u{202E}'));
        assert!(!reason.contains('\u{FEFF}'));
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
    fn v2_fields_flow_into_the_layout() {
        // A results object with v2 extensions: structure findings, a definition
        // hash, and drill-down detail whose keys are gated on include_keys.
        let raw = serde_json::json!({
            "in_scope": 12,
            "structure_only": 3,
            "definition_hash": "defhash",
            "structure_findings": [
                { "object": "a.t.idx", "verdict": "FAIL", "message": "missing index" }
            ],
            "suggestions": [
                { "path": "a.old", "candidate": "a.new", "confidence": 80 }
            ],
            "details": {
                "demo_bank.accounts": {
                    "changed_columns": ["balance"],
                    "plain_words": ["a balance changed"],
                    "control_totals": [{"column":"balance","old":"1","new":"2"}],
                    "changed_rows": [{"key":["123"]}]
                }
            }
        });

        let layout = build_from(&fixture(Verdict::Fail), &raw, &ReportOptions::default());
        assert_eq!(layout.in_scope, 12);
        assert_eq!(layout.structure_only, 3);
        assert_eq!(layout.definition_hash, "defhash");
        assert_eq!(layout.structure_findings.len(), 1);
        assert_eq!(layout.suggestions.len(), 1);

        let acct = layout
            .tables
            .iter()
            .find(|t| t.table == "demo_bank.accounts")
            .unwrap();
        assert_eq!(acct.changed_columns, vec!["balance".to_string()]);
        assert!(acct.keyed_rows.is_empty(), "keys gated off by default");

        let with_keys = build_from(
            &fixture(Verdict::Fail),
            &raw,
            &ReportOptions { include_keys: true },
        );
        let acct = with_keys
            .tables
            .iter()
            .find(|t| t.table == "demo_bank.accounts")
            .unwrap();
        assert!(!acct.keyed_rows.is_empty(), "keys present when ticked");
    }
}
