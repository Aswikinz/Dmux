//! Run history, opening a results file, the run log and purging history
//! (PLAN 2.3, 4.7, 4.8, T066).
//!
//! Every path to the screen goes through verification: a history entry's file
//! or a file the user picks is read (size-limited), checked exactly like
//! `dmux report` checks it (SHA-256 over the typed results, hook envelope,
//! inconsistent PASS refused), and only then registered in app state under a
//! fresh run id.  The Results and Report screens then work from that id; the
//! webview never supplies results or a path.  File dialogs are opened from
//! Rust only.

use std::path::Path;

use serde::Serialize;
use tauri::State;

use dmux_report::layout::ReportOptions;

use crate::commands::data::{map_opened, ResultsDto};
use crate::commands::report::ExportReportResult;
use crate::error::CommandError;
use crate::history::{self, Opened};
use crate::state::AppState;
use crate::store::HistoryRow;

/// The outcome of the "open a results file" dialog.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum OpenResultsOutcome {
    Opened { results: Box<ResultsDto> },
    Cancelled,
}

/// Every stored run, newest first.
#[tauri::command]
pub fn list_history(state: State<'_, AppState>) -> Result<Vec<HistoryRow>, CommandError> {
    state.store()?.history_list()
}

/// Load and verify a history entry's results file, register the verified
/// results under a fresh run id and return them for the Results screen.
pub(crate) fn open_history_with(id: &str, state: &AppState) -> Result<ResultsDto, CommandError> {
    let opened = load_history_entry(id, state)?;
    register(opened, state)
}

fn load_history_entry(id: &str, state: &AppState) -> Result<Opened, CommandError> {
    let row = state
        .store()?
        .history_get(id)?
        .ok_or_else(|| CommandError::InvalidInput {
            message: "that run is not in the history".to_string(),
        })?;
    let path = history::results_path(&state.history_dir(), id)?;
    let verification = history::read_verified(&path)?;
    // The file must be the one the history row recorded, not another valid
    // results file dropped in its place.
    if verification.results.results_sha256 != row.results_sha256 {
        return Err(CommandError::Report {
            message: "this stored results file is not the one Dmux recorded for this run, so \
                      it is not shown"
                .to_string(),
        });
    }
    Ok(Opened::from_verification(verification))
}

fn register(opened: Opened, state: &AppState) -> Result<ResultsDto, CommandError> {
    let run_id = state.register_opened(opened.clone())?;
    Ok(map_opened(&run_id, &opened))
}

/// Verify the file at `path` and register it (a file the user picked).
pub(crate) fn open_file_with(path: &Path, state: &AppState) -> Result<ResultsDto, CommandError> {
    let verification = history::read_verified(path)?;
    register(Opened::from_verification(verification), state)
}

/// The run log of a history entry as readable text.
pub(crate) fn history_log_with(id: &str, state: &AppState) -> Result<String, CommandError> {
    let opened = load_history_entry(id, state)?;
    Ok(dmux_report::log::text_log(
        &opened.results,
        opened.hooks.as_ref(),
        &ReportOptions::default(),
    ))
}

/// Delete every stored results file and history row; profiles are kept.
pub(crate) fn purge_with(state: &AppState) -> Result<usize, CommandError> {
    let removed = history::purge(&state.store, &state.history_dir())?;
    state
        .opened
        .lock()
        .map_err(|_| CommandError::internal("opened results lock poisoned"))?
        .clear();
    Ok(removed)
}

fn join_err(e: tauri::Error) -> CommandError {
    CommandError::internal(format!("the task failed: {e}"))
}

/// Open a history entry (verified) for the Results and Report screens.
#[tauri::command]
pub async fn open_history_entry<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    id: String,
) -> Result<ResultsDto, CommandError> {
    use tauri::Manager;
    tauri::async_runtime::spawn_blocking(move || open_history_with(&id, &app.state::<AppState>()))
        .await
        .map_err(join_err)?
}

/// Choose a results file in a native open dialog, verify it like `dmux report`
/// does, and show it.  A file that fails verification is refused with a plain
/// message and nothing is shown.
#[tauri::command]
pub async fn open_results_file<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
) -> Result<OpenResultsOutcome, CommandError> {
    use tauri::Manager;
    use tauri_plugin_dialog::DialogExt;

    let dialog = app
        .dialog()
        .file()
        .set_title("Open a Dmux results file")
        .add_filter("Dmux results", &["json"]);
    let chosen = tauri::async_runtime::spawn_blocking(move || dialog.blocking_pick_file())
        .await
        .map_err(join_err)?;
    let Some(chosen) = chosen else {
        return Ok(OpenResultsOutcome::Cancelled);
    };
    let path = chosen.into_path().map_err(|_| CommandError::Report {
        message: "the chosen location is not a local file".to_string(),
    })?;
    let results = tauri::async_runtime::spawn_blocking(move || {
        open_file_with(&path, &app.state::<AppState>())
    })
    .await
    .map_err(join_err)??;
    Ok(OpenResultsOutcome::Opened {
        results: Box::new(results),
    })
}

/// The run log of a history entry, as text for the screen.
#[tauri::command]
pub async fn get_history_log<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    id: String,
) -> Result<String, CommandError> {
    use tauri::Manager;
    tauri::async_runtime::spawn_blocking(move || history_log_with(&id, &app.state::<AppState>()))
        .await
        .map_err(join_err)?
}

/// Save a history entry's run log through a native save dialog (mode 0600).
#[tauri::command]
pub async fn save_history_log<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    id: String,
) -> Result<ExportReportResult, CommandError> {
    use tauri::Manager;
    use tauri_plugin_dialog::DialogExt;

    // Refuse before the dialog opens when the entry cannot be verified.
    let text = {
        let app2 = app.clone();
        let id2 = id.clone();
        tauri::async_runtime::spawn_blocking(move || {
            history_log_with(&id2, &app2.state::<AppState>())
        })
        .await
        .map_err(join_err)??
    };
    let name = if history::is_safe_id(&id) {
        format!("dmux-{id}-log.txt")
    } else {
        "dmux-run-log.txt".to_string()
    };
    let dialog = app
        .dialog()
        .file()
        .set_title("Save the Dmux run log")
        .set_file_name(name)
        .add_filter("Text file", &["txt"]);
    let chosen = tauri::async_runtime::spawn_blocking(move || dialog.blocking_save_file())
        .await
        .map_err(join_err)?;
    let Some(chosen) = chosen else {
        return Ok(ExportReportResult::Cancelled);
    };
    let path = chosen.into_path().map_err(|_| CommandError::Report {
        message: "the chosen location is not a local file".to_string(),
    })?;
    let target = path.clone();
    tauri::async_runtime::spawn_blocking(move || {
        history::write_private_file(&target, text.as_bytes())
    })
    .await
    .map_err(join_err)?
    .map_err(|e| CommandError::Report {
        message: format!("the run log could not be written: {e}"),
    })?;
    Ok(ExportReportResult::Saved {
        path: path.display().to_string(),
    })
}

/// Delete all stored results files and history rows (profiles are kept).
/// Returns how many runs were removed.
#[tauri::command]
pub async fn purge_history<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
) -> Result<usize, CommandError> {
    use tauri::Manager;
    tauri::async_runtime::spawn_blocking(move || purge_with(&app.state::<AppState>()))
        .await
        .map_err(join_err)?
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::report::{render_report, ReportFormat};
    use crate::history::tests::sample_results;
    use crate::run_manager::{RunHandle, RunManager};
    use dmux_engine::results::{finalise, Verdict};
    use dmux_engine::run::{Input, RunConfig};

    fn state() -> AppState {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().to_path_buf();
        std::mem::forget(dir);
        AppState::with_password_store(path, Box::new(crate::secrets::InMemoryPasswordStore::new()))
            .unwrap()
    }

    fn config() -> RunConfig {
        let profile = || {
            Input::Live(dmux_engine::db::ConnProfile::new(
                "127.0.0.1",
                1,
                "unused",
                "unused",
                String::new(),
                dmux_engine::db::TlsMode::Disable("history test".to_string()),
            ))
        };
        RunConfig::new(profile(), profile())
    }

    /// Run a fake run to completion in app state, as the drive task would.
    fn finish_run(state: &AppState, verdict: Verdict) -> String {
        let id = RunManager::fresh_id();
        state.runs.insert(&id, RunHandle::new(config(), None));
        state.note_run_start(&id, "Prod old", "Prod new");
        let ticket = state.runs.begin_first(&id).unwrap();
        state
            .runs
            .finish(&id, ticket.generation, Ok(sample_results(verdict)));
        id
    }

    #[test]
    fn a_finished_run_is_written_to_the_history_and_can_be_reopened() {
        let s = state();
        let id = finish_run(&s, Verdict::Fail);
        let rows = s.store().unwrap().history_list().unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, id);
        assert_eq!(rows[0].verdict, "FAIL");
        assert_eq!(rows[0].old_label, "Prod old");
        assert_eq!(rows[0].new_label, "Prod new");
        assert!(s.history_dir().join(format!("{id}.json")).is_file());
        assert!(
            s.run_meta.lock().unwrap().is_empty(),
            "the run's metadata is released once recorded"
        );

        let dto = open_history_with(&id, &s).unwrap();
        assert_eq!(dto.verdict, "FAIL");
        assert_ne!(dto.run_id, id, "opened results get a fresh id");
        // The Results screen's get_results works for the opened id.
        let again = crate::commands::data::get_results_with(&dto.run_id, &s).unwrap();
        assert_eq!(again.verdict, "FAIL");
    }

    #[test]
    fn a_run_that_failed_or_was_stopped_leaves_no_history() {
        let s = state();
        let id = RunManager::fresh_id();
        s.runs.insert(&id, RunHandle::new(config(), None));
        let t = s.runs.begin_first(&id).unwrap();
        s.runs.finish(
            &id,
            t.generation,
            Err(dmux_engine::run::RunError::Config("boom".into())),
        );
        assert!(s.store().unwrap().history_list().unwrap().is_empty());
        assert!(
            !s.history_dir().exists() || std::fs::read_dir(s.history_dir()).unwrap().count() == 0
        );
    }

    #[test]
    fn opened_history_results_export_a_report_by_their_id() {
        let s = state();
        let id = finish_run(&s, Verdict::Pass);
        let dto = open_history_with(&id, &s).unwrap();
        let dir = tempfile::tempdir().unwrap();
        for format in [ReportFormat::Pdf, ReportFormat::Excel] {
            let path = dir.path().join("r.out");
            render_report(&dto.run_id, format, false, &s, &path).unwrap();
            let bytes = std::fs::read(&path).unwrap();
            assert!(bytes.starts_with(b"%PDF") || bytes.starts_with(b"PK"));
        }
        // An id nobody registered is still refused.
        assert!(render_report(
            "run-nope",
            ReportFormat::Pdf,
            false,
            &s,
            &dir.path().join("x")
        )
        .is_err());
    }

    #[test]
    fn a_tampered_history_file_is_refused_and_not_registered() {
        let s = state();
        let id = finish_run(&s, Verdict::Fail);
        let path = s.history_dir().join(format!("{id}.json"));
        let text = std::fs::read_to_string(&path).unwrap();
        // Verdicts serialise in upper case; the edit must really change the file.
        let edited = text.replacen("\"FAIL\"", "\"PASS\"", 1);
        assert_ne!(
            text, edited,
            "the fixture must contain a FAIL verdict to flip"
        );
        std::fs::write(&path, edited).unwrap();
        let err = open_history_with(&id, &s).unwrap_err();
        assert!(matches!(err, CommandError::Report { .. }), "{err:?}");
        assert_eq!(s.opened.lock().unwrap().len(), 0);
        assert!(history_log_with(&id, &s).is_err());
    }

    #[test]
    fn another_valid_results_file_in_its_place_is_refused() {
        let s = state();
        let id = finish_run(&s, Verdict::Fail);
        let other = finalise({
            let mut r = sample_results(Verdict::Pass);
            r.run_id = "run-other".into();
            r
        });
        std::fs::write(
            s.history_dir().join(format!("{id}.json")),
            serde_json::to_vec(&other).unwrap(),
        )
        .unwrap();
        let err = open_history_with(&id, &s).unwrap_err();
        let CommandError::Report { message } = err else {
            panic!()
        };
        assert!(message.contains("not the one Dmux recorded"), "{message}");
    }

    #[test]
    fn an_unknown_or_unsafe_history_id_is_refused() {
        let s = state();
        assert!(open_history_with("run-nope", &s).is_err());
        assert!(open_history_with("../etc/passwd", &s).is_err());
    }

    #[test]
    fn a_picked_file_is_verified_before_it_is_shown() {
        let s = state();
        let dir = tempfile::tempdir().unwrap();
        let good = dir.path().join("good.json");
        std::fs::write(
            &good,
            serde_json::to_vec(&sample_results(Verdict::Fail)).unwrap(),
        )
        .unwrap();
        let dto = open_file_with(&good, &s).unwrap();
        assert_eq!(dto.verdict, "FAIL");

        // Edited after writing: refused.
        let bad = dir.path().join("bad.json");
        let text = std::fs::read_to_string(&good).unwrap();
        let edited = text.replacen("\"FAIL\"", "\"PASS\"", 1);
        assert_ne!(
            text, edited,
            "the fixture must contain a FAIL verdict to flip"
        );
        std::fs::write(&bad, edited).unwrap();
        assert!(open_file_with(&bad, &s).is_err());

        // A PASS that hides a failing table, with an honest digest: refused.
        let mut lying = sample_results(Verdict::Fail);
        lying.verdict = Verdict::Pass;
        let lying = finalise(lying);
        let liar = dir.path().join("liar.json");
        std::fs::write(&liar, serde_json::to_vec(&lying).unwrap()).unwrap();
        assert!(open_file_with(&liar, &s).is_err());

        // Not results at all.
        let junk = dir.path().join("junk.json");
        std::fs::write(&junk, b"hello").unwrap();
        assert!(open_file_with(&junk, &s).is_err());
        assert_eq!(s.opened.lock().unwrap().len(), 1, "only the good file");
    }

    #[test]
    fn the_run_log_is_readable_text_for_a_history_entry() {
        let s = state();
        let id = finish_run(&s, Verdict::Pass);
        let text = history_log_with(&id, &s).unwrap();
        assert!(text.starts_with("Dmux run log"));
        assert!(text.contains("verdict: PASS"));
        assert!(text.contains("coverage: 1 of 1 tables fully checked"));
    }

    #[test]
    fn purge_deletes_files_rows_and_opened_results_but_not_profiles() {
        let s = state();
        s.store()
            .unwrap()
            .save(&crate::store::StoredProfile {
                id: "keep".into(),
                label: "Kept".into(),
                host: "h".into(),
                port: 5432,
                database: "d".into(),
                user: "u".into(),
                encryption: "verify".into(),
                tls_fingerprint: None,
                plaintext_reason: None,
                ca_file: None,
            })
            .unwrap();
        let a = finish_run(&s, Verdict::Pass);
        let b = finish_run(&s, Verdict::Fail);
        let opened = open_history_with(&a, &s).unwrap();
        assert!(s.history_dir().join(format!("{a}.json")).is_file());

        assert_eq!(purge_with(&s).unwrap(), 2);
        for id in [&a, &b] {
            assert!(!s.history_dir().join(format!("{id}.json")).exists());
        }
        assert_eq!(std::fs::read_dir(s.history_dir()).unwrap().count(), 0);
        assert!(s.store().unwrap().history_list().unwrap().is_empty());
        assert_eq!(s.store().unwrap().list().unwrap().len(), 1);
        assert!(crate::commands::data::get_results_with(&opened.run_id, &s).is_err());
        assert!(open_history_with(&a, &s).is_err());
    }

    #[test]
    fn the_webview_gets_permission_for_the_history_commands_only() {
        let caps: serde_json::Value =
            serde_json::from_str(include_str!("../../capabilities/default.json")).unwrap();
        let perms: Vec<&str> = caps["permissions"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|p| p.as_str())
            .collect();
        for p in [
            "allow-list-history",
            "allow-open-history-entry",
            "allow-open-results-file",
            "allow-get-history-log",
            "allow-save-history-log",
            "allow-purge-history",
        ] {
            assert!(perms.contains(&p), "{p}");
        }
        assert!(perms
            .iter()
            .all(|p| !p.starts_with("dialog:") && !p.starts_with("fs:")));
    }

    #[test]
    fn the_outcome_serialises_for_the_ui() {
        let v = serde_json::to_value(OpenResultsOutcome::Cancelled).unwrap();
        assert_eq!(v, serde_json::json!({"status": "cancelled"}));
    }
}
