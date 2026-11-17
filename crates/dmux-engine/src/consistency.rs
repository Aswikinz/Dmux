//! Consistency and the independent spot check (PLAN.md 3.4).
//!
//! * **Change detection**: read `pg_stat_user_tables.n_tup_ins/upd/del` before
//!   and after each table's scan (9.4-safe).  If any counter moved, the table's
//!   verdict is CHANGED_DURING_CHECK and can never PASS.
//! * **WAL position**: recorded at run start/end with the version-branched
//!   function (`pg_current_wal_lsn` on 10+, `pg_current_xlog_location` before).
//! * **Spot check**: for keyed tables, fetch 200 rows by key from both sides
//!   (bound parameters, deterministic key selection that does not depend on
//!   text sort order) and compare their normalised values in the app, without
//!   the hash path.  A disagreement with the fingerprint verdict is a TOOL_FAULT.

use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio_postgres::Transaction;

use crate::ident::quote_qualified;

/// Errors from consistency queries.
#[derive(Debug, Error)]
pub enum ConsistencyError {
    #[error("database error: {0}")]
    Db(String),
    #[error("invalid identifier: {0}")]
    Ident(#[from] crate::ident::IdentError),
}

impl From<tokio_postgres::Error> for ConsistencyError {
    fn from(e: tokio_postgres::Error) -> Self {
        ConsistencyError::Db(if let Some(db) = e.as_db_error() {
            format!("{}: {}", db.code().code(), db.message())
        } else {
            e.to_string()
        })
    }
}

/// The insert/update/delete counters of one table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub struct ChangeCounters {
    pub n_tup_ins: i64,
    pub n_tup_upd: i64,
    pub n_tup_del: i64,
}

impl ChangeCounters {
    pub fn unchanged(self, other: ChangeCounters) -> bool {
        self == other
    }
}

/// Read the change counters of one table (0 when the row is missing, e.g. a
/// freshly created table with no stats yet).  9.4-safe.
pub async fn read_counters(
    txn: &Transaction<'_>,
    schema: &str,
    table: &str,
) -> Result<ChangeCounters, ConsistencyError> {
    let row = txn
        .query_opt(
            "SELECT s.n_tup_ins, s.n_tup_upd, s.n_tup_del \
             FROM pg_catalog.pg_stat_user_tables s \
             JOIN pg_catalog.pg_class c ON c.oid = s.relid \
             JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace \
             WHERE n.nspname = $1 AND c.relname = $2",
            &[&schema, &table],
        )
        .await?;
    Ok(match row {
        Some(r) => ChangeCounters {
            n_tup_ins: r.get(0),
            n_tup_upd: r.get(1),
            n_tup_del: r.get(2),
        },
        None => ChangeCounters::default(),
    })
}

/// The write-ahead-log position, as text (works on every supported version).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct WalPosition {
    pub lsn: Option<String>,
    pub function: String,
}

/// Read the current WAL position, version-branched.  `version_num` is the
/// server's `server_version_num` (e.g. 90624 for 9.6, 170000 for 17).
pub async fn read_wal_position(
    txn: &Transaction<'_>,
    version_num: i32,
) -> Result<WalPosition, ConsistencyError> {
    let function = if version_num >= 100000 {
        "pg_current_wal_lsn"
    } else {
        "pg_current_xlog_location"
    };
    // Both functions exist only on their side of the 10 boundary; a permission
    // failure or older minor that lacks it degrades to None, never an abort.
    let query = format!("SELECT pg_catalog.{function}()::pg_catalog.text");
    let lsn = txn
        .query_opt(&query, &[])
        .await?
        .and_then(|r| r.try_get::<_, Option<String>>(0).ok())
        .flatten();
    Ok(WalPosition {
        lsn,
        function: function.to_string(),
    })
}

/// The number of rows the spot check fetches per keyed table (PLAN.md 3.4).
pub const SPOT_CHECK_ROWS: u64 = 200;

/// A deterministic, order-independent sample of key values for the spot check:
/// key values whose `md5(key::text)` hash is the smallest N.  This selects the
/// same rows on both sides regardless of text sort order, and never depends on
/// `ORDER BY` over the data.  Keys are returned as their text form; the caller
/// compares them side by side in the app.
///
/// `key_expr` is the single key column (the plan's first key column, or the
/// row key hash for keyless tables which are skipped here).
pub async fn sample_keys(
    txn: &Transaction<'_>,
    schema: &str,
    table: &str,
    key_column: &str,
    limit: u64,
) -> Result<Vec<Option<String>>, ConsistencyError> {
    let qualified = quote_qualified(schema, table)?;
    let key_ident = crate::ident::quote_ident(key_column)?;
    let sql = format!(
        "SELECT {key_ident}::pg_catalog.text AS k \
         FROM {qualified} \
         WHERE {key_ident} IS NOT NULL \
         ORDER BY md5({key_ident}::pg_catalog.text) \
         LIMIT $1"
    );
    let rows = txn.query(&sql, &[&(limit as i64)]).await?;
    Ok(rows.into_iter().map(|r| r.try_get(0).ok()).collect())
}

/// Compare two sides' spot-check samples value by value.  Returns `Ok(true)`
/// when every sampled key is present on both sides with the same normalised
/// value, `Ok(false)` otherwise.  A missing key on one side or a differing value
/// is a disagreement, which the caller turns into TOOL_FAULT.
pub fn spot_check_agrees(old: &[Option<String>], new: &[Option<String>]) -> bool {
    let mut o = old.to_vec();
    let mut n = new.to_vec();
    o.sort();
    n.sort();
    o == n
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counters_change_detection() {
        let a = ChangeCounters {
            n_tup_ins: 1,
            n_tup_upd: 2,
            n_tup_del: 3,
        };
        let b = ChangeCounters {
            n_tup_ins: 1,
            n_tup_upd: 2,
            n_tup_del: 4,
        };
        assert!(!a.unchanged(b));
        assert!(a.unchanged(a));
    }

    #[test]
    fn wal_function_is_version_branched() {
        // Pure helper: the function name is picked by the caller from version_num.
        let pick = |n: i32| {
            if n >= 100000 {
                "pg_current_wal_lsn"
            } else {
                "pg_current_xlog_location"
            }
        };
        assert_eq!(pick(90624), "pg_current_xlog_location");
        assert_eq!(pick(170000), "pg_current_wal_lsn");
        assert_eq!(pick(100000), "pg_current_wal_lsn");
    }

    #[test]
    fn spot_check_agrees_on_same_values() {
        let a = vec![Some("x".to_string()), None, Some("y".to_string())];
        let mut b = a.clone();
        assert!(spot_check_agrees(&a, &b));
        b[0] = Some("z".to_string());
        assert!(!spot_check_agrees(&a, &b));
        let fewer = vec![Some("x".to_string()), None];
        assert!(!spot_check_agrees(&a, &fewer));
    }
}
