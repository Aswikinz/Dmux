//! Passwords live in the operating system keychain, never in SQLite, logs or
//! the UI state (PLAN 6.5 / AGENTS.md hard rule 7).
//!
//! [`Keychain`] wraps the `keyring` crate (Windows Credential Manager, or the
//! Secret Service on Linux).  Where no usable keychain exists, callers fall back
//! to asking the user for the password on each run (PLAN 4.1): [`Keychain::get`]
//! returns [`Error::Missing`], which the frontend turns into a prompt.

use crate::error::{CommandError, PasswordMissing};

/// The keyring service name, namespacing every credential we store.
const SERVICE: &str = "dmux-verify";

/// A keychain-backed password store.
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
    pub fn entry(profile_key: &str) -> keyring::Entry {
        keyring::Entry::new(SERVICE, profile_key).expect("static service name is valid")
    }

    /// Store a password.  On success the password never touches disk outside the
    /// OS keychain.  A keychain-less host is recorded and reported as missing so
    /// the UI can ask each run.
    pub fn set(&mut self, profile_key: &str, password: &str) -> Result<(), CommandError> {
        if !self.available {
            return Ok(());
        }
        match Self::entry(profile_key).set_password(password) {
            Ok(()) => Ok(()),
            Err(_) => {
                // No keychain on this machine: degrade gracefully (PLAN 4.1).
                self.available = false;
                Ok(())
            }
        }
    }

    /// Read a password back.  A missing entry or an unusable keychain becomes
    /// [`Error::Missing`] with a distinct [`PasswordMissing`] reason.
    pub fn get(&mut self, profile_key: &str) -> Result<String, CommandError> {
        if !self.available {
            return Err(CommandError::PasswordMissing {
                reason: PasswordMissing::NoKeychain,
                profile: profile_key.to_string(),
            });
        }
        match Self::entry(profile_key).get_password() {
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
    pub fn delete(&mut self, profile_key: &str) -> Result<(), CommandError> {
        if !self.available {
            return Ok(());
        }
        match Self::entry(profile_key).delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(_) => {
                self.available = false;
                Ok(())
            }
        }
    }
}
