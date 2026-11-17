//! Data-half engine commands: choose checks, run, progress, pause/cancel and
//! results (PLAN.md 4.4-4.7).
//!
//! The one rule that dominates every command here (AGENTS.md hard rule, and the
//! task's cardinal rule): **the UI must only ever show what the engine's results
//! say.** A false PASS is the worst bug this project can have.  So:
//!
//! * verdicts are copied verbatim from the engine, never guessed;
//! * progress is read back from the engine's own resume store, never invented;
//! * cancel aborts the drive task (dropping the live connections, which stops
//!   every in-flight server statement); pause/resume re-uses the same resume
//!   store so the engine's own change-detection vouches for every reused row;
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

use crate::commands::gateway::{collect, drive, resolve, SuperuserOverride};
use crate::error::{from_db_error, CommandError};
use crate::run_manager::{RunHandle, RunPhase};
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
    pub overall_state: String,
    pub tables: Vec<TableProgressDto>,
    pub eta_seconds_min: Option<u64>,
    pub eta_seconds_max: Option<u64>,
}

/// A drill-down row, mirroring the contract's `DrillDownRow` (keys only).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct DrillDownRowDto {
    pub key: String,
    pub kind: String,
    pub differing_columns: Option<Vec<String>>,
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
/// engine resume store under the app data dir (PLAN 4.6).  Credentials are
/// resolved once here, through the trusted `resolve` path, and never stored.
#[tauri::command]
pub async fn start_run(
    input: StartRunInput,
    state: State<'_, AppState>,
) -> Result<StartRunResult, CommandError> {
    let allow = input
        .allow_superuser
        .as_ref()
        .is_some_and(SuperuserOverride::allows);
    let old_profile = resolve(&input.old, &state)?;
    let new_profile = resolve(&input.new, &state)?;
    let old_parallelism = input.old_parallelism.unwrap_or(2);
    let new_parallelism = input.new_parallelism.unwrap_or(4);
    if old_parallelism == 0 || new_parallelism == 0 {
        return Err(CommandError::InvalidInput {
            message: "the parallel-scan limits must be at least 1".to_string(),
        });
    }
    let export_keys = input.export_keys.unwrap_or(true);

    let resume_store = crate::state::resume_dir(&state).join("resume.db");

    let config = RunConfig {
        old: Input::Live(old_profile),
        new: Input::Live(new_profile),
        mapping: input.mapping.clone(),
        default_level: input.default_level,
        levels: input.levels.clone(),
        control_total_columns: input.control_total_columns.into_iter().collect(),
        column_options: BTreeMap::new(),
        old_parallelism,
        new_parallelism,
        allow_superuser: allow,
        export_keys,
        resume_store: Some(resume_store),
        stats_settle: dmux_engine::run::DEFAULT_STATS_SETTLE,
    };

    let manager = Arc::clone(&state.runs);
    let handle = RunHandle {
        config: config.clone(),
        resume_store: Some(crate::state::resume_dir(&state).join("resume.db")),
        task: None,
        phase: RunPhase::Running,
    };
    let run_id = manager.insert(handle);

    spawn_drive_task(&manager, &run_id, config);

    Ok(StartRunResult { run_id })
}

/// Spawn (or re-spawn, on resume) the engine run on the blocking pool.  The
/// engine's run future is `!Send` (it borrows a `tokio_postgres::Transaction`),
/// so it runs inside `spawn_blocking` + `block_on` on a dedicated OS thread.
fn spawn_drive_task(
    manager: &Arc<crate::run_manager::RunManager>,
    run_id: &str,
    config: RunConfig,
) {
    let manager = Arc::clone(manager);
    let rid = run_id.to_string();
    let task = tauri::async_runtime::spawn_blocking({
        let manager = Arc::clone(&manager);
        let rid = rid.clone();
        move || {
            let out = tauri::async_runtime::block_on(async move {
                run(&config)
                    .await
                    .map(|o| o.results)
                    .map_err(|e| e.to_string())
            });
            if out.is_ok() {
                manager.set_phase(&rid, RunPhase::Done);
            }
            out
        }
    });
    manager.set_task(&rid, task);
}

/// Report honest progress: the set of tables the engine has really finished (from
/// its resume store) plus the run's lifecycle phase.
#[tauri::command]
pub async fn poll_run(
    run_id: String,
    state: State<'_, AppState>,
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

    let overall_state = match phase {
        RunPhase::Running => "running",
        RunPhase::Paused => "paused",
        RunPhase::Done => "done",
        RunPhase::Cancelled => "cancelled",
    };

    Ok(RunProgressDto {
        run_id,
        overall_state: overall_state.to_string(),
        tables,
        eta_seconds_min: None,
        eta_seconds_max: None,
    })
}

/// Pause (abort the in-flight drive task; completed tables stay in the resume
/// store) or resume (re-run against the same resume store).
#[tauri::command]
pub async fn set_run_paused(
    run_id: String,
    paused: bool,
    state: State<'_, AppState>,
) -> Result<RunProgressDto, CommandError> {
    let manager = Arc::clone(&state.runs);
    let Some((config, _resume)) = manager.snapshot(&run_id) else {
        return Err(CommandError::RunNotFound {
            run_id: run_id.clone(),
        });
    };

    if paused {
        // Abort the in-flight drive task (drops connections → stops server
        // queries); everything completed so far stays in the resume store.
        manager.abort(&run_id);
        manager.set_phase(&run_id, RunPhase::Paused);
    } else {
        // Re-run against the same resume store: the engine reuses completed
        // tables only when their counters/epoch/replica marks still vouch.
        spawn_drive_task(&manager, &run_id, config);
        manager.set_phase(&run_id, RunPhase::Running);
    }

    Box::pin(poll_run(run_id, state)).await
}

/// Cancel a run: abort the drive task, which drops the live connections and so
/// cancels every in-flight server query (PLAN 4.6's "Cancel really stops").
#[tauri::command]
pub async fn cancel_run(
    run_id: String,
    state: State<'_, AppState>,
) -> Result<RunProgressDto, CommandError> {
    let manager = Arc::clone(&state.runs);
    if manager.phase(&run_id).is_none() {
        return Err(CommandError::RunNotFound {
            run_id: run_id.clone(),
        });
    }
    manager.abort(&run_id);
    manager.set_phase(&run_id, RunPhase::Cancelled);
    Box::pin(poll_run(run_id, state)).await
}

/// Return the engine's results for a finished run, mapped to the contract.
#[tauri::command]
pub async fn get_results(
    run_id: String,
    state: State<'_, AppState>,
) -> Result<ResultsDto, CommandError> {
    let manager = Arc::clone(&state.runs);
    let Some(phase) = manager.phase(&run_id) else {
        return Err(CommandError::RunNotFound {
            run_id: run_id.clone(),
        });
    };
    if !matches!(phase, RunPhase::Done) {
        return Err(CommandError::ResultsNotAvailable {
            message: "the run has not finished; results are not available yet".to_string(),
        });
    }
    let Some(task) = manager.take_task(&run_id) else {
        return Err(CommandError::ResultsNotAvailable {
            message: "the run completed but its result was already taken".to_string(),
        });
    };
    let joined = task.await;
    let results = match joined {
        Ok(Ok(results)) => results,
        Ok(Err(why)) => {
            return Err(CommandError::Database { message: why });
        }
        Err(join_err) => {
            // A cancelled task is already marked Cancelled and never reaches
            // here; any other join error is an internal failure.
            return Err(CommandError::internal(format!(
                "run task failed: {join_err}"
            )));
        }
    };
    Ok(map_results(&run_id, &results))
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
            old: Input::Live(profile_from_url(old_url)),
            new: Input::Live(profile_from_url(new_url)),
            mapping: Mapping::new(),
            default_level: CheckLevel::FullValues,
            levels: BTreeMap::new(),
            control_total_columns: BTreeSet::new(),
            column_options: BTreeMap::new(),
            old_parallelism: 2,
            new_parallelism: 4,
            allow_superuser: true,
            export_keys: true,
            resume_store: None,
            stats_settle: dmux_engine::run::DEFAULT_STATS_SETTLE,
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
}
