//! Report export from the desktop app (PLAN 4.7, T059).
//!
//! The report is rendered from the **engine-held results of a finished run**,
//! looked up by run id in app state.  Nothing the webview sends is rendered
//! (audit L7): the webview names only the run, the format and whether keys are
//! included.  The results go through the same verification as `dmux report`
//! (`dmux_report::verify`), so a results object whose digest does not match,
//! or whose PASS contradicts a table, is refused.
//!
//! The destination is chosen with the native **save** dialog, opened from
//! Rust.  The webview therefore never names a path, and has no dialog or
//! filesystem permission of its own; the Rust side writes the file, mode 0600
//! on Unix (`dmux_report`'s private writer).

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tauri::State;

use dmux_report::layout::Layout;
use dmux_report::{LeaveOut, ReportOptions};

use crate::error::CommandError;
use crate::state::AppState;

/// The export formats the Report screen offers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReportFormat {
    Pdf,
    Excel,
    /// The interactive HTML report: one offline file with filters and sorting.
    Html,
}

impl ReportFormat {
    fn extension(self) -> &'static str {
        match self {
            ReportFormat::Pdf => "pdf",
            ReportFormat::Excel => "xlsx",
            ReportFormat::Html => "html",
        }
    }

    fn filter_name(self) -> &'static str {
        match self {
            ReportFormat::Pdf => "PDF report",
            ReportFormat::Excel => "Excel workbook",
            ReportFormat::Html => "HTML report",
        }
    }
}

/// What the Report screen asks for.  There is deliberately no path field.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct ExportReportInput {
    pub run_id: String,
    pub format: ReportFormat,
    /// Include key / covered-column names.  Off unless the user ticks it.
    #[serde(default)]
    pub include_keys: bool,
    /// Parts the user chose to leave out.  Data differences cannot be left
    /// out (there is no field for them); page 1 names whatever was.
    #[serde(default)]
    pub leave_out: LeaveOutInput,
}

/// What the Report screen may leave out of a report.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct LeaveOutInput {
    #[serde(default)]
    pub structure: bool,
    #[serde(default)]
    pub other: bool,
    #[serde(default)]
    pub warnings: bool,
    #[serde(default)]
    pub passing: bool,
}

impl From<LeaveOutInput> for LeaveOut {
    fn from(l: LeaveOutInput) -> Self {
        LeaveOut {
            structure: l.structure,
            other: l.other,
            warnings: l.warnings,
            passing: l.passing,
        }
    }
}

/// The user's report choices: row keys, and what to leave out.  A plain
/// `bool` means keys on or off with nothing left out.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ReportChoices {
    pub include_keys: bool,
    pub leave_out: LeaveOut,
}

impl From<bool> for ReportChoices {
    fn from(include_keys: bool) -> Self {
        ReportChoices {
            include_keys,
            leave_out: LeaveOut::default(),
        }
    }
}

impl From<&ExportReportInput> for ReportChoices {
    fn from(input: &ExportReportInput) -> Self {
        ReportChoices {
            include_keys: input.include_keys,
            leave_out: input.leave_out.into(),
        }
    }
}

/// The export outcome: the saved path, or that the dialog was dismissed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum ExportReportResult {
    Saved { path: String },
    Cancelled,
}

/// The verified results of a run id: a live run's own results (re-verified), or
/// results the app verified when opening a history entry or a results file
/// (registered under a fresh id).  Never anything the webview sent.
pub(crate) fn verification_for(
    run_id: &str,
    state: &AppState,
) -> Result<dmux_report::Verification, CommandError> {
    if state.runs.phase(run_id).is_none() {
        if let Some(opened) = state.opened_results(run_id)? {
            return Ok(opened.to_verification());
        }
    }
    let results = state.runs.results(run_id)?;
    // When custom checks ran, render from their envelope (the results plus the
    // findings and the script hashes), verified like `dmux report` verifies a
    // file written by `dmux run --hooks`.
    let verified = match state.runs.hook_run(run_id)? {
        Some(hooks) => dmux_report::verify::verify_value(&hooks.envelope),
        None => dmux_report::verify::verify_results(&results),
    };
    verified.map_err(|e| CommandError::Report {
        message: format!("the run's results cannot be reported: {e}"),
    })
}

/// The verified layout and digest for a finished run, from app state only.
/// A run that is unknown, still running, stopped or failed is refused with
/// the run manager's typed error.
pub(crate) fn verified_layout(
    run_id: &str,
    choices: impl Into<ReportChoices>,
    state: &AppState,
) -> Result<(Layout, String), CommandError> {
    let choices = choices.into();
    let verification = verification_for(run_id, state)?;
    let layout = dmux_report::build_layout(
        &verification,
        &ReportOptions {
            include_keys: choices.include_keys,
            generated_at: Some(dmux_report::layout::now_utc()),
            leave_out: choices.leave_out,
        },
    );
    Ok((layout, verification.sha256))
}

/// Give `path` the format's extension when the user left it off.
fn with_extension(path: PathBuf, format: ReportFormat) -> PathBuf {
    let has = path
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case(format.extension()));
    if has {
        path
    } else {
        let mut name = path.into_os_string();
        name.push(".");
        name.push(format.extension());
        PathBuf::from(name)
    }
}

/// Render one report for a finished run to `path` (blocking file I/O).
pub(crate) fn render_report(
    run_id: &str,
    format: ReportFormat,
    choices: impl Into<ReportChoices>,
    state: &AppState,
    path: &Path,
) -> Result<(), CommandError> {
    let (layout, sha256) = verified_layout(run_id, choices, state)?;
    let failed = |e: String| CommandError::Report {
        message: format!("the report could not be written: {e}"),
    };
    match format {
        ReportFormat::Pdf => dmux_report::pdf::write_summary_layout(&layout, &sha256, path)
            .map_err(|e| failed(e.to_string())),
        ReportFormat::Excel => dmux_report::xlsx::write_detail_layout(&layout, &sha256, path)
            .map_err(|e| failed(e.to_string())),
        ReportFormat::Html => dmux_report::html::write_html_layout(&layout, &sha256, path)
            .map_err(|e| failed(e.to_string())),
    }
}

/// A suggested file name: the run id is app-generated (`run-<hex>-...`), so
/// it is safe in a name; anything else falls back to a fixed name.
fn suggested_name(run_id: &str, format: ReportFormat) -> String {
    let safe = !run_id.is_empty()
        && run_id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-');
    let stem = if safe { run_id } else { "report" };
    format!("dmux-{stem}.{}", format.extension())
}

/// Export the current run's report as PDF, Excel or HTML through a native save
/// dialog.  The run is checked before the dialog opens, so a run that cannot
/// be reported never asks for a location.
#[tauri::command]
pub async fn export_report<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    input: ExportReportInput,
    state: State<'_, AppState>,
) -> Result<ExportReportResult, CommandError> {
    use tauri::Manager;
    use tauri_plugin_dialog::DialogExt;

    // Refuse early: unknown, unfinished, stopped or failed runs, and results
    // that do not verify.
    let choices = ReportChoices::from(&input);
    verified_layout(&input.run_id, choices, &state)?;

    let format = input.format;
    let dialog = app
        .dialog()
        .file()
        .set_title("Save the Dmux report")
        .set_file_name(suggested_name(&input.run_id, format))
        .add_filter(format.filter_name(), &[format.extension()]);
    let chosen = tauri::async_runtime::spawn_blocking(move || dialog.blocking_save_file())
        .await
        .map_err(|e| CommandError::internal(format!("the save dialog failed: {e}")))?;
    let Some(chosen) = chosen else {
        return Ok(ExportReportResult::Cancelled);
    };
    let path = chosen.into_path().map_err(|_| CommandError::Report {
        message: "the chosen location is not a local file".to_string(),
    })?;
    let path = with_extension(path, format);

    let app2 = app.clone();
    let run_id = input.run_id.clone();
    let target = path.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let state = app2.state::<AppState>();
        render_report(&run_id, format, choices, &state, &target)
    })
    .await
    .map_err(|e| CommandError::internal(format!("the report task failed: {e}")))??;
    Ok(ExportReportResult::Saved {
        path: path.display().to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::run_manager::{RunHandle, RunManager, RunPhase};
    use dmux_engine::results::{
        finalise, CheckLevel, RunResults, TableResult, Verdict, RESULTS_VERSION,
    };
    use dmux_engine::run::{Input, RunConfig};
    use std::collections::BTreeMap;

    fn state() -> AppState {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().to_path_buf();
        std::mem::forget(dir);
        AppState::with_password_store(path, Box::new(crate::secrets::InMemoryPasswordStore::new()))
            .expect("state")
    }

    fn config() -> RunConfig {
        let profile = || {
            Input::Live(dmux_engine::db::ConnProfile::new(
                "127.0.0.1",
                1,
                "unused",
                "unused",
                String::new(),
                dmux_engine::db::TlsMode::Disable("report test".to_string()),
            ))
        };
        RunConfig::new(profile(), profile())
    }

    /// Covered-column names stand in for keys: they appear only with keys on.
    const KEY_SENTINEL: &str = "acct_key_sentinel_col";

    fn results(verdict: Verdict) -> RunResults {
        let table = TableResult {
            table: "demo_bank.accounts".into(),
            level: CheckLevel::FullValues,
            verdict,
            old_rows: Some(3),
            new_rows: Some(3),
            columns_covered: Some(vec![KEY_SENTINEL.into(), "balance".into()]),
            differing_buckets: Some(0),
            missing_buckets: Some(0),
            extra_buckets: Some(0),
            missing_rows: Some(0),
            extra_rows: Some(0),
            changed_rows: Some(0),
            reasons: vec![],
            column_notes: vec![],
        };
        finalise(RunResults {
            results_version: RESULTS_VERSION,
            run_id: "run-1".into(),
            tool_version: "0.3.0".into(),
            mapping_hash: "mh".into(),
            verdict,
            coverage_line:
                "1 of 1 tables fully checked, 0 counts only, 0 structure only, 0 ignored".into(),
            fully_checked: 1,
            tables: BTreeMap::from([("demo_bank.accounts".into(), table)]),
            ..RunResults::default()
        })
    }

    /// A run in app state that finished with `outcome`.
    fn finished(
        state: &AppState,
        outcome: Result<RunResults, dmux_engine::run::RunError>,
    ) -> String {
        let id = RunManager::fresh_id();
        state.runs.insert(&id, RunHandle::new(config(), None));
        let t = state.runs.begin_first(&id).expect("ticket");
        state.runs.finish(&id, t.generation, outcome);
        id
    }

    fn tmp(name: &str) -> PathBuf {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join(name);
        std::mem::forget(dir);
        p
    }

    #[test]
    fn renders_pdf_and_excel_from_the_runs_own_results() {
        let s = state();
        let id = finished(&s, Ok(results(Verdict::Pass)));
        assert_eq!(s.runs.phase(&id), Some(RunPhase::Done));
        for format in [ReportFormat::Pdf, ReportFormat::Excel, ReportFormat::Html] {
            let path = tmp(&format!("r.{}", format.extension()));
            render_report(&id, format, false, &s, &path).expect("render");
            let bytes = std::fs::read(&path).unwrap();
            match format {
                ReportFormat::Pdf => assert!(bytes.starts_with(b"%PDF")),
                ReportFormat::Excel => assert!(bytes.starts_with(b"PK")),
                ReportFormat::Html => assert!(bytes.starts_with(b"<!doctype html>")),
            }
        }
        // The layout comes from the engine-held results.
        let (layout, sha) = verified_layout(&id, false, &s).unwrap();
        assert_eq!(layout.verdict_word, "PASS");
        assert_eq!(sha, results(Verdict::Pass).results_sha256);
    }

    #[cfg(unix)]
    #[test]
    fn the_report_file_is_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let s = state();
        let id = finished(&s, Ok(results(Verdict::Fail)));
        for format in [ReportFormat::Pdf, ReportFormat::Excel, ReportFormat::Html] {
            let path = tmp(&format!("m.{}", format.extension()));
            render_report(&id, format, false, &s, &path).unwrap();
            let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600, "{format:?}");
        }
    }

    /// A run whose custom checks (a failing finding) were recorded, as the
    /// drive task does it: plan, then outcome for the current attempt, then
    /// finish.
    fn finished_with_hooks(state: &AppState, results: RunResults) -> String {
        use crate::hooks::{run_custom_checks, HookPlan};
        use dmux_hooks::{Finding, HookResult, Severity};
        let id = RunManager::fresh_id();
        state.runs.insert(&id, RunHandle::new(config(), None));
        state.runs.set_hook_plan(
            &id,
            Some(HookPlan {
                folder: None,
                interpreter: None,
                timeout: std::time::Duration::from_secs(1),
            }),
        );
        let ticket = state.runs.begin_first(&id).expect("ticket");
        // Build the envelope from a known finding (no Python needed here).
        let mut run = run_custom_checks(state.runs.hook_plan(&id).as_ref().unwrap(), &results);
        let hook = HookResult {
            script_hash: "f".repeat(64),
            findings: vec![Finding {
                id: "balance-sum".into(),
                severity: Severity::Fail,
                table: Some("demo_bank.accounts".into()),
                message: "totals differ by 12".into(),
            }],
            warnings: vec![],
        };
        let mut folded = results.clone();
        folded.verdict = Verdict::Fail;
        folded.results_sha256 = dmux_engine::results::sha256_of(&folded);
        let envelope = dmux_hooks::build_envelope(
            &serde_json::to_value(&folded).unwrap(),
            &[("balance.py".to_string(), &hook)],
        );
        let v = dmux_report::verify::verify_value(&envelope).unwrap();
        run.results = v.results;
        run.evidence = v.hooks.unwrap();
        run.envelope = envelope;
        state.runs.set_hook_run(&id, ticket.generation, run);
        state.runs.finish(&id, ticket.generation, Ok(results));
        id
    }

    #[test]
    fn the_report_includes_the_custom_check_evidence_and_never_shows_pass() {
        let s = state();
        let id = finished_with_hooks(&s, results(Verdict::Pass));
        let (layout, digest) = verified_layout(&id, false, &s).expect("layout");
        assert_eq!(layout.verdict_word, "FAIL");
        assert_eq!(layout.custom_checks.len(), 1);
        assert_eq!(layout.custom_checks[0].id, "balance-sum");
        assert_eq!(layout.hook_scripts.len(), 1);
        assert_eq!(layout.hook_scripts[0].sha256, "f".repeat(64));
        // The digest on the report is the envelope's, covering the evidence.
        let env = s.runs.hook_run(&id).unwrap().unwrap().envelope;
        assert_eq!(digest, env["envelope_sha256"].as_str().unwrap());
        // And it renders to a file.
        let path = tmp("hooks.pdf");
        render_report(&id, ReportFormat::Pdf, false, &s, &path).expect("render");
        assert!(path.metadata().unwrap().len() > 0);
    }

    #[test]
    fn a_run_without_custom_checks_reports_as_before() {
        let s = state();
        let id = finished(&s, Ok(results(Verdict::Pass)));
        let (layout, _) = verified_layout(&id, false, &s).unwrap();
        assert!(layout.custom_checks.is_empty());
        assert_eq!(layout.verdict_word, "PASS");
    }

    #[test]
    fn a_run_that_should_have_custom_checks_but_has_none_is_not_reported() {
        let s = state();
        let id = RunManager::fresh_id();
        s.runs.insert(&id, RunHandle::new(config(), None));
        s.runs.set_hook_plan(
            &id,
            Some(crate::hooks::HookPlan {
                folder: None,
                interpreter: None,
                timeout: std::time::Duration::from_secs(1),
            }),
        );
        let t = s.runs.begin_first(&id).unwrap();
        s.runs.finish(&id, t.generation, Ok(results(Verdict::Pass)));
        assert!(matches!(
            verified_layout(&id, false, &s),
            Err(CommandError::ResultsNotAvailable { .. })
        ));
    }

    #[test]
    fn a_run_that_is_not_done_or_failed_is_refused_with_a_typed_error() {
        let s = state();
        let path = tmp("x.pdf");
        // Unknown run.
        assert!(matches!(
            render_report("run-nope", ReportFormat::Pdf, false, &s, &path),
            Err(CommandError::RunNotFound { .. })
        ));
        // Still running.
        let id = RunManager::fresh_id();
        s.runs.insert(&id, RunHandle::new(config(), None));
        s.runs.begin_first(&id).unwrap();
        assert!(matches!(
            render_report(&id, ReportFormat::Pdf, false, &s, &path),
            Err(CommandError::ResultsNotAvailable { .. })
        ));
        // Failed.
        let id = finished(&s, Err(dmux_engine::run::RunError::Config("boom".into())));
        assert_eq!(s.runs.phase(&id), Some(RunPhase::Failed));
        assert!(matches!(
            render_report(&id, ReportFormat::Excel, false, &s, &path),
            Err(CommandError::RunFailed { .. })
        ));
        // Cancelled.
        let id = RunManager::fresh_id();
        s.runs.insert(&id, RunHandle::new(config(), None));
        s.runs.begin_first(&id).unwrap();
        s.runs
            .transition(&id, &[RunPhase::Running], RunPhase::Cancelled)
            .unwrap();
        assert!(matches!(
            render_report(&id, ReportFormat::Pdf, false, &s, &path),
            Err(CommandError::RunStopped { .. })
        ));
        assert!(!path.exists(), "nothing is written for a refused run");
    }

    #[test]
    fn results_that_do_not_verify_are_refused() {
        let s = state();
        // Edited after finalising: the digest no longer matches.
        let mut edited = results(Verdict::Fail);
        edited.verdict = Verdict::Pass;
        let id = finished(&s, Ok(edited));
        let err = render_report(&id, ReportFormat::Pdf, false, &s, &tmp("t.pdf")).unwrap_err();
        assert!(matches!(err, CommandError::Report { .. }), "{err:?}");
        // Never finalised.
        let mut raw = results(Verdict::Pass);
        raw.results_sha256 = String::new();
        let id = finished(&s, Ok(raw));
        assert!(matches!(
            verified_layout(&id, false, &s),
            Err(CommandError::Report { .. })
        ));
    }

    #[test]
    fn what_the_user_leaves_out_reaches_the_report_but_never_the_data() {
        let input: ExportReportInput = serde_json::from_str(
            r#"{"run_id":"r","format":"pdf","leave_out":{"structure":true,"passing":true}}"#,
        )
        .unwrap();
        let choices = ReportChoices::from(&input);
        assert!(choices.leave_out.structure && choices.leave_out.passing);
        assert!(!choices.leave_out.other && !choices.leave_out.warnings);
        // There is no way to name data differences: an unknown key is refused.
        assert!(serde_json::from_str::<ExportReportInput>(
            r#"{"run_id":"r","format":"pdf","leave_out":{"data":true}}"#
        )
        .is_err());
        // Absent, nothing is left out.
        let plain: ExportReportInput =
            serde_json::from_str(r#"{"run_id":"r","format":"pdf"}"#).unwrap();
        assert_eq!(plain.leave_out, LeaveOutInput::default());

        let s = state();
        let id = finished(&s, Ok(results(Verdict::Pass)));
        // The one table passed: leaving out passing tables drops it, and page
        // 1 says so; the verdict is untouched.
        let (all, _) = verified_layout(&id, false, &s).unwrap();
        let (trimmed, _) = verified_layout(&id, choices, &s).unwrap();
        assert_eq!(all.tables.len(), 1);
        assert!(trimmed.tables.is_empty());
        assert_eq!(trimmed.omitted, vec!["1 table that passed"]);
        assert_eq!(trimmed.verdict_word, all.verdict_word);
    }

    #[test]
    fn keys_appear_only_when_the_user_opts_in() {
        let s = state();
        let id = finished(&s, Ok(results(Verdict::Fail)));
        let has_keys = |l: &Layout| {
            l.tables
                .iter()
                .any(|t| t.keys.iter().any(|k| k.contains(KEY_SENTINEL)))
        };
        let (off, _) = verified_layout(&id, false, &s).unwrap();
        assert!(!has_keys(&off));
        let (on, _) = verified_layout(&id, true, &s).unwrap();
        assert!(has_keys(&on));
        // The input defaults to keys off when the field is absent.
        let input: ExportReportInput =
            serde_json::from_str(r#"{"run_id":"run-1","format":"pdf"}"#).unwrap();
        assert!(!input.include_keys);
        let html: ExportReportInput =
            serde_json::from_str(r#"{"run_id":"run-1","format":"html"}"#).unwrap();
        assert_eq!(html.format, ReportFormat::Html);
    }

    #[test]
    fn the_input_never_carries_a_path_or_results() {
        for extra in [
            r#""path":"/etc/x""#,
            r#""results":{}"#,
            r#""file_path":"/x""#,
        ] {
            let json = format!(r#"{{"run_id":"r","format":"excel",{extra}}}"#);
            assert!(
                serde_json::from_str::<ExportReportInput>(&json).is_err(),
                "{extra}"
            );
        }
    }

    #[test]
    fn extensions_and_suggested_names() {
        let p = with_extension(PathBuf::from("/tmp/report"), ReportFormat::Pdf);
        assert_eq!(p, PathBuf::from("/tmp/report.pdf"));
        let p = with_extension(PathBuf::from("/tmp/r.XLSX"), ReportFormat::Excel);
        assert_eq!(p, PathBuf::from("/tmp/r.XLSX"));
        let p = with_extension(PathBuf::from("/tmp/r.pdf"), ReportFormat::Excel);
        assert_eq!(p, PathBuf::from("/tmp/r.pdf.xlsx"));
        assert_eq!(
            suggested_name("run-ab-cd", ReportFormat::Pdf),
            "dmux-run-ab-cd.pdf"
        );
        assert_eq!(
            suggested_name("../evil", ReportFormat::Excel),
            "dmux-report.xlsx"
        );
    }

    #[test]
    fn the_webview_gets_no_dialog_or_filesystem_permission() {
        let caps: serde_json::Value =
            serde_json::from_str(include_str!("../../capabilities/default.json")).unwrap();
        let perms: Vec<&str> = caps["permissions"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|p| p.as_str())
            .collect();
        assert!(perms.contains(&"allow-export-report"));
        for p in &perms {
            assert!(
                !p.starts_with("dialog:") && !p.starts_with("fs:"),
                "the webview must not get {p}"
            );
        }
    }

    #[test]
    fn the_result_serialises_for_the_ui() {
        let saved = serde_json::to_value(ExportReportResult::Saved {
            path: "/a.pdf".into(),
        })
        .unwrap();
        assert_eq!(
            saved,
            serde_json::json!({"status": "saved", "path": "/a.pdf"})
        );
        let c = serde_json::to_value(ExportReportResult::Cancelled).unwrap();
        assert_eq!(c, serde_json::json!({"status": "cancelled"}));
    }
}
