//! Serde DTOs that cross the Tauri invoke boundary.
//!
//! These mirror the engine's public serde types where possible, and add the
//! small input shapes the frontend sends.  Everything is `snake_case` to match
//! the frontend contract and the engine's own serde output.

use std::fmt;

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
    /// Optional path to a private-CA PEM bundle, appended to the public roots
    /// when `encryption == "verify"` (a bank's internal CA).  Optional so older
    /// frontends and test fixtures that omit it still deserialise.
    #[serde(default)]
    pub ca_file: Option<String>,
}

/// The shape a command receives when it must connect: a profile plus, where the
/// keychain cannot supply it, a password the user typed this run.
///
/// This is deserialised from the frontend only; it is never serialised back, and
/// its `Debug` impl deliberately redacts the password (PLAN 6.5: no secret in
/// logs or error output).
#[derive(Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ConnectInput {
    pub profile: ProfileInput,
    /// `None` means "read the password from the keychain"; `Some` is the
    /// ask-each-run value for this invocation.
    pub password: Option<String>,
}

impl fmt::Debug for ConnectInput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ConnectInput")
            .field("profile", &self.profile)
            .field("password", &self.password.as_ref().map(|_| "<redacted>"))
            .finish()
    }
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
    let mut profile = dmux_engine::db::ConnProfile::new(
        input.host.trim(),
        input.port,
        input.database.trim(),
        input.user.trim(),
        password,
        tls,
    );
    if let Some(ca) = input
        .ca_file
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        profile = profile.with_ca_file(ca);
    }
    profile
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
            ca_file: None,
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

    #[test]
    fn build_profile_carries_the_ca_file() {
        let mut p = input("verify");
        assert_eq!(
            build_profile(&p, String::new(), TlsMode::VerifyFull).ca_file,
            None
        );

        p.ca_file = Some("/etc/bank/ca.pem".to_string());
        let profile = build_profile(&p, String::new(), TlsMode::VerifyFull);
        assert_eq!(profile.ca_file.as_deref(), Some("/etc/bank/ca.pem"));

        // A blank CA file is treated as absent, not as a broken path.
        p.ca_file = Some("   ".to_string());
        assert_eq!(
            build_profile(&p, String::new(), TlsMode::VerifyFull).ca_file,
            None
        );
    }

    #[test]
    fn connect_input_debug_redacts_the_password() {
        let ci = ConnectInput {
            profile: input("verify"),
            password: Some("s3cr3t".to_string()),
        };
        let text = format!("{ci:?}");
        assert!(!text.contains("s3cr3t"), "debug leaked password: {text}");
        assert!(text.contains("<redacted>"));

        let none = ConnectInput {
            profile: input("verify"),
            password: None,
        };
        let none_text = format!("{none:?}");
        assert!(!none_text.contains("s3cr3t"));
    }
}
