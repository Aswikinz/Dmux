//! The per-run resume store (PLAN.md 4.6): a bundled SQLite database that saves
//! each table's result as it finishes, so a crashed or interrupted run resumes
//! where it stopped.
//!
//! Resume is only safe when nothing has changed, so the store is keyed:
//!
//! * the store records the **run key** (run-definition hash, mapping hash, tool
//!   version) and the run id.  [`Store::bind`] keeps the stored results and the
//!   run id only when the key is identical; otherwise it clears every stored
//!   table and starts a new run id;
//! * every saved table carries, for both sides, the change counters (tuple
//!   counters and relfilenodes), the server epoch (postmaster start time and
//!   the database's stats reset time) and the replica replay position at the
//!   moment it finished.  On resume the runner reads them again and reuses the
//!   row only when [`StoredTableResult::rescan_reason`] finds nothing: any
//!   counter or storage change, a restart, a stats reset, or replica replay
//!   since the row was stored forces a rescan;
//! * only conclusive results are reused, and a row that cannot be read back is
//!   reported as [`Loaded::Unreadable`], which the runner rescans; nothing is
//!   ever silently dropped.  Every write error is returned to the caller.
//!
//! It **never** stores passwords or row values (rule 7): only counts, hashes,
//! verdicts, names and reasons.

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::consistency::{ChangeCounters, ReplicaState, ServerEpoch};
use crate::results::{CheckLevel, TableResult, Verdict};

/// Errors from the resume store.
#[derive(Debug, Error)]
pub enum StoreError {
    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("could not encode a stored result: {0}")]
    Encode(String),
}

/// Store format version, recorded in `run_meta`; a store from another version
/// is cleared rather than trusted.
pub const STORE_VERSION: &str = "3";

/// What a saved table result was computed against.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct RunKey {
    pub definition_hash: String,
    pub mapping_hash: String,
    pub tool_version: String,
}

/// What one side looked like when a table finished (or looks like now).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct SideMark {
    pub counters: ChangeCounters,
    pub epoch: ServerEpoch,
    pub replica: ReplicaState,
}

impl SideMark {
    /// Why counters read in `self` (stored) and `now` cannot vouch that the
    /// table is unchanged, if they cannot.
    pub fn rescan_reason(&self, now: &SideMark) -> Option<&'static str> {
        if !self.epoch.same_as(&now.epoch) {
            return Some("the server restarted or its statistics were reset");
        }
        if self.counters.moved(&now.counters) {
            return Some("the table changed since its result was stored");
        }
        if self.replica.in_recovery != now.replica.in_recovery {
            return Some("the server entered or left recovery");
        }
        if now.replica.in_recovery
            && (now.replica.replay_lsn.is_none()
                || self.replica.replay_lsn != now.replica.replay_lsn)
        {
            return Some("the replica replayed changes since the result was stored");
        }
        None
    }
}

/// One table's saved result (no row values).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct StoredTableResult {
    pub table: String,
    pub level: CheckLevel,
    pub result: TableResult,
    /// Each side when the table finished.
    pub old: SideMark,
    pub new: SideMark,
    /// New-side WAL position when the table finished.
    pub wal_lsn: Option<String>,
}

impl StoredTableResult {
    /// Only a result that reached a conclusion may be reused; a fault or a
    /// changed-during-check result is always rescanned.
    pub fn is_reusable(&self) -> bool {
        matches!(
            self.result.verdict,
            Verdict::Pass | Verdict::Fail | Verdict::WidelyDifferent | Verdict::NeedsReview
        )
    }

    /// Why this stored row must be rescanned rather than reused, given the
    /// level now requested and both sides as they are now; `None` to reuse.
    pub fn rescan_reason(
        &self,
        level: CheckLevel,
        old_now: &SideMark,
        new_now: &SideMark,
    ) -> Option<&'static str> {
        if !self.is_reusable() {
            return Some("the stored result is not conclusive");
        }
        if self.level != level || self.result.level != level {
            return Some("the check level changed");
        }
        self.old
            .rescan_reason(old_now)
            .or_else(|| self.new.rescan_reason(new_now))
    }
}

/// The outcome of reading one stored table back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Loaded {
    Missing,
    /// A row exists but cannot be decoded: the runner must rescan the table.
    Unreadable,
    Found(Box<StoredTableResult>),
}

/// The resume store handle.
pub struct Store {
    conn: Connection,
}

const SCHEMA: &str = "CREATE TABLE IF NOT EXISTS run_meta (
         key TEXT PRIMARY KEY,
         value TEXT NOT NULL
     );
     CREATE TABLE IF NOT EXISTS table_results (
         table_name TEXT PRIMARY KEY,
         json TEXT NOT NULL
     );";

impl Store {
    /// Open (or create) the store at `path`, with the schema created if needed.
    pub fn open(path: &std::path::Path) -> Result<Self, StoreError> {
        let conn = Connection::open(path)?;
        conn.execute_batch(SCHEMA)?;
        Ok(Store { conn })
    }

    /// Open an in-memory store (tests).
    pub fn open_memory() -> Result<Self, StoreError> {
        let conn = Connection::open_in_memory()?;
        conn.execute_batch(SCHEMA)?;
        Ok(Store { conn })
    }

    /// Read a run-metadata value.
    pub fn meta(&self, key: &str) -> Result<Option<String>, StoreError> {
        Ok(self
            .conn
            .query_row(
                "SELECT value FROM run_meta WHERE key = ?1",
                params![key],
                |r| r.get(0),
            )
            .optional()?)
    }

    /// Write a run-metadata value.
    pub fn set_meta(&self, key: &str, value: &str) -> Result<(), StoreError> {
        self.conn.execute(
            "INSERT INTO run_meta(key, value) VALUES(?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )?;
        Ok(())
    }

    /// Bind the store to a run key and return the run id to use.
    ///
    /// When the stored key (and store version) equals `key`, the stored run id
    /// is returned and the saved tables are kept for resume.  Otherwise every
    /// saved table is cleared, `new_run_id` is recorded with the key, and it is
    /// returned.
    pub fn bind(&self, key: &RunKey, new_run_id: &str) -> Result<String, StoreError> {
        let same = self.meta("store_version")?.as_deref() == Some(STORE_VERSION)
            && self.meta("definition_hash")?.as_deref() == Some(key.definition_hash.as_str())
            && self.meta("mapping_hash")?.as_deref() == Some(key.mapping_hash.as_str())
            && self.meta("tool_version")?.as_deref() == Some(key.tool_version.as_str());
        if same {
            if let Some(id) = self.meta("run_id")? {
                return Ok(id);
            }
        }
        self.clear_tables()?;
        self.set_meta("store_version", STORE_VERSION)?;
        self.set_meta("definition_hash", &key.definition_hash)?;
        self.set_meta("mapping_hash", &key.mapping_hash)?;
        self.set_meta("tool_version", &key.tool_version)?;
        self.set_meta("run_id", new_run_id)?;
        Ok(new_run_id.to_string())
    }

    /// Save one finished table's result.
    pub fn save_table(&self, result: &StoredTableResult) -> Result<(), StoreError> {
        let json = serde_json::to_string(result).map_err(|e| StoreError::Encode(e.to_string()))?;
        self.conn.execute(
            "INSERT INTO table_results(table_name, json) VALUES(?1, ?2)
             ON CONFLICT(table_name) DO UPDATE SET json = excluded.json",
            params![result.table, json],
        )?;
        Ok(())
    }

    /// Read one table back.  A row that fails to decode is
    /// [`Loaded::Unreadable`], never an error that drops the table.
    pub fn load_table(&self, table: &str) -> Result<Loaded, StoreError> {
        let json: Option<String> = self
            .conn
            .query_row(
                "SELECT json FROM table_results WHERE table_name = ?1",
                params![table],
                |r| r.get(0),
            )
            .optional()?;
        Ok(match json {
            None => Loaded::Missing,
            Some(j) => match serde_json::from_str::<StoredTableResult>(&j) {
                Ok(s) if s.table == table => Loaded::Found(Box::new(s)),
                _ => Loaded::Unreadable,
            },
        })
    }

    /// The names of every table with a saved row (readable or not).
    pub fn finished_tables(&self) -> Result<Vec<String>, StoreError> {
        let mut stmt = self
            .conn
            .prepare("SELECT table_name FROM table_results ORDER BY table_name")?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    /// Remove all saved table results (a fresh run).
    pub fn clear_tables(&self) -> Result<(), StoreError> {
        self.conn.execute("DELETE FROM table_results", [])?;
        Ok(())
    }

    /// Overwrite a stored row with arbitrary text (tests: corrupt a row).
    #[doc(hidden)]
    pub fn put_raw(&self, table: &str, json: &str) -> Result<(), StoreError> {
        self.conn.execute(
            "INSERT INTO table_results(table_name, json) VALUES(?1, ?2)
             ON CONFLICT(table_name) DO UPDATE SET json = excluded.json",
            params![table, json],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(d: &str) -> RunKey {
        RunKey {
            definition_hash: d.into(),
            mapping_hash: "m".into(),
            tool_version: "0.0.1".into(),
        }
    }

    fn mark() -> SideMark {
        SideMark {
            counters: ChangeCounters::default(),
            epoch: ServerEpoch {
                postmaster_start: Some("2026-11-17 08:00:00+00".into()),
                stats_reset: None,
            },
            replica: ReplicaState::default(),
        }
    }

    #[test]
    fn an_unchanged_table_is_reused() {
        let st = stored("s.a", Verdict::Pass);
        assert_eq!(
            st.rescan_reason(CheckLevel::FullValues, &st.old, &st.new),
            None
        );
        assert!(st
            .rescan_reason(CheckLevel::CountsAndTotals, &st.old, &st.new)
            .is_some());
    }

    #[test]
    fn a_truncate_since_the_store_forces_a_rescan() {
        let st = stored("s.a", Verdict::Pass);
        let mut now = st.new.clone();
        now.counters.storage = "16384:16999".into();
        assert!(st
            .rescan_reason(CheckLevel::FullValues, &st.old, &now)
            .is_some());
    }

    #[test]
    fn a_stats_reset_or_restart_forces_a_rescan() {
        let st = stored("s.a", Verdict::Pass);
        // The counters read back identical (both zeroed by the reset), but the
        // epoch differs: they cannot vouch for the table.
        let mut now = st.old.clone();
        now.epoch.stats_reset = Some("2026-11-17 09:00:00+00".into());
        assert_eq!(
            st.rescan_reason(CheckLevel::FullValues, &now, &st.new),
            Some("the server restarted or its statistics were reset")
        );
        let mut now = st.new.clone();
        now.epoch.postmaster_start = Some("2026-11-17 10:00:00+00".into());
        assert!(st
            .rescan_reason(CheckLevel::FullValues, &st.old, &now)
            .is_some());
        // An unknown start time never vouches.
        let mut blind = st.clone();
        blind.old.epoch.postmaster_start = None;
        let mut now = st.old.clone();
        now.epoch.postmaster_start = None;
        assert!(blind
            .rescan_reason(CheckLevel::FullValues, &now, &st.new)
            .is_some());
    }

    #[test]
    fn replica_replay_since_the_store_forces_a_rescan() {
        let mut st = stored("s.a", Verdict::Pass);
        st.new.replica = ReplicaState {
            in_recovery: true,
            replay_lsn: Some("0/3000060".into()),
            paused: false,
        };
        let same = st.new.clone();
        assert_eq!(
            st.rescan_reason(CheckLevel::FullValues, &st.old, &same),
            None
        );
        let mut moved = st.new.clone();
        moved.replica.replay_lsn = Some("0/3000148".into());
        assert!(st
            .rescan_reason(CheckLevel::FullValues, &st.old, &moved)
            .is_some());
        let mut unknown = st.new.clone();
        unknown.replica.replay_lsn = None;
        assert!(st
            .rescan_reason(CheckLevel::FullValues, &st.old, &unknown)
            .is_some());
        let promoted = SideMark {
            replica: ReplicaState::default(),
            ..st.new.clone()
        };
        assert!(st
            .rescan_reason(CheckLevel::FullValues, &st.old, &promoted)
            .is_some());
    }

    fn stored(table: &str, verdict: Verdict) -> StoredTableResult {
        StoredTableResult {
            table: table.into(),
            level: CheckLevel::FullValues,
            result: TableResult {
                table: table.into(),
                level: CheckLevel::FullValues,
                verdict,
                old_rows: Some(100),
                new_rows: Some(99),
                columns_covered: Some(vec!["id".into()]),
                differing_buckets: Some(1),
                missing_buckets: Some(0),
                extra_buckets: Some(0),
                missing_rows: Some(1),
                extra_rows: Some(0),
                changed_rows: Some(0),
                reasons: vec!["row counts differ".into()],
            },
            old: mark(),
            new: SideMark {
                counters: ChangeCounters {
                    n_tup_ins: 1,
                    n_tup_upd: 2,
                    n_tup_del: 3,
                    storage: "16384:16384".into(),
                },
                ..mark()
            },
            wal_lsn: Some("0/16B3748".into()),
        }
    }

    #[test]
    fn round_trips_a_table() {
        let s = Store::open_memory().unwrap();
        s.save_table(&stored("s.a", Verdict::Fail)).unwrap();
        assert_eq!(
            s.load_table("s.a").unwrap(),
            Loaded::Found(Box::new(stored("s.a", Verdict::Fail)))
        );
        assert_eq!(s.load_table("s.b").unwrap(), Loaded::Missing);
        assert_eq!(s.finished_tables().unwrap(), vec!["s.a"]);
    }

    #[test]
    fn run_id_is_stable_for_the_same_key_and_new_for_another() {
        let s = Store::open_memory().unwrap();
        let id1 = s.bind(&key("d1"), "run-1").unwrap();
        assert_eq!(id1, "run-1");
        s.save_table(&stored("s.a", Verdict::Pass)).unwrap();
        // Same key: the stored id and tables survive, the proposed id is ignored.
        assert_eq!(s.bind(&key("d1"), "run-2").unwrap(), "run-1");
        assert_eq!(s.finished_tables().unwrap(), vec!["s.a"]);
        // Any key difference: tables cleared, new id.
        assert_eq!(s.bind(&key("d2"), "run-3").unwrap(), "run-3");
        assert!(s.finished_tables().unwrap().is_empty());
        let mut k = key("d2");
        k.tool_version = "9.9.9".into();
        s.save_table(&stored("s.a", Verdict::Pass)).unwrap();
        assert_eq!(s.bind(&k, "run-4").unwrap(), "run-4");
        assert!(s.finished_tables().unwrap().is_empty());
        let mut k = key("d2");
        k.mapping_hash = "other".into();
        assert_eq!(s.bind(&k, "run-5").unwrap(), "run-5");
    }

    #[test]
    fn an_unreadable_row_is_reported_not_dropped() {
        let s = Store::open_memory().unwrap();
        s.put_raw("s.a", "{not json").unwrap();
        assert_eq!(s.load_table("s.a").unwrap(), Loaded::Unreadable);
        // A row stored under another table's name is not trusted either.
        s.put_raw(
            "s.b",
            &serde_json::to_string(&stored("s.x", Verdict::Pass)).unwrap(),
        )
        .unwrap();
        assert_eq!(s.load_table("s.b").unwrap(), Loaded::Unreadable);
        assert_eq!(s.finished_tables().unwrap(), vec!["s.a", "s.b"]);
    }

    #[test]
    fn only_conclusive_results_are_reusable() {
        assert!(stored("s.a", Verdict::Pass).is_reusable());
        assert!(stored("s.a", Verdict::Fail).is_reusable());
        assert!(!stored("s.a", Verdict::ToolFault).is_reusable());
        assert!(!stored("s.a", Verdict::ChangedDuringCheck).is_reusable());
        assert!(!stored("s.a", Verdict::Warning).is_reusable());
    }

    #[test]
    fn save_errors_are_returned() {
        let s = Store::open_memory().unwrap();
        s.conn.execute_batch("DROP TABLE table_results").unwrap();
        assert!(s.save_table(&stored("s.a", Verdict::Pass)).is_err());
        assert!(s.load_table("s.a").is_err());
    }

    #[test]
    fn stores_no_row_values_by_construction() {
        let json = serde_json::to_string(&stored("s.a", Verdict::Pass)).unwrap();
        for field in ["counters", "epoch", "replica", "result", "wal_lsn"] {
            assert!(json.contains(field));
        }
    }
}
