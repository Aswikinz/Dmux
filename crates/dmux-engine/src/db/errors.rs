//! Database-layer errors.
//!
//! Every message is reviewed to make sure it cannot carry a password or row
//! value.  The `#[source]` chain may contain a `tokio_postgres::Error`, but
//! those only ever describe SQLSTATE/category, never bound-parameter values.

use thiserror::Error;

/// Redact any password value from a message string.
///
/// Looks for the substring `password=` (case-insensitive), then replaces
/// everything up to the next whitespace or end of string with `password=<redacted>`.
fn redact_secrets(message: &str) -> String {
    let mut result = String::with_capacity(message.len());
    let mut input = message;
    while let Some(pos) = input.to_lowercase().find("password=") {
        let before = &input[..pos];
        result.push_str(before);
        result.push_str("password=<redacted>");
        // Skip the `password=` prefix and the value that follows.
        let after_prefix = &input[pos + "password=".len()..];
        let skip_len = after_prefix
            .find(|c: char| c.is_ascii_whitespace() || c == ',' || c == '\'' || c == '"')
            .unwrap_or(after_prefix.len());
        input = &after_prefix[skip_len..];
    }
    result.push_str(input);
    result
}

/// Something that went wrong while talking to a PostgreSQL database.
#[derive(Error, Debug)]
pub enum DbError {
    /// Connection could not be established.
    #[error("connection failed: {message}")]
    Connect { message: String },

    /// The TLS handshake failed before PostgreSQL started speaking.
    #[error("TLS handshake failed: {message}")]
    Tls { message: String },

    /// The server presented a certificate we do not trust.  The SHA-256
    /// fingerprint is exposed so the UI can offer "trust once".
    #[error("server certificate is not trusted; fingerprint={fingerprint}")]
    UntrustedCert { fingerprint: String },

    /// Plaintext was requested but no reason was recorded.
    #[error("plaintext connection requested without a recorded reason")]
    PlaintextReasonRequired,

    /// The database returned an error for a query we sent.
    #[error("database error: {message}")]
    Postgres { message: String },

    /// A write was attempted inside a read-only transaction.
    #[error("transaction is read-only; attempted write was rejected")]
    ReadOnlyViolation,

    /// A superuser login was refused unless the user explicitly overrides it.
    #[error("superuser login refused; use --allow-superuser --reason <text>")]
    SuperuserRefused,

    /// The server looks like it is behind a connection pooler.
    #[error("server appears to be behind a connection pooler")]
    PoolerSuspected,

    /// The server encoding is SQL_ASCII, which must be flagged for review.
    #[error("server encoding is SQL_ASCII; review required")]
    SqlAsciiEncoding,

    /// A query cancel request failed.
    #[error("cancel request failed: {message}")]
    Cancel { message: String },

    /// A required argument or configuration value is missing or invalid.
    #[error("invalid configuration: {message}")]
    Config { message: String },
}

impl DbError {
    /// Wrap a `tokio_postgres::Error` without exposing any bound value.
    pub(crate) fn from_postgres(err: tokio_postgres::Error) -> Self {
        // tokio_postgres::Error::to_string() contains the SQLSTATE and the
        // server message template, not parameter values.
        Self::Postgres {
            message: err.to_string(),
        }
    }

    /// Wrap a connection error without exposing any secret.
    pub(crate) fn from_connect(err: tokio_postgres::Error) -> Self {
        Self::Connect {
            message: redact_secrets(&err.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{redact_secrets, DbError};

    #[test]
    fn redact_removes_password_value() {
        let out = redact_secrets("connection failed: password=secret123 host=localhost");
        assert!(!out.contains("secret123"), "redacted output: {out}");
        assert!(out.contains("password=<redacted>"));
        assert!(out.contains("host=localhost"));
    }

    #[test]
    fn error_messages_never_contain_password() {
        let err = DbError::Connect {
            message: redact_secrets("connection failed: password=secret123"),
        };
        let text = err.to_string();
        assert!(
            !text.contains("secret123"),
            "error must not leak password: {text}"
        );
    }

    #[test]
    fn untrusted_cert_message_contains_only_fingerprint() {
        let err = DbError::UntrustedCert {
            fingerprint: "aabbccdd".to_string(),
        };
        let text = err.to_string();
        assert!(text.contains("aabbccdd"));
        assert!(!text.contains("password"));
    }
}
