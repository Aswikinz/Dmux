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
    pub control_total_columns: Option<Vec<String>>,
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
    // The mapping is accepted (and treated as untrusted) but the plan's table
    // list and levels come from the live catalogue, never from the mapping file
    // (PLAN 4.4 / the "column types from the live catalogue" rule).
    let _ = mapping;
    let control_totals: BTreeSet<String> = control_total_columns
        .unwrap_or_default()
        .into_iter()
        .collect();

    let old_inv = collect(&old, allow, &state).await?;
    // The new inventory is collected too: the pairs themselves are what the run
    // re-derives from the live catalogue, but a missing new table must still be
    // listed (the run records it as MissingOnNew, never silently dropped).
    let _new_inv = collect(&new, allow, &state).await?;

    // Old tables (partitions already folded), mapped to their new side.
    let mut plan_tables: Vec<TableCheckDto> = Vec::new();
    let mut estimate_inputs: Vec<(String, String)> = Vec::new();
    for schema in &old_inv.schemas {
        for table in &schema.tables {
            let path = format!("{}.{}", schema.name, table.name);

            // Suggested control totals: the confirmed names this table has, or
            // the name heuristic when none are confirmed yet (PLAN 4.4).
            let controls: Vec<String> = if control_totals.is_empty() {
                table
                    .columns
                    .iter()
                    .filter(|c| is_control_total_name(&c.name))
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
    let mapping = validate_mapping(&input.mapping)?;
    let old_profile = resolve(&input.old, state)?;
    let new_profile = resolve(&input.new, state)?;
    let old_parallelism = input.old_parallelism.unwrap_or(2);
    let new_parallelism = input.new_parallelism.unwrap_or(4);
    if old_parallelism == 0 || new_parallelism == 0 {
        return Err(CommandError::InvalidInput {
            message: "the parallel-scan limits must be at least 1".to_string(),
        });
    }
    let export_keys = input.export_keys.unwrap_or(true);

    let run_id = RunManager::fresh_id();
    let resume_store = crate::state::resume_store_path(state, &run_id)?;

    let config = RunConfig {
        mapping,
        default_level: input.default_level,
        levels: input.levels.clone(),
        control_total_columns: input.control_total_columns.into_iter().collect(),
        old_parallelism,
        new_parallelism,
        allow_superuser: allow,
        allow_superuser_reason: allow_reason,
        export_keys,
        resume_store: Some(resume_store.clone()),
        ..RunConfig::new(Input::Live(old_profile), Input::Live(new_profile))
    };

    let manager = Arc::clone(&state.runs);
    manager.insert(&run_id, RunHandle::new(config, Some(resume_store)));
    let ticket = manager
        .begin_first(&run_id)
        .ok_or_else(|| CommandError::internal("the new run could not be started"))?;
    spawn_drive_task(&manager, &run_id, ticket);

    Ok(StartRunResult { run_id })
}

/// Spawn one drive attempt on the blocking pool.  The engine's run future is
/// `!Send` (it borrows a `tokio_postgres::Transaction`), so it runs inside
/// `spawn_blocking` + `block_on` on a dedicated OS thread.  The task reports
/// the engine's outcome through [`RunManager::finish`], which applies it only
/// to its own attempt and only while the run is still running.
fn spawn_drive_task(manager: &Arc<RunManager>, run_id: &str, ticket: DriveTicket) {
    let generation = ticket.generation;
    let task = tauri::async_runtime::spawn_blocking({
        let manager = Arc::clone(manager);
        let rid = run_id.to_string();
        move || {
            let config = ticket.config;
            let out =
                tauri::async_runtime::block_on(async { run(&config).await }).map(|o| o.results);
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

    let mut tables: Vec<TableProgressDto> = Vec::new();
    for path in config.levels.keys() {
        let (schema, table) = split_path(path);
        let done = finished.contains(path);
        tables.push(TableProgressDto {
            table_id: path.clone(),
            schema_name: schema,
            table_name: table,
            state: if done { "done" } else { "queued" }.to_string(),
            progress_percent: if done { 100 } else { 0 },
            eta_seconds: None,
        });
    }
    tables.sort_by(|a, b| a.table_id.cmp(&b.table_id));

    Ok(RunProgressDto {
        run_id: run_id.clone(),
        overall_state: phase_string(phase).to_string(),
        tables,
        eta_seconds_min: None,
        eta_seconds_max: None,
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
            spawn_drive_task(&manager, &run_id, ticket);
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
    let results = state.runs.results(run_id)?;
    Ok(map_results(run_id, &results))
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

        if t.verdict != Verdict::Pass {
            failures.push(FailureCardDto {
                id: format!("fail-{path}"),
                table_id: path.clone(),
                schema_name: schema.clone(),
                table_name: table.clone(),
                level: t.level,
                verdict: verdict.clone(),
                summary: summary_for(t),
                detail: detail_for(t),
            });
        }

        table_results.push(TableResultDto {
            table_id: path.clone(),
            schema_name: schema,
            table_name: table,
            verdict,
            level: t.level,
            checked,
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
}
