//! The results model: per-table and run-level verdicts, the coverage line and
//! the stable JSON schema the CLI, the UI and the report hooks share.
//!
//! This module turns the comparison core's verdicts into the run-level verdict
//! vocabulary of PLAN.md 1 and docs/how-dmux-decides.md.  It never passes a run
//! unless every in-scope table passed at its own level, at least one table was
//! actually checked, and nothing was skipped, changed, or faulted without a
//! recorded reason.  The per-table decision itself lives in
//! `crate::run::verdict`, the one place every comparison path goes through.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use crate::core::compare::Verdict as CoreVerdict;
use crate::estimate::Estimate;

/// The run-level verdict vocabulary.  Serialises to the SCREAMING_SNAKE strings
/// the UI and report contract expects (PASS, FAIL, WIDELY_DIFFERENT,
/// NEEDS_REVIEW, CHANGED_DURING_CHECK, TOOL_FAULT, WARNING).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Hash)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Verdict {
    Pass,
    Fail,
    WidelyDifferent,
    NeedsReview,
    ChangedDuringCheck,
    ToolFault,
    /// Something to look at, never a data PASS on its own.
    Warning,
}

impl Verdict {
    pub fn is_pass(self) -> bool {
        self == Verdict::Pass
    }

    /// The worse of two verdicts (see [`severity`]).
    pub fn worst(self, other: Verdict) -> Verdict {
        if severity(other) > severity(self) {
            other
        } else {
            self
        }
    }
}

/// Map a comparison-core verdict into the run vocabulary.
///
/// The core's `Error` is always a tool fault: a statement that failed, a scan
/// that errored, or a table that could not be compared is never the data's blame
/// and never a PASS.
pub fn from_core(v: CoreVerdict) -> Verdict {
    match v {
        CoreVerdict::Pass => Verdict::Pass,
        CoreVerdict::Fail => Verdict::Fail,
        CoreVerdict::WidelyDifferent => Verdict::WidelyDifferent,
        CoreVerdict::NeedsReview => Verdict::NeedsReview,
        CoreVerdict::Error => Verdict::ToolFault,
    }
}

/// The three check levels a table can get (PLAN.md 1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Hash)]
#[serde(rename_all = "snake_case")]
pub enum CheckLevel {
    Structure,
    CountsAndTotals,
    FullValues,
}

impl std::fmt::Display for CheckLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            CheckLevel::Structure => "structure",
            CheckLevel::CountsAndTotals => "counts",
            CheckLevel::FullValues => "full values",
        };
        f.write_str(s)
    }
}

/// Why a table or column was skipped, with the required reason.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct IgnoredItem {
    pub path: String,
    pub reason: String,
}

/// The result for one table.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct TableResult {
    pub table: String,
    pub level: CheckLevel,
    pub verdict: Verdict,
    /// Old and new row counts, when both sides scanned.
    pub old_rows: Option<u64>,
    pub new_rows: Option<u64>,
    /// Columns covered by the comparison (in plan order).
    pub columns_covered: Option<Vec<String>>,
    /// Differing, missing and extra bucket counts from the comparison.
    pub differing_buckets: Option<usize>,
    pub missing_buckets: Option<usize>,
    pub extra_buckets: Option<usize>,
    /// Drill-down summary: missing, extra and changed row counts (≤100 changed
    /// rows carry column detail in [`RunResults::details`]).
    pub missing_rows: Option<usize>,
    pub extra_rows: Option<usize>,
    pub changed_rows: Option<usize>,
    /// Per-column warnings/flags and reasons this table did not PASS.
    pub reasons: Vec<String>,
}

/// One changed row with its differing columns (keys only; no non-key values).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ChangedRowDetail {
    pub key: Vec<Option<String>>,
    pub key_hash: String,
    pub columns: Vec<usize>,
}

/// One structure (inventory) difference, after the mapping is applied.  FAIL
/// for a red difference (missing object, invalid index, unvalidated
/// constraint, sequence behind, lost large object), WARNING for amber ("review").
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct StructureFinding {
    /// The object in dotted form (`schema.table.column`, `database.encoding`).
    pub object: String,
    /// The raw path from the schema diff (`["schema", s, "table", t, ...]`).
    pub path: Vec<String>,
    pub verdict: Verdict,
    pub message: String,
}

/// A suggested match for an unmapped table or column.  Suggestions are never
/// applied automatically (PLAN.md 4.3); a person must tick them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct MappingSuggestion {
    pub path: String,
    pub candidate: String,
    pub confidence: u8,
}

/// One changed row of the drill-down: the differing column names, and the key
/// only when the user ticked "export keys".
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ChangedRowExport {
    pub key: Option<Vec<Option<String>>>,
    pub columns: Vec<String>,
}

/// A control total that differs, by column (sums are aggregates, not row data).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ControlTotalExport {
    pub column: String,
    pub old: String,
    pub new: String,
}

/// Drill-down detail for one table.  Keys appear only when exported.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub struct TableDetail {
    /// Union of the columns that differ in the inspected changed rows.
    pub changed_columns: Vec<String>,
    pub changed_rows: Vec<ChangedRowExport>,
    pub missing_keys: Option<Vec<Vec<Option<String>>>>,
    pub extra_keys: Option<Vec<Vec<Option<String>>>>,
    pub control_totals: Vec<ControlTotalExport>,
    /// Plain-language explanations ("an empty text became NULL in column x").
    pub plain_words: Vec<String>,
}

/// The whole run's results.  This is the stable schema for UI, reports and hooks.
///
/// Fields added in results version 2 carry `#[serde(default)]`, so version-1
/// readers of the JSON are unaffected; Rust code building this struct can use
/// `..Default::default()`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct RunResults {
    /// The stable results schema version.
    pub results_version: u32,
    /// A run identifier that is stable across resumes.
    pub run_id: String,
    pub tool_version: String,
    pub mapping_hash: String,
    /// The overall verdict (see [`overall_verdict`] and `run::verdict`).
    pub verdict: Verdict,
    /// The PLAN.md 1 coverage line, see [`coverage_line`].
    pub coverage_line: String,
    /// Tables whose comparison at full values reached a conclusion.
    pub fully_checked: usize,
    /// Tables whose count-and-totals comparison reached a conclusion.
    pub counts_only: usize,
    pub ignored: usize,
    /// Per-table verdicts keyed by (old-side) table name.
    pub tables: BTreeMap<String, TableResult>,
    /// Skipped tables and columns with their reasons (first page of every report).
    pub ignored_items: Vec<IgnoredItem>,
    /// Changed-during-check detections (never PASS).
    pub changed_during_check: Vec<String>,
    /// Warnings and flags (SQL_ASCII, nothing checked, parallelism note, ...).
    pub warnings: Vec<String>,
    /// The write-ahead-log position at run start and end (version-branched).
    #[serde(default)]
    pub wal_start_lsn: Option<String>,
    #[serde(default)]
    pub wal_end_lsn: Option<String>,
    /// A SHA-256 of the canonical results JSON, set by [`finalise`].
    pub results_sha256: String,
    /// Tables whose structure check reached a conclusion (level Structure).
    #[serde(default)]
    pub structure_only: usize,
    /// Every table in scope: reported tables plus ignored tables.
    #[serde(default)]
    pub in_scope: usize,
    /// Structure (inventory) differences after the mapping is applied.
    #[serde(default)]
    pub structure_findings: Vec<StructureFinding>,
    /// Suggested matches for unmapped tables and columns.
    #[serde(default)]
    pub suggestions: Vec<MappingSuggestion>,
    /// Drill-down detail per table (only for tables with differences).
    #[serde(default)]
    pub details: BTreeMap<String, TableDetail>,
    /// The pre-run time estimate for the whole run, and per table.
    #[serde(default)]
    pub estimate: Option<Estimate>,
    #[serde(default)]
    pub table_estimates: BTreeMap<String, Estimate>,
    /// The measured time per table, in milliseconds (refines the estimate).
    #[serde(default)]
    pub table_elapsed_ms: BTreeMap<String, u64>,
    /// Tables whose stored result was reused on `--resume`.
    #[serde(default)]
    pub resumed_tables: Vec<String>,
    /// SHA-256 of the run definition (connections without passwords, levels,
    /// options, mapping hash) the results were produced from.
    #[serde(default)]
    pub definition_hash: String,
}

impl Default for RunResults {
    fn default() -> Self {
        RunResults {
            results_version: RESULTS_VERSION,
            run_id: String::new(),
            tool_version: crate::version().to_string(),
            mapping_hash: String::new(),
            verdict: Verdict::Warning,
            coverage_line: String::new(),
            fully_checked: 0,
            counts_only: 0,
            ignored: 0,
            tables: BTreeMap::new(),
            ignored_items: Vec::new(),
            changed_during_check: Vec::new(),
            warnings: Vec::new(),
            wal_start_lsn: None,
            wal_end_lsn: None,
            results_sha256: String::new(),
            structure_only: 0,
            in_scope: 0,
            structure_findings: Vec::new(),
            suggestions: Vec::new(),
            details: BTreeMap::new(),
            estimate: None,
            table_estimates: BTreeMap::new(),
            table_elapsed_ms: BTreeMap::new(),
            resumed_tables: Vec::new(),
            definition_hash: String::new(),
        }
    }
}

/// Results schema version this module writes.
pub const RESULTS_VERSION: u32 = 2;

/// The precedence order for combining verdicts: a run is PASS only when every
/// contributing verdict is PASS.  Any non-PASS verdict pulls the run down;
/// TOOL_FAULT is the gravest because a reader must never mistake a broken tool
/// for clean data.
pub fn severity(v: Verdict) -> u8 {
    match v {
        Verdict::Pass => 0,
        Verdict::Warning => 1,
        Verdict::NeedsReview => 2,
        Verdict::ChangedDuringCheck => 3,
        Verdict::WidelyDifferent => 4,
        Verdict::Fail => 5,
        Verdict::ToolFault => 6,
    }
}

/// Combine any number of verdicts.  The worst (highest severity) wins, so
/// TOOL_FAULT / FAIL / CHANGED_DURING_CHECK / NEEDS_REVIEW / WIDELY_DIFFERENT
/// all force the overall off PASS.
///
/// **Nothing to combine is not a PASS**: an empty input gives WARNING, because
/// PLAN.md 1 requires at least one table checked before a run can pass.
pub fn overall_verdict<'a>(verdicts: impl IntoIterator<Item = &'a Verdict>) -> Verdict {
    let mut worst: Option<Verdict> = None;
    for v in verdicts {
        worst = Some(worst.map_or(*v, |w| w.worst(*v)));
    }
    worst.unwrap_or(Verdict::Warning)
}

/// The coverage line exactly as the report spells it (PLAN.md 1), e.g.
/// "1480 of 1529 tables fully checked, 40 counts only, 0 structure only, 9 ignored".
pub fn coverage_line(
    in_scope: usize,
    fully_checked: usize,
    counts_only: usize,
    structure_only: usize,
    ignored: usize,
) -> String {
    format!(
        "{fully_checked} of {in_scope} tables fully checked, {counts_only} counts only, \
         {structure_only} structure only, {ignored} ignored"
    )
}

/// Generate a fresh, URL-safe run identifier.
pub fn fresh_run_id() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let pid = std::process::id();
    let mix = (nanos as u64) ^ u64::from(pid).wrapping_mul(0x9E37_79B9);
    format!("run-{nanos:x}-{pid:x}-{}", hex::encode(mix.to_le_bytes()))
}

/// Insert a table result, **fail-closed**: when the table is already present,
/// the worse verdict wins and the reasons are merged, so a later insert can
/// never overwrite an earlier FAIL.  Counters are not touched here; the run
/// builder derives them from the final verdicts.
pub fn add_table(results: &mut RunResults, table: TableResult) {
    match results.tables.get_mut(&table.table) {
        None => {
            results.tables.insert(table.table.clone(), table);
        }
        Some(existing) => {
            existing.verdict = existing.verdict.worst(table.verdict);
            for r in table.reasons {
                if !existing.reasons.contains(&r) {
                    existing.reasons.push(r);
                }
            }
            existing.old_rows = existing.old_rows.or(table.old_rows);
            existing.new_rows = existing.new_rows.or(table.new_rows);
            existing.columns_covered = existing.columns_covered.take().or(table.columns_covered);
            existing.differing_buckets = existing.differing_buckets.or(table.differing_buckets);
            existing.missing_buckets = existing.missing_buckets.or(table.missing_buckets);
            existing.extra_buckets = existing.extra_buckets.or(table.extra_buckets);
            existing.missing_rows = existing.missing_rows.or(table.missing_rows);
            existing.extra_rows = existing.extra_rows.or(table.extra_rows);
            existing.changed_rows = existing.changed_rows.or(table.changed_rows);
        }
    }
}

/// Recompute the coverage line from the counters.
pub fn apply_coverage(mut results: RunResults) -> RunResults {
    results.coverage_line = coverage_line(
        results.in_scope,
        results.fully_checked,
        results.counts_only,
        results.structure_only,
        results.ignored,
    );
    results
}

/// Stamp the final SHA-256 over the canonical results JSON (everything except
/// `results_sha256`).
pub fn finalise(mut results: RunResults) -> RunResults {
    results.results_sha256 = sha256_of(&results);
    results
}

/// SHA-256 of the results' canonical form, ignoring the `results_sha256` field
/// so the digest is self-verifying.
pub fn sha256_of(results: &RunResults) -> String {
    use sha2::Digest;
    let mut clone = results.clone();
    clone.results_sha256 = String::new();
    let canonical = serde_json::to_string(&clone).unwrap_or_default();
    hex::encode(sha2::Sha256::digest(canonical.as_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::compare::Verdict::*;

    fn table(name: &str, v: Verdict, reason: &str) -> TableResult {
        TableResult {
            table: name.into(),
            level: CheckLevel::FullValues,
            verdict: v,
            old_rows: None,
            new_rows: None,
            columns_covered: None,
            differing_buckets: None,
            missing_buckets: None,
            extra_buckets: None,
            missing_rows: None,
            extra_rows: None,
            changed_rows: None,
            reasons: vec![reason.into()],
        }
    }

    #[test]
    fn core_error_is_a_tool_fault() {
        assert_eq!(from_core(Error), Verdict::ToolFault);
        assert_eq!(from_core(Pass), Verdict::Pass);
        assert_eq!(from_core(Fail), Verdict::Fail);
        assert_eq!(from_core(WidelyDifferent), Verdict::WidelyDifferent);
        assert_eq!(from_core(NeedsReview), Verdict::NeedsReview);
    }

    #[test]
    fn overall_is_pass_only_when_everything_passes() {
        assert!(overall_verdict(&[Verdict::Pass, Verdict::Pass]).is_pass());
        for v in [
            Verdict::Fail,
            Verdict::WidelyDifferent,
            Verdict::NeedsReview,
            Verdict::ChangedDuringCheck,
            Verdict::ToolFault,
            Verdict::Warning,
        ] {
            assert!(
                !overall_verdict(&[Verdict::Pass, v]).is_pass(),
                "{v:?} must force the overall off PASS"
            );
        }
        // Precedence: ToolFault > Fail > WidelyDifferent > ChangedDuringCheck >
        // NeedsReview > Warning > Pass.
        assert_eq!(
            overall_verdict(&[Verdict::ChangedDuringCheck, Verdict::Fail]),
            Verdict::Fail
        );
        assert_eq!(
            overall_verdict(&[Verdict::Fail, Verdict::ToolFault]),
            Verdict::ToolFault
        );
        assert_eq!(
            overall_verdict(&[Verdict::NeedsReview, Verdict::ChangedDuringCheck]),
            Verdict::ChangedDuringCheck
        );
        assert_eq!(
            overall_verdict(&[Verdict::Warning, Verdict::NeedsReview]),
            Verdict::NeedsReview
        );
        assert_eq!(
            overall_verdict(&[Verdict::WidelyDifferent, Verdict::ChangedDuringCheck]),
            Verdict::WidelyDifferent
        );
    }

    #[test]
    fn nothing_to_combine_is_never_a_pass() {
        // Fix 1: an empty run used to fold to PASS.
        let none: [Verdict; 0] = [];
        assert_eq!(overall_verdict(&none), Verdict::Warning);
        assert!(!RunResults::default().verdict.is_pass());
    }

    #[test]
    fn add_table_is_fail_closed() {
        let mut r = RunResults::default();
        add_table(&mut r, table("s.t", Verdict::Fail, "column missing"));
        add_table(&mut r, table("s.t", Verdict::Pass, "data equal"));
        let t = &r.tables["s.t"];
        assert_eq!(
            t.verdict,
            Verdict::Fail,
            "a later PASS must not overwrite a FAIL"
        );
        assert_eq!(t.reasons, vec!["column missing", "data equal"]);
        add_table(&mut r, table("s.t", Verdict::ToolFault, "scan failed"));
        assert_eq!(r.tables["s.t"].verdict, Verdict::ToolFault);
    }

    #[test]
    fn verdicts_serialise_to_screaming_snake() {
        for (v, s) in [
            (Verdict::ChangedDuringCheck, "CHANGED_DURING_CHECK"),
            (Verdict::ToolFault, "TOOL_FAULT"),
            (Verdict::WidelyDifferent, "WIDELY_DIFFERENT"),
            (Verdict::NeedsReview, "NEEDS_REVIEW"),
            (Verdict::Pass, "PASS"),
            (Verdict::Fail, "FAIL"),
            (Verdict::Warning, "WARNING"),
        ] {
            assert_eq!(serde_json::to_value(v).unwrap(), serde_json::json!(s));
        }
    }

    #[test]
    fn coverage_line_shape() {
        assert_eq!(
            coverage_line(1529, 1480, 40, 0, 9),
            "1480 of 1529 tables fully checked, 40 counts only, 0 structure only, 9 ignored"
        );
    }

    #[test]
    fn version_one_json_still_parses() {
        let mut v = serde_json::to_value(RunResults::default()).unwrap();
        let o = v.as_object_mut().unwrap();
        for k in [
            "structure_only",
            "in_scope",
            "structure_findings",
            "suggestions",
            "details",
            "estimate",
            "table_estimates",
            "table_elapsed_ms",
            "resumed_tables",
            "definition_hash",
        ] {
            o.remove(k);
        }
        let back: RunResults = serde_json::from_value(v).unwrap();
        assert_eq!(back.structure_only, 0);
    }
}
