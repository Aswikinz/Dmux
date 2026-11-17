//! Probe a PostgreSQL server for the information shown in the "Test connection"
//! flow and required by the comparison report.
//!
//! Every probe query runs inside the canonical read-only transaction wrapper.

use serde::{Deserialize, Serialize};
use tokio_postgres::Transaction;

use super::config::ConnProfile;
use super::connect::ReadOnlySession;
use super::errors::DbError;

/// Information reported by `dmux probe` and the desktop app's test connection.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ServerInfo {
    pub version_num: i32,
    pub version_string: String,
    pub encoding: String,
    pub collation: String,
    pub ctype: String,
    pub lc_monetary: String,
    pub is_superuser: bool,
    pub can_write: bool,
    pub pooler_suspected: bool,
    pub md5_available: bool,
    pub sql_ascii: bool,
}

async fn read_text(txn: &Transaction<'_>, sql: &str) -> Result<String, DbError> {
    let row = txn
        .query_one(sql, &[])
        .await
        .map_err(DbError::from_postgres)?;
    Ok(row.get(0))
}

async fn read_version_num(txn: &Transaction<'_>) -> Result<i32, DbError> {
    let text: String = read_text(txn, "SHOW server_version_num").await?;
    text.parse::<i32>().map_err(|_| DbError::Config {
        message: format!("server_version_num is not an integer: {text}"),
    })
}

/// Encoding, collation and ctype from `pg_database` (not `SHOW lc_collate` /
/// `SHOW lc_ctype`, which error on PostgreSQL 16+).
async fn read_database_props(txn: &Transaction<'_>) -> Result<(String, String, String), DbError> {
    let row = txn
        .query_one(
            "SELECT pg_catalog.pg_encoding_to_char(d.encoding), d.datcollate, d.datctype \
             FROM pg_catalog.pg_database d WHERE d.datname = pg_catalog.current_database()",
            &[],
        )
        .await
        .map_err(DbError::from_postgres)?;
    Ok((row.get(0), row.get(1), row.get(2)))
}

async fn read_can_write(txn: &Transaction<'_>) -> Result<bool, DbError> {
    let row = txn
        .query_one(
            "SELECT EXISTS ( \
                SELECT 1 \
                FROM pg_catalog.pg_class c \
                JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace \
                WHERE c.relkind IN ('r', 'p', 'f') \
                  AND n.nspname !~ '^(pg_catalog|information_schema|pg_toast|pg_temp_[0-9]+|pg_toast_temp_[0-9]+)$' \
                  AND ( \
                    pg_catalog.has_table_privilege(c.oid, 'INSERT') \
                    OR pg_catalog.has_table_privilege(c.oid, 'UPDATE') \
                    OR pg_catalog.has_table_privilege(c.oid, 'DELETE') \
                    OR pg_catalog.has_table_privilege(c.oid, 'TRUNCATE') \
                    OR pg_catalog.has_any_column_privilege(c.oid, 'INSERT') \
                    OR pg_catalog.has_any_column_privilege(c.oid, 'UPDATE') \
                  ) \
             )",
            &[],
        )
        .await
        .map_err(DbError::from_postgres)?;
    Ok(row.get(0))
}

/// `md5` availability is checked in its own transaction so a FIPS or removed
/// `md5` error (SQLSTATE 0A000 / function error) degrades to `false` instead of
/// aborting the probe.
async fn md5_available(session: &mut ReadOnlySession) -> bool {
    session
        .with_read_only("5s", |txn| {
            Box::pin(async move {
                match txn.query_opt("SELECT pg_catalog.md5('x')", &[]).await {
                    Ok(Some(_)) => Ok(true),
                    _ => Err(DbError::Postgres {
                        message: "md5 unavailable".to_string(),
                    }),
                }
            })
        })
        .await
        .unwrap_or(false)
}

/// Pooler heuristic: inside a read-only transaction, set application_name
/// locally and read it back.  If a pooler rewrote or dropped the session state,
/// the echoed value will differ.
async fn pooler_suspected(session: &mut ReadOnlySession) -> Result<bool, DbError> {
    session
        .with_read_only("5s", |txn| {
            Box::pin(async move {
                txn.execute("SET LOCAL application_name = 'dmux-pooler-check'", &[])
                    .await
                    .map_err(DbError::from_postgres)?;
                let echoed: String = txn
                    .query_one("SHOW application_name", &[])
                    .await
                    .map(|row| row.get(0))
                    .map_err(DbError::from_postgres)?;
                Ok(echoed != "dmux-pooler-check")
            })
        })
        .await
}

/// Gather server information from a profile.
///
/// This uses only 9.4-compatible queries and runs every query through the
/// canonical read-only transaction wrapper.
pub async fn probe_server(
    profile: &ConnProfile,
    allow_superuser: bool,
) -> Result<ServerInfo, DbError> {
    let mut session = ReadOnlySession::open(profile, allow_superuser).await?;
    let is_superuser = session.is_superuser();

    let (version_num, version_string, encoding, collation, ctype, lc_monetary, can_write) = session
        .with_read_only("5s", |txn| {
            Box::pin(async move {
                let version_num = read_version_num(txn).await?;
                let version_string = read_text(txn, "SHOW server_version").await?;
                let (encoding, collation, ctype) = read_database_props(txn).await?;
                let lc_monetary = read_text(txn, "SHOW lc_monetary").await?;
                let can_write = read_can_write(txn).await?;
                Ok((
                    version_num,
                    version_string,
                    encoding,
                    collation,
                    ctype,
                    lc_monetary,
                    can_write,
                ))
            })
        })
        .await?;

    let md5_available = md5_available(&mut session).await;
    let pooler_suspected = pooler_suspected(&mut session).await?;
    let sql_ascii = encoding.eq_ignore_ascii_case("SQL_ASCII");

    Ok(ServerInfo {
        version_num,
        version_string,
        encoding,
        collation,
        ctype,
        lc_monetary,
        is_superuser,
        can_write,
        pooler_suspected,
        md5_available,
        sql_ascii,
    })
}

#[cfg(test)]
mod tests {
    use super::ServerInfo;

    #[test]
    fn server_info_serializes_to_snake_case() {
        let info = ServerInfo {
            version_num: 170000,
            version_string: "17.0".to_string(),
            encoding: "UTF8".to_string(),
            collation: "en_US.utf8".to_string(),
            ctype: "en_US.utf8".to_string(),
            lc_monetary: "en_US.utf8".to_string(),
            is_superuser: false,
            can_write: false,
            pooler_suspected: false,
            md5_available: true,
            sql_ascii: false,
        };
        let json = serde_json::to_string(&info).unwrap();
        assert!(json.contains("version_num"));
        assert!(json.contains("version_string"));
        assert!(json.contains("lc_monetary"));
    }
}
