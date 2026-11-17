//! Inventory collection: read-only catalog introspection for PostgreSQL 9.4+.
//!
//! Every query in this module is fixed SQL with bound parameters only.  No
//! table or column identifiers are interpolated.  The module is designed so
//! the parallel `db` module (T017) can pass any `GenericClient` or
//! `Transaction` straight to [`collect_inventory`].

pub mod queries;

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use thiserror::Error;
use tokio_postgres::{Client, Error as PgError, GenericClient, Row};

pub use queries::EXCLUDED_SCHEMAS_FILTER_SQL;

/// Errors that can occur while collecting an inventory.
#[derive(Debug, Error)]
pub enum InventoryError {
    #[error("postgres error: {0}")]
    Postgres(#[from] PgError),
    #[error("unexpected null in catalog column {0}")]
    UnexpectedNull(&'static str),
    #[error("version query returned no rows")]
    VersionMissing,
    #[error("could not parse server version number: {0}")]
    VersionParse(String),
}

/// A complete structural inventory of one database.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct DatabaseInventory {
    pub server_version: String,
    pub server_version_num: i32,
    pub encoding: String,
    pub collation: String,
    pub ctype: String,
    pub lc_monetary: String,
    pub schemas: Vec<Schema>,
    pub extensions: Vec<Extension>,
    pub large_object_count: i64,
    pub warnings: Vec<String>,
}

/// A schema and the objects it contains.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct Schema {
    pub name: String,
    pub owner: String,
    pub tables: Vec<Table>,
    pub views: Vec<View>,
    pub materialized_views: Vec<View>,
    pub functions: Vec<Function>,
    pub triggers: Vec<Trigger>,
    pub sequences: Vec<Sequence>,
}

/// A table, including columns, constraints, indexes and child partitions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct Table {
    pub name: String,
    pub owner: String,
    pub has_oids: bool,
    pub is_partitioned: bool,
    pub columns: Vec<Column>,
    pub constraints: Vec<Constraint>,
    pub indexes: Vec<Index>,
    pub partitions: Vec<String>,
    pub inherited_from: Vec<String>,
    pub n_live_tup: i64,
    pub size_bytes: i64,
    pub last_analyze: Option<String>,
    pub stats_missing: bool,
    pub warnings: Vec<String>,
}

/// A table column.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Hash)]
#[serde(rename_all = "snake_case")]
pub struct Column {
    pub name: String,
    pub type_name: String,
    pub length: Option<i32>,
    pub nullable: bool,
    pub default: Option<String>,
}

/// A table constraint.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct Constraint {
    pub name: String,
    pub kind: ConstraintKind,
    pub validated: bool,
    pub definition: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Hash)]
#[serde(rename_all = "snake_case")]
pub enum ConstraintKind {
    PrimaryKey,
    Unique,
    ForeignKey,
    Check,
    Other,
}

/// An index.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct Index {
    pub name: String,
    pub valid: bool,
    pub definition: String,
}

/// A sequence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub struct Sequence {
    pub name: String,
    pub data_type: String,
    pub start_value: Option<String>,
    pub min_value: Option<String>,
    pub max_value: Option<String>,
    pub increment: Option<String>,
    pub last_value: Option<String>,
    pub cycle: Option<bool>,
    pub value_skipped: bool,
}

/// A view or materialised view.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct View {
    pub name: String,
    pub definition: String,
}

/// A function or procedure.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct Function {
    pub name: String,
    pub arguments: String,
    pub definition_hash: String,
    pub language: String,
}

/// A trigger.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct Trigger {
    pub name: String,
    pub event_manipulation: String,
    pub event_object: String,
    pub action_orientation: String,
    pub timing: String,
    pub action_statement: String,
    pub enabled: String,
}

/// An installed extension.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct Extension {
    pub name: String,
    pub schema: String,
    pub version: String,
}

/// Collect a full inventory from `client`.
///
/// The caller is responsible for opening a read-only transaction and applying
/// the canonical settings block; this function only issues `SELECT` statements.
pub async fn collect_inventory(
    client: &impl GenericClient,
) -> Result<DatabaseInventory, InventoryError> {
    let version_num = queries::server_version_num(client).await?;
    let server_version = queries::server_version(client).await?;
    let db_props = queries::database_properties(client).await?;

    let mut warnings = Vec::new();

    if version_num < 100000 {
        warnings.push(
            "Sequence last_value cannot be read on PostgreSQL < 10 without identifier-based queries; \
             skipped, sequences marked 'value_skipped'."
                .to_string(),
        );
    }

    let schemas = queries::list_schemas(client).await?;
    let mut schema_objects: BTreeMap<String, Schema> = schemas
        .into_iter()
        .map(|s| {
            (
                s.name.clone(),
                Schema {
                    name: s.name,
                    owner: s.owner,
                    tables: Vec::new(),
                    views: Vec::new(),
                    materialized_views: Vec::new(),
                    functions: Vec::new(),
                    triggers: Vec::new(),
                    sequences: Vec::new(),
                },
            )
        })
        .collect();

    let tables = queries::list_tables(client, version_num).await?;
    let columns = queries::list_columns(client).await?;
    let constraints = queries::list_constraints(client).await?;
    let indexes = queries::list_indexes(client).await?;
    let sequences = queries::list_sequences(client, version_num).await?;
    let views = queries::list_views(client).await?;
    let functions = queries::list_functions(client).await?;
    let triggers = queries::list_triggers(client).await?;
    let inheritances = queries::list_inheritances(client).await?;
    let extensions = queries::list_extensions(client).await?;
    let large_object_count = queries::large_object_count(client).await?;

    // Build a map of table identity -> schema name + table name.
    let mut table_map: BTreeMap<(String, String), Table> = BTreeMap::new();
    for t in tables {
        let mut table_warnings = Vec::new();
        if t.has_oids {
            table_warnings.push(
                "Table is stored WITH OIDS; this is deprecated and removed in PostgreSQL 12+."
                    .to_string(),
            );
        }
        if t.stats_missing {
            table_warnings.push(
                "No ANALYZE statistics found; table may be empty or statistics are missing."
                    .to_string(),
            );
        }

        table_map.insert(
            (t.schema_name.clone(), t.name.clone()),
            Table {
                name: t.name,
                owner: t.owner,
                has_oids: t.has_oids,
                is_partitioned: t.is_partitioned,
                columns: Vec::new(),
                constraints: Vec::new(),
                indexes: Vec::new(),
                partitions: Vec::new(),
                inherited_from: Vec::new(),
                n_live_tup: t.n_live_tup,
                size_bytes: t.size_bytes,
                last_analyze: t.last_analyze,
                stats_missing: t.stats_missing,
                warnings: table_warnings,
            },
        );
    }

    // Attach columns.
    for col in columns {
        if let Some(table) = table_map.get_mut(&(col.schema_name.clone(), col.table_name.clone())) {
            table.columns.push(col.column);
        }
    }

    // Attach constraints.
    for con in constraints {
        if let Some(table) = table_map.get_mut(&(con.schema_name.clone(), con.table_name.clone())) {
            table.constraints.push(con.constraint);
        }
    }

    // Attach indexes.
    for idx in indexes {
        if let Some(table) = table_map.get_mut(&(idx.schema_name.clone(), idx.table_name.clone())) {
            table.indexes.push(idx.index);
        }
    }

    // Fold inheritance / partition relationships.
    // A child inherits from a parent -> child is a partition of parent, parent is_partitioned.
    for inh in &inheritances {
        if let Some(parent) =
            table_map.get_mut(&(inh.parent_schema.clone(), inh.parent_name.clone()))
        {
            parent
                .partitions
                .push(format!("{}.{}", inh.child_schema, inh.child_name));
        }
        if let Some(child) = table_map.get_mut(&(inh.child_schema.clone(), inh.child_name.clone()))
        {
            child
                .inherited_from
                .push(format!("{}.{}", inh.parent_schema, inh.parent_name));
        }
    }

    // Attach tables to schemas, dropping child partition tables from the top
    // level because they are folded under their parent.
    for ((schema_name, table_name), table) in table_map {
        if table.inherited_from.is_empty() {
            if let Some(schema) = schema_objects.get_mut(&schema_name) {
                schema.tables.push(table);
            }
        } else {
            // Child partition tables are folded under the parent; nothing to
            // attach at schema level, but log a debug-style note if needed.
            let _ = (schema_name, table_name);
        }
    }

    // Attach sequences to schemas.
    for seq in sequences {
        if let Some(schema) = schema_objects.get_mut(&seq.schema_name) {
            schema.sequences.push(seq.sequence);
        }
    }

    // Attach views / matviews.
    for v in views {
        if let Some(schema) = schema_objects.get_mut(&v.schema_name) {
            match v.kind.as_str() {
                "m" => schema.materialized_views.push(v.view),
                _ => schema.views.push(v.view),
            }
        }
    }

    // Attach functions.
    for f in functions {
        if let Some(schema) = schema_objects.get_mut(&f.schema_name) {
            schema.functions.push(f.function);
        }
    }

    // Attach triggers.
    for tr in triggers {
        if let Some(schema) = schema_objects.get_mut(&tr.schema_name) {
            schema.triggers.push(tr.trigger);
        }
    }

    // Sort everything by name for deterministic output.
    for schema in schema_objects.values_mut() {
        schema.tables.sort_by(|a, b| a.name.cmp(&b.name));
        schema.views.sort_by(|a, b| a.name.cmp(&b.name));
        schema
            .materialized_views
            .sort_by(|a, b| a.name.cmp(&b.name));
        schema.functions.sort_by(|a, b| a.name.cmp(&b.name));
        schema.triggers.sort_by(|a, b| a.name.cmp(&b.name));
        schema.sequences.sort_by(|a, b| a.name.cmp(&b.name));
        for table in &mut schema.tables {
            table.columns.sort_by(|a, b| a.name.cmp(&b.name));
            table.constraints.sort_by(|a, b| a.name.cmp(&b.name));
            table.indexes.sort_by(|a, b| a.name.cmp(&b.name));
            table.partitions.sort();
        }
    }

    Ok(DatabaseInventory {
        server_version,
        server_version_num: version_num,
        encoding: db_props.encoding,
        collation: db_props.collation,
        ctype: db_props.ctype,
        lc_monetary: db_props.lc_monetary,
        schemas: schema_objects.into_values().collect(),
        extensions,
        large_object_count,
        warnings,
    })
}

/// Convenience: collect an inventory from a connected [`Client`].
pub async fn collect_inventory_from_client(
    client: &Client,
) -> Result<DatabaseInventory, InventoryError> {
    collect_inventory(client).await
}

// --- internal helpers ---

pub(crate) fn opt_string(row: &Row, idx: usize) -> Result<Option<String>, InventoryError> {
    Ok(row.try_get::<_, Option<String>>(idx)?)
}

pub(crate) fn req_string(row: &Row, idx: usize) -> Result<String, InventoryError> {
    row.try_get::<_, Option<String>>(idx)?
        .ok_or(InventoryError::UnexpectedNull("string column"))
}

pub(crate) fn req_bool(row: &Row, idx: usize) -> Result<bool, InventoryError> {
    row.try_get::<_, Option<bool>>(idx)?
        .ok_or(InventoryError::UnexpectedNull("bool column"))
}

pub(crate) fn req_i64(row: &Row, idx: usize) -> Result<i64, InventoryError> {
    row.try_get::<_, Option<i64>>(idx)?
        .ok_or(InventoryError::UnexpectedNull("i64 column"))
}
