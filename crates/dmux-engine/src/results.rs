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
use crate::core::normalise::{Flag, OptionKind};
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

/// One per-column note carried into the results (H2 / decision 23): a ticked
/// option or a type-change flag, in plain language.  `kind` is a stable
/// identifier for the flag; `note` is what the report prints.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ColumnNote {
    pub column: String,
    pub kind: String,
    pub note: String,
}

/// Connection-safety record for one side of a run (audit M5 / PLAN 2.3, 6.2,
/// 6.7).  Every field is a decision the signer must be able to see on page 1 of
/// the report: how the connection was secured, whether the login was a
/// superuser (and the recorded override), whether it could write, and what the
/// server actually was.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub struct ConnectionInfo {
    /// `"verified"`, `"pinned"` or `"plaintext"`; empty for a side that was not
    /// a live database (a saved fingerprint file).
    pub tls: String,
    /// The pinned certificate fingerprint when `tls == "pinned"`.
    #[serde(default)]
    pub tls_fingerprint: Option<String>,
    /// The recorded reason for a plaintext connection.
    #[serde(default)]
    pub plaintext_reason: Option<String>,
    pub superuser: bool,
    /// Why a superuser was allowed (the run's override reason).
    #[serde(default)]
    pub superuser_reason: Option<String>,
    pub can_write: bool,
    /// Fixed, plain-language reasons the login can write (no object names, no
    /// row data).  Empty when the login is read-only in every way checked.
    #[serde(default)]
    pub can_write_reasons: Vec<String>,
    pub pooler_suspected: bool,
    pub md5_available: bool,
    pub server_version: String,
    pub server_encoding: String,
    /// Whether the login has `BYPASSRLS` (PostgreSQL 9.5+; `false` on 9.4).
    #[serde(default)]
    pub rolbypassrls: bool,
    /// `schema.table` of every relation with row-level security enabled
    /// (PostgreSQL 9.5+; empty on 9.4).
    #[serde(default)]
    pub rls_tables: Vec<String>,
}

/// The connection-safety record for both sides (audit M5).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub struct Connections {
    pub old: ConnectionInfo,
    pub new: ConnectionInfo,
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
    /// Per-column notes (ticked options and type-change flags), in stable order
    /// (column, then kind).  Carried by the table result so a resumed table
    /// keeps its notes (the resume store persists `TableResult`).
    #[serde(default)]
    pub column_notes: Vec<ColumnNote>,
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

/// The rule one column was compared by, in plain words (no values).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ColumnRule {
    pub column: String,
    pub rule: String,
}

/// A control total for one column with both sides' values (sums, not rows).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ControlTotalFact {
    pub column: String,
    pub old: String,
    pub new: String,
}

/// The independent spot check for one table.  `state` is `agreed`,
/// `disagreed` or `not_run`; `note` is a plain-language reason (no values).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct SpotFact {
    pub state: String,
    pub compared: Option<usize>,
    pub differing: Option<usize>,
    pub note: String,
}

/// Facts about how one table was checked, recorded so the results can explain
/// a verdict (a PASS included).  Added after results version 3: every field
/// defaults, and an empty map is not serialised, so older files still load and
/// keep their digest.  Never carries row values.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub struct TableFacts {
    /// The key columns the rows were matched by (empty: compared as a counted set).
    #[serde(default)]
    pub key_columns: Vec<String>,
    /// Every compared column with its rule.
    #[serde(default)]
    pub columns: Vec<ColumnRule>,
    /// Buckets compared in total (equal, differing, missing and extra).
    #[serde(default)]
    pub buckets_total: Option<usize>,
    #[serde(default)]
    pub spot: Option<SpotFact>,
    /// Change detection on the old side: `available`, `track_counts_off` or
    /// `foreign_table`; `None` when it was not assessed.
    #[serde(default)]
    pub change_detection_old: Option<String>,
    #[serde(default)]
    pub change_detection_new: Option<String>,
    /// Whether the change counters moved during this table's scans.
    #[serde(default)]
    pub changed_during_scan: Option<bool>,
    /// Control totals for every summed column, both sides.
    #[serde(default)]
    pub control_totals: Vec<ControlTotalFact>,
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
    /// Connection-safety decisions per side (audit M5).
    #[serde(default)]
    pub connections: Connections,
    /// Whether any column option was applied anywhere (H2 / decision 23).  When
    /// the overall verdict is PASS and this is true, the report says
    /// "PASS (with options)".
    #[serde(default)]
    pub options_applied: bool,
    /// SHA-256 of the old-side capture file, present only when the old input
    /// was a saved fingerprint file (audit M1).
    #[serde(default)]
    pub capture_sha256_old: Option<String>,
    /// SHA-256 of the new-side capture file, present only when the new input
    /// was a saved fingerprint file (audit M1).
    #[serde(default)]
    pub capture_sha256_new: Option<String>,
    /// How each table was checked (rules, spot check, change detection, control
    /// totals), keyed like `tables`.  Not serialised when empty.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub table_facts: BTreeMap<String, TableFacts>,
    /// When the check started and finished, in UTC (RFC 3339, whole seconds),
    /// for the compliance record on every report.  Absent in files written
    /// before they were recorded; not serialised when absent, so those files'
    /// SHA-256 still verifies.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<String>,
    /// What each side was, in plain words: "bank on db1:5432 (login dmux)" for
    /// a live database or "saved capture file" for a fingerprint file.  Never
    /// a password.  Absent in older files.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub old_source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub new_source: Option<String>,
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
            connections: Connections::default(),
            options_applied: false,
            capture_sha256_old: None,
            capture_sha256_new: None,
            table_facts: BTreeMap::new(),
            started_at: None,
            finished_at: None,
            old_source: None,
            new_source: None,
        }
    }
}

/// Results schema version this module writes.
pub const RESULTS_VERSION: u32 = 3;

/// The results schema version at which per-column option notes (and the
/// `options_applied` flag) were first recorded.  A file with an older
/// `results_version` never recorded options, so a reader must say
/// "options not recorded" rather than assume none were applied (a bare PASS).
pub const OPTIONS_RECORDED_VERSION: u32 = 3;

/// Whether a results file recorded per-column options (H2 / decision 23).
/// Files with `results_version < OPTIONS_RECORDED_VERSION` carry no option
/// notes, so their options are unknown, never "none".
pub fn options_recorded(results: &RunResults) -> bool {
    results.results_version >= OPTIONS_RECORDED_VERSION
}

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
            for n in table.column_notes {
                if !existing.column_notes.contains(&n) {
                    existing.column_notes.push(n);
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

/// Plain-language notes for one column's planner flags (H2 / decision 23).
/// The caller collects notes across columns and sorts by (column, kind) so the
/// report order never depends on the server or on flag insertion order (rule 4).
pub fn column_notes(column: &str, flags: &[Flag]) -> Vec<ColumnNote> {
    flags.iter().map(|f| note_for(column, f)).collect()
}

/// The stable `kind` and plain-language `note` for one planner flag.
fn note_for(column: &str, flag: &Flag) -> ColumnNote {
    let (kind, note): (&str, String) = match flag {
        Flag::TypeChanged { old, new } => {
            ("type_changed", format!("type changed from {old} to {new}"))
        }
        Flag::JsonJsonbChange => (
            "json_jsonb",
            "json on one side, jsonb on the other (key order and duplicates are normalised)"
                .to_string(),
        ),
        Flag::TimestampTzChange => (
            "timestamp_tz",
            "timestamp without time zone on one side, with time zone on the other".to_string(),
        ),
        Flag::CharVarcharChange => (
            "char_varchar",
            "char(n) on one side, varchar or text on the other".to_string(),
        ),
        Flag::NumericScaleRaised { scale } => (
            "numeric_scale",
            format!("both sides cast up to numeric scale {scale}"),
        ),
        Flag::NumericScaleUnchecked => (
            "numeric_scale_unchecked",
            "one numeric side is unconstrained, so the printed forms are compared as stored"
                .to_string(),
        ),
        Flag::PrintedFormWarning { type_name } => (
            "printed_form",
            format!("type {type_name} is compared by a printed form that may depend on the server"),
        ),
        Flag::UnknownType { type_name } => (
            "unknown_type",
            format!("unrecognised type {type_name} is compared by its printed form"),
        ),
        Flag::OptionApplied(kind) => match kind {
            OptionKind::IgnoreTrailingSpaces => ("option", "ignore trailing spaces".to_string()),
            OptionKind::NullEqualsEmpty => ("option", "treat NULL as an empty value".to_string()),
            OptionKind::DateOnly => ("option", "compare the date part only".to_string()),
            OptionKind::Round(n) => ("option", format!("round to {n} decimals")),
        },
        Flag::DomainResolved { type_name } => (
            "domain_resolved",
            format!("domain {type_name} is compared by its base type"),
        ),
        Flag::NeedsReview { type_name } => (
            "needs_review",
            format!("type {type_name} is compared by a printed form that is not provably exact"),
        ),
    };
    ColumnNote {
        column: column.to_string(),
        kind: kind.to_string(),
        note,
    }
}

/// Whether any table applied a column option (H2): drives the "PASS (with
/// options)" wording on page 1 of the report.
pub fn any_options_applied(results: &RunResults) -> bool {
    results
        .tables
        .values()
        .flat_map(|t| &t.column_notes)
        .any(|n| n.kind == "option")
}

#[cfg(test)]
mod tests {

    #[test]
    fn results_without_table_facts_serialise_without_the_key_and_keep_their_digest() {
        // A results file written before table facts existed must load, and its
        // self-verifying digest must still match after a round trip.
        let r = finalise(RunResults::default());
        let json = serde_json::to_string(&r).unwrap();
        assert!(!json.contains("table_facts"));
        let back: RunResults = serde_json::from_str(&json).unwrap();
        assert!(back.table_facts.is_empty());
        assert_eq!(sha256_of(&back), back.results_sha256);
    }

    #[test]
    fn table_facts_round_trip_and_are_covered_by_the_digest() {
        let mut r = RunResults::default();
        r.table_facts.insert(
            "s.t".into(),
            TableFacts {
                key_columns: vec!["id".into()],
                columns: vec![ColumnRule {
                    column: "id".into(),
                    rule: "compared as an exact whole number".into(),
                }],
                buckets_total: Some(256),
                spot: Some(SpotFact {
                    state: "agreed".into(),
                    compared: Some(200),
                    differing: Some(0),
                    note: String::new(),
                }),
                change_detection_old: Some("available".into()),
                change_detection_new: Some("available".into()),
                changed_during_scan: Some(false),
                control_totals: vec![ControlTotalFact {
                    column: "amount".into(),
                    old: "1.50".into(),
                    new: "1.50".into(),
                }],
            },
        );
        let r = finalise(r);
        let back: RunResults = serde_json::from_str(&serde_json::to_string(&r).unwrap()).unwrap();
        assert_eq!(back, r);
        assert_eq!(sha256_of(&back), back.results_sha256);
        // Changing a recorded fact changes the digest.
        let mut tampered = back.clone();
        tampered.table_facts.get_mut("s.t").unwrap().buckets_total = Some(1);
        assert_ne!(sha256_of(&tampered), back.results_sha256);
    }

    #[test]
    fn table_facts_fields_all_default_so_a_partial_object_loads() {
        let f: TableFacts = serde_json::from_str("{}").unwrap();
        assert_eq!(f, TableFacts::default());
    }
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
            column_notes: vec![],
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
            "connections",
            "options_applied",
            "capture_sha256_old",
            "capture_sha256_new",
        ] {
            o.remove(k);
        }
        let back: RunResults = serde_json::from_value(v).unwrap();
        assert_eq!(back.structure_only, 0);
        assert!(!back.options_applied);
        assert_eq!(back.connections, Connections::default());
        assert_eq!(back.capture_sha256_old, None);
    }

    #[test]
    fn column_notes_map_every_flag_and_serialise() {
        use crate::core::normalise::PgType;
        let m = crate::core::normalise::ColumnMapping {
            old_name: "balance".into(),
            new_name: "balance".into(),
            old_type: PgType::Numeric(Some((18, 2))),
            new_type: PgType::Numeric(Some((18, 4))),
            options: crate::core::normalise::ColumnOptions {
                round_decimals: Some(2),
                ..Default::default()
            },
            control_total: false,
        };
        let plan = crate::core::normalise::plan_column(&m).unwrap();
        let mut notes = column_notes("balance", &plan.flags);
        notes.sort_by(|a, b| a.column.cmp(&b.column).then_with(|| a.kind.cmp(&b.kind)));
        assert!(notes
            .iter()
            .any(|n| n.kind == "option" && n.note == "round to 2 decimals"));
        assert!(notes.iter().any(
            |n| n.kind == "numeric_scale" && n.note == "both sides cast up to numeric scale 4"
        ));
        // A type change carries its note too (H2): the declared types differ.
        assert!(notes
            .iter()
            .any(|n| n.kind == "type_changed" && n.note.contains("type changed from")));
        // Stable order: within a column, notes sort by kind.
        assert!(notes.windows(2).all(|w| w[0].kind <= w[1].kind));

        let mut r = RunResults::default();
        add_table(
            &mut r,
            TableResult {
                column_notes: notes.clone(),
                ..table("s.t", Verdict::Pass, "data equal")
            },
        );
        assert!(any_options_applied(&r));
        let json = serde_json::to_string(&r).unwrap();
        assert!(json.contains("round to 2 decimals"));
    }

    #[test]
    fn add_table_merges_column_notes_fail_closed() {
        let mut r = RunResults::default();
        let note = ColumnNote {
            column: "c".into(),
            kind: "option".into(),
            note: "round to 2 decimals".into(),
        };
        add_table(
            &mut r,
            TableResult {
                column_notes: vec![note.clone()],
                ..table("s.t", Verdict::Fail, "column missing")
            },
        );
        add_table(
            &mut r,
            TableResult {
                column_notes: vec![note],
                ..table("s.t", Verdict::Pass, "data equal")
            },
        );
        assert_eq!(r.tables["s.t"].column_notes.len(), 1);
        assert_eq!(r.tables["s.t"].verdict, Verdict::Fail);
    }
}
