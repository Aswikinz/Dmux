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

use dmux_engine::results::{self as results_mod, RunResults};

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
    /// the log never invents a user, time, profile or query.
    pub user: String,
    pub time: String,
    pub profiles: String,
    /// The ticked column options ("table.column: option"), or the honest
    /// "not recorded" note for an older results file (H2 / decision 23).
    pub options: String,
    pub queries: String,
    pub warnings: Vec<String>,
    pub changed_during_check: Vec<String>,
    /// SHA-256 of the run definition (results schema v2).
    pub definition_hash: String,
    /// Connection-safety summary lines (audit M5, H1).
    pub connections: Vec<String>,
    /// Hook script names + the SHA-256 of the code that ran (PLAN 4.9).
    pub hook_scripts: Vec<String>,
}

const NOT_RECORDED: &str = "not recorded in this results file";

/// The ticked options in "table.column: option" form (sorted), or the honest
/// "not recorded" note for a results file that predates option recording.
fn options_text(results: &RunResults) -> String {
    if !results_mod::options_recorded(results) {
        return layout::OPTIONS_NOT_RECORDED.to_string();
    }
    let mut lines = Vec::new();
    for (table, t) in &results.tables {
        for n in &t.column_notes {
            if n.kind == "option" {
                lines.push(format!("{table}.{}: {}", n.column, n.note));
            }
        }
    }
    lines.sort();
    if lines.is_empty() {
        "none".to_string()
    } else {
        lines.join("; ")
    }
}

/// The connection-safety summary ("side: ...") for the run (audit M5, H1).
fn connections_text(results: &RunResults) -> Vec<String> {
    layout::build(results, None, &ReportOptions::default()).connections
}

/// Build the log record from the typed results and optional hook evidence.
pub fn build_log(
    results: &RunResults,
    hooks: Option<&HookEvidence>,
    _options: &ReportOptions,
) -> RunLog {
    // The displayed verdict folds hook findings over the inner verdict, so a
    // `fail` custom check is never logged as PASS.  It also carries the
    // "(with options)" / "options not recorded" qualifier (never a bare PASS
    // that the file cannot vouch for).
    let inner =
        crate::verify::displayed_verdict(results, hooks.unwrap_or(&HookEvidence::default()));
    let verdict = layout::verdict_display(
        layout::verdict_word(inner),
        results.options_applied,
        results_mod::options_recorded(results),
    );
    RunLog {
        tool_version: results.tool_version.clone(),
        run_id: results.run_id.clone(),
        mapping_hash: results.mapping_hash.clone(),
        verdict,
        coverage: results.coverage_line.clone(),
        results_sha256: results.results_sha256.clone(),
        user: NOT_RECORDED.to_string(),
        time: NOT_RECORDED.to_string(),
        profiles: NOT_RECORDED.to_string(),
        options: options_text(results),
        queries: NOT_RECORDED.to_string(),
        warnings: results.warnings.clone(),
        changed_during_check: results.changed_during_check.clone(),
        definition_hash: results.definition_hash.clone(),
        connections: connections_text(results),
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
    for l in &log.connections {
        let _ = writeln!(out, "connection: {l}");
    }
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
        crate::write_private(path, &bytes)?;
    } else {
        let text = text_log(results, hooks, options);
        crate::write_private(path, text.as_bytes())?;
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

    #[test]
    fn an_old_results_file_logs_options_not_recorded_not_a_bare_pass() {
        let mut r = results();
        r.results_version = 2;
        r.options_applied = false;
        let r = finalise(r);
        let text = text_log(&r, None, &ReportOptions::default());
        assert!(
            text.contains("verdict: PASS (options not recorded — older results file)"),
            "{text}"
        );
        assert!(
            text.contains("options: options not recorded (older results file)"),
            "{text}"
        );
    }

    #[test]
    fn the_log_lists_applied_options_and_connections() {
        use dmux_engine::results::{ColumnNote, ConnectionInfo, Connections, TableResult};
        let mut r = results();
        r.options_applied = true;
        r.tables.insert(
            "s.t".into(),
            TableResult {
                table: "s.t".into(),
                level: dmux_engine::results::CheckLevel::FullValues,
                verdict: Verdict::Pass,
                old_rows: None,
                new_rows: None,
                columns_covered: None,
                differing_buckets: None,
                missing_buckets: None,
                extra_buckets: None,
                missing_rows: None,
                extra_rows: None,
                changed_rows: None,
                reasons: vec![],
                column_notes: vec![ColumnNote {
                    column: "balance".into(),
                    kind: "option".into(),
                    note: "round to 2 decimals".into(),
                }],
            },
        );
        r.connections = Connections {
            old: ConnectionInfo {
                tls: "plaintext".into(),
                plaintext_reason: Some("test network".into()),
                server_version: "17.0".into(),
                server_encoding: "UTF8".into(),
                ..ConnectionInfo::default()
            },
            new: ConnectionInfo::default(),
        };
        let r = finalise(r);
        let text = text_log(&r, None, &ReportOptions::default());
        assert!(text.contains("verdict: PASS (with options)"), "{text}");
        assert!(
            text.contains("options: s.t.balance: round to 2 decimals"),
            "{text}"
        );
        assert!(
            text.contains(
                "connection: old: plaintext (reason: test network); PostgreSQL 17.0, encoding UTF8"
            ),
            "{text}"
        );
    }
}
