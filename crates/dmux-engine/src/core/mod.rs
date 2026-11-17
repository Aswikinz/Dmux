//! The comparison core: normalise, fingerprint, compare and drill down
//! (PLAN.md section 3). Only arjun-core edits this module.
//!
//! Every function that talks to a database takes a client that is already
//! inside the read-only transaction opened by the `db` wrapper. The core
//! never opens, commits or ends a transaction itself; it checks the session
//! settings before each scan and refuses to run in any other session.

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
    #[error("not inside the Dmux read-only session: {0} is not set as required")]
    UnsafeSession(&'static str),
    #[error("malformed result from the database: {0}")]
    Malformed(&'static str),
    #[error("arithmetic overflow in {0}")]
    Overflow(&'static str),
}

impl From<tokio_postgres::Error> for CoreError {
    fn from(e: tokio_postgres::Error) -> Self {
        CoreError::Db(
            e.code()
                .map(|c| c.code().to_string())
                .unwrap_or_else(|| "none".into()),
        )
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
    let names = REQUIRED_SETTINGS
        .iter()
        .map(|(name, _)| *name)
        .chain(NONZERO_SETTINGS.iter().copied());
    let cols: Vec<String> = names
        .map(|name| format!("current_setting('{name}')"))
        .collect();
    let rows = text_rows(client, &format!("SELECT {}", cols.join(", "))).await?;
    let row = rows.first().ok_or(CoreError::Malformed("settings check"))?;
    for (i, (name, want)) in REQUIRED_SETTINGS.iter().enumerate() {
        if row.get(i).and_then(|v| v.as_deref()) != Some(*want) {
            return Err(CoreError::UnsafeSession(name));
        }
    }
    for (i, name) in NONZERO_SETTINGS.iter().enumerate() {
        match row
            .get(REQUIRED_SETTINGS.len() + i)
            .and_then(|v| v.as_deref())
        {
            Some(v) if v != "0" => {}
            _ => return Err(CoreError::UnsafeSession(name)),
        }
    }
    Ok(())
}
