//! The run log (PLAN.md 4.7, 2.3).
//!
//! Records everything the plan lists — user, time, tool version, profiles (never
//! passwords), mapping version/hash, options ticked and the queries sent — **from
//! the results file where it is available, and notes each item that the results
//! file does not carry**.  `RunResults` holds the tool version, run ID, mapping
//! hash, verdict, coverage, warnings and WAL positions; it does not carry the
//! user name, wall-clock time, connection profiles, ticked options or the SQL
//! text, so those are emitted as explicit "not recorded in this results file"
//! lines rather than invented.  Hook script hashes are recorded when present.

use std::fmt::Write as _;
use std::path::Path;

use serde::Serialize;

use dmux_engine::results::RunResults;

use crate::layout::{self, ReportOptions};
use crate::verify::HookEvidence;

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
    /// SHA-256 of the run definition (results schema v2).
    pub definition_hash: String,
    /// Hook script names + the SHA-256 of the code that ran (PLAN 4.9).
    pub hook_scripts: Vec<String>,
}

const NOT_RECORDED: &str = "not recorded in this results file";

/// Build the log record from the typed results and optional hook evidence.
pub fn build_log(
    results: &RunResults,
    hooks: Option<&HookEvidence>,
    _options: &ReportOptions,
) -> RunLog {
    RunLog {
        tool_version: results.tool_version.clone(),
        run_id: results.run_id.clone(),
        mapping_hash: results.mapping_hash.clone(),
        // The displayed verdict folds hook findings over the inner verdict, so a
        // `fail` custom check is never logged as PASS.
        verdict: layout::verdict_word(crate::verify::displayed_verdict(
            results,
            hooks.unwrap_or(&HookEvidence::default()),
        ))
        .to_string(),
        coverage: results.coverage_line.clone(),
        results_sha256: results.results_sha256.clone(),
        user: NOT_RECORDED.to_string(),
        time: NOT_RECORDED.to_string(),
        profiles: NOT_RECORDED.to_string(),
        options: NOT_RECORDED.to_string(),
        queries: NOT_RECORDED.to_string(),
        warnings: results.warnings.clone(),
        changed_during_check: results.changed_during_check.clone(),
        definition_hash: results.definition_hash.clone(),
        hook_scripts: hooks
            .map(|h| {
                h.scripts
                    .iter()
                    .map(|s| format!("{} {}", s.script, s.sha256))
                    .collect()
            })
            .unwrap_or_default(),
    }
}

/// A human-readable, single-line-per-field log, deterministic for a given file.
pub fn text_log(
    results: &RunResults,
    hooks: Option<&HookEvidence>,
    options: &ReportOptions,
) -> String {
    let log = build_log(results, hooks, options);
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
    for s in &log.hook_scripts {
        let _ = writeln!(out, "hook_script: {s}");
    }
    out
}

/// Write the run log to `path`.  If the path ends in `.json`, write JSON;
/// otherwise write the plain-text form.
pub fn write_run_log(
    results: &RunResults,
    hooks: Option<&HookEvidence>,
    options: &ReportOptions,
    path: &Path,
) -> Result<(), LogError> {
    if path.extension().and_then(|e| e.to_str()) == Some("json") {
        let log = build_log(results, hooks, options);
        let bytes = serde_json::to_vec_pretty(&log)?;
        std::fs::write(path, bytes)?;
    } else {
        let text = text_log(results, hooks, options);
        std::fs::write(path, text)?;
    }
    Ok(())
}

/// Alias kept for the shared `render_all` path (log is built from the parsed
/// results + evidence there).
pub fn write_run_log_locked(
    results: &RunResults,
    hooks: Option<&HookEvidence>,
    options: &ReportOptions,
    path: &Path,
) -> Result<(), LogError> {
    write_run_log(results, hooks, options, path)
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
            coverage_line:
                "2 of 2 tables fully checked, 0 counts only, 0 structure only, 0 ignored".into(),
            fully_checked: 2,
            counts_only: 0,
            ignored: 0,
            tables: BTreeMap::new(),
            ..RunResults::default()
        })
    }

    #[test]
    fn log_records_available_fields_and_notes_the_rest() {
        let text = text_log(&results(), None, &ReportOptions::default());
        assert!(text.contains("tool_version: 0.1.0"));
        assert!(text.contains("run_id: run-1"));
        assert!(text.contains("mapping_hash: mh"));
        assert!(text.contains("queries: not recorded in this results file"));
        assert!(text.contains("user: not recorded in this results file"));
    }

    #[test]
    fn log_is_deterministic() {
        let a = text_log(&results(), None, &ReportOptions::default());
        let b = text_log(&results(), None, &ReportOptions::default());
        assert_eq!(a, b);
    }

    #[test]
    fn json_log_serializes() {
        let log = build_log(&results(), None, &ReportOptions::default());
        let json = serde_json::to_string(&log).unwrap();
        assert!(json.contains("\"tool_version\""));
        assert!(json.contains("\"run_id\""));
    }

    #[test]
    fn definition_hash_flows_from_typed_results() {
        let mut r = results();
        r.definition_hash = "defhash".into();
        let r = finalise(r);
        let log = build_log(&r, None, &ReportOptions::default());
        assert_eq!(log.definition_hash, "defhash");
    }

    #[test]
    fn hook_script_hashes_are_recorded() {
        use crate::verify::HookScript;
        let hooks = HookEvidence {
            scripts: vec![HookScript {
                script: "hook.py".into(),
                sha256: "ab".repeat(32),
            }],
            ..Default::default()
        };
        let text = text_log(&results(), Some(&hooks), &ReportOptions::default());
        assert!(text.contains("hook.py"), "{text}");
        assert!(text.contains(&"ab".repeat(32)));
    }
}
