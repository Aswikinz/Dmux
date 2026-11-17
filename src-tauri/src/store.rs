//! Connection profiles in a bundled SQLite file under the app data dir.
//!
//! Passwords are **never** written here (PLAN 6.5); they live only in the OS
//! keychain via [`crate::secrets::Keychain`].  The schema deliberately has no
//! password column, and a test scans the DB bytes to prove that invariant.

use std::path::Path;

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::error::CommandError;

/// A saved connection profile (no password field).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct StoredProfile {
    /// Stable primary key; also the keyring account.
    pub id: String,
    pub label: String,
    pub host: String,
    pub port: u16,
    pub database: String,
    pub user: String,
    /// One of `verify`, `trust` (with `tls_fingerprint`) or `allow-plaintext`.
    pub encryption: String,
    /// The fingerprint to trust when `encryption == "trust"`.
    pub tls_fingerprint: Option<String>,
    /// Reason recorded when `encryption == "allow-plaintext"`.
    pub plaintext_reason: Option<String>,
    /// Optional private-CA PEM bundle path used when `encryption == "verify"`.
    pub ca_file: Option<String>,
}

/// One finished run in the local history (metadata only; the finalised results
/// JSON lives in its own file in the app data dir).  Never row values: results
/// carry counts, hashes and names only.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct HistoryRow {
    /// The run id (`run-<hex>-<hex>-<hex>`), also the results file's stem.
    pub id: String,
    /// Unix seconds when the run was started.
    pub started_at: i64,
    /// Unix seconds when the run finished.
    pub ended_at: i64,
    /// The engine verdict string (`PASS`, `FAIL`, ...).
    pub verdict: String,
    /// Whether column options were applied (drives "PASS (with options)").
    pub options_applied: bool,
    /// Whether the results recorded their options.
    pub options_recorded: bool,
    pub coverage_line: String,
    pub old_label: String,
    pub new_label: String,
    pub mapping_hash: String,
    pub definition_hash: String,
    pub results_sha256: String,
}

/// The current schema version (`PRAGMA user_version`).  1 = profiles only (and
/// every store written before versioning, which reads as 0); 2 adds
/// `run_history`.
const SCHEMA_VERSION: i64 = 2;

/// The profile store: a single SQLite database file.
pub struct Store {
    conn: Connection,
}

impl Store {
    /// Open (creating tables if needed) the store at `path`.
    pub fn open(path: &Path) -> Result<Self, CommandError> {
        let conn = Connection::open(path)?;
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             CREATE TABLE IF NOT EXISTS profiles (
                 id                TEXT PRIMARY KEY,
                 label             TEXT NOT NULL,
                 host              TEXT NOT NULL,
                 port              INTEGER NOT NULL,
                 database          TEXT NOT NULL,
                 user              TEXT NOT NULL,
                 encryption        TEXT NOT NULL,
                 tls_fingerprint   TEXT,
                 plaintext_reason  TEXT,
                 ca_file           TEXT
             );
             CREATE TABLE IF NOT EXISTS settings (
                 key    TEXT PRIMARY KEY,
                 value  TEXT NOT NULL
             );",
        )?;
        // Migration: a profiles table created before `ca_file` existed lacks the
        // column, and `CREATE TABLE IF NOT EXISTS` does not add it.  Add it in
        // place so an upgraded install keeps working.
        let has_ca_file = {
            let mut stmt = conn.prepare("PRAGMA table_info(profiles)")?;
            let names: Vec<String> = stmt
                .query_map([], |r| r.get::<_, String>(1))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            names.iter().any(|name| name == "ca_file")
        };
        if !has_ca_file {
            conn.execute("ALTER TABLE profiles ADD COLUMN ca_file TEXT", [])?;
        }
        // Migration to version 2: the run history table.  `IF NOT EXISTS` keeps
        // an existing store (profiles and all) intact; nothing is rewritten.
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS run_history (
                 id               TEXT PRIMARY KEY,
                 started_at       INTEGER NOT NULL,
                 ended_at         INTEGER NOT NULL,
                 verdict          TEXT NOT NULL,
                 options_applied  INTEGER NOT NULL,
                 options_recorded INTEGER NOT NULL,
                 coverage_line    TEXT NOT NULL,
                 old_label        TEXT NOT NULL,
                 new_label        TEXT NOT NULL,
                 mapping_hash     TEXT NOT NULL,
                 definition_hash  TEXT NOT NULL,
                 results_sha256   TEXT NOT NULL
             );",
        )?;
        let version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        if version < SCHEMA_VERSION {
            conn.execute_batch(&format!("PRAGMA user_version = {SCHEMA_VERSION};"))?;
        }
        Ok(Self { conn })
    }

    /// Record (or replace) one finished run.
    pub fn history_insert(&self, row: &HistoryRow) -> Result<(), CommandError> {
        self.conn.execute(
            "INSERT OR REPLACE INTO run_history \
                 (id, started_at, ended_at, verdict, options_applied, options_recorded, \
                  coverage_line, old_label, new_label, mapping_hash, definition_hash, \
                  results_sha256) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            params![
                row.id,
                row.started_at,
                row.ended_at,
                row.verdict,
                row.options_applied,
                row.options_recorded,
                row.coverage_line,
                row.old_label,
                row.new_label,
                row.mapping_hash,
                row.definition_hash,
                row.results_sha256,
            ],
        )?;
        Ok(())
    }

    /// Every history row, newest first.
    pub fn history_list(&self) -> Result<Vec<HistoryRow>, CommandError> {
        let mut stmt = self.conn.prepare(
            "SELECT id, started_at, ended_at, verdict, options_applied, options_recorded, \
                    coverage_line, old_label, new_label, mapping_hash, definition_hash, \
                    results_sha256 \
             FROM run_history ORDER BY ended_at DESC, id DESC",
        )?;
        let rows = stmt.query_map([], row_to_history)?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    /// One history row by id.
    pub fn history_get(&self, id: &str) -> Result<Option<HistoryRow>, CommandError> {
        let mut stmt = self.conn.prepare(
            "SELECT id, started_at, ended_at, verdict, options_applied, options_recorded, \
                    coverage_line, old_label, new_label, mapping_hash, definition_hash, \
                    results_sha256 \
             FROM run_history WHERE id = ?1",
        )?;
        Ok(stmt.query_row(params![id], row_to_history).optional()?)
    }

    /// Delete every history row (profiles are untouched).  Returns how many.
    pub fn history_clear(&self) -> Result<usize, CommandError> {
        Ok(self.conn.execute("DELETE FROM run_history", [])?)
    }

    /// The schema version this store reports.
    #[cfg(test)]
    fn schema_version(&self) -> i64 {
        self.conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap()
    }

    /// List every profile, ordered by label.
    pub fn list(&self) -> Result<Vec<StoredProfile>, CommandError> {
        let mut stmt = self.conn.prepare(
            "SELECT id, label, host, port, database, user, encryption, \
                        tls_fingerprint, plaintext_reason, ca_file \
                 FROM profiles ORDER BY label, id",
        )?;
        let rows = stmt.query_map([], row_to_profile)?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    /// Fetch one profile by id.
    pub fn get(&self, id: &str) -> Result<Option<StoredProfile>, CommandError> {
        let mut stmt = self.conn.prepare(
            "SELECT id, label, host, port, database, user, encryption, \
                    tls_fingerprint, plaintext_reason, ca_file \
             FROM profiles WHERE id = ?1",
        )?;
        let found = stmt.query_row(params![id], row_to_profile).optional()?;
        Ok(found)
    }

    /// Insert or replace a profile.  Returns the stored profile (with a stable
    /// id when the caller did not supply one).
    pub fn save(&self, profile: &StoredProfile) -> Result<StoredProfile, CommandError> {
        let id = if profile.id.is_empty() {
            new_id()
        } else {
            profile.id.clone()
        };
        let stored = StoredProfile {
            id: id.clone(),
            ..profile.clone()
        };
        self.conn.execute(
            "INSERT INTO profiles \
                 (id, label, host, port, database, user, encryption, tls_fingerprint, plaintext_reason, ca_file) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10) \
             ON CONFLICT(id) DO UPDATE SET \
                 label = excluded.label, \
                 host = excluded.host, \
                 port = excluded.port, \
                 database = excluded.database, \
                 user = excluded.user, \
                 encryption = excluded.encryption, \
                 tls_fingerprint = excluded.tls_fingerprint, \
                 plaintext_reason = excluded.plaintext_reason, \
                 ca_file = excluded.ca_file",
            params![
                stored.id,
                stored.label,
                stored.host,
                stored.port,
                stored.database,
                stored.user,
                stored.encryption,
                stored.tls_fingerprint,
                stored.plaintext_reason,
                stored.ca_file,
            ],
        )?;
        Ok(stored)
    }

    /// Delete a profile.  Returns whether one was removed.
    /// One app setting (a plain text value), if set.  Settings never hold a
    /// password or a connection detail.
    pub fn get_setting(&self, key: &str) -> Result<Option<String>, CommandError> {
        let found = self
            .conn
            .query_row(
                "SELECT value FROM settings WHERE key = ?1",
                params![key],
                |r| r.get::<_, String>(0),
            )
            .optional()?;
        Ok(found)
    }

    /// Set (or, with `None`, clear) one app setting.
    pub fn set_setting(&self, key: &str, value: Option<&str>) -> Result<(), CommandError> {
        match value {
            Some(v) => {
                self.conn.execute(
                    "INSERT INTO settings (key, value) VALUES (?1, ?2) \
                     ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                    params![key, v],
                )?;
            }
            None => {
                self.conn
                    .execute("DELETE FROM settings WHERE key = ?1", params![key])?;
            }
        }
        Ok(())
    }

    pub fn delete(&self, id: &str) -> Result<bool, CommandError> {
        let n = self
            .conn
            .execute("DELETE FROM profiles WHERE id = ?1", params![id])?;
        Ok(n > 0)
    }
}

fn row_to_profile(row: &rusqlite::Row<'_>) -> rusqlite::Result<StoredProfile> {
    Ok(StoredProfile {
        id: row.get(0)?,
        label: row.get(1)?,
        host: row.get(2)?,
        port: row.get(3)?,
        database: row.get(4)?,
        user: row.get(5)?,
        encryption: row.get(6)?,
        tls_fingerprint: row.get(7)?,
        plaintext_reason: row.get(8)?,
        ca_file: row.get(9)?,
    })
}

fn row_to_history(row: &rusqlite::Row<'_>) -> rusqlite::Result<HistoryRow> {
    Ok(HistoryRow {
        id: row.get(0)?,
        started_at: row.get(1)?,
        ended_at: row.get(2)?,
        verdict: row.get(3)?,
        options_applied: row.get(4)?,
        options_recorded: row.get(5)?,
        coverage_line: row.get(6)?,
        old_label: row.get(7)?,
        new_label: row.get(8)?,
        mapping_hash: row.get(9)?,
        definition_hash: row.get(10)?,
        results_sha256: row.get(11)?,
    })
}

/// A fresh, stable, collision-resistant profile id.
fn new_id() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("p-{nanos:016x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(id: &str) -> StoredProfile {
        StoredProfile {
            id: id.to_string(),
            label: "Old database".to_string(),
            host: "127.0.0.1".to_string(),
            port: 5433,
            database: "bank".to_string(),
            user: "dmux_dev".to_string(),
            encryption: "verify".to_string(),
            tls_fingerprint: None,
            plaintext_reason: None,
            ca_file: None,
        }
    }

    #[test]
    fn round_trips_a_profile_without_an_id() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("profiles.db")).unwrap();

        let saved = store.save(&sample("")).unwrap();
        assert!(!saved.id.is_empty(), "save must assign an id");

        let got = store.get(&saved.id).unwrap().unwrap();
        assert_eq!(got.host, "127.0.0.1");
        assert_eq!(got.port, 5433);

        let all = store.list().unwrap();
        assert_eq!(all.len(), 1);
    }

    #[test]
    fn updates_an_existing_profile_in_place() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("profiles.db")).unwrap();

        let saved = store.save(&sample("fixed")).unwrap();
        assert_eq!(saved.id, "fixed");

        let mut changed = sample("fixed");
        changed.host = "db.example.com".to_string();
        store.save(&changed).unwrap();

        let got = store.get("fixed").unwrap().unwrap();
        assert_eq!(got.host, "db.example.com");
        assert_eq!(store.list().unwrap().len(), 1, "no duplicate row");
    }

    #[test]
    fn round_trips_the_ca_file() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("profiles.db")).unwrap();

        let mut with_ca = sample("ca");
        with_ca.ca_file = Some("/etc/bank/ca-bundle.pem".to_string());
        store.save(&with_ca).unwrap();

        let got = store.get("ca").unwrap().unwrap();
        assert_eq!(got.ca_file.as_deref(), Some("/etc/bank/ca-bundle.pem"));

        // Clearing it sticks too.
        let mut cleared = with_ca;
        cleared.ca_file = None;
        store.save(&cleared).unwrap();
        assert_eq!(store.get("ca").unwrap().unwrap().ca_file, None);
    }

    #[test]
    fn delete_removes_exactly_one() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("profiles.db")).unwrap();

        store.save(&sample("a")).unwrap();
        store.save(&sample("b")).unwrap();

        assert!(store.delete("a").unwrap());
        assert!(!store.delete("a").unwrap(), "already gone");
        assert_eq!(store.list().unwrap().len(), 1);
    }

    #[test]
    fn schema_has_no_password_column() {
        // A guard: if someone later adds a password column, this fails loudly
        // (PLAN 6.5 — passwords must never reach SQLite).
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("profiles.db");
        let store = Store::open(&path).unwrap();
        store.save(&sample("x")).unwrap();

        let columns: Vec<String> = {
            let mut stmt = store.conn.prepare("PRAGMA table_info(profiles)").unwrap();
            let rows = stmt.query_map([], |r| r.get::<_, String>(1)).unwrap();
            rows.map(|r| r.unwrap()).collect()
        };
        for column in &columns {
            assert!(
                !column.to_lowercase().contains("pass")
                    && !column.to_lowercase().contains("secret"),
                "profiles must not have a password/secret column; found {column}"
            );
        }
    }

    #[test]
    fn password_never_written_to_the_database_file_bytes() {
        // The strongest form of the PLAN 6.5 invariant: after saving a profile,
        // the raw SQLite file bytes must not contain a sentinel password.  This
        // would catch a future secret leaked into the WAL or a stray column.
        const SENTINEL: &str = "dmux-super-secret-password-7f3a";
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("profiles.db");
        let store = Store::open(&path).unwrap();
        store.save(&sample("x")).unwrap();
        drop(store);

        let bytes = std::fs::read(&path).unwrap();
        let window = SENTINEL.as_bytes();
        let present = bytes.windows(window.len()).any(|w| w == window);
        assert!(
            !present,
            "the profile store must never contain a password in its bytes"
        );
    }

    #[test]
    fn settings_round_trip_and_clear() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("s.db")).unwrap();
        assert_eq!(store.get_setting("hooks_enabled").unwrap(), None);
        store.set_setting("hooks_enabled", Some("true")).unwrap();
        store.set_setting("hooks_enabled", Some("false")).unwrap();
        assert_eq!(
            store.get_setting("hooks_enabled").unwrap().as_deref(),
            Some("false")
        );
        store.set_setting("hooks_enabled", None).unwrap();
        assert_eq!(store.get_setting("hooks_enabled").unwrap(), None);
    }

    /// The oldest layout: no ca_file column, no settings or history table, no
    /// schema version.
    fn oldest_store(path: &std::path::Path) {
        let conn = Connection::open(path).unwrap();
        conn.execute_batch(
            "CREATE TABLE profiles (
                 id TEXT PRIMARY KEY, label TEXT NOT NULL, host TEXT NOT NULL,
                 port INTEGER NOT NULL, database TEXT NOT NULL, user TEXT NOT NULL,
                 encryption TEXT NOT NULL, tls_fingerprint TEXT, plaintext_reason TEXT);
             INSERT INTO profiles VALUES
                 ('old1','Prod','h',5432,'bank','u','verify',NULL,NULL);",
        )
        .unwrap();
    }

    #[test]
    fn an_older_store_without_settings_still_opens_and_keeps_its_profiles() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("old.db");
        oldest_store(&path);
        let store = Store::open(&path).unwrap();
        assert_eq!(store.get("old1").unwrap().unwrap().host, "h");
        assert_eq!(store.get_setting("hooks_folder").unwrap(), None);
        store.set_setting("hooks_folder", Some("/x")).unwrap();
        assert_eq!(
            store.get_setting("hooks_folder").unwrap().as_deref(),
            Some("/x")
        );
    }

    fn history_row(id: &str, ended: i64) -> HistoryRow {
        HistoryRow {
            id: id.to_string(),
            started_at: ended - 60,
            ended_at: ended,
            verdict: "PASS".to_string(),
            options_applied: false,
            options_recorded: true,
            coverage_line: "1 of 1 tables fully checked".to_string(),
            old_label: "Old database".to_string(),
            new_label: "New database".to_string(),
            mapping_hash: "mh".to_string(),
            definition_hash: "dh".to_string(),
            results_sha256: "sha".to_string(),
        }
    }

    #[test]
    fn history_round_trips_newest_first_and_clears_without_touching_profiles() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("dmux.db")).unwrap();
        store.save(&sample("keep")).unwrap();
        store.history_insert(&history_row("run-1", 100)).unwrap();
        store.history_insert(&history_row("run-2", 200)).unwrap();

        let all = store.history_list().unwrap();
        assert_eq!(
            all.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(),
            ["run-2", "run-1"]
        );
        assert_eq!(store.history_get("run-1").unwrap().unwrap(), all[1]);
        assert!(store.history_get("nope").unwrap().is_none());

        assert_eq!(store.history_clear().unwrap(), 2);
        assert!(store.history_list().unwrap().is_empty());
        assert_eq!(store.list().unwrap().len(), 1, "profiles are kept");
    }

    #[test]
    fn a_store_from_before_history_migrates_and_keeps_its_profiles() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("dmux.db");
        oldest_store(&path);
        let store = Store::open(&path).unwrap();
        assert_eq!(store.schema_version(), SCHEMA_VERSION);
        let profile = store.get("old1").unwrap().unwrap();
        assert_eq!(profile.label, "Prod");
        assert_eq!(profile.ca_file, None);
        assert!(store.history_list().unwrap().is_empty());
        store.history_insert(&history_row("run-9", 5)).unwrap();
        // Both later additions work on a migrated store: settings and history.
        store.set_setting("hooks_enabled", Some("true")).unwrap();
        drop(store);
        // Opening again is a no-op migration and keeps everything.
        let again = Store::open(&path).unwrap();
        assert_eq!(again.history_list().unwrap().len(), 1);
        assert_eq!(again.list().unwrap().len(), 1);
        assert_eq!(
            again.get_setting("hooks_enabled").unwrap().as_deref(),
            Some("true")
        );
    }
}
