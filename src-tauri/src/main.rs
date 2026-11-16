// Prevents an extra console window on Windows in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

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
        .run(context)
        .expect("error while running the Dmux app");
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
