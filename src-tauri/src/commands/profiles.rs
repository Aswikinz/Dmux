//! Connection-profile commands.
//!
//! The SQLite store holds everything except the password; the keychain holds
//! the password keyed by profile id.  Together they satisfy PLAN 6.5 and 4.1.

use serde::Serialize;
use tauri::State;

use crate::error::CommandError;
use crate::secrets::NOT_SAVED_MESSAGE;
use crate::state::AppState;
use crate::store::StoredProfile;
use crate::types::ProfileInput;

/// Fetch every saved profile (no passwords).
#[tauri::command]
pub fn list_profiles(state: State<'_, AppState>) -> Result<Vec<StoredProfile>, CommandError> {
    state.store()?.list()
}

/// The result of saving a profile: the stored profile plus, when the password
/// could not be written to the keychain, a plain-language warning.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct SaveProfileResult {
    pub profile: StoredProfile,
    /// `Some` when the password could not be stored; the user will be asked for
    /// it on each run instead.
    pub warning: Option<String>,
}

/// Save (insert or update) a profile.  The optional `password`, when present,
/// is stored in the keychain only (never SQLite).  A keychain failure degrades
/// to a typed warning, not an error — the profile is still saved.
#[tauri::command]
pub fn save_profile(
    profile: ProfileInput,
    password: Option<String>,
    state: State<'_, AppState>,
) -> Result<SaveProfileResult, CommandError> {
    validate_profile(&profile)?;
    let stored = state.store()?.save(&StoredProfile {
        id: profile.id.clone(),
        label: profile.label.clone(),
        host: profile.host.clone(),
        port: profile.port,
        database: profile.database.clone(),
        user: profile.user.clone(),
        encryption: profile.encryption.clone(),
        tls_fingerprint: profile.tls_fingerprint.clone(),
        plaintext_reason: profile.plaintext_reason.clone(),
    })?;

    // The password goes to the keychain keyed by the final profile id (which is
    // stable after the store assigned it).  A failure is surfaced as a warning,
    // never as a stored-secret loss nor a silent drop.
    let mut warning = None;
    if let Some(password) = password {
        match state.keychain()?.set(&stored.id, &password) {
            Ok(()) => {}
            Err(e) if e.to_string().contains(NOT_SAVED_MESSAGE) => {
                warning = Some(NOT_SAVED_MESSAGE.to_string());
            }
            Err(e) => warning = Some(e.to_string()),
        }
    }

    Ok(SaveProfileResult {
        profile: stored,
        warning,
    })
}

/// Delete a profile and its keychain password.
#[tauri::command]
pub fn delete_profile(id: String, state: State<'_, AppState>) -> Result<bool, CommandError> {
    let removed = state.store()?.delete(&id)?;
    if removed {
        state.keychain()?.delete(&id)?;
    }
    Ok(removed)
}

/// Return whether a password is stored for a profile, without returning it.
#[tauri::command]
pub fn profile_has_password(id: String, state: State<'_, AppState>) -> Result<bool, CommandError> {
    match state.store()?.get(&id)? {
        Some(_) => {}
        None => return Err(CommandError::ProfileNotFound { key: id }),
    }
    match state.keychain()?.get(&id) {
        Ok(_) => Ok(true),
        Err(CommandError::PasswordMissing { .. }) => Ok(false),
        Err(e) => Err(e),
    }
}

/// Reject obviously-invalid profiles before they reach the store or the network.
fn validate_profile(profile: &ProfileInput) -> Result<(), CommandError> {
    if profile.host.trim().is_empty() {
        return Err(CommandError::InvalidInput {
            message: "host may not be empty".to_string(),
        });
    }
    if profile.port == 0 {
        return Err(CommandError::InvalidInput {
            message: "port may not be 0".to_string(),
        });
    }
    if profile.database.trim().is_empty() {
        return Err(CommandError::InvalidInput {
            message: "database may not be empty".to_string(),
        });
    }
    if profile.user.trim().is_empty() {
        return Err(CommandError::InvalidInput {
            message: "user may not be empty".to_string(),
        });
    }
    // The TLS mode itself is validated lazily in types::parse_tls_mode, but we
    // reject an obviously-blank mode early for a clearer message.
    if profile.encryption.trim().is_empty() {
        return Err(CommandError::InvalidInput {
            message: "encryption mode may not be empty".to_string(),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_empty_host() {
        let mut p = crate::types::ProfileInput {
            id: String::new(),
            label: String::new(),
            host: "   ".to_string(),
            port: 5432,
            database: "d".to_string(),
            user: "u".to_string(),
            encryption: "verify".to_string(),
            tls_fingerprint: None,
            plaintext_reason: None,
        };
        assert!(validate_profile(&p).is_err());

        p.host = "db".to_string();
        p.port = 0;
        assert!(validate_profile(&p).is_err());
    }
}
