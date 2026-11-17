//! The run log (PLAN.md 4.7, 2.3).
//!
//! Records everything the plan lists — user, time, tool version, profiles (never
//! passwords), mapping version/hash, options ticked and the queries sent — **from
//! the results file where it is available, and notes each item that the results
//! file does not carry**.  `RunResults` holds the tool version, run ID, mapping
//! hash, verdict, coverage, warnings and WAL positions; it does not carry the
//! user name, wall-clock time, connection profiles, ticked options or the SQL
//! text, so those are emitted as explicit "not recorded in this results file"
//! lines rather than invented.

use std::fmt::Write as _;
use std::path::Path;

use serde::Serialize;

use dmux_engine::results::RunResults;

use crate::layout::{self, ReportOptions};

/// Errors writing the run log.
#[derive(Debug, thiserror::Error)]
pub enum LogError {
    #[error("cannot write log: {0}")]
    Io(#[from] std::io::Error),
    #[error("cannot serialize the log: {0}")]
    Json(#[from] serde_json::Error),
}

/// The serializable run-log record (a subset of the JSON log).
#[derive(Debug, Clone, Serialize)]
pub struct RunLog {
    pub tool_version: String,
    pub run_id: String,
    pub mapping_hash: String,
    pub verdict: String,
    pub coverage: String,
    pub results_sha256: String,
    /// Fields the results file does not carry, recorded as an explicit note so
    /// the log never invents a user, time, profile, option or query.
    pub user: String,
    pub time: String,
    pub profiles: String,
    pub options: String,
    pub queries: String,
    pub warnings: Vec<String>,
    pub changed_during_check: Vec<String>,
    /// SHA-256 of the run definition (connections without passwords, levels,
    /// options, mapping hash), when the results carry it (results schema v2).
    pub definition_hash: String,
}

const NOT_RECORDED: &str = "not recorded in this results file";

/// Build the log record from a results payload and its raw results JSON.
pub fn build_log(
    results: &RunResults,
    raw: &serde_json::Value,
    _options: &ReportOptions,
) -> RunLog {
    RunLog {
        tool_version: results.tool_version.clone(),
        run_id: results.run_id.clone(),
        mapping_hash: results.mapping_hash.clone(),
        verdict: layout::verdict_word(results.verdict).to_string(),
        coverage: results.coverage_line.clone(),
        results_sha256: results.results_sha256.clone(),
        user: NOT_RECORDED.to_string(),
        time: NOT_RECORDED.to_string(),
        profiles: NOT_RECORDED.to_string(),
        options: NOT_RECORDED.to_string(),
        queries: NOT_RECORDED.to_string(),
        warnings: results.warnings.clone(),
        changed_during_check: results.changed_during_check.clone(),
        definition_hash: crate::v2::definition_hash(raw),
    }
}

/// A human-readable, single-line-per-field log, deterministic for a given file.
pub fn text_log(results: &RunResults, raw: &serde_json::Value, options: &ReportOptions) -> String {
    let log = build_log(results, raw, options);
    let mut out = String::new();
    let _ = writeln!(out, "Dmux run log");
    let _ = writeln!(out, "tool_version: {}", log.tool_version);
    let _ = writeln!(out, "run_id: {}", log.run_id);
    let _ = writeln!(out, "mapping_hash: {}", log.mapping_hash);
    let _ = writeln!(out, "definition_hash: {}", log.definition_hash);
    let _ = writeln!(out, "verdict: {}", log.verdict);
    let _ = writeln!(out, "coverage: {}", log.coverage);
    let _ = writeln!(out, "results_sha256: {}", log.results_sha256);
    let _ = writeln!(out, "user: {}", log.user);
    let _ = writeln!(out, "time: {}", log.time);
    let _ = writeln!(out, "profiles: {}", log.profiles);
    let _ = writeln!(out, "options: {}", log.options);
    let _ = writeln!(out, "queries: {}", log.queries);
    if !log.warnings.is_empty() {
        let _ = writeln!(out, "warnings: {}", log.warnings.join("; "));
    }
    if !log.changed_during_check.is_empty() {
        let _ = writeln!(
            out,
            "changed_during_check: {}",
            log.changed_during_check.join("; ")
        );
    }
    out
}

/// Write the run log to `path`.  If the path ends in `.json`, write JSON;
/// otherwise write the plain-text form.
pub fn write_run_log(
    results: &RunResults,
    raw: &serde_json::Value,
    options: &ReportOptions,
    path: &Path,
) -> Result<(), LogError> {
    if path.extension().and_then(|e| e.to_str()) == Some("json") {
        let log = build_log(results, raw, options);
        let bytes = serde_json::to_vec_pretty(&log)?;
        std::fs::write(path, bytes)?;
    } else {
        let text = text_log(results, raw, options);
        std::fs::write(path, text)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use dmux_engine::results::{finalise, RunResults, Verdict, RESULTS_VERSION};
    use std::collections::BTreeMap;

    fn results() -> RunResults {
        finalise(RunResults {
            results_version: RESULTS_VERSION,
            run_id: "run-1".into(),
            tool_version: "0.1.0".into(),
            mapping_hash: "mh".into(),
            verdict: Verdict::Pass,
            coverage_line: "2 tables fully checked, 0 counts only, 0 ignored".into(),
            fully_checked: 2,
            counts_only: 0,
            ignored: 0,
            tables: BTreeMap::new(),
            ignored_items: vec![],
            changed_during_check: vec![],
            warnings: vec![],
            wal_start_lsn: None,
            wal_end_lsn: None,
            results_sha256: String::new(),
        })
    }

    fn raw() -> serde_json::Value {
        serde_json::Value::Null
    }

    #[test]
    fn log_records_available_fields_and_notes_the_rest() {
        let text = text_log(&results(), &raw(), &ReportOptions::default());
        assert!(text.contains("tool_version: 0.1.0"));
        assert!(text.contains("run_id: run-1"));
        assert!(text.contains("mapping_hash: mh"));
        assert!(text.contains("queries: not recorded in this results file"));
        assert!(text.contains("user: not recorded in this results file"));
    }

    #[test]
    fn log_is_deterministic() {
        let a = text_log(&results(), &raw(), &ReportOptions::default());
        let b = text_log(&results(), &raw(), &ReportOptions::default());
        assert_eq!(a, b);
    }

    #[test]
    fn json_log_serializes() {
        let log = build_log(&results(), &raw(), &ReportOptions::default());
        let json = serde_json::to_string(&log).unwrap();
        assert!(json.contains("\"tool_version\""));
        assert!(json.contains("\"run_id\""));
    }

    #[test]
    fn definition_hash_flows_from_raw() {
        let raw = serde_json::json!({ "definition_hash": "defhash" });
        let log = build_log(&results(), &raw, &ReportOptions::default());
        assert_eq!(log.definition_hash, "defhash");
    }
}
