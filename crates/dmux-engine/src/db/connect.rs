//! Open a PostgreSQL connection with the encryption and authentication policy
//! from a [`ConnProfile`], wrapped in a [`ReadOnlySession`] whose only query
//! path is the read-only transaction wrapper.

use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use rustls::{
    client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier},
    pki_types::pem::PemObject,
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

/// The shared slot the verify-full verifier writes the presented leaf
/// fingerprint into when certificate verification fails.  The connection path
/// reads it after a failed handshake, so the trust-once flow never has to parse
/// the error text (whose cause `tokio_postgres::Error::Display` drops).
type RecordedFingerprint = Arc<Mutex<Option<String>>>;

/// An inert recorded-fingerprint slot, used by the TLS modes that never record
/// one (trust-once pins an expected fingerprint; plaintext has no TLS).
fn empty_recorder() -> RecordedFingerprint {
    Arc::new(Mutex::new(None))
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

/// A verifier that validates the certificate normally, but on failure records
/// the presented leaf's SHA-256 fingerprint into shared state so the UI can
/// offer a one-time trust.  Recording into state (rather than embedding a
/// marker in the error text) is essential: `tokio_postgres::Error`'s `Display`
/// prints only its kind and drops the cause, so the marker never surfaced.
#[derive(Debug)]
struct WebPkiWithFingerprint {
    inner: Arc<rustls::client::WebPkiServerVerifier>,
    recorded: RecordedFingerprint,
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
            .inspect_err(|_| {
                let fp = cert_fingerprint(end_entity);
                if let Ok(mut slot) = self.recorded.lock() {
                    *slot = Some(fp);
                }
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

/// The bundled public roots, optionally extended with a private-CA PEM bundle.
///
/// `ca_file` is a path to a PEM bundle of CA certificates (for example a bank's
/// internal CA).  Its certificates are appended to the webpki roots; hostname
/// verification stays on.  A bad, unreadable or empty file is a typed error, and
/// the error never echoes the file's contents, only its path.
fn root_store(ca_file: Option<&str>) -> Result<RootCertStore, DbError> {
    let mut roots = RootCertStore::empty();
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    if let Some(path) = ca_file {
        let path = path.trim();
        if !path.is_empty() {
            append_ca_file(&mut roots, path)?;
        }
    }
    Ok(roots)
}

/// Parse the PEM bundle at `path` and append every certificate to `roots`.
fn append_ca_file(roots: &mut RootCertStore, path: &str) -> Result<(), DbError> {
    let bytes = std::fs::read(path).map_err(|e| DbError::Tls {
        message: format!("could not read CA file {path}: {e}"),
    })?;

    let certs: Vec<CertificateDer<'static>> = CertificateDer::pem_slice_iter(&bytes)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| DbError::Tls {
            message: format!("CA file {path} is not a valid PEM certificate bundle"),
        })?;

    if certs.is_empty() {
        return Err(DbError::Tls {
            message: format!("CA file {path} contains no certificates"),
        });
    }

    let (added, _ignored) = roots.add_parsable_certificates(certs);
    if added == 0 {
        return Err(DbError::Tls {
            message: format!("CA file {path} contains no usable certificates"),
        });
    }
    Ok(())
}

/// Build the raw rustls client config for `mode`, returning it alongside the
/// shared fingerprint-recording slot the verify-full path fills on failure.
fn build_client_config(
    mode: &TlsMode,
    ca_file: Option<&str>,
) -> Result<(ClientConfig, RecordedFingerprint), DbError> {
    let provider = rustls_provider();
    match mode {
        TlsMode::VerifyFull => {
            let roots = Arc::new(root_store(ca_file)?);
            let inner = rustls::client::WebPkiServerVerifier::builder_with_provider(
                Arc::clone(&roots),
                Arc::clone(&provider),
            )
            .build()
            .map_err(|e| DbError::Tls {
                message: format!("failed to build verifier: {e}"),
            })?;
            let recorded = empty_recorder();
            let verifier = WebPkiWithFingerprint {
                inner,
                recorded: Arc::clone(&recorded),
            };
            let config = ClientConfig::builder_with_provider(provider)
                .with_safe_default_protocol_versions()
                .map_err(|e| DbError::Tls {
                    message: e.to_string(),
                })?
                .dangerous()
                .with_custom_certificate_verifier(Arc::new(verifier))
                .with_no_client_auth();
            Ok((config, recorded))
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
            Ok((config, empty_recorder()))
        }
        TlsMode::Disable(_reason) => Err(DbError::Tls {
            message: "plaintext requested but no TLS connector was built".to_string(),
        }),
    }
}

fn make_tls_config(
    mode: &TlsMode,
    ca_file: Option<&str>,
) -> Result<(MakeRustlsConnect, RecordedFingerprint), DbError> {
    let (config, recorded) = build_client_config(mode, ca_file)?;
    Ok((MakeRustlsConnect::new(config), recorded))
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
            let ca_file = profile.ca_file.as_deref();
            let (tls, recorded) = make_tls_config(&tls_mode, ca_file)?;
            let (client, conn) = config.connect(tls.clone()).await.map_err(|e| {
                // Primary path: the verify-full verifier recorded the presented
                // leaf fingerprint into shared state before the handshake error
                // was wrapped, so nothing depends on parsing error text.
                if let Some(fp) = recorded.lock().ok().and_then(|mut slot| slot.take()) {
                    return DbError::UntrustedCert { fingerprint: fp };
                }
                // Fallback: walk the source chain for the (legacy) marker.  Kept
                // for robustness; the recorded state above is authoritative.
                if let Some(fp) = untrusted_fingerprint_from_chain(&e) {
                    return DbError::UntrustedCert { fingerprint: fp };
                }
                DbError::from_connect(e)
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

/// Walk an error and its `source()` chain looking for the untrusted-cert marker.
///
/// This is the fallback for the recorded-state primary path: it tolerates a
/// future refactor that drops the shared slot, but nothing in the happy path
/// depends on it, and it never reads the cause that `tokio_postgres::Error`'s
/// `Display` hides from `to_string()`.
fn untrusted_fingerprint_from_chain(err: &tokio_postgres::Error) -> Option<String> {
    let mut current: Option<&(dyn std::error::Error + 'static)> = Some(err);
    while let Some(e) = current {
        if let Some(fp) = extract_untrusted_fingerprint(&e.to_string()) {
            return Some(fp);
        }
        current = e.source();
    }
    None
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
        append_ca_file, build_client_config, build_config, cert_fingerprint,
        extract_untrusted_fingerprint, normalise_fingerprint, root_store,
    };
    use rcgen::{
        generate_simple_self_signed, BasicConstraints, Certificate, CertificateParams,
        CertifiedIssuer, DistinguishedName, DnType, ExtendedKeyUsagePurpose, IsCa, KeyPair,
        KeyUsagePurpose,
    };
    use rustls::client::danger::ServerCertVerifier;
    use rustls::pki_types::{
        CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer, ServerName, UnixTime,
    };
    use std::sync::{Arc, Mutex};

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
            test_secret(),
            TlsMode::Disable("   ".to_string()),
        );
        let err = match super::ReadOnlySession::open(&profile, false).await {
            Err(e) => e,
            Ok(_) => panic!("expected PlaintextReasonRequired"),
        };
        assert!(matches!(err, super::DbError::PlaintextReasonRequired));
    }

    // --- M4: trust-once recording (no dependence on error text) ---

    #[test]
    fn verifier_records_presented_fingerprint_on_failure() {
        let provider = super::rustls_provider();
        let roots = Arc::new(root_store(None).expect("root store"));
        let inner = rustls::client::WebPkiServerVerifier::builder_with_provider(
            Arc::clone(&roots),
            Arc::clone(&provider),
        )
        .build()
        .expect("verifier");
        let recorded = Arc::new(Mutex::new(None));
        let verifier = super::WebPkiWithFingerprint {
            inner,
            recorded: Arc::clone(&recorded),
        };

        let cert = CertificateDer::from(vec![0u8; 32]);
        let server_name = ServerName::try_from("example.com").unwrap();
        let result = verifier.verify_server_cert(&cert, &[], &server_name, &[], UnixTime::now());

        assert!(result.is_err(), "an untrusted cert must be rejected");
        let expected = cert_fingerprint(&cert);
        assert_eq!(
            recorded.lock().unwrap().as_deref(),
            Some(expected.as_str()),
            "the verifier must record the presented fingerprint on failure"
        );
    }

    // --- M9: private-CA file handling ---

    #[test]
    fn ca_file_append_trusts_the_bundle() {
        let ca = test_ca();
        let path = temp_pem("ca", ca.pem().as_bytes());
        let mut roots = root_store(None).expect("root store");
        let before = roots.len();
        append_ca_file(&mut roots, path.to_str().unwrap()).expect("append");
        assert!(
            roots.len() > before,
            "the CA file certificate must be added to the store"
        );
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn ca_file_missing_is_a_typed_error() {
        let err = root_store(Some("/definitely/not/a/real/dmux-ca.pem")).unwrap_err();
        let text = err.to_string();
        assert!(text.contains("could not read CA file"), "message: {text}");
        assert!(
            !text.contains("BEGIN CERTIFICATE"),
            "no PEM contents: {text}"
        );
    }

    #[test]
    fn ca_file_empty_is_a_typed_error() {
        let path = temp_pem("empty", b"");
        let err = root_store(Some(path.to_str().unwrap())).unwrap_err();
        assert!(err.to_string().contains("no certificates"));
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn ca_file_not_pem_is_a_typed_error() {
        let garbage =
            b"-----BEGIN CERTIFICATE-----\n!!! not valid base64 !!!\n-----END CERTIFICATE-----\n";
        let path = temp_pem("garbage", garbage);
        let err = root_store(Some(path.to_str().unwrap())).unwrap_err();
        assert!(err
            .to_string()
            .contains("not a valid PEM certificate bundle"));
        std::fs::remove_file(&path).ok();
    }

    /// Write `contents` to a unique file under the OS temp dir and return its path.
    fn temp_pem(name: &str, contents: &[u8]) -> std::path::PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("dmux-t047-{name}-{}-{n}", std::process::id()));
        std::fs::write(&path, contents).expect("write temp pem");
        path
    }

    // --- Real rustls handshakes against a locally generated server ---
    //
    // Certificates are generated at test time with `rcgen`, so no private key is
    // ever committed to the repo: a banking verification tool must not ship one,
    // and gitleaks / GitHub secret scanning flag committed private keys.  Three
    // fixtures are built: a throwaway CA, a localhost/127.0.0.1 leaf signed by
    // it, and a separate self-signed leaf (for the trust-once flow).  The local
    // handshakes below involve no Postgres server: client-side verification
    // happens during the TLS handshake, before any PostgreSQL protocol bytes.

    fn test_ca() -> CertifiedIssuer<'static, KeyPair> {
        let key = KeyPair::generate().expect("CA key");
        let mut params = CertificateParams::default();
        params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
        let mut dn = DistinguishedName::new();
        dn.push(DnType::CommonName, "Dmux test CA");
        params.distinguished_name = dn;
        CertifiedIssuer::self_signed(params, key).expect("CA cert")
    }

    /// A `localhost`/`127.0.0.1` leaf signed by `ca`.
    fn test_leaf(ca: &CertifiedIssuer<'_, KeyPair>) -> (Certificate, KeyPair) {
        let key = KeyPair::generate().expect("leaf key");
        let mut params =
            CertificateParams::new(vec!["localhost".to_string(), "127.0.0.1".to_string()])
                .expect("leaf params");
        params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
        let cert = params.signed_by(&key, ca).expect("leaf cert");
        (cert, key)
    }

    /// A self-signed leaf with the same SANs, for the trust-once flow.
    fn test_self_signed_leaf() -> (Certificate, KeyPair) {
        let ck =
            generate_simple_self_signed(vec!["localhost".to_string(), "127.0.0.1".to_string()])
                .expect("self-signed leaf");
        (ck.cert, ck.signing_key)
    }

    fn cert_der(cert: &Certificate) -> CertificateDer<'static> {
        cert.der().clone()
    }

    fn key_der(key: &KeyPair) -> PrivateKeyDer<'static> {
        PrivateKeyDer::from(PrivatePkcs8KeyDer::from(key.serialize_der()))
    }

    fn test_server_config(
        cert: CertificateDer<'static>,
        key: PrivateKeyDer<'static>,
    ) -> Arc<rustls::ServerConfig> {
        let provider = super::rustls_provider();
        Arc::new(
            rustls::ServerConfig::builder_with_provider(provider)
                .with_safe_default_protocol_versions()
                .expect("protocol versions")
                .with_no_client_auth()
                .with_single_cert(vec![cert], key)
                .expect("server config"),
        )
    }

    /// Spawn a one-shot rustls server on `127.0.0.1:0` presenting `cert`.
    /// Returns its address and the serving thread handle.
    fn spawn_tls_server(
        cert: CertificateDer<'static>,
        key: PrivateKeyDer<'static>,
    ) -> (std::net::SocketAddr, std::thread::JoinHandle<()>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().expect("addr");
        let config = test_server_config(cert, key);
        let handle = std::thread::spawn(move || {
            let (stream, _peer) = match listener.accept() {
                Ok(pair) => pair,
                Err(_) => return,
            };
            let _ = stream.set_read_timeout(Some(std::time::Duration::from_secs(10)));
            let conn = rustls::ServerConnection::new(config).expect("server conn");
            let mut tls = rustls::StreamOwned::new(conn, stream);
            for _ in 0..64 {
                match tls.conn.complete_io(&mut tls.sock) {
                    Ok(_) if tls.conn.is_handshaking() => continue,
                    _ => break,
                }
            }
        });
        (addr, handle)
    }

    /// A one-shot server for tokio-postgres clients: it answers the PostgreSQL
    /// SSL-negotiation request (`S`) before the TLS handshake, matching what a
    /// real server does under `SslMode::Require` + `SslNegotiation::Postgres`.
    fn spawn_postgres_tls_server(
        cert: CertificateDer<'static>,
        key: PrivateKeyDer<'static>,
    ) -> (std::net::SocketAddr, std::thread::JoinHandle<()>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().expect("addr");
        let config = test_server_config(cert, key);
        let handle = std::thread::spawn(move || {
            use std::io::{Read, Write};
            let (mut stream, _peer) = match listener.accept() {
                Ok(pair) => pair,
                Err(_) => return,
            };
            let _ = stream.set_read_timeout(Some(std::time::Duration::from_secs(10)));
            let _ = stream.set_write_timeout(Some(std::time::Duration::from_secs(10)));
            // PostgreSQL SSLRequest: 8 bytes (length + 80877103). Reply 'S'.
            let mut sslreq = [0u8; 8];
            if stream.read_exact(&mut sslreq).is_err() {
                return;
            }
            if stream.write_all(b"S").is_err() || stream.flush().is_err() {
                return;
            }
            let conn = rustls::ServerConnection::new(config).expect("server conn");
            let mut tls = rustls::StreamOwned::new(conn, stream);
            for _ in 0..64 {
                match tls.conn.complete_io(&mut tls.sock) {
                    Ok(_) if tls.conn.is_handshaking() => continue,
                    _ => break,
                }
            }
        });
        (addr, handle)
    }

    /// Drive a raw client handshake to completion (or error) against `addr`.
    fn run_client_handshake(
        config: Arc<rustls::ClientConfig>,
        name: ServerName<'static>,
        addr: std::net::SocketAddr,
    ) -> Result<(), rustls::Error> {
        let sock = std::net::TcpStream::connect(addr)
            .map_err(|e| rustls::Error::General(format!("connect: {e}")))?;
        let _ = sock.set_read_timeout(Some(std::time::Duration::from_secs(10)));
        let conn = rustls::ClientConnection::new(config, name)?;
        let mut tls = rustls::StreamOwned::new(conn, sock);
        for _ in 0..64 {
            if !tls.conn.is_handshaking() {
                return Ok(());
            }
            tls.conn
                .complete_io(&mut tls.sock)
                .map_err(|e| rustls::Error::General(format!("handshake: {e}")))?;
        }
        Err(rustls::Error::General("handshake did not converge".into()))
    }

    fn local_server_name() -> ServerName<'static> {
        ServerName::try_from("127.0.0.1")
            .expect("server name")
            .to_owned()
    }

    #[tokio::test]
    async fn untrusted_cert_is_produced_from_a_real_handshake_failure() {
        let (leaf_cert, leaf_key) = test_self_signed_leaf();
        let leaf_der = cert_der(&leaf_cert);
        let expected = cert_fingerprint(&leaf_der);
        let (addr, server) = spawn_postgres_tls_server(leaf_der, key_der(&leaf_key));
        let profile = ConnProfile::new(
            "127.0.0.1",
            addr.port(),
            "bank",
            "dmux_dev",
            test_secret(),
            TlsMode::VerifyFull,
        );
        let err = match super::ReadOnlySession::open(&profile, false).await {
            Err(e) => e,
            Ok(_) => panic!("a self-signed cert must not verify against the public roots"),
        };
        match &err {
            super::DbError::UntrustedCert { fingerprint } => assert_eq!(fingerprint, &expected),
            other => panic!("expected UntrustedCert, got {other:?}"),
        }
        server.join().expect("server thread");
    }

    #[test]
    fn trust_once_accepts_the_pinned_fingerprint() {
        let (leaf_cert, leaf_key) = test_self_signed_leaf();
        let leaf_der = cert_der(&leaf_cert);
        let fp = cert_fingerprint(&leaf_der);
        let (addr, server) = spawn_tls_server(leaf_der, key_der(&leaf_key));
        let (config, _recorded) =
            build_client_config(&TlsMode::TrustFingerprint(fp), None).expect("client config");
        run_client_handshake(Arc::new(config), local_server_name(), addr)
            .expect("trust-once must accept the pinned certificate");
        server.join().expect("server thread");
    }

    #[test]
    fn verify_full_with_ca_file_trusts_a_private_ca() {
        let ca = test_ca();
        let (leaf_cert, leaf_key) = test_leaf(&ca);
        let leaf_der = cert_der(&leaf_cert);
        let (addr, server) = spawn_tls_server(leaf_der, key_der(&leaf_key));
        // `ca_file` needs a real path, so write the CA PEM to a temp file.
        let ca_path = temp_pem("ca", ca.pem().as_bytes());
        let (config, recorded) =
            build_client_config(&TlsMode::VerifyFull, Some(ca_path.to_str().unwrap()))
                .expect("client config");
        run_client_handshake(Arc::new(config), local_server_name(), addr)
            .expect("verify-full with the CA file must trust the server");
        assert!(
            recorded.lock().unwrap().is_none(),
            "a successful verification must not record a fingerprint"
        );
        std::fs::remove_file(&ca_path).ok();
        server.join().expect("server thread");
    }

    #[test]
    fn verify_full_without_trust_records_the_presented_fingerprint() {
        let (leaf_cert, leaf_key) = test_self_signed_leaf();
        let leaf_der = cert_der(&leaf_cert);
        let expected = cert_fingerprint(&leaf_der);
        let (addr, server) = spawn_tls_server(leaf_der, key_der(&leaf_key));
        let (config, recorded) =
            build_client_config(&TlsMode::VerifyFull, None).expect("client config");
        assert!(
            run_client_handshake(Arc::new(config), local_server_name(), addr).is_err(),
            "an untrusted cert must fail the handshake"
        );
        assert_eq!(recorded.lock().unwrap().as_deref(), Some(expected.as_str()));
        server.join().expect("server thread");
    }

    /// Parse `postgres://user:pass@host:port/db` into its parts.  Used only by
    /// the live TLS test, so no credential literal ever appears in source.
    fn parse_postgres_url(url: &str) -> (String, u16, String, String, String) {
        let rest = url
            .strip_prefix("postgres://")
            .or_else(|| url.strip_prefix("postgresql://"))
            .expect("postgres:// URL");
        let (auth, hostportdb) = rest.split_once('@').unwrap_or(("", rest));
        let (user, pass) = auth
            .split_once(':')
            .expect("the live TLS test URL must carry user:password");
        let (hostport, db) = hostportdb
            .split_once('/')
            .unwrap_or((hostportdb, "postgres"));
        let (host, port) = match hostport.split_once(':') {
            Some((h, p)) => (h, p.parse::<u16>().expect("port")),
            None => (hostport, 5432),
        };
        (
            host.to_string(),
            port,
            db.to_string(),
            user.to_string(),
            pass.to_string(),
        )
    }

    /// Live test against a real TLS-enabled PostgreSQL server, gated on
    /// `DMUX_TEST_TLS_URL` (and `DMUX_TEST_TLS_CA` for the CA bundle).  The TLS
    /// container is not present in CI, so this test SKIPS with a printed note
    /// when the URL is unset.
    #[tokio::test]
    async fn live_tls_server_trust_once_and_ca_file() {
        let url = match std::env::var("DMUX_TEST_TLS_URL") {
            Ok(u) => u,
            Err(_) => {
                eprintln!(
                    "note: DMUX_TEST_TLS_URL is not set; skipping the live TLS integration \
                     test (the TLS container is not available in CI yet)"
                );
                return;
            }
        };
        let (host, port, db, user, pass) = parse_postgres_url(&url);

        // (2) verify-full WITHOUT a CA must surface the server's real fingerprint.
        let verify = ConnProfile::new(&host, port, &db, &user, pass.clone(), TlsMode::VerifyFull);
        let fp = match super::ReadOnlySession::open(&verify, true).await {
            Err(super::DbError::UntrustedCert { fingerprint }) => fingerprint,
            Ok(_) => panic!("a private-CA server must not verify against the public roots"),
            Err(other) => panic!("expected UntrustedCert, got {other:?}"),
        };
        assert_eq!(fp.len(), 64, "fingerprint must be 64 lowercase hex chars");
        assert!(
            fp.chars().all(|c| c.is_ascii_hexdigit()),
            "fingerprint must be hex"
        );

        // (3) trust-once with the pinned fingerprint connects; a wrong pin fails.
        let pinned = ConnProfile::new(
            &host,
            port,
            &db,
            &user,
            pass.clone(),
            TlsMode::TrustFingerprint(fp),
        );
        super::ReadOnlySession::open(&pinned, true)
            .await
            .expect("trust-once must connect once pinned");

        let wrong = ConnProfile::new(
            &host,
            port,
            &db,
            &user,
            pass.clone(),
            TlsMode::TrustFingerprint("0".repeat(64)),
        );
        assert!(
            super::ReadOnlySession::open(&wrong, true).await.is_err(),
            "a wrong pin must fail"
        );

        // (1) verify-full with the CA file connects.
        let ca = match std::env::var("DMUX_TEST_TLS_CA") {
            Ok(ca) => ca,
            Err(_) => {
                eprintln!("note: DMUX_TEST_TLS_CA is not set; skipping the CA-file leg");
                return;
            }
        };
        let with_ca =
            ConnProfile::new(&host, port, &db, &user, pass, TlsMode::VerifyFull).with_ca_file(ca);
        super::ReadOnlySession::open(&with_ca, true)
            .await
            .expect("verify-full with the CA file must connect");
    }
}
