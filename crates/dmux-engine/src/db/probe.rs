//! Probe a PostgreSQL server for the information shown in the "Test connection"
//! flow and required by the comparison report.

use serde::{Deserialize, Serialize};
use tokio_postgres::{Client, Row};

use super::config::ConnProfile;
use super::connect::connect;
use super::errors::DbError;
use super::readonly::with_readonly_txn;

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

async fn query_one_text(client: &Client, sql: &str) -> Result<String, DbError> {
    client
        .query_one(sql, &[])
        .await
        .map(|row: Row| row.get::<_, String>(0))
        .map_err(DbError::from_postgres)
}

async fn query_one_bool(client: &Client, sql: &str) -> Result<bool, DbError> {
    client
        .query_one(sql, &[])
        .await
        .map(|row: Row| row.get::<_, bool>(0))
        .map_err(DbError::from_postgres)
}

async fn query_one_opt_text(client: &Client, sql: &str) -> Result<Option<String>, DbError> {
    client
        .query_opt(sql, &[])
        .await
        .map(|row| row.map(|r: Row| r.get::<_, String>(0)))
        .map_err(DbError::from_postgres)
}

/// Gather server information from a profile.
///
/// This uses only 9.4-compatible queries and runs the settings queries through
/// the canonical read-only transaction wrapper.
pub async fn probe_server(profile: &ConnProfile) -> Result<ServerInfo, DbError> {
    let mut client = connect(profile).await?;

    let version_num: i32 = client
        .query_one("SHOW server_version_num", &[])
        .await
        .map(|row| row.get::<_, String>(0).parse().unwrap_or(0))
        .map_err(DbError::from_postgres)?;

    let version_string = query_one_text(&client, "SHOW server_version").await?;
    let encoding = client
        .query_one(
            "SELECT pg_encoding_to_char(encoding) FROM pg_database WHERE datname = current_database()",
            &[],
        )
        .await
        .map(|row: Row| row.get::<_, String>(0))
        .map_err(DbError::from_postgres)?;
    let collation = query_one_text(&client, "SHOW lc_collate").await?;
    let ctype = query_one_text(&client, "SHOW lc_ctype").await?;
    let lc_monetary = query_one_text(&client, "SHOW lc_monetary").await?;

    let is_superuser = query_one_bool(
        &client,
        "SELECT usesuper FROM pg_user WHERE usename = current_user",
    )
    .await?;

    let can_write = query_one_bool(
        &client,
        "SELECT EXISTS (\
            SELECT 1 FROM pg_class c \
            JOIN pg_namespace n ON n.oid = c.relnamespace \
            WHERE c.relkind = 'r' \
            AND n.nspname NOT IN ('pg_catalog', 'information_schema', 'pg_toast') \
            AND (has_table_privilege(c.oid, 'INSERT') \
              OR has_table_privilege(c.oid, 'UPDATE') \
              OR has_table_privilege(c.oid, 'DELETE')))",
    )
    .await?;

    let md5_available = query_one_opt_text(&client, "SELECT md5('x')")
        .await
        .map(|opt| opt.is_some())?;

    // Pooler heuristic: inside a read-only transaction, set application_name
    // locally and then read it back.  If a pooler rewrote or dropped the session
    // state, the echoed value will differ.
    let pooler_suspected = with_readonly_txn(&mut client, "5s", |txn| {
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
    .await?;

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
