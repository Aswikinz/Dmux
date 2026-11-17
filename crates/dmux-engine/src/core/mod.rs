//! The comparison core: normalise, fingerprint, compare and drill down
//! (PLAN.md section 3). Only arjun-core edits this module.
//!
//! Every function that talks to a database takes a client that is already
//! inside the read-only transaction opened by the `db` wrapper. The core
//! never opens, commits or ends a transaction itself; it checks the session
//! settings before each scan and refuses to run in any other session.

pub mod apphash;
pub mod compare;
pub mod drilldown;
pub mod fingerprint;
pub mod normalise;

#[cfg(test)]
mod db_tests;
#[cfg(test)]
mod model;

use crate::ident::IdentError;
use normalise::NormaliseError;
use tokio_postgres::{GenericClient, SimpleQueryMessage};

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CoreError {
    #[error(transparent)]
    Ident(#[from] IdentError),
    #[error(transparent)]
    Normalise(#[from] NormaliseError),
    #[error("invalid table plan: {0}")]
    Plan(&'static str),
    /// Only the SQLSTATE is kept: server messages can quote row values.
    #[error("database error (SQLSTATE {0})")]
    Db(String),
    /// SQLSTATE 42501 (insufficient_privilege): either a plain missing SELECT
    /// grant, or a query that hit row-level security (with `row_security =
    /// off` PostgreSQL raises this instead of silently filtering rows).  Either
    /// way it must never become a PASS.  The message is fixed: it never carries
    /// a table name or a row value.
    #[error(
        "the Dmux login is not allowed to read this table: grant SELECT, and BYPASSRLS if the table uses row-level security"
    )]
    RowSecurity,
    #[error("not inside the Dmux read-only session: {0} is not set as required")]
    UnsafeSession(&'static str),
    #[error("malformed result from the database: {0}")]
    Malformed(&'static str),
    #[error("arithmetic overflow in {0}")]
    Overflow(&'static str),
}

impl From<tokio_postgres::Error> for CoreError {
    fn from(e: tokio_postgres::Error) -> Self {
        core_error_from_sqlstate(e.code().map(|c| c.code()))
    }
}

/// Classify a database error by its SQLSTATE alone (rule 7: the server message
/// may quote row values, so it is never kept).
///
/// 42501 (insufficient_privilege) is what PostgreSQL raises when a scan would
/// be affected by a row-level security policy while `row_security` is `off`.
/// It becomes a typed [`CoreError::RowSecurity`] so the engine can surface a
/// clear fault instead of a silent, filtered PASS.
fn core_error_from_sqlstate(code: Option<&str>) -> CoreError {
    match code {
        Some("42501") => CoreError::RowSecurity,
        Some(code) => CoreError::Db(code.to_string()),
        None => CoreError::Db("none".into()),
    }
}

/// The settings the canonical `SET LOCAL` block must have applied (T003 section 4).
const REQUIRED_SETTINGS: &[(&str, &str)] = &[
    ("transaction_read_only", "on"),
    ("transaction_isolation", "repeatable read"),
    ("search_path", "pg_catalog"),
    ("TimeZone", "UTC"),
    ("DateStyle", "ISO, YMD"),
    ("IntervalStyle", "iso_8601"),
    ("extra_float_digits", "0"),
    ("bytea_output", "hex"),
    ("xmloption", "document"),
];

/// Settings that must be set to something other than 0 (off) (P17).
const NONZERO_SETTINGS: &[&str] = &["statement_timeout", "lock_timeout"];

/// `row_security` only exists on PostgreSQL 9.5 and later (rule 3).  Below
/// that the setting is unknown and `current_setting('row_security')` raises
/// "unrecognized configuration parameter".
pub(crate) const fn row_security_required(version_num: i32) -> bool {
    version_num >= 90500
}

/// The settings that must be present for a given server version.  On 9.5+ the
/// canonical block also sets `row_security = off`, so a session that skipped it
/// (and would silently under-read an RLS-protected table) is refused.
fn required_settings(version_num: i32) -> Vec<(&'static str, &'static str)> {
    let mut settings = REQUIRED_SETTINGS.to_vec();
    if row_security_required(version_num) {
        settings.push(("row_security", "off"));
    }
    settings
}

/// Read `server_version_num` as an integer, so `check_session` can branch on it.
async fn read_version_num(client: &impl GenericClient) -> Result<i32, CoreError> {
    let rows = text_rows(client, "SELECT current_setting('server_version_num')").await?;
    let row = rows.first().ok_or(CoreError::Malformed("server version"))?;
    let text = row
        .first()
        .and_then(|v| v.as_deref())
        .ok_or(CoreError::Malformed("server version"))?;
    text.parse::<i32>()
        .map_err(|_| CoreError::Malformed("server version"))
}

/// Run one statement with the simple protocol and return every value as text.
pub(crate) async fn text_rows(
    client: &impl GenericClient,
    sql: &str,
) -> Result<Vec<Vec<Option<String>>>, CoreError> {
    let mut out = Vec::new();
    for msg in client.simple_query(sql).await? {
        if let SimpleQueryMessage::Row(row) = msg {
            out.push(
                (0..row.len())
                    .map(|i| row.get(i).map(str::to_owned))
                    .collect(),
            );
        }
    }
    Ok(out)
}

/// Refuse to scan unless the session is read-only, repeatable read and has
/// the canonical settings that the normalisation rules rely on.
pub async fn check_session(client: &impl GenericClient) -> Result<(), CoreError> {
    let version_num = read_version_num(client).await?;
    let required = required_settings(version_num);
    let names = required
        .iter()
        .map(|(name, _)| *name)
        .chain(NONZERO_SETTINGS.iter().copied());
    let cols: Vec<String> = names
        .map(|name| format!("current_setting('{name}')"))
        .collect();
    let rows = text_rows(client, &format!("SELECT {}", cols.join(", "))).await?;
    let row = rows.first().ok_or(CoreError::Malformed("settings check"))?;
    for (i, (name, want)) in required.iter().enumerate() {
        if row.get(i).and_then(|v| v.as_deref()) != Some(*want) {
            return Err(CoreError::UnsafeSession(name));
        }
    }
    for (i, name) in NONZERO_SETTINGS.iter().enumerate() {
        match row.get(required.len() + i).and_then(|v| v.as_deref()) {
            Some(v) if v != "0" => {}
            _ => return Err(CoreError::UnsafeSession(name)),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{core_error_from_sqlstate, required_settings, row_security_required, CoreError};

    #[test]
    fn row_security_is_required_only_on_95_plus() {
        assert!(!row_security_required(90426));
        assert!(!row_security_required(90499));
        assert!(row_security_required(90500));
        assert!(row_security_required(90624));
        assert!(row_security_required(170000));
    }

    #[test]
    fn required_settings_branch_on_version() {
        // 9.4: the base block, no row_security.
        let base = required_settings(90426);
        assert!(!base.iter().any(|(n, _)| *n == "row_security"));
        assert_eq!(base.len(), 9);
        // 9.5+: row_security = off is appended.
        let rls = required_settings(90500);
        assert_eq!(rls.len(), 10);
        assert_eq!(rls.last(), Some(&("row_security", "off")));
        // The base settings are unchanged by the branch.
        assert_eq!(&rls[..base.len()], &base[..]);
    }

    #[test]
    fn sqlstate_42501_maps_to_row_security_not_db() {
        assert_eq!(
            core_error_from_sqlstate(Some("42501")),
            CoreError::RowSecurity
        );
        // The message is fixed and never carries a table name or row value.
        // 42501 is also a plain missing SELECT, so it names both causes.
        let text = CoreError::RowSecurity.to_string();
        assert_eq!(
            text,
            "the Dmux login is not allowed to read this table: grant SELECT, and BYPASSRLS if the table uses row-level security"
        );
        assert!(text.contains("grant SELECT"));
        assert!(text.contains("BYPASSRLS"));
        assert!(!text.contains("42"));
    }

    #[test]
    fn other_sqlstates_stay_as_plain_db_errors() {
        assert_eq!(
            core_error_from_sqlstate(Some("42P01")),
            CoreError::Db("42P01".into())
        );
        assert_eq!(
            core_error_from_sqlstate(Some("0A000")),
            CoreError::Db("0A000".into())
        );
        assert_eq!(core_error_from_sqlstate(None), CoreError::Db("none".into()));
    }
}
