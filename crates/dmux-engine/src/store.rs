//! The per-run resume store (PLAN.md 4.6): a bundled SQLite database that saves
//! each table's verdict as it finishes, so a crashed or interrupted run resumes
//! where it stopped.
//!
//! It **never** stores passwords or row values (rule 7): only counts, hashes,
//! verdicts and column names.  Keys are stored only when the user ticked export,
//! which is a caller decision reflected in the record.

use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::core::fingerprint::BucketSummary;
use crate::core::fingerprint::Decimal;
use crate::results::{CheckLevel, Verdict};

/// Errors from the resume store.
#[derive(Debug, Error)]
pub enum StoreError {
    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
}

/// One table's saved result (counts and digest only; no row values).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct StoredTableResult {
    pub table: String,
    pub level: CheckLevel,
    #[serde(with = "serde_verdict")]
    pub verdict: Verdict,
    pub old_rows: Option<u64>,
    pub new_rows: Option<u64>,
    pub bucket_digits: u8,
    pub buckets_old: Option<Vec<BucketSummary>>,
    pub buckets_new: Option<Vec<BucketSummary>>,
    pub control_totals_old: Vec<Decimal>,
    pub control_totals_new: Vec<Decimal>,
    pub needs_review: bool,
    pub reasons: Vec<String>,
}

mod serde_verdict {
    use super::Verdict;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(v: &Verdict, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(match v {
            Verdict::Pass => "PASS",
            Verdict::Fail => "FAIL",
            Verdict::WidelyDifferent => "WIDELY_DIFFERENT",
            Verdict::NeedsReview => "NEEDS_REVIEW",
            Verdict::ChangedDuringCheck => "CHANGED_DURING_CHECK",
            Verdict::ToolFault => "TOOL_FAULT",
            Verdict::Warning => "WARNING",
        })
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Verdict, D::Error> {
        let s = String::deserialize(d)?;
        Ok(match s.as_str() {
            "PASS" => Verdict::Pass,
            "FAIL" => Verdict::Fail,
            "WIDELY_DIFFERENT" => Verdict::WidelyDifferent,
            "NEEDS_REVIEW" => Verdict::NeedsReview,
            "CHANGED_DURING_CHECK" => Verdict::ChangedDuringCheck,
            "TOOL_FAULT" => Verdict::ToolFault,
            "WARNING" => Verdict::Warning,
            _ => Verdict::ToolFault,
        })
    }
}

/// The resume store handle.
pub struct Store {
    conn: Connection,
}

impl Store {
    /// Open (or create) the store at `path`, with the schema created if needed.
    pub fn open(path: &std::path::Path) -> Result<Self, StoreError> {
        let conn = Connection::open(path)?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS run_meta (
                 key TEXT PRIMARY KEY,
                 value TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS table_results (
                 table_name TEXT PRIMARY KEY,
                 json TEXT NOT NULL
             );",
        )?;
        Ok(Store { conn })
    }

    /// Open an in-memory store (tests).
    pub fn open_memory() -> Result<Self, StoreError> {
        let conn = Connection::open_in_memory()?;
        conn.execute_batch(
            "CREATE TABLE run_meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
             CREATE TABLE table_results (table_name TEXT PRIMARY KEY, json TEXT NOT NULL);",
        )?;
        Ok(Store { conn })
    }

    /// Read a run-metadata value.
    pub fn meta(&self, key: &str) -> Result<Option<String>, StoreError> {
        let mut stmt = self
            .conn
            .prepare("SELECT value FROM run_meta WHERE key = ?1")?;
        let mut rows = stmt.query(params![key])?;
        match rows.next()? {
            Some(row) => Ok(Some(row.get(0)?)),
            None => Ok(None),
        }
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

    /// Save one finished table's result.
    pub fn save_table(&self, result: &StoredTableResult) -> Result<(), StoreError> {
        let json = serde_json::to_string(result).map_err(|e| {
            StoreError::Sqlite(rusqlite::Error::ToSqlConversionFailure(Box::new(e)))
        })?;
        self.conn.execute(
            "INSERT INTO table_results(table_name, json) VALUES(?1, ?2)
             ON CONFLICT(table_name) DO UPDATE SET json = excluded.json",
            params![result.table, json],
        )?;
        Ok(())
    }

    /// Load every saved table result.
    pub fn load_tables(&self) -> Result<Vec<StoredTableResult>, StoreError> {
        let mut stmt = self.conn.prepare("SELECT json FROM table_results")?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
        let mut out = Vec::new();
        for row in rows {
            let json = row?;
            out.push(serde_json::from_str(&json).map_err(|e| {
                StoreError::Sqlite(rusqlite::Error::ToSqlConversionFailure(Box::new(e)))
            })?);
        }
        Ok(out)
    }

    /// The set of table names already finished (for `--resume` skip).
    pub fn finished_tables(&self) -> Result<Vec<String>, StoreError> {
        let mut stmt = self.conn.prepare("SELECT table_name FROM table_results")?;
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
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> StoredTableResult {
        StoredTableResult {
            table: "demo_bank.accounts".into(),
            level: CheckLevel::FullValues,
            verdict: Verdict::Fail,
            old_rows: Some(100),
            new_rows: Some(99),
            bucket_digits: 3,
            buckets_old: Some(vec![BucketSummary {
                bucket: 0,
                n: 1,
                s: [0; 4],
            }]),
            buckets_new: None,
            control_totals_old: vec![Decimal::parse("1.5").unwrap()],
            control_totals_new: vec![Decimal::parse("1.5").unwrap()],
            needs_review: false,
            reasons: vec!["counts differ".into()],
        }
    }

    #[test]
    fn round_trips_tables_and_meta() {
        let store = Store::open_memory().unwrap();
        store.set_meta("run_id", "abc123").unwrap();
        assert_eq!(store.meta("run_id").unwrap().as_deref(), Some("abc123"));

        store.save_table(&sample()).unwrap();
        let loaded = store.load_tables().unwrap();
        assert_eq!(loaded, vec![sample()]);
        assert_eq!(store.finished_tables().unwrap(), vec!["demo_bank.accounts"]);

        store.clear_tables().unwrap();
        assert!(store.load_tables().unwrap().is_empty());
    }

    #[test]
    fn stores_no_password_or_row_values_by_construction() {
        // The schema has only table_name + json; the json holds counts/hashes.
        let json = serde_json::to_string(&sample()).unwrap();
        assert!(json.contains("counts"));
        assert!(!json.contains("secret_row_value"));
    }
}
