//! Query-cancel token for stopping long-running server statements.

use tokio_postgres::CancelToken as PgCancelToken;

/// A thin wrapper around `tokio_postgres::CancelToken`, exposed for the
/// engine's pause/cancel paths.
///
/// `CancelToken` is `Clone` because PostgreSQL cancel requests are fire-and-forget
/// messages on a separate connection.
#[derive(Clone)]
pub struct CancelToken {
    inner: PgCancelToken,
}

impl std::fmt::Debug for CancelToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CancelToken").finish_non_exhaustive()
    }
}

impl CancelToken {
    /// Create a token from a connected client.
    #[must_use]
    pub fn new(inner: PgCancelToken) -> Self {
        Self { inner }
    }

    /// Send a cancel request to the server for the statement that produced this token.
    pub async fn cancel(self) -> Result<(), super::errors::DbError> {
        self.inner
            .cancel_query(tokio_postgres::NoTls)
            .await
            .map_err(|e| super::errors::DbError::Cancel {
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
        // Cannot build a standalone token without a connection; this test only
        // validates the type implements Clone.
        assert_clone::<CancelToken>();
    }
}
