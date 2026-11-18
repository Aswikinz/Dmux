//! Save the mapping to a file from the Map screen.
//!
//! The mapping the webview sends is untrusted (audit L7), so it is validated
//! exactly like an imported mapping file, re-hashed, and written in the
//! engine's canonical form ([`mapping::to_json`]): the same bytes `dmux run`
//! reads with `mapping = "<file>"` and the app's own import accepts, embedded
//! hash included.  The destination comes from the native **save** dialog,
//! opened from Rust, so the webview never names a path; the file is written
//! owner-only (mode 0600 on Unix).

use std::path::PathBuf;

use serde::Serialize;

use dmux_engine::mapping::{self, Mapping};

use crate::commands::gateway::validate_mapping;
use crate::error::CommandError;

/// The outcome: where the file went and its mapping code, or that the dialog
/// was dismissed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum SaveMappingResult {
    Saved { path: String, hash: String },
    Cancelled,
}

/// The canonical file bytes and the mapping's hash, after validation.
pub(crate) fn mapping_file_bytes(m: &Mapping) -> Result<(Vec<u8>, String), CommandError> {
    let checked = validate_mapping(m)?;
    let bytes = mapping::to_json(&checked).map_err(|e| CommandError::InvalidMapping {
        message: e.to_string(),
    })?;
    Ok((bytes, checked.mapping_hash))
}

/// A suggested file name from the mapping code (hex only), else a fixed name.
fn suggested_name(hash: &str) -> String {
    let short: String = hash.chars().take(12).collect();
    if !short.is_empty() && short.chars().all(|c| c.is_ascii_hexdigit()) {
        format!("dmux-mapping-{short}.json")
    } else {
        "dmux-mapping.json".to_string()
    }
}

/// Give `path` a `.json` extension when the user left it off.
fn with_json_extension(path: PathBuf) -> PathBuf {
    if path
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("json"))
    {
        path
    } else {
        let mut name = path.into_os_string();
        name.push(".json");
        PathBuf::from(name)
    }
}

/// Save the mapping through a native save dialog.  An invalid mapping is
/// refused before the dialog opens.
#[tauri::command]
pub async fn save_mapping<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    mapping: Mapping,
) -> Result<SaveMappingResult, CommandError> {
    use tauri_plugin_dialog::DialogExt;

    let (bytes, hash) = mapping_file_bytes(&mapping)?;
    let dialog = app
        .dialog()
        .file()
        .set_title("Save the Dmux mapping")
        .set_file_name(suggested_name(&hash))
        .add_filter("Dmux mapping", &["json"]);
    let chosen = tauri::async_runtime::spawn_blocking(move || dialog.blocking_save_file())
        .await
        .map_err(|e| CommandError::internal(format!("the save dialog failed: {e}")))?;
    let Some(chosen) = chosen else {
        return Ok(SaveMappingResult::Cancelled);
    };
    let path = chosen.into_path().map_err(|_| CommandError::InvalidInput {
        message: "the chosen location is not a local file".to_string(),
    })?;
    let path = with_json_extension(path);
    let target = path.clone();
    tauri::async_runtime::spawn_blocking(move || {
        crate::history::write_private_file(&target, &bytes)
    })
    .await
    .map_err(|e| CommandError::internal(format!("the save task failed: {e}")))?
    .map_err(|e| CommandError::Store {
        message: format!("the mapping could not be written: {e}"),
    })?;
    Ok(SaveMappingResult::Saved {
        path: path.display().to_string(),
        hash,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_saved_bytes_are_the_canonical_file_that_import_and_the_cli_read() {
        let mut m = Mapping::new();
        m.ignored.push(mapping::IgnoredItem {
            path: "demo_bank.archive".into(),
            reason: "archived before the migration".into(),
        });
        // A stale hash from the webview is never trusted: it is re-derived.
        m.mapping_hash = "stale".into();
        let (bytes, hash) = mapping_file_bytes(&m).unwrap();
        let back = mapping::from_json(&bytes).expect("the saved file imports");
        assert_eq!(back.mapping_hash, hash);
        assert_ne!(hash, "stale");
        assert_eq!(back.ignored, m.ignored);
        // The same mapping always gives the same file.
        assert_eq!(mapping_file_bytes(&m).unwrap().0, bytes);
    }

    #[test]
    fn an_invalid_mapping_is_refused_before_any_dialog() {
        let mut m = Mapping::new();
        m.ignored.push(mapping::IgnoredItem {
            path: "demo_bank.archive".into(),
            reason: String::new(),
        });
        assert!(matches!(
            mapping_file_bytes(&m),
            Err(CommandError::InvalidMapping { .. })
        ));
    }

    #[test]
    fn names_and_extensions() {
        assert_eq!(
            suggested_name("0123456789abcdef"),
            "dmux-mapping-0123456789ab.json"
        );
        assert_eq!(suggested_name("../evil"), "dmux-mapping.json");
        assert_eq!(suggested_name(""), "dmux-mapping.json");
        assert_eq!(
            with_json_extension(PathBuf::from("/tmp/m")),
            PathBuf::from("/tmp/m.json")
        );
        assert_eq!(
            with_json_extension(PathBuf::from("/tmp/m.JSON")),
            PathBuf::from("/tmp/m.JSON")
        );
    }

    #[test]
    fn the_window_may_call_save_mapping() {
        // Without this the command is blocked in the real app, while every
        // unit test still passes.
        let caps: serde_json::Value =
            serde_json::from_str(include_str!("../../capabilities/default.json")).unwrap();
        let perms: Vec<&str> = caps["permissions"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|p| p.as_str())
            .collect();
        assert!(perms.contains(&"allow-save-mapping"));
        assert!(include_str!("../../build.rs").contains("\"save_mapping\""));
    }

    #[cfg(unix)]
    #[test]
    fn the_file_is_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("m.json");
        let (bytes, _) = mapping_file_bytes(&Mapping::new()).unwrap();
        crate::history::write_private_file(&path, &bytes).unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }
}
