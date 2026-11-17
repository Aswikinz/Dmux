//! Database-layer errors.
//!
//! Every message is reviewed to make sure it cannot carry a password or row
//! value.  The `#[source]` chain may contain a `tokio_postgres::Error`, but
//! those only ever describe SQLSTATE/category, never bound-parameter values.

use thiserror::Error;

/// Redact any password value from a message string.
///
/// Looks for the ASCII substring `password=` (case-insensitive) directly over
/// the input's bytes, then replaces everything up to the next ASCII whitespace,
/// comma or quote with `password=<redacted>`.  The search never lowercases the
/// input first: lowercasing can change byte lengths for non-ASCII text (for
/// example `İ`), which would make a byte offset slice land mid-character and
/// panic or redact the wrong span.  Every slice here starts and ends on an
/// ASCII byte, and ASCII bytes never appear as a UTF-8 continuation byte, so
/// the result is always valid and correctly aligned.
fn redact_secrets(message: &str) -> String {
    const NEEDLE: &[u8] = b"password=";
    let bytes = message.as_bytes();
    let mut result = String::with_capacity(message.len());
    let mut start = 0usize;
    let mut i = 0usize;
    while i < bytes.len() {
        let matched = i + NEEDLE.len() <= bytes.len()
            && bytes[i..i + NEEDLE.len()]
                .iter()
                .zip(NEEDLE)
                .all(|(b, n)| b.eq_ignore_ascii_case(n));
        if matched {
            result.push_str(&message[start..i]);
            result.push_str("password=<redacted>");
            let mut j = i + NEEDLE.len();
            while j < bytes.len() {
                let b = bytes[j];
                if b.is_ascii_whitespace() || b == b',' || b == b'\'' || b == b'"' {
                    break;
                }
                j += 1;
            }
            i = j;
            start = j;
        } else {
            i += 1;
        }
    }
    result.push_str(&message[start..]);
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
    fn redact_is_case_insensitive_over_ascii_bytes() {
        let out = redact_secrets("conn: PASSWORD=hunter2 user=dmux");
        assert!(!out.contains("hunter2"), "redacted output: {out}");
        assert!(out.contains("password=<redacted>"));
        assert!(out.contains("user=dmux"));
    }

    #[test]
    fn redact_does_not_panic_or_misredact_on_non_ascii_before_password() {
        // "İ" is U+0130: two bytes in UTF-8, whose lowercase form is three bytes.
        // A lowercased-copy offset search would slice mid-character here.
        let out = redact_secrets("İstanbul Password=secret");
        assert!(!out.contains("secret"), "redacted output: {out}");
        assert!(
            out.contains("İstanbul"),
            "non-ASCII text must be preserved: {out}"
        );
        assert!(out.contains("password=<redacted>"));
    }

    #[test]
    fn redact_stops_at_commas_and_whitespace() {
        let out = redact_secrets("a=1,password=s3cr3t,b=2");
        assert!(!out.contains("s3cr3t"), "redacted output: {out}");
        assert!(out.contains("password=<redacted>"));
        assert!(out.contains("a=1"));
        assert!(out.contains("b=2"));
        // A value that runs to the end of the string is fully redacted.
        let out = redact_secrets("host=db password=trailing-secret");
        assert!(!out.contains("trailing-secret"), "redacted output: {out}");
        assert!(out.contains("host=db"));
    }

    #[test]
    fn redact_without_password_is_unchanged() {
        let input = "connection timed out after 30s";
        assert_eq!(redact_secrets(input), input);
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
