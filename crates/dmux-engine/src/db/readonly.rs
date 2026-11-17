//! Read-only transaction wrapper used for every engine query.
//!
//! Every statement runs inside `BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY`,
//! followed by the canonical `SET LOCAL` block from `docs/phase0/T003-sql-review.md` §4.

use std::future::Future;
use std::pin::Pin;

use tokio_postgres::{Client, IsolationLevel, Transaction};

use super::errors::DbError;

/// The fixed `SET LOCAL` statements emitted before the caller-supplied
/// `statement_timeout`.  `statement_timeout` is applied with `set_config(..., true)`
/// (a bound parameter, never interpolated) and then `lock_timeout` and
/// `application_name` close the block in the T003 §4 order.
pub const READ_ONLY_SETTINGS: [&str; 7] = [
    "SET LOCAL search_path = pg_catalog",
    "SET LOCAL timezone = 'UTC'",
    "SET LOCAL datestyle = 'ISO, YMD'",
    "SET LOCAL intervalstyle = 'iso_8601'",
    "SET LOCAL bytea_output = 'hex'",
    "SET LOCAL extra_float_digits = 0",
    "SET LOCAL xmloption = 'document'",
];

/// `statement_timeout` is applied with `set_config(..., true)`; the value is a
/// bound parameter `$1`, never a formatted string.
pub const STATEMENT_TIMEOUT_SQL: &str =
    "SELECT pg_catalog.set_config('statement_timeout', $1, true)";

/// `lock_timeout`, after `statement_timeout` (T003 §4 order).
pub const LOCK_TIMEOUT_SQL: &str = "SET LOCAL lock_timeout = '5s'";

/// `application_name`, closing the settings block.
pub const APPLICATION_NAME_SQL: &str = "SET LOCAL application_name = 'dmux'";

/// Validate a `statement_timeout` value.  Accepted forms match
/// `^[0-9]{1,9}(ms|s|min)?$`.  Anything else is rejected rather than being
/// interpolated into SQL.
pub fn validate_statement_timeout(value: &str) -> Result<(), DbError> {
    let bytes = value.as_bytes();
    let mut digits = 0;
    while digits < bytes.len() && bytes[digits].is_ascii_digit() {
        digits += 1;
    }
    if digits == 0 || digits > 9 {
        return Err(DbError::Config {
            message: format!("invalid statement_timeout: {value:?}"),
        });
    }
    let unit = &value[digits..];
    if !unit.is_empty() && unit != "ms" && unit != "s" && unit != "min" {
        return Err(DbError::Config {
            message: format!("invalid statement_timeout: {value:?}"),
        });
    }
    Ok(())
}

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
    validate_statement_timeout(statement_timeout)?;
    for sql in READ_ONLY_SETTINGS {
        executor.execute(sql, &[]).await?;
    }
    executor
        .execute(STATEMENT_TIMEOUT_SQL, &[&statement_timeout])
        .await?;
    executor.execute(LOCK_TIMEOUT_SQL, &[]).await?;
    executor.execute(APPLICATION_NAME_SQL, &[]).await?;
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
        &'b mut Transaction<'a>,
    ) -> Pin<Box<dyn Future<Output = Result<T, DbError>> + 'b>>,
{
    let mut txn = client
        .build_transaction()
        .isolation_level(IsolationLevel::RepeatableRead)
        .read_only(true)
        .start()
        .await
        .map_err(DbError::from_postgres)?;

    apply_readonly_settings(&txn, statement_timeout).await?;

    let result = f(&mut txn).await;
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
    use super::{
        apply_readonly_settings, validate_statement_timeout, APPLICATION_NAME_SQL,
        LOCK_TIMEOUT_SQL, READ_ONLY_SETTINGS, STATEMENT_TIMEOUT_SQL,
    };

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
    }

    #[test]
    fn statement_timeout_is_a_bound_parameter_not_interpolated() {
        assert!(STATEMENT_TIMEOUT_SQL.contains("$1"));
        assert!(!STATEMENT_TIMEOUT_SQL.contains("{"));
        assert_eq!(
            STATEMENT_TIMEOUT_SQL,
            "SELECT pg_catalog.set_config('statement_timeout', $1, true)"
        );
    }

    #[test]
    fn statement_timeout_validation() {
        for ok in ["5s", "30s", "300ms", "10min", "1800s", "0", "123456789s"] {
            assert!(
                validate_statement_timeout(ok).is_ok(),
                "{ok} should be valid"
            );
        }
        for bad in ["", "s", "5 seconds", "1234567890", "5s5", "5; DROP", "'5s'"] {
            assert!(
                validate_statement_timeout(bad).is_err(),
                "{bad:?} should be rejected"
            );
        }
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
                STATEMENT_TIMEOUT_SQL,
                LOCK_TIMEOUT_SQL,
                APPLICATION_NAME_SQL,
            ],
        ]
        .concat()
        .into_iter()
        .map(String::from)
        .collect();

        assert_eq!(*calls.lock().unwrap(), expected);
    }

    #[tokio::test]
    async fn rejects_invalid_statement_timeout_before_any_query() {
        let calls = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let executor = MockExecutor {
            calls: std::sync::Arc::clone(&calls),
        };
        let err = apply_readonly_settings(&executor, "5; DROP")
            .await
            .unwrap_err();
        assert!(matches!(err, super::DbError::Config { .. }));
        assert!(
            calls.lock().unwrap().is_empty(),
            "no SQL may run when the timeout is invalid"
        );
    }
}
