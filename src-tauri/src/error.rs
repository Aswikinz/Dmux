//! Typed errors surfaced to the UI.
//!
//! Every command returns `Result<T, CommandError>`.  `CommandError` serialises
//! to a small JSON object so the frontend can show a `kind` and a plain,
//! secret-free `message` (PLAN 6.5: no passwords or row values may ever leave
//! through an error).  The underlying database error text already passes
//! through the engine's secret redaction (`db::errors::redact_secrets`).

use serde::Serialize;
use std::fmt;
use thiserror::Error;

/// The reason a password could not be read from the keychain.  The frontend
/// uses this to switch to the ask-each-run prompt.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PasswordMissing {
    /// No entry stored for this profile yet.
    NotStored,
    /// No usable keychain exists on this machine.
    NoKeychain,
}

impl fmt::Display for PasswordMissing {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotStored => f.write_str("no password stored for this profile"),
            Self::NoKeychain => f.write_str("no keychain available on this machine"),
        }
    }
}

/// A typed, serialisable command error.
#[derive(Debug, Error, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CommandError {
    /// Invalid input from the frontend (unknown TLS mode, bad port, etc.).
    #[error("{message}")]
    InvalidInput { message: String },

    /// A database profile was referenced but does not exist.
    #[error("profile not found: {key}")]
    ProfileNotFound { key: String },

    /// The password is not available from the keychain (ask the user).
    #[error("password not available: {reason}")]
    PasswordMissing {
        reason: PasswordMissing,
        profile: String,
    },

    /// Connecting to or querying the database failed.
    #[error("{message}")]
    Database { message: String },

    /// The server certificate is not trusted; the UI shows its fingerprint for
    /// a one-time trust (PLAN 6.7).
    #[error("server certificate is not trusted; fingerprint={fingerprint}")]
    UntrustedCert { fingerprint: String },

    /// A superuser login was refused (PLAN 6.2).
    #[error("superuser login refused")]
    SuperuserRefused,

    /// A plaintext connection was requested without a recorded reason.
    #[error("plaintext connection requested without a recorded reason")]
    PlaintextReasonRequired,

    /// The mapping file is not valid (untrusted input, PLAN 6.3).
    #[error("{message}")]
    InvalidMapping { message: String },

    /// The local profile/mapping store failed.
    #[error("{message}")]
    Store { message: String },

    /// The keychain is unavailable or returned an error.
    #[error("{message}")]
    Keychain { message: String },

    /// Something unexpected went wrong (no sensitive detail).
    #[error("{message}")]
    Internal { message: String },
}

impl CommandError {
    /// Wrap a generic message without risking a leak.
    pub fn internal(message: impl Into<String>) -> Self {
        Self::Internal {
            message: message.into(),
        }
    }
}

/// Map an engine [`DbError`] into a [`CommandError`], keeping the typed
/// superuser / untrusted-cert / plaintext refusal cases distinct so the UI can
/// act on them.
pub fn from_db_error(err: dmux_engine::db::DbError) -> CommandError {
    match err {
        dmux_engine::db::DbError::UntrustedCert { fingerprint } => {
            CommandError::UntrustedCert { fingerprint }
        }
        dmux_engine::db::DbError::SuperuserRefused => CommandError::SuperuserRefused,
        dmux_engine::db::DbError::PlaintextReasonRequired => CommandError::PlaintextReasonRequired,
        other => CommandError::Database {
            message: other.to_string(),
        },
    }
}

impl From<rusqlite::Error> for CommandError {
    fn from(err: rusqlite::Error) -> Self {
        CommandError::Store {
            message: err.to_string(),
        }
    }
}

impl From<serde_json::Error> for CommandError {
    fn from(err: serde_json::Error) -> Self {
        CommandError::invalid(format!("JSON error: {err}"))
    }
}

impl CommandError {
    fn invalid(message: impl Into<String>) -> Self {
        CommandError::InvalidInput {
            message: message.into(),
        }
    }
}

/// Implemented so a command can use `?` on an [`rusqlite::Result`] and get a
/// [`CommandError`] instead.  The trait is kept tiny on purpose.
impl From<keyring::Error> for CommandError {
    fn from(err: keyring::Error) -> Self {
        // `keyring::Error` does not carry the secret; only the reason string.
        CommandError::Keychain {
            message: err.to_string(),
        }
    }
}
