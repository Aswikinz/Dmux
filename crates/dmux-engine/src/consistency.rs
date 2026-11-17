//! Consistency (PLAN.md 3.4): change counters, storage identity, server
//! epochs, replica replay and the WAL position.
//!
//! * **Change detection**: for every table, and every table that inherits from
//!   it (partitions and inheritance children, found through `pg_inherits`), the
//!   sum of `pg_stat_user_tables.n_tup_ins/upd/del` plus each relation's
//!   `pg_class.relfilenode` (all 9.4-safe).  TRUNCATE, VACUUM FULL and CLUSTER
//!   do not move the tuple counters but always give the relation a new
//!   relfilenode, so a truncate-and-reload with the same row count is still
//!   seen.  A table whose counters moved is CHANGED_DURING_CHECK.
//! * **Stats flush lag**: the tuple counters are server statistics that each
//!   writing backend flushes with a short delay (about half a second, up to a
//!   second under load).  The runner therefore waits `RunConfig::stats_settle`
//!   (default 2 s, definition key `stats_settle_ms`) before the final counter
//!   read.  A run on a frozen source or a paused replica is still the backstop.
//! * **Server epoch**: `pg_postmaster_start_time()` and the current database's
//!   `pg_stat_database.stats_reset`.  Counters are only comparable within one
//!   epoch (a restart or a stats reset zeroes them), so a stored resume row is
//!   reused only when both are identical.
//! * **Replicas**: on a server in recovery the counters never move, because
//!   replayed changes are not counted.  The replay position is read before and
//!   after each table's scans; if it advanced (or cannot be read) the table is
//!   CHANGED_DURING_CHECK.  Replay that is paused and did not move is recorded
//!   as "replay paused", which is the safe way to check a replica.
//! * **WAL position**: recorded at run start and end and per finished table,
//!   version-branched, and replica-safe.
//!
//! The independent spot check lives in `run::spot`.

use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio_postgres::Transaction;

use crate::results::Verdict;

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

/// The change counters of one table and its descendants.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct ChangeCounters {
    pub n_tup_ins: i64,
    pub n_tup_upd: i64,
    pub n_tup_del: i64,
    /// `oid:relfilenode` of the table and every descendant, sorted by oid.  A
    /// TRUNCATE, VACUUM FULL or CLUSTER changes it; so does a partition being
    /// attached or detached.
    #[serde(default)]
    pub storage: String,
}

impl ChangeCounters {
    /// Whether anything moved between `self` (before) and `after`: any tuple
    /// counter or any relation's storage.
    pub fn moved(&self, after: &ChangeCounters) -> bool {
        self.n_tup_ins != after.n_tup_ins
            || self.n_tup_upd != after.n_tup_upd
            || self.n_tup_del != after.n_tup_del
            || self.storage != after.storage
    }
}

/// The counters query: the table found by exact name (bound parameters), then
/// every relation inheriting from it, recursively.  9.4-safe (`WITH
/// RECURSIVE`, `pg_inherits`, `string_agg ... ORDER BY`).
pub const COUNTERS_SQL: &str = "WITH RECURSIVE tree(oid) AS ( \
       SELECT c.oid FROM pg_catalog.pg_class c \
       JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace \
       WHERE n.nspname = $1 AND c.relname = $2 \
     UNION \
       SELECT i.inhrelid FROM pg_catalog.pg_inherits i JOIN tree t ON i.inhparent = t.oid \
     ) \
     SELECT count(*)::pg_catalog.int8, \
       COALESCE(sum(s.n_tup_ins), 0)::pg_catalog.int8, \
       COALESCE(sum(s.n_tup_upd), 0)::pg_catalog.int8, \
       COALESCE(sum(s.n_tup_del), 0)::pg_catalog.int8, \
       COALESCE(string_agg(c.oid::pg_catalog.text || ':' || c.relfilenode::pg_catalog.text, ',' ORDER BY c.oid), '') \
     FROM tree t JOIN pg_catalog.pg_class c ON c.oid = t.oid \
     LEFT JOIN pg_catalog.pg_stat_user_tables s ON s.relid = c.oid";

/// Read the change counters of one table and its descendants (zeros and empty
/// storage when the table does not exist).  9.4-safe.
pub async fn read_counters(
    txn: &Transaction<'_>,
    schema: &str,
    table: &str,
) -> Result<ChangeCounters, ConsistencyError> {
    let r = txn.query_one(COUNTERS_SQL, &[&schema, &table]).await?;
    Ok(ChangeCounters {
        n_tup_ins: r.try_get(1)?,
        n_tup_upd: r.try_get(2)?,
        n_tup_del: r.try_get(3)?,
        storage: r.try_get(4)?,
    })
}

/// Read whether `track_counts` is on for this session (9.4-safe, read-only,
/// fixed SQL).  When it is off the tuple counters never move, so change
/// detection cannot vouch for a table (M3).
pub async fn read_track_counts(txn: &Transaction<'_>) -> Result<bool, ConsistencyError> {
    let row = txn
        .query_one("SELECT pg_catalog.current_setting('track_counts')", &[])
        .await?;
    let value: String = row.try_get(0)?;
    Ok(value.eq_ignore_ascii_case("on"))
}

/// Why change detection cannot vouch for one live side of one table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SideChangeDetection {
    /// `track_counts` is on and the table is not foreign: counters move.
    Ok,
    /// `track_counts` is off: tuple counters never move.
    TrackCountsOff,
    /// A foreign table (relkind `f`): it has no counters at all.
    ForeignTable,
}

impl SideChangeDetection {
    pub fn of(track_counts_on: bool, foreign: bool) -> Self {
        if foreign {
            SideChangeDetection::ForeignTable
        } else if !track_counts_on {
            SideChangeDetection::TrackCountsOff
        } else {
            SideChangeDetection::Ok
        }
    }

    /// Whether a stored result for this side may be reused on resume.
    pub fn ok(self) -> bool {
        matches!(self, SideChangeDetection::Ok)
    }
}

/// The per-table NEEDS_REVIEW floor when change detection is unavailable on
/// either side, with the reason.  `None` when both sides can vouch, so the
/// runner must apply this floor to every data-level table (M3).
pub fn change_detection_floor(
    old: SideChangeDetection,
    new: SideChangeDetection,
) -> Option<(Verdict, String)> {
    match (old, new) {
        (SideChangeDetection::Ok, SideChangeDetection::Ok) => None,
        (SideChangeDetection::TrackCountsOff, _) => Some((
            Verdict::NeedsReview,
            "change detection unavailable: track_counts is off on old".to_string(),
        )),
        (_, SideChangeDetection::TrackCountsOff) => Some((
            Verdict::NeedsReview,
            "change detection unavailable: track_counts is off on new".to_string(),
        )),
        _ => Some((
            Verdict::NeedsReview,
            "change detection unavailable: foreign table".to_string(),
        )),
    }
}

/// What the tuple counters of a server are relative to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct ServerEpoch {
    /// `pg_postmaster_start_time()` as text; `None` when it could not be read.
    pub postmaster_start: Option<String>,
    /// The current database's `pg_stat_database.stats_reset` (NULL when the
    /// statistics were never reset).
    pub stats_reset: Option<String>,
}

impl ServerEpoch {
    /// Counters read in `self` and `now` are comparable: the server did not
    /// restart, the statistics were not reset, and the start time is known.
    pub fn same_as(&self, now: &ServerEpoch) -> bool {
        self.postmaster_start.is_some() && self == now
    }
}

/// The epoch query (9.4-safe: both exist since 9.1).
pub const EPOCH_SQL: &str = "SELECT pg_catalog.pg_postmaster_start_time()::pg_catalog.text, \
       (SELECT d.stats_reset::pg_catalog.text FROM pg_catalog.pg_stat_database d \
        WHERE d.datname = pg_catalog.current_database())";

pub async fn read_server_epoch(txn: &Transaction<'_>) -> Result<ServerEpoch, ConsistencyError> {
    let r = txn.query_one(EPOCH_SQL, &[]).await?;
    Ok(ServerEpoch {
        postmaster_start: r.try_get(0)?,
        stats_reset: r.try_get(1)?,
    })
}

/// A server's recovery (replica) state at one moment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct ReplicaState {
    pub in_recovery: bool,
    /// The last replayed WAL position (only in recovery).
    pub replay_lsn: Option<String>,
    /// Replay is paused (only in recovery).
    pub paused: bool,
}

/// The replica-state query for a server version.  The replay functions raise
/// an error outside recovery on some versions, so each is guarded by `CASE`.
pub fn replica_state_sql(version_num: i32) -> &'static str {
    if version_num >= 100000 {
        "SELECT pg_catalog.pg_is_in_recovery(), \
           CASE WHEN pg_catalog.pg_is_in_recovery() \
             THEN pg_catalog.pg_last_wal_replay_lsn()::pg_catalog.text END, \
           CASE WHEN pg_catalog.pg_is_in_recovery() \
             THEN pg_catalog.pg_is_wal_replay_paused() ELSE false END"
    } else {
        "SELECT pg_catalog.pg_is_in_recovery(), \
           CASE WHEN pg_catalog.pg_is_in_recovery() \
             THEN pg_catalog.pg_last_xlog_replay_location()::pg_catalog.text END, \
           CASE WHEN pg_catalog.pg_is_in_recovery() \
             THEN pg_catalog.pg_is_xlog_replay_paused() ELSE false END"
    }
}

pub async fn read_replica_state(
    txn: &Transaction<'_>,
    version_num: i32,
) -> Result<ReplicaState, ConsistencyError> {
    let r = txn.query_one(replica_state_sql(version_num), &[]).await?;
    Ok(ReplicaState {
        in_recovery: r.try_get(0)?,
        replay_lsn: r.try_get(1)?,
        paused: r.try_get::<_, Option<bool>>(2)?.unwrap_or(false),
    })
}

/// What a replica's state before and after a table's scans says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReplicaDecision {
    /// Not a replica: the change counters are authoritative.
    NotReplica,
    /// A replica whose replay did not move.
    Steady,
    /// A replica whose replay was paused throughout and did not move.
    Paused,
    /// Replay advanced, the position is unknown, or the recovery state
    /// flipped: changes may have been applied during the scans.
    Moved(&'static str),
}

impl ReplicaDecision {
    pub fn moved(&self) -> bool {
        matches!(self, ReplicaDecision::Moved(_))
    }
}

/// The replica decision matrix.  Fail-closed: an advanced position wins over
/// a paused flag (the pause came too late), and an unknown position while in
/// recovery counts as moved.
pub fn replica_decision(before: &ReplicaState, after: &ReplicaState) -> ReplicaDecision {
    if before.in_recovery != after.in_recovery {
        return ReplicaDecision::Moved("the server entered or left recovery during the check");
    }
    if !before.in_recovery {
        return ReplicaDecision::NotReplica;
    }
    match (&before.replay_lsn, &after.replay_lsn) {
        (Some(a), Some(b)) if a == b => {
            if before.paused && after.paused {
                ReplicaDecision::Paused
            } else {
                ReplicaDecision::Steady
            }
        }
        (Some(_), Some(_)) => ReplicaDecision::Moved("replica replay advanced during the scans"),
        _ => ReplicaDecision::Moved("the replica replay position could not be read"),
    }
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

    fn counters(ins: i64, storage: &str) -> ChangeCounters {
        ChangeCounters {
            n_tup_ins: ins,
            n_tup_upd: 2,
            n_tup_del: 3,
            storage: storage.into(),
        }
    }

    #[test]
    fn counters_change_detection() {
        let a = counters(1, "16384:16384");
        assert!(!a.moved(&a.clone()));
        assert!(a.moved(&ChangeCounters {
            n_tup_del: 4,
            ..a.clone()
        }));
    }

    #[test]
    fn change_detection_floor_for_track_counts_off_and_foreign() {
        use SideChangeDetection::*;
        assert_eq!(change_detection_floor(Ok, Ok), None);
        assert_eq!(
            change_detection_floor(TrackCountsOff, Ok),
            Some((
                Verdict::NeedsReview,
                "change detection unavailable: track_counts is off on old".into()
            ))
        );
        assert_eq!(
            change_detection_floor(Ok, TrackCountsOff),
            Some((
                Verdict::NeedsReview,
                "change detection unavailable: track_counts is off on new".into()
            ))
        );
        for (old, new) in [
            (ForeignTable, Ok),
            (Ok, ForeignTable),
            (ForeignTable, ForeignTable),
        ] {
            assert_eq!(
                change_detection_floor(old, new),
                Some((
                    Verdict::NeedsReview,
                    "change detection unavailable: foreign table".into()
                ))
            );
        }
        // The (track_counts, foreign) decision for one side.
        assert_eq!(SideChangeDetection::of(true, false), Ok);
        assert_eq!(SideChangeDetection::of(false, false), TrackCountsOff);
        assert_eq!(SideChangeDetection::of(true, true), ForeignTable);
        assert_eq!(SideChangeDetection::of(false, true), ForeignTable);
        assert!(Ok.ok());
        assert!(!TrackCountsOff.ok());
        assert!(!ForeignTable.ok());
    }

    #[test]
    fn a_truncate_and_reload_with_the_same_counts_is_detected_by_storage() {
        // TRUNCATE keeps n_tup_ins/upd/del; the reload of the same number of
        // rows after a matching delete leaves every counter equal.  Only the
        // relfilenode tells.
        let before = counters(10, "16384:16384,16390:16390");
        let after = counters(10, "16384:16401,16390:16390");
        assert!(before.moved(&after));
        // A partition attached or detached changes the descendant list.
        assert!(before.moved(&counters(10, "16384:16384")));
    }

    #[test]
    fn counters_sql_is_bound_and_covers_descendants() {
        assert!(COUNTERS_SQL.contains("nspname = $1 AND c.relname = $2"));
        assert!(COUNTERS_SQL.contains("pg_inherits"));
        assert!(COUNTERS_SQL.contains("relfilenode"));
        assert!(COUNTERS_SQL.contains("ORDER BY c.oid"));
        assert!(!COUNTERS_SQL.contains("array_position"), "9.4-safe");
    }

    #[test]
    fn epochs_must_be_known_and_identical() {
        let e = ServerEpoch {
            postmaster_start: Some("2026-11-17 08:00:00+00".into()),
            stats_reset: None,
        };
        assert!(e.same_as(&e.clone()));
        let reset = ServerEpoch {
            stats_reset: Some("2026-11-17 09:00:00+00".into()),
            ..e.clone()
        };
        assert!(
            !e.same_as(&reset),
            "a stats reset makes counters incomparable"
        );
        let restarted = ServerEpoch {
            postmaster_start: Some("2026-11-17 10:00:00+00".into()),
            ..e.clone()
        };
        assert!(!e.same_as(&restarted));
        assert!(!ServerEpoch::default().same_as(&ServerEpoch::default()));
    }

    fn replica(lsn: Option<&str>, paused: bool) -> ReplicaState {
        ReplicaState {
            in_recovery: true,
            replay_lsn: lsn.map(str::to_string),
            paused,
        }
    }

    #[test]
    fn the_replica_decision_matrix() {
        let primary = ReplicaState::default();
        assert_eq!(
            replica_decision(&primary, &primary),
            ReplicaDecision::NotReplica
        );
        // Not moved.
        let a = replica(Some("0/3000060"), false);
        assert_eq!(replica_decision(&a, &a), ReplicaDecision::Steady);
        // Moved.
        let b = replica(Some("0/3000148"), false);
        assert!(replica_decision(&a, &b).moved());
        // Paused and not moved: recorded, not a change.
        let p = replica(Some("0/3000060"), true);
        assert_eq!(replica_decision(&p, &p), ReplicaDecision::Paused);
        // Paused only at the end, or moved although paused: still moved.
        assert_eq!(replica_decision(&a, &p), ReplicaDecision::Steady);
        assert!(replica_decision(&p, &replica(Some("0/3000148"), true)).moved());
        // Unknown position or a flipped recovery state: moved.
        assert!(replica_decision(&replica(None, true), &replica(None, true)).moved());
        assert!(replica_decision(&primary, &a).moved());
        assert!(replica_decision(&a, &primary).moved());
    }

    #[test]
    fn replica_sql_is_version_branched_and_guarded() {
        let new = replica_state_sql(170000);
        let old = replica_state_sql(90624);
        assert!(new.contains("pg_last_wal_replay_lsn") && new.contains("pg_is_wal_replay_paused"));
        assert!(
            old.contains("pg_last_xlog_replay_location")
                && old.contains("pg_is_xlog_replay_paused")
        );
        assert!(!old.contains("_wal_"), "no 10+ function on 9.x");
        for sql in [new, old] {
            assert_eq!(
                sql.matches("CASE WHEN pg_catalog.pg_is_in_recovery()")
                    .count(),
                2
            );
        }
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
