//! Data-half engine commands: choose checks, run, progress, pause/cancel and
//! results (PLAN.md 4.4-4.7).
//!
//! The one rule that dominates every command here (AGENTS.md hard rule, and the
//! task's cardinal rule): **the UI must only ever show what the engine's results
//! say.** A false PASS is the worst bug this project can have.  So:
//!
//! * verdicts are copied verbatim from the engine, never guessed;
//! * progress is read back from the engine's own resume store, never invented;
//! * cancel and pause stop the engine through its `RunControl` and cancel every
//!   in-flight server statement; a stopped run never reports results, and its
//!   phase can never be overwritten by the stopped task (see `run_manager`);
//!   resume re-uses the run's own resume store, so the engine's change
//!   detection vouches for every reused row;
//! * row keys reach the UI only via `get_results` (on screen, never logged);
//!   the report export (T024) owns the separate include-keys gate.
//!
//! Credentials flow through [`crate::commands::gateway::resolve`], so a password
//! comes only from the keychain or a typed value — never from the plan or the
//! mapping (PLAN 6.5).

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tauri::State;

use dmux_engine::db::ReadOnlySession;
use dmux_engine::estimate::{estimate_from_seconds, table_size_bytes, timed_sample, Estimate};
use dmux_engine::eta::Remaining;
use dmux_engine::mapping::Mapping;
use dmux_engine::results::{CheckLevel, RunResults, Verdict};
use dmux_engine::run::{run, Input, RunConfig};

use crate::commands::gateway::{collect, drive, resolve, validate_mapping, SuperuserOverride};
use crate::error::{from_db_error, CommandError};
use crate::run_manager::{DriveTicket, RunHandle, RunManager, RunPhase};
use crate::state::AppState;
use crate::types::ConnectInput;

// ---------------------------------------------------------------------------
// Input DTOs.
// ---------------------------------------------------------------------------

/// The checks the user chose, as sent from the frontend `CheckPlan`.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct StartRunInput {
    pub old: ConnectInput,
    pub new: ConnectInput,
    /// The engine mapping (serde), produced earlier by `build_mapping`.
    pub mapping: Mapping,
    /// Per-table levels keyed by old `schema.table`; every in-scope table is
    /// listed here by the frontend (the default is only a fallback).
    #[serde(default)]
    pub levels: BTreeMap<String, CheckLevel>,
    /// The level for any table not named in `levels`.
    pub default_level: CheckLevel,
    /// Control-total column names (applied to every table that has them).
    #[serde(default)]
    pub control_total_columns: Vec<String>,
    /// Per-side parallel-scan maxima (PLAN 4.6); default 2 (old) / 4 (new).
    #[serde(default)]
    pub old_parallelism: Option<usize>,
    #[serde(default)]
    pub new_parallelism: Option<usize>,
    /// Surface row keys in results (on screen; the report gate is separate).
    #[serde(default)]
    pub export_keys: Option<bool>,
    pub allow_superuser: Option<SuperuserOverride>,
    /// An optional daily run window such as "22:00-05:00" (local time).  Off
    /// when absent.
    #[serde(default)]
    pub run_window: Option<String>,
    /// Old-side `schema.table` paths the user left out of this run on the
    /// Choose checks screen.  Each is recorded as an ignored item with
    /// [`NOT_SELECTED_REASON`], so it is never read, never silently dropped,
    /// and shows in the coverage line and on the first page of every report.
    #[serde(default)]
    pub skip_tables: Vec<String>,
}

/// The reason recorded for a table the user did not choose for a run.  Written
/// by the app, never by the webview, so a report reader can trust it.
pub use dmux_engine::run::NOT_SELECTED_REASON;

/// Add the user's left-out tables to the mapping as ignored items and re-hash
/// it.  A path must be a plain `schema.table`; one that names no table is
/// refused later by the engine's unknown-path check, never skipped silently.
/// A table the mapping already ignores keeps its own reason.
pub(crate) fn with_skipped_tables(
    mut mapping: Mapping,
    skip: &[String],
) -> Result<Mapping, CommandError> {
    if skip.is_empty() {
        return Ok(mapping);
    }
    for path in skip {
        let ok = path
            .split_once('.')
            .is_some_and(|(s, t)| !s.is_empty() && !t.is_empty() && !t.contains('.'));
        if !ok {
            return Err(CommandError::InvalidInput {
                message: "a table left out of the run must be named as schema.table".to_string(),
            });
        }
        if !mapping.ignored.iter().any(|i| &i.path == path) {
            mapping.ignored.push(dmux_engine::mapping::IgnoredItem {
                path: path.clone(),
                reason: NOT_SELECTED_REASON.to_string(),
            });
        }
    }
    validate_mapping(&mapping.with_fresh_hash())
}

// ---------------------------------------------------------------------------
// Output DTOs (the frontend contract in snake_case).
// ---------------------------------------------------------------------------

/// One table in the check plan, mirroring the contract's `TableCheck`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct TableCheckDto {
    pub table_id: String,
    pub schema_name: String,
    pub table_name: String,
    pub level: CheckLevel,
    /// Columns Dmux *suggests* as control totals.  Nothing is checked as a
    /// control total until the user confirms it (the UI sends only confirmed
    /// names to `start_run`).
    pub control_total_columns: Option<Vec<String>>,
    /// The table's size on the old database, in bytes (0 when unknown), so the
    /// UI can select tables by size.
    pub size_bytes: i64,
}

/// The check plan, mirroring the contract's `CheckPlan`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct CheckPlanDto {
    pub tables: Vec<TableCheckDto>,
    pub presets: Vec<String>,
    pub estimate_seconds_min: u64,
    pub estimate_seconds_max: u64,
}

/// What `start_run` returns: an app-level run id for progress/results polling.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct StartRunResult {
    pub run_id: String,
}

/// One table's progress row, mirroring the contract's `TableProgress`.
///
/// `done` here means exactly "the engine has stored a conclusive result for this
/// table in its resume store" — read back from the store, never fabricated.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct TableProgressDto {
    pub table_id: String,
    pub schema_name: String,
    pub table_name: String,
    pub state: String,
    pub progress_percent: u32,
    pub eta_seconds: Option<u64>,
}

/// The whole run's progress, mirroring the contract's `RunProgress`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct RunProgressDto {
    pub run_id: String,
    /// `running`, `paused`, `done`, `failed` or `cancelled`.
    pub overall_state: String,
    pub tables: Vec<TableProgressDto>,
    pub eta_seconds_min: Option<u64>,
    pub eta_seconds_max: Option<u64>,
    /// While the run waits for its run window: the time it opens ("22:00").
    pub waiting_for_window: Option<String>,
    /// Plain notices for the person watching (for example, sleep could not be
    /// blocked).
    pub notices: Vec<String>,
    /// The typed engine error when `overall_state` is `failed`.
    pub error: Option<CommandError>,
}

/// A drill-down row, mirroring the contract's `DrillDownRow` (keys only).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct DrillDownRowDto {
    pub key: String,
    pub kind: String,
    pub differing_columns: Option<Vec<String>>,
}

/// A per-column note (a ticked option or type-change flag), mirroring the
/// contract's `ColumnNote` (H2 / decision 23).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct ColumnNoteDto {
    pub column: String,
    pub kind: String,
    pub note: String,
}

/// A failure card, mirroring the contract's `FailureCard`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct FailureCardDto {
    pub id: String,
    pub table_id: String,
    pub schema_name: String,
    pub table_name: String,
    pub level: CheckLevel,
    pub verdict: String,
    pub summary: String,
    pub detail: String,
    /// Data differences, structure differences or other reasons
    /// (`dmux_engine::results::table_issue_kind`).
    pub kind: dmux_engine::results::IssueKind,
}

/// One table's result, mirroring the contract's `TableResult`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct TableResultDto {
    pub table_id: String,
    pub schema_name: String,
    pub table_name: String,
    pub verdict: String,
    pub level: CheckLevel,
    pub checked: bool,
    /// The kind of problem, or `None` for a PASS.
    pub kind: Option<dmux_engine::results::IssueKind>,
    pub drill_down: Option<Vec<DrillDownRowDto>>,
    pub column_notes: Vec<ColumnNoteDto>,
    /// Facts the Results detail panel explains a verdict from.  Everything
    /// here is a count, a column name or a plain-language rule, never a value.
    pub old_rows: Option<u64>,
    pub new_rows: Option<u64>,
    pub differing_buckets: Option<usize>,
    pub missing_buckets: Option<usize>,
    pub extra_buckets: Option<usize>,
    pub missing_rows: Option<usize>,
    pub extra_rows: Option<usize>,
    pub changed_rows: Option<usize>,
    /// The engine's own reasons this table got its verdict.
    pub reasons: Vec<String>,
    /// How the table was checked; absent for resumed or structure-only tables.
    pub facts: Option<dmux_engine::results::TableFacts>,
    /// Columns of this table that were ignored, each with its reason.
    pub ignored_columns: Vec<IgnoredDto>,
}

/// Ignored item (path + required reason).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct IgnoredDto {
    pub id: String,
    pub name: String,
    pub reason: String,
}

/// Results summary counts, mirroring the contract's `CoverageSummary`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct CoverageDto {
    pub total_tables: usize,
    pub full_values_tables: usize,
    pub counts_tables: usize,
    pub structure_tables: usize,
    pub ignored_tables: usize,
}

/// The results payload, mirroring the contract's `Results`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct ResultsDto {
    pub run_id: String,
    pub verdict: String,
    pub coverage: CoverageDto,
    pub ignored: Vec<IgnoredDto>,
    pub failures: Vec<FailureCardDto>,
    pub table_results: Vec<TableResultDto>,
    /// Whether any column option was applied (H2): drives "PASS (with options)".
    pub options_applied: bool,
    /// Whether the results file recorded options (schema >= 3); when false the
    /// UI says "options not recorded", never a bare PASS.
    pub options_recorded: bool,
    /// Run-level warnings (can-write, superuser, row-level security, ...).
    pub warnings: Vec<String>,
    /// What was checked, run-wide: the tool, the mapping, the digest and how
    /// each side was connected.  Never a password or a row value.
    pub tool_version: String,
    pub mapping_hash: String,
    pub results_sha256: String,
    pub connections: dmux_engine::results::Connections,
    /// Custom checks (Python scripts), present only when they ran for this run.
    /// The `verdict` above already includes any custom failure.
    pub custom_checks: Option<crate::hooks::CustomChecksDto>,
}

// ---------------------------------------------------------------------------
// build_check_plan
// ---------------------------------------------------------------------------

/// Suggest control-total columns by name (PLAN 4.4: number columns whose names
/// contain `amt` or `bal`).
fn is_control_total_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    ["amt", "bal", "amount", "balance", "total"]
        .iter()
        .any(|s| lower.contains(s))
}

/// Whether a column's type is a number (PLAN 4.4 suggests number columns only:
/// summing text or dates means nothing).
fn is_number_type(type_name: &str) -> bool {
    let t = type_name.trim().to_ascii_lowercase();
    [
        "numeric",
        "decimal",
        "integer",
        "int",
        "bigint",
        "smallint",
        "real",
        "double precision",
        "float",
        "money",
    ]
    .iter()
    .any(|p| t == *p || t.starts_with(&format!("{p}(")) || t.starts_with(&format!("{p} ")))
}

/// The old tables a run will compare, as `schema.table`: paired with a table in
/// the new database (through the mapping, exactly as the run pairs them) and not
/// ignored.  Only these are offered on Choose checks; the run still records the
/// others (not mapped, or ignored with their reason).
pub(crate) fn compared_old_tables(
    old: &[(String, String)],
    new_tables: &BTreeSet<String>,
    mapping: Option<&Mapping>,
) -> BTreeSet<String> {
    let ignored: BTreeSet<&str> = mapping
        .map(|m| {
            m.ignored
                .iter()
                .filter(|i| i.path.matches('.').count() == 1)
                .map(|i| i.path.as_str())
                .collect()
        })
        .unwrap_or_default();
    old.iter()
        .filter_map(|(schema, table)| {
            let path = format!("{schema}.{table}");
            if ignored.contains(path.as_str()) {
                return None;
            }
            let (ns, nt) = match mapping {
                Some(m) => dmux_engine::run::map_table(m, schema, table),
                None => (schema.clone(), table.clone()),
            };
            new_tables.contains(&format!("{ns}.{nt}")).then_some(path)
        })
        .collect()
}

/// Build a check plan from two live databases (PLAN 4.4-4.5).  Every level and
/// estimate comes from the engine's catalogue and estimate module; nothing is
/// guessed.
#[tauri::command]
pub async fn build_check_plan(
    old: ConnectInput,
    new: ConnectInput,
    mapping: Option<Mapping>,
    presets: Option<Vec<String>>,
    control_total_columns: Option<Vec<String>>,
    allow_superuser: Option<SuperuserOverride>,
    state: State<'_, AppState>,
) -> Result<CheckPlanDto, CommandError> {
    let allow = allow_superuser
        .as_ref()
        .is_some_and(SuperuserOverride::allows);
    // The mapping is untrusted: validated like an imported file, and used only
    // to pair tables exactly as the run does.  Levels and column types still
    // come from the live catalogue (PLAN 4.4).
    let mapping = match mapping {
        Some(m) => Some(validate_mapping(&m)?),
        None => None,
    };
    let control_totals: BTreeSet<String> = control_total_columns
        .unwrap_or_default()
        .into_iter()
        .collect();

    let old_inv = collect(&old, allow, &state).await?;
    // The new inventory is collected too: the pairs themselves are what the run
    // re-derives from the live catalogue, but a missing new table must still be
    // listed (the run records it as MissingOnNew, never silently dropped).
    let new_inv = collect(&new, allow, &state).await?;
    // Only tables the run will compare are offered.  An old table with no new
    // partner, or one the mapping ignores, has nothing to choose: the run still
    // records it (as not mapped, or ignored with its reason), so nothing is
    // dropped from the results by leaving it off this list.
    let new_tables: BTreeSet<String> = new_inv
        .schemas
        .iter()
        .flat_map(|s| {
            s.tables
                .iter()
                .map(move |t| format!("{}.{}", s.name, t.name))
        })
        .collect();
    let old_names: Vec<(String, String)> = old_inv
        .schemas
        .iter()
        .flat_map(|s| {
            s.tables
                .iter()
                .map(move |t| (s.name.clone(), t.name.clone()))
        })
        .collect();
    let compared = compared_old_tables(&old_names, &new_tables, mapping.as_ref());

    // Old tables (partitions already folded), mapped to their new side.
    let mut plan_tables: Vec<TableCheckDto> = Vec::new();
    let mut estimate_inputs: Vec<(String, String)> = Vec::new();
    for schema in &old_inv.schemas {
        for table in &schema.tables {
            let path = format!("{}.{}", schema.name, table.name);
            if !compared.contains(&path) {
                continue;
            }

            // Suggested control totals: the confirmed names this table has, or
            // the name heuristic when none are confirmed yet (PLAN 4.4).
            let controls: Vec<String> = if control_totals.is_empty() {
                table
                    .columns
                    .iter()
                    .filter(|c| is_control_total_name(&c.name) && is_number_type(&c.type_name))
                    .map(|c| c.name.clone())
                    .collect()
            } else {
                table
                    .columns
                    .iter()
                    .filter(|c| control_totals.contains(&c.name))
                    .map(|c| c.name.clone())
                    .collect()
            };

            plan_tables.push(TableCheckDto {
                table_id: path.clone(),
                schema_name: schema.name.clone(),
                table_name: table.name.clone(),
                level: CheckLevel::FullValues,
                control_total_columns: (!controls.is_empty()).then_some(controls),
                size_bytes: table.size_bytes.max(0),
            });
            estimate_inputs.push((schema.name.clone(), table.name.clone()));
        }
    }
    plan_tables.sort_by(|a, b| a.table_id.cmp(&b.table_id));

    let estimate = estimate_run(&old, &new, allow, &state, estimate_inputs).await?;

    Ok(CheckPlanDto {
        tables: plan_tables,
        presets: presets.unwrap_or_else(|| {
            vec![
                "full values everywhere".to_string(),
                "counts for all".to_string(),
            ]
        }),
        estimate_seconds_min: estimate.map(|e| e.low_seconds).unwrap_or(0),
        estimate_seconds_max: estimate.map(|e| e.high_seconds).unwrap_or(0),
    })
}

/// Measure a bytes/second rate on each side's largest data table and fold
/// per-table sizes into a total estimate (PLAN 4.5), using only the engine's
/// public `estimate` module.  Everything runs on the blocking pool because the
/// engine's transaction work is `!Send`.
async fn estimate_run(
    old: &ConnectInput,
    new: &ConnectInput,
    allow: bool,
    state: &AppState,
    tables: Vec<(String, String)>,
) -> Result<Option<Estimate>, CommandError> {
    if tables.is_empty() {
        return Ok(None);
    }
    const SAMPLE: u64 = 10_000;

    let old_profile = resolve(old, state)?;
    let new_profile = resolve(new, state)?;
    let tables = Arc::new(tables);

    drive(move || {
        let tables = Arc::clone(&tables);
        async move {
            let mut old_session = ReadOnlySession::open(&old_profile, allow)
                .await
                .map_err(from_db_error)?;
            let mut new_session = ReadOnlySession::open(&new_profile, allow)
                .await
                .map_err(from_db_error)?;

            // Size each table on disk (indexes + TOAST included), and remember
            // the largest so the timed sample lands on the heaviest work.
            let mut largest: Option<(String, String, i64)> = None;
            let mut total_size: i64 = 0;
            for (schema, table) in tables.iter() {
                let size = old_session
                    .with_read_only("10s", |txn| {
                        let schema = schema.clone();
                        let table = table.clone();
                        Box::pin(async move {
                            table_size_bytes(&*txn, &schema, &table).await.map_err(|e| {
                                dmux_engine::db::DbError::Postgres {
                                    message: e.to_string(),
                                }
                            })
                        })
                    })
                    .await
                    .map_err(from_db_error)?
                    .unwrap_or(0);
                let size = size.max(0);
                total_size += size;
                if size > largest.as_ref().map(|(_, _, s)| *s).unwrap_or(0) {
                    largest = Some((schema.clone(), table.clone(), size));
                }
            }

            if total_size <= 0 {
                return Ok(None);
            }
            let Some((schema, table, _size)) = largest else {
                return Ok(None);
            };

            // Time the bounded sample on the largest table (PLAN 4.5: a LIMIT
            // sample works on every Postgres version).  Both sides are sampled;
            // the slower side bounds the estimate.
            let new_schema = schema.clone();
            let new_table = table.clone();
            let old_took = old_session
                .with_read_only("60s", |txn| {
                    let schema = schema.clone();
                    let table = table.clone();
                    Box::pin(async move {
                        timed_sample(&*txn, &schema, &table, SAMPLE)
                            .await
                            .map_err(|e| dmux_engine::db::DbError::Postgres {
                                message: e.to_string(),
                            })
                    })
                })
                .await
                .map_err(from_db_error)?;
            let new_took = new_session
                .with_read_only("60s", |txn| {
                    let schema = new_schema.clone();
                    let table = new_table.clone();
                    Box::pin(async move {
                        timed_sample(&*txn, &schema, &table, SAMPLE)
                            .await
                            .map_err(|e| dmux_engine::db::DbError::Postgres {
                                message: e.to_string(),
                            })
                    })
                })
                .await
                .map_err(from_db_error)?;

            // Bytes per second = full table size / time to sample SAMPLE rows,
            // scaled; then a ± band.  This is an estimate (PLAN 4.5), corrected
            // by the engine as tables actually finish.
            let slowest = old_took.max(new_took).as_secs_f64().max(0.001);
            let rows_per_sec = SAMPLE as f64 / slowest;
            // Approximate row width from size vs the sample share: use a nominal
            // 100 bytes/row fallback so a tiny table never yields NaN.
            let approx_rows = (total_size as f64 / 100.0).max(1.0);
            let secs = approx_rows / rows_per_sec.max(0.01);
            Ok(Some(estimate_from_seconds(secs)))
        }
    })
    .await
}

// ---------------------------------------------------------------------------
// start_run / poll_run / pause / cancel / results.
// ---------------------------------------------------------------------------

/// Spawn the engine run on a background task, persisting per-table results to the
/// run's own resume store under the app data dir (PLAN 4.6).  Credentials are
/// resolved once here, through the trusted `resolve` path, and never stored.
#[tauri::command]
pub async fn start_run(
    input: StartRunInput,
    state: State<'_, AppState>,
) -> Result<StartRunResult, CommandError> {
    start_run_with(input, &state)
}

/// [`start_run`] over a plain `&AppState` (so the lifecycle is testable).
pub(crate) fn start_run_with(
    input: StartRunInput,
    state: &AppState,
) -> Result<StartRunResult, CommandError> {
    let allow = input
        .allow_superuser
        .as_ref()
        .is_some_and(SuperuserOverride::allows);
    // The recorded reason behind a valid superuser override, threaded through
    // the run so the engine records it in the results `connections` block
    // (audit M5).  The plaintext reason is already carried by the resolved
    // profile's TLS mode (TlsMode::Disable(reason)).
    let allow_reason = input
        .allow_superuser
        .as_ref()
        .and_then(SuperuserOverride::reason)
        .map(str::to_string);
    // The webview-supplied mapping is untrusted input (audit L7): validate it
    // exactly like an imported mapping file before it drives the run.
    let mapping = with_skipped_tables(validate_mapping(&input.mapping)?, &input.skip_tables)?;
    // A table left out of the run takes no level.
    let mut levels = input.levels.clone();
    levels.retain(|t, _| !input.skip_tables.contains(t));
    let old_profile = resolve(&input.old, state)?;
    let new_profile = resolve(&input.new, state)?;
    // The request's limits, else the ones saved in Settings (2 and 4 by default).
    let prefs = crate::prefs::Prefs::load(&*state.store()?)?;
    let (old_parallelism, new_parallelism) =
        crate::prefs::effective_parallelism(input.old_parallelism, input.new_parallelism, &prefs);
    if old_parallelism == 0 || new_parallelism == 0 {
        return Err(CommandError::InvalidInput {
            message: "the parallel-scan limits must be at least 1".to_string(),
        });
    }
    let export_keys = input.export_keys.unwrap_or(true);
    let run_window = match input.run_window.as_deref().map(str::trim) {
        None | Some("") => None,
        Some(text) => Some(
            dmux_engine::run_window::RunWindow::parse(text).map_err(|e| {
                CommandError::InvalidInput {
                    message: e.to_string(),
                }
            })?,
        ),
    };

    let run_id = RunManager::fresh_id();
    let resume_store = crate::state::resume_store_path(state, &run_id)?;
    state.note_run_start(&run_id, &input.old.profile.label, &input.new.profile.label);

    let config = RunConfig {
        mapping,
        default_level: input.default_level,
        levels,
        control_total_columns: input.control_total_columns.into_iter().collect(),
        old_parallelism,
        new_parallelism,
        allow_superuser: allow,
        allow_superuser_reason: allow_reason,
        export_keys,
        resume_store: Some(resume_store.clone()),
        run_window,
        // Tables were left out: the verdict is about the chosen ones (decision 30).
        only_chosen_tables: !input.skip_tables.is_empty(),
        ..RunConfig::new(Input::Live(old_profile), Input::Live(new_profile))
    };

    let manager = Arc::clone(&state.runs);
    manager.insert(&run_id, RunHandle::new(config, Some(resume_store)));
    manager.set_hook_plan(&run_id, crate::hooks::plan_for_new_run(state)?);
    let ticket = manager
        .begin_first(&run_id)
        .ok_or_else(|| CommandError::internal("the new run could not be started"))?;
    spawn_drive_task(&manager, &run_id, ticket, Arc::clone(&state.sleep));

    Ok(StartRunResult { run_id })
}

/// Spawn one drive attempt on the blocking pool.  The engine's run future is
/// `!Send` (it borrows a `tokio_postgres::Transaction`), so it runs inside
/// `spawn_blocking` + `block_on` on a dedicated OS thread.  The task reports
/// the engine's outcome through [`RunManager::finish`], which applies it only
/// to its own attempt and only while the run is still running.
///
/// While the engine works the computer is kept awake (`sleep`); the hold is
/// released when the engine returns (done, failed, cancelled or paused).  If
/// the hold cannot be taken, the run goes on and says so.
fn spawn_drive_task(
    manager: &Arc<RunManager>,
    run_id: &str,
    ticket: DriveTicket,
    sleep: Arc<dyn crate::sleep::SleepInhibitor>,
) {
    let generation = ticket.generation;
    let task = tauri::async_runtime::spawn_blocking({
        let manager = Arc::clone(manager);
        let rid = run_id.to_string();
        move || {
            let config = ticket.config;
            let hold = sleep.acquire();
            let blocked = hold.is_err();
            if blocked {
                if let Some(c) = &config.control {
                    c.with_live(|l| l.add_notice(crate::sleep::SLEEP_WARNING));
                }
            }
            let out = tauri::async_runtime::block_on(async { run(&config).await }).map(|o| {
                let mut results = o.results;
                if blocked
                    && !results
                        .warnings
                        .iter()
                        .any(|w| w == crate::sleep::SLEEP_WARNING)
                {
                    results
                        .warnings
                        .push(crate::sleep::SLEEP_WARNING.to_string());
                }
                results
            });
            // Custom checks run here, on this blocking thread, before the run
            // is reported finished (so results never appear without them).
            crate::hooks::after_run(&manager, &rid, generation, &out);
            // Release the hold before reporting: the run is no longer working.
            drop(hold);
            manager.finish(&rid, generation, out);
        }
    });
    manager.set_task(run_id, generation, task);
}

/// How often, and for how long, a stop re-sends its server-side cancels.  A
/// statement the engine started just before the stop flag was set may reach
/// the server after the first cancel; repeating until the attempt's drive task
/// returns closes that window.
const CANCEL_REPEAT: std::time::Duration = std::time::Duration::from_millis(250);
const CANCEL_PATIENCE: std::time::Duration = std::time::Duration::from_secs(15);

/// Stop the run's current attempt: set the engine's stop flag and cancel every
/// registered session's statement on the server, then keep cancelling in the
/// background until that attempt's drive task has returned.
async fn stop_attempt(manager: &Arc<RunManager>, run_id: &str) {
    let Some((generation, control)) = manager.stop(run_id) else {
        return;
    };
    for token in control.tokens() {
        // A failed cancel request is not fatal: the stop flag still ends the
        // run at its next check, and the background loop retries.
        let _ = token.cancel().await;
    }
    let manager = Arc::clone(manager);
    let rid = run_id.to_string();
    tauri::async_runtime::spawn(async move {
        let started = std::time::Instant::now();
        while started.elapsed() < CANCEL_PATIENCE && !manager.drive_returned(&rid, generation) {
            tokio::time::sleep(CANCEL_REPEAT).await;
            if manager.drive_returned(&rid, generation) {
                break;
            }
            for token in control.tokens() {
                let _ = token.cancel().await;
            }
        }
    });
}

/// Report honest progress: the set of tables the engine has really finished (from
/// its resume store) plus the run's lifecycle phase, and the typed error of a
/// failed run.
#[tauri::command]
pub async fn poll_run(
    run_id: String,
    state: State<'_, AppState>,
) -> Result<RunProgressDto, CommandError> {
    poll_run_with(run_id, &state)
}

pub(crate) fn poll_run_with(
    run_id: String,
    state: &AppState,
) -> Result<RunProgressDto, CommandError> {
    let manager = Arc::clone(&state.runs);
    let Some((config, resume)) = manager.snapshot(&run_id) else {
        return Err(CommandError::RunNotFound {
            run_id: run_id.clone(),
        });
    };
    let Some(phase) = manager.phase(&run_id) else {
        return Err(CommandError::RunNotFound {
            run_id: run_id.clone(),
        });
    };

    // The engine's resume store is the only source of per-table completion.
    let finished: BTreeSet<String> = match &resume {
        Some(path) => dmux_engine::store::Store::open(path)
            .map_err(|e| CommandError::Store {
                message: e.to_string(),
            })?
            .finished_tables()
            .map_err(|e| CommandError::Store {
                message: e.to_string(),
            })?
            .into_iter()
            .collect(),
        None => BTreeSet::new(),
    };

    // Live numbers from the engine's current attempt: the plan's estimates,
    // corrected by the tables measured so far (PLAN 4.5), the table being
    // scanned, and whether the run waits for its window.  Nothing is shown for
    // a run that is not working.
    let working = phase == RunPhase::Running;
    let (remaining, running_table, waiting_for_window, notices) =
        match manager.control(&run_id).filter(|_| working) {
            Some(c) => c.with_live(|l| {
                (
                    l.remaining(&finished, std::time::Instant::now()),
                    l.current_table().map(str::to_string),
                    l.window_opens_at().map(str::to_string),
                    l.notices().to_vec(),
                )
            }),
            None => (Remaining::default(), None, None, Vec::new()),
        };

    let mut tables: Vec<TableProgressDto> = Vec::new();
    for path in config.levels.keys() {
        let (schema, table) = split_path(path);
        let done = finished.contains(path);
        // Once the run has ended, a table with no saved result was never
        // scanned (no counterpart in the new database, or the run stopped
        // first); "queued" would claim it is still to come.  Its verdict, if
        // any, is on the results.
        let ended = matches!(
            phase,
            RunPhase::Done | RunPhase::Failed | RunPhase::Cancelled
        );
        let state = if done {
            "done"
        } else if working && running_table.as_deref() == Some(path.as_str()) {
            "running"
        } else if ended {
            "not-scanned"
        } else {
            "queued"
        };
        tables.push(TableProgressDto {
            table_id: path.clone(),
            schema_name: schema,
            table_name: table,
            state: state.to_string(),
            progress_percent: if done { 100 } else { 0 },
            // The middle of the table's range, only for the table being scanned.
            eta_seconds: remaining
                .tables
                .get(path)
                .filter(|_| state == "running")
                .map(|(lo, hi)| lo + (hi - lo) / 2),
        });
    }
    tables.sort_by(|a, b| a.table_id.cmp(&b.table_id));

    Ok(RunProgressDto {
        run_id: run_id.clone(),
        overall_state: phase_string(phase).to_string(),
        tables,
        eta_seconds_min: remaining.total.map(|t| t.0),
        eta_seconds_max: remaining.total.map(|t| t.1),
        waiting_for_window,
        notices,
        error: manager.error(&run_id),
    })
}

/// The `overall_state` string of a phase.
fn phase_string(phase: RunPhase) -> &'static str {
    match phase {
        RunPhase::Running => "running",
        RunPhase::Paused => "paused",
        RunPhase::Done => "done",
        RunPhase::Failed => "failed",
        RunPhase::Cancelled => "cancelled",
    }
}

/// Pause (stop the engine and its server queries; completed tables stay in
/// the run's resume store) or resume (wait for the stopped attempt to return,
/// then run again against the same store).
#[tauri::command]
pub async fn set_run_paused(
    run_id: String,
    paused: bool,
    state: State<'_, AppState>,
) -> Result<RunProgressDto, CommandError> {
    set_run_paused_with(run_id, paused, &state).await
}

pub(crate) async fn set_run_paused_with(
    run_id: String,
    paused: bool,
    state: &AppState,
) -> Result<RunProgressDto, CommandError> {
    let manager = Arc::clone(&state.runs);
    if paused {
        match manager.transition(&run_id, &[RunPhase::Running], RunPhase::Paused) {
            Ok(_) => stop_attempt(&manager, &run_id).await,
            Err(None) => {
                return Err(CommandError::RunNotFound {
                    run_id: run_id.clone(),
                })
            }
            Err(Some(RunPhase::Paused)) => {}
            Err(Some(phase)) => {
                return Err(CommandError::RunState {
                    message: format!("a {} run cannot be paused", phase_string(phase)),
                })
            }
        }
    } else {
        // One resume at a time; the previous attempt is joined before the next
        // starts, so there are never two drive tasks on one run.
        let (ticket, previous) = manager.begin_resume(&run_id)?;
        if let Some(task) = previous {
            let _ = task.await;
        }
        if manager.end_resume(&run_id, ticket.generation) {
            spawn_drive_task(&manager, &run_id, ticket, Arc::clone(&state.sleep));
        }
    }
    poll_run_with(run_id, state)
}

/// Cancel a run: stop the engine and cancel its server queries (PLAN 4.6's
/// "Cancel really stops").  The phase becomes `Cancelled` for good.
#[tauri::command]
pub async fn cancel_run(
    run_id: String,
    state: State<'_, AppState>,
) -> Result<RunProgressDto, CommandError> {
    cancel_run_with(run_id, &state).await
}

pub(crate) async fn cancel_run_with(
    run_id: String,
    state: &AppState,
) -> Result<RunProgressDto, CommandError> {
    let manager = Arc::clone(&state.runs);
    match manager.transition(
        &run_id,
        &[RunPhase::Running, RunPhase::Paused],
        RunPhase::Cancelled,
    ) {
        Ok(_) => stop_attempt(&manager, &run_id).await,
        Err(None) => {
            return Err(CommandError::RunNotFound {
                run_id: run_id.clone(),
            })
        }
        Err(Some(RunPhase::Cancelled)) => {}
        Err(Some(phase)) => {
            return Err(CommandError::RunState {
                message: format!("a {} run cannot be cancelled", phase_string(phase)),
            })
        }
    }
    poll_run_with(run_id, state)
}

/// Return the engine's results for a finished run, mapped to the contract.
/// Refused unless the run is `Done` and the engine returned results.
#[tauri::command]
pub async fn get_results(
    run_id: String,
    state: State<'_, AppState>,
) -> Result<ResultsDto, CommandError> {
    get_results_with(&run_id, &state)
}

pub(crate) fn get_results_with(run_id: &str, state: &AppState) -> Result<ResultsDto, CommandError> {
    if state.runs.phase(run_id).is_none() {
        if let Some(opened) = state.opened_results(run_id)? {
            return Ok(map_opened(run_id, &opened));
        }
    }
    let results = state.runs.results(run_id)?;
    // Custom checks, when they ran: the folded results, the verdict that
    // includes any custom failure, and the findings themselves.
    match state.runs.hook_run(run_id)? {
        None => Ok(map_results(run_id, &results)),
        Some(hooks) => {
            let mut dto = map_results(run_id, &hooks.results);
            dto.verdict = verdict_string(hooks.displayed_verdict());
            dto.custom_checks = Some(crate::hooks::CustomChecksDto::from_run(&hooks));
            Ok(dto)
        }
    }
}

/// Map verified results that were opened from history or a file.  For a hook
/// envelope the overall verdict is the worse of the results verdict and every
/// custom-check finding, and each finding is listed as a warning, so a failed
/// custom check can never show as PASS.
pub(crate) fn map_opened(run_id: &str, opened: &crate::history::Opened) -> ResultsDto {
    let mut dto = map_results(run_id, &opened.results);
    if let Some(hooks) = &opened.hooks {
        dto.verdict = verdict_string(opened.displayed_verdict());
        for f in &hooks.custom_checks {
            dto.warnings.push(format!(
                "Custom check ({}): {}{}",
                f.severity,
                f.message,
                f.table
                    .as_deref()
                    .map(|t| format!(" [{t}]"))
                    .unwrap_or_default()
            ));
        }
        dto.warnings.extend(hooks.warnings.iter().cloned());
    }
    dto
}

// ---------------------------------------------------------------------------
// Mapping helpers (engine -> contract).
// ---------------------------------------------------------------------------

fn split_path(path: &str) -> (String, String) {
    match path.split_once('.') {
        Some((schema, table)) => (schema.to_string(), table.to_string()),
        None => (String::new(), path.to_string()),
    }
}

/// Map the engine's `RunResults` onto the contract `Results`.  Verdicts are
/// copied verbatim; anything the engine did not emit is never invented here.
fn map_results(run_id: &str, results: &RunResults) -> ResultsDto {
    let mut failures: Vec<FailureCardDto> = Vec::new();
    let mut table_results: Vec<TableResultDto> = Vec::new();

    for (path, t) in &results.tables {
        let (schema, table) = split_path(path);
        let verdict = verdict_string(t.verdict);
        let checked = matches!(
            t.verdict,
            Verdict::Pass
                | Verdict::Fail
                | Verdict::WidelyDifferent
                | Verdict::NeedsReview
                | Verdict::ChangedDuringCheck
        );

        let kind = dmux_engine::results::table_issue_kind(path, t, results);
        if let Some(kind) = kind {
            failures.push(FailureCardDto {
                id: format!("fail-{path}"),
                table_id: path.clone(),
                schema_name: schema.clone(),
                table_name: table.clone(),
                level: t.level,
                verdict: verdict.clone(),
                summary: summary_for(t),
                detail: detail_for(t),
                kind,
            });
        }

        table_results.push(TableResultDto {
            table_id: path.clone(),
            schema_name: schema,
            table_name: table,
            verdict,
            level: t.level,
            checked,
            kind,
            drill_down: drill_down_for(path, results),
            column_notes: t
                .column_notes
                .iter()
                .map(|n| ColumnNoteDto {
                    column: n.column.clone(),
                    kind: n.kind.clone(),
                    note: n.note.clone(),
                })
                .collect(),
            old_rows: t.old_rows,
            new_rows: t.new_rows,
            differing_buckets: t.differing_buckets,
            missing_buckets: t.missing_buckets,
            extra_buckets: t.extra_buckets,
            missing_rows: t.missing_rows,
            extra_rows: t.extra_rows,
            changed_rows: t.changed_rows,
            reasons: t.reasons.clone(),
            facts: results.table_facts.get(path).cloned(),
            ignored_columns: results
                .ignored_items
                .iter()
                .filter(|i| i.path.starts_with(&format!("{path}.")))
                .map(|i| IgnoredDto {
                    id: i.path.clone(),
                    name: i.path.clone(),
                    reason: i.reason.clone(),
                })
                .collect(),
        });
    }

    let ignored: Vec<IgnoredDto> = results
        .ignored_items
        .iter()
        .map(|i| IgnoredDto {
            id: i.path.clone(),
            name: i.path.clone(),
            reason: i.reason.clone(),
        })
        .collect();

    // Structure failures without a table row still surface (a structural FAIL
    // can never hide behind an absent table).
    for f in &results.structure_findings {
        if f.verdict == Verdict::Pass {
            continue;
        }
        let (schema, table) = split_path(&f.object);
        failures.push(FailureCardDto {
            id: format!("struct-{}", f.object),
            table_id: f.object.clone(),
            schema_name: schema,
            table_name: table,
            level: CheckLevel::Structure,
            verdict: verdict_string(f.verdict),
            summary: f.message.clone(),
            detail: "an inventory object differs between the two databases".to_string(),
            kind: dmux_engine::results::IssueKind::Structure,
        });
    }

    failures.sort_by(|a, b| a.table_id.cmp(&b.table_id));
    table_results.sort_by(|a, b| a.table_id.cmp(&b.table_id));

    ResultsDto {
        run_id: run_id.to_string(),
        verdict: verdict_string(results.verdict),
        coverage: CoverageDto {
            total_tables: results.in_scope,
            full_values_tables: results.fully_checked,
            counts_tables: results.counts_only,
            structure_tables: results.structure_only,
            ignored_tables: results.ignored,
        },
        ignored,
        failures,
        table_results,
        options_applied: results.options_applied,
        options_recorded: dmux_engine::results::options_recorded(results),
        warnings: results.warnings.clone(),
        tool_version: results.tool_version.clone(),
        mapping_hash: results.mapping_hash.clone(),
        results_sha256: results.results_sha256.clone(),
        connections: results.connections.clone(),
        custom_checks: None,
    }
}

/// The engine verdict string, verbatim.  The vocabulary is fixed; any value the
/// frontend cannot recognise is demoted to TOOL_FAULT (non-PASS) there.
fn verdict_string(v: Verdict) -> String {
    match v {
        Verdict::Pass => "PASS",
        Verdict::Fail => "FAIL",
        Verdict::WidelyDifferent => "WIDELY_DIFFERENT",
        Verdict::NeedsReview => "NEEDS_REVIEW",
        Verdict::ChangedDuringCheck => "CHANGED_DURING_CHECK",
        Verdict::ToolFault => "TOOL_FAULT",
        Verdict::Warning => "WARNING",
    }
    .to_string()
}

fn summary_for(t: &dmux_engine::results::TableResult) -> String {
    if let Some(first) = t.reasons.first() {
        return first.clone();
    }
    match (t.old_rows, t.new_rows) {
        (Some(o), Some(n)) => format!("{n} rows in the new database vs {o} in the old"),
        _ => "the table did not pass the comparison".to_string(),
    }
}

fn detail_for(t: &dmux_engine::results::TableResult) -> String {
    t.reasons.join("; ")
}

fn drill_down_for(path: &str, results: &RunResults) -> Option<Vec<DrillDownRowDto>> {
    let detail = results.details.get(path)?;
    let mut rows: Vec<DrillDownRowDto> = Vec::new();

    if let Some(keys) = &detail.missing_keys {
        for key in keys {
            rows.push(DrillDownRowDto {
                key: render_key(key),
                kind: "missing".to_string(),
                differing_columns: None,
            });
        }
    }
    if let Some(keys) = &detail.extra_keys {
        for key in keys {
            rows.push(DrillDownRowDto {
                key: render_key(key),
                kind: "extra".to_string(),
                differing_columns: None,
            });
        }
    }
    for changed in &detail.changed_rows {
        rows.push(DrillDownRowDto {
            key: changed
                .key
                .as_ref()
                .map(|k| render_key(k))
                .unwrap_or_default(),
            kind: "changed".to_string(),
            differing_columns: Some(changed.columns.clone()),
        });
    }

    if rows.is_empty() {
        None
    } else {
        Some(rows)
    }
}

fn render_key(key: &[Option<String>]) -> String {
    key.iter()
        .map(|part| part.as_deref().unwrap_or(""))
        .collect::<Vec<_>>()
        .join(" / ")
}

// ---------------------------------------------------------------------------
// Tests.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use dmux_engine::db::{ConnProfile, TlsMode};

    /// Read a gated mock URL, failing under CI when unset (the live data-half
    /// tests MUST run in CI against the demo_bank / bank mocks).
    fn env_url(name: &str) -> Option<String> {
        match std::env::var(name) {
            Ok(u) => Some(u),
            Err(_) => {
                assert!(
                    std::env::var("CI").as_deref() != Ok("true"),
                    "CI=true but {name} is not set: the live data-half tests must run in CI"
                );
                eprintln!("skipped: {name} not set");
                None
            }
        }
    }

    /// Read the URL of an owner-only database (edge_bank): CI never has one,
    /// so an unset URL always skips, in CI too.  Mandatory CI databases use
    /// [`env_url`], which fails under CI instead.
    fn owner_url(name: &str) -> Option<String> {
        match std::env::var(name) {
            Ok(u) => Some(u),
            Err(_) => {
                eprintln!("skipped: {name} not set (owner-only check)");
                None
            }
        }
    }

    fn profile_from_url(url: &str) -> ConnProfile {
        let url = url
            .strip_prefix("postgres://")
            .expect("url must be postgres");
        let (creds, rest) = url
            .split_once('@')
            .expect("url must contain user:pass@host");
        let (user, password) = creds.split_once(':').expect("url must contain user:pass");
        let (host_port, dbname) = rest.split_once('/').unwrap_or((rest, ""));
        let (host, port_str) = host_port
            .split_once(':')
            .expect("url must contain host:port");
        let port = port_str.parse().expect("port must be numeric");
        ConnProfile::new(
            host,
            port,
            dbname,
            user,
            password,
            TlsMode::Disable("mock database has no TLS".to_string()),
        )
    }

    /// Run the engine live→live against the two URLs with an identity mapping at
    /// full values (keys exported so the on-screen drill-down is reachable).
    async fn run_live(old_url: &str, new_url: &str) -> RunResults {
        let config = RunConfig {
            allow_superuser: true,
            export_keys: true,
            ..RunConfig::new(
                Input::Live(profile_from_url(old_url)),
                Input::Live(profile_from_url(new_url)),
            )
        };
        run(&config)
            .await
            .expect("the live run must complete")
            .results
    }

    // --- Pure mapping tests (no server) ---

    #[test]
    fn only_number_types_are_suggested_as_control_totals() {
        for t in [
            "numeric(18,2)",
            "numeric",
            "bigint",
            "integer",
            "double precision",
            "money",
            "real",
        ] {
            assert!(is_number_type(t), "{t}");
        }
        for t in [
            "text",
            "character varying(20)",
            "date",
            "timestamp without time zone",
            "interval",
            "bytea",
        ] {
            assert!(!is_number_type(t), "{t}");
        }
    }

    #[test]
    fn control_total_names_are_recognised() {
        assert!(is_control_total_name("amount"));
        assert!(is_control_total_name("balance"));
        assert!(is_control_total_name("running_amt"));
        assert!(is_control_total_name("TOTAL"));
        assert!(!is_control_total_name("name"));
        assert!(!is_control_total_name("id"));
    }

    #[test]
    fn verdict_strings_are_the_engine_vocabulary() {
        assert_eq!(verdict_string(Verdict::Pass), "PASS");
        assert_eq!(
            verdict_string(Verdict::ChangedDuringCheck),
            "CHANGED_DURING_CHECK"
        );
        assert_eq!(verdict_string(Verdict::ToolFault), "TOOL_FAULT");
    }

    #[test]
    fn key_rendering_never_invents_values() {
        assert_eq!(render_key(&[Some("1001".to_string())]), "1001");
        assert_eq!(
            render_key(&[Some("a".to_string()), Some("b".to_string())]),
            "a / b"
        );
        assert_eq!(render_key(&[None, Some("b".to_string())]), " / b");
    }

    #[test]
    fn split_path_handles_plain_and_empty() {
        assert_eq!(split_path("s.t"), ("s".to_string(), "t".to_string()));
        assert_eq!(split_path("t"), ("".to_string(), "t".to_string()));
    }

    /// Every failure card and table row says what kind of problem it is, so
    /// the Results screen can put data differences first.
    #[test]
    fn map_results_tags_each_failure_with_its_kind() {
        use dmux_engine::results::{IssueKind, StructureFinding, TableResult};
        let row = |name: &str, verdict: Verdict, changed: usize| TableResult {
            table: name.into(),
            level: CheckLevel::FullValues,
            verdict,
            old_rows: Some(10),
            new_rows: Some(10),
            columns_covered: None,
            differing_buckets: Some(changed),
            missing_buckets: Some(0),
            extra_buckets: Some(0),
            missing_rows: Some(0),
            extra_rows: Some(0),
            changed_rows: Some(changed),
            reasons: vec![],
            column_notes: vec![],
        };
        let r = RunResults {
            tables: BTreeMap::from([
                ("s.data".into(), row("s.data", Verdict::Fail, 3)),
                ("s.cols".into(), row("s.cols", Verdict::Fail, 0)),
                ("s.ok".into(), row("s.ok", Verdict::Pass, 0)),
            ]),
            structure_findings: vec![StructureFinding {
                object: "s.cols.legacy_flag".into(),
                path: vec![],
                verdict: Verdict::Fail,
                message: "column missing in new: legacy_flag".into(),
            }],
            ..RunResults::default()
        };
        let dto = map_results("run-k", &r);
        let card = |id: &str| dto.failures.iter().find(|f| f.id == id).unwrap().kind;
        assert_eq!(card("fail-s.data"), IssueKind::Data);
        assert_eq!(card("fail-s.cols"), IssueKind::Structure);
        assert_eq!(card("struct-s.cols.legacy_flag"), IssueKind::Structure);
        let row_kind = |id: &str| {
            dto.table_results
                .iter()
                .find(|t| t.table_id == id)
                .unwrap()
                .kind
        };
        assert_eq!(row_kind("s.ok"), None);
        assert_eq!(row_kind("s.data"), Some(IssueKind::Data));
        let json = serde_json::to_string(&dto.failures[0]).unwrap();
        assert!(json.contains("\"kind\":\""), "{json}");
    }

    /// The facts the Results detail panel explains a table from reach the DTO
    /// unchanged, and carry counts, names and rules only.
    #[test]
    fn map_results_carries_the_recorded_facts() {
        use dmux_engine::results::{
            ColumnRule, Connections, ControlTotalFact, IgnoredItem, SpotFact, TableFacts,
            TableResult,
        };
        let mut r = RunResults {
            tool_version: "9.9.9".into(),
            mapping_hash: "map".into(),
            results_sha256: "sha".into(),
            connections: Connections::default(),
            ..RunResults::default()
        };
        r.connections.old.superuser = true;
        r.connections.old.superuser_reason = Some("rehearsal".into());
        r.tables.insert(
            "bank.accounts".into(),
            TableResult {
                table: "bank.accounts".into(),
                level: CheckLevel::FullValues,
                verdict: Verdict::Pass,
                old_rows: Some(10),
                new_rows: Some(10),
                columns_covered: None,
                differing_buckets: Some(0),
                missing_buckets: Some(0),
                extra_buckets: Some(0),
                missing_rows: None,
                extra_rows: None,
                changed_rows: None,
                reasons: vec!["spot check: 2 rows compared".into()],
                column_notes: vec![],
            },
        );
        r.table_facts.insert(
            "bank.accounts".into(),
            TableFacts {
                key_columns: vec!["id".into()],
                columns: vec![ColumnRule {
                    column: "id".into(),
                    rule: "compared as an exact whole number".into(),
                }],
                buckets_total: Some(256),
                spot: Some(SpotFact {
                    state: "agreed".into(),
                    compared: Some(2),
                    differing: Some(0),
                    note: String::new(),
                }),
                change_detection_old: Some("available".into()),
                change_detection_new: Some("available".into()),
                changed_during_scan: Some(false),
                control_totals: vec![ControlTotalFact {
                    column: "amount".into(),
                    old: "1.50".into(),
                    new: "1.50".into(),
                }],
            },
        );
        r.ignored_items.push(IgnoredItem {
            path: "bank.accounts.note".into(),
            reason: "free text".into(),
        });
        r.ignored_items.push(IgnoredItem {
            path: "bank.accounts_archive.note".into(),
            reason: "another table".into(),
        });
        let dto = map_results("run-x", &r);
        let t = &dto.table_results[0];
        assert_eq!((t.old_rows, t.new_rows), (Some(10), Some(10)));
        assert_eq!(t.differing_buckets, Some(0));
        assert_eq!(t.reasons, vec!["spot check: 2 rows compared".to_string()]);
        let f = t.facts.as_ref().expect("facts are mapped");
        assert_eq!(f.buckets_total, Some(256));
        assert_eq!(f.spot.as_ref().unwrap().state, "agreed");
        assert_eq!(f.control_totals[0].column, "amount");
        // Only this table's ignored columns, not a table whose name starts the same.
        assert_eq!(t.ignored_columns.len(), 1);
        assert_eq!(t.ignored_columns[0].reason, "free text");
        assert_eq!(dto.tool_version, "9.9.9");
        assert_eq!(dto.mapping_hash, "map");
        assert_eq!(dto.results_sha256, "sha");
        assert_eq!(
            dto.connections.old.superuser_reason.as_deref(),
            Some("rehearsal")
        );
        // The wire form uses the snake_case names the UI reads, and nothing else.
        let v = serde_json::to_value(&dto).unwrap();
        assert!(v["table_results"][0]["facts"]["key_columns"].is_array());
        assert!(v["connections"]["old"]["superuser"].as_bool().unwrap());
    }

    #[test]
    fn map_results_has_no_facts_for_a_table_without_them() {
        let mut r = RunResults::default();
        r.tables.insert(
            "s.t".into(),
            dmux_engine::results::TableResult {
                table: "s.t".into(),
                level: CheckLevel::Structure,
                verdict: Verdict::Pass,
                old_rows: None,
                new_rows: None,
                columns_covered: None,
                differing_buckets: None,
                missing_buckets: None,
                extra_buckets: None,
                missing_rows: None,
                extra_rows: None,
                changed_rows: None,
                reasons: vec![],
                column_notes: vec![],
            },
        );
        let dto = map_results("run-x", &r);
        assert!(dto.table_results[0].facts.is_none());
        assert!(dto.table_results[0].ignored_columns.is_empty());
    }

    // --- Live tests (gated on the mock DBs; FAIL under CI when unset) ---

    /// A planted demo_bank run must surface FAIL, with the answer key's m01
    /// twelve missing keys reachable through the drill-down — never a PASS.
    #[tokio::test(flavor = "current_thread")]
    async fn planted_demo_bank_surfaces_fail_with_m01_missing_keys() {
        let Some(old) = env_url("DMUX_TEST_DEMO_OLD_URL") else {
            return;
        };
        let Some(new) = env_url("DMUX_TEST_DEMO_NEW_URL") else {
            return;
        };

        let results = run_live(&old, &new).await;
        let dto = map_results("run-test", &results);

        // The overall verdict is FAIL, not a fabricated PASS.
        assert_eq!(dto.verdict, "FAIL", "planted differences must fail the run");

        let txn = dto
            .table_results
            .iter()
            .find(|t| t.table_id == "demo_bank.transactions")
            .expect("transactions must be reported");
        assert_ne!(
            txn.verdict, "PASS",
            "a table with 12 missing rows must not be PASS"
        );

        // The twelve missing keys (m01) are reachable on screen via drill-down.
        let drill = txn
            .drill_down
            .as_ref()
            .expect("the transactions drill-down must be present");
        let missing_keys: Vec<&str> = drill
            .iter()
            .filter(|r| r.kind == "missing")
            .map(|r| r.key.as_str())
            .collect();
        assert_eq!(
            missing_keys.len(),
            12,
            "m01 plants exactly 12 missing rows; got {missing_keys:?}"
        );
        // Spot-check a couple of the answer key's ids.
        assert!(missing_keys.contains(&"137"), "key 137 must be listed");
        assert!(missing_keys.contains(&"19888"), "key 19888 must be listed");
    }

    /// An identical-data run (bank) shows the engine's own verdict — verbatim,
    /// never a verdict the adapter invented.  A false PASS is ruled out
    /// structurally: whatever the engine emits reaches the UI unchanged.
    #[tokio::test(flavor = "current_thread")]
    async fn identical_bank_shows_the_engine_verdict() {
        let Some(old) = env_url("DMUX_TEST_OLD_URL") else {
            return;
        };
        let Some(new) = env_url("DMUX_TEST_NEW_URL") else {
            return;
        };

        let results = run_live(&old, &new).await;
        let dto = map_results("run-test", &results);

        // The adapter only mirrors the engine's verdict string 1:1.
        assert_eq!(dto.verdict, verdict_string(results.verdict));

        // Every reported table verdict round-trips through the same 1:1 mapping,
        // so no table can be shown as PASS unless the engine said PASS.
        for (path, t) in &results.tables {
            let mirrored = dto
                .table_results
                .iter()
                .find(|d| d.table_id == *path)
                .expect("every engine table must be mirrored");
            assert_eq!(mirrored.verdict, verdict_string(t.verdict));
        }

        // The bank database has no tables here, so the engine reports a non-PASS
        // (never a fabricated clean PASS on nothing).
        assert_ne!(dto.verdict, "PASS", "an empty run must not pass");
    }

    /// A live demo_bank run records how every full-values table was checked:
    /// key, rules, buckets, spot check, change detection and control totals.
    #[tokio::test(flavor = "current_thread")]
    async fn live_run_records_facts_for_every_scanned_table() {
        let Some(old) = env_url("DMUX_TEST_DEMO_OLD_URL") else {
            return;
        };
        let Some(new) = env_url("DMUX_TEST_DEMO_NEW_URL") else {
            return;
        };
        let results = run_live(&old, &new).await;
        let dto = map_results("run-test", &results);
        let mut scanned = 0;
        for t in &dto.table_results {
            if t.level == CheckLevel::Structure || t.old_rows.is_none() {
                continue;
            }
            scanned += 1;
            let f = t
                .facts
                .as_ref()
                .unwrap_or_else(|| panic!("{} has no recorded facts", t.table_id));
            assert!(f.change_detection_old.is_some(), "{}", t.table_id);
            if t.level == CheckLevel::FullValues {
                assert!(!f.columns.is_empty(), "{}: no column rules", t.table_id);
                assert!(f.buckets_total.unwrap_or(0) > 0, "{}", t.table_id);
                assert!(f.spot.is_some(), "{}: no spot check fact", t.table_id);
            }
        }
        assert!(scanned > 0, "the demo run scans data-level tables");
    }

    // --- Live command lifecycle (gated on the large mock) ---

    /// URLs of the `demo_bank_large` copy, big enough that a scan lasts long
    /// enough to stop mid-flight.  CI's service containers do not carry this
    /// database (it takes far longer to build than a CI job allows), so this
    /// test runs only where `DMUX_TEST_LARGE_OLD_URL` / `DMUX_TEST_LARGE_NEW_URL`
    /// are set (the release checklist runs it locally).  The non-live phase and
    /// cancellation tests in `run_manager` still run everywhere.
    fn large_urls() -> Option<(String, String)> {
        match (
            std::env::var("DMUX_TEST_LARGE_OLD_URL"),
            std::env::var("DMUX_TEST_LARGE_NEW_URL"),
        ) {
            (Ok(old), Ok(new)) => Some((old, new)),
            _ => {
                eprintln!(
                    "skipped: DMUX_TEST_LARGE_OLD_URL / DMUX_TEST_LARGE_NEW_URL not set \
                     (live cancel test needs demo_bank_large)"
                );
                None
            }
        }
    }

    fn connect_input(url: &str) -> ConnectInput {
        let p = profile_from_url(url);
        ConnectInput {
            profile: crate::types::ProfileInput {
                id: format!("test-{}", p.port),
                label: "lifecycle test".into(),
                host: p.host.clone(),
                port: p.port,
                database: p.dbname.clone(),
                user: p.user.clone(),
                encryption: "allow-plaintext".into(),
                tls_fingerprint: None,
                plaintext_reason: Some("mock database has no TLS".into()),
                ca_file: None,
            },
            password: Some(p.password.clone()),
        }
    }

    fn start_input(old: &str, new: &str, levels: &[(&str, CheckLevel)]) -> StartRunInput {
        StartRunInput {
            old: connect_input(old),
            new: connect_input(new),
            mapping: Mapping::new(),
            levels: levels.iter().map(|(t, l)| (t.to_string(), *l)).collect(),
            default_level: CheckLevel::Structure,
            control_total_columns: vec![],
            old_parallelism: None,
            new_parallelism: None,
            export_keys: Some(false),
            allow_superuser: Some(SuperuserOverride {
                allow: true,
                reason: "local mock login is a superuser".into(),
            }),
            run_window: None,
            skip_tables: vec![],
        }
    }

    #[test]
    fn choose_checks_offers_only_tables_the_run_will_compare() {
        let old = vec![
            ("core".to_string(), "accounts".to_string()),
            ("core".to_string(), "client_notes".to_string()),
            ("core".to_string(), "archive".to_string()),
            ("core".to_string(), "old_ledger".to_string()),
        ];
        let new: BTreeSet<String> = [
            "core.accounts",
            "core.archive",
            "core.ledger",
            "core.customer_notes",
        ]
        .into_iter()
        .map(String::from)
        .collect();
        // Without a mapping: only same-named pairs.
        let plain = compared_old_tables(&old, &new, None);
        assert_eq!(
            plain.into_iter().collect::<Vec<_>>(),
            vec!["core.accounts", "core.archive"]
        );
        // With a mapping: a rename pairs old_ledger, an ignore drops archive,
        // and client_notes (no partner) is still not offered.
        let mut m = Mapping::new();
        m.ignored.push(dmux_engine::mapping::IgnoredItem {
            path: "core.archive".into(),
            reason: "archived before the migration".into(),
        });
        m.schema_mappings.insert(
            "core".into(),
            dmux_engine::mapping::SchemaMapping {
                target_schema: "core".into(),
                table_mappings: BTreeMap::from([(
                    "old_ledger".to_string(),
                    dmux_engine::mapping::TableMapping {
                        target_table: "ledger".into(),
                        status: dmux_engine::mapping::MappingStatus::Confirmed,
                        column_mappings: BTreeMap::new(),
                    },
                )]),
            },
        );
        let mapped = compared_old_tables(&old, &new, Some(&m));
        assert_eq!(
            mapped.into_iter().collect::<Vec<_>>(),
            vec!["core.accounts", "core.old_ledger"]
        );
    }

    #[test]
    fn left_out_tables_become_ignored_items_with_the_apps_reason() {
        let base = Mapping::new();
        let m = with_skipped_tables(
            base.clone(),
            &["demo_bank.audit_log".into(), "demo_bank.loans".into()],
        )
        .unwrap();
        let paths: Vec<(&str, &str)> = m
            .ignored
            .iter()
            .map(|i| (i.path.as_str(), i.reason.as_str()))
            .collect();
        assert_eq!(
            paths,
            vec![
                ("demo_bank.audit_log", NOT_SELECTED_REASON),
                ("demo_bank.loans", NOT_SELECTED_REASON)
            ]
        );
        // The hash covers the change, so the results name the real mapping.
        assert_ne!(m.mapping_hash, base.mapping_hash);
        // Nothing to skip leaves the mapping as it was.
        assert_eq!(with_skipped_tables(base.clone(), &[]).unwrap(), base);
    }

    #[test]
    fn a_table_already_ignored_keeps_its_own_reason() {
        let mut base = Mapping::new();
        base.ignored.push(dmux_engine::mapping::IgnoredItem {
            path: "demo_bank.archive".into(),
            reason: "archived before the migration".into(),
        });
        let m = with_skipped_tables(base.with_fresh_hash(), &["demo_bank.archive".into()]).unwrap();
        assert_eq!(m.ignored.len(), 1);
        assert_eq!(m.ignored[0].reason, "archived before the migration");
    }

    #[test]
    fn a_left_out_path_that_is_not_schema_dot_table_is_refused() {
        for bad in ["", "accounts", ".accounts", "demo_bank.", "a.b.c"] {
            assert!(
                matches!(
                    with_skipped_tables(Mapping::new(), &[bad.to_string()]),
                    Err(CommandError::InvalidInput { .. })
                ),
                "{bad:?}"
            );
        }
    }

    #[tokio::test]
    async fn a_run_reads_only_the_chosen_tables_and_records_the_rest() {
        let Some(old) = env_url("DMUX_TEST_DEMO_OLD_URL") else {
            return;
        };
        let Some(new) = env_url("DMUX_TEST_DEMO_NEW_URL") else {
            return;
        };
        let dir = tempfile::tempdir().unwrap();
        let state = AppState::with_password_store(
            dir.path().to_path_buf(),
            Box::new(crate::secrets::InMemoryPasswordStore::new()),
        )
        .unwrap();
        let mut input = start_input(&old, &new, &[]);
        // Leave out everything except demo_bank.account_types.
        let tables = run_live(&old, &new).await.tables.into_keys();
        input.skip_tables = tables.filter(|t| t != "demo_bank.account_types").collect();
        assert!(!input.skip_tables.is_empty());
        let id = start_run_with(input.clone(), &state).unwrap().run_id;
        assert!(
            wait_until(std::time::Duration::from_secs(120), || {
                let s = poll_run_with(id.clone(), &state).unwrap().overall_state;
                async move { s != "running" }
            })
            .await
        );
        let r = state.runs.results(&id).expect("the run finished");
        assert_eq!(
            r.tables.keys().collect::<Vec<_>>(),
            vec!["demo_bank.account_types"],
            "only the chosen table was compared"
        );
        for t in &input.skip_tables {
            assert!(
                r.ignored_items
                    .iter()
                    .any(|i| &i.path == t && i.reason == NOT_SELECTED_REASON),
                "{t} is recorded as not selected"
            );
        }
        assert_eq!(r.ignored, input.skip_tables.len());
        // The verdict is about the chosen table: every structure finding left
        // belongs to it, and nothing outside it can fail the run.
        assert!(
            r.structure_findings
                .iter()
                .all(|f| f.object == "demo_bank.account_types"
                    || f.object.starts_with("demo_bank.account_types.")),
            "{:?}",
            r.structure_findings
        );
        assert!(r.unmapped.is_empty(), "{:?}", r.unmapped);
        assert_eq!(
            r.verdict, r.tables["demo_bank.account_types"].verdict,
            "warnings: {:?}",
            r.warnings
        );
    }

    /// Scoped runs never invent a failure and never hide one (decision 30):
    /// each compared table of edge_bank, checked on its own, keeps exactly its
    /// full-run verdict, and the run's verdict comes only from that table and
    /// its own structure findings.  Runs only when DMUX_TEST_EDGE_OLD_URL and
    /// DMUX_TEST_EDGE_NEW_URL point at testdata/edge_bank (owner check).
    #[tokio::test]
    async fn a_scoped_run_matches_the_full_run_for_every_table() {
        let Some(old) = owner_url("DMUX_TEST_EDGE_OLD_URL") else {
            return;
        };
        let Some(new) = owner_url("DMUX_TEST_EDGE_NEW_URL") else {
            return;
        };
        let dir = tempfile::tempdir().unwrap();
        let state = AppState::with_password_store(
            dir.path().to_path_buf(),
            Box::new(crate::secrets::InMemoryPasswordStore::new()),
        )
        .unwrap();
        let run = |input: StartRunInput| {
            let id = start_run_with(input, &state).unwrap().run_id;
            let state = &state;
            async move {
                assert!(
                    wait_until(std::time::Duration::from_secs(900), || {
                        let s = poll_run_with(id.clone(), state).unwrap().overall_state;
                        async move { s != "running" }
                    })
                    .await
                );
                state.runs.results(&id).expect("the run finished")
            }
        };
        let full_input = || {
            let mut i = start_input(&old, &new, &[]);
            i.default_level = CheckLevel::FullValues;
            i
        };
        let full = run(full_input()).await;
        let compared: Vec<String> = full
            .tables
            .keys()
            .filter(|t| !full.unmapped.contains_key(*t))
            .cloned()
            .collect();
        assert!(compared.len() > 20, "{compared:?}");
        for chosen in &compared {
            let mut input = full_input();
            input.skip_tables = compared.iter().filter(|t| *t != chosen).cloned().collect();
            let r = run(input).await;
            // Only the chosen table was compared, with its full-run verdict.
            assert_eq!(
                r.tables.keys().collect::<Vec<_>>(),
                vec![chosen],
                "{chosen}"
            );
            assert_eq!(
                r.tables[chosen].verdict, full.tables[chosen].verdict,
                "{chosen}: scoped verdict differs from the full run"
            );
            // Its own structure findings are the same as in the full run.
            let mine = |res: &RunResults| -> Vec<String> {
                let mut v: Vec<String> = res
                    .structure_findings
                    .iter()
                    .filter(|f| f.object == *chosen || f.object.starts_with(&format!("{chosen}.")))
                    .map(|f| format!("{}={:?}", f.object, f.verdict))
                    .collect();
                v.sort();
                v
            };
            assert_eq!(mine(&r), mine(&full), "{chosen}: findings differ");
            assert_eq!(
                r.structure_findings.len(),
                mine(&r).len(),
                "{chosen}: outside findings kept"
            );
            // Every structure finding a full check fails on, outside the chosen
            // table, is accounted for in the summary warning (the real path:
            // findings on left-out tables are recovered, not lost).
            let mut outside_fails: Vec<&str> = full
                .structure_findings
                .iter()
                .filter(|f| {
                    f.verdict == dmux_engine::results::Verdict::Fail
                        && f.object != *chosen
                        && !f.object.starts_with(&format!("{chosen}."))
                })
                .map(|f| f.object.as_str())
                .collect();
            outside_fails.sort();
            outside_fails.dedup();
            if !outside_fails.is_empty() {
                let w = r
                    .warnings
                    .iter()
                    .find(|w| w.starts_with("this run checked only the chosen tables"))
                    .unwrap_or_else(|| panic!("{chosen}: no scope warning in {:?}", r.warnings));
                assert!(
                    w.contains(&format!(
                        "({} of them would fail a full check: [",
                        outside_fails.len()
                    )),
                    "{chosen}: expected {} named failures {outside_fails:?} in {w}",
                    outside_fails.len()
                );
                for o in outside_fails.iter().take(10) {
                    assert!(w.contains(o), "{chosen}: {o} not named in {w}");
                }
            }
            // The run's verdict comes only from the table and its findings.
            let mut own = vec![r.tables[chosen].verdict];
            own.extend(r.structure_findings.iter().map(|f| f.verdict));
            assert_eq!(
                r.verdict,
                dmux_engine::results::overall_verdict(&own),
                "{chosen}: verdict {:?} not explained by the chosen table; warnings {:?}",
                r.verdict,
                r.warnings
            );
        }
    }

    /// How many of Dmux's sessions on `url`'s database are running a
    /// statement (`text` filters the statement; `None` counts every session,
    /// active or idle).  The watcher's own session is excluded.
    async fn dmux_sessions(url: &str, text: Option<&str>, active_only: bool) -> i64 {
        let mut s = ReadOnlySession::open(&profile_from_url(url), true)
            .await
            .expect("watcher session");
        let pattern = format!("%{}%", text.unwrap_or(""));
        s.with_read_only("10s", |txn| {
            Box::pin(async move {
                let row = txn
                    .query_one(
                        "SELECT count(*) FROM pg_catalog.pg_stat_activity \
                         WHERE application_name = 'dmux' AND pid <> pg_catalog.pg_backend_pid() \
                         AND datname = pg_catalog.current_database() \
                         AND ($2::bool IS FALSE OR state = 'active') AND query LIKE $1",
                        &[&pattern, &active_only],
                    )
                    .await
                    .map_err(|e| dmux_engine::db::DbError::Postgres {
                        message: e.to_string(),
                    })?;
                Ok(row.get::<_, i64>(0))
            })
        })
        .await
        .expect("pg_stat_activity")
    }

    /// Poll `cond` every 50 ms until it holds or `limit` passes.
    async fn wait_until<F, Fut>(limit: std::time::Duration, mut cond: F) -> bool
    where
        F: FnMut() -> Fut,
        Fut: std::future::Future<Output = bool>,
    {
        let start = std::time::Instant::now();
        while start.elapsed() < limit {
            if cond().await {
                return true;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        false
    }

    async fn quiet(old: &str, new: &str) -> bool {
        dmux_sessions(old, None, true).await == 0 && dmux_sessions(new, None, true).await == 0
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn cancel_and_pause_really_stop_and_never_show_results() {
        let Some((old, new)) = large_urls() else {
            return;
        };
        let dir = tempfile::tempdir().unwrap();
        let state = AppState::with_password_store(
            dir.path().to_path_buf(),
            Box::new(crate::secrets::InMemoryPasswordStore::new()),
        )
        .unwrap();
        let secs = std::time::Duration::from_secs;

        // 1. Cancel during the 20M-row fingerprint scan.
        let id = start_run_with(
            start_input(
                &old,
                &new,
                &[("demo_bank.transactions", CheckLevel::FullValues)],
            ),
            &state,
        )
        .unwrap()
        .run_id;
        assert!(
            wait_until(secs(180), || async {
                dmux_sessions(&old, Some("transactions"), true).await > 0
            })
            .await,
            "the transactions scan never started"
        );
        let p = cancel_run_with(id.clone(), &state).await.unwrap();
        assert_eq!(p.overall_state, "cancelled");
        assert!(
            wait_until(secs(5), || quiet(&old, &new)).await,
            "a dmux query is still running on a server after cancel"
        );
        tokio::time::timeout(secs(30), state.runs.join(&id))
            .await
            .expect("the stopped drive task returns");
        assert_eq!(
            poll_run_with(id.clone(), &state).unwrap().overall_state,
            "cancelled"
        );
        assert!(matches!(
            get_results_with(&id, &state),
            Err(CommandError::RunStopped { .. })
        ));
        assert!(
            wait_until(secs(5), || async {
                dmux_sessions(&old, None, false).await == 0
                    && dmux_sessions(&new, None, false).await == 0
            })
            .await,
            "the cancelled run's sessions are closed, not left behind"
        );
        // Cancel is final: no resume.
        assert!(set_run_paused_with(id.clone(), false, &state)
            .await
            .is_err());

        // 2. Pause after the first table finished, during the second scan;
        //    then resume from the run's store and complete.
        let id = start_run_with(
            start_input(
                &old,
                &new,
                &[
                    ("demo_bank.account_types", CheckLevel::FullValues),
                    ("demo_bank.audit_log", CheckLevel::FullValues),
                ],
            ),
            &state,
        )
        .unwrap()
        .run_id;
        let store = crate::state::resume_store_path(&state, &id).unwrap();
        let first_done = || {
            dmux_engine::store::Store::open(&store)
                .and_then(|s| s.finished_tables())
                .map(|t| t.contains(&"demo_bank.account_types".to_string()))
                .unwrap_or(false)
        };
        assert!(
            wait_until(secs(180), || {
                let done = first_done();
                let old = old.clone();
                async move { done && dmux_sessions(&old, Some("audit_log"), true).await > 0 }
            })
            .await,
            "the audit_log scan never started after account_types finished"
        );
        let p = set_run_paused_with(id.clone(), true, &state).await.unwrap();
        assert_eq!(p.overall_state, "paused");
        assert!(
            wait_until(secs(5), || quiet(&old, &new)).await,
            "a dmux query is still running on a server after pause"
        );
        tokio::time::timeout(secs(30), state.runs.join(&id))
            .await
            .expect("the paused drive task returns");
        assert_eq!(
            poll_run_with(id.clone(), &state).unwrap().overall_state,
            "paused"
        );
        assert!(get_results_with(&id, &state).is_err());

        let p = set_run_paused_with(id.clone(), false, &state)
            .await
            .unwrap();
        assert_eq!(p.overall_state, "running");
        assert!(
            set_run_paused_with(id.clone(), false, &state)
                .await
                .is_err(),
            "a running run cannot be resumed again"
        );
        assert!(
            wait_until(secs(300), || {
                let s = poll_run_with(id.clone(), &state).unwrap().overall_state;
                async move { s != "running" }
            })
            .await,
            "the resumed run never finished"
        );
        let p = poll_run_with(id.clone(), &state).unwrap();
        assert_eq!(p.overall_state, "done", "{:?}", p.error);
        assert!(get_results_with(&id, &state).is_ok());
        let r = state.runs.results(&id).unwrap();
        assert!(
            r.resumed_tables
                .contains(&"demo_bank.account_types".to_string()),
            "the finished table came from the store: {:?}",
            r.resumed_tables
        );
        assert!(!r
            .resumed_tables
            .contains(&"demo_bank.audit_log".to_string()));
        assert!(
            r.tables["demo_bank.audit_log"].old_rows.is_some(),
            "rescanned"
        );

        // 3. An engine error is Failed with its typed error, never "running".
        let gone = |u: &str| format!("{}/dmux_no_such_database", u.rsplit_once('/').unwrap().0);
        let id = start_run_with(start_input(&old, &gone(&new), &[]), &state)
            .unwrap()
            .run_id;
        assert!(
            wait_until(secs(60), || {
                let s = poll_run_with(id.clone(), &state).unwrap().overall_state;
                async move { s == "failed" }
            })
            .await
        );
        let p = poll_run_with(id.clone(), &state).unwrap();
        assert!(
            matches!(p.error, Some(CommandError::Database { .. })),
            "{:?}",
            p.error
        );
        assert!(matches!(
            get_results_with(&id, &state),
            Err(CommandError::Database { .. })
        ));
        assert!(
            cancel_run_with(id.clone(), &state).await.is_err(),
            "failed is final"
        );
        assert_eq!(poll_run_with(id, &state).unwrap().overall_state, "failed");
    }

    // --- T068: run window, sleep hold and time remaining ---

    /// A URL for a port nothing listens on; the secret is built at run time.
    fn closed_url() -> String {
        format!("postgres://u:t{}@127.0.0.1:1/none", std::process::id())
    }

    /// A run against a port nothing listens on: it fails fast, which is all
    /// these lifecycle tests need.
    fn closed_port_input() -> StartRunInput {
        let url = closed_url();
        start_input(&url, &url, &[])
    }

    async fn wait_for_drive_to_return(state: &AppState, id: &str) {
        let limit = std::time::Duration::from_secs(30);
        assert!(
            wait_until(limit, || async { state.runs.drive_returned(id, 1) }).await,
            "the drive task never returned"
        );
    }

    fn state_with(
        inhibitor: Arc<crate::sleep::stub::CountingInhibitor>,
    ) -> (tempfile::TempDir, AppState) {
        let dir = tempfile::tempdir().unwrap();
        let mut state = AppState::with_password_store(
            dir.path().to_path_buf(),
            Box::new(crate::secrets::InMemoryPasswordStore::new()),
        )
        .unwrap();
        state.sleep = inhibitor;
        (dir, state)
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn the_sleep_hold_is_taken_for_the_attempt_and_released_when_it_returns() {
        let hold = Arc::new(crate::sleep::stub::CountingInhibitor::default());
        let (_dir, state) = state_with(Arc::clone(&hold));
        let id = start_run_with(closed_port_input(), &state).unwrap().run_id;
        wait_for_drive_to_return(&state, &id).await;
        assert_eq!(hold.acquired.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert_eq!(hold.held(), 0, "released once the engine returned");
        assert_eq!(
            poll_run_with(id.clone(), &state).unwrap().overall_state,
            "failed"
        );
        let notices = state
            .runs
            .control(&id)
            .unwrap()
            .with_live(|l| l.notices().to_vec());
        assert!(notices.is_empty(), "a hold that worked says nothing");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_failed_hold_never_fails_the_run_and_is_noted() {
        let hold = Arc::new(crate::sleep::stub::CountingInhibitor {
            fail: true,
            ..Default::default()
        });
        let (_dir, state) = state_with(Arc::clone(&hold));
        let id = start_run_with(closed_port_input(), &state).unwrap().run_id;
        wait_for_drive_to_return(&state, &id).await;
        // The engine still ran (it failed on the connection, not on sleep).
        let p = poll_run_with(id.clone(), &state).unwrap();
        assert_eq!(p.overall_state, "failed");
        assert!(
            matches!(p.error, Some(CommandError::Database { .. })),
            "{:?}",
            p.error
        );
        let notices = state
            .runs
            .control(&id)
            .unwrap()
            .with_live(|l| l.notices().to_vec());
        assert_eq!(notices, [crate::sleep::SLEEP_WARNING.to_string()]);
        assert_eq!(hold.held(), 0);
    }

    #[test]
    fn the_real_inhibitor_never_panics_whatever_the_machine() {
        use crate::sleep::SleepInhibitor;
        // On a build machine without a session bus this is an Err; with one it
        // holds and releases.  Either way the call returns.
        let _ = crate::sleep::SystemInhibitor.acquire();
    }

    #[tokio::test]
    async fn a_bad_run_window_is_refused_before_anything_starts() {
        let hold = Arc::new(crate::sleep::stub::CountingInhibitor::default());
        let (_dir, state) = state_with(Arc::clone(&hold));
        for bad in ["25:00-05:00", "22:00", "08:00-08:00", "nonsense"] {
            let mut input = closed_port_input();
            input.run_window = Some(bad.to_string());
            let err = start_run_with(input, &state).unwrap_err();
            assert!(
                matches!(err, CommandError::InvalidInput { .. }),
                "{bad}: {err:?}"
            );
        }
        assert_eq!(hold.acquired.load(std::sync::atomic::Ordering::SeqCst), 0);
        // A blank window means off, and a good one is accepted.
        let mut input = closed_port_input();
        input.run_window = Some("  ".to_string());
        assert!(start_run_with(input, &state).is_ok());
        let mut input = closed_port_input();
        input.run_window = Some("22:00-05:00".to_string());
        assert!(start_run_with(input, &state).is_ok());
    }

    fn live_state_run() -> (
        tempfile::TempDir,
        AppState,
        String,
        Arc<dmux_engine::run::RunControl>,
    ) {
        let (dir, state) = state_with(Arc::new(crate::sleep::stub::CountingInhibitor::default()));
        let url = closed_url();
        let mut config = RunConfig::new(
            Input::Live(profile_from_url(&url)),
            Input::Live(profile_from_url(&url)),
        );
        for t in ["s.a", "s.b"] {
            config.levels.insert(t.to_string(), CheckLevel::FullValues);
        }
        // Never driven: the test plays the engine through the live numbers.
        state.runs.insert("run-live", RunHandle::new(config, None));
        let ticket = state.runs.begin_first("run-live").unwrap();
        let control = ticket.config.control.clone().unwrap();
        (dir, state, "run-live".to_string(), control)
    }

    #[test]
    fn polling_reports_the_corrected_time_remaining_and_the_window_wait() {
        let (_dir, state, id, control) = live_state_run();
        // Before the plan exists nothing is claimed.
        let p = poll_run_with(id.clone(), &state).unwrap();
        assert!(p.eta_seconds_min.is_none() && p.eta_seconds_max.is_none());
        assert!(p.waiting_for_window.is_none() && p.notices.is_empty());
        assert!(p
            .tables
            .iter()
            .all(|t| t.state == "queued" && t.eta_seconds.is_none()));

        control.with_live(|l| {
            l.set_plan(
                ["s.a", "s.b"]
                    .iter()
                    .map(|t| (t.to_string(), estimate_from_seconds(100.0)))
                    .collect(),
            );
            l.table_started("s.a", std::time::Instant::now());
        });
        let p = poll_run_with(id.clone(), &state).unwrap();
        let (lo, hi) = (p.eta_seconds_min.unwrap(), p.eta_seconds_max.unwrap());
        assert!(lo > 0 && lo < hi, "{lo}..{hi}");
        let a = p.tables.iter().find(|t| t.table_id == "s.a").unwrap();
        let b = p.tables.iter().find(|t| t.table_id == "s.b").unwrap();
        assert_eq!(a.state, "running");
        assert!(a.eta_seconds.is_some_and(|e| e > 0));
        assert_eq!((b.state.as_str(), b.eta_seconds), ("queued", None));

        control.with_live(|l| l.set_window_wait(Some("22:00".to_string())));
        let p = poll_run_with(id.clone(), &state).unwrap();
        assert_eq!(p.waiting_for_window.as_deref(), Some("22:00"));

        // A table finishing slowly raises what is left for the other one.
        control.with_live(|l| l.table_finished("s.a", 300.0));
        let p2 = poll_run_with(id.clone(), &state).unwrap();
        // (The store is absent, so s.a still counts as unfinished here; the
        // total therefore reflects both tables at the corrected speed.)
        assert!(p2.eta_seconds_max.unwrap() > hi);

        // A run that is not working shows no live numbers.
        state
            .runs
            .transition(&id, &[RunPhase::Running], RunPhase::Paused)
            .unwrap();
        let p = poll_run_with(id.clone(), &state).unwrap();
        assert!(p.eta_seconds_min.is_none() && p.waiting_for_window.is_none());
        // Paused: an unscanned table is still to come.
        assert!(p.tables.iter().all(|t| t.state == "queued"));

        // Ended: a table the run never scanned is not "queued" any more.
        state
            .runs
            .transition(&id, &[RunPhase::Paused], RunPhase::Cancelled)
            .unwrap();
        let p = poll_run_with(id, &state).unwrap();
        assert!(
            p.tables.iter().all(|t| t.state == "not-scanned"),
            "{:?}",
            p.tables.iter().map(|t| &t.state).collect::<Vec<_>>()
        );
    }
}
