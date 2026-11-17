//! Passwords live in the operating system keychain, never in SQLite, logs or
//! the UI state (PLAN 6.5 / AGENTS.md hard rule 7).
//!
//! [`Keychain`] wraps the `keyring` crate (Windows Credential Manager, or the
//! Secret Service on Linux).  Where no usable keychain exists, callers fall back
//! to asking the user for the password on each run (PLAN 4.1): a [`PasswordStore`]
//! read returns a missing-password error, which the frontend turns into a prompt.
//!
//! The operations are behind the [`PasswordStore`] trait so command logic can be
//! unit-tested against an in-memory store without touching the OS keychain.

use crate::error::{CommandError, PasswordMissing};

#[cfg(test)]
use std::collections::HashMap;

/// The keyring service name, namespacing every credential we store.
const SERVICE: &str = "dmux-verify";

/// The plain-language message shown when a password could not be stored.
pub const NOT_SAVED_MESSAGE: &str = "password not saved; you will be asked each run";

/// The plain-language warning shown when connection details changed but no new
/// password was supplied, so the saved password was removed (see `save_profile`).
pub const PASSWORD_REMOVED_MESSAGE: &str =
    "connection details changed; saved password removed — you will be asked for it";

/// Abstraction over where a profile's password lives.  Both the OS keychain and
/// an in-memory test double implement it.
pub trait PasswordStore: Send + 'static {
    /// Store a password for `key`.
    fn set(&mut self, key: &str, password: &str) -> Result<(), CommandError>;
    /// Read a password for `key`, or a missing-password error.
    fn get(&mut self, key: &str) -> Result<String, CommandError>;
    /// Remove a stored password (best-effort; deleting a missing entry is a no-op).
    fn delete(&mut self, key: &str) -> Result<(), CommandError>;
}

/// A keychain-backed password store for the running app.
#[derive(Debug, Clone)]
pub struct Keychain {
    /// Set to `false` when password storage/retrieval has been determined to be
    /// unusable on this machine, so every later call short-circuits to a clean
    /// "ask each run" result instead of erroring noisily.
    available: bool,
}

impl Default for Keychain {
    fn default() -> Self {
        Self { available: true }
    }
}

impl Keychain {
    /// The deterministic keyring account for a profile: the app-scoped service
    /// plus a stable per-profile key (the profile's SQLite primary key).
    ///
    /// Returns a typed error for an invalid (e.g. empty) profile key instead of
    /// panicking.
    pub fn entry(profile_key: &str) -> Result<keyring::Entry, CommandError> {
        keyring::Entry::new(SERVICE, profile_key).map_err(|e| CommandError::Keychain {
            message: format!("invalid profile id: {e}"),
        })
    }

    /// A keychain already known to be unusable (test helper).
    #[cfg(test)]
    fn unavailable() -> Self {
        Self { available: false }
    }
}

impl PasswordStore for Keychain {
    /// Store a password.  On success the password never touches disk outside the
    /// OS keychain.  On failure (no usable keychain) an error is returned so the
    /// caller can surface a typed warning; the keychain is then marked unusable
    /// so later reads fall back to asking each run.
    fn set(&mut self, profile_key: &str, password: &str) -> Result<(), CommandError> {
        if !self.available {
            return Err(CommandError::Keychain {
                message: NOT_SAVED_MESSAGE.to_string(),
            });
        }
        match Self::entry(profile_key)?.set_password(password) {
            Ok(()) => Ok(()),
            Err(_) => {
                self.available = false;
                Err(CommandError::Keychain {
                    message: NOT_SAVED_MESSAGE.to_string(),
                })
            }
        }
    }

    /// Read a password back.  A missing entry or an unusable keychain becomes a
    /// missing-password error with a distinct [`PasswordMissing`] reason.
    fn get(&mut self, profile_key: &str) -> Result<String, CommandError> {
        if !self.available {
            return Err(CommandError::PasswordMissing {
                reason: PasswordMissing::NoKeychain,
                profile: profile_key.to_string(),
            });
        }
        match Self::entry(profile_key)?.get_password() {
            Ok(password) => Ok(password),
            Err(keyring::Error::NoEntry) => Err(CommandError::PasswordMissing {
                reason: PasswordMissing::NotStored,
                profile: profile_key.to_string(),
            }),
            Err(_) => {
                // Any other keyring failure is treated as "no keychain" so the
                // UI falls back to asking each run rather than blocking.
                self.available = false;
                Err(CommandError::PasswordMissing {
                    reason: PasswordMissing::NoKeychain,
                    profile: profile_key.to_string(),
                })
            }
        }
    }

    /// Remove a stored password (best-effort; deleting a non-entry is a no-op).
    fn delete(&mut self, profile_key: &str) -> Result<(), CommandError> {
        if !self.available {
            return Ok(());
        }
        match Self::entry(profile_key)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(_) => {
                // Mark the keychain unusable so a later read never returns a
                // stale password that failed to delete.
                self.available = false;
                Ok(())
            }
        }
    }
}

/// An in-memory [`PasswordStore`] for tests: deterministic, observable, and
/// never touches a real OS keychain.
#[cfg(test)]
pub struct InMemoryPasswordStore {
    entries: HashMap<String, String>,
}

#[cfg(test)]
impl InMemoryPasswordStore {
    pub fn new() -> Self {
        Self {
            entries: HashMap::new(),
        }
    }

    /// Whether a password is currently stored for `key`.
    pub fn contains(&self, key: &str) -> bool {
        self.entries.contains_key(key)
    }
}

#[cfg(test)]
impl Default for InMemoryPasswordStore {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
impl PasswordStore for InMemoryPasswordStore {
    fn set(&mut self, key: &str, password: &str) -> Result<(), CommandError> {
        self.entries.insert(key.to_string(), password.to_string());
        Ok(())
    }

    fn get(&mut self, key: &str) -> Result<String, CommandError> {
        self.entries
            .get(key)
            .cloned()
            .ok_or_else(|| CommandError::PasswordMissing {
                reason: PasswordMissing::NotStored,
                profile: key.to_string(),
            })
    }

    fn delete(&mut self, key: &str) -> Result<(), CommandError> {
        self.entries.remove(key);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entry_returns_a_typed_result_not_a_panic() {
        // `Keychain::entry` must never panic (no `.expect` on the keyring call).
        // A keychain-less host yields a typed `CommandError::Keychain`; an
        // available one yields `Ok`.  Either way it is a typed `Result`.
        match Keychain::entry("some-profile-id") {
            Ok(_) => {}
            Err(CommandError::Keychain { .. }) => {}
            Err(other) => panic!("unexpected error kind: {other:?}"),
        }
    }

    #[test]
    fn set_surfaces_a_typed_warning_when_unavailable() {
        let mut kc = Keychain::unavailable();
        let err = kc.set("x", "pw").unwrap_err();
        match err {
            CommandError::Keychain { message } => {
                assert!(message.contains("asked each run"), "message: {message}");
            }
            other => panic!("expected keychain error, got {other:?}"),
        }
    }

    #[test]
    fn get_reports_no_keychain_when_unavailable() {
        let mut kc = Keychain::unavailable();
        let err = kc.get("x").unwrap_err();
        assert!(matches!(
            err,
            CommandError::PasswordMissing {
                reason: PasswordMissing::NoKeychain,
                ..
            }
        ));
    }

    #[test]
    fn in_memory_store_round_trips_and_deletes() {
        let mut store = InMemoryPasswordStore::new();
        assert!(!store.contains("k"));
        store.set("k", "pw").unwrap();
        assert!(store.contains("k"));
        assert_eq!(store.get("k").unwrap(), "pw");
        store.delete("k").unwrap();
        assert!(!store.contains("k"));
        assert!(matches!(
            store.get("k"),
            Err(CommandError::PasswordMissing { .. })
        ));
    }
}
