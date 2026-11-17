// Prevents an extra console window on Windows in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;
mod error;
mod run_manager;
mod secrets;
mod state;
mod store;
mod types;

use tauri::Manager;

/// Unique application identifier (decision 1: distinct from the other tool
/// named dmux). Must match `identifier` in tauri.conf.json; Tauri 2 allows
/// only alphanumerics, hyphens and periods, in reverse-domain notation.
const APP_IDENTIFIER: &str = "verify.dmux.desktop";

fn main() {
    let context = tauri::generate_context!();
    // Guard against drift between the code constant and tauri.conf.json.
    assert_eq!(
        context.config().identifier,
        APP_IDENTIFIER,
        "tauri.conf.json identifier drifted from APP_IDENTIFIER"
    );

    tauri::Builder::default()
        // The save dialog for report export is opened from Rust only; the
        // webview has no dialog permission (see capabilities/default.json).
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            commands::profiles::list_profiles,
            commands::profiles::save_profile,
            commands::profiles::delete_profile,
            commands::profiles::profile_has_password,
            commands::gateway::probe_connection,
            commands::gateway::compare_structure,
            commands::gateway::build_mapping,
            commands::gateway::update_mapping,
            commands::gateway::export_mapping,
            commands::gateway::import_mapping,
            commands::mapping_edit::edit_mapping,
            commands::mapping_edit::preview_rules,
            commands::mapping_edit::rebuild_mapping,
            commands::gateway::get_version,
            commands::data::build_check_plan,
            commands::data::start_run,
            commands::data::poll_run,
            commands::data::set_run_paused,
            commands::data::cancel_run,
            commands::data::get_results,
            commands::report::export_report,
        ])
        .setup(|app| {
            let data_dir = state::app_data_dir(app.handle());
            let app_state = state::AppState::new(data_dir)?;
            app.manage(app_state);
            Ok(())
        })
        .build(context)
        .expect("error while building the Dmux app")
        .run(|app, event| {
            // Session-only passwords (no keychain, T055) never outlive the app.
            if let tauri::RunEvent::Exit = event {
                if let Some(state) = app.try_state::<state::AppState>() {
                    state.clear_session_passwords();
                }
            }
        });
}

#[cfg(test)]
mod tests {
    use super::APP_IDENTIFIER;

    #[test]
    fn identifier_uses_only_allowed_characters() {
        assert!(!APP_IDENTIFIER.is_empty());
        assert!(APP_IDENTIFIER
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.'));
    }

    #[test]
    fn identifier_matches_the_tauri_config() {
        let config = include_str!("../tauri.conf.json");
        let expected = format!("\"identifier\": \"{APP_IDENTIFIER}\"");
        assert!(
            config.contains(&expected),
            "tauri.conf.json identifier drifted from APP_IDENTIFIER"
        );
    }
}
