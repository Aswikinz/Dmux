//! The comparison orchestration (PLAN.md 3 and 4.6-4.8): turn live databases or
//! a saved fingerprint into per-table verdicts, a coverage line and an overall
//! verdict.
//!
//! This module composes the comparison core.  It opens read-only sessions only
//! through `db::ReadOnlySession` and never passes a table or a run unless the
//! core said so and the consistency checks (change detection, spot check) agree.

pub mod capture;
pub mod compare_files;

use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use thiserror::Error;

use crate::consistency::{
    read_counters, read_wal_position, sample_keys, spot_check_agrees, ChangeCounters,
    SPOT_CHECK_ROWS,
};
use crate::core::compare::{compare, SideOutcome};
use crate::core::drilldown::{classify, fetch_rows};
use crate::core::fingerprint::{fingerprint, Side, Source, TablePlan};
use crate::db::{probe_server, ConnProfile, DbError, ReadOnlySession};
use crate::mapping::Mapping;
use crate::plan::{
    build_table_spec, core_table_plan, read_live_table, read_live_table_opt, row_estimate,
    TableSpec,
};
use crate::results::{
    add_table, apply_coverage, finalise, fresh_run_id, from_core, overall_verdict, CheckLevel,
    IgnoredItem, RunResults, TableResult, Verdict, RESULTS_VERSION,
};

/// An input to a run: a live database or a saved fingerprint file.
#[derive(Debug, Clone)]
pub enum Input {
    Live(ConnProfile),
    File(crate::fingerprint_file::FingerprintFile),
}

/// What a run compares and how.
#[derive(Debug, Clone)]
pub struct RunConfig {
    pub old: Input,
    pub new: Input,
    pub mapping: Mapping,
    pub default_level: CheckLevel,
    pub levels: BTreeMap<String, CheckLevel>,
    /// Control-total column names (applied to every matching column).
    pub control_total_columns: BTreeSet<String>,
    /// Parallel-scan limits per side (PLAN.md 4.6).
    pub old_parallelism: usize,
    pub new_parallelism: usize,
    pub allow_superuser: bool,
    pub export_keys: bool,
    /// An optional resume store path: tables already finished there are skipped
    /// on `--resume`, and each table's result is saved as it completes.
    pub resume_store: Option<std::path::PathBuf>,
}

impl Default for RunConfig {
    fn default() -> Self {
        Self {
            old: Input::Live(ConnProfile::new(
                "127.0.0.1",
                5433,
                "bank",
                "dmux_dev",
                "",
                crate::db::TlsMode::Disable("default".to_string()),
            )),
            new: Input::Live(ConnProfile::new(
                "127.0.0.1",
                5434,
                "bank",
                "dmux_dev",
                "",
                crate::db::TlsMode::Disable("default".to_string()),
            )),
            mapping: Mapping::new(),
            default_level: CheckLevel::FullValues,
            levels: BTreeMap::new(),
            control_total_columns: BTreeSet::new(),
            old_parallelism: 2,
            new_parallelism: 4,
            allow_superuser: false,
            export_keys: false,
            resume_store: None,
        }
    }
}

/// Errors from running a comparison.
#[derive(Debug, Error)]
pub enum RunError {
    #[error("database error: {0}")]
    Db(#[from] DbError),
    #[error("inventory error: {0}")]
    Inventory(#[from] crate::inventory::InventoryError),
    #[error("planning error: {0}")]
    Plan(#[from] crate::plan::PlanError),
    #[error("core error: {0}")]
    Core(#[from] crate::core::CoreError),
    #[error("consistency error: {0}")]
    Consistency(#[from] crate::consistency::ConsistencyError),
    #[error("comparison error: {0}")]
    Compare(#[from] crate::core::compare::CompareError),
    #[error("resume store error: {0}")]
    Store(#[from] crate::store::StoreError),
    #[error("a full run needs both sides live")]
    NeedsLive,
}

impl From<tokio_postgres::Error> for RunError {
    fn from(e: tokio_postgres::Error) -> Self {
        RunError::Db(DbError::Postgres {
            // `Error::to_string()` for a server error is just "db error"; the
            // real reason lives in `as_db_error()`.  Keep the public message
            // free of row values (SQLSTATE + server message only).
            message: render_pg_error(&e),
        })
    }
}

/// Render a tokio-postgres error including the server message and SQLSTATE,
/// without ever including bound-parameter values (they are not part of it).
fn render_pg_error(e: &tokio_postgres::Error) -> String {
    if let Some(db) = e.as_db_error() {
        format!("{}: {}", db.code().code(), db.message())
    } else {
        e.to_string()
    }
}

impl From<crate::core::normalise::NormaliseError> for RunError {
    fn from(e: crate::core::normalise::NormaliseError) -> Self {
        RunError::Core(crate::core::CoreError::Normalise(e))
    }
}

/// A [`RunError`] as a [`DbError`], for the `with_read_only` closure boundary.
fn run_err(e: RunError) -> DbError {
    DbError::Postgres {
        message: e.to_string(),
    }
}

/// The outcome of a run, ready for JSON serialisation.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct RunOutcome {
    pub results: RunResults,
    /// 0 pass, 1 fail, 2 warnings (PLAN.md 4.8); not part of the JSON payload.
    #[serde(skip)]
    pub exit_code: u8,
}

/// Enumerate `(schema, table)` pairs in scope from a live session, partitions
/// already folded under their parent by the inventory.
pub async fn list_tables(session: &mut ReadOnlySession) -> Result<Vec<(String, String)>, RunError> {
    let inventory = session
        .with_read_only("30s", |txn| {
            Box::pin(async move {
                crate::inventory::collect_inventory(txn)
                    .await
                    .map_err(RunError::from)
                    .map_err(run_err)
            })
        })
        .await?;
    let mut out = Vec::new();
    for schema in &inventory.schemas {
        for table in &schema.tables {
            out.push((schema.name.clone(), table.name.clone()));
        }
    }
    out.sort();
    Ok(out)
}

/// Map an old table name to its new name: explicit `table_mappings` first, then
/// schema/prefix/suffix rules, then identity.
fn map_table(mapping: &Mapping, schema: &str, table: &str) -> (String, String) {
    // Explicit per-table rename in the mapping.
    if let Some(sm) = mapping.schema_mappings.get(schema) {
        if let Some(tm) = sm.table_mappings.get(table) {
            if !tm.target_table.is_empty() {
                return (sm.target_schema.clone(), tm.target_table.clone());
            }
        }
        if !sm.target_schema.is_empty() && sm.target_schema != schema {
            return (sm.target_schema.clone(), table.to_string());
        }
    }
    for rule in &mapping.rules {
        if matches!(rule.kind, crate::mapping::RuleKind::SchemaRename)
            && rule.pattern.eq_ignore_ascii_case(schema)
        {
            return (rule.target.clone(), table.to_string());
        }
    }
    for rule in &mapping.rules {
        if let Some(renamed) = crate::mapping::apply_rule(rule, table) {
            return (schema.to_string(), renamed);
        }
    }
    (schema.to_string(), table.to_string())
}

/// Run a full live comparison over both databases.
pub async fn run(config: &RunConfig) -> Result<RunOutcome, RunError> {
    let (Input::Live(old_profile), Input::Live(new_profile)) = (&config.old, &config.new) else {
        let result = match &config.old {
            Input::File(old_file) => match &config.new {
                Input::File(new_file) => compare_files::compare_two_files(old_file, new_file),
                Input::Live(new_profile) => {
                    compare_files::compare_file_to_live(
                        old_file,
                        new_profile,
                        config.allow_superuser,
                    )
                    .await
                }
            },
            Input::Live(old_profile) => match &config.new {
                Input::File(new_file) => {
                    compare_files::compare_file_to_live(
                        new_file,
                        old_profile,
                        config.allow_superuser,
                    )
                    .await
                }
                Input::Live(_) => Err(RunError::NeedsLive),
            },
        }?;
        return Ok(result);
    };

    let tool_version = crate::version().to_string();
    let mapping_hash = config.mapping.mapping_hash.clone();
    // A separate copy for the per-table scan closures (the primary one is moved
    // into the results payload).
    let scan_mapping_hash = mapping_hash.clone();

    let old_info = probe_server(old_profile, config.allow_superuser).await?;
    let new_info = probe_server(new_profile, config.allow_superuser).await?;

    let mut warnings = Vec::new();
    if old_info.sql_ascii {
        warnings
            .push("old database is SQL_ASCII; never a clean PASS on encoding (P19)".to_string());
    }
    if new_info.sql_ascii {
        warnings
            .push("new database is SQL_ASCII; never a clean PASS on encoding (P19)".to_string());
    }
    let sql_ascii = old_info.sql_ascii || new_info.sql_ascii;

    let mut old_session = ReadOnlySession::open(old_profile, config.allow_superuser).await?;
    let mut new_session = ReadOnlySession::open(new_profile, config.allow_superuser).await?;

    // Open the resume store, if requested, and load the set of already-finished
    // tables to skip.
    let store = match config.resume_store.as_ref() {
        Some(path) => Some(crate::store::Store::open(path)?),
        None => None,
    };
    let skip_tables: BTreeSet<String> = match store.as_ref() {
        Some(s) => s.finished_tables()?.into_iter().collect(),
        None => BTreeSet::new(),
    };
    let store = std::cell::RefCell::new(store);

    // Record the write-ahead-log position at run start (version-branched).
    let wal_start = new_session
        .with_read_only("5s", |to| {
            let v = new_info.version_num;
            Box::pin(async move {
                read_wal_position(&*to, v)
                    .await
                    .map_err(RunError::from)
                    .map_err(run_err)
            })
        })
        .await?;

    let old_tables = list_tables(&mut old_session).await?;

    // Plan every table first (resolves column types from the live catalogue).
    let mut specs: Vec<TableSpec> = Vec::with_capacity(old_tables.len());
    let mut missing_tables: Vec<String> = Vec::new();
    for (schema, table) in &old_tables {
        let (new_schema, new_name) = map_table(&config.mapping, schema, table);
        let level = config
            .levels
            .get(&format!("{schema}.{table}"))
            .copied()
            .unwrap_or(config.default_level);

        let old_live = old_session
            .with_read_only("5s", |to| {
                Box::pin(async move {
                    read_live_table(&*to, schema, table)
                        .await
                        .map_err(RunError::from)
                        .map_err(run_err)
                })
            })
            .await?;
        let (ns_n, nn_n) = (new_schema.clone(), new_name.clone());
        let new_live = new_session
            .with_read_only("5s", |to| {
                Box::pin(async move {
                    read_live_table_opt(&*to, &ns_n, &nn_n)
                        .await
                        .map_err(RunError::from)
                        .map_err(run_err)
                })
            })
            .await?;
        let Some(new_live) = new_live else {
            // The mapped new-side table does not exist: a structure difference
            // (rename/drop), recorded here so the run never aborts and the
            // table is reported as missing, not PASS.
            missing_tables.push(format!("{schema}.{table}"));
            continue;
        };
        let estimate = old_session
            .with_read_only("5s", |to| {
                Box::pin(async move {
                    row_estimate(&*to, schema, table)
                        .await
                        .map_err(RunError::from)
                        .map_err(run_err)
                })
            })
            .await?;

        specs.push(build_table_spec(
            &old_live,
            &new_live,
            &new_schema,
            &new_name,
            level,
            &config.mapping,
            &config.control_total_columns,
            estimate,
        ));
    }

    // Build core plans up front so a bad column fails before any scan.
    let mut plans: Vec<(TablePlan, Source, Source)> = Vec::with_capacity(specs.len());
    for spec in &specs {
        let plan = core_table_plan(spec)?;
        let old_src = Source::Table {
            schema: spec.old_schema.clone(),
            name: spec.old_name.clone(),
        };
        let new_src = Source::Table {
            schema: spec.new_schema.clone(),
            name: spec.new_name.clone(),
        };
        plans.push((plan, old_src, new_src));
    }

    let mut results = RunResults {
        results_version: RESULTS_VERSION,
        run_id: fresh_run_id(),
        tool_version,
        mapping_hash,
        verdict: Verdict::Pass,
        coverage_line: String::new(),
        fully_checked: 0,
        counts_only: 0,
        ignored: 0,
        tables: BTreeMap::new(),
        ignored_items: Vec::new(),
        changed_during_check: Vec::new(),
        warnings,
        wal_start_lsn: wal_start.lsn.clone(),
        wal_end_lsn: None,
        results_sha256: String::new(),
    };
    if let Some(s) = store.borrow().as_ref() {
        let _ = s.set_meta("run_id", &results.run_id);
        let _ = s.set_meta("tool_version", &results.tool_version);
    }

    // Tables whose mapped new-side counterpart does not exist are recorded as
    // missing (a structure difference), never silently PASSed.
    for name in &missing_tables {
        results.tables.insert(
            name.clone(),
            TableResult {
                table: name.clone(),
                level: config.default_level,
                verdict: Verdict::Fail,
                old_rows: None,
                new_rows: None,
                columns_covered: None,
                differing_buckets: None,
                missing_buckets: None,
                extra_buckets: None,
                missing_rows: None,
                extra_rows: None,
                changed_rows: None,
                reasons: vec!["table missing in the new database".to_string()],
            },
        );
        results.fully_checked += 1;
    }

    // Change detection (PLAN 3.4): snapshot every table's counters before the
    // scans, then again after, and flag any table that moved.  This run-level
    // sweep is robust to a small table scanned in a few milliseconds.
    let before_old_counters = snapshot_counters(&mut old_session, &specs, Side::Old).await?;
    let before_new_counters = snapshot_counters(&mut new_session, &specs, Side::New).await?;

    for (i, spec) in specs.iter().enumerate() {
        let plan = plans[i].0.clone();
        let old_src = plans[i].1.clone();
        let new_src = plans[i].2.clone();
        let table_path = format!("{}.{}", spec.old_schema, spec.old_name);

        // Resume: skip a table already finished in a previous run.
        if skip_tables.contains(&table_path) {
            if let Some(prev) = store.borrow().as_ref().and_then(|s| {
                s.load_tables()
                    .ok()
                    .and_then(|v| v.into_iter().find(|t| t.table == table_path))
            }) {
                add_table(
                    &mut results,
                    TableResult {
                        table: table_path.clone(),
                        level: prev.level,
                        verdict: prev.verdict,
                        old_rows: prev.old_rows,
                        new_rows: prev.new_rows,
                        columns_covered: None,
                        differing_buckets: None,
                        missing_buckets: None,
                        extra_buckets: None,
                        missing_rows: None,
                        extra_rows: None,
                        changed_rows: None,
                        reasons: prev.reasons.clone(),
                    },
                );
            }
            continue;
        }

        // Table-level ignore.
        if let Some(item) = config
            .mapping
            .ignored
            .iter()
            .find(|it| it.path == table_path)
        {
            results.ignored_items.push(IgnoredItem {
                path: table_path,
                reason: item.reason.clone(),
            });
            results.ignored += 1;
            continue;
        }

        // Record columns with no counterpart as schema notes for the report; they
        // are structure differences (handled by `dmux diff`), so they do not
        // change the data verdict by themselves.
        let mut schema_notes: Vec<String> = Vec::new();
        for c in &spec.uncovered {
            schema_notes.push(format!("column has no mapped counterpart: {c}"));
        }

        // Scan both sides.
        let old_fp = old_session
            .with_read_only("300s", |to| {
                let p = plan.clone();
                let s = old_src.clone();
                let mh = scan_mapping_hash.clone();
                Box::pin(async move {
                    fingerprint(&*to, &p, Side::Old, &s, &mh)
                        .await
                        .map_err(RunError::from)
                        .map_err(run_err)
                })
            })
            .await?;
        let new_fp = new_session
            .with_read_only("300s", |to| {
                let p = plan.clone();
                let s = new_src.clone();
                let mh = scan_mapping_hash.clone();
                Box::pin(async move {
                    fingerprint(&*to, &p, Side::New, &s, &mh)
                        .await
                        .map_err(RunError::from)
                        .map_err(run_err)
                })
            })
            .await?;

        // Change detection is resolved after the loop from the before/after
        // counter snapshots (below).

        let comparison = compare(SideOutcome::Ok(&old_fp), SideOutcome::Ok(&new_fp))?;

        // Drill down.
        let (mut missing_rows, mut extra_rows, mut changed_rows) = (None, None, None);
        // Any schema notes (dropped/added columns) are reported here as
        // informational; they do not change the data verdict.
        let mut reasons: Vec<String> = schema_notes;
        // Translate the core's not-pass reasons into the report.
        for r in &comparison.reasons {
            reasons.push(match r {
                crate::core::compare::NotPassReason::CountsDiffer => {
                    "row counts differ".to_string()
                }
                crate::core::compare::NotPassReason::BucketsDiffer => {
                    "bucket summaries differ".to_string()
                }
                crate::core::compare::NotPassReason::ControlTotalsDiffer => {
                    "control totals differ".to_string()
                }
                crate::core::compare::NotPassReason::NeedsReview => {
                    "a column was compared by an inexact printed form".to_string()
                }
                crate::core::compare::NotPassReason::SideErrored => {
                    "a side failed to scan".to_string()
                }
                crate::core::compare::NotPassReason::Truncated => {
                    "a side returned a truncated scan".to_string()
                }
            });
        }
        let mut drill_fault = false;
        if comparison.verdict != crate::core::compare::Verdict::Pass
            && comparison.verdict != crate::core::compare::Verdict::Error
        {
            let buckets = comparison.drilldown_buckets();
            let old_refs = old_session
                .with_read_only("300s", |to| {
                    let p = plan.clone();
                    let s = old_src.clone();
                    let b = buckets.clone();
                    Box::pin(async move {
                        fetch_rows(&*to, &p, Side::Old, &s, &b)
                            .await
                            .map_err(RunError::from)
                            .map_err(run_err)
                    })
                })
                .await?;
            let new_refs = new_session
                .with_read_only("300s", |to| {
                    let p = plan.clone();
                    let s = new_src.clone();
                    let b = buckets.clone();
                    Box::pin(async move {
                        fetch_rows(&*to, &p, Side::New, &s, &b)
                            .await
                            .map_err(RunError::from)
                            .map_err(run_err)
                    })
                })
                .await?;
            let drill = classify(old_refs, new_refs);
            missing_rows = Some(drill.missing.len());
            extra_rows = Some(drill.extra.len());
            changed_rows = Some(drill.changed.len());
            if !drill.explains_difference() {
                reasons
                    .push("drill-down found no row explaining the differing buckets".to_string());
                drill_fault = true;
            }
        }

        // Independent spot check for keyed tables.
        let mut spot_fault = false;
        if spec.is_keyed() {
            let key_col = spec.columns[spec.key[0]].old_name.clone();
            let old_keys = old_session
                .with_read_only("30s", |to| {
                    let k = key_col.clone();
                    Box::pin(async move {
                        sample_keys(&*to, &spec.old_schema, &spec.old_name, &k, SPOT_CHECK_ROWS)
                            .await
                            .map_err(RunError::from)
                            .map_err(run_err)
                    })
                })
                .await?;
            let new_keys = new_session
                .with_read_only("30s", |to| {
                    let k = key_col.clone();
                    Box::pin(async move {
                        sample_keys(&*to, &spec.new_schema, &spec.new_name, &k, SPOT_CHECK_ROWS)
                            .await
                            .map_err(RunError::from)
                            .map_err(run_err)
                    })
                })
                .await?;
            if !spot_check_agrees(&old_keys, &new_keys) {
                reasons.push("independent spot check disagrees with the fingerprint".to_string());
                spot_fault = true;
            }
        }

        let verdict = if spot_fault || drill_fault {
            Verdict::ToolFault
        } else {
            from_core(comparison.verdict)
        };

        add_table(
            &mut results,
            TableResult {
                table: table_path.clone(),
                level: spec.level,
                verdict,
                old_rows: comparison.old_rows,
                new_rows: comparison.new_rows,
                columns_covered: Some(spec.columns_covered()),
                differing_buckets: Some(comparison.differing.len()),
                missing_buckets: Some(comparison.missing.len()),
                extra_buckets: Some(comparison.extra.len()),
                missing_rows,
                extra_rows,
                changed_rows,
                reasons: reasons.clone(),
            },
        );

        // Save this finished table to the resume store (counts and hashes only,
        // never row values).
        if let Some(s) = store.borrow().as_ref() {
            if let Some(t) = results.tables.get(&table_path) {
                let _ = s.save_table(&crate::store::StoredTableResult {
                    table: table_path.clone(),
                    level: spec.level,
                    verdict: t.verdict,
                    old_rows: t.old_rows,
                    new_rows: t.new_rows,
                    bucket_digits: spec.bucket_digits,
                    buckets_old: Some(old_fp.buckets.clone()),
                    buckets_new: Some(new_fp.buckets.clone()),
                    control_totals_old: old_fp.control_totals.clone(),
                    control_totals_new: new_fp.control_totals.clone(),
                    needs_review: old_fp.needs_review || new_fp.needs_review,
                    reasons,
                });
            }
        }
    }

    // Ignored columns also go into the report.
    for spec in &specs {
        for (path, reason) in &spec.ignored_columns {
            results.ignored_items.push(IgnoredItem {
                path: path.clone(),
                reason: reason.clone(),
            });
        }
    }

    // Change detection: any table whose counters moved during the run is
    // CHANGED_DURING_CHECK, never PASS (PLAN 3.4).
    let after_old_counters = snapshot_counters(&mut old_session, &specs, Side::Old).await?;
    let after_new_counters = snapshot_counters(&mut new_session, &specs, Side::New).await?;
    for spec in &specs {
        let table_path = format!("{}.{}", spec.old_schema, spec.old_name);
        let old_moved = before_old_counters.get(&table_path) != after_old_counters.get(&table_path);
        let new_moved = before_new_counters.get(&table_path) != after_new_counters.get(&table_path);
        if old_moved || new_moved {
            if let Some(t) = results.tables.get_mut(&table_path) {
                t.verdict = Verdict::ChangedDuringCheck;
                t.reasons
                    .push("the table changed during the check".to_string());
            }
            results.changed_during_check.push(table_path);
        }
    }

    if !results.changed_during_check.is_empty() {
        results.warnings.push(format!(
            "{} table(s) changed during check",
            results.changed_during_check.len()
        ));
    }

    let verdicts: Vec<Verdict> = results.tables.values().map(|t| t.verdict).collect();
    if sql_ascii {
        let v = overall_verdict(&verdicts);
        results.verdict = if v.is_pass() { Verdict::NeedsReview } else { v };
    } else {
        results.verdict = overall_verdict(&verdicts);
    }
    results = apply_coverage(results);

    // Record the write-ahead-log position at run end.
    let wal_end = new_session
        .with_read_only("5s", |to| {
            let v = new_info.version_num;
            Box::pin(async move {
                read_wal_position(&*to, v)
                    .await
                    .map_err(RunError::from)
                    .map_err(run_err)
            })
        })
        .await?;
    results.wal_end_lsn = wal_end.lsn;

    let exit_code = exit_code_for(results.verdict);
    Ok(RunOutcome {
        results: finalise(results),
        exit_code,
    })
}

fn exit_code_for(v: Verdict) -> u8 {
    match v {
        Verdict::Pass => 0,
        Verdict::Warning | Verdict::NeedsReview => 2,
        _ => 1,
    }
}

/// Read every in-scope table's change counters in one pass, keyed by
/// `schema.table`.  Used for the run-level before/after change-detection sweep.
async fn snapshot_counters(
    session: &mut ReadOnlySession,
    specs: &[TableSpec],
    side: Side,
) -> Result<BTreeMap<String, ChangeCounters>, RunError> {
    let mut out = BTreeMap::new();
    for spec in specs {
        let (schema, table) = match side {
            Side::Old => (&spec.old_schema, &spec.old_name),
            Side::New => (&spec.new_schema, &spec.new_name),
        };
        let counters = session
            .with_read_only("5s", |to| {
                let s = schema.clone();
                let t = table.clone();
                Box::pin(async move {
                    read_counters(&*to, &s, &t)
                        .await
                        .map_err(RunError::from)
                        .map_err(run_err)
                })
            })
            .await?;
        let key = match side {
            Side::Old => spec.old_qualified.clone(),
            Side::New => spec.old_qualified.clone(), // keyed by old identity on both sides
        };
        out.insert(key, counters);
    }
    Ok(out)
}
