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
    /// Whether the login has `BYPASSRLS` (PostgreSQL 9.5+; always `false` on 9.4).
    pub rolbypassrls: bool,
    /// `schema.table` of every relation with row-level security enabled
    /// (PostgreSQL 9.5+; always empty on 9.4).  Dmux refuses to read these
    /// under a policy rather than silently under-read them.
    pub rls_tables: Vec<String>,
    /// Fixed, plain-language reasons why the login can write (no object names,
    /// no row data).  Empty when the login is read-only in every way checked.
    pub can_write_reasons: Vec<String>,
}

/// Postgres versions at which optional probe checks become available.
pub const RLS_MIN_VERSION: i32 = 90500;
pub const WRITE_SERVER_FILES_MIN_VERSION: i32 = 110000;
pub const WRITE_ALL_DATA_MIN_VERSION: i32 = 140000;
pub const LARGE_OBJECT_PRIV_MIN_VERSION: i32 = 180000;

/// `rolbypassrls` from the public `pg_roles` view (9.5+; the column does not
/// exist on 9.4, so callers branch on [`RLS_MIN_VERSION`]).
const RLS_BYPASS_SQL: &str =
    "SELECT rolbypassrls FROM pg_catalog.pg_roles WHERE rolname = current_user";

/// Relations with `relrowsecurity` (9.5+).  System and temporary schemas are
/// excluded so only real, user-visible tables are listed.
const RLS_TABLES_SQL: &str = "SELECT n.nspname || '.' || c.relname \
     FROM pg_catalog.pg_class c \
     JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace \
     WHERE c.relrowsecurity \
       AND n.nspname !~ '^(pg_catalog|information_schema|pg_toast|pg_temp_[0-9]+|pg_toast_temp_[0-9]+)$' \
     ORDER BY n.nspname, c.relname";

/// CREATE privilege on any non-system schema.
const CAN_CREATE_SCHEMA_SQL: &str = "SELECT EXISTS ( \
     SELECT 1 FROM pg_catalog.pg_namespace n \
     WHERE n.nspname !~ '^(pg_catalog|information_schema|pg_toast|pg_temp_[0-9]+|pg_toast_temp_[0-9]+)$' \
       AND pg_catalog.has_schema_privilege(n.oid, 'CREATE') \
     )";

/// UPDATE privilege on any non-system sequence.  `OFFSET 0` is a planner
/// barrier: without it PostgreSQL may evaluate `has_sequence_privilege` on a
/// non-sequence relation (e.g. `pg_statistic`) and error "not a sequence".
const CAN_UPDATE_SEQUENCE_SQL: &str = "SELECT EXISTS ( \
     SELECT 1 FROM ( \
       SELECT c.oid FROM pg_catalog.pg_class c \
       JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace \
       WHERE c.relkind = 'S' \
         AND n.nspname !~ '^(pg_catalog|information_schema|pg_toast|pg_temp_[0-9]+|pg_toast_temp_[0-9]+)$' \
       OFFSET 0 \
     ) s \
     WHERE pg_catalog.has_sequence_privilege(s.oid, 'UPDATE') \
     )";

/// Write (UPDATE) privilege on any existing large object.  Only cheaply
/// detectable on PostgreSQL 18+, where `has_largeobject_privilege` exists.
const LARGE_OBJECT_WRITE_SQL: &str = "SELECT EXISTS ( \
     SELECT 1 FROM pg_catalog.pg_largeobject_metadata l \
     WHERE pg_catalog.has_largeobject_privilege(l.oid, 'UPDATE') \
     )";

/// The predefined write roles to test for membership, keyed by the minimum
/// server version that defines the role.
const WRITE_ROLE_CHECKS: &[(i32, &str, &str)] = &[
    (
        WRITE_SERVER_FILES_MIN_VERSION,
        "pg_write_server_files",
        "member of pg_write_server_files",
    ),
    (
        WRITE_SERVER_FILES_MIN_VERSION,
        "pg_execute_server_program",
        "member of pg_execute_server_program",
    ),
    (
        WRITE_ALL_DATA_MIN_VERSION,
        "pg_write_all_data",
        "member of pg_write_all_data",
    ),
];

/// The predefined write roles to probe at `version_num`, as `(role, reason)`.
fn write_role_checks(version_num: i32) -> Vec<(&'static str, &'static str)> {
    WRITE_ROLE_CHECKS
        .iter()
        .filter(|(min, _, _)| version_num >= *min)
        .map(|(_, role, reason)| (*role, *reason))
        .collect()
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

async fn read_bool(txn: &Transaction<'_>, sql: &str) -> Result<bool, DbError> {
    let row = txn
        .query_one(sql, &[])
        .await
        .map_err(DbError::from_postgres)?;
    Ok(row.get(0))
}

/// Whether the login is a member (directly or indirectly) of a predefined role.
/// `role` is always a compile-time constant from [`WRITE_ROLE_CHECKS`]; it is
/// bound as a parameter, never interpolated into the statement.
async fn read_role_membership(txn: &Transaction<'_>, role: &str) -> Result<bool, DbError> {
    let row = txn
        .query_one("SELECT pg_catalog.pg_has_role($1, 'MEMBER')", &[&role])
        .await
        .map_err(DbError::from_postgres)?;
    Ok(row.get(0))
}

/// Whether the login has `rolbypassrls` (9.5+; `false` on 9.4 where the column
/// and the RLS feature do not exist).
async fn read_rolbypassrls(txn: &Transaction<'_>, version_num: i32) -> Result<bool, DbError> {
    if version_num < RLS_MIN_VERSION {
        return Ok(false);
    }
    read_bool(txn, RLS_BYPASS_SQL).await
}

/// The `schema.table` names of relations with row-level security (9.5+; empty
/// on 9.4).  Ordered by schema then name so the list is deterministic.
async fn read_rls_tables(txn: &Transaction<'_>, version_num: i32) -> Result<Vec<String>, DbError> {
    if version_num < RLS_MIN_VERSION {
        return Ok(Vec::new());
    }
    let rows = txn
        .query(RLS_TABLES_SQL, &[])
        .await
        .map_err(DbError::from_postgres)?;
    Ok(rows.into_iter().map(|row| row.get(0)).collect())
}

/// Every write capability the login holds, as fixed plain-language reasons.
/// The order is stable and every reason is a compile-time constant (no object
/// names, no row data), so it is safe to surface in the report.
async fn read_can_write_reasons(
    txn: &Transaction<'_>,
    version_num: i32,
) -> Result<Vec<String>, DbError> {
    let mut reasons = Vec::new();

    if read_can_write(txn).await? {
        reasons.push("can INSERT/UPDATE/DELETE on a table".to_string());
    }
    for (role, reason) in write_role_checks(version_num) {
        if read_role_membership(txn, role).await? {
            reasons.push(reason.to_string());
        }
    }
    if read_bool(txn, CAN_CREATE_SCHEMA_SQL).await? {
        reasons.push("can CREATE in a non-system schema".to_string());
    }
    if read_bool(txn, CAN_UPDATE_SEQUENCE_SQL).await? {
        reasons.push("can UPDATE a sequence".to_string());
    }
    if version_num >= LARGE_OBJECT_PRIV_MIN_VERSION
        && read_bool(txn, LARGE_OBJECT_WRITE_SQL).await?
    {
        reasons.push("can write a large object".to_string());
    }

    Ok(reasons)
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

    let (
        version_num,
        version_string,
        encoding,
        collation,
        ctype,
        lc_monetary,
        can_write_reasons,
        rolbypassrls,
        rls_tables,
    ) = session
        .with_read_only("5s", |txn| {
            Box::pin(async move {
                let version_num = read_version_num(txn).await?;
                let version_string = read_text(txn, "SHOW server_version").await?;
                let (encoding, collation, ctype) = read_database_props(txn).await?;
                let lc_monetary = read_text(txn, "SHOW lc_monetary").await?;
                let can_write_reasons = read_can_write_reasons(txn, version_num).await?;
                let rolbypassrls = read_rolbypassrls(txn, version_num).await?;
                let rls_tables = read_rls_tables(txn, version_num).await?;
                Ok((
                    version_num,
                    version_string,
                    encoding,
                    collation,
                    ctype,
                    lc_monetary,
                    can_write_reasons,
                    rolbypassrls,
                    rls_tables,
                ))
            })
        })
        .await?;

    let md5_available = md5_available(&mut session).await;
    let pooler_suspected = pooler_suspected(&mut session).await?;
    let sql_ascii = encoding.eq_ignore_ascii_case("SQL_ASCII");
    let can_write = !can_write_reasons.is_empty();

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
        rolbypassrls,
        rls_tables,
        can_write_reasons,
    })
}

#[cfg(test)]
mod tests {
    use super::{
        write_role_checks, ServerInfo, RLS_MIN_VERSION, WRITE_ALL_DATA_MIN_VERSION,
        WRITE_SERVER_FILES_MIN_VERSION,
    };

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
            rolbypassrls: false,
            rls_tables: vec!["public.accounts".to_string()],
            can_write_reasons: vec!["can CREATE in a non-system schema".to_string()],
        };
        let json = serde_json::to_string(&info).unwrap();
        assert!(json.contains("version_num"));
        assert!(json.contains("version_string"));
        assert!(json.contains("lc_monetary"));
        assert!(json.contains("rolbypassrls"));
        assert!(json.contains("rls_tables"));
        assert!(json.contains("can_write_reasons"));
    }

    #[test]
    fn write_role_checks_are_version_branched() {
        // 9.4/9.6/10: no predefined write roles.
        assert!(write_role_checks(RLS_MIN_VERSION - 1).is_empty());
        assert!(write_role_checks(WRITE_SERVER_FILES_MIN_VERSION - 1).is_empty());
        // 11+: pg_write_server_files and pg_execute_server_program.
        let v11 = write_role_checks(WRITE_SERVER_FILES_MIN_VERSION);
        assert_eq!(v11.len(), 2);
        assert!(v11.iter().any(|(r, _)| *r == "pg_write_server_files"));
        assert!(v11.iter().any(|(r, _)| *r == "pg_execute_server_program"));
        // 14+: adds pg_write_all_data.
        let v14 = write_role_checks(WRITE_ALL_DATA_MIN_VERSION);
        assert_eq!(v14.len(), 3);
        assert!(v14.iter().any(|(r, _)| *r == "pg_write_all_data"));
    }
}
