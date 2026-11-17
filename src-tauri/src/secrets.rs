//! Passwords live in the operating system keychain, never in SQLite, logs or
//! the UI state (PLAN 6.5 / AGENTS.md hard rule 7).
//!
//! [`Keychain`] wraps the `keyring` crate (Windows Credential Manager, or the
//! Secret Service on Linux).  Where no usable keychain exists (PLAN 4.1), a
//! password the user types is kept for the app session only in
//! [`SessionPasswords`]: memory only, zeroed on drop, bound to the exact
//! connection fields it was typed for, and cleared when the app exits.  The UI
//! never holds it after the call that sent it.
//!
//! The operations are behind the [`PasswordStore`] trait so command logic can be
//! unit-tested against an in-memory store without touching the OS keychain.

use std::collections::HashMap;

use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

use crate::error::{CommandError, PasswordMissing};
use crate::store::StoredProfile;
use crate::types::ProfileInput;

/// The keyring service name, namespacing every credential we store.
const SERVICE: &str = "dmux-verify";

/// The plain-language message shown when a password could not be stored.
pub const NOT_SAVED_MESSAGE: &str = "password not saved; you will be asked each run";

/// The plain-language warning shown when connection details changed but no new
/// password was supplied, so the saved password was removed (see `save_profile`).
pub const PASSWORD_REMOVED_MESSAGE: &str =
    "connection details changed; saved password removed — you will be asked for it";

/// The plain-language notice shown when a password is kept for this session
/// only, because the machine has no usable keychain (PLAN 4.1, T055).
pub const SESSION_PASSWORD_MESSAGE: &str =
    "No system keychain: Dmux will keep this password in memory until you close the app.";

/// The connection fields a password is bound to.  A password typed for one set
/// of fields is never sent anywhere else (PLAN 6.5): the session cache is keyed
/// by a hash of exactly these.
#[derive(serde::Serialize)]
struct ConnectionFields<'a> {
    host: &'a str,
    port: u16,
    database: &'a str,
    user: &'a str,
    encryption: &'a str,
    tls_fingerprint: Option<&'a str>,
    plaintext_reason: Option<&'a str>,
    ca_file: Option<&'a str>,
}

impl ConnectionFields<'_> {
    /// SHA-256 over the fields' canonical JSON (unambiguous: every field is
    /// quoted and delimited).
    fn hash(&self) -> String {
        let canonical = serde_json::to_vec(self).unwrap_or_default();
        hex::encode(Sha256::digest(&canonical))
    }
}

/// The connection-fields hash of a profile as the UI sent it.
pub fn input_fields_hash(p: &ProfileInput) -> String {
    ConnectionFields {
        host: &p.host,
        port: p.port,
        database: &p.database,
        user: &p.user,
        encryption: &p.encryption,
        tls_fingerprint: p.tls_fingerprint.as_deref(),
        plaintext_reason: p.plaintext_reason.as_deref(),
        ca_file: p.ca_file.as_deref(),
    }
    .hash()
}

/// The connection-fields hash of a saved profile.
pub fn stored_fields_hash(p: &StoredProfile) -> String {
    ConnectionFields {
        host: &p.host,
        port: p.port,
        database: &p.database,
        user: &p.user,
        encryption: &p.encryption,
        tls_fingerprint: p.tls_fingerprint.as_deref(),
        plaintext_reason: p.plaintext_reason.as_deref(),
        ca_file: p.ca_file.as_deref(),
    }
    .hash()
}

/// Session-only passwords for machines with no usable keychain (PLAN 4.1:
/// "where no keychain exists, Dmux asks for the password at each run"; here a
/// run is the app session).
///
/// - Memory only: never written to SQLite, logs, errors or the UI's state, and
///   never serialised (no `Serialize`; `Debug` shows only the entry count).
/// - One entry per profile id, bound to the hash of the connection fields the
///   password was typed for.  A lookup with any other fields misses, so the
///   password can never follow a changed host, port, user or TLS setting.
/// - The secret is zeroed when it is replaced, forgotten or cleared, and the
///   whole cache is cleared when the app exits.
#[derive(Default)]
pub struct SessionPasswords {
    entries: HashMap<String, (String, Zeroizing<String>)>,
}

impl std::fmt::Debug for SessionPasswords {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SessionPasswords")
            .field("entries", &self.entries.len())
            .finish()
    }
}

impl SessionPasswords {
    /// Keep `password` for profile `id`, bound to `fields_hash`.  Replaces (and
    /// zeroes) any earlier entry for the id.
    pub fn put(&mut self, id: &str, fields_hash: String, password: &str) {
        self.entries.insert(
            id.to_string(),
            (fields_hash, Zeroizing::new(password.to_string())),
        );
    }

    /// The password for `id`, only when it was kept for exactly `fields_hash`.
    pub fn get(&self, id: &str, fields_hash: &str) -> Option<String> {
        self.entries
            .get(id)
            .filter(|(h, _)| h == fields_hash)
            .map(|(_, pw)| pw.as_str().to_string())
    }

    /// Whether a password is kept for `id` with exactly `fields_hash`.
    pub fn contains(&self, id: &str, fields_hash: &str) -> bool {
        self.entries.get(id).is_some_and(|(h, _)| h == fields_hash)
    }

    /// Drop (and zero) the password for `id`.
    pub fn forget(&mut self, id: &str) {
        self.entries.remove(id);
    }

    /// Drop (and zero) every password: on app exit.
    pub fn clear(&mut self) {
        self.entries.clear();
    }

    /// How many passwords are kept.
    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether no password is kept.
    #[cfg(test)]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

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
        // An entry that cannot even be created means no usable keychain here
        // (for example no session bus): the same as a failed store.
        let stored = Self::entry(profile_key).map(|e| e.set_password(password));
        match stored {
            Ok(Ok(())) => Ok(()),
            _ => {
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
        let read = Self::entry(profile_key).map(|e| e.get_password());
        match read {
            Ok(Ok(password)) => Ok(password),
            Ok(Err(keyring::Error::NoEntry)) => Err(CommandError::PasswordMissing {
                reason: PasswordMissing::NotStored,
                profile: profile_key.to_string(),
            }),
            _ => {
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

    /// Remove a stored password.
    ///
    /// Fails closed: any error other than "no such entry" is returned so the
    /// caller knows the credential may still exist.  This always attempts the
    /// real OS delete — even after a previous operation marked the keychain
    /// unusable — because the credential may have been stored before the flag
    /// was set, and a stale password that survives a restart must never be
    /// silently reused against changed connection details.
    fn delete(&mut self, profile_key: &str) -> Result<(), CommandError> {
        match Self::entry(profile_key)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(CommandError::Keychain {
                message: e.to_string(),
            }),
        }
    }
}

/// An in-memory [`PasswordStore`] for tests: deterministic, observable, and
/// never touches a real OS keychain.
#[cfg(test)]
pub struct InMemoryPasswordStore {
    entries: HashMap<String, String>,
    /// When set, `delete` returns an OS-style failure (leaving the entry in
    /// place) to exercise the fail-closed path.
    fail_delete: bool,
    /// When set, `set` returns a "password not saved" error (leaving the entry
    /// unchanged) to exercise the set-failure warning path.
    fail_set: bool,
    /// When set, the store behaves like a machine with no usable keychain:
    /// `set` fails and `get` reports [`PasswordMissing::NoKeychain`].
    unavailable: bool,
}

#[cfg(test)]
impl InMemoryPasswordStore {
    pub fn new() -> Self {
        Self {
            entries: HashMap::new(),
            fail_delete: false,
            fail_set: false,
            unavailable: false,
        }
    }

    /// Return a store whose `delete` always fails (simulating a keychain that
    /// refuses to remove the credential).
    pub fn with_failing_delete() -> Self {
        Self {
            entries: HashMap::new(),
            fail_delete: true,
            fail_set: false,
            unavailable: false,
        }
    }

    /// Return a store whose `set` always fails (simulating a keychain that
    /// refuses to store, e.g. a keychain-less host).  `delete` still works so a
    /// stale password is removable.
    pub fn with_failing_set() -> Self {
        Self {
            entries: HashMap::new(),
            fail_delete: false,
            fail_set: true,
            unavailable: false,
        }
    }

    /// Return a store that behaves like a machine with no usable keychain.
    pub fn unavailable() -> Self {
        Self {
            entries: HashMap::new(),
            fail_delete: false,
            fail_set: true,
            unavailable: true,
        }
    }

    /// No usable keychain at all: storing fails, reading reports no keychain,
    /// and deleting fails too (as on a machine with no Secret Service).
    pub fn unavailable_everywhere() -> Self {
        Self {
            entries: HashMap::new(),
            fail_delete: true,
            fail_set: true,
            unavailable: true,
        }
    }

    /// Inject a password directly, bypassing `set` (used to pre-seed a "saved"
    /// password before a later operation's set fails).
    pub fn inject(&mut self, key: &str, password: &str) {
        self.entries.insert(key.to_string(), password.to_string());
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
        if self.fail_set {
            return Err(CommandError::Keychain {
                message: NOT_SAVED_MESSAGE.to_string(),
            });
        }
        self.entries.insert(key.to_string(), password.to_string());
        Ok(())
    }

    fn get(&mut self, key: &str) -> Result<String, CommandError> {
        if self.unavailable {
            return Err(CommandError::PasswordMissing {
                reason: PasswordMissing::NoKeychain,
                profile: key.to_string(),
            });
        }
        self.entries
            .get(key)
            .cloned()
            .ok_or_else(|| CommandError::PasswordMissing {
                reason: PasswordMissing::NotStored,
                profile: key.to_string(),
            })
    }

    fn delete(&mut self, key: &str) -> Result<(), CommandError> {
        if self.fail_delete {
            return Err(CommandError::Keychain {
                message: "simulated keychain delete failure".to_string(),
            });
        }
        // Removing a missing entry is a no-op (success), mirroring the OS
        // "no such entry" -> Ok mapping.
        self.entries.remove(key);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn a_keyring_entry_that_cannot_be_made_reads_as_missing_not_as_an_error() {
        // An empty key is refused by the keyring crate on some platforms; the
        // read must still be a missing password (ask, or use the session
        // cache), never a hard keychain error that blocks the flow.
        let mut kc = Keychain::default();
        assert!(matches!(
            kc.get(""),
            Err(CommandError::PasswordMissing { .. })
        ));
    }

    #[test]
    fn set_surfaces_a_typed_warning_when_unavailable() {
        let mut kc = Keychain::unavailable();
        let err = kc.set("x", &test_secret()).unwrap_err();
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

    fn input(host: &str) -> ProfileInput {
        ProfileInput {
            id: "p".into(),
            label: "l".into(),
            host: host.into(),
            port: 5432,
            database: "bank".into(),
            user: "reader".into(),
            encryption: "verify".into(),
            tls_fingerprint: None,
            plaintext_reason: None,
            ca_file: None,
        }
    }

    #[test]
    fn fields_hash_covers_every_connection_field_but_not_the_label() {
        let base = input("db.bank");
        let h = input_fields_hash(&base);
        let mut label = base.clone();
        label.label = "renamed".into();
        assert_eq!(input_fields_hash(&label), h);
        let variants: Vec<fn(&mut ProfileInput)> = vec![
            |p| p.host = "other".into(),
            |p| p.port = 5433,
            |p| p.database = "x".into(),
            |p| p.user = "x".into(),
            |p| p.encryption = "allow-plaintext".into(),
            |p| p.tls_fingerprint = Some("ab".into()),
            |p| p.plaintext_reason = Some("r".into()),
            |p| p.ca_file = Some("/ca.pem".into()),
        ];
        for change in variants {
            let mut p = base.clone();
            change(&mut p);
            assert_ne!(input_fields_hash(&p), h);
        }
        // A saved profile with the same fields hashes the same.
        let stored = StoredProfile {
            id: base.id.clone(),
            label: "different".into(),
            host: base.host.clone(),
            port: base.port,
            database: base.database.clone(),
            user: base.user.clone(),
            encryption: base.encryption.clone(),
            tls_fingerprint: None,
            plaintext_reason: None,
            ca_file: None,
        };
        assert_eq!(stored_fields_hash(&stored), h);
    }

    #[test]
    fn session_passwords_are_bound_to_their_fields() {
        let secret = test_secret();
        let mut s = SessionPasswords::default();
        assert!(s.is_empty());
        let h = input_fields_hash(&input("a"));
        s.put("p", h.clone(), &secret);
        assert_eq!(s.get("p", &h).as_deref(), Some(secret.as_str()));
        assert!(s.contains("p", &h));
        // Another host, or another profile id, misses.
        assert_eq!(s.get("p", &input_fields_hash(&input("b"))), None);
        assert_eq!(s.get("q", &h), None);
        // A new password for the same id replaces the old one.
        let other = test_secret();
        let h2 = input_fields_hash(&input("b"));
        s.put("p", h2.clone(), &other);
        assert_eq!(s.len(), 1);
        assert_eq!(s.get("p", &h), None);
        assert_eq!(s.get("p", &h2).as_deref(), Some(other.as_str()));
        s.forget("p");
        assert!(s.is_empty());
        s.put("p", h.clone(), &secret);
        s.put("q", h, &secret);
        s.clear();
        assert!(s.is_empty());
    }

    #[test]
    fn session_passwords_never_print_a_secret() {
        let secret = test_secret();
        let mut s = SessionPasswords::default();
        s.put("p", input_fields_hash(&input("a")), &secret);
        let shown = format!("{s:?} {s:#?}");
        assert!(!shown.contains(&secret), "{shown}");
        assert!(shown.contains("entries: 1"), "{shown}");
    }

    #[test]
    fn the_unavailable_stub_behaves_like_no_keychain() {
        let mut kc = InMemoryPasswordStore::unavailable();
        assert!(kc
            .set("p", &test_secret())
            .unwrap_err()
            .to_string()
            .contains(NOT_SAVED_MESSAGE));
        assert!(matches!(
            kc.get("p"),
            Err(CommandError::PasswordMissing {
                reason: PasswordMissing::NoKeychain,
                ..
            })
        ));
    }

    #[test]
    fn in_memory_store_round_trips_and_deletes() {
        let mut store = InMemoryPasswordStore::new();
        let secret = test_secret();
        assert!(!store.contains("k"));
        store.set("k", &secret).unwrap();
        assert!(store.contains("k"));
        assert_eq!(store.get("k").unwrap(), secret);
        store.delete("k").unwrap();
        assert!(!store.contains("k"));
        assert!(matches!(
            store.get("k"),
            Err(CommandError::PasswordMissing { .. })
        ));
    }
}
