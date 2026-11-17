//! Connection-profile commands.
//!
//! The SQLite store holds everything except the password; the keychain holds
//! the password keyed by profile id.  Together they satisfy PLAN 6.5 and 4.1.
//!
//! A critical safety rule: a saved password must never travel to a *changed*
//! destination.  When a `save_profile` update changes any connection field
//! (host, port, database, user, encryption, fingerprint, reason) without a new
//! password in the same call, the old keychain entry is deleted, so the next
//! use asks for the password again rather than reusing it against new (possibly
//! attacker-controlled) details.

use serde::Serialize;
use tauri::State;

use crate::error::CommandError;
use crate::secrets::{NOT_SAVED_MESSAGE, PASSWORD_REMOVED_MESSAGE};
use crate::state::AppState;
use crate::store::StoredProfile;
use crate::types::ProfileInput;

/// Fetch every saved profile (no passwords).
#[tauri::command]
pub fn list_profiles(state: State<'_, AppState>) -> Result<Vec<StoredProfile>, CommandError> {
    state.store()?.list()
}

/// The result of saving a profile: the stored profile plus, when the password
/// could not be written to (or had to be removed from) the keychain, a
/// plain-language warning.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct SaveProfileResult {
    pub profile: StoredProfile,
    /// `Some` when the password could not be stored, or when it was removed
    /// because the connection details changed without a new password.
    pub warning: Option<String>,
}

/// The incoming [`ProfileInput`] as a [`StoredProfile`].
fn as_stored(profile: &ProfileInput) -> StoredProfile {
    StoredProfile {
        id: profile.id.clone(),
        label: profile.label.clone(),
        host: profile.host.clone(),
        port: profile.port,
        database: profile.database.clone(),
        user: profile.user.clone(),
        encryption: profile.encryption.clone(),
        tls_fingerprint: profile.tls_fingerprint.clone(),
        plaintext_reason: profile.plaintext_reason.clone(),
    }
}

/// Whether any *connection* field differs between two profiles.  `label` and
/// `id` are presentation/identity, not connection targets, so a label-only
/// change does not count.
fn connection_fields_differ(existing: &StoredProfile, incoming: &StoredProfile) -> bool {
    existing.host != incoming.host
        || existing.port != incoming.port
        || existing.database != incoming.database
        || existing.user != incoming.user
        || existing.encryption != incoming.encryption
        || existing.tls_fingerprint != incoming.tls_fingerprint
        || existing.plaintext_reason != incoming.plaintext_reason
}

/// Save (insert or update) a profile, applying the saved-password safety rule.
pub fn do_save_profile(
    profile: ProfileInput,
    password: Option<String>,
    state: &AppState,
) -> Result<SaveProfileResult, CommandError> {
    validate_profile(&profile)?;
    let incoming = as_stored(&profile);

    let existing = state.store()?.get(&incoming.id)?;
    let is_update = existing.is_some();
    let fields_changed = existing
        .as_ref()
        .is_some_and(|ex| connection_fields_differ(ex, &incoming));

    let mut warning = None;

    // Fail-closed password hygiene: whenever this call will leave the id
    // WITHOUT a freshly-supplied password, any password that may still exist
    // for it must be removed BEFORE the profile changes on disk, so a stale
    // secret can never be reused against new connection details.
    if password.is_none() {
        //   - an update that changed connection fields, or
        //   - a brand-new profile (whose id may still hold a leftover entry
        //     orphaned by an earlier deletion),
        // both require clearing any existing entry.
        let must_clear = !is_update || fields_changed;
        if must_clear {
            state
                .keychain()?
                .delete(&incoming.id)
                .map_err(|_| CommandError::PasswordRemoveFailed)?;
            // Only warn when we dropped a password the user knew about (an
            // update), not when clearing an unreachable orphan on a new profile.
            if is_update {
                warning = Some(PASSWORD_REMOVED_MESSAGE.to_string());
            }
        }
    }

    let stored = state.store()?.save(&incoming)?;

    // A supplied password goes to the keychain (never SQLite).  A failure is
    // surfaced as a warning, never as a silent drop.
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

/// Save (insert or update) a profile.  The optional `password`, when present,
/// is stored in the keychain only (never SQLite).
#[tauri::command]
pub fn save_profile(
    profile: ProfileInput,
    password: Option<String>,
    state: State<'_, AppState>,
) -> Result<SaveProfileResult, CommandError> {
    do_save_profile(profile, password, &state)
}

/// Delete a profile and its keychain password, keychain-first and fail-closed.
///
/// The keychain entry is removed BEFORE the SQLite row so a failed delete can
/// abort (row untouched) and never leave an orphaned password that a later
/// save of the same id could reuse.
pub fn do_delete_profile(id: &str, state: &AppState) -> Result<bool, CommandError> {
    match state.store()?.get(id)? {
        Some(_) => {}
        None => return Ok(false),
    }
    state
        .keychain()?
        .delete(id)
        .map_err(|_| CommandError::PasswordRemoveFailed)?;
    state.store()?.delete(id)?;
    Ok(true)
}

/// Delete a profile and its keychain password.
#[tauri::command]
pub fn delete_profile(id: String, state: State<'_, AppState>) -> Result<bool, CommandError> {
    do_delete_profile(&id, &state)
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
    use crate::secrets::{InMemoryPasswordStore, PasswordStore};
    use crate::state::AppState;

    fn profile(overrides: impl FnOnce(&mut ProfileInput)) -> ProfileInput {
        let mut p = ProfileInput {
            id: "prod".to_string(),
            label: "Production".to_string(),
            host: "127.0.0.1".to_string(),
            port: 5432,
            database: "bank".to_string(),
            user: "dmux_dev".to_string(),
            encryption: "verify".to_string(),
            tls_fingerprint: None,
            plaintext_reason: None,
        };
        overrides(&mut p);
        p
    }

    fn test_state() -> AppState {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().to_path_buf();
        // Keep the tempdir alive for the life of the store's open connection.
        std::mem::forget(dir);
        AppState::with_password_store(path, Box::new(InMemoryPasswordStore::new())).expect("state")
    }

    /// The password `resolve`/`get` would see for this profile under the current
    /// keychain state: `Some` if present, `Err(Missing)` if removed or absent.
    fn saved_password(state: &AppState, id: &str) -> Result<String, CommandError> {
        state.keychain().unwrap().get(id)
    }

    #[test]
    fn rejects_empty_host() {
        let mut p = profile(|_| {});
        p.host = "   ".to_string();
        assert!(validate_profile(&p).is_err());

        p.host = "db".to_string();
        p.port = 0;
        assert!(validate_profile(&p).is_err());
    }

    // --- The saved-password safety rule ---

    #[test]
    fn host_change_without_a_password_removes_the_saved_password() {
        let s = test_state();
        // Seed an existing profile with a stored password.
        do_save_profile(profile(|_| {}), Some("s3cr3t".to_string()), &s).unwrap();

        // Change the host without supplying a password.
        let res = do_save_profile(
            profile(|p| p.host = "evil.example.com".to_string()),
            None,
            &s,
        )
        .unwrap();
        assert!(
            res.warning
                .as_deref()
                .is_some_and(|w| w.contains("password removed")),
            "expected a removal warning, got {:?}",
            res.warning
        );

        // The keychain entry is gone: a later read reports missing, never the
        // old password.
        assert!(matches!(
            saved_password(&s, "prod"),
            Err(CommandError::PasswordMissing { .. })
        ));
    }

    #[test]
    fn plaintext_downgrade_without_a_password_removes_the_saved_password() {
        let s = test_state();
        do_save_profile(profile(|_| {}), Some("s3cr3t".to_string()), &s).unwrap();

        let res = do_save_profile(
            profile(|p| {
                p.encryption = "allow-plaintext".to_string();
                p.plaintext_reason = Some("attacker".to_string());
            }),
            None,
            &s,
        )
        .unwrap();
        assert!(res
            .warning
            .as_deref()
            .is_some_and(|w| w.contains("password removed")));
        assert!(matches!(
            saved_password(&s, "prod"),
            Err(CommandError::PasswordMissing { .. })
        ));
    }

    #[test]
    fn trust_pin_without_a_password_removes_the_saved_password() {
        let s = test_state();
        do_save_profile(profile(|_| {}), Some("s3cr3t".to_string()), &s).unwrap();

        let res = do_save_profile(
            profile(|p| {
                p.encryption = "trust".to_string();
                p.tls_fingerprint = Some("deadbeef".to_string());
            }),
            None,
            &s,
        )
        .unwrap();
        assert!(res
            .warning
            .as_deref()
            .is_some_and(|w| w.contains("password removed")));
        assert!(matches!(
            saved_password(&s, "prod"),
            Err(CommandError::PasswordMissing { .. })
        ));
    }

    #[test]
    fn label_only_change_keeps_the_saved_password() {
        let s = test_state();
        do_save_profile(profile(|_| {}), Some("s3cr3t".to_string()), &s).unwrap();

        let res = do_save_profile(
            profile(|p| p.label = "Prod (renamed)".to_string()),
            None,
            &s,
        )
        .unwrap();
        assert!(
            res.warning.is_none(),
            "label-only change must keep the password"
        );
        assert_eq!(saved_password(&s, "prod").unwrap(), "s3cr3t");
    }

    #[test]
    fn host_change_with_a_new_password_stores_the_new_password() {
        let s = test_state();
        do_save_profile(profile(|_| {}), Some("old-pw".to_string()), &s).unwrap();

        let res = do_save_profile(
            profile(|p| p.host = "new.example.com".to_string()),
            Some("new-pw".to_string()),
            &s,
        )
        .unwrap();
        // A new password was supplied, so no removal warning is expected.
        assert!(
            res.warning
                .as_deref()
                .is_none_or(|w| !w.contains("removed")),
            "unexpected removal warning: {:?}",
            res.warning
        );
        assert_eq!(saved_password(&s, "prod").unwrap(), "new-pw");
    }

    #[test]
    fn new_profile_save_does_not_emit_a_removal_warning() {
        let s = test_state();
        let res = do_save_profile(profile(|_| {}), Some("pw".to_string()), &s).unwrap();
        assert!(
            res.warning.is_none(),
            "a brand-new profile has nothing to remove"
        );
        assert_eq!(saved_password(&s, "prod").unwrap(), "pw");
    }

    // --- Fail-closed password removal ---

    fn failing_delete_state() -> AppState {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().to_path_buf();
        std::mem::forget(dir);
        AppState::with_password_store(path, Box::new(InMemoryPasswordStore::with_failing_delete()))
            .expect("state")
    }

    #[test]
    fn failed_password_removal_aborts_the_save_and_leaves_the_profile_unchanged() {
        let s = failing_delete_state();
        // Seed an existing profile with a stored password.
        do_save_profile(profile(|_| {}), Some("s3cr3t".to_string()), &s).unwrap();

        // Changing the host without a new password must try to delete the old
        // password; when the delete fails, the whole save is aborted.
        let err = do_save_profile(
            profile(|p| p.host = "evil.example.com".to_string()),
            None,
            &s,
        )
        .unwrap_err();
        assert!(
            matches!(err, CommandError::PasswordRemoveFailed),
            "got {err:?}"
        );

        // The on-disk profile is unchanged (still the original host).
        let stored = s.store().unwrap().get("prod").unwrap().unwrap();
        assert_eq!(stored.host, "127.0.0.1");

        // The old password is still present (never reused, never silently left
        // paired with a changed profile).
        assert_eq!(saved_password(&s, "prod").unwrap(), "s3cr3t");
    }

    #[test]
    fn deleting_a_missing_entry_succeeds() {
        let mut store = InMemoryPasswordStore::new();
        // "No such entry" is success, matching the OS keychain's NoEntry -> Ok.
        store.delete("never-saved").unwrap();
        assert!(!store.contains("never-saved"));
    }

    // --- delete_profile: keychain-first, fail-closed ---

    #[test]
    fn delete_profile_with_failing_keychain_delete_aborts_and_keeps_the_row_and_password() {
        let s = failing_delete_state();
        do_save_profile(profile(|_| {}), Some("s3cr3t".to_string()), &s).unwrap();

        let err = do_delete_profile("prod", &s).unwrap_err();
        assert!(
            matches!(err, CommandError::PasswordRemoveFailed),
            "got {err:?}"
        );

        // The SQLite row is untouched.
        assert!(s.store().unwrap().get("prod").unwrap().is_some());
        // The password is still present (not orphaned against a deleted row).
        assert_eq!(saved_password(&s, "prod").unwrap(), "s3cr3t");
    }

    #[test]
    fn delete_profile_removes_the_row_and_password_when_delete_succeeds() {
        let s = test_state();
        do_save_profile(profile(|_| {}), Some("s3cr3t".to_string()), &s).unwrap();

        assert!(do_delete_profile("prod", &s).unwrap());
        assert!(s.store().unwrap().get("prod").unwrap().is_none());
        assert!(matches!(
            saved_password(&s, "prod"),
            Err(CommandError::PasswordMissing { .. })
        ));
    }

    #[test]
    fn delete_profile_of_a_missing_profile_returns_false() {
        let s = test_state();
        assert!(!do_delete_profile("nope", &s).unwrap());
    }

    // --- New profile must clear a leftover keychain entry ---

    #[test]
    fn new_profile_without_a_password_clears_a_leftover_keychain_entry() {
        let s = test_state();
        // Simulate an orphaned password left behind for this id (e.g. from a
        // previously-deleted profile) — no store row exists yet.
        s.keychain().unwrap().set("prod", "orphan-pw").unwrap();

        // Saving a NEW profile with no password must clear the leftover entry
        // so it can never be reused against the new profile.
        let res = do_save_profile(profile(|_| {}), None, &s).unwrap();
        assert!(
            res.warning.is_none(),
            "no user-facing warning for an orphan"
        );
        assert!(matches!(
            saved_password(&s, "prod"),
            Err(CommandError::PasswordMissing { .. })
        ));
        assert!(s.store().unwrap().get("prod").unwrap().is_some());
    }

    #[test]
    fn new_profile_without_a_password_aborts_if_the_leftover_delete_fails() {
        let s = failing_delete_state();
        // Leftover entry, and a keychain whose delete always fails.
        s.keychain().unwrap().set("prod", "orphan-pw").unwrap();

        let err = do_save_profile(profile(|_| {}), None, &s).unwrap_err();
        assert!(
            matches!(err, CommandError::PasswordRemoveFailed),
            "got {err:?}"
        );

        // The save was aborted: no store row was created.
        assert!(s.store().unwrap().get("prod").unwrap().is_none());
        // The leftover password was not silently left paired with a new profile.
        assert_eq!(saved_password(&s, "prod").unwrap(), "orphan-pw");
    }
}
