//! Fixed catalog queries used by [`super::collect_inventory`].
//!
//! Every statement is a literal string with `$1` bound parameters only.  The
//! one place an identifier is built at runtime is the per-sequence `last_value`
//! read on PostgreSQL < 10, and that goes through `crate::ident::quote_qualified`.

use super::{Extension, Function, Index, Sequence, Trigger, View};
use crate::inventory::{
    opt_string, req_bool, req_i64, req_string, Column, Constraint, ConstraintKind, InventoryError,
};
use sha2::{Digest, Sha256};
use tokio_postgres::GenericClient;

/// Filter fragment that excludes system and temporary schemas.
pub const EXCLUDED_SCHEMAS_FILTER_SQL: &str =
    "!~ '^(pg_catalog|information_schema|pg_toast|pg_temp_[0-9]+|pg_toast_temp_[0-9]+)$'";

pub(crate) async fn server_version<T: GenericClient>(client: &T) -> Result<String, InventoryError> {
    let row = client.query_one("SHOW server_version", &[]).await?;
    req_string(&row, 0)
}

pub(crate) async fn server_version_num<T: GenericClient>(
    client: &T,
) -> Result<i32, InventoryError> {
    let row = client.query_one("SHOW server_version_num", &[]).await?;
    let text: String = req_string(&row, 0)?;
    text.parse::<i32>()
        .map_err(|_| InventoryError::VersionParse(text.clone()))
}

pub(crate) async fn database_properties<T: GenericClient>(
    client: &T,
) -> Result<DatabaseProperties, InventoryError> {
    let row = client
        .query_one(
            "SELECT pg_catalog.pg_encoding_to_char(d.encoding) AS encoding, \
             d.datcollate AS collation, d.datctype AS ctype \
             FROM pg_catalog.pg_database d WHERE d.datname = pg_catalog.current_database()",
            &[],
        )
        .await?;

    let lc_monetary = client
        .query_one("SHOW lc_monetary", &[])
        .await
        .map(|r| r.try_get::<_, String>(0).unwrap_or_default())
        .unwrap_or_default();

    Ok(DatabaseProperties {
        encoding: req_string(&row, 0)?,
        collation: req_string(&row, 1)?,
        ctype: req_string(&row, 2)?,
        lc_monetary,
    })
}

pub(crate) async fn list_schemas<T: GenericClient>(
    client: &T,
) -> Result<Vec<ListSchema>, InventoryError> {
    let rows = client
        .query(
            "SELECT n.nspname AS schema_name, pg_catalog.pg_get_userbyid(n.nspowner) AS owner \
             FROM pg_catalog.pg_namespace n \
             WHERE n.nspname !~ '^(pg_catalog|information_schema|pg_toast|pg_temp_[0-9]+|pg_toast_temp_[0-9]+)$' \
             ORDER BY n.nspname",
            &[],
        )
        .await?;

    rows.into_iter()
        .map(|row| {
            Ok(ListSchema {
                name: req_string(&row, 0)?,
                owner: req_string(&row, 1)?,
            })
        })
        .collect()
}

pub(crate) async fn list_tables<T: GenericClient>(
    client: &T,
    version_num: i32,
) -> Result<Vec<ListTable>, InventoryError> {
    // Columns are identical across the two branches; only the has_oids source
    // differs (relhasoids was removed in PostgreSQL 12).  relkind includes
    // foreign tables ('f') so they are compared, not silently ignored.
    let sql = if version_num >= 120000 {
        "SELECT \
             n.nspname AS schema_name, \
             c.relname AS table_name, \
             pg_catalog.pg_get_userbyid(c.relowner) AS owner, \
             false AS has_oids, \
             (c.relkind = 'p' OR EXISTS (SELECT 1 FROM pg_catalog.pg_inherits i WHERE i.inhparent = c.oid)) AS is_partitioned, \
             COALESCE(s.n_live_tup, 0) AS n_live_tup, \
             GREATEST(s.last_analyze, s.last_autoanalyze)::text AS last_analyze, \
             pg_catalog.pg_total_relation_size(c.oid) AS size_bytes \
         FROM pg_catalog.pg_class c \
         JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace \
         LEFT JOIN pg_catalog.pg_stat_user_tables s ON s.relid = c.oid \
         WHERE c.relkind IN ('r', 'p', 'f') \
           AND n.nspname !~ '^(pg_catalog|information_schema|pg_toast|pg_temp_[0-9]+|pg_toast_temp_[0-9]+)$' \
         ORDER BY n.nspname, c.relname"
    } else {
        "SELECT \
             n.nspname AS schema_name, \
             c.relname AS table_name, \
             pg_catalog.pg_get_userbyid(c.relowner) AS owner, \
             c.relhasoids AS has_oids, \
             (c.relkind = 'p' OR EXISTS (SELECT 1 FROM pg_catalog.pg_inherits i WHERE i.inhparent = c.oid)) AS is_partitioned, \
             COALESCE(s.n_live_tup, 0) AS n_live_tup, \
             GREATEST(s.last_analyze, s.last_autoanalyze)::text AS last_analyze, \
             pg_catalog.pg_total_relation_size(c.oid) AS size_bytes \
         FROM pg_catalog.pg_class c \
         JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace \
         LEFT JOIN pg_catalog.pg_stat_user_tables s ON s.relid = c.oid \
         WHERE c.relkind IN ('r', 'p', 'f') \
           AND n.nspname !~ '^(pg_catalog|information_schema|pg_toast|pg_temp_[0-9]+|pg_toast_temp_[0-9]+)$' \
         ORDER BY n.nspname, c.relname"
    };
    let rows = client.query(sql, &[]).await?;

    rows.into_iter()
        .map(|row| {
            let last_analyze = opt_string(&row, 6)?;
            let n_live_tup = req_i64(&row, 5)?;
            let stats_missing = last_analyze.is_none() && n_live_tup > 0;
            Ok(ListTable {
                schema_name: req_string(&row, 0)?,
                name: req_string(&row, 1)?,
                owner: req_string(&row, 2)?,
                has_oids: req_bool(&row, 3)?,
                is_partitioned: req_bool(&row, 4)?,
                n_live_tup,
                last_analyze,
                size_bytes: req_i64(&row, 7)?,
                stats_missing,
            })
        })
        .collect()
}

pub(crate) async fn list_columns<T: GenericClient>(
    client: &T,
) -> Result<Vec<ColumnInTable>, InventoryError> {
    let rows = client
        .query(
            "SELECT \
                 n.nspname AS schema_name, \
                 c.relname AS table_name, \
                 a.attname AS column_name, \
                 pg_catalog.format_type(a.atttypid, a.atttypmod) AS type_name, \
                 NOT a.attnotnull AS nullable, \
                 pg_catalog.pg_get_expr(d.adbin, d.adrelid) AS default_expr \
             FROM pg_catalog.pg_attribute a \
             JOIN pg_catalog.pg_class c ON c.oid = a.attrelid \
             JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace \
             LEFT JOIN pg_catalog.pg_attrdef d ON d.adrelid = a.attrelid AND d.adnum = a.attnum \
             WHERE c.relkind IN ('r', 'p', 'f') \
               AND n.nspname !~ '^(pg_catalog|information_schema|pg_toast|pg_temp_[0-9]+|pg_toast_temp_[0-9]+)$' \
               AND a.attnum > 0 \
               AND NOT a.attisdropped \
             ORDER BY n.nspname, c.relname, a.attname",
            &[],
        )
        .await?;

    rows.into_iter()
        .map(|row| {
            let type_name = req_string(&row, 3)?;
            let length = parse_length(&type_name);
            Ok(ColumnInTable {
                schema_name: req_string(&row, 0)?,
                table_name: req_string(&row, 1)?,
                column: Column {
                    name: req_string(&row, 2)?,
                    type_name,
                    length,
                    nullable: req_bool(&row, 4)?,
                    default: opt_string(&row, 5)?,
                },
            })
        })
        .collect()
}

pub(crate) async fn list_constraints<T: GenericClient>(
    client: &T,
) -> Result<Vec<ConstraintInTable>, InventoryError> {
    let rows = client
        .query(
            "SELECT \
                 n.nspname AS schema_name, \
                 c.relname AS table_name, \
                 con.conname AS constraint_name, \
                 con.contype::text AS type, \
                 con.convalidated AS validated, \
                 pg_catalog.pg_get_constraintdef(con.oid, true) AS definition \
             FROM pg_catalog.pg_constraint con \
             JOIN pg_catalog.pg_class c ON c.oid = con.conrelid \
             JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace \
             WHERE con.contype <> 'n' \
               AND n.nspname !~ '^(pg_catalog|information_schema|pg_toast|pg_temp_[0-9]+|pg_toast_temp_[0-9]+)$' \
             ORDER BY n.nspname, c.relname, con.conname",
            &[],
        )
        .await?;

    rows.into_iter()
        .map(|row| {
            let type_char = req_string(&row, 3)?;
            let kind = match type_char.as_str() {
                "p" => ConstraintKind::PrimaryKey,
                "u" => ConstraintKind::Unique,
                "f" => ConstraintKind::ForeignKey,
                "c" => ConstraintKind::Check,
                _ => ConstraintKind::Other,
            };
            Ok(ConstraintInTable {
                schema_name: req_string(&row, 0)?,
                table_name: req_string(&row, 1)?,
                constraint: Constraint {
                    name: req_string(&row, 2)?,
                    kind,
                    validated: req_bool(&row, 4)?,
                    definition: req_string(&row, 5)?,
                },
            })
        })
        .collect()
}

pub(crate) async fn list_indexes<T: GenericClient>(
    client: &T,
) -> Result<Vec<IndexInTable>, InventoryError> {
    let rows = client
        .query(
            "SELECT \
                 n.nspname AS schema_name, \
                 c.relname AS table_name, \
                 i.relname AS index_name, \
                 ix.indisvalid AS is_valid, \
                 pg_catalog.pg_get_indexdef(i.oid, 0, true) AS definition \
             FROM pg_catalog.pg_index ix \
             JOIN pg_catalog.pg_class c ON c.oid = ix.indrelid \
             JOIN pg_catalog.pg_class i ON i.oid = ix.indexrelid \
             JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace \
             WHERE n.nspname !~ '^(pg_catalog|information_schema|pg_toast|pg_temp_[0-9]+|pg_toast_temp_[0-9]+)$' \
             ORDER BY n.nspname, c.relname, i.relname",
            &[],
        )
        .await?;

    rows.into_iter()
        .map(|row| {
            Ok(IndexInTable {
                schema_name: req_string(&row, 0)?,
                table_name: req_string(&row, 1)?,
                index: Index {
                    name: req_string(&row, 2)?,
                    valid: req_bool(&row, 3)?,
                    definition: req_string(&row, 4)?,
                },
            })
        })
        .collect()
}

pub(crate) async fn list_sequences<T: GenericClient>(
    client: &mut T,
    version_num: i32,
) -> Result<Vec<SequenceInSchema>, InventoryError> {
    if version_num < 100000 {
        // PostgreSQL < 10 has no pg_sequence catalog.  List names and the SELECT
        // privilege, then read last_value + is_called per sequence (through the
        // quoting helper).  Sequences are always bigint before the `AS` clause.
        let rows = client
            .query(
                "SELECT n.nspname AS schema_name, c.relname AS sequence_name, \
                        pg_catalog.has_sequence_privilege(c.oid, 'SELECT') AS readable \
                 FROM pg_catalog.pg_class c \
                 JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace \
                 WHERE c.relkind = 'S' \
                   AND n.nspname !~ '^(pg_catalog|information_schema|pg_toast|pg_temp_[0-9]+|pg_toast_temp_[0-9]+)$' \
                 ORDER BY n.nspname, c.relname",
                &[],
            )
            .await?;

        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            let schema_name = req_string(&row, 0)?;
            let name = req_string(&row, 1)?;
            let readable = req_bool(&row, 2)?;
            let qualified = crate::ident::quote_qualified(&schema_name, &name)?;

            let (last_value, is_called, unreadable) = if !readable {
                (None, false, true)
            } else {
                match read_sequence_state(client, &qualified).await? {
                    SeqState::Called(value) => (Some(value), true, false),
                    SeqState::NotCalled => (None, false, false),
                    SeqState::Unreadable => (None, false, true),
                }
            };

            out.push(SequenceInSchema {
                schema_name,
                sequence: Sequence {
                    name,
                    data_type: "bigint".to_string(),
                    start_value: None,
                    min_value: None,
                    max_value: None,
                    increment: None,
                    last_value,
                    cycle: None,
                    is_called,
                    unreadable,
                },
            });
        }
        return Ok(out);
    }

    // PostgreSQL 10+: pg_sequence has seqrelid for the privilege check and
    // pg_sequence_last_value for the current value (NULL when never called).
    //
    // The last-value read is wrapped in CASE WHEN has_sequence_privilege(...)
    // so a low-privilege login gets an unreadable (Warning) sequence rather
    // than an aborted inventory (T022 item 9).  Both privilege checks use the
    // same 'SELECT,USAGE' rights the read needs.
    let rows = client
        .query(
            "SELECT n.nspname AS schema_name, c.relname AS sequence_name, \
                    pg_catalog.format_type(s.seqtypid, NULL) AS data_type, \
                    s.seqstart AS start_value, s.seqmin AS min_value, \
                    s.seqmax AS max_value, s.seqincrement AS increment, \
                    s.seqcycle AS cycle, \
                    CASE WHEN pg_catalog.has_sequence_privilege(s.seqrelid, 'SELECT,USAGE') \
                         THEN pg_catalog.pg_sequence_last_value(s.seqrelid) \
                         ELSE NULL END AS last_value, \
                    pg_catalog.has_sequence_privilege(s.seqrelid, 'SELECT,USAGE') AS readable \
             FROM pg_catalog.pg_sequence s \
             JOIN pg_catalog.pg_class c ON c.oid = s.seqrelid \
             JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace \
             WHERE n.nspname !~ '^(pg_catalog|information_schema|pg_toast|pg_temp_[0-9]+|pg_toast_temp_[0-9]+)$' \
             ORDER BY n.nspname, c.relname",
            &[],
        )
        .await?;

    rows.into_iter()
        .map(|row| {
            let readable = req_bool(&row, 9)?;
            let raw_last = row.try_get::<_, Option<i64>>(8)?;
            let last_value = if readable { raw_last } else { None };
            let is_called = readable && last_value.is_some();
            Ok(SequenceInSchema {
                schema_name: req_string(&row, 0)?,
                sequence: Sequence {
                    name: req_string(&row, 1)?,
                    data_type: opt_string(&row, 2)?.unwrap_or_default(),
                    start_value: row.try_get::<_, Option<i64>>(3)?,
                    min_value: row.try_get::<_, Option<i64>>(4)?,
                    max_value: row.try_get::<_, Option<i64>>(5)?,
                    increment: row.try_get::<_, Option<i64>>(6)?,
                    cycle: row.try_get::<_, Option<bool>>(7)?,
                    last_value,
                    is_called,
                    unreadable: !readable,
                },
            })
        })
        .collect()
}

/// The state of one sequence, read through the quoting helper.
enum SeqState {
    /// The sequence has been advanced and its last value is known.
    Called(i64),
    /// The sequence exists but has never been called.
    NotCalled,
    /// The state could not be read (privilege or an unexpected error).
    Unreadable,
}

/// Read one sequence's `last_value` + `is_called` inside a savepoint so an
/// error never aborts the surrounding read-only transaction.
async fn read_sequence_state<T: GenericClient>(
    client: &mut T,
    qualified: &str,
) -> Result<SeqState, InventoryError> {
    let sql = format!("SELECT last_value, is_called FROM {qualified}");
    let sp = client.transaction().await?;
    match sp.query_opt(&sql, &[]).await {
        Ok(Some(row)) => {
            let value: i64 = row.try_get(0)?;
            let called: bool = row.try_get(1)?;
            sp.commit().await?;
            Ok(if called {
                SeqState::Called(value)
            } else {
                SeqState::NotCalled
            })
        }
        _ => {
            let _ = sp.rollback().await;
            Ok(SeqState::Unreadable)
        }
    }
}

pub(crate) async fn list_views<T: GenericClient>(
    client: &T,
) -> Result<Vec<ListView>, InventoryError> {
    let rows = client
        .query(
            "SELECT \
                 n.nspname AS schema_name, \
                 c.relname AS view_name, \
                 c.relkind::text AS kind, \
                 pg_catalog.pg_get_viewdef(c.oid, true) AS definition \
             FROM pg_catalog.pg_class c \
             JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace \
             WHERE c.relkind IN ('v', 'm') \
               AND n.nspname !~ '^(pg_catalog|information_schema|pg_toast|pg_temp_[0-9]+|pg_toast_temp_[0-9]+)$' \
             ORDER BY n.nspname, c.relname",
            &[],
        )
        .await?;

    rows.into_iter()
        .map(|row| {
            Ok(ListView {
                schema_name: req_string(&row, 0)?,
                kind: req_string(&row, 2)?,
                view: View {
                    name: req_string(&row, 1)?,
                    definition: opt_string(&row, 3)?.unwrap_or_default(),
                },
            })
        })
        .collect()
}

pub(crate) async fn list_functions<T: GenericClient>(
    client: &T,
    version_num: i32,
) -> Result<Vec<ListFunction>, InventoryError> {
    // Aggregate functions have no pg_get_functiondef representation and would
    // error the query.  `proisagg` exists before PostgreSQL 11; `prokind` after.
    let sql = if version_num >= 110000 {
        "SELECT \
             n.nspname AS schema_name, \
             p.proname AS function_name, \
             pg_catalog.pg_get_function_arguments(p.oid) AS arguments, \
             pg_catalog.pg_get_function_identity_arguments(p.oid) AS identity_arguments, \
             l.lanname AS language, \
             pg_catalog.pg_get_functiondef(p.oid) AS definition \
         FROM pg_catalog.pg_proc p \
         JOIN pg_catalog.pg_namespace n ON n.oid = p.pronamespace \
         JOIN pg_catalog.pg_language l ON l.oid = p.prolang \
         WHERE n.nspname !~ '^(pg_catalog|information_schema|pg_toast|pg_temp_[0-9]+|pg_toast_temp_[0-9]+)$' \
           AND p.prokind <> 'a' \
         ORDER BY n.nspname, p.proname, pg_catalog.pg_get_function_identity_arguments(p.oid)"
    } else {
        "SELECT \
             n.nspname AS schema_name, \
             p.proname AS function_name, \
             pg_catalog.pg_get_function_arguments(p.oid) AS arguments, \
             pg_catalog.pg_get_function_identity_arguments(p.oid) AS identity_arguments, \
             l.lanname AS language, \
             pg_catalog.pg_get_functiondef(p.oid) AS definition \
         FROM pg_catalog.pg_proc p \
         JOIN pg_catalog.pg_namespace n ON n.oid = p.pronamespace \
         JOIN pg_catalog.pg_language l ON l.oid = p.prolang \
         WHERE n.nspname !~ '^(pg_catalog|information_schema|pg_toast|pg_temp_[0-9]+|pg_toast_temp_[0-9]+)$' \
           AND NOT p.proisagg \
         ORDER BY n.nspname, p.proname, pg_catalog.pg_get_function_identity_arguments(p.oid)"
    };

    let rows = client.query(sql, &[]).await?;

    rows.into_iter()
        .map(|row| {
            let name = req_string(&row, 1)?;
            let arguments = req_string(&row, 2)?;
            let identity_arguments = req_string(&row, 3)?;
            let language = req_string(&row, 4)?;
            let def = opt_string(&row, 5)?.unwrap_or_default();
            let hash = hex::encode(Sha256::digest(def.as_bytes()));
            Ok(ListFunction {
                schema_name: req_string(&row, 0)?,
                function: Function {
                    name,
                    arguments,
                    identity_arguments,
                    definition_hash: hash,
                    language,
                },
            })
        })
        .collect()
}

pub(crate) async fn list_triggers<T: GenericClient>(
    client: &T,
) -> Result<Vec<ListTrigger>, InventoryError> {
    let rows = client
        .query(
            "SELECT \
                 n.nspname AS schema_name, \
                 c.relname AS table_name, \
                 t.tgname AS trigger_name, \
                 t.tgenabled::text AS enabled, \
                 pg_catalog.pg_get_triggerdef(t.oid, true) AS definition \
             FROM pg_catalog.pg_trigger t \
             JOIN pg_catalog.pg_class c ON c.oid = t.tgrelid \
             JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace \
             WHERE NOT t.tgisinternal \
               AND n.nspname !~ '^(pg_catalog|information_schema|pg_toast|pg_temp_[0-9]+|pg_toast_temp_[0-9]+)$' \
             ORDER BY n.nspname, c.relname, t.tgname",
            &[],
        )
        .await?;

    rows.into_iter()
        .map(|row| {
            let def = opt_string(&row, 4)?.unwrap_or_default();
            // Parse a simple CREATE TRIGGER ... ON <table> to extract timing,
            // event and orientation where possible.
            let parts = parse_trigger_def(&def);
            Ok(ListTrigger {
                schema_name: req_string(&row, 0)?,
                trigger: Trigger {
                    name: req_string(&row, 2)?,
                    table_name: req_string(&row, 1)?,
                    enabled: req_string(&row, 3)?,
                    event_manipulation: parts.event_manipulation,
                    event_object: parts.event_object,
                    action_orientation: parts.action_orientation,
                    timing: parts.timing,
                    action_statement: parts.action_statement,
                },
            })
        })
        .collect()
}

pub(crate) async fn list_inheritances<T: GenericClient>(
    client: &T,
) -> Result<Vec<Inheritance>, InventoryError> {
    let rows = client
        .query(
            "SELECT \
                 pn.nspname AS parent_schema, \
                 pc.relname AS parent_name, \
                 cn.nspname AS child_schema, \
                 cc.relname AS child_name \
             FROM pg_catalog.pg_inherits i \
             JOIN pg_catalog.pg_class pc ON pc.oid = i.inhparent \
             JOIN pg_catalog.pg_namespace pn ON pn.oid = pc.relnamespace \
             JOIN pg_catalog.pg_class cc ON cc.oid = i.inhrelid \
             JOIN pg_catalog.pg_namespace cn ON cn.oid = cc.relnamespace \
             WHERE pn.nspname !~ '^(pg_catalog|information_schema|pg_toast|pg_temp_[0-9]+|pg_toast_temp_[0-9]+)$' \
             ORDER BY pn.nspname, pc.relname, cn.nspname, cc.relname",
            &[],
        )
        .await?;

    rows.into_iter()
        .map(|row| {
            Ok(Inheritance {
                parent_schema: req_string(&row, 0)?,
                parent_name: req_string(&row, 1)?,
                child_schema: req_string(&row, 2)?,
                child_name: req_string(&row, 3)?,
            })
        })
        .collect()
}

pub(crate) async fn list_extensions<T: GenericClient>(
    client: &T,
) -> Result<Vec<Extension>, InventoryError> {
    let rows = client
        .query(
            "SELECT e.extname AS name, n.nspname AS schema, e.extversion AS version \
             FROM pg_catalog.pg_extension e \
             JOIN pg_catalog.pg_namespace n ON n.oid = e.extnamespace \
             ORDER BY e.extname",
            &[],
        )
        .await?;

    rows.into_iter()
        .map(|row| {
            Ok(Extension {
                name: req_string(&row, 0)?,
                schema: req_string(&row, 1)?,
                version: req_string(&row, 2)?,
            })
        })
        .collect()
}

pub(crate) async fn large_object_count<T: GenericClient>(
    client: &T,
) -> Result<i64, InventoryError> {
    let row = client
        .query_one(
            "SELECT count(*) FROM pg_catalog.pg_largeobject_metadata",
            &[],
        )
        .await?;
    req_i64(&row, 0)
}

// --- internal types used only while assembling the inventory ---

pub(crate) struct DatabaseProperties {
    pub encoding: String,
    pub collation: String,
    pub ctype: String,
    pub lc_monetary: String,
}

pub(crate) struct ListSchema {
    pub name: String,
    pub owner: String,
}

pub(crate) struct ListTable {
    pub schema_name: String,
    pub name: String,
    pub owner: String,
    pub has_oids: bool,
    pub is_partitioned: bool,
    pub n_live_tup: i64,
    pub last_analyze: Option<String>,
    pub size_bytes: i64,
    pub stats_missing: bool,
}

pub(crate) struct ColumnInTable {
    pub schema_name: String,
    pub table_name: String,
    pub column: Column,
}

pub(crate) struct ConstraintInTable {
    pub schema_name: String,
    pub table_name: String,
    pub constraint: Constraint,
}

pub(crate) struct IndexInTable {
    pub schema_name: String,
    pub table_name: String,
    pub index: Index,
}

pub(crate) struct SequenceInSchema {
    pub schema_name: String,
    pub sequence: Sequence,
}

pub(crate) struct ListView {
    pub schema_name: String,
    pub kind: String,
    pub view: View,
}

pub(crate) struct ListFunction {
    pub schema_name: String,
    pub function: Function,
}

pub(crate) struct ListTrigger {
    pub schema_name: String,
    pub trigger: Trigger,
}

pub(crate) struct Inheritance {
    pub parent_schema: String,
    pub parent_name: String,
    pub child_schema: String,
    pub child_name: String,
}

struct TriggerParts {
    event_manipulation: String,
    event_object: String,
    action_orientation: String,
    timing: String,
    action_statement: String,
}

fn parse_trigger_def(def: &str) -> TriggerParts {
    let upper = def.to_uppercase();
    let mut timing = String::new();
    let mut action_statement = String::new();
    let mut action_orientation = String::new();
    let mut event_object = String::new();

    // Best-effort extraction; never fails, just leaves blanks on unexpected shapes.
    if let Some(pos) = upper.find(" CREATE TRIGGER ") {
        let after = &def[pos + 16..];
        if let Some(on_pos) = after.to_uppercase().find(" ON ") {
            event_object = after[on_pos + 4..]
                .split_whitespace()
                .next()
                .unwrap_or("")
                .to_string();
        }
    }

    if upper.contains("BEFORE") {
        timing = "BEFORE".to_string();
    } else if upper.contains("AFTER") {
        timing = "AFTER".to_string();
    } else if upper.contains("INSTEAD OF") {
        timing = "INSTEAD OF".to_string();
    }

    let events = ["INSERT", "UPDATE", "DELETE", "TRUNCATE"];
    let mut found = Vec::new();
    for ev in &events {
        if upper.contains(ev) {
            found.push(*ev);
        }
    }
    let event_manipulation = found.join(" OR ");

    if upper.contains("FOR EACH ROW") {
        action_orientation = "ROW".to_string();
    } else if upper.contains("FOR EACH STATEMENT") {
        action_orientation = "STATEMENT".to_string();
    }

    if let Some(exec_pos) = upper.find("EXECUTE ") {
        action_statement = def[exec_pos..].to_string();
    } else if let Some(fn_pos) = upper.find(" FUNCTION ") {
        action_statement = def[fn_pos..].to_string();
    }

    TriggerParts {
        event_manipulation,
        event_object,
        action_orientation,
        timing,
        action_statement,
    }
}

fn parse_length(type_name: &str) -> Option<i32> {
    // Extract a numeric length/precision from types such as varchar(50),
    // numeric(10,2), char(8).  Returns the first integer inside parentheses.
    let open = type_name.find('(')?;
    let close = type_name.find(')')?;
    let inside = &type_name[open + 1..close];
    inside
        .split(',')
        .next()
        .and_then(|s| s.trim().parse::<i32>().ok())
}
