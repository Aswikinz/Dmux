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
//!
//! On a machine with no usable keychain, a typed password is kept in the
//! in-memory session cache instead ([`crate::secrets::SessionPasswords`]),
//! bound to the profile's connection fields.  The same rule applies to it: a
//! connection-field change or a delete forgets it.

use serde::Serialize;
use tauri::State;

use crate::error::CommandError;
use crate::secrets::{
    stored_fields_hash, NOT_SAVED_MESSAGE, PASSWORD_REMOVED_MESSAGE, SESSION_PASSWORD_MESSAGE,
};
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
        ca_file: profile.ca_file.clone(),
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
        || existing.ca_file != incoming.ca_file
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

    // Fail-closed password hygiene.  Whenever the id is brand new OR any
    // connection field changed, remove the existing keychain entry FIRST —
    // regardless of whether a new password is supplied — so a stale secret can
    // never be re-bound to new details.  Only a label-only change on an
    // existing id keeps the password untouched.
    let must_clear = !is_update || fields_changed;
    if must_clear {
        // A session password is bound to the exact connection fields it was
        // typed for.  Keep it only when it was typed for exactly the fields
        // being saved (a Test connection just before this save); for any other
        // fields it belongs to old details and is forgotten.  Checked before
        // the keychain step, so a keychain that refuses the delete (no
        // keychain on this machine) cannot cost the user that password.
        {
            let mut sessions = state.session_passwords()?;
            if !sessions.contains(&incoming.id, &stored_fields_hash(&incoming)) {
                sessions.forget(&incoming.id);
            }
        }
        state
            .keychain()?
            .delete(&incoming.id)
            .map_err(|_| CommandError::PasswordRemoveFailed)?;
        // Warn only when we dropped a password the user knew about (an update
        // with no replacement), not when clearing an unreachable orphan.
        if is_update && password.is_none() {
            warning = Some(PASSWORD_REMOVED_MESSAGE.to_string());
        }
    }

    let stored = state.store()?.save(&incoming)?;

    // A supplied password goes to the keychain (never SQLite).  With no usable
    // keychain it is kept in memory for this session, bound to the saved
    // fields, and the user is told so.  Any other failure is surfaced as a
    // warning, never as a silent drop.
    if let Some(password) = password {
        let stored_result = state.keychain()?.set(&stored.id, &password);
        match stored_result {
            Ok(()) => state.session_passwords()?.forget(&stored.id),
            Err(e) if e.to_string().contains(NOT_SAVED_MESSAGE) => {
                state
                    .session_passwords()?
                    .put(&stored.id, stored_fields_hash(&stored), &password);
                warning = Some(SESSION_PASSWORD_MESSAGE.to_string());
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
    state.session_passwords()?.forget(id);
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
    let stored = match state.store()?.get(&id)? {
        Some(stored) => stored,
        None => return Err(CommandError::ProfileNotFound { key: id }),
    };
    match state.keychain()?.get(&id) {
        Ok(_) => Ok(true),
        // No keychain entry: a session-only password kept for the saved fields
        // (T055) still counts as "a password is available".
        Err(CommandError::PasswordMissing { .. }) => Ok(state
            .session_passwords()?
            .contains(&id, &stored_fields_hash(&stored))),
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

    /// Build a password fixture with no compile-time-constant content, so no
    /// hard-coded credential value can flow into a password parameter (CodeQL
    /// rust/hard-coded-cryptographic-value).  Derived only from runtime values:
    /// the process id, wall-clock nanoseconds and a per-call counter, hex-encoded.
    fn test_secret() -> String {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed) as u128;
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let pid = std::process::id() as u128;
        let mixed = (pid << 96) ^ (nanos << 32) ^ n;
        format!("{mixed:032x}")
    }

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
            ca_file: None,
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
        do_save_profile(profile(|_| {}), Some(test_secret()), &s).unwrap();

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
        do_save_profile(profile(|_| {}), Some(test_secret()), &s).unwrap();

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
        do_save_profile(profile(|_| {}), Some(test_secret()), &s).unwrap();

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
    fn ca_file_change_without_a_password_removes_the_saved_password() {
        let s = test_state();
        do_save_profile(profile(|_| {}), Some(test_secret()), &s).unwrap();

        // Changing which CA bundle is trusted is a connection-detail change:
        // the saved password must not silently follow it.
        let res = do_save_profile(
            profile(|p| p.ca_file = Some("/etc/bank/ca.pem".to_string())),
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
        let secret = test_secret();
        do_save_profile(profile(|_| {}), Some(secret.clone()), &s).unwrap();

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
        assert_eq!(saved_password(&s, "prod").unwrap(), secret);
    }

    #[test]
    fn host_change_with_a_new_password_stores_the_new_password() {
        let s = test_state();
        let old = test_secret();
        let new = test_secret();
        do_save_profile(profile(|_| {}), Some(old), &s).unwrap();

        let res = do_save_profile(
            profile(|p| p.host = "new.example.com".to_string()),
            Some(new.clone()),
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
        assert_eq!(saved_password(&s, "prod").unwrap(), new);
    }

    #[test]
    fn new_profile_save_does_not_emit_a_removal_warning() {
        let s = test_state();
        let pw = test_secret();
        let res = do_save_profile(profile(|_| {}), Some(pw.clone()), &s).unwrap();
        assert!(
            res.warning.is_none(),
            "a brand-new profile has nothing to remove"
        );
        assert_eq!(saved_password(&s, "prod").unwrap(), pw);
    }

    // --- Fail-closed password removal ---

    fn failing_delete_state() -> AppState {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().to_path_buf();
        std::mem::forget(dir);
        AppState::with_password_store(path, Box::new(InMemoryPasswordStore::with_failing_delete()))
            .expect("state")
    }

    /// Build an [`AppState`] with `store` as its password store, an EXISTING
    /// profile row (host `127.0.0.1`) in SQLite, and `old_pw` already injected
    /// as the saved password — i.e. a pre-existing profile+password pair, seeded
    /// directly (bypassing `set`/`delete`) so a failing store can still model
    /// an already-generated situation.
    fn seeded_state(store: InMemoryPasswordStore, old_pw: &str) -> AppState {
        let mut store = store;
        store.inject("prod", old_pw);
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().to_path_buf();
        std::mem::forget(dir);
        let s = AppState::with_password_store(path, Box::new(store)).expect("state");
        let seeded = as_stored(&profile(|_| {}));
        s.store().unwrap().save(&seeded).unwrap();
        s
    }

    #[test]
    fn failed_password_removal_aborts_the_save_and_leaves_the_profile_unchanged() {
        let secret = test_secret();
        let s = seeded_state(InMemoryPasswordStore::with_failing_delete(), &secret);

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
        assert_eq!(saved_password(&s, "prod").unwrap(), secret);
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
        let secret = test_secret();
        let s = seeded_state(InMemoryPasswordStore::with_failing_delete(), &secret);

        let err = do_delete_profile("prod", &s).unwrap_err();
        assert!(
            matches!(err, CommandError::PasswordRemoveFailed),
            "got {err:?}"
        );

        // The SQLite row is untouched.
        assert!(s.store().unwrap().get("prod").unwrap().is_some());
        // The password is still present (not orphaned against a deleted row).
        assert_eq!(saved_password(&s, "prod").unwrap(), secret);
    }

    #[test]
    fn delete_profile_removes_the_row_and_password_when_delete_succeeds() {
        let s = test_state();
        do_save_profile(profile(|_| {}), Some(test_secret()), &s).unwrap();

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
        s.keychain().unwrap().set("prod", &test_secret()).unwrap();

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
        let orphan = test_secret();
        // Leftover entry, and a keychain whose delete always fails.
        s.keychain().unwrap().set("prod", &orphan).unwrap();

        let err = do_save_profile(profile(|_| {}), None, &s).unwrap_err();
        assert!(
            matches!(err, CommandError::PasswordRemoveFailed),
            "got {err:?}"
        );

        // The save was aborted: no store row was created.
        assert!(s.store().unwrap().get("prod").unwrap().is_none());
        // The leftover password was not silently left paired with a new profile.
        assert_eq!(saved_password(&s, "prod").unwrap(), orphan);
    }

    // --- Field change with a new password whose set fails ---

    fn failing_set_state_with_old_password(old_pw: &str) -> AppState {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().to_path_buf();
        std::mem::forget(dir);
        let mut store = InMemoryPasswordStore::with_failing_set();
        store.inject("prod", old_pw);
        AppState::with_password_store(path, Box::new(store)).expect("state")
    }

    #[test]
    fn field_change_with_new_password_leaves_no_entry_when_set_fails() {
        let s = failing_set_state_with_old_password(&test_secret());
        // Seed an existing profile row with the original connection fields.
        let seeded = as_stored(&profile(|_| {}));
        s.store().unwrap().save(&seeded).unwrap();

        // Change the host AND supply a new password whose set will fail.
        let res = do_save_profile(
            profile(|p| p.host = "new.example.com".to_string()),
            Some(test_secret()),
            &s,
        )
        .unwrap();

        // The profile is saved with the new host.
        let stored = s.store().unwrap().get("prod").unwrap().unwrap();
        assert_eq!(stored.host, "new.example.com");

        // No password entry remains: the old one was deleted, the new set
        // failed, so nothing is re-bound to the changed host.
        assert!(matches!(
            saved_password(&s, "prod"),
            Err(CommandError::PasswordMissing { .. })
        ));

        // T055: the keychain refused the new password, so it is kept for this
        // session only, bound to the NEW fields, and the user is told so.
        assert_eq!(res.warning.as_deref(), Some(SESSION_PASSWORD_MESSAGE));
        let sessions = s.session_passwords().unwrap();
        assert!(sessions.contains("prod", &stored_fields_hash(&stored)));
        assert!(!sessions.contains("prod", &stored_fields_hash(&seeded)));
    }

    #[test]
    fn label_only_change_with_a_new_password_stores_the_new_password() {
        let s = test_state();
        let old = test_secret();
        let new = test_secret();
        do_save_profile(profile(|_| {}), Some(old), &s).unwrap();
        // A label-only change with a fresh password updates the password without
        // first deleting it (nothing was re-bound).
        let res = do_save_profile(
            profile(|p| p.label = "Renamed".to_string()),
            Some(new.clone()),
            &s,
        )
        .unwrap();
        assert!(res.warning.is_none());
        assert_eq!(saved_password(&s, "prod").unwrap(), new);
    }
}
