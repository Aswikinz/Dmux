//! Run planning: turn an inventory, a mapping and the live catalogue into the
//! per-table plans the comparison core scans.
//!
//! The one rule that matters most (Daniel, core re-review): **column types for
//! the plan come from the live catalogue via `core::normalise::resolve_type`,
//! never from the (untrusted) mapping file.**  The mapping names a column pair;
//! the live database says what each side's type really is.
//!
//! P3 column coverage is computed here and enforced by `run::verdict`: every
//! column on each side must be paired with a column on the other side or
//! ignored with a recorded reason.  An old column with no counterpart
//! (`dropped`) fails the table; a new column with no counterpart (`added`)
//! makes it NEEDS_REVIEW, because its values are not checked.
//!
//! Tables are looked up by exact catalogue name with bound parameters
//! (`nspname = $1 AND relname = $2`), never by parsing a `schema.table` string,
//! so an upper-case or dotted name can never resolve to a different table.

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
pub(crate) fn render_pg_error(e: &tokio_postgres::Error) -> String {
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
    /// The paired columns, sorted by old name (order never matters).
    pub columns: Vec<ColumnMapping>,
    pub key: Vec<usize>,
    pub bucket_digits: u8,
    /// Columns ignored with a recorded reason (path -> reason).
    pub ignored_columns: BTreeMap<String, String>,
    /// Old columns with no new counterpart and no ignore (old paths).
    pub dropped: Vec<String>,
    /// New columns with no old counterpart and no ignore (new paths).
    pub added: Vec<String>,
    /// `dropped.is_empty() && added.is_empty()`.
    pub coverage_complete: bool,
    /// `dropped` followed by `added`.
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

/// The catalogue oid of a table (`r`, `p` or `f`), by exact schema and table
/// name as bound parameters.  `None` when there is no such table.
pub async fn table_oid(
    txn: &Transaction<'_>,
    schema: &str,
    table: &str,
) -> Result<Option<u32>, PlanError> {
    let row = txn
        .query_opt(
            "SELECT c.oid FROM pg_catalog.pg_class c \
             JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace \
             WHERE n.nspname = $1 AND c.relname = $2 AND c.relkind IN ('r', 'p', 'f')",
            &[&schema, &table],
        )
        .await?;
    Ok(row.map(|r| r.get::<_, u32>(0)))
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
    let Some(oid) = table_oid(txn, schema, table).await? else {
        return Ok(None);
    };

    let raw = txn
        .query(
            "SELECT a.attname, a.atttypid, a.atttypmod, a.attnotnull, a.attnum \
             FROM pg_catalog.pg_attribute a \
             WHERE a.attrelid = $1 AND a.attnum > 0 AND NOT a.attisdropped \
             ORDER BY a.attnum",
            &[&oid],
        )
        .await?;

    let mut columns = Vec::with_capacity(raw.len());
    for r in raw {
        let name: String = r.get(0);
        let col_oid: u32 = r.get(1);
        let typmod: i32 = r.get(2);
        let nullable = !r.get::<_, bool>(3);
        let attnum: i16 = r.get(4);
        let resolved_type = resolve_type(txn, col_oid, typmod)
            .await
            .map_err(|e| PlanError::Resolve(format!("{schema}.{table}.{name}: {e}")))?;
        columns.push(LiveColumn {
            name,
            oid: col_oid,
            typmod,
            nullable,
            attnum,
            resolved_type,
        });
    }

    let pk: Vec<String> = txn
        .query(
            "SELECT a.attname FROM pg_catalog.pg_index i \
             JOIN pg_catalog.pg_attribute a ON a.attrelid = i.indrelid AND a.attnum = ANY(i.indkey) \
             WHERE i.indrelid = $1 AND i.indisprimary \
             ORDER BY a.attnum",
            &[&oid],
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
/// planner.  Any column whose type pair cannot be planned is an error, which
/// the run records as a TOOL_FAULT for that table.
pub fn core_table_plan(spec: &TableSpec) -> Result<crate::core::fingerprint::TablePlan, CoreError> {
    let mut cols = Vec::with_capacity(spec.columns.len());
    for m in &spec.columns {
        cols.push(plan_column(m)?);
    }
    crate::core::fingerprint::TablePlan::new(cols, spec.key.clone(), spec.bucket_digits)
}

/// The explicit per-column renames the mapping holds for one old table.
pub fn column_renames(mapping: &Mapping, schema: &str, table: &str) -> BTreeMap<String, String> {
    mapping
        .schema_mappings
        .get(schema)
        .and_then(|s| s.table_mappings.get(table))
        .map(|t| {
            t.column_mappings
                .iter()
                .filter(|(_, v)| !v.target_column.is_empty())
                .map(|(k, v)| (k.clone(), v.target_column.clone()))
                .collect()
        })
        .unwrap_or_default()
}

/// What a caller supplies besides the two live tables.
#[derive(Debug, Clone, Copy)]
pub struct SpecInputs<'a> {
    pub new_schema: &'a str,
    pub new_name: &'a str,
    pub level: CheckLevel,
    pub mapping: &'a Mapping,
    /// Control-total column names (applied to every table that has them).
    pub control_total_columns: &'a BTreeSet<String>,
    /// Per-column options, keyed by old path `schema.table.column`.
    pub column_options: &'a BTreeMap<String, ColumnOptions>,
    pub estimated_rows: u64,
}

/// Build a [`TableSpec`] from two live sides whose column types are already
/// resolved.  This is a pure function: it pairs columns by name (through the
/// mapping), records ignores, and splits unpaired columns into `dropped` and
/// `added` for the P3 coverage rule.
pub fn build_table_spec(old: &LiveTable, new: &LiveTable, inp: SpecInputs<'_>) -> TableSpec {
    let mapping = inp.mapping;
    let new_by_name: BTreeMap<_, _> = new.columns.iter().map(|c| (c.name.clone(), c)).collect();
    let old_pk_set: BTreeSet<_> = old.primary_key.iter().cloned().collect();

    let table_rules: Vec<_> = mapping
        .rules
        .iter()
        .filter(|r| !matches!(r.kind, RuleKind::SchemaRename))
        .cloned()
        .collect();
    let renames = column_renames(mapping, &old.schema, &old.name);

    let new_name_for = |old_col: &str| -> Option<String> {
        // Explicit mapping first, then exact name, then prefix/suffix rules.
        if let Some(target) = renames.get(old_col) {
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
    let ignore_reason = |path: &str| {
        mapping
            .ignored
            .iter()
            .find(|i| i.path == path)
            .map(|i| i.reason.clone())
    };

    let mut columns: Vec<ColumnMapping> = Vec::new();
    let mut key_names: Vec<String> = Vec::new();
    let mut dropped: Vec<String> = Vec::new();
    let mut added: Vec<String> = Vec::new();
    let mut ignored_reasons = BTreeMap::new();
    // New-side counterparts of ignored old columns: accounted for, not "added".
    let mut ignored_targets: BTreeSet<String> = BTreeSet::new();

    for old_c in &old.columns {
        let path = format!("{}.{}.{}", old.schema, old.name, old_c.name);
        // An explicit ignore wins even when a counterpart exists: the column is
        // deliberately not compared, and the reason goes into the report.
        if let Some(reason) = ignore_reason(&path) {
            ignored_reasons.insert(path, reason);
            if let Some(target) = new_name_for(&old_c.name) {
                ignored_targets.insert(target);
            }
            continue;
        }
        match new_name_for(&old_c.name) {
            Some(new_col_name) => {
                let new_c = &new_by_name[&new_col_name];
                columns.push(ColumnMapping {
                    old_name: old_c.name.clone(),
                    new_name: new_col_name,
                    old_type: old_c.resolved_type.clone(),
                    new_type: new_c.resolved_type.clone(),
                    options: inp.column_options.get(&path).cloned().unwrap_or_default(),
                    control_total: inp.control_total_columns.contains(&old_c.name),
                });
                if old_pk_set.contains(&old_c.name) {
                    key_names.push(old_c.name.clone());
                }
            }
            None => dropped.push(path),
        }
    }

    for new_c in &new.columns {
        if columns.iter().any(|c| c.new_name == new_c.name) {
            continue;
        }
        let path = format!("{}.{}.{}", inp.new_schema, inp.new_name, new_c.name);
        if let Some(reason) = ignore_reason(&path) {
            ignored_reasons.insert(path, reason);
        } else if !ignored_targets.contains(&new_c.name) {
            added.push(path);
        }
    }

    // Canonical order: columns sorted by old name, so column order on either
    // side never changes the hash (PLAN.md 4.2: matched by name, never position).
    columns.sort_by(|a, b| a.old_name.cmp(&b.old_name));
    let key: Vec<usize> = columns
        .iter()
        .enumerate()
        .filter(|(_, c)| key_names.contains(&c.old_name))
        .map(|(i, _)| i)
        .collect();

    let uncovered: Vec<String> = dropped.iter().chain(added.iter()).cloned().collect();
    TableSpec {
        old_qualified: format!("{}.{}", old.schema, old.name),
        old_schema: old.schema.clone(),
        old_name: old.name.clone(),
        new_qualified: format!("{}.{}", inp.new_schema, inp.new_name),
        new_schema: inp.new_schema.to_string(),
        new_name: inp.new_name.to_string(),
        level: inp.level,
        columns,
        key,
        bucket_digits: bucket_digits_for(inp.estimated_rows),
        ignored_columns: ignored_reasons,
        coverage_complete: uncovered.is_empty(),
        dropped,
        added,
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
pub(crate) mod tests {
    use super::*;
    use crate::mapping::IgnoredItem;

    pub(crate) fn col(name: &str, ty: &str) -> LiveColumn {
        LiveColumn {
            name: name.into(),
            oid: 0,
            typmod: -1,
            nullable: true,
            attnum: 0,
            resolved_type: PgType::parse(ty),
        }
    }

    pub(crate) fn live(name: &str, cols: Vec<LiveColumn>, pk: &[&str]) -> LiveTable {
        LiveTable {
            schema: "s".into(),
            name: name.into(),
            columns: cols,
            primary_key: pk.iter().map(|s| s.to_string()).collect(),
        }
    }

    fn spec(old: &LiveTable, new: &LiveTable, mapping: &Mapping) -> TableSpec {
        let none = BTreeSet::new();
        let opts = BTreeMap::new();
        build_table_spec(
            old,
            new,
            SpecInputs {
                new_schema: "s",
                new_name: &new.name,
                level: CheckLevel::FullValues,
                mapping,
                control_total_columns: &none,
                column_options: &opts,
                estimated_rows: 0,
            },
        )
    }

    #[test]
    fn dropped_and_added_columns_are_split() {
        let old = live(
            "t",
            vec![col("id", "integer"), col("gone", "text")],
            &["id"],
        );
        let new = live(
            "t",
            vec![col("id", "integer"), col("fresh", "text")],
            &["id"],
        );
        let s = spec(&old, &new, &Mapping::new());
        assert_eq!(s.dropped, vec!["s.t.gone"]);
        assert_eq!(s.added, vec!["s.t.fresh"]);
        assert!(!s.coverage_complete);
        assert_eq!(s.columns_covered(), vec!["id"]);
        assert_eq!(s.key, vec![0]);
    }

    #[test]
    fn an_ignored_column_is_accounted_for_with_its_reason() {
        let old = live(
            "t",
            vec![col("id", "integer"), col("gone", "text")],
            &["id"],
        );
        let new = live("t", vec![col("id", "integer")], &["id"]);
        let mut m = Mapping::new();
        m.ignored.push(IgnoredItem {
            path: "s.t.gone".into(),
            reason: "retired in the new design".into(),
        });
        let s = spec(&old, &new, &m);
        assert!(s.coverage_complete);
        assert_eq!(s.ignored_columns["s.t.gone"], "retired in the new design");
    }

    #[test]
    fn column_order_never_matters() {
        let a = live("t", vec![col("b", "text"), col("a", "integer")], &["a"]);
        let b = live("t", vec![col("a", "integer"), col("b", "text")], &["a"]);
        let s1 = spec(&a, &b, &Mapping::new());
        let s2 = spec(&b, &a, &Mapping::new());
        assert_eq!(s1.columns_covered(), vec!["a", "b"]);
        assert_eq!(s1.columns_covered(), s2.columns_covered());
        assert_eq!(s1.key, s2.key);
    }

    #[test]
    fn options_come_from_the_run_and_reach_the_plan() {
        let old = live("t", vec![col("id", "integer"), col("n", "text")], &["id"]);
        let mut opts = BTreeMap::new();
        opts.insert(
            "s.t.n".to_string(),
            ColumnOptions {
                ignore_trailing_spaces: true,
                ..Default::default()
            },
        );
        let none = BTreeSet::new();
        let m = Mapping::new();
        let s = build_table_spec(
            &old,
            &old,
            SpecInputs {
                new_schema: "s",
                new_name: "t",
                level: CheckLevel::FullValues,
                mapping: &m,
                control_total_columns: &none,
                column_options: &opts,
                estimated_rows: 0,
            },
        );
        let p = core_table_plan(&s).unwrap();
        assert!(p.columns()[1].old_expr.starts_with("rtrim("));
    }
}
