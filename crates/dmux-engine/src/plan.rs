//! Run planning: turn an inventory, a mapping and the live catalogue into the
//! per-table plans the comparison core scans.
//!
//! The one rule that matters most (Daniel, core re-review): **column types for
//! the plan come from the live catalogue via `core::normalise::resolve_type`,
//! never from the (untrusted) mapping file.**  The mapping names a column pair;
//! the live database says what each side's type really is.
//!
//! The other rule enforced here is P3 column coverage: every column on each
//! side must be accounted for — mapped, or ignored with a recorded reason.  A
//! column left unmapped without a reason makes the table unable to PASS.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use thiserror::Error;

use crate::core::fingerprint::bucket_digits_for;
use crate::core::normalise::{plan_column, resolve_type, ColumnMapping, ColumnOptions, PgType};
use crate::core::CoreError;
use crate::mapping::{apply_rule, Mapping, RuleKind};
use crate::results::CheckLevel;
use tokio_postgres::Transaction;

/// Errors from run planning.
#[derive(Debug, Error)]
pub enum PlanError {
    #[error("database error: {0}")]
    Db(String),
    #[error("planning error: {0}")]
    Plan(String),
    #[error("column type could not be resolved for {0}")]
    Resolve(String),
    #[error(transparent)]
    Core(#[from] CoreError),
}

impl From<tokio_postgres::Error> for PlanError {
    fn from(e: tokio_postgres::Error) -> Self {
        PlanError::Db(render_pg_error(&e))
    }
}

/// Render a postgres error with its SQLSTATE and server message (no bound
/// values are ever part of this).
fn render_pg_error(e: &tokio_postgres::Error) -> String {
    if let Some(db) = e.as_db_error() {
        format!("{}: {}", db.code().code(), db.message())
    } else {
        e.to_string()
    }
}

/// The per-table plan the comparison core scans (column types from the live
/// catalogue).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableSpec {
    pub old_qualified: String,
    pub old_schema: String,
    pub old_name: String,
    pub new_qualified: String,
    pub new_schema: String,
    pub new_name: String,
    pub level: CheckLevel,
    pub columns: Vec<ColumnMapping>,
    pub key: Vec<usize>,
    pub bucket_digits: u8,
    pub ignored_columns: BTreeMap<String, String>,
    pub coverage_complete: bool,
    pub uncovered: Vec<String>,
}

impl TableSpec {
    pub fn is_keyed(&self) -> bool {
        !self.key.is_empty()
    }

    pub fn columns_covered(&self) -> Vec<String> {
        self.columns.iter().map(|c| c.old_name.clone()).collect()
    }
}

/// One column as read from the live catalogue, with its resolved [`PgType`].
#[derive(Debug, Clone)]
pub struct LiveColumn {
    pub name: String,
    pub oid: u32,
    pub typmod: i32,
    pub nullable: bool,
    pub attnum: i16,
    pub resolved_type: PgType,
}

/// A side's live columns plus primary-key names.
#[derive(Debug, Clone)]
pub struct LiveTable {
    pub schema: String,
    pub name: String,
    pub columns: Vec<LiveColumn>,
    pub primary_key: Vec<String>,
}

/// Read a table's live columns, resolve each column's type from the catalogue
/// (`resolve_type`), and return the primary-key names.  Runs inside the caller's
/// read-only transaction.
pub async fn read_live_table(
    txn: &Transaction<'_>,
    schema: &str,
    table: &str,
) -> Result<LiveTable, PlanError> {
    read_live_table_opt(txn, schema, table)
        .await?
        .ok_or_else(|| PlanError::Plan(format!("table {schema}.{table} does not exist")))
}

/// Like [`read_live_table`], but returns `Ok(None)` when the table does not
/// exist on this side (a structure difference the caller records, not an abort).
pub async fn read_live_table_opt(
    txn: &Transaction<'_>,
    schema: &str,
    table: &str,
) -> Result<Option<LiveTable>, PlanError> {
    // Existence check first: `to_regclass` returns NULL when absent (9.4-safe),
    // so a missing table never reaches the regclass cast that would error.
    let exists: Option<String> = txn
        .query_opt(
            "SELECT pg_catalog.to_regclass($1)::pg_catalog.text",
            &[&format!("{schema}.{table}")],
        )
        .await?
        .and_then(|r| r.try_get::<_, Option<String>>(0).ok())
        .flatten();
    if exists.is_none() {
        return Ok(None);
    }

    let raw = txn
        .query(
            "SELECT a.attname, a.atttypid, a.atttypmod, a.attnotnull, a.attnum \
             FROM pg_catalog.pg_attribute a \
             JOIN pg_catalog.pg_class c ON c.oid = a.attrelid \
             JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace \
             WHERE n.nspname = $1 AND c.relname = $2 \
               AND a.attnum > 0 AND NOT a.attisdropped \
             ORDER BY a.attnum",
            &[&schema, &table],
        )
        .await?;

    let mut columns = Vec::with_capacity(raw.len());
    for r in raw {
        let name: String = r.get(0);
        let oid: u32 = r.get(1);
        let typmod: i32 = r.get(2);
        let nullable = !r.get::<_, bool>(3);
        let attnum: i16 = r.get(4);
        let resolved_type = resolve_type(txn, oid, typmod)
            .await
            .map_err(|e| PlanError::Resolve(format!("{schema}.{table}.{name}: {e}")))?;
        columns.push(LiveColumn {
            name,
            oid,
            typmod,
            nullable,
            attnum,
            resolved_type,
        });
    }

    let regclass = format!("{schema}.{table}");
    let pk: Vec<String> = txn
        .query(
            "SELECT a.attname FROM pg_catalog.pg_index i \
             JOIN pg_catalog.pg_attribute a ON a.attrelid = i.indrelid AND a.attnum = ANY(i.indkey) \
             WHERE i.indrelid = $1::pg_catalog.text::pg_catalog.regclass AND i.indisprimary \
             ORDER BY a.attnum",
            &[&regclass],
        )
        .await?
        .into_iter()
        .map(|r| r.get(0))
        .collect();

    Ok(Some(LiveTable {
        schema: schema.to_string(),
        name: table.to_string(),
        columns,
        primary_key: pk,
    }))
}

/// Build the core `TablePlan` from a `TableSpec` by running the per-column
/// planner.  Any column whose type pair cannot be planned is an error.
pub fn core_table_plan(spec: &TableSpec) -> Result<crate::core::fingerprint::TablePlan, CoreError> {
    let mut cols = Vec::with_capacity(spec.columns.len());
    for m in &spec.columns {
        cols.push(plan_column(m)?);
    }
    crate::core::fingerprint::TablePlan::new(cols, spec.key.clone(), spec.bucket_digits)
}

/// Build a [`TableSpec`] from two live sides whose column types are already
/// resolved.  This is a pure function: it pairs columns by name (through the
/// mapping's rename rules) and records which columns are ignored or uncovered.
///
/// `estimated_rows` is the row estimate used to choose bucket digits; it is
/// computed by the caller (from `pg_stat_user_tables`).
#[allow(clippy::too_many_arguments)]
pub fn build_table_spec(
    old: &LiveTable,
    new: &LiveTable,
    new_schema: &str,
    new_name: &str,
    level: CheckLevel,
    mapping: &Mapping,
    control_total_columns: &BTreeSet<String>,
    estimated_rows: u64,
) -> TableSpec {
    let new_by_name: BTreeMap<_, _> = new.columns.iter().map(|c| (c.name.clone(), c)).collect();
    let old_pk_set: BTreeSet<_> = old.primary_key.iter().cloned().collect();

    let table_rules: Vec<_> = mapping
        .rules
        .iter()
        .filter(|r| !matches!(r.kind, RuleKind::SchemaRename))
        .cloned()
        .collect();

    // The mapping's explicit per-column renames (old name -> new name).
    let column_renames: BTreeMap<String, String> = mapping
        .schema_mappings
        .get(&old.schema)
        .and_then(|s| s.table_mappings.get(&old.name))
        .map(|t| {
            t.column_mappings
                .iter()
                .map(|(k, v)| (k.clone(), v.target_column.clone()))
                .collect()
        })
        .unwrap_or_default();

    let new_name_for = |old_col: &str| -> Option<String> {
        // Explicit mapping first, then exact name, then prefix/suffix rules.
        if let Some(target) = column_renames.get(old_col) {
            if new_by_name.contains_key(target) {
                return Some(target.clone());
            }
        }
        if new_by_name.contains_key(old_col) {
            return Some(old_col.to_string());
        }
        for rule in &table_rules {
            if let Some(renamed) = apply_rule(rule, old_col) {
                if new_by_name.contains_key(&renamed) {
                    return Some(renamed);
                }
            }
        }
        None
    };

    let mut columns: Vec<ColumnMapping> = Vec::new();
    let mut key_names: Vec<String> = Vec::new();
    let mut uncovered: Vec<String> = Vec::new();
    let mut ignored_reasons = BTreeMap::new();

    for old_c in &old.columns {
        let path = format!("{}.{}.{}", old.schema, old.name, old_c.name);
        match new_name_for(&old_c.name) {
            Some(new_col_name) => {
                let new_c = &new_by_name[&new_col_name];
                let cm = ColumnMapping {
                    old_name: old_c.name.clone(),
                    new_name: new_col_name,
                    old_type: old_c.resolved_type.clone(),
                    new_type: new_c.resolved_type.clone(),
                    options: ColumnOptions::default(),
                    control_total: control_total_columns.contains(&old_c.name),
                };
                columns.push(cm);
                if old_pk_set.contains(&old_c.name) {
                    key_names.push(old_c.name.clone());
                }
            }
            None => {
                // A column that exists on the old side but has no counterpart:
                // a structure difference (drop/rename), reported by the schema
                // diff, so it does not fail the data plan by itself.
                if let Some(it) = mapping.ignored.iter().find(|i| i.path == path) {
                    ignored_reasons.insert(path.clone(), it.reason.clone());
                } else {
                    uncovered.push(path);
                }
            }
        }
    }

    for new_c in &new.columns {
        if !columns.iter().any(|c| c.new_name == new_c.name) {
            let path = format!("{new_schema}.{new_name}.{}", new_c.name);
            if !mapping.ignored.iter().any(|i| i.path == path) {
                uncovered.push(path);
            }
        }
    }

    // Canonical order: columns sorted by old name, so column order on either
    // side never changes the hash (PLAN.md 4.2: matched by name, never position).
    let mut reorder: Vec<(usize, String)> = columns
        .iter()
        .enumerate()
        .map(|(i, c)| (i, c.old_name.clone()))
        .collect();
    reorder.sort_by(|a, b| a.1.cmp(&b.1));
    let order: Vec<usize> = reorder.iter().map(|(i, _)| *i).collect();
    columns = order.iter().map(|i| columns[*i].clone()).collect();
    // Rebuild key column indexes in the canonical order.
    let mut key: Vec<usize> = Vec::new();
    for (new_idx, old_name) in reorder.iter().enumerate() {
        if key_names.contains(&old_name.1) {
            key.push(new_idx);
        }
    }

    let bucket_digits = bucket_digits_for(estimated_rows);

    TableSpec {
        old_qualified: format!("{}.{}", old.schema, old.name),
        old_schema: old.schema.clone(),
        old_name: old.name.clone(),
        new_qualified: format!("{new_schema}.{new_name}"),
        new_schema: new_schema.to_string(),
        new_name: new_name.to_string(),
        level,
        columns,
        key,
        bucket_digits,
        ignored_columns: ignored_reasons,
        coverage_complete: uncovered.is_empty(),
        uncovered,
    }
}

/// Read the `pg_stat_user_tables.n_live_tup` estimate for a table (9.4-safe).
pub async fn row_estimate(
    txn: &Transaction<'_>,
    schema: &str,
    table: &str,
) -> Result<u64, PlanError> {
    let row = txn
        .query_opt(
            "SELECT s.n_live_tup::bigint FROM pg_catalog.pg_stat_user_tables s \
             JOIN pg_catalog.pg_class c ON c.oid = s.relid \
             JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace \
             WHERE n.nspname = $1 AND c.relname = $2",
            &[&schema, &table],
        )
        .await?;
    Ok(row
        .and_then(|r| r.try_get::<_, Option<i64>>(0).ok().flatten())
        .map(|n| n.max(0) as u64)
        .unwrap_or(0))
}

/// Serializable description of one planned table (used by the definition file).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct PlannedTable {
    pub old_qualified: String,
    pub new_qualified: String,
    pub level: CheckLevel,
    pub columns: Vec<String>,
    pub key_columns: Vec<String>,
    pub bucket_digits: u8,
    pub ignored_columns: BTreeMap<String, String>,
    pub uncovered: Vec<String>,
}

impl From<&TableSpec> for PlannedTable {
    fn from(s: &TableSpec) -> Self {
        PlannedTable {
            old_qualified: s.old_qualified.clone(),
            new_qualified: s.new_qualified.clone(),
            level: s.level,
            columns: s.columns.iter().map(|c| c.old_name.clone()).collect(),
            key_columns: s
                .key
                .iter()
                .map(|i| s.columns[*i].old_name.clone())
                .collect(),
            bucket_digits: s.bucket_digits,
            ignored_columns: s.ignored_columns.clone(),
            uncovered: s.uncovered.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numeric_type_parse_is_unaffected() {
        assert_eq!(
            PgType::parse("numeric(20,4)"),
            PgType::Numeric(Some((20, 4)))
        );
    }

    #[test]
    fn bucket_digits_are_finite() {
        assert!(bucket_digits_for(0) >= 1);
    }
}
