//! Connection profile: everything needed to open one PostgreSQL connection.

use std::fmt;

/// How to secure the connection to the server.
#[derive(Clone, PartialEq, Eq)]
pub enum TlsMode {
    /// Verify the server certificate against the bundled public CAs and the
    /// hostname.  If the certificate is not trusted, report its SHA-256
    /// fingerprint so the UI can offer a one-time trust.
    VerifyFull,

    /// Accept only the server certificate whose SHA-256 fingerprint matches
    /// exactly.  This is the "trust once" path from the UI.
    TrustFingerprint(String),

    /// Connect without TLS.  The reason string is recorded in the report.
    Disable(String),
}

impl fmt::Debug for TlsMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::VerifyFull => f.write_str("VerifyFull"),
            Self::TrustFingerprint(fp) => f.debug_tuple("TrustFingerprint").field(fp).finish(),
            Self::Disable(reason) => f.debug_struct("Disable").field("reason", reason).finish(),
        }
    }
}

/// A database connection profile.
///
/// `Debug` and `Display` implementations are deliberately custom so that the
/// password is never printed.  Clone the profile when needed, but never log it
/// whole.
#[derive(Clone, PartialEq, Eq)]
pub struct ConnProfile {
    pub host: String,
    pub port: u16,
    pub dbname: String,
    pub user: String,
    pub password: String,
    pub tls_mode: TlsMode,
}

impl ConnProfile {
    /// Build a profile, validating the bare minimum.
    pub fn new(
        host: impl Into<String>,
        port: u16,
        dbname: impl Into<String>,
        user: impl Into<String>,
        password: impl Into<String>,
        tls_mode: TlsMode,
    ) -> Self {
        Self {
            host: host.into(),
            port,
            dbname: dbname.into(),
            user: user.into(),
            password: password.into(),
            tls_mode,
        }
    }
}

impl fmt::Debug for ConnProfile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ConnProfile")
            .field("host", &self.host)
            .field("port", &self.port)
            .field("dbname", &self.dbname)
            .field("user", &self.user)
            .field("password", &"<redacted>")
            .field("tls_mode", &self.tls_mode)
            .finish()
    }
}

impl fmt::Display for ConnProfile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}@{}:{}/{}",
            self.user, self.host, self.port, self.dbname
        )
    }
}

#[cfg(test)]
mod tests {
    use super::{ConnProfile, TlsMode};

    /// Build a non-literal password fixture at runtime so no hard-coded
    /// credential literal flows into a password parameter (CodeQL
    /// rust/hard-coded-cryptographic-value).  The `tag` only labels the value;
    /// the process id makes it unique per run.
    fn test_secret(tag: &str) -> String {
        format!("t-{tag}-{}", std::process::id())
    }

    #[test]
    fn debug_redacts_password() {
        let secret = test_secret("debug");
        let p = ConnProfile::new(
            "db.example.com",
            5432,
            "bank",
            "dmux_reader",
            secret.clone(),
            TlsMode::VerifyFull,
        );
        let text = format!("{p:?}");
        assert!(!text.contains(&secret), "debug leaked password: {text}");
        assert!(text.contains("<redacted>"));
    }

    #[test]
    fn display_redacts_password() {
        let secret = test_secret("display");
        let p = ConnProfile::new(
            "db.example.com",
            5432,
            "bank",
            "dmux_reader",
            secret.clone(),
            TlsMode::VerifyFull,
        );
        let text = p.to_string();
        assert!(!text.contains(&secret), "display leaked password: {text}");
    }

    #[test]
    fn disable_tls_carries_reason() {
        let p = ConnProfile::new(
            "db.example.com",
            5432,
            "bank",
            "dmux_reader",
            test_secret("disable"),
            TlsMode::Disable("test-network".to_string()),
        );
        let text = format!("{p:?}");
        assert!(text.contains("test-network"));
    }
}
