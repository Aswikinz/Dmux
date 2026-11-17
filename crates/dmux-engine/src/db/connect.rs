//! Open a PostgreSQL connection with the encryption and authentication policy
//! from a [`ConnProfile`].

use std::sync::Arc;

use rustls::{
    client::danger::ServerCertVerifier,
    pki_types::{CertificateDer, ServerName, UnixTime},
    ClientConfig, RootCertStore, SignatureScheme,
};
use sha2::{Digest, Sha256};
use tokio_postgres::Client;
use tokio_postgres_rustls::MakeRustlsConnect;

use super::config::{ConnProfile, TlsMode};
use super::errors::DbError;

/// SHA-256 fingerprint of a certificate, formatted as lowercase hex.
fn cert_fingerprint(cert: &CertificateDer<'_>) -> String {
    let mut hasher = Sha256::new();
    hasher.update(cert.as_ref());
    format_hex(&hasher.finalize())
}

fn format_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// A verifier that accepts the server certificate only if its SHA-256
/// fingerprint matches the expected value.
#[derive(Debug)]
struct FingerprintVerifier {
    expected: String,
}

impl ServerCertVerifier for FingerprintVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        let fp = cert_fingerprint(end_entity);
        if fp.eq_ignore_ascii_case(&self.expected) {
            Ok(rustls::client::danger::ServerCertVerified::assertion())
        } else {
            Err(rustls::Error::General(format!(
                "certificate fingerprint {fp} does not match trusted fingerprint {}",
                self.expected
            )))
        }
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        vec![
            SignatureScheme::ECDSA_NISTP256_SHA256,
            SignatureScheme::ECDSA_NISTP384_SHA384,
            SignatureScheme::ED25519,
            SignatureScheme::RSA_PSS_SHA256,
            SignatureScheme::RSA_PSS_SHA384,
            SignatureScheme::RSA_PSS_SHA512,
            SignatureScheme::RSA_PKCS1_SHA256,
            SignatureScheme::RSA_PKCS1_SHA384,
            SignatureScheme::RSA_PKCS1_SHA512,
        ]
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
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
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
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        self.inner.verify_tls12_signature(message, cert, dss)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
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
                expected: expected.clone(),
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

/// Open a connection using the supplied profile and spawn the connection driver.
pub async fn connect(profile: &ConnProfile) -> Result<Client, DbError> {
    let tls_mode = profile.tls_mode.clone();
    let host = profile.host.clone();
    let port = profile.port;
    let dbname = profile.dbname.clone();
    let user = profile.user.clone();
    let password = profile.password.clone();

    let sslmode = match &tls_mode {
        TlsMode::VerifyFull | TlsMode::TrustFingerprint(_) => "require",
        TlsMode::Disable(_) => "disable",
    };

    let config_str = format!(
        "host={host} port={port} dbname={dbname} user={user} password={password} \
         sslmode={sslmode} application_name=dmux"
    );

    let config = config_str
        .parse::<tokio_postgres::Config>()
        .map_err(|e| DbError::Config {
            message: format!("invalid connection string: {e}"),
        })?;

    // Match arms have different connection stream types, so drive each branch separately
    // and return the `Client`.
    let client = match &tls_mode {
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
            client
        }
        _ => {
            let tls = make_tls_config(&tls_mode)?;
            let (client, conn) = config.connect(tls).await.map_err(|e| {
                let msg = e.to_string();
                if let Some(rest) = msg.strip_prefix("DMUX_UNTRUSTED:") {
                    let mut parts = rest.splitn(2, ':');
                    let fp = parts.next().unwrap_or("unknown").to_string();
                    DbError::UntrustedCert { fingerprint: fp }
                } else {
                    DbError::from_connect(e)
                }
            })?;
            tokio::spawn(async move {
                if let Err(e) = conn.await {
                    eprintln!("dmux connection driver finished: {e}");
                }
            });
            client
        }
    };

    Ok(client)
}

#[cfg(test)]
mod tests {
    use super::cert_fingerprint;

    #[test]
    fn fingerprint_is_lowercase_hex() {
        let cert = rustls::pki_types::CertificateDer::from(vec![0u8; 32]);
        let fp = cert_fingerprint(&cert);
        assert_eq!(fp.len(), 64);
        assert!(fp.chars().all(|c| c.is_ascii_hexdigit()));
        assert!(
            !fp.chars().any(|c| c.is_ascii_uppercase()),
            "fingerprint must be lowercase: {fp}"
        );
    }
}
