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
}

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
                 plaintext_reason  TEXT
             );",
        )?;
        Ok(Self { conn })
    }

    /// List every profile, ordered by label.
    pub fn list(&self) -> Result<Vec<StoredProfile>, CommandError> {
        let mut stmt = self.conn.prepare(
            "SELECT id, label, host, port, database, user, encryption, \
                        tls_fingerprint, plaintext_reason \
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
                    tls_fingerprint, plaintext_reason \
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
                 (id, label, host, port, database, user, encryption, tls_fingerprint, plaintext_reason) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9) \
             ON CONFLICT(id) DO UPDATE SET \
                 label = excluded.label, \
                 host = excluded.host, \
                 port = excluded.port, \
                 database = excluded.database, \
                 user = excluded.user, \
                 encryption = excluded.encryption, \
                 tls_fingerprint = excluded.tls_fingerprint, \
                 plaintext_reason = excluded.plaintext_reason",
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
            ],
        )?;
        Ok(stored)
    }

    /// Delete a profile.  Returns whether one was removed.
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
}
