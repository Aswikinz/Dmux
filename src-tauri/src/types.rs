//! Serde DTOs that cross the Tauri invoke boundary.
//!
//! These mirror the engine's public serde types where possible, and add the
//! small input shapes the frontend sends.  Everything is `snake_case` to match
//! the frontend contract and the engine's own serde output.

use serde::{Deserialize, Serialize};

use crate::error::CommandError;
use dmux_engine::db::TlsMode;

/// How the frontend describes a connection profile when saving one.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct ProfileInput {
    pub id: String,
    pub label: String,
    pub host: String,
    pub port: u16,
    pub database: String,
    pub user: String,
    pub encryption: String,
    pub tls_fingerprint: Option<String>,
    pub plaintext_reason: Option<String>,
}

/// The shape a command receives when it must connect: a profile plus, where the
/// keychain cannot supply it, a password the user typed this run.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct ConnectInput {
    pub profile: ProfileInput,
    /// `None` means "read the password from the keychain"; `Some` is the
    /// ask-each-run value for this invocation.
    pub password: Option<String>,
}

/// Parse an `encryption` mode string into the engine's [`TlsMode`].
///
/// Accepted values match the frontend contract: `verify`, `trust` (must carry a
/// fingerprint) and `allow-plaintext` (must carry a non-empty reason).
pub fn parse_tls_mode(input: &ProfileInput) -> Result<TlsMode, CommandError> {
    match input.encryption.as_str() {
        "verify" => Ok(TlsMode::VerifyFull),
        "trust" => {
            let fp = input
                .tls_fingerprint
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .ok_or_else(|| CommandError::InvalidInput {
                    message: "trust encryption requires a certificate fingerprint".to_string(),
                })?;
            Ok(TlsMode::TrustFingerprint(fp.to_string()))
        }
        "allow-plaintext" => {
            let reason = input
                .plaintext_reason
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .ok_or_else(|| CommandError::InvalidInput {
                    message: "allow-plaintext requires a recorded reason".to_string(),
                })?;
            Ok(TlsMode::Disable(reason.to_string()))
        }
        other => Err(CommandError::InvalidInput {
            message: format!("unknown encryption mode: {other}"),
        }),
    }
}

/// Build an engine [`dmux_engine::db::ConnProfile`] from the frontend shape,
/// with the resolved password.
pub fn build_profile(
    input: &ProfileInput,
    password: String,
    tls: TlsMode,
) -> dmux_engine::db::ConnProfile {
    dmux_engine::db::ConnProfile::new(
        input.host.trim(),
        input.port,
        input.database.trim(),
        input.user.trim(),
        password,
        tls,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(encryption: &str) -> ProfileInput {
        ProfileInput {
            id: "p".to_string(),
            label: "x".to_string(),
            host: "h".to_string(),
            port: 5432,
            database: "d".to_string(),
            user: "u".to_string(),
            encryption: encryption.to_string(),
            tls_fingerprint: None,
            plaintext_reason: None,
        }
    }

    #[test]
    fn parses_verify() {
        assert!(matches!(
            parse_tls_mode(&input("verify")).unwrap(),
            TlsMode::VerifyFull
        ));
    }

    #[test]
    fn trust_requires_fingerprint() {
        assert!(parse_tls_mode(&input("trust")).is_err());

        let mut p = input("trust");
        p.tls_fingerprint = Some("aabb".to_string());
        match parse_tls_mode(&p).unwrap() {
            TlsMode::TrustFingerprint(fp) => assert_eq!(fp, "aabb"),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn plaintext_requires_a_reason() {
        assert!(parse_tls_mode(&input("allow-plaintext")).is_err());

        let mut p = input("allow-plaintext");
        p.plaintext_reason = Some("test network".to_string());
        match parse_tls_mode(&p).unwrap() {
            TlsMode::Disable(reason) => assert_eq!(reason, "test network"),
            other => panic!("unexpected {other:?}"),
        }
    }
}
