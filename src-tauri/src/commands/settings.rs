//! App settings: custom checks (Python scripts), appearance stays in the UI
//! (T074).
//!
//! The scripts folder and the Python interpreter are chosen with native
//! dialogs opened **from Rust**.  The webview can switch custom checks on or
//! off and set the time limit, but it never sends a path: nothing that is
//! later executed is named by the webview.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tauri::State;

use crate::error::CommandError;
use crate::hooks::{
    found_interpreter, scan_folder, FolderScan, HookSettings, PythonSource, HOOK_WARNING,
};
use crate::prefs::Prefs;
use crate::state::AppState;

/// What the Settings screen shows.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct SettingsDto {
    pub custom_checks_enabled: bool,
    /// The folder chosen with the folder dialog.
    pub hooks_folder: Option<String>,
    /// What is in that folder and which scripts are refused (and why).
    pub scan: Option<FolderScan>,
    /// A Python chosen with the file dialog (`None`: automatic).
    pub python_path: Option<String>,
    pub python_source: PythonSource,
    /// The interpreter that would run now; `None` when none can be found.
    pub python_found: Option<String>,
    pub timeout_secs: u64,
    /// The honest warning, in the words the UI shows.
    pub warning: String,
    /// The check level a new plan starts from: `suggested`, `structure`,
    /// `counts` or `full_values`.
    pub starting_level: String,
    pub old_parallelism: usize,
    pub new_parallelism: usize,
    /// Where the Report screen's "include row keys" box starts (off).
    pub include_keys_default: bool,
    pub app_version: String,
    /// Where Dmux keeps its settings, profiles and run files (read-only).
    pub data_dir: String,
    pub licence: String,
}

/// What the webview may change.  There is deliberately no path field.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct UpdateSettingsInput {
    #[serde(default)]
    pub custom_checks_enabled: Option<bool>,
    #[serde(default)]
    pub timeout_secs: Option<u64>,
    #[serde(default)]
    pub starting_level: Option<String>,
    #[serde(default)]
    pub old_parallelism: Option<usize>,
    #[serde(default)]
    pub new_parallelism: Option<usize>,
    #[serde(default)]
    pub include_keys_default: Option<bool>,
}

/// Build the screen's view of the settings (blocking: reads the folder).
pub(crate) fn settings_dto(
    settings: &HookSettings,
    prefs: &Prefs,
    data_dir: &Path,
    bundled: Option<&Path>,
) -> SettingsDto {
    let scan = settings.folder.as_deref().map(scan_folder);
    let (_, source) = crate::hooks::resolve_interpreter(settings, bundled);
    SettingsDto {
        custom_checks_enabled: settings.enabled,
        hooks_folder: settings.folder.as_ref().map(|p| p.display().to_string()),
        scan,
        python_path: settings.python.as_ref().map(|p| p.display().to_string()),
        python_source: source,
        python_found: found_interpreter(settings, bundled).map(|(p, _)| p.display().to_string()),
        timeout_secs: settings.timeout_secs,
        warning: HOOK_WARNING.to_string(),
        starting_level: prefs.starting_level.clone(),
        old_parallelism: prefs.old_parallelism,
        new_parallelism: prefs.new_parallelism,
        include_keys_default: prefs.include_keys_default,
        app_version: dmux_engine::version().to_string(),
        data_dir: data_dir.display().to_string(),
        licence: "Apache-2.0".to_string(),
    }
}

async fn current_dto(state: &AppState) -> Result<SettingsDto, CommandError> {
    let (settings, prefs) = {
        let store = state.store()?;
        (HookSettings::load(&store)?, Prefs::load(&store)?)
    };
    let bundled = state.bundled_python();
    let data_dir = state.data_dir.clone();
    tauri::async_runtime::spawn_blocking(move || {
        settings_dto(&settings, &prefs, &data_dir, bundled.as_deref())
    })
    .await
    .map_err(|e| CommandError::internal(format!("the settings task failed: {e}")))
}

/// Apply the webview's changes.
pub(crate) fn apply_update(
    state: &AppState,
    input: &UpdateSettingsInput,
) -> Result<(), CommandError> {
    let store = state.store()?;
    if let Some(enabled) = input.custom_checks_enabled {
        HookSettings::save_enabled(&store, enabled)?;
    }
    if let Some(secs) = input.timeout_secs {
        HookSettings::save_timeout(&store, secs)?;
    }
    if let Some(level) = &input.starting_level {
        Prefs::save_level(&store, level)?;
    }
    Prefs::save_parallelism(&store, input.old_parallelism, input.new_parallelism)?;
    if let Some(on) = input.include_keys_default {
        Prefs::save_include_keys(&store, on)?;
    }
    Ok(())
}

/// Store a folder the user picked in the native dialog.
pub(crate) fn store_folder(state: &AppState, folder: &Path) -> Result<(), CommandError> {
    if !folder.is_dir() {
        return Err(CommandError::InvalidInput {
            message: "that is not a folder".to_string(),
        });
    }
    HookSettings::save_folder(&*state.store()?, Some(folder))
}

/// Store an interpreter the user picked in the native dialog.
pub(crate) fn store_python(state: &AppState, python: &Path) -> Result<(), CommandError> {
    if !python.is_file() {
        return Err(CommandError::InvalidInput {
            message: "that is not a program file".to_string(),
        });
    }
    HookSettings::save_python(&*state.store()?, Some(python))
}

#[tauri::command]
pub async fn get_settings(state: State<'_, AppState>) -> Result<SettingsDto, CommandError> {
    current_dto(&state).await
}

#[tauri::command]
pub async fn update_settings(
    input: UpdateSettingsInput,
    state: State<'_, AppState>,
) -> Result<SettingsDto, CommandError> {
    apply_update(&state, &input)?;
    current_dto(&state).await
}

/// Pick the scripts folder in a native dialog.  A dismissed dialog changes
/// nothing.
#[tauri::command]
pub async fn choose_hooks_folder<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    state: State<'_, AppState>,
) -> Result<SettingsDto, CommandError> {
    use tauri_plugin_dialog::DialogExt;
    let dialog = app
        .dialog()
        .file()
        .set_title("Choose the folder with your custom checks");
    let chosen = tauri::async_runtime::spawn_blocking(move || dialog.blocking_pick_folder())
        .await
        .map_err(|e| CommandError::internal(format!("the folder dialog failed: {e}")))?;
    if let Some(chosen) = chosen {
        let path: PathBuf = chosen.into_path().map_err(|_| CommandError::InvalidInput {
            message: "the chosen location is not a local folder".to_string(),
        })?;
        store_folder(&state, &path)?;
    }
    current_dto(&state).await
}

#[tauri::command]
pub async fn clear_hooks_folder(state: State<'_, AppState>) -> Result<SettingsDto, CommandError> {
    HookSettings::save_folder(&*state.store()?, None)?;
    current_dto(&state).await
}

/// Pick the Python program in a native dialog.  A dismissed dialog changes
/// nothing.
#[tauri::command]
pub async fn choose_python<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    state: State<'_, AppState>,
) -> Result<SettingsDto, CommandError> {
    use tauri_plugin_dialog::DialogExt;
    let dialog = app.dialog().file().set_title("Choose the Python program");
    let chosen = tauri::async_runtime::spawn_blocking(move || dialog.blocking_pick_file())
        .await
        .map_err(|e| CommandError::internal(format!("the file dialog failed: {e}")))?;
    if let Some(chosen) = chosen {
        let path: PathBuf = chosen.into_path().map_err(|_| CommandError::InvalidInput {
            message: "the chosen location is not a local file".to_string(),
        })?;
        store_python(&state, &path)?;
    }
    current_dto(&state).await
}

/// Go back to the bundled or automatically found Python.
#[tauri::command]
pub async fn clear_python(state: State<'_, AppState>) -> Result<SettingsDto, CommandError> {
    HookSettings::save_python(&*state.store()?, None)?;
    current_dto(&state).await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state() -> (tempfile::TempDir, AppState) {
        let dir = tempfile::tempdir().expect("tempdir");
        let s = AppState::with_password_store(
            dir.path().to_path_buf(),
            Box::new(crate::secrets::InMemoryPasswordStore::new()),
        )
        .expect("state");
        (dir, s)
    }

    fn no_change() -> UpdateSettingsInput {
        UpdateSettingsInput {
            custom_checks_enabled: None,
            timeout_secs: None,
            starting_level: None,
            old_parallelism: None,
            new_parallelism: None,
            include_keys_default: None,
        }
    }

    fn dto_of(st: &AppState) -> SettingsDto {
        let store = st.store().unwrap();
        settings_dto(
            &HookSettings::load(&store).unwrap(),
            &Prefs::load(&store).unwrap(),
            &st.data_dir,
            None,
        )
    }

    #[test]
    fn run_and_report_defaults_persist_and_show_in_the_dto() {
        let (dir, st) = state();
        let d = dto_of(&st);
        assert_eq!((d.old_parallelism, d.new_parallelism), (2, 4));
        assert!(!d.include_keys_default);
        assert_eq!(d.starting_level, "suggested");
        assert_eq!(d.data_dir, st.data_dir.display().to_string());
        assert_eq!(d.app_version, dmux_engine::version());
        assert_eq!(d.licence, "Apache-2.0");
        apply_update(
            &st,
            &UpdateSettingsInput {
                starting_level: Some("counts".into()),
                old_parallelism: Some(1),
                new_parallelism: Some(6),
                include_keys_default: Some(true),
                ..no_change()
            },
        )
        .unwrap();
        drop(st);
        let reopened = AppState::with_password_store(
            dir.path().to_path_buf(),
            Box::new(crate::secrets::InMemoryPasswordStore::new()),
        )
        .unwrap();
        let d = dto_of(&reopened);
        assert_eq!(d.starting_level, "counts");
        assert_eq!((d.old_parallelism, d.new_parallelism), (1, 6));
        assert!(d.include_keys_default);
    }

    #[test]
    fn bad_run_defaults_are_refused_and_change_nothing() {
        let (_d, st) = state();
        for bad in [
            UpdateSettingsInput {
                old_parallelism: Some(0),
                ..no_change()
            },
            UpdateSettingsInput {
                new_parallelism: Some(1000),
                ..no_change()
            },
            UpdateSettingsInput {
                starting_level: Some("all".into()),
                ..no_change()
            },
        ] {
            assert!(apply_update(&st, &bad).is_err());
        }
        assert_eq!(dto_of(&st).old_parallelism, 2);
    }

    #[test]
    fn the_webview_cannot_name_a_path() {
        let ok: Result<UpdateSettingsInput, _> =
            serde_json::from_str(r#"{"custom_checks_enabled": true, "timeout_secs": 30}"#);
        assert!(ok.is_ok());
        for field in ["hooks_folder", "folder", "python", "python_path", "path"] {
            let json = format!(r#"{{"{field}": "/tmp/x"}}"#);
            assert!(
                serde_json::from_str::<UpdateSettingsInput>(&json).is_err(),
                "{field} must be refused"
            );
        }
    }

    #[test]
    fn settings_start_off_and_persist_across_a_reopen() {
        let (dir, st) = state();
        let dto = dto_of(&st);
        assert!(!dto.custom_checks_enabled);
        assert_eq!(dto.warning, HOOK_WARNING);
        assert!(dto.scan.is_none());

        apply_update(
            &st,
            &UpdateSettingsInput {
                custom_checks_enabled: Some(true),
                timeout_secs: Some(20),
                ..no_change()
            },
        )
        .unwrap();
        let scripts = tempfile::tempdir().unwrap();
        store_folder(&st, scripts.path()).unwrap();
        drop(st);

        let reopened = AppState::with_password_store(
            dir.path().to_path_buf(),
            Box::new(crate::secrets::InMemoryPasswordStore::new()),
        )
        .unwrap();
        let s = HookSettings::load(&reopened.store().unwrap()).unwrap();
        assert!(s.enabled);
        assert_eq!(s.timeout_secs, 20);
        assert_eq!(s.folder.as_deref(), Some(scripts.path()));
        let plan = crate::hooks::plan_for_new_run(&reopened).unwrap();
        assert!(plan.is_some());
    }

    #[test]
    fn a_bad_time_limit_or_a_non_folder_is_refused() {
        let (_d, st) = state();
        let bad = UpdateSettingsInput {
            timeout_secs: Some(0),
            ..no_change()
        };
        assert!(apply_update(&st, &bad).is_err());
        assert!(store_folder(&st, Path::new("/definitely/not/here")).is_err());
        assert!(store_python(&st, Path::new("/definitely/not/here")).is_err());
    }

    #[test]
    fn the_dto_lists_a_chosen_folder_and_the_chosen_python() {
        let (_d, st) = state();
        let scripts = tempfile::tempdir().unwrap();
        std::fs::write(scripts.path().join("a.py"), "x").unwrap();
        store_folder(&st, scripts.path()).unwrap();
        let py = scripts.path().join("python-stub");
        std::fs::write(&py, "x").unwrap();
        store_python(&st, &py).unwrap();
        let dto = dto_of(&st);
        assert_eq!(dto.python_source, PythonSource::Chosen);
        assert_eq!(dto.python_found.as_deref(), Some(py.to_str().unwrap()));
        assert!(dto.scan.unwrap().scripts.iter().any(|s| s.name == "a.py"));
    }
}
