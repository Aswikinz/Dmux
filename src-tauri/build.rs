// build.rs: configure the Tauri build and declare the app's custom commands so
// tauri-build generates `allow-<command>` / `deny-<command>` permission
// identifiers for them (snake_case).  The capability file then references
// exactly these, keeping app permissions tight (PLAN 6.9).

fn main() {
    tauri_build::try_build(tauri_build::Attributes::new().app_manifest(
        tauri_build::AppManifest::new().commands(&[
            "list_profiles",
            "save_profile",
            "delete_profile",
            "profile_has_password",
            "probe_connection",
            "compare_structure",
            "build_mapping",
            "update_mapping",
            "export_mapping",
            "save_mapping",
            "save_check_choices",
            "open_check_choices",
            "import_mapping",
            "edit_mapping",
            "preview_rules",
            "rebuild_mapping",
            "get_version",
            "build_check_plan",
            "start_run",
            "poll_run",
            "set_run_paused",
            "cancel_run",
            "get_results",
            "export_report",
            "get_settings",
            "update_settings",
            "choose_hooks_folder",
            "clear_hooks_folder",
            "choose_python",
            "clear_python",
            "list_history",
            "open_history_entry",
            "open_results_file",
            "get_history_log",
            "save_history_log",
            "purge_history",
        ]),
    ))
    .expect("failed to run tauri build script");
}
