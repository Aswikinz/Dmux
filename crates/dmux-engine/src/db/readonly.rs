//! Read-only transaction wrapper used for every engine query.
//!
//! Every statement runs inside `BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY`,
//! followed by the canonical `SET LOCAL` block from `docs/phase0/T003-sql-review.md` §4.

use std::future::Future;
use std::pin::Pin;

use tokio_postgres::{Client, IsolationLevel, Transaction};

use super::errors::DbError;

/// SQL statements emitted inside every read-only transaction, in order.
///
/// `statement_timeout` is supplied by the caller; the rest are fixed.
pub const READ_ONLY_SETTINGS: [&str; 8] = [
    "SET LOCAL search_path = pg_catalog",
    "SET LOCAL timezone = 'UTC'",
    "SET LOCAL datestyle = 'ISO, YMD'",
    "SET LOCAL intervalstyle = 'iso_8601'",
    "SET LOCAL bytea_output = 'hex'",
    "SET LOCAL extra_float_digits = 0",
    "SET LOCAL xmloption = 'document'",
    "SET LOCAL lock_timeout = '5s'",
];

/// An executor that can run the read-only settings block.
///
/// It is implemented for `tokio_postgres::Transaction` and is mockable in
/// unit tests.
#[async_trait::async_trait]
pub trait SettingsExecutor {
    async fn execute(
        &self,
        sql: &str,
        params: &[&(dyn tokio_postgres::types::ToSql + Sync)],
    ) -> Result<u64, DbError>;
}

#[async_trait::async_trait]
impl SettingsExecutor for Transaction<'_> {
    async fn execute(
        &self,
        sql: &str,
        params: &[&(dyn tokio_postgres::types::ToSql + Sync)],
    ) -> Result<u64, DbError> {
        Transaction::execute(self, sql, params)
            .await
            .map_err(DbError::from_postgres)
    }
}

/// Apply the canonical read-only `SET LOCAL` block.
///
/// This is public so unit tests can feed a mock executor and assert the exact
/// SQL sequence.
pub async fn apply_readonly_settings(
    executor: &impl SettingsExecutor,
    statement_timeout: &str,
) -> Result<(), DbError> {
    for sql in READ_ONLY_SETTINGS {
        executor.execute(sql, &[]).await?;
    }
    executor
        .execute(
            &format!("SET LOCAL statement_timeout = '{}'", statement_timeout),
            &[],
        )
        .await?;
    executor
        .execute("SET LOCAL application_name = 'dmux'", &[])
        .await?;
    Ok(())
}

/// Start a read-only, repeatable-read transaction, apply the canonical settings
/// block, run `f`, then commit.  Rolls back on error.
pub async fn with_readonly_txn<'a, T, F>(
    client: &'a mut Client,
    statement_timeout: &str,
    f: F,
) -> Result<T, DbError>
where
    F: for<'b> FnOnce(
        &'b Transaction<'a>,
    ) -> Pin<Box<dyn Future<Output = Result<T, DbError>> + 'b>>,
{
    let txn = client
        .build_transaction()
        .isolation_level(IsolationLevel::RepeatableRead)
        .read_only(true)
        .start()
        .await
        .map_err(DbError::from_postgres)?;

    apply_readonly_settings(&txn, statement_timeout).await?;

    let result = f(&txn).await;
    match result {
        Ok(value) => {
            txn.commit().await.map_err(DbError::from_postgres)?;
            Ok(value)
        }
        Err(err) => {
            txn.rollback().await.ok();
            Err(err)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{apply_readonly_settings, READ_ONLY_SETTINGS};

    struct MockExecutor {
        calls: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
    }

    #[async_trait::async_trait]
    impl super::SettingsExecutor for MockExecutor {
        async fn execute(
            &self,
            sql: &str,
            _params: &[&(dyn tokio_postgres::types::ToSql + Sync)],
        ) -> Result<u64, super::DbError> {
            self.calls.lock().unwrap().push(sql.to_string());
            Ok(0)
        }
    }

    #[test]
    fn settings_block_has_expected_statements() {
        assert_eq!(READ_ONLY_SETTINGS[0], "SET LOCAL search_path = pg_catalog");
        assert_eq!(READ_ONLY_SETTINGS[1], "SET LOCAL timezone = 'UTC'");
        assert_eq!(READ_ONLY_SETTINGS[2], "SET LOCAL datestyle = 'ISO, YMD'");
        assert_eq!(
            READ_ONLY_SETTINGS[3],
            "SET LOCAL intervalstyle = 'iso_8601'"
        );
        assert_eq!(READ_ONLY_SETTINGS[4], "SET LOCAL bytea_output = 'hex'");
        assert_eq!(READ_ONLY_SETTINGS[5], "SET LOCAL extra_float_digits = 0");
        assert_eq!(READ_ONLY_SETTINGS[6], "SET LOCAL xmloption = 'document'");
        assert_eq!(READ_ONLY_SETTINGS[7], "SET LOCAL lock_timeout = '5s'");
    }

    #[tokio::test]
    async fn applies_settings_in_exact_order() {
        let calls = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let executor = MockExecutor {
            calls: std::sync::Arc::clone(&calls),
        };
        apply_readonly_settings(&executor, "30s").await.unwrap();

        let expected: Vec<String> = [
            READ_ONLY_SETTINGS.as_slice(),
            &[
                "SET LOCAL statement_timeout = '30s'",
                "SET LOCAL application_name = 'dmux'",
            ],
        ]
        .concat()
        .into_iter()
        .map(String::from)
        .collect();

        assert_eq!(*calls.lock().unwrap(), expected);
    }
}
