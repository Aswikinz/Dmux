//! Query-cancel token for stopping long-running server statements.
//!
//! The token remembers the connection's TLS mode so a cancel request reuses it
//! (a TLS connection cannot be cancelled over a plaintext cancel socket).

use tokio_postgres::CancelToken as PgCancelToken;
use tokio_postgres_rustls::MakeRustlsConnect;

use super::errors::DbError;

/// The TLS connector used to open the connection, so `cancel()` reuses the same
/// mode instead of assuming plaintext.
#[derive(Clone)]
pub(crate) enum CancelTls {
    Plain(tokio_postgres::NoTls),
    Rustls(MakeRustlsConnect),
}

/// A thin wrapper around `tokio_postgres::CancelToken`, exposed for the
/// engine's pause/cancel paths.
///
/// `CancelToken` is `Clone` because PostgreSQL cancel requests are fire-and-forget
/// messages on a separate connection.
#[derive(Clone)]
pub struct CancelToken {
    inner: PgCancelToken,
    tls: CancelTls,
}

impl std::fmt::Debug for CancelToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CancelToken").finish_non_exhaustive()
    }
}

impl CancelToken {
    /// Create a token from a connected client and the connection's TLS mode.
    #[must_use]
    pub(crate) fn new(inner: PgCancelToken, tls: CancelTls) -> Self {
        Self { inner, tls }
    }

    /// Send a cancel request to the server for the statement that produced this
    /// token, reusing the connection's TLS mode.
    pub async fn cancel(self) -> Result<(), DbError> {
        let result = match self.tls {
            CancelTls::Plain(tls) => self.inner.cancel_query(tls).await,
            CancelTls::Rustls(tls) => self.inner.cancel_query(tls).await,
        };
        result.map_err(|e| DbError::Cancel {
            message: e.to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::CancelToken;

    // Constructing a real cancel token requires a connected client; that is
    // covered by integration tests against the mock databases.  Here we just
    // ensure the wrapper is public and Clone.
    #[test]
    fn cancel_token_wrapper_is_clone() {
        fn assert_clone<T: Clone>() {}
        assert_clone::<CancelToken>();
    }
}
