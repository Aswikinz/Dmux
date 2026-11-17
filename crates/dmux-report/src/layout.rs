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

use dmux_engine::results::{CheckLevel, RunResults, TableDetail, TableResult, Verdict};

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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ReportOptions {
    /// Include key / covered-column names in the differences detail.  Default
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
#[derive(Debug, Clone, PartialEq, Eq, Default)]
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
    /// Drill-down detail: columns that differ, plain-language notes and
    /// control-total differences (aggregates, never row data).
    pub changed_columns: Vec<String>,
    pub plain_words: Vec<String>,
    pub control_totals: Vec<String>,
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
    pub structure_findings: Vec<StructureFindingLine>,
    pub suggestions: Vec<SuggestionLine>,
    pub definition_hash: String,
    /// Hook evidence (present only for an envelope).
    pub custom_checks: Vec<CustomCheck>,
    pub hook_scripts: Vec<HookScriptLine>,
    pub hook_warnings: Vec<String>,
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
                results.details.get(*name),
                include_keys,
            )
        })
        .collect();

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
        structure_only: results.structure_only,
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
        structure_findings: results
            .structure_findings
            .iter()
            .map(|f| StructureFindingLine {
                object: sanitize(&f.object),
                verdict: verdict_word(f.verdict).to_string(),
                message: sanitize(&f.message),
            })
            .collect(),
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
    }
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
        CheckLevel, CheckLevel::FullValues, IgnoredItem, RunResults, StructureFinding, TableResult,
        Verdict, RESULTS_VERSION,
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
            &ReportOptions { include_keys: true },
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

        let with_keys = build(&r, None, &ReportOptions { include_keys: true });
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
}
