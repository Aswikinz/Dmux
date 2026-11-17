//! Consistency (PLAN.md 3.4): change counters and the WAL position.
//!
//! * **Change detection**: `pg_stat_user_tables.n_tup_ins/upd/del` (9.4-safe)
//!   is read for every table before the scans and after them; a table whose
//!   counters moved is CHANGED_DURING_CHECK and can never PASS.  These are
//!   server statistics, flushed by each writing backend with a short delay
//!   (up to about a second), so a write committed in the last moment before the
//!   final read may not be visible yet; the DBA guide's advice to run on a
//!   frozen source or a replica is the backstop.
//! * **WAL position**: recorded at run start and end and per finished table,
//!   version-branched, and replica-safe.
//!
//! The independent spot check lives in `run::spot`.

use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio_postgres::Transaction;

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

/// The SQL that reads the WAL position for a server version.  On a primary it
/// is the current insert position; on a replica (in recovery, where
/// `pg_current_wal_lsn()` raises an error) it is the last replayed position,
/// so a run against a replica (the advice in the DBA guide) never aborts here.
pub fn wal_position_sql(version_num: i32) -> (&'static str, &'static str) {
    if version_num >= 100000 {
        (
            "pg_current_wal_lsn",
            "SELECT CASE WHEN pg_catalog.pg_is_in_recovery() \
             THEN pg_catalog.pg_last_wal_replay_lsn()::pg_catalog.text \
             ELSE pg_catalog.pg_current_wal_lsn()::pg_catalog.text END",
        )
    } else {
        (
            "pg_current_xlog_location",
            "SELECT CASE WHEN pg_catalog.pg_is_in_recovery() \
             THEN pg_catalog.pg_last_xlog_replay_location()::pg_catalog.text \
             ELSE pg_catalog.pg_current_xlog_location()::pg_catalog.text END",
        )
    }
}

/// Read the current WAL position, version-branched.  `version_num` is the
/// server's `server_version_num` (e.g. 90624 for 9.6, 170000 for 17).
pub async fn read_wal_position(
    txn: &Transaction<'_>,
    version_num: i32,
) -> Result<WalPosition, ConsistencyError> {
    let (function, sql) = wal_position_sql(version_num);
    let lsn = txn
        .query_opt(sql, &[])
        .await?
        .and_then(|r| r.try_get::<_, Option<String>>(0).ok())
        .flatten();
    Ok(WalPosition {
        lsn,
        function: function.to_string(),
    })
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
    fn wal_function_is_version_branched_and_replica_safe() {
        assert_eq!(wal_position_sql(90624).0, "pg_current_xlog_location");
        assert_eq!(wal_position_sql(170000).0, "pg_current_wal_lsn");
        assert_eq!(wal_position_sql(100000).0, "pg_current_wal_lsn");
        for v in [90400, 90624, 100000, 180000] {
            let sql = wal_position_sql(v).1;
            assert!(sql.contains("pg_is_in_recovery()"), "{v}");
            assert!(!sql.contains('$'), "no interpolation");
        }
        assert!(wal_position_sql(90400)
            .1
            .contains("pg_last_xlog_replay_location"));
        assert!(wal_position_sql(170000)
            .1
            .contains("pg_last_wal_replay_lsn"));
    }
}
