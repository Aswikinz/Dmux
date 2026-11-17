//! The results model: per-table and run-level verdicts, the coverage line and
//! the stable JSON schema the CLI, the UI and the report hooks share.
//!
//! This module turns the comparison core's verdicts into the run-level verdict
//! vocabulary of PLAN.md 1 and docs/how-dmux-decides.md.  It never passes a run
//! unless every in-scope table passed at its own level and nothing was skipped,
//! changed, or faulted without a recorded reason.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use crate::core::compare::Verdict as CoreVerdict;

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
    /// A run-level warning (something to look at), never a data PASS on its own.
    Warning,
}

impl Verdict {
    pub fn is_pass(self) -> bool {
        self == Verdict::Pass
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

/// Why a table was skipped, with the required reason.
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
    /// Columns covered by the fingerprint (in mapping order).
    pub columns_covered: Option<Vec<String>>,
    /// Differing, missing and extra bucket counts from the comparison.
    pub differing_buckets: Option<usize>,
    pub missing_buckets: Option<usize>,
    pub extra_buckets: Option<usize>,
    /// Drill-down summary: missing, extra and changed row counts (≤100 changed
    /// rows carry detail).
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

/// The whole run's results.  This is the stable schema for UI, reports and hooks.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct RunResults {
    /// The stable results schema version.
    pub results_version: u32,
    /// A run identifier that is stable across resumes.
    pub run_id: String,
    pub tool_version: String,
    pub mapping_hash: String,
    /// The overall verdict, computed by [`overall_verdict`].
    pub verdict: Verdict,
    /// The PLAN.md 1 coverage line: "N of M tables fully checked, X counts only, Y ignored".
    pub coverage_line: String,
    pub fully_checked: usize,
    pub counts_only: usize,
    pub ignored: usize,
    /// Per-table verdicts keyed by table name.
    pub tables: BTreeMap<String, TableResult>,
    /// Skipped tables with their reasons (the first page of every report).
    pub ignored_items: Vec<IgnoredItem>,
    /// Changed-during-check detections (never PASS).
    pub changed_during_check: Vec<String>,
    /// Warnings and flags (SQL_ASCII, can-write, pooler, needs-review columns).
    pub warnings: Vec<String>,
    /// The write-ahead-log position at run start and end (version-branched
    /// function); a purely informational record that a change happened during
    /// the run.  `None` when unavailable (e.g. no live database).
    #[serde(default)]
    pub wal_start_lsn: Option<String>,
    #[serde(default)]
    pub wal_end_lsn: Option<String>,
    /// A SHA-256 of the canonical results JSON, set by [`Self::finalise`].
    pub results_sha256: String,
}

/// Results schema version this module writes.
pub const RESULTS_VERSION: u32 = 1;

/// The precedence order for combining run-level verdicts: a run is PASS only
/// when every contributing verdict is PASS.  Any non-PASS verdict pulls the run
/// down; TOOL_FAULT is the gravest because a reader must never mistake a broken
/// tool for clean data.
fn severity(v: Verdict) -> u8 {
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
pub fn overall_verdict<'a>(verdicts: impl IntoIterator<Item = &'a Verdict>) -> Verdict {
    let mut worst = Verdict::Pass;
    for v in verdicts {
        if severity(*v) > severity(worst) {
            worst = *v;
        }
    }
    worst
}

/// Compute the coverage line exactly as the report spells it (PLAN.md 1).
pub fn coverage_line(fully_checked: usize, counts_only: usize, ignored: usize) -> String {
    format!("{fully_checked} tables fully checked, {counts_only} counts only, {ignored} ignored")
}

/// Generate a fresh, random, URL-safe run identifier.
pub fn fresh_run_id() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let pid = std::process::id();
    let mut extra = [0u8; 8];
    // A deterministic-but-unique suffix using the process id and time; the
    // run ID is not a secret, just a stable handle.
    extra.copy_from_slice(&(nanos as u64 ^ (pid as u64).wrapping_mul(0x9E37_79B9)).to_le_bytes());
    format!("run-{nanos:x}-{pid:x}-{}", hex::encode(extra))
}

/// Add a table result and update the fully-checked / counts-only counters.
pub fn add_table(results: &mut RunResults, table: TableResult) {
    match table.level {
        CheckLevel::FullValues => results.fully_checked += 1,
        CheckLevel::CountsAndTotals => results.counts_only += 1,
        CheckLevel::Structure => {}
    }
    results.tables.insert(table.table.clone(), table);
}

/// Recompute the coverage line from the counters.
pub fn apply_coverage(mut results: RunResults) -> RunResults {
    results.coverage_line =
        coverage_line(results.fully_checked, results.counts_only, results.ignored);
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
        ] {
            assert!(
                !overall_verdict(&[Verdict::Pass, v]).is_pass(),
                "{v:?} must force the overall off PASS"
            );
        }
        // WARNING alone is not a data PASS: the caller must decide, but it does
        // not equal Pass.
        assert!(!overall_verdict(&[Verdict::Warning]).is_pass());
        // Precedence: ToolFault beats Fail beats ChangedDuringCheck, etc.
        assert_eq!(
            overall_verdict(&[Verdict::ChangedDuringCheck, Verdict::Fail]),
            Verdict::Fail
        );
        assert_eq!(
            overall_verdict(&[Verdict::Fail, Verdict::ToolFault]),
            Verdict::ToolFault
        );
    }

    #[test]
    fn verdicts_serialise_to_screaming_snake() {
        assert_eq!(
            serde_json::to_value(Verdict::ChangedDuringCheck).unwrap(),
            serde_json::json!("CHANGED_DURING_CHECK")
        );
        assert_eq!(
            serde_json::to_value(Verdict::ToolFault).unwrap(),
            serde_json::json!("TOOL_FAULT")
        );
        assert_eq!(
            serde_json::to_value(Verdict::WidelyDifferent).unwrap(),
            serde_json::json!("WIDELY_DIFFERENT")
        );
        assert_eq!(
            serde_json::to_value(Verdict::NeedsReview).unwrap(),
            serde_json::json!("NEEDS_REVIEW")
        );
        assert_eq!(
            serde_json::to_value(Verdict::Pass).unwrap(),
            serde_json::json!("PASS")
        );
        assert_eq!(
            serde_json::to_value(Verdict::Fail).unwrap(),
            serde_json::json!("FAIL")
        );
        assert_eq!(
            serde_json::to_value(Verdict::Warning).unwrap(),
            serde_json::json!("WARNING")
        );
    }

    #[test]
    fn coverage_line_shape() {
        assert_eq!(
            coverage_line(1480, 40, 9),
            "1480 tables fully checked, 40 counts only, 9 ignored"
        );
    }
}
