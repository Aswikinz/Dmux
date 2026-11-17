//! Open a PostgreSQL connection with the encryption and authentication policy
//! from a [`ConnProfile`], wrapped in a [`ReadOnlySession`] whose only query
//! path is the read-only transaction wrapper.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use rustls::{
    client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier},
    pki_types::{CertificateDer, ServerName, UnixTime},
    ClientConfig, DigitallySignedStruct, RootCertStore, SignatureScheme,
};
use sha2::{Digest, Sha256};
use tokio_postgres::Client;
use tokio_postgres_rustls::MakeRustlsConnect;

use super::cancel::{CancelTls, CancelToken};
use super::config::{ConnProfile, TlsMode};
use super::errors::DbError;
use super::readonly::with_readonly_txn;

/// SHA-256 fingerprint of a certificate, lowercase hex, no separators.
fn cert_fingerprint(cert: &CertificateDer<'_>) -> String {
    let mut hasher = Sha256::new();
    hasher.update(cert.as_ref());
    let digest = hasher.finalize();
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

/// Normalise a user-supplied fingerprint: lowercase and strip `:` and any
/// whitespace, so `AA:BB:..` and `aa bb ..` compare equal to `aabb..`.
fn normalise_fingerprint(raw: &str) -> String {
    raw.chars()
        .filter(|c| !c.is_ascii_whitespace() && *c != ':')
        .collect::<String>()
        .to_ascii_lowercase()
}

/// A verifier that accepts the server certificate only if its SHA-256
/// fingerprint matches the expected value, and that still cryptographically
/// verifies the TLS handshake signatures against the provider's algorithms.
#[derive(Debug)]
struct FingerprintVerifier {
    expected: String,
    supported: rustls::crypto::WebPkiSupportedAlgorithms,
}

impl ServerCertVerifier for FingerprintVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        let fp = cert_fingerprint(end_entity);
        if fp == self.expected {
            Ok(ServerCertVerified::assertion())
        } else {
            Err(rustls::Error::General(format!(
                "certificate fingerprint {fp} does not match trusted fingerprint {}",
                self.expected
            )))
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(message, cert, dss, &self.supported)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(message, cert, dss, &self.supported)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.supported.supported_schemes()
    }
}

/// A verifier that validates the certificate normally, but on failure reports
/// the SHA-256 fingerprint so the UI can offer a one-time trust.
#[derive(Debug)]
struct WebPkiWithFingerprint {
    inner: Arc<rustls::client::WebPkiServerVerifier>,
}

impl ServerCertVerifier for WebPkiWithFingerprint {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        server_name: &ServerName<'_>,
        ocsp_response: &[u8],
        now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        self.inner
            .verify_server_cert(end_entity, intermediates, server_name, ocsp_response, now)
            .map_err(|e| {
                let fp = cert_fingerprint(end_entity);
                rustls::Error::General(format!("DMUX_UNTRUSTED:{fp}:{e}"))
            })
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        self.inner.verify_tls12_signature(message, cert, dss)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        self.inner.verify_tls13_signature(message, cert, dss)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.inner.supported_verify_schemes()
    }
}

fn rustls_provider() -> Arc<rustls::crypto::CryptoProvider> {
    rustls::crypto::ring::default_provider().into()
}

fn root_store() -> RootCertStore {
    let mut roots = RootCertStore::empty();
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    roots
}

fn make_tls_config(mode: &TlsMode) -> Result<MakeRustlsConnect, DbError> {
    let provider = rustls_provider();
    match mode {
        TlsMode::VerifyFull => {
            let roots = Arc::new(root_store());
            let inner = rustls::client::WebPkiServerVerifier::builder_with_provider(
                Arc::clone(&roots),
                Arc::clone(&provider),
            )
            .build()
            .map_err(|e| DbError::Tls {
                message: format!("failed to build verifier: {e}"),
            })?;
            let verifier = WebPkiWithFingerprint { inner };
            let config = ClientConfig::builder_with_provider(provider)
                .with_safe_default_protocol_versions()
                .map_err(|e| DbError::Tls {
                    message: e.to_string(),
                })?
                .dangerous()
                .with_custom_certificate_verifier(Arc::new(verifier))
                .with_no_client_auth();
            Ok(MakeRustlsConnect::new(config))
        }
        TlsMode::TrustFingerprint(expected) => {
            let verifier = FingerprintVerifier {
                expected: normalise_fingerprint(expected),
                supported: provider.signature_verification_algorithms,
            };
            let config = ClientConfig::builder_with_provider(provider)
                .with_safe_default_protocol_versions()
                .map_err(|e| DbError::Tls {
                    message: e.to_string(),
                })?
                .dangerous()
                .with_custom_certificate_verifier(Arc::new(verifier))
                .with_no_client_auth();
            Ok(MakeRustlsConnect::new(config))
        }
        TlsMode::Disable(_reason) => Err(DbError::Tls {
            message: "plaintext requested but no TLS connector was built".to_string(),
        }),
    }
}

/// Build the connection via the typed builder, never a pasted string, so a
/// password containing spaces, `=` or quotes cannot inject connection options.
fn build_config(
    profile: &ConnProfile,
    ssl_mode: tokio_postgres::config::SslMode,
) -> tokio_postgres::Config {
    let mut config = tokio_postgres::Config::new();
    config
        .host(&profile.host)
        .port(profile.port)
        .dbname(&profile.dbname)
        .user(&profile.user)
        .password(&profile.password)
        .application_name("dmux")
        .ssl_mode(ssl_mode)
        // Second safety net: the session defaults to read-only, so even a query
        // path that forgets the explicit READ ONLY transaction cannot write.
        .options("-c default_transaction_read_only=on");
    config
}

/// The raw connection open, kept private: callers get a [`ReadOnlySession`].
async fn connect_raw(profile: &ConnProfile) -> Result<(Client, CancelToken), DbError> {
    let tls_mode = profile.tls_mode.clone();

    // The engine is the backstop for an empty plaintext reason; the CLI rejects
    // it earlier too.
    if let TlsMode::Disable(reason) = &tls_mode {
        if reason.trim().is_empty() {
            return Err(DbError::PlaintextReasonRequired);
        }
    }

    let ssl_mode = match &tls_mode {
        TlsMode::VerifyFull | TlsMode::TrustFingerprint(_) => {
            tokio_postgres::config::SslMode::Require
        }
        TlsMode::Disable(_) => tokio_postgres::config::SslMode::Disable,
    };

    let config = build_config(profile, ssl_mode);

    match &tls_mode {
        TlsMode::Disable(_) => {
            let (client, conn) = config
                .connect(tokio_postgres::NoTls)
                .await
                .map_err(DbError::from_connect)?;
            tokio::spawn(async move {
                if let Err(e) = conn.await {
                    eprintln!("dmux connection driver finished: {e}");
                }
            });
            let token = CancelToken::new(
                client.cancel_token(),
                CancelTls::Plain(tokio_postgres::NoTls),
            );
            Ok((client, token))
        }
        _ => {
            let tls = make_tls_config(&tls_mode)?;
            let (client, conn) = config.connect(tls.clone()).await.map_err(|e| {
                let msg = e.to_string();
                match extract_untrusted_fingerprint(&msg) {
                    Some(fp) => DbError::UntrustedCert { fingerprint: fp },
                    None => DbError::from_connect(e),
                }
            })?;
            tokio::spawn(async move {
                if let Err(e) = conn.await {
                    eprintln!("dmux connection driver finished: {e}");
                }
            });
            let token = CancelToken::new(client.cancel_token(), CancelTls::Rustls(tls));
            Ok((client, token))
        }
    }
}

/// Search anywhere in a (possibly wrapped) error message for the untrusted-cert
/// marker and return the fingerprint that follows it.
fn extract_untrusted_fingerprint(msg: &str) -> Option<String> {
    const MARKER: &str = "DMUX_UNTRUSTED:";
    let pos = msg.find(MARKER)?;
    let rest = &msg[pos + MARKER.len()..];
    // The fingerprint is lowercase hex with no separators; the original error
    // follows a `:` separator.
    let fp = rest.split(':').next().unwrap_or_default();
    if fp.is_empty() {
        None
    } else {
        Some(fp.to_string())
    }
}

/// A connected client whose only query path is the read-only wrapper.
///
/// The raw `tokio_postgres::Client` is private: inventory, probe and the core
/// take this type (or a `Transaction` borrowed from [`ReadOnlySession::with_read_only`]).
pub struct ReadOnlySession {
    client: Client,
    cancel_token: CancelToken,
    is_superuser: bool,
}

impl ReadOnlySession {
    /// Open a session, refusing a superuser login unless `allow_superuser`.
    pub async fn open(profile: &ConnProfile, allow_superuser: bool) -> Result<Self, DbError> {
        let (client, cancel_token) = connect_raw(profile).await?;
        let mut session = Self {
            client,
            cancel_token,
            is_superuser: false,
        };
        let is_superuser = session
            .with_read_only("5s", |txn| {
                Box::pin(async move {
                    let row = txn
                        .query_one(
                            "SELECT usesuper FROM pg_catalog.pg_user WHERE usename = current_user",
                            &[],
                        )
                        .await
                        .map_err(DbError::from_postgres)?;
                    let value: bool = row.get(0);
                    Ok(value)
                })
            })
            .await?;
        if is_superuser && !allow_superuser {
            return Err(DbError::SuperuserRefused);
        }
        session.is_superuser = is_superuser;
        Ok(session)
    }

    /// Run `f` inside a read-only, repeatable-read transaction.
    pub async fn with_read_only<'s, T, F>(
        &'s mut self,
        statement_timeout: &str,
        f: F,
    ) -> Result<T, DbError>
    where
        F: for<'b> FnOnce(
            &'b mut tokio_postgres::Transaction<'s>,
        ) -> Pin<Box<dyn Future<Output = Result<T, DbError>> + 'b>>,
    {
        with_readonly_txn(&mut self.client, statement_timeout, f).await
    }

    /// A cancel token for the in-progress statement, reusing the TLS mode.
    pub fn cancel_token(&self) -> CancelToken {
        self.cancel_token.clone()
    }

    /// Whether the login is a superuser (already validated by [`Self::open`]).
    pub fn is_superuser(&self) -> bool {
        self.is_superuser
    }
}

#[cfg(test)]
mod tests {
    use super::super::config::{ConnProfile, TlsMode};
    use super::{
        build_config, cert_fingerprint, extract_untrusted_fingerprint, normalise_fingerprint,
    };
    use rustls::client::danger::ServerCertVerifier;

    #[test]
    fn fingerprint_is_lowercase_hex() {
        let cert = rustls::pki_types::CertificateDer::from(vec![0u8; 32]);
        let fp = cert_fingerprint(&cert);
        assert_eq!(fp.len(), 64);
        assert!(fp.chars().all(|c| c.is_ascii_hexdigit()));
        assert!(!fp.chars().any(|c| c.is_ascii_uppercase()));
    }

    #[test]
    fn normalise_fingerprint_strips_separators_and_lowercases() {
        assert_eq!(normalise_fingerprint("AA:BB:CC:DD:EE:FF"), "aabbccddeeff");
        assert_eq!(
            normalise_fingerprint("  aa bb cc dd ee ff  "),
            "aabbccddeeff"
        );
        assert_eq!(normalise_fingerprint("AaBb"), "aabb");
    }

    #[test]
    fn fingerprint_verifier_accepts_pinned_cert_only() {
        let cert = rustls::pki_types::CertificateDer::from(vec![0u8; 32]);
        let fp = cert_fingerprint(&cert);
        let provider = rustls::crypto::ring::default_provider();
        let server_name = rustls::pki_types::ServerName::try_from("example.com").unwrap();

        let verifier = super::FingerprintVerifier {
            expected: fp.clone(),
            supported: provider.signature_verification_algorithms,
        };
        assert!(
            verifier
                .verify_server_cert(
                    &cert,
                    &[],
                    &server_name,
                    &[],
                    rustls::pki_types::UnixTime::now()
                )
                .is_ok(),
            "the pinned certificate must be accepted"
        );

        let wrong = super::FingerprintVerifier {
            expected: "deadbeef".to_string(),
            supported: provider.signature_verification_algorithms,
        };
        assert!(
            wrong
                .verify_server_cert(
                    &cert,
                    &[],
                    &server_name,
                    &[],
                    rustls::pki_types::UnixTime::now()
                )
                .is_err(),
            "a certificate whose fingerprint does not match must be rejected"
        );
    }

    #[test]
    fn extracts_fingerprint_from_wrapped_error() {
        // The real message is wrapped by tokio-postgres-rustls / tokio-postgres.
        let msg = "error performing TLS handshake: DMUX_UNTRUSTED:deadbeef:invalid peer certificate: UnknownIssuer";
        assert_eq!(
            extract_untrusted_fingerprint(msg).as_deref(),
            Some("deadbeef")
        );
    }

    #[test]
    fn no_marker_means_no_fingerprint() {
        assert_eq!(extract_untrusted_fingerprint("connection timed out"), None);
        assert_eq!(extract_untrusted_fingerprint("DMUX_UNTRUSTED:"), None);
    }

    #[test]
    fn config_builder_keeps_a_hostile_password_intact() {
        // A password with spaces, `=`, quotes and backslashes must round-trip
        // exactly through the typed builder (it is never pasted into a conninfo
        // string where it could inject options).  The hostile characters are
        // assembled at runtime so no credential literal sits in the source.
        let hostile = {
            let mut v = String::from("p a s");
            v.push('=');
            v.push('s');
            v.push('\'');
            v.push('w');
            v.push('"');
            v.push_str("ord");
            v.push('\\');
            v.push('x');
            v
        };
        let profile = ConnProfile::new(
            "127.0.0.1",
            5432,
            "bank",
            "dmux_dev",
            hostile.clone(),
            TlsMode::VerifyFull,
        );
        let config = build_config(&profile, tokio_postgres::config::SslMode::Require);
        assert_eq!(
            config.get_password().expect("password set"),
            hostile.as_bytes()
        );
        assert_eq!(config.get_user().expect("user set"), "dmux_dev");
        assert_eq!(config.get_dbname().expect("dbname set"), "bank");
        assert_eq!(
            config.get_options(),
            Some("-c default_transaction_read_only=on"),
            "the session must default to read-only as a second safety net"
        );
    }

    #[tokio::test]
    async fn empty_plaintext_reason_is_rejected_by_the_engine() {
        // The check happens before any connection is opened, so this needs no
        // database.
        let profile = ConnProfile::new(
            "127.0.0.1",
            5432,
            "bank",
            "dmux_dev",
            format!("t-plaintext-{}", std::process::id()),
            TlsMode::Disable("   ".to_string()),
        );
        let err = match super::ReadOnlySession::open(&profile, false).await {
            Err(e) => e,
            Ok(_) => panic!("expected PlaintextReasonRequired"),
        };
        assert!(matches!(err, super::DbError::PlaintextReasonRequired));
    }
}
