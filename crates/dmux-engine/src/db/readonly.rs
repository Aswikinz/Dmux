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

/// `row_security = off` (PostgreSQL 9.5+).  With it off, PostgreSQL raises
/// SQLSTATE 42501 instead of silently filtering rows whenever a policy would
/// apply, which is the fail-closed behaviour Dmux wants.  It must never be
/// emitted on 9.4, where the setting does not exist (rule 3).
pub const ROW_SECURITY_OFF_SQL: &str = "SET LOCAL row_security = off";

/// PostgreSQL `server_version_num` at which `row_security` became a setting.
pub const ROW_SECURITY_MIN_VERSION: i32 = 90500;

/// The `SET LOCAL` statements to emit for a given server version.  On 9.5 and
/// later this appends [`ROW_SECURITY_OFF_SQL`]; on 9.4 it stays the base block.
pub fn readonly_settings(version_num: i32) -> Vec<&'static str> {
    let mut settings = READ_ONLY_SETTINGS.to_vec();
    if version_num >= ROW_SECURITY_MIN_VERSION {
        settings.push(ROW_SECURITY_OFF_SQL);
    }
    settings
}

/// Read `server_version_num` from a client before a transaction is opened, so
/// the read-only settings block can branch on the version.
async fn server_version_num(client: &mut Client) -> Result<i32, DbError> {
    let row = client
        .query_one("SHOW server_version_num", &[])
        .await
        .map_err(DbError::from_postgres)?;
    let text: String = row.get(0);
    text.parse::<i32>().map_err(|_| DbError::Config {
        message: format!("server_version_num is not an integer: {text}"),
    })
}

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
/// SQL sequence.  `version_num` is the server's `server_version_num`; on 9.5+
/// the block also disables row-level security (fail-closed), on 9.4 it does
/// not.
pub async fn apply_readonly_settings(
    executor: &impl SettingsExecutor,
    statement_timeout: &str,
    version_num: i32,
) -> Result<(), DbError> {
    validate_statement_timeout(statement_timeout)?;
    for sql in readonly_settings(version_num) {
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
    let version_num = server_version_num(client).await?;
    let mut txn = client
        .build_transaction()
        .isolation_level(IsolationLevel::RepeatableRead)
        .read_only(true)
        .start()
        .await
        .map_err(DbError::from_postgres)?;

    apply_readonly_settings(&txn, statement_timeout, version_num).await?;

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
        apply_readonly_settings, readonly_settings, validate_statement_timeout,
        APPLICATION_NAME_SQL, LOCK_TIMEOUT_SQL, READ_ONLY_SETTINGS, ROW_SECURITY_MIN_VERSION,
        ROW_SECURITY_OFF_SQL, STATEMENT_TIMEOUT_SQL,
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
        assert_eq!(ROW_SECURITY_OFF_SQL, "SET LOCAL row_security = off");
    }

    #[test]
    fn settings_block_branches_on_version() {
        // 9.4: no row_security setting.
        let v94 = readonly_settings(ROW_SECURITY_MIN_VERSION - 1);
        assert_eq!(v94.len(), READ_ONLY_SETTINGS.len());
        assert!(!v94.contains(&ROW_SECURITY_OFF_SQL));
        // 9.5+: row_security = off is appended last.
        let v95 = readonly_settings(ROW_SECURITY_MIN_VERSION);
        assert_eq!(v95.len(), READ_ONLY_SETTINGS.len() + 1);
        assert_eq!(v95.last(), Some(&ROW_SECURITY_OFF_SQL));
        assert_eq!(&v95[..READ_ONLY_SETTINGS.len()], READ_ONLY_SETTINGS);
        // The base block is unchanged.
        assert_eq!(&v94[..], READ_ONLY_SETTINGS);
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
    async fn applies_settings_in_exact_order_on_95_plus() {
        let calls = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let executor = MockExecutor {
            calls: std::sync::Arc::clone(&calls),
        };
        apply_readonly_settings(&executor, "30s", 170000)
            .await
            .unwrap();

        let expected: Vec<String> = [
            readonly_settings(170000).as_slice(),
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
        assert_eq!(
            calls
                .lock()
                .unwrap()
                .iter()
                .filter(|s| *s == ROW_SECURITY_OFF_SQL)
                .count(),
            1,
            "row_security = off must be emitted once on 9.5+"
        );
    }

    #[tokio::test]
    async fn applies_settings_without_row_security_on_94() {
        let calls = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let executor = MockExecutor {
            calls: std::sync::Arc::clone(&calls),
        };
        apply_readonly_settings(&executor, "30s", 90426)
            .await
            .unwrap();

        let expected: Vec<String> = [
            readonly_settings(90426).as_slice(),
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
        assert!(
            !calls
                .lock()
                .unwrap()
                .iter()
                .any(|s| s == ROW_SECURITY_OFF_SQL),
            "9.4 must not receive row_security = off (rule 3)"
        );
    }

    #[tokio::test]
    async fn rejects_invalid_statement_timeout_before_any_query() {
        let calls = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let executor = MockExecutor {
            calls: std::sync::Arc::clone(&calls),
        };
        let err = apply_readonly_settings(&executor, "5; DROP", 170000)
            .await
            .unwrap_err();
        assert!(matches!(err, super::DbError::Config { .. }));
        assert!(
            calls.lock().unwrap().is_empty(),
            "no SQL may run when the timeout is invalid"
        );
    }
}
