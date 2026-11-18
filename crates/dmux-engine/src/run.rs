//! The comparison orchestration (PLAN.md 3 and 4.6-4.8): turn live databases or
//! saved fingerprints into per-table verdicts, structure findings, a coverage
//! line and an overall verdict.
//!
//! This module composes the comparison core.  It opens read-only sessions only
//! through `db::ReadOnlySession`.  Every path (live, file against live, file
//! against file) records what it measured as [`verdict::Evidence`] and lets the
//! shared [`verdict::Builder`] decide, so the rules cannot drift between paths.
//! A table whose plan or scan fails becomes an explicit TOOL_FAULT result: the
//! run never aborts on one table and never drops one silently.
//!
//! ## Levels
//!
//! * Structure — no data scan; the verdict comes from the table's structure
//!   findings and is reported as "structure only".
//! * CountsAndTotals — one `count(*)` plus control-total sums per side; no value
//!   is hashed; reported as "counts only".
//! * FullValues — the fingerprint, drill-down and independent spot check.
//!
//! ## Parallelism
//!
//! `old_parallelism` / `new_parallelism` are **maxima** (PLAN.md 4.6).  This
//! runner scans one table at a time on each side, which never exceeds them; it
//! does not yet run scans concurrently.  A limit below 1 is rejected.

pub mod capture;
pub mod compare_files;
pub mod scan;
pub mod spot;
pub mod structure;
pub mod verdict;

use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use std::time::Instant;
use thiserror::Error;

use crate::consistency::{
    change_detection_floor, read_counters, read_replica_state, read_server_epoch,
    read_track_counts, read_wal_position, replica_decision, ChangeCounters, ReplicaDecision,
    ReplicaState, ServerEpoch, SideChangeDetection,
};
use crate::core::compare::{compare, Comparison, SideOutcome, Verdict as CoreVerdict};
use crate::core::drilldown::{classify, fetch_rows};
use crate::core::fingerprint::{
    fingerprint, fingerprint_range, merge_fingerprints, range_cuts, range_parts, Fingerprint,
    RangeFilter, RangeType, Side, Source, TablePlan,
};
use crate::core::normalise::{plan_column, ColumnOptions};
use crate::db::{probe_server, ConnProfile, DbError, ReadOnlySession, ServerInfo};
use crate::estimate::{estimate_from_seconds, timed_sample, total_estimate, Estimate};
use crate::inventory::{ConstraintKind, DatabaseInventory};
use crate::mapping::Mapping;
use crate::plan::{build_table_spec, core_table_plan, read_live_table_opt, SpecInputs, TableSpec};
use crate::results::{
    fresh_run_id, ChangedRowExport, CheckLevel, ColumnNote, ConnectionInfo, Connections,
    ControlTotalExport, RunResults, TableDetail,
};
use crate::store::{Loaded, RunKey, SideMark, Store, StoredTableResult};

use verdict::{Builder, CountsComparison, Evidence, Outcome, Spot};

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
    /// Per-table level overrides, by old `schema.table`.
    pub levels: BTreeMap<String, CheckLevel>,
    /// Control-total column names (applied to every table that has them).
    pub control_total_columns: BTreeSet<String>,
    /// Per-column options (PLAN.md 3.2), by old `schema.table.column`.
    pub column_options: BTreeMap<String, ColumnOptions>,
    /// Maximum concurrent scans on the old side (see the module docs).
    pub old_parallelism: usize,
    /// Maximum concurrent scans on the new side (see the module docs).
    pub new_parallelism: usize,
    pub allow_superuser: bool,
    /// The recorded reason for a superuser override, when one was accepted.
    /// Recorded in the results `connections` block (audit M5); `None` when the
    /// caller allowed a superuser without a recorded reason.
    pub allow_superuser_reason: Option<String>,
    /// Put row keys of differing rows into the results (the user ticked export).
    pub export_keys: bool,
    /// An optional resume store path (see `crate::store`).
    pub resume_store: Option<std::path::PathBuf>,
    /// How long to wait before the final change-counter read, so writes made
    /// during the run have been flushed into the server statistics (see
    /// `crate::consistency`).  Default 2 s; definition key `stats_settle_ms`.
    pub stats_settle: std::time::Duration,
    /// Pause/cancel switch (see [`RunControl`]); `None` runs to completion.
    pub control: Option<std::sync::Arc<RunControl>>,
    /// An optional daily window (local time) outside which the run waits
    /// between tables (PLAN 4.6).  Deliberately NOT part of
    /// [`definition_hash`]: it only says when the run works, not what it
    /// compares, so a resumed run may use a different window.
    pub run_window: Option<crate::run_window::RunWindow>,
    /// The user chose specific tables (decision 30): the verdict is about those
    /// tables only.  Structure differences outside them, tables only in the new
    /// database and old tables with no partner are reported (one warning, and
    /// the unpaired ones as ignored items with the reason) but do not decide
    /// the verdict.  `false` (a full check) compares the whole database.
    pub only_chosen_tables: bool,
}

/// The reason the desktop app records for a table the user did not choose.
pub const NOT_SELECTED_REASON: &str = "not selected for this run";

/// Whether an ignored item was left out because the run checked only chosen
/// tables (not selected, or not mapped outside the run), as opposed to an
/// ignore a person decided with their own reason.
pub fn is_outside_run(reason: &str) -> bool {
    reason == NOT_SELECTED_REASON || reason == NOT_MAPPED_OUTSIDE_RUN
}

/// The reason recorded for an old table with no partner in a run that checks
/// only the chosen tables.
pub const NOT_MAPPED_OUTSIDE_RUN: &str =
    "not mapped: no table in the new database; outside this run, which checked only the chosen tables";

/// How often a run waiting for its window looks at the clock and the stop flag.
const WINDOW_POLL: std::time::Duration = std::time::Duration::from_millis(500);

/// The default [`RunConfig::stats_settle`].
pub const DEFAULT_STATS_SETTLE: std::time::Duration = std::time::Duration::from_secs(2);

impl RunConfig {
    /// A configuration comparing `old` with `new`: the identity mapping, full
    /// values for every table, no control totals or options, parallel-scan
    /// maxima 2 (old) and 4 (new), superusers refused, no exported keys, no
    /// resume store, the default stats settle and no run control.
    ///
    /// There is deliberately no `Default`: a run always names its own
    /// connections, so no built-in host, login or TLS choice can ship.
    pub fn new(old: Input, new: Input) -> Self {
        Self {
            old,
            new,
            mapping: Mapping::new(),
            default_level: CheckLevel::FullValues,
            levels: BTreeMap::new(),
            control_total_columns: BTreeSet::new(),
            column_options: BTreeMap::new(),
            old_parallelism: 2,
            new_parallelism: 4,
            allow_superuser: false,
            allow_superuser_reason: None,
            export_keys: false,
            resume_store: None,
            stats_settle: DEFAULT_STATS_SETTLE,
            control: None,
            run_window: None,
            only_chosen_tables: false,
        }
    }
}

/// The stop switch for a running comparison (PLAN.md 4.6: pause and cancel
/// really stop).
///
/// The caller keeps an `Arc` and hands a clone to [`RunConfig::control`].
/// The run registers the cancel token of every session it opens and checks the
/// stop flag before each table, before each scan and between the drill-down
/// and spot-check queries.  [`RunControl::stop`] sets the flag and returns the
/// registered tokens so the caller can cancel the statements in flight on the
/// server; the run then returns [`RunError::Cancelled`], never results.
/// Tables finished before the stop stay in the resume store.
#[derive(Default)]
pub struct RunControl {
    stopped: std::sync::atomic::AtomicBool,
    tokens: std::sync::Mutex<Vec<crate::db::CancelToken>>,
    live: std::sync::Mutex<crate::eta::LiveProgress>,
}

impl std::fmt::Debug for RunControl {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RunControl")
            .field("stopped", &self.is_stopped())
            .finish_non_exhaustive()
    }
}

impl RunControl {
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether a stop was requested.
    pub fn is_stopped(&self) -> bool {
        self.stopped.load(std::sync::atomic::Ordering::SeqCst)
    }

    /// Request a stop.  Returns every registered cancel token; the caller
    /// sends each one (`CancelToken::cancel`) to stop the server-side query.
    pub fn stop(&self) -> Vec<crate::db::CancelToken> {
        self.stopped
            .store(true, std::sync::atomic::Ordering::SeqCst);
        self.tokens()
    }

    /// Register a session's cancel token.  A session registered after a stop
    /// is cancelled by the run's next stop check, before it queries.
    pub fn register(&self, token: crate::db::CancelToken) {
        self.tokens
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(token);
    }

    /// The registered cancel tokens.
    pub fn tokens(&self) -> Vec<crate::db::CancelToken> {
        self.tokens
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    /// Read or update the run's live numbers (estimates, measured tables,
    /// window wait) under the lock.
    pub fn with_live<T>(&self, f: impl FnOnce(&mut crate::eta::LiveProgress) -> T) -> T {
        f(&mut self
            .live
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner))
    }

    /// How many sessions are registered.
    pub fn registered(&self) -> usize {
        self.tokens
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .len()
    }
}

/// Whether the run was asked to stop.
pub(crate) fn stopped(config: &RunConfig) -> bool {
    config.control.as_ref().is_some_and(|c| c.is_stopped())
}

/// `Err(RunError::Cancelled)` once a stop was requested.
pub(crate) fn checkpoint(config: &RunConfig) -> Result<(), RunError> {
    if stopped(config) {
        Err(RunError::Cancelled)
    } else {
        Ok(())
    }
}

/// Between tables: wait while the run window is closed, then resume by itself.
///
/// Returns at once when no window is set or it is open.  Otherwise records one
/// warning in `results`, shows the wait in the run's live state and polls the
/// clock (`now`, minutes after local midnight) once a `step`, checking the stop
/// flag each time so pause and cancel still work.  Never called mid-scan, so no
/// half result exists.
pub(crate) async fn wait_for_window(
    config: &RunConfig,
    results: &mut RunResults,
    now: impl Fn() -> u32,
    step: std::time::Duration,
) -> Result<(), RunError> {
    let Some(window) = config.run_window else {
        return Ok(());
    };
    if window.contains(now()) {
        return Ok(());
    }
    let note =
        format!("the run paused outside its run window ({window}) and carried on when it opened");
    if !results.warnings.contains(&note) {
        results.warnings.push(note);
    }
    let opens = window.opens_at();
    let set = |v: Option<String>| {
        if let Some(c) = &config.control {
            c.with_live(|l| l.set_window_wait(v));
        }
    };
    set(Some(opens));
    let outcome = loop {
        if let Err(e) = checkpoint(config) {
            break Err(e);
        }
        if window.contains(now()) {
            break Ok(());
        }
        tokio::time::sleep(step).await;
    };
    set(None);
    outcome
}

/// Register a freshly opened session with the run control, then check the
/// stop flag (a stop that raced the open is honoured before any query).
pub(crate) fn register(config: &RunConfig, s: &ReadOnlySession) -> Result<(), RunError> {
    if let Some(c) = &config.control {
        c.register(s.cancel_token());
    }
    checkpoint(config)
}

/// Sleep for `d`, waking early (with `Err(Cancelled)`) on a stop.
pub(crate) async fn settle(config: &RunConfig, d: std::time::Duration) -> Result<(), RunError> {
    let step = std::time::Duration::from_millis(50);
    let end = Instant::now() + d;
    loop {
        checkpoint(config)?;
        let now = Instant::now();
        if now >= end {
            return Ok(());
        }
        tokio::time::sleep(step.min(end - now)).await;
    }
}

/// Errors that stop a whole run (one table's failure never does).
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
    #[error("configuration error: {0}")]
    Config(String),
    #[error("a full run needs both sides live")]
    NeedsLive,
    /// The run was stopped through its [`RunControl`] (pause or cancel).  No
    /// results exist; finished tables remain in the resume store.
    #[error("the run was stopped before it finished")]
    Cancelled,
}

impl From<tokio_postgres::Error> for RunError {
    fn from(e: tokio_postgres::Error) -> Self {
        RunError::Db(DbError::Postgres {
            message: crate::plan::render_pg_error(&e),
        })
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
    let inventory = inventory(session).await?;
    let mut out = Vec::new();
    for schema in &inventory.schemas {
        for table in &schema.tables {
            out.push((schema.name.clone(), table.name.clone()));
        }
    }
    out.sort();
    Ok(out)
}

/// Map an old table name to its new `(schema, table)`: the schema through the
/// schema mapping or a schema-rename rule, the table through an explicit table
/// mapping or else the first prefix/suffix rule that matches; identity otherwise.
pub fn map_table(mapping: &Mapping, schema: &str, table: &str) -> (String, String) {
    let new_schema = structure::map_schema(mapping, schema);
    if let Some(tm) = mapping
        .schema_mappings
        .get(schema)
        .and_then(|sm| sm.table_mappings.get(table))
    {
        if !tm.target_table.is_empty() {
            return (new_schema, tm.target_table.clone());
        }
    }
    for rule in &mapping.rules {
        if matches!(rule.kind, crate::mapping::RuleKind::SchemaRename) {
            continue;
        }
        if let Some(renamed) = crate::mapping::apply_rule(rule, table) {
            return (new_schema, renamed);
        }
    }
    (new_schema, table.to_string())
}

/// SHA-256 of what a run is: both inputs (connection identity without the
/// password, or the fingerprint file's own hash), levels, control totals,
/// options, the mapping hash and the export choice.  The resume store is keyed
/// by it.
pub fn definition_hash(config: &RunConfig) -> String {
    use sha2::Digest;
    let input = |i: &Input| match i {
        Input::Live(p) => {
            let mut v = serde_json::json!({
                "host": p.host, "port": p.port, "db": p.dbname, "user": p.user,
                "tls": match &p.tls_mode {
                    crate::db::TlsMode::VerifyFull => "verify".to_string(),
                    crate::db::TlsMode::TrustFingerprint(f) => format!("fingerprint:{f}"),
                    crate::db::TlsMode::Disable(_) => "disable".to_string(),
                },
            });
            // Which certificates the run trusts is part of its definition, so a
            // resumed run cannot silently swap the CA bundle.  Added only when
            // set, so runs without one keep their earlier hash.
            if let Some(ca) = &p.ca_file {
                v["ca_file"] = serde_json::Value::String(ca.clone());
            }
            v
        }
        Input::File(f) => serde_json::json!({ "file_sha256": f.sha256 }),
    };
    let canonical = serde_json::json!({
        "old": input(&config.old),
        "new": input(&config.new),
        "default_level": config.default_level,
        "levels": config.levels,
        "control_total_columns": config.control_total_columns,
        "column_options": config.column_options,
        "mapping_hash": config.mapping.mapping_hash,
        "export_keys": config.export_keys,
    });
    // Whether the run checks only chosen tables changes what decides its
    // verdict, so a resumed run cannot switch it.  Added only when set, so full
    // checks keep their earlier hash.
    let mut canonical = canonical;
    if config.only_chosen_tables {
        canonical["only_chosen_tables"] = serde_json::Value::Bool(true);
    }
    hex::encode(sha2::Sha256::digest(canonical.to_string().as_bytes()))
}

fn level_for(config: &RunConfig, path: &str) -> CheckLevel {
    config
        .levels
        .get(path)
        .copied()
        .unwrap_or(config.default_level)
}

/// The connection-safety record for one live side, from its profile's actual
/// TLS mode and the probe's server information (audit M5).  The superuser
/// override reason is the run-level override; `None` when no reason was
/// recorded.
pub(crate) fn connection_info(
    profile: &ConnProfile,
    info: &ServerInfo,
    superuser_reason: Option<&str>,
) -> ConnectionInfo {
    let (tls, tls_fingerprint, plaintext_reason) = match &profile.tls_mode {
        crate::db::TlsMode::VerifyFull => ("verified".to_string(), None, None),
        crate::db::TlsMode::TrustFingerprint(fp) => ("pinned".to_string(), Some(fp.clone()), None),
        crate::db::TlsMode::Disable(reason) => {
            ("plaintext".to_string(), None, Some(reason.clone()))
        }
    };
    ConnectionInfo {
        tls,
        tls_fingerprint,
        plaintext_reason,
        superuser: info.is_superuser,
        superuser_reason: if info.is_superuser {
            superuser_reason.map(str::to_string)
        } else {
            None
        },
        can_write: info.can_write,
        can_write_reasons: info.can_write_reasons.clone(),
        pooler_suspected: info.pooler_suspected,
        md5_available: info.md5_available,
        server_version: info.version_string.clone(),
        server_encoding: info.encoding.clone(),
        rolbypassrls: info.rolbypassrls,
        rls_tables: info.rls_tables.clone(),
    }
}

/// Record the plain-language run warnings the connection-safety decisions
/// imply (audit M5, H1): a login that can write, a superuser login, and
/// row-level security that Dmux refuses to read filtered.
pub(crate) fn connection_warnings(
    results: &mut RunResults,
    old: &ConnectionInfo,
    new: &ConnectionInfo,
) {
    for (side, info) in [("old", old), ("new", new)] {
        if info.can_write {
            let reasons = if info.can_write_reasons.is_empty() {
                "it can write".to_string()
            } else {
                info.can_write_reasons.join("; ")
            };
            let w = format!(
                "the {side} database login can write ({reasons}); the login should be read-only"
            );
            if !results.warnings.contains(&w) {
                results.warnings.push(w);
            }
        }
        if info.superuser {
            let why = info
                .superuser_reason
                .as_deref()
                .unwrap_or("no reason recorded");
            let w = format!("the {side} database login is a superuser (override: {why})");
            if !results.warnings.contains(&w) {
                results.warnings.push(w);
            }
        }
        if !info.rls_tables.is_empty() && !info.rolbypassrls {
            let w = format!(
                "the {side} database: {} tables use row-level security; Dmux refused to read them filtered",
                info.rls_tables.len()
            );
            if !results.warnings.contains(&w) {
                results.warnings.push(w);
            }
        }
    }
}

/// Per-column notes for one table's plan, in stable (column, then kind) order
/// (H2 / decision 23).  Re-planning is pure and cheap; a column that cannot be
/// planned faults the table elsewhere and simply contributes no note here.
pub(crate) fn collect_column_notes(spec: &TableSpec) -> Vec<ColumnNote> {
    let mut notes: Vec<ColumnNote> = Vec::new();
    for m in &spec.columns {
        if let Ok(cp) = plan_column(m) {
            notes.extend(crate::results::column_notes(&m.old_name, &cp.flags));
        }
    }
    notes.sort_by(|a, b| a.column.cmp(&b.column).then_with(|| a.kind.cmp(&b.kind)));
    notes
}

/// Run a comparison: live against live, file against live (either side), or
/// file against file.
pub async fn run(config: &RunConfig) -> Result<RunOutcome, RunError> {
    if config.old_parallelism == 0 || config.new_parallelism == 0 {
        return Err(RunError::Config(
            "the parallel-scan limits must be at least 1".into(),
        ));
    }
    let started = jiff::Timestamp::now();
    let outcome = match (&config.old, &config.new) {
        (Input::Live(o), Input::Live(n)) => run_live(config, o, n).await,
        (Input::File(o), Input::File(n)) => compare_files::compare_two_files_with(o, n, config),
        (Input::File(f), Input::Live(p)) => {
            compare_files::compare_file_to_live(f, Side::Old, p, config).await
        }
        (Input::Live(p), Input::File(f)) => {
            compare_files::compare_file_to_live(f, Side::New, p, config).await
        }
    }?;
    Ok(stamp_record(
        outcome,
        started,
        jiff::Timestamp::now(),
        &config.old,
        &config.new,
    ))
}

/// In a run of chosen tables (decision 30), keep only the structure findings
/// that belong to the compared tables, and say in one warning what was left
/// outside: other structure differences, tables only in the new database, and
/// old tables with no partner (those are also ignored items, with the reason).
fn scope_to_chosen(
    results: &mut RunResults,
    pairs: &[Pair],
    left_out: &[crate::results::StructureFinding],
    new_only: usize,
    unmapped: usize,
) {
    let chosen: BTreeSet<String> = pairs.iter().map(Pair::new_path).collect();
    let belongs = |object: &str| {
        chosen
            .iter()
            .any(|t| object == t || object.starts_with(&format!("{t}.")))
    };
    // Findings outside the chosen tables that a full check would FAIL on are
    // named (some are about data, such as a changed large-object count), so
    // they cannot be overlooked; the rest are only counted.
    // Findings on left-out tables were dropped with their table-level ignore
    // before this point; `left_out` brings them back so they are counted and,
    // when they would fail, named.
    let mut would_fail: Vec<String> = results
        .structure_findings
        .iter()
        .chain(left_out.iter())
        .filter(|f| !belongs(&f.object) && f.verdict == crate::results::Verdict::Fail)
        .map(|f| f.object.clone())
        .collect();
    would_fail.sort();
    would_fail.dedup();
    let before = results.structure_findings.len();
    results.structure_findings.retain(|f| belongs(&f.object));
    let outside = before - results.structure_findings.len() + left_out.len();
    let mut parts = Vec::new();
    if outside > 0 {
        let named = if would_fail.is_empty() {
            String::new()
        } else {
            let shown = would_fail
                .iter()
                .take(10)
                .cloned()
                .collect::<Vec<_>>()
                .join(", ");
            let more = would_fail.len().saturating_sub(10);
            // Bracketed, so the list cannot run into the next part (and a
            // function signature's own commas stay inside it).
            format!(
                " ({} of them would fail a full check: [{shown}{}])",
                would_fail.len(),
                if more > 0 {
                    format!(" and {more} more")
                } else {
                    String::new()
                }
            )
        };
        parts.push(format!(
            "{outside} structure difference{} in other tables and database objects (extensions, functions and so on){named}",
            if outside == 1 { "" } else { "s" }
        ));
    }
    if new_only > 0 {
        parts.push(format!(
            "{new_only} table{} only in the new database",
            if new_only == 1 { "" } else { "s" }
        ));
    }
    if unmapped > 0 {
        parts.push(format!(
            "{unmapped} old table{} with no partner (listed as ignored)",
            if unmapped == 1 { "" } else { "s" }
        ));
    }
    if !parts.is_empty() {
        results.warnings.push(format!(
            "this run checked only the chosen tables; not part of it: {}. Run a check of every table to review them",
            parts.join("; ")
        ));
    }
}

/// A UTC instant as RFC 3339 with whole seconds ("2026-11-18T09:14:03Z").
pub fn utc_rfc3339(t: jiff::Timestamp) -> String {
    t.strftime("%Y-%m-%dT%H:%M:%SZ").to_string()
}

/// One side in plain words for the compliance record.  Never the password.
pub fn source_label(input: &Input) -> String {
    match input {
        Input::Live(p) => format!("{} on {}:{} (login {})", p.dbname, p.host, p.port, p.user),
        Input::File(f) => format!("saved capture file (made by Dmux {})", f.tool_version),
    }
}

/// Record when the check ran and what it compared, then re-stamp the results
/// SHA-256 so the file stays self-verifying.  The verdict is untouched.
pub(crate) fn stamp_record(
    mut outcome: RunOutcome,
    started: jiff::Timestamp,
    finished: jiff::Timestamp,
    old: &Input,
    new: &Input,
) -> RunOutcome {
    let r = &mut outcome.results;
    r.started_at = Some(utc_rfc3339(started));
    r.finished_at = Some(utc_rfc3339(finished));
    r.old_source = Some(source_label(old));
    r.new_source = Some(source_label(new));
    outcome.results = crate::results::finalise(std::mem::take(&mut outcome.results));
    outcome
}

// ---------------------------------------------------------------------------
// Small read-only helpers.  Each runs in its own read-only transaction and
// returns a reason string on failure (never row data).
// ---------------------------------------------------------------------------

pub(crate) async fn inventory(s: &mut ReadOnlySession) -> Result<DatabaseInventory, RunError> {
    Ok(s.with_read_only("60s", |txn| {
        Box::pin(async move {
            crate::inventory::collect_inventory(txn)
                .await
                .map_err(|e| run_err(RunError::from(e)))
        })
    })
    .await?)
}

pub(crate) async fn live_table(
    s: &mut ReadOnlySession,
    schema: &str,
    table: &str,
) -> Result<Option<crate::plan::LiveTable>, String> {
    s.with_read_only("30s", |to| {
        Box::pin(async move {
            read_live_table_opt(&*to, schema, table)
                .await
                .map_err(|e| run_err(RunError::from(e)))
        })
    })
    .await
    .map_err(|e| e.to_string())
}

pub(crate) async fn counters(
    s: &mut ReadOnlySession,
    schema: &str,
    table: &str,
) -> Result<ChangeCounters, String> {
    s.with_read_only("10s", |to| {
        Box::pin(async move {
            read_counters(&*to, schema, table)
                .await
                .map_err(|e| run_err(RunError::from(e)))
        })
    })
    .await
    .map_err(|e| e.to_string())
}

pub(crate) async fn epoch(s: &mut ReadOnlySession) -> Option<ServerEpoch> {
    s.with_read_only("10s", |to| {
        Box::pin(async move {
            read_server_epoch(&*to)
                .await
                .map_err(|e| run_err(RunError::from(e)))
        })
    })
    .await
    .ok()
}

pub(crate) async fn replica(s: &mut ReadOnlySession, version_num: i32) -> Option<ReplicaState> {
    s.with_read_only("10s", |to| {
        Box::pin(async move {
            read_replica_state(&*to, version_num)
                .await
                .map_err(|e| run_err(RunError::from(e)))
        })
    })
    .await
    .ok()
}

/// The replica decision for one side, with an unreadable state counted as
/// moved (fail-closed).
pub(crate) fn replica_outcome(
    before: &Option<ReplicaState>,
    after: &Option<ReplicaState>,
) -> ReplicaDecision {
    match (before, after) {
        (Some(b), Some(a)) => replica_decision(b, a),
        _ => ReplicaDecision::Moved("the recovery state could not be read"),
    }
}

/// Apply a replica decision to a table's evidence: moved replay is a change
/// during the check, paused replay is recorded.
pub(crate) fn apply_replica(e: &mut verdict::Evidence, side: &str, d: &ReplicaDecision) {
    match d {
        ReplicaDecision::Moved(why) => {
            e.changed = true;
            e.notes.push(format!("{side} database: {why}"));
        }
        ReplicaDecision::Paused => e.notes.push(format!(
            "{side} database: replica replay paused during the check"
        )),
        ReplicaDecision::Steady | ReplicaDecision::NotReplica => {}
    }
}

pub(crate) async fn wal(s: &mut ReadOnlySession, version_num: i32) -> Option<String> {
    s.with_read_only("10s", |to| {
        Box::pin(async move {
            read_wal_position(&*to, version_num)
                .await
                .map_err(|e| run_err(RunError::from(e)))
        })
    })
    .await
    .ok()
    .and_then(|w| w.lsn)
}

/// Whether `track_counts` is on for a live side; an unreadable setting is
/// treated as off (fail-closed: change detection cannot vouch).
pub(crate) async fn track_counts_on(s: &mut ReadOnlySession) -> bool {
    s.with_read_only("10s", |to| {
        Box::pin(async move {
            read_track_counts(&*to)
                .await
                .map_err(|e| run_err(RunError::from(e)))
        })
    })
    .await
    .unwrap_or(false)
}

/// The `schema.table` paths of every foreign table on a live side.  A failed
/// lookup is an error, never "no foreign tables" (fail closed, M3).
pub(crate) async fn foreign_tables_set(
    s: &mut ReadOnlySession,
) -> Result<BTreeSet<String>, RunError> {
    s.with_read_only("30s", |to| {
        Box::pin(async move {
            crate::plan::foreign_tables(&*to)
                .await
                .map_err(|e| run_err(RunError::from(e)))
        })
    })
    .await
    .map_err(RunError::from)
}

/// How one side turns rows into hashes (PLAN.md 2.2): on the server with
/// `md5()`, or, where the server cannot run `md5()` (FIPS mode), in Dmux from
/// the same normalised values.  Both give identical fingerprints.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HashMode {
    Server,
    App,
}

impl HashMode {
    pub(crate) fn of(md5_available: bool) -> Self {
        if md5_available {
            HashMode::Server
        } else {
            HashMode::App
        }
    }
}

/// The hash mode of each side.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Modes {
    pub old: HashMode,
    pub new: HashMode,
}

/// The run warning for a side that hashes in the app, if any.
pub(crate) fn app_hash_warning(side: &str, mode: HashMode) -> Option<String> {
    (mode == HashMode::App).then(|| {
        format!(
            "the {side} database cannot compute md5 (for example a FIPS-mode server), so Dmux hashed its rows itself; the result is the same, the scan is slower"
        )
    })
}

/// Scan one side, or one range of it, in its own read-only transaction.
pub(crate) async fn scan_fingerprint(
    s: &mut ReadOnlySession,
    plan: &TablePlan,
    side: Side,
    src: &Source,
    mapping_hash: &str,
    mode: HashMode,
    range: Option<&RangeFilter>,
) -> Result<Fingerprint, String> {
    s.with_read_only("300s", |to| {
        Box::pin(async move {
            let r = match (mode, range) {
                (HashMode::Server, None) => fingerprint(&*to, plan, side, src, mapping_hash).await,
                (HashMode::Server, Some(r)) => {
                    fingerprint_range(&*to, plan, side, src, r, mapping_hash).await
                }
                (HashMode::App, r) => {
                    crate::core::apphash::fingerprint_app(&*to, plan, side, src, r, mapping_hash)
                        .await
                }
            };
            r.map_err(|e| run_err(RunError::from(e)))
        })
    })
    .await
    .map_err(|e| e.to_string())
}

/// Tables with at least this many rows (the larger side's estimate) are split
/// into ranges of their first key column, when it is a number or a date.
pub const SPLIT_MIN_ROWS: i64 = 2_000_000;
/// About this many rows per range.
pub const SPLIT_ROWS_PER_PART: i64 = 1_000_000;
/// At most this many ranges per table.
pub const SPLIT_MAX_PARTS: i64 = 32;

/// How many ranges a table of `rows` rows is split into (0 = not split).
pub(crate) fn split_parts_for(rows: i64) -> u32 {
    if rows < SPLIT_MIN_ROWS {
        return 0;
    }
    (rows / SPLIT_ROWS_PER_PART).clamp(2, SPLIT_MAX_PARTS) as u32
}

/// The split column for a planned table: the first key column, when both
/// sides give it the same number or date type (PLAN.md 3.1: never text).
/// Returns (old name, new name, type).
pub(crate) fn split_column(spec: &crate::plan::TableSpec) -> Option<(String, String, RangeType)> {
    let first = *spec.key.first()?;
    let m = &spec.columns[first];
    let (o, n) = (RangeType::of(&m.old_type)?, RangeType::of(&m.new_type)?);
    (o == n).then(|| (m.old_name.clone(), m.new_name.clone(), o))
}

/// Scan one side of a huge table range by range, up to `parallelism` ranges at
/// a time (each on its own read-only session), and sum the parts.  The parts
/// cover every row exactly once (`range_parts`), so the sum equals the whole
/// table's fingerprint.  A stop is honoured between ranges; the caller's
/// `checkpoint` then reports it as a cancellation, never as a result.
#[allow(clippy::too_many_arguments)]
async fn scan_side_in_ranges(
    config: &RunConfig,
    profile: &ConnProfile,
    s: &mut ReadOnlySession,
    plan: &TablePlan,
    side: Side,
    src: &Source,
    mapping_hash: &str,
    mode: HashMode,
    filters: Vec<RangeFilter>,
    parallelism: usize,
) -> Result<Fingerprint, String> {
    let workers = parallelism.clamp(1, filters.len().max(1));
    let mut extra = Vec::new();
    for _ in 1..workers {
        let x = ReadOnlySession::open(profile, config.allow_superuser)
            .await
            .map_err(|e| e.to_string())?;
        register(config, &x).map_err(|e| e.to_string())?;
        extra.push(x);
    }
    let mut queues: Vec<Vec<RangeFilter>> = vec![Vec::new(); workers];
    for (i, f) in filters.into_iter().enumerate() {
        queues[i % workers].push(f);
    }
    let sessions: Vec<&mut ReadOnlySession> = std::iter::once(s).chain(extra.iter_mut()).collect();
    let jobs = sessions
        .into_iter()
        .zip(queues)
        .map(|(sess, queue)| async move {
            let mut parts = Vec::with_capacity(queue.len());
            for f in &queue {
                if stopped(config) {
                    return Err("the run was stopped".to_string());
                }
                parts.push(
                    scan_fingerprint(sess, plan, side, src, mapping_hash, mode, Some(f)).await?,
                );
            }
            Ok::<_, String>(parts)
        });
    let mut parts = Vec::new();
    for r in futures_util::future::join_all(jobs).await {
        parts.extend(r?);
    }
    merge_fingerprints(&parts).map_err(|e| e.to_string())
}

/// The cut points for a split, read on one side.
async fn read_cuts(
    s: &mut ReadOnlySession,
    src: &Source,
    column: &str,
    ty: RangeType,
    parts: u32,
) -> Result<Vec<String>, String> {
    s.with_read_only("300s", |to| {
        Box::pin(async move {
            range_cuts(&*to, src, column, ty, parts)
                .await
                .map_err(|e| run_err(RunError::from(e)))
        })
    })
    .await
    .map_err(|e| e.to_string())
}

pub(crate) async fn scan_counts(
    s: &mut ReadOnlySession,
    schema: &str,
    table: &str,
    controls: &[String],
) -> Result<(u64, Vec<crate::core::fingerprint::Decimal>), String> {
    s.with_read_only("300s", |to| {
        Box::pin(async move {
            scan::counts_and_totals(&*to, schema, table, controls)
                .await
                .map_err(|e| run_err(RunError::from(e)))
        })
    })
    .await
    .map_err(|e| e.to_string())
}

async fn drill_rows(
    s: &mut ReadOnlySession,
    plan: &TablePlan,
    side: Side,
    src: &Source,
    buckets: &[u32],
    mode: HashMode,
) -> Result<Vec<crate::core::drilldown::RowRef>, String> {
    s.with_read_only("300s", |to| {
        Box::pin(async move {
            let r = match mode {
                HashMode::Server => fetch_rows(&*to, plan, side, src, buckets).await,
                HashMode::App => {
                    crate::core::apphash::fetch_rows_app(&*to, plan, side, src, buckets).await
                }
            };
            r.map_err(|e| run_err(RunError::from(e)))
        })
    })
    .await
    .map_err(|e| e.to_string())
}

async fn drill_values(
    s: &mut ReadOnlySession,
    plan: &TablePlan,
    side: Side,
    src: &Source,
    hashes: &[String],
    mode: HashMode,
) -> Result<std::collections::HashMap<String, Vec<Option<String>>>, String> {
    s.with_read_only("300s", |to| {
        Box::pin(async move {
            let r = match mode {
                HashMode::Server => scan::values_by_key_hash(&*to, plan, side, src, hashes).await,
                HashMode::App => {
                    crate::core::apphash::values_by_key_hash_app(&*to, plan, side, src, hashes)
                        .await
                }
            };
            r.map_err(|e| run_err(RunError::from(e)))
        })
    })
    .await
    .map_err(|e| e.to_string())
}

async fn spot_sample(
    s: &mut ReadOnlySession,
    plan: &TablePlan,
    side: Side,
    src: &Source,
) -> Result<Vec<spot::SpotRow>, String> {
    s.with_read_only("300s", |to| {
        Box::pin(async move {
            spot::sample(&*to, plan, side, src)
                .await
                .map_err(|e| run_err(RunError::from(e)))
        })
    })
    .await
    .map_err(|e| e.to_string())
}

async fn spot_fetch(
    s: &mut ReadOnlySession,
    plan: &TablePlan,
    side: Side,
    src: &Source,
    khs: &[String],
) -> Result<Vec<spot::SpotRow>, String> {
    s.with_read_only("300s", |to| {
        Box::pin(async move {
            spot::fetch(&*to, plan, side, src, khs)
                .await
                .map_err(|e| run_err(RunError::from(e)))
        })
    })
    .await
    .map_err(|e| e.to_string())
}

async fn sample_time(
    s: &mut ReadOnlySession,
    schema: &str,
    table: &str,
    rows: u64,
) -> Result<std::time::Duration, String> {
    s.with_read_only("60s", |to| {
        Box::pin(async move {
            timed_sample(&*to, schema, table, rows)
                .await
                .map_err(|e| DbError::Postgres {
                    message: e.to_string(),
                })
        })
    })
    .await
    .map_err(|e| e.to_string())
}

// ---------------------------------------------------------------------------
// The live run.
// ---------------------------------------------------------------------------

/// One old table paired with an existing new table.
#[derive(Debug, Clone)]
struct Pair {
    path: String,
    old: (String, String),
    new: (String, String),
    level: CheckLevel,
    keyed_hint: bool,
    old_size: i64,
    new_size: i64,
    old_rows: i64,
    new_rows: i64,
}

impl Pair {
    fn new_path(&self) -> String {
        format!("{}.{}", self.new.0, self.new.1)
    }
}

/// Before/after change counters of one table (None: could not be read).
type CounterPair = Option<(ChangeCounters, ChangeCounters)>;

/// Whether a table's (old, new) counters moved; anything unreadable is moved.
pub(crate) fn counters_moved(before: &CounterPair, after: Option<&CounterPair>) -> bool {
    match (before, after) {
        (Some((bo, bn)), Some(Some((ao, an)))) => bo.moved(ao) || bn.moved(an),
        _ => true,
    }
}

/// Whether the server epoch changed between two reads (an unreadable read
/// after a readable one counts as changed).
pub(crate) fn epoch_changed(start: &Option<ServerEpoch>, end: &Option<ServerEpoch>) -> bool {
    match (start, end) {
        (Some(a), Some(b)) => a != b,
        (None, None) => false,
        _ => true,
    }
}

/// Record "replay paused" once per side in the run warnings.
pub(crate) fn note_paused(results: &mut RunResults, side: &str, d: &ReplicaDecision) {
    if *d == ReplicaDecision::Paused {
        let w = format!("{side} database is a replica with replay paused during the check");
        if !results.warnings.contains(&w) {
            results.warnings.push(w);
        }
    }
}

async fn snapshot(
    config: &RunConfig,
    old_s: &mut ReadOnlySession,
    new_s: &mut ReadOnlySession,
    pairs: &[Pair],
) -> Result<BTreeMap<String, CounterPair>, RunError> {
    let mut out = BTreeMap::new();
    for p in pairs.iter().filter(|p| p.level != CheckLevel::Structure) {
        checkpoint(config)?;
        let o = counters(old_s, &p.old.0, &p.old.1).await;
        let n = counters(new_s, &p.new.0, &p.new.1).await;
        out.insert(p.path.clone(), o.ok().zip(n.ok()));
    }
    Ok(out)
}

/// Size-on-disk plus a timed LIMIT sample per side (PLAN.md 4.5).
async fn estimates(
    old_s: &mut ReadOnlySession,
    new_s: &mut ReadOnlySession,
    pairs: &[Pair],
) -> (Option<Estimate>, BTreeMap<String, Estimate>) {
    const SAMPLE: u64 = 10_000;
    let data: Vec<&Pair> = pairs
        .iter()
        .filter(|p| p.level != CheckLevel::Structure)
        .collect();
    // Bytes per second on one side, timed on its largest data table.
    async fn rate(
        s: &mut ReadOnlySession,
        biggest: Option<(&(String, String), i64, i64)>,
    ) -> Option<f64> {
        let ((schema, table), size, rows) = biggest?;
        if size <= 0 {
            return None;
        }
        let took = sample_time(s, schema, table, SAMPLE).await.ok()?;
        let share = if rows > 0 {
            (SAMPLE as f64 / rows as f64).min(1.0)
        } else {
            1.0
        };
        Some(size as f64 * share / took.as_secs_f64().max(0.001))
    }
    let big_old = data
        .iter()
        .max_by_key(|p| p.old_size)
        .map(|p| (&p.old, p.old_size, p.old_rows));
    let big_new = data
        .iter()
        .max_by_key(|p| p.new_size)
        .map(|p| (&p.new, p.new_size, p.new_rows));
    let (Some(ro), Some(rn)) = (rate(old_s, big_old).await, rate(new_s, big_new).await) else {
        return (None, BTreeMap::new());
    };
    let mut per = BTreeMap::new();
    for p in &data {
        // A keyed full-values table is read twice per side (fingerprint and
        // spot-check sample); everything else once.
        let passes = if p.level == CheckLevel::FullValues && p.keyed_hint {
            2.0
        } else {
            1.0
        };
        let secs = passes * (p.old_size.max(0) as f64 / ro + p.new_size.max(0) as f64 / rn);
        per.insert(p.path.clone(), estimate_from_seconds(secs));
    }
    let total = total_estimate(&per.values().copied().collect::<Vec<_>>());
    (Some(total), per)
}

async fn run_live(
    config: &RunConfig,
    old_profile: &ConnProfile,
    new_profile: &ConnProfile,
) -> Result<RunOutcome, RunError> {
    let mapping_hash = config.mapping.mapping_hash.clone();
    let def_hash = definition_hash(config);

    // The probes use short-lived sessions of their own; the stop flag is
    // checked around them.  Every session the run keeps is registered.
    checkpoint(config)?;
    let old_info = probe_server(old_profile, config.allow_superuser).await?;
    checkpoint(config)?;
    let new_info = probe_server(new_profile, config.allow_superuser).await?;
    checkpoint(config)?;
    let mut old_s = ReadOnlySession::open(old_profile, config.allow_superuser).await?;
    register(config, &old_s)?;
    let mut new_s = ReadOnlySession::open(new_profile, config.allow_superuser).await?;
    register(config, &new_s)?;

    // Resume store: bound to this run's key; the run id survives resumes.
    let store = match config.resume_store.as_deref() {
        Some(path) => Some(Store::open(path)?),
        None => None,
    };
    let run_id = match &store {
        Some(s) => s.bind(
            &RunKey {
                definition_hash: def_hash.clone(),
                mapping_hash: mapping_hash.clone(),
                tool_version: crate::version().to_string(),
            },
            &fresh_run_id(),
        )?,
        None => fresh_run_id(),
    };

    let mut b = Builder::new(run_id, mapping_hash.clone(), def_hash);
    b.set_sql_ascii(old_info.sql_ascii || new_info.sql_ascii);
    // md5 unavailable on a side (FIPS): hash that side in the app (PLAN 2.2).
    let modes = Modes {
        old: HashMode::of(old_info.md5_available),
        new: HashMode::of(new_info.md5_available),
    };
    for (side, mode) in [("old", modes.old), ("new", modes.new)] {
        if let Some(w) = app_hash_warning(side, mode) {
            b.results.warnings.push(w);
        }
    }
    if old_info.sql_ascii {
        b.results
            .warnings
            .push("old database is SQL_ASCII; never a clean PASS on encoding (P19)".into());
    }
    if new_info.sql_ascii {
        b.results
            .warnings
            .push("new database is SQL_ASCII; never a clean PASS on encoding (P19)".into());
    }
    // Connection-safety decisions reach the results and page 1 of the report
    // (audit M5): TLS state, superuser override and write access become plain
    // warnings a signer can read.
    let old_conn = connection_info(
        old_profile,
        &old_info,
        config.allow_superuser_reason.as_deref(),
    );
    let new_conn = connection_info(
        new_profile,
        &new_info,
        config.allow_superuser_reason.as_deref(),
    );
    connection_warnings(&mut b.results, &old_conn, &new_conn);
    b.results.connections = Connections {
        old: old_conn,
        new: new_conn,
    };
    b.results.wal_start_lsn = wal(&mut new_s, new_info.version_num).await;
    // What the counters are relative to (see `crate::consistency`).
    let epoch_old = epoch(&mut old_s).await;
    let epoch_new = epoch(&mut new_s).await;
    // Change-detection availability (M3): when `track_counts` is off on a side
    // the counters never move, so every data-level table on that side is
    // floored at NEEDS_REVIEW and a stored result cannot be reused.
    let old_track_counts = track_counts_on(&mut old_s).await;
    let new_track_counts = track_counts_on(&mut new_s).await;

    // Structure: both inventories, compared after the mapping is applied.
    checkpoint(config)?;
    let old_inv = inventory(&mut old_s).await;
    checkpoint(config)?;
    let new_inv = inventory(&mut new_s).await;
    checkpoint(config)?;
    let (old_inv, new_inv) = (old_inv?, new_inv?);
    // L4: a level, column-option or ignore key that names no table or column is
    // a configuration error, never a silent no-op.
    let unknown = crate::plan::unknown_paths(
        &old_inv,
        &new_inv,
        &config.levels,
        &config.column_options,
        &config.mapping.ignored,
    );
    if !unknown.is_empty() {
        return Err(RunError::Config(format!(
            "unknown paths in the run definition or mapping: {}",
            unknown.join(", ")
        )));
    }
    // Foreign tables have no change counters (M3).
    let old_foreign = foreign_tables_set(&mut old_s).await?;
    let new_foreign = foreign_tables_set(&mut new_s).await?;
    let findings = structure::apply(&mut b, &old_inv, &new_inv, &config.mapping);

    // Pair every old table with its mapped new table.
    let table_ignores: BTreeMap<String, String> = config
        .mapping
        .ignored
        .iter()
        .filter(|i| i.path.matches('.').count() == 1)
        .map(|i| (i.path.clone(), i.reason.clone()))
        .collect();
    let mut new_tables = BTreeMap::new();
    for s in &new_inv.schemas {
        for t in &s.tables {
            new_tables.insert(format!("{}.{}", s.name, t.name), (s.name.clone(), t));
        }
    }
    let mut targets = BTreeSet::new();
    let mut pairs: Vec<Pair> = Vec::new();
    let mut outside_unmapped = 0usize;
    let mut mapped: Vec<(String, String)> = Vec::new();
    for s in &old_inv.schemas {
        for t in &s.tables {
            let path = format!("{}.{}", s.name, t.name);
            let (ns, nt) = map_table(&config.mapping, &s.name, &t.name);
            let new_path = format!("{ns}.{nt}");
            targets.insert(new_path.clone());
            if let Some(reason) = table_ignores.get(&path) {
                b.ignore_table(&path, reason);
                continue;
            }
            mapped.push((path.clone(), new_path.clone()));
            let level = level_for(config, &path);
            match new_tables.get(&new_path) {
                None if config.only_chosen_tables => {
                    b.ignore_table(&path, NOT_MAPPED_OUTSIDE_RUN);
                    outside_unmapped += 1;
                }
                None => b.record(Evidence::new(&path, level, Outcome::MissingOnNew)),
                Some((_, new_t)) => pairs.push(Pair {
                    path,
                    old: (s.name.clone(), t.name.clone()),
                    new: (ns, nt),
                    level,
                    keyed_hint: t
                        .constraints
                        .iter()
                        .any(|c| c.kind == ConstraintKind::PrimaryKey),
                    old_size: t.size_bytes,
                    new_size: new_t.size_bytes,
                    old_rows: t.n_live_tup,
                    new_rows: new_t.n_live_tup,
                }),
            }
        }
    }
    // M2: refuse two old tables mapping to one new table (the mapping does not
    // say which old table the new one is, so neither may PASS against it).
    let duplicates = crate::mapping::duplicate_table_targets(&mapped);
    if !duplicates.is_empty() {
        let mut kept = Vec::with_capacity(pairs.len());
        for p in pairs {
            let np = p.new_path();
            if let Some(olds) = duplicates.get(&np) {
                b.record(Evidence::new(
                    &p.path,
                    p.level,
                    Outcome::Fault(format!(
                        "{} old tables ({}) map to new table {np}",
                        if olds.len() == 2 { "two" } else { "multiple" },
                        olds.join(", ")
                    )),
                ));
            } else {
                kept.push(p);
            }
        }
        pairs = kept;
    }
    let mut outside_new_only = 0usize;
    for (new_path, _) in new_tables.iter().filter(|(p, _)| !targets.contains(*p)) {
        if let Some(reason) = table_ignores.get(new_path) {
            b.ignore_table(new_path, reason);
        } else if config.only_chosen_tables {
            outside_new_only += 1;
        } else {
            b.record(Evidence::new(
                new_path,
                config.default_level,
                Outcome::OnlyOnNew,
            ));
        }
    }

    if config.only_chosen_tables {
        let left_out = structure::left_out_findings(&old_inv, &new_inv, &config.mapping);
        scope_to_chosen(
            &mut b.results,
            &pairs,
            &left_out,
            outside_new_only,
            outside_unmapped,
        );
    }

    checkpoint(config)?;
    let (total, per) = estimates(&mut old_s, &mut new_s, &pairs).await;
    checkpoint(config)?;
    b.results.estimate = total;
    if let Some(c) = &config.control {
        let plan = per.clone();
        c.with_live(|l| l.set_plan(plan));
    }
    b.results.table_estimates = per;

    // The run window is honoured before the first snapshot as well, so the
    // wait does not count against "changed during the check".
    wait_for_window(
        config,
        &mut b.results,
        crate::run_window::local_minute_of_day,
        WINDOW_POLL,
    )
    .await?;
    let before = snapshot(config, &mut old_s, &mut new_s, &pairs).await?;

    for p in &pairs {
        // Stop before each table: finished tables are already in the store.
        checkpoint(config)?;
        // Outside the run window: wait here, between tables, never mid-scan.
        wait_for_window(
            config,
            &mut b.results,
            crate::run_window::local_minute_of_day,
            WINDOW_POLL,
        )
        .await?;
        let started = Instant::now();
        if p.level == CheckLevel::Structure {
            let mine: Vec<_> = findings
                .iter()
                .filter(|f| structure::table_of(f).as_deref() == Some(p.new_path().as_str()))
                .cloned()
                .collect();
            b.record(Evidence::new(&p.path, p.level, Outcome::Structure(mine)));
            continue;
        }
        let Some(Some((bo, bn))) = before.get(&p.path).cloned() else {
            b.record(Evidence::new(
                &p.path,
                p.level,
                Outcome::Fault(
                    "the change counters could not be read, so changes during the check cannot be ruled out"
                        .into(),
                ),
            ));
            continue;
        };

        // Change-detection availability for this table (M3).
        let old_cd = SideChangeDetection::of(
            old_track_counts,
            old_foreign.contains(&format!("{}.{}", p.old.0, p.old.1)),
        );
        let new_cd = SideChangeDetection::of(new_track_counts, new_foreign.contains(&p.new_path()));
        let cd_floor = change_detection_floor(old_cd, new_cd);

        let ro = replica(&mut old_s, old_info.version_num).await;
        let rn = replica(&mut new_s, new_info.version_num).await;

        // Resume: reuse a stored conclusive result only if nothing that could
        // vouch for the table moved since it was stored (counters, storage,
        // server epoch, replica replay).  Anything else is rescanned.
        if let (Some(s), Some(ro), Some(rn)) = (&store, &ro, &rn) {
            if let Loaded::Found(st) = s.load_table(&p.path)? {
                let now_old = SideMark {
                    counters: bo.clone(),
                    epoch: epoch_old.clone().unwrap_or_default(),
                    replica: ro.clone(),
                    change_detection_ok: old_cd.ok(),
                };
                let now_new = SideMark {
                    counters: bn.clone(),
                    epoch: epoch_new.clone().unwrap_or_default(),
                    replica: rn.clone(),
                    change_detection_ok: new_cd.ok(),
                };
                if st.rescan_reason(p.level, &now_old, &now_new).is_none() {
                    b.record_resumed(st.result.clone());
                    continue;
                }
            }
        }

        let scan_started = Instant::now();
        if let Some(c) = &config.control {
            c.with_live(|l| l.table_started(&p.path, scan_started));
        }
        let mut e = scan_pair(
            config,
            &mut old_s,
            &mut new_s,
            p,
            &mapping_hash,
            &mut b.results,
            modes,
        )
        .await?;
        e.facts.change_detection_old = Some(change_detection_word(old_cd));
        e.facts.change_detection_new = Some(change_detection_word(new_cd));
        // A stop during the last query (which the server cancelled) must
        // never be recorded or stored as this table's result.
        checkpoint(config)?;
        // Change detection unavailable (track_counts off or a foreign table):
        // floor the table at NEEDS_REVIEW, never a clean PASS (M3).
        if let Some((floor, reason)) = cd_floor {
            e.floors.push((floor, reason));
        }

        // Replica replay during this table's scans (counters are blind there).
        let ro_after = replica(&mut old_s, old_info.version_num).await;
        let rn_after = replica(&mut new_s, new_info.version_num).await;
        for (side, d) in [
            ("old", replica_outcome(&ro, &ro_after)),
            ("new", replica_outcome(&rn, &rn_after)),
        ] {
            note_paused(&mut b.results, side, &d);
            apply_replica(&mut e, side, &d);
        }

        // Per-table change check, and the marks stored for resume.
        let ao = counters(&mut old_s, &p.old.0, &p.old.1).await;
        let an = counters(&mut new_s, &p.new.0, &p.new.1).await;
        checkpoint(config)?;
        match (ao, an) {
            (Ok(ao), Ok(an)) => {
                if bo.moved(&ao) || bn.moved(&an) {
                    e.changed = true;
                }
                if let Some(s) = &store {
                    // The final, flag-applied verdict (P19 included).
                    let (result, _) = b.decide_now(&e);
                    s.save_table(&StoredTableResult {
                        table: p.path.clone(),
                        level: p.level,
                        result,
                        old: SideMark {
                            counters: ao,
                            epoch: epoch_old.clone().unwrap_or_default(),
                            replica: ro_after.clone().unwrap_or_default(),
                            change_detection_ok: old_cd.ok(),
                        },
                        new: SideMark {
                            counters: an,
                            epoch: epoch_new.clone().unwrap_or_default(),
                            replica: rn_after.clone().unwrap_or_default(),
                            change_detection_ok: new_cd.ok(),
                        },
                        wal_lsn: wal(&mut new_s, new_info.version_num).await,
                    })?;
                }
            }
            _ => e.changed = true,
        }
        b.record(e);
        b.results
            .table_elapsed_ms
            .insert(p.path.clone(), started.elapsed().as_millis() as u64);
        if let Some(c) = &config.control {
            c.with_live(|l| l.table_finished(&p.path, scan_started.elapsed().as_secs_f64()));
        }
    }

    // Run-level change sweep: any table that moved since the first snapshot is
    // CHANGED_DURING_CHECK, never PASS.  Unreadable counters count as moved.
    // Wait first, so writes made during the run reach the statistics.
    settle(config, config.stats_settle).await?;
    let after = snapshot(config, &mut old_s, &mut new_s, &pairs).await?;
    // A restart or stats reset during the run makes every counter comparison
    // meaningless: every scanned table is treated as changed.
    let epoch_moved = epoch_changed(&epoch_old, &epoch(&mut old_s).await)
        || epoch_changed(&epoch_new, &epoch(&mut new_s).await);
    if epoch_moved {
        b.results.warnings.push(
            "a server restarted or its statistics were reset during the run; changes cannot be ruled out"
                .into(),
        );
    }
    for (path, b4) in &before {
        if epoch_moved || counters_moved(b4, after.get(path)) {
            b.mark_changed(path);
        }
    }

    b.results.wal_end_lsn = wal(&mut new_s, new_info.version_num).await;
    // A stop that arrived during the last reads still wins: no results.
    checkpoint(config)?;
    Ok(b.finish())
}

/// Scan one paired table at its level and return the evidence.  Detail for
/// the report (changed columns, exported keys, differing control totals) goes
/// into `results.details`.  The stop flag is checked before every query;
/// a stop is `Err(RunError::Cancelled)`, never a table result.
async fn scan_pair(
    config: &RunConfig,
    old_s: &mut ReadOnlySession,
    new_s: &mut ReadOnlySession,
    p: &Pair,
    mapping_hash: &str,
    results: &mut RunResults,
    modes: Modes,
) -> Result<Evidence, RunError> {
    let fault = |why: String| Ok(Evidence::new(&p.path, p.level, Outcome::Fault(why)));
    checkpoint(config)?;
    let old_live = live_table(old_s, &p.old.0, &p.old.1).await;
    checkpoint(config)?;
    let old_live = match old_live {
        Ok(Some(t)) => t,
        Ok(None) => {
            return fault("the table disappeared from the old database during the run".into())
        }
        Err(e) => return fault(format!("the old table could not be read: {e}")),
    };
    let new_live = live_table(new_s, &p.new.0, &p.new.1).await;
    checkpoint(config)?;
    let new_live = match new_live {
        Ok(Some(t)) => t,
        Ok(None) => {
            return fault("the table disappeared from the new database during the run".into())
        }
        Err(e) => return fault(format!("the new table could not be read: {e}")),
    };
    let spec = build_table_spec(
        &old_live,
        &new_live,
        SpecInputs {
            new_schema: &p.new.0,
            new_name: &p.new.1,
            level: p.level,
            mapping: &config.mapping,
            control_total_columns: &config.control_total_columns,
            column_options: &config.column_options,
            estimated_rows: p.old_rows.max(p.new_rows).max(0) as u64,
        },
    );
    // M2: two old columns pairing with the same new column is refused.
    if let Some(dup) = crate::plan::duplicate_column_target(&spec) {
        return fault(dup.to_string());
    }

    let notes = collect_column_notes(&spec);
    let with_coverage = |e: Result<Evidence, RunError>| {
        e.map(|mut e| {
            e.dropped = spec.dropped.clone();
            e.added = spec.added.clone();
            e.ignored_columns = spec.ignored_columns.clone();
            e.columns_covered = Some(spec.columns_covered());
            e.column_notes = notes.clone();
            e.facts.key_columns = spec
                .key
                .iter()
                .filter_map(|i| spec.columns.get(*i))
                .map(|c| c.old_name.clone())
                .collect();
            e.facts.columns = spec.columns.iter().map(column_rule_fact).collect();
            e
        })
    };
    if spec.columns.is_empty() {
        return with_coverage(fault(
            "no column could be paired between the two sides".into(),
        ));
    }
    let old_src = Source::Table {
        schema: p.old.0.clone(),
        name: p.old.1.clone(),
    };
    let new_src = Source::Table {
        schema: p.new.0.clone(),
        name: p.new.1.clone(),
    };

    match p.level {
        CheckLevel::Structure => unreachable!("structure-level tables are not scanned"),
        CheckLevel::CountsAndTotals => {
            let mut old_controls = Vec::new();
            let mut new_controls = Vec::new();
            let mut control_names = Vec::new();
            for m in &spec.columns {
                match plan_column(m) {
                    Ok(cp) => {
                        if let (Some(o), Some(n)) = (cp.old_control, cp.new_control) {
                            old_controls.push(o);
                            new_controls.push(n);
                            control_names.push(m.old_name.clone());
                        }
                    }
                    Err(e) => {
                        return with_coverage(fault(format!(
                            "column {} cannot be planned: {e}",
                            m.old_name
                        )))
                    }
                }
            }
            checkpoint(config)?;
            let o = scan_counts(old_s, &p.old.0, &p.old.1, &old_controls).await;
            checkpoint(config)?;
            let n = scan_counts(new_s, &p.new.0, &p.new.1, &new_controls).await;
            checkpoint(config)?;
            match (o, n) {
                (Ok((orows, ototals)), Ok((nrows, ntotals))) => {
                    export_control_totals(results, &p.path, &control_names, &ototals, &ntotals);
                    let facts = control_facts(&control_names, &ototals, &ntotals);
                    with_coverage(Ok(Evidence::new(
                        &p.path,
                        p.level,
                        Outcome::Counts(CountsComparison {
                            old_rows: orows,
                            new_rows: nrows,
                            old_totals: ototals,
                            new_totals: ntotals,
                        }),
                    )))
                    .map(|mut e| {
                        e.facts.control_totals = facts;
                        e
                    })
                }
                (Err(e), _) | (_, Err(e)) => {
                    with_coverage(fault(format!("count scan failed: {e}")))
                }
            }
        }
        CheckLevel::FullValues => {
            let plan = match core_table_plan(&spec) {
                Ok(plan) => plan,
                Err(e) => return with_coverage(fault(format!("the plan cannot be built: {e}"))),
            };
            checkpoint(config)?;
            // Huge tables are split into ranges of a number or date key column
            // (PLAN.md 3.1); the same cut points are used on both sides.
            let parts = split_parts_for(p.old_rows.max(p.new_rows));
            let split = match (parts, split_column(&spec), &config.old, &config.new) {
                (n, Some(col), Input::Live(op), Input::Live(np)) if n >= 2 => {
                    match read_cuts(old_s, &old_src, &col.0, col.2, n).await {
                        Ok(cuts) => range_parts(&cuts).map(|ps| (col, ps, op, np)),
                        Err(why) => {
                            return with_coverage(fault(format!(
                                "the ranges for splitting this table could not be read: {why}"
                            )))
                        }
                    }
                }
                _ => None,
            };
            checkpoint(config)?;
            let (ofp, nfp) = match split {
                None => {
                    let ofp = scan_fingerprint(
                        old_s,
                        &plan,
                        Side::Old,
                        &old_src,
                        mapping_hash,
                        modes.old,
                        None,
                    )
                    .await;
                    checkpoint(config)?;
                    let nfp = scan_fingerprint(
                        new_s,
                        &plan,
                        Side::New,
                        &new_src,
                        mapping_hash,
                        modes.new,
                        None,
                    )
                    .await;
                    (ofp, nfp)
                }
                Some(((old_col, new_col, ty), ps, op, np)) => {
                    let filters = |column: &str| -> Vec<RangeFilter> {
                        ps.iter()
                            .map(|part| RangeFilter {
                                column: column.to_string(),
                                ty,
                                part: part.clone(),
                            })
                            .collect()
                    };
                    let ofp = scan_side_in_ranges(
                        config,
                        op,
                        old_s,
                        &plan,
                        Side::Old,
                        &old_src,
                        mapping_hash,
                        modes.old,
                        filters(&old_col),
                        config.old_parallelism,
                    )
                    .await;
                    checkpoint(config)?;
                    let nfp = scan_side_in_ranges(
                        config,
                        np,
                        new_s,
                        &plan,
                        Side::New,
                        &new_src,
                        mapping_hash,
                        modes.new,
                        filters(&new_col),
                        config.new_parallelism,
                    )
                    .await;
                    results.warnings.push(format!(
                        "{}: scanned in {} ranges of column {old_col} (a large table)",
                        p.path,
                        ps.len()
                    ));
                    (ofp, nfp)
                }
            };
            checkpoint(config)?;
            let (ofp, nfp) = match (ofp, nfp) {
                (Ok(a), Ok(b)) => (a, b),
                (Err(e), _) | (_, Err(e)) => {
                    return with_coverage(fault(format!("fingerprint scan failed: {e}")))
                }
            };
            let comparison = match compare(SideOutcome::Ok(&ofp), SideOutcome::Ok(&nfp)) {
                Ok(c) => c,
                Err(e) => {
                    return with_coverage(fault(format!("fingerprints cannot be compared: {e}")))
                }
            };
            let names: Vec<String> = spec.columns.iter().map(|c| c.old_name.clone()).collect();
            let control_names: Vec<String> = plan
                .columns()
                .iter()
                .zip(names.iter())
                .filter(|(c, _)| c.old_control.is_some())
                .map(|(_, n)| n.clone())
                .collect();
            export_control_totals(
                results,
                &p.path,
                &control_names,
                &ofp.control_totals,
                &nfp.control_totals,
            );

            let mut e = with_coverage(Ok(Evidence::new(
                &p.path,
                p.level,
                Outcome::Full(comparison.clone()),
            )))?;
            e.facts.control_totals =
                control_facts(&control_names, &ofp.control_totals, &nfp.control_totals);
            drill(
                config,
                old_s,
                new_s,
                &plan,
                &old_src,
                &new_src,
                &comparison,
                &names,
                &p.path,
                &mut e,
                results,
                modes,
            )
            .await?;
            e.spot = if plan.is_keyed() {
                spot_check(config, old_s, new_s, &plan, &old_src, &new_src, &comparison).await?
            } else {
                Spot::NotRun("the table has no key; it is compared as a counted set".into())
            };
            Ok(e)
        }
    }
}

fn change_detection_word(c: crate::consistency::SideChangeDetection) -> String {
    use crate::consistency::SideChangeDetection as S;
    match c {
        S::Ok => "available",
        S::TrackCountsOff => "track_counts_off",
        S::ForeignTable => "foreign_table",
    }
    .to_string()
}

/// Every summed column with both sides' totals (sums, never row values).
fn control_facts(
    names: &[String],
    old: &[crate::core::fingerprint::Decimal],
    new: &[crate::core::fingerprint::Decimal],
) -> Vec<crate::results::ControlTotalFact> {
    names
        .iter()
        .zip(old.iter().zip(new.iter()))
        .map(|(c, (o, n))| crate::results::ControlTotalFact {
            column: c.clone(),
            old: o.to_string(),
            new: n.to_string(),
        })
        .collect()
}

/// One column's comparison rule in plain words, from its types and ticked options.
pub(crate) fn column_rule_text(m: &crate::core::normalise::ColumnMapping) -> String {
    use crate::core::normalise::PgType as T;
    let (o, n) = (m.old_type.base(), m.new_type.base());
    let mut rule = match (o, n) {
        (T::Float4, T::Float8) | (T::Float8, T::Float4) => {
            "compared by its exact 8-byte binary form (the 4-byte side is widened)".to_string()
        }
        (T::Float4, _) | (_, T::Float4) => "compared by its exact 4-byte binary form".to_string(),
        (T::Float8, _) | (_, T::Float8) => "compared by its exact 8-byte binary form".to_string(),
        (T::Geometric(_), _) => "compared by its exact binary form".to_string(),
        (T::Bytea, _) => "compared by a fingerprint of its raw bytes".to_string(),
        (T::Numeric(Some(a)), T::Numeric(Some(b))) if a.1 != b.1 => {
            "compared as an exact decimal, both sides cast to the larger number of decimals"
                .to_string()
        }
        (T::Numeric(_) | T::Money, _) => {
            "compared as an exact decimal, digit for digit".to_string()
        }
        (T::Int2 | T::Int4 | T::Int8, _) => "compared as an exact whole number".to_string(),
        (T::Date | T::Timestamp | T::Timestamptz | T::Time | T::Timetz | T::Interval, _) => {
            "compared as an exact date or time value".to_string()
        }
        (T::Json | T::Jsonb, _) => {
            "compared as normalised JSON (key order does not matter)".to_string()
        }
        (T::Array(_), _) => "compared element by element".to_string(),
        (T::Range { .. } | T::Composite { .. } | T::Unknown(_) | T::Xml, _) => {
            "compared by its printed form, which is not provably exact: needs review".to_string()
        }
        _ => "compared exactly as text".to_string(),
    };
    let opts = &m.options;
    if let Some(d) = opts.round_decimals {
        rule.push_str(&format!(
            "; rounded to {d} decimals because you ticked that option"
        ));
    }
    if opts.ignore_trailing_spaces {
        rule.push_str("; trailing spaces ignored because you ticked that option");
    }
    if opts.null_equals_empty {
        rule.push_str("; an empty value counts as NULL because you ticked that option");
    }
    if opts.date_only {
        rule.push_str("; only the date part compared because you ticked that option");
    }
    rule
}

fn column_rule_fact(m: &crate::core::normalise::ColumnMapping) -> crate::results::ColumnRule {
    crate::results::ColumnRule {
        column: m.old_name.clone(),
        rule: column_rule_text(m),
    }
}

fn export_control_totals(
    results: &mut RunResults,
    table: &str,
    names: &[String],
    old: &[crate::core::fingerprint::Decimal],
    new: &[crate::core::fingerprint::Decimal],
) {
    let differ: Vec<ControlTotalExport> = names
        .iter()
        .zip(old.iter().zip(new.iter()))
        .filter(|(_, (o, n))| o != n)
        .map(|(c, (o, n))| ControlTotalExport {
            column: c.clone(),
            old: o.to_string(),
            new: n.to_string(),
        })
        .collect();
    if !differ.is_empty() {
        results
            .details
            .entry(table.to_string())
            .or_default()
            .control_totals = differ;
    }
}

/// The drill-down for a table with differing buckets: missing, extra and
/// changed rows, and for up to 100 changed rows the differing columns.
#[allow(clippy::too_many_arguments)]
async fn drill(
    config: &RunConfig,
    old_s: &mut ReadOnlySession,
    new_s: &mut ReadOnlySession,
    plan: &TablePlan,
    old_src: &Source,
    new_src: &Source,
    comparison: &Comparison,
    names: &[String],
    table: &str,
    e: &mut Evidence,
    results: &mut RunResults,
    modes: Modes,
) -> Result<(), RunError> {
    if comparison.non_equal().is_empty() || comparison.verdict == CoreVerdict::Error {
        return Ok(());
    }
    let buckets = comparison.drilldown_buckets();
    checkpoint(config)?;
    let o = drill_rows(old_s, plan, Side::Old, old_src, &buckets, modes.old).await;
    checkpoint(config)?;
    let n = drill_rows(new_s, plan, Side::New, new_src, &buckets, modes.new).await;
    checkpoint(config)?;
    let (o, n) = match (o, n) {
        (Ok(o), Ok(n)) => (o, n),
        (Err(why), _) | (_, Err(why)) => {
            e.floors.push((
                crate::results::Verdict::ToolFault,
                format!("the drill-down scan failed: {why}"),
            ));
            return Ok(());
        }
    };
    let d = classify(o, n);
    e.missing_rows = Some(d.missing.len());
    e.extra_rows = Some(d.extra.len());
    e.changed_rows = Some(d.changed.len());
    if !d.explains_difference() {
        e.drill_unexplained = true;
    }

    let mut detail = TableDetail::default();
    if config.export_keys {
        detail.missing_keys = Some(d.missing.iter().map(|r| r.key.clone()).collect());
        detail.extra_keys = Some(d.extra.iter().map(|r| r.key.clone()).collect());
    }
    if plan.is_keyed() && !d.changed.is_empty() {
        let take = &d.changed[..d
            .changed
            .len()
            .min(crate::core::drilldown::CHANGED_DETAIL_LIMIT)];
        let hashes: Vec<String> = take.iter().map(|(o, _)| o.key_hash.clone()).collect();
        checkpoint(config)?;
        let ov = drill_values(old_s, plan, Side::Old, old_src, &hashes, modes.old).await;
        checkpoint(config)?;
        let nv = drill_values(new_s, plan, Side::New, new_src, &hashes, modes.new).await;
        checkpoint(config)?;
        match (ov, nv) {
            (Ok(ov), Ok(nv)) => {
                let mut all_cols = BTreeSet::new();
                for (o, _) in take {
                    let (Some(a), Some(b)) = (ov.get(&o.key_hash), nv.get(&o.key_hash)) else {
                        // The row vanished between scans: the data moved.
                        e.drill_unexplained = true;
                        continue;
                    };
                    let (cols, words) = scan::differing_columns(names, a, b);
                    if cols.is_empty() {
                        e.drill_unexplained = true;
                        continue;
                    }
                    let cols: Vec<String> = cols.iter().map(|i| names[*i].clone()).collect();
                    all_cols.extend(cols.iter().cloned());
                    for w in words {
                        if !detail.plain_words.contains(&w) {
                            detail.plain_words.push(w);
                        }
                    }
                    detail.changed_rows.push(ChangedRowExport {
                        key: config.export_keys.then(|| o.key.clone()),
                        columns: cols,
                    });
                }
                detail.changed_columns = all_cols.into_iter().collect();
            }
            (Err(why), _) | (_, Err(why)) => e.floors.push((
                crate::results::Verdict::ToolFault,
                format!("the changed-row detail scan failed: {why}"),
            )),
        }
    }
    for w in &detail.plain_words {
        e.notes.push(w.clone());
    }
    let entry = results.details.entry(table.to_string()).or_default();
    entry.changed_columns = detail.changed_columns;
    entry.changed_rows = detail.changed_rows;
    entry.missing_keys = detail.missing_keys;
    entry.extra_keys = detail.extra_keys;
    entry.plain_words = detail.plain_words;
    Ok(())
}

/// The independent spot check: 200 rows by key hash from each side, compared
/// by normalised value in the app, judged against the fingerprint.  A spot
/// query that fails is a disagreement (the check could not vouch for the hash
/// path), never a silent skip.
async fn spot_check(
    config: &RunConfig,
    old_s: &mut ReadOnlySession,
    new_s: &mut ReadOnlySession,
    plan: &TablePlan,
    old_src: &Source,
    new_src: &Source,
    comparison: &Comparison,
) -> Result<Spot, RunError> {
    let failed = |why: String| Spot::Disagrees(format!("the spot check could not run: {why}"));
    checkpoint(config)?;
    let so = spot_sample(old_s, plan, Side::Old, old_src).await;
    checkpoint(config)?;
    let sn = spot_sample(new_s, plan, Side::New, new_src).await;
    checkpoint(config)?;
    let (so, sn) = match (so, sn) {
        (Ok(a), Ok(b)) => (a, b),
        (Err(e), _) | (_, Err(e)) => return Ok(failed(e)),
    };
    let khs: Vec<String> = so
        .iter()
        .chain(sn.iter())
        .map(|r| r.kh.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let o = spot_fetch(old_s, plan, Side::Old, old_src, &khs).await;
    checkpoint(config)?;
    let n = spot_fetch(new_s, plan, Side::New, new_src, &khs).await;
    checkpoint(config)?;
    Ok(match (o, n) {
        (Ok(o), Ok(n)) => {
            let non_equal: BTreeSet<u32> = comparison.non_equal().into_iter().collect();
            spot::judge(plan.bucket_digits(), &non_equal, &o, &n)
        }
        (Err(e), _) | (_, Err(e)) => failed(e),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn live(password: &str) -> Input {
        Input::Live(ConnProfile::new(
            "db1.internal",
            5432,
            "bank",
            "dmux_reader",
            password,
            crate::db::TlsMode::VerifyFull,
        ))
    }

    #[test]
    fn a_run_of_chosen_tables_keeps_only_their_findings_and_says_what_was_outside() {
        use crate::results::StructureFinding;
        let finding = |object: &str, verdict: crate::results::Verdict| StructureFinding {
            object: object.into(),
            path: vec![],
            verdict,
            message: "differs".into(),
        };
        let mut r = RunResults {
            structure_findings: vec![
                finding("core.accounts.balance", crate::results::Verdict::Fail),
                finding("core.accounts", crate::results::Verdict::Warning),
                finding("core.accountsx.id", crate::results::Verdict::Fail),
                finding("core.loans.idx", crate::results::Verdict::Fail),
                finding(
                    "public.hash_ltree(public.ltree)",
                    crate::results::Verdict::Warning,
                ),
            ],
            ..RunResults::default()
        };
        let pair = Pair {
            path: "core.accounts".into(),
            old: ("core".into(), "accounts".into()),
            new: ("core".into(), "accounts".into()),
            level: CheckLevel::FullValues,
            keyed_hint: true,
            old_size: 0,
            new_size: 0,
            old_rows: 0,
            new_rows: 0,
        };
        let left_out = vec![finding(
            "core.archive.legacy_flag",
            crate::results::Verdict::Fail,
        )];
        scope_to_chosen(&mut r, &[pair], &left_out, 4, 1);
        let kept: Vec<&str> = r
            .structure_findings
            .iter()
            .map(|f| f.object.as_str())
            .collect();
        // Only the chosen table and its columns; "accountsx" is another table.
        assert_eq!(kept, vec!["core.accounts.balance", "core.accounts"]);
        assert_eq!(r.warnings.len(), 1);
        let w = &r.warnings[0];
        // Findings on a left-out table count, and the failing ones are named,
        // in brackets so the list cannot run into the next part.
        assert!(w.contains("4 structure differences in other tables"), "{w}");
        assert!(
            w.contains(
                "(3 of them would fail a full check: [core.accountsx.id, core.archive.legacy_flag, core.loans.idx])"
            ),
            "{w}"
        );
        assert!(
            w.contains("]); 4 tables only in the new database; 1 old table"),
            "{w}"
        );
        assert!(w.contains("4 tables only in the new database"), "{w}");
        assert!(w.contains("1 old table with no partner"), "{w}");
    }

    #[test]
    fn the_scope_is_part_of_the_definition_and_a_full_check_keeps_its_hash() {
        let mut c = RunConfig::new(live("x"), live("x"));
        let full = definition_hash(&c);
        c.only_chosen_tables = true;
        assert_ne!(definition_hash(&c), full);
        assert!(is_outside_run(NOT_SELECTED_REASON));
        assert!(is_outside_run(NOT_MAPPED_OUTSIDE_RUN));
        assert!(!is_outside_run("archived before the migration"));
    }

    #[test]
    fn the_record_names_both_sides_and_the_times_but_never_a_password() {
        // Built at runtime: no credential literal reaches a password parameter.
        let secret: String = ["pw", "-", "sentinel", "-", "77"].concat();
        let outcome = RunOutcome {
            results: crate::results::finalise(RunResults::default()),
            exit_code: 0,
        };
        let started = jiff::Timestamp::from_second(1_795_000_000).unwrap();
        let finished = jiff::Timestamp::from_second(1_795_000_125).unwrap();
        let out = stamp_record(outcome, started, finished, &live(&secret), &live(&secret));
        let r = &out.results;
        assert_eq!(r.started_at.as_deref(), Some("2026-11-18T11:06:40Z"));
        assert_eq!(r.finished_at.as_deref(), Some("2026-11-18T11:08:45Z"));
        assert_eq!(
            r.old_source.as_deref(),
            Some("bank on db1.internal:5432 (login dmux_reader)")
        );
        let json = serde_json::to_string(r).unwrap();
        assert!(!json.contains(&secret), "password leaked into results");
        // The SHA-256 is re-stamped over the new fields.
        assert_eq!(r.results_sha256, crate::results::sha256_of(r));
    }

    #[test]
    fn a_results_file_without_the_record_still_verifies() {
        let r = crate::results::finalise(RunResults::default());
        let json = serde_json::to_string(&r).unwrap();
        assert!(
            !json.contains("started_at"),
            "absent fields are not written"
        );
        let back: RunResults = serde_json::from_str(&json).unwrap();
        assert_eq!(back.started_at, None);
        assert_eq!(back.results_sha256, crate::results::sha256_of(&back));
    }

    fn mapping(
        old: crate::core::normalise::PgType,
        new: crate::core::normalise::PgType,
    ) -> crate::core::normalise::ColumnMapping {
        crate::core::normalise::ColumnMapping {
            old_name: "c".into(),
            new_name: "c".into(),
            old_type: old,
            new_type: new,
            options: Default::default(),
            control_total: false,
        }
    }

    #[test]
    fn column_rules_are_plain_words_per_type() {
        use crate::core::normalise::PgType as T;
        let rule = |o: T, n: T| column_rule_text(&mapping(o, n));
        assert_eq!(rule(T::Text, T::Text), "compared exactly as text");
        assert_eq!(rule(T::Int4, T::Int8), "compared as an exact whole number");
        assert_eq!(
            rule(T::Float8, T::Float8),
            "compared by its exact 8-byte binary form"
        );
        assert_eq!(
            rule(T::Float4, T::Float4),
            "compared by its exact 4-byte binary form"
        );
        assert!(rule(T::Float4, T::Float8).contains("widened"));
        assert_eq!(
            rule(T::Bytea, T::Bytea),
            "compared by a fingerprint of its raw bytes"
        );
        assert!(
            rule(T::Numeric(Some((18, 2))), T::Numeric(Some((18, 2)))).contains("exact decimal")
        );
        assert!(rule(T::Numeric(Some((18, 2))), T::Numeric(Some((19, 4))))
            .contains("larger number of decimals"));
        assert!(rule(T::Timestamp, T::Timestamptz).contains("date or time"));
        assert!(rule(T::Jsonb, T::Jsonb).contains("normalised JSON"));
        assert!(
            rule(T::Unknown("ltree".into()), T::Unknown("ltree".into())).contains("needs review")
        );
    }

    #[test]
    fn ticked_options_appear_in_the_rule_with_their_reason() {
        use crate::core::normalise::PgType as T;
        let mut m = mapping(T::Numeric(Some((12, 4))), T::Numeric(Some((12, 4))));
        m.options.round_decimals = Some(2);
        m.options.ignore_trailing_spaces = true;
        m.options.null_equals_empty = true;
        m.options.date_only = true;
        let r = column_rule_text(&m);
        assert!(
            r.contains("rounded to 2 decimals because you ticked that option"),
            "{r}"
        );
        assert!(r.contains("trailing spaces ignored"), "{r}");
        assert!(r.contains("empty value counts as NULL"), "{r}");
        assert!(r.contains("only the date part"), "{r}");
    }

    #[test]
    fn change_detection_words_are_stable() {
        use crate::consistency::SideChangeDetection as S;
        assert_eq!(change_detection_word(S::Ok), "available");
        assert_eq!(change_detection_word(S::TrackCountsOff), "track_counts_off");
        assert_eq!(change_detection_word(S::ForeignTable), "foreign_table");
    }

    #[test]
    fn control_facts_keep_both_sides_even_when_equal() {
        use crate::core::fingerprint::Decimal;
        let f = control_facts(
            &["amount".into()],
            &[Decimal::parse("1.50").unwrap()],
            &[Decimal::parse("1.5").unwrap()],
        );
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].column, "amount");
        assert_eq!(f[0].old, "1.50");
    }

    /// Test-only connections.  Nothing here ships: the password is built at run
    /// time (from `DMUX_TEST_OLD_URL` when set), never written in the source.
    fn test_password() -> String {
        std::env::var("DMUX_TEST_OLD_URL")
            .ok()
            .and_then(|u| {
                let creds = u
                    .strip_prefix("postgres://")?
                    .split_once('@')?
                    .0
                    .to_string();
                Some(creds.split_once(':')?.1.to_string())
            })
            .unwrap_or_else(|| format!("unused-{}", std::process::id()))
    }

    /// A live-to-live configuration against local mock ports, for unit tests that
    /// never need a server (or that must fail to reach one).
    fn mock_config() -> RunConfig {
        let local = |port| {
            Input::Live(ConnProfile::new(
                "127.0.0.1",
                port,
                "bank",
                "dmux_dev",
                test_password(),
                crate::db::TlsMode::Disable("unit test: no server".to_string()),
            ))
        };
        RunConfig::new(local(5433), local(5434))
    }
    use crate::mapping::{Rule, RuleKind, SchemaMapping, TableMapping};

    #[test]
    fn a_schema_rename_rule_never_renames_a_table() {
        let mut m = Mapping::new();
        m.rules.push(Rule {
            kind: RuleKind::SchemaRename,
            pattern: "accounts".into(),
            target: "elsewhere".into(),
        });
        // Before the fix the table-rule loop applied this schema rule to the
        // table name and returned ("s", "elsewhere").
        assert_eq!(
            map_table(&m, "s", "accounts"),
            ("s".into(), "accounts".into())
        );
        assert_eq!(
            map_table(&m, "accounts", "t"),
            ("elsewhere".into(), "t".into())
        );
    }

    #[test]
    fn explicit_table_mappings_and_prefix_rules() {
        let mut m = Mapping::new();
        let mut sm = SchemaMapping::default();
        sm.table_mappings.insert(
            "limits".into(),
            TableMapping {
                target_table: "product_limits".into(),
                ..TableMapping::default()
            },
        );
        m.schema_mappings.insert("s".into(), sm);
        m.rules.push(Rule {
            kind: RuleKind::PrefixReplace,
            pattern: "old_".into(),
            target: "new_".into(),
        });
        // An empty target schema keeps the schema (it used to return "").
        assert_eq!(
            map_table(&m, "s", "limits"),
            ("s".into(), "product_limits".into())
        );
        assert_eq!(map_table(&m, "s", "old_x"), ("s".into(), "new_x".into()));
        assert_eq!(map_table(&m, "s", "y"), ("s".into(), "y".into()));
    }

    fn cc(ins: i64, storage: &str) -> ChangeCounters {
        ChangeCounters {
            n_tup_ins: ins,
            storage: storage.into(),
            ..ChangeCounters::default()
        }
    }

    #[test]
    fn the_sweep_sees_a_truncate_and_unreadable_counters() {
        let before = Some((cc(5, "1:1"), cc(5, "2:2")));
        assert!(!counters_moved(&before, Some(&before.clone())));
        // Same counts, new relfilenode on the new side: a truncate and reload.
        assert!(counters_moved(
            &before,
            Some(&Some((cc(5, "1:1"), cc(5, "2:9"))))
        ));
        assert!(counters_moved(&before, Some(&None)));
        assert!(counters_moved(&None, Some(&before)));
        assert!(counters_moved(&before, None));
    }

    #[test]
    fn an_epoch_change_during_the_run_is_seen() {
        let e = Some(ServerEpoch {
            postmaster_start: Some("t0".into()),
            stats_reset: None,
        });
        assert!(!epoch_changed(&e, &e.clone()));
        let reset = Some(ServerEpoch {
            postmaster_start: Some("t0".into()),
            stats_reset: Some("t1".into()),
        });
        assert!(epoch_changed(&e, &reset));
        assert!(epoch_changed(&e, &None));
    }

    #[test]
    fn replica_outcomes_mark_or_record() {
        let st = |lsn: &str, paused| {
            Some(ReplicaState {
                in_recovery: true,
                replay_lsn: Some(lsn.into()),
                paused,
            })
        };
        let mut e = Evidence::new("s.t", CheckLevel::FullValues, Outcome::OnlyOnNew);
        apply_replica(
            &mut e,
            "old",
            &replica_outcome(&st("0/1", false), &st("0/1", false)),
        );
        assert!(!e.changed);
        apply_replica(
            &mut e,
            "old",
            &replica_outcome(&st("0/1", true), &st("0/1", true)),
        );
        assert!(!e.changed);
        assert!(e.notes.iter().any(|n| n.contains("replay paused")));
        apply_replica(
            &mut e,
            "new",
            &replica_outcome(&st("0/1", false), &st("0/2", false)),
        );
        assert!(e.changed, "replay advanced during the scans");
        let mut e = Evidence::new("s.t", CheckLevel::FullValues, Outcome::OnlyOnNew);
        apply_replica(&mut e, "new", &replica_outcome(&st("0/1", false), &None));
        assert!(e.changed, "an unreadable state is moved");
        let mut r = RunResults::default();
        note_paused(&mut r, "old", &ReplicaDecision::Paused);
        note_paused(&mut r, "old", &ReplicaDecision::Paused);
        assert_eq!(r.warnings.len(), 1);
    }

    #[test]
    fn the_definition_hash_ignores_passwords_but_not_scope() {
        let base = mock_config();
        let mut pw = base.clone();
        if let Input::Live(p) = &mut pw.old {
            p.password = format!("{}-changed", test_password());
        }
        assert_eq!(definition_hash(&base), definition_hash(&pw));
        let mut lvl = base.clone();
        lvl.default_level = CheckLevel::CountsAndTotals;
        assert_ne!(definition_hash(&base), definition_hash(&lvl));
        let mut db = base.clone();
        if let Input::Live(p) = &mut db.new {
            p.dbname = "demo_bank".into();
        }
        assert_ne!(definition_hash(&base), definition_hash(&db));
    }

    #[test]
    fn the_definition_hash_covers_the_ca_file() {
        let base = mock_config();
        let with_ca = |side_old: bool, path: &str| {
            let mut c = base.clone();
            let side = if side_old { &mut c.old } else { &mut c.new };
            if let Input::Live(p) = side {
                *p = p.clone().with_ca_file(path);
            }
            c
        };
        let a = with_ca(true, "/etc/bank/ca-a.pem");
        let b = with_ca(true, "/etc/bank/ca-b.pem");
        let n = with_ca(false, "/etc/bank/ca-a.pem");
        // Adding, changing or moving the CA file changes the hash.
        assert_ne!(definition_hash(&base), definition_hash(&a));
        assert_ne!(definition_hash(&a), definition_hash(&b));
        assert_ne!(definition_hash(&a), definition_hash(&n));
        // The same CA file gives the same hash.
        assert_eq!(
            definition_hash(&a),
            definition_hash(&with_ca(true, "/etc/bank/ca-a.pem"))
        );
    }

    #[tokio::test]
    async fn parallel_limits_below_one_are_rejected() {
        let c = RunConfig {
            old_parallelism: 0,
            ..mock_config()
        };
        assert!(matches!(run(&c).await, Err(RunError::Config(_))));
    }

    #[test]
    fn a_new_config_names_its_connections_and_runs_to_completion() {
        let c = mock_config();
        assert!(c.control.is_none());
        assert_eq!((c.old_parallelism, c.new_parallelism), (2, 4));
        assert!(!c.allow_superuser && !c.export_keys);
        assert_eq!(c.stats_settle, DEFAULT_STATS_SETTLE);
        // The control is not part of what a run is.
        let mut with = c.clone();
        with.control = Some(std::sync::Arc::new(RunControl::new()));
        assert_eq!(definition_hash(&c), definition_hash(&with));
    }

    #[test]
    fn connection_info_records_tls_superuser_and_write_access() {
        let profile = ConnProfile::new(
            "h",
            5432,
            "db",
            "u",
            format!("{:x}", std::process::id() ^ 0x5a5a),
            crate::db::TlsMode::Disable("test network".to_string()),
        );
        let info = ServerInfo {
            version_num: 170000,
            version_string: "17.0".into(),
            encoding: "UTF8".into(),
            collation: "C".into(),
            ctype: "C".into(),
            lc_monetary: "C".into(),
            is_superuser: true,
            can_write: true,
            pooler_suspected: true,
            md5_available: false,
            sql_ascii: false,
            rolbypassrls: false,
            rls_tables: vec!["s.accounts".into()],
            can_write_reasons: vec!["has INSERT on at least one table".into()],
        };
        let c = connection_info(&profile, &info, Some("migration needs it"));
        assert_eq!(c.tls, "plaintext");
        assert_eq!(c.plaintext_reason.as_deref(), Some("test network"));
        assert!(c.superuser);
        assert_eq!(c.superuser_reason.as_deref(), Some("migration needs it"));
        assert!(c.can_write);
        assert_eq!(
            c.can_write_reasons,
            vec!["has INSERT on at least one table".to_string()]
        );
        assert!(!c.rolbypassrls);
        assert_eq!(c.rls_tables, vec!["s.accounts".to_string()]);
        assert!(c.pooler_suspected);
        assert!(!c.md5_available);
        assert_eq!(c.server_version, "17.0");
        assert_eq!(c.server_encoding, "UTF8");

        let mut r = RunResults::default();
        connection_warnings(&mut r, &c, &ConnectionInfo::default());
        assert!(r.warnings.iter().any(|w| w.contains("can write")));
        assert!(r
            .warnings
            .iter()
            .any(|w| w.contains("has INSERT on at least one table")));
        assert!(r.warnings.iter().any(|w| w.contains("superuser")));
        assert!(r.warnings.iter().any(|w| w.contains("migration needs it")));
        assert!(r
            .warnings
            .iter()
            .any(|w| w.contains("tables use row-level security")));
    }

    fn stopped_config() -> (RunConfig, std::sync::Arc<RunControl>) {
        let ctl = std::sync::Arc::new(RunControl::new());
        let c = RunConfig {
            control: Some(ctl.clone()),
            ..mock_config()
        };
        (c, ctl)
    }

    #[test]
    fn checkpoints_follow_the_stop_flag() {
        let (c, ctl) = stopped_config();
        assert!(checkpoint(&c).is_ok());
        assert!(
            checkpoint(&mock_config()).is_ok(),
            "no control: never stops"
        );
        assert!(ctl.stop().is_empty(), "nothing registered yet");
        assert!(ctl.is_stopped());
        assert!(matches!(checkpoint(&c), Err(RunError::Cancelled)));
        // Stopping twice is harmless.
        ctl.stop();
        assert!(matches!(checkpoint(&c), Err(RunError::Cancelled)));
    }

    fn windowed() -> (RunConfig, std::sync::Arc<RunControl>) {
        let (mut c, ctl) = stopped_config();
        c.run_window = crate::run_window::RunWindow::parse("22:00-05:00").ok();
        (c, ctl)
    }

    const TICK: std::time::Duration = std::time::Duration::from_millis(1);

    #[test]
    fn the_run_window_is_not_part_of_the_definition_hash() {
        let (c, _) = windowed();
        assert_eq!(definition_hash(&c), definition_hash(&mock_config()));
    }

    #[tokio::test]
    async fn no_window_or_an_open_window_never_waits_or_warns() {
        let mut r = RunResults::default();
        let c = mock_config();
        wait_for_window(&c, &mut r, || 12 * 60, TICK).await.unwrap();
        let (c, ctl) = windowed();
        wait_for_window(&c, &mut r, || 23 * 60, TICK).await.unwrap();
        wait_for_window(&c, &mut r, || 22 * 60, TICK).await.unwrap();
        wait_for_window(&c, &mut r, || 4 * 60 + 59, TICK)
            .await
            .unwrap();
        assert!(r.warnings.is_empty());
        assert_eq!(
            ctl.with_live(|l| l.window_opens_at().map(str::to_string)),
            None
        );
    }

    #[tokio::test]
    async fn outside_the_window_the_run_waits_then_carries_on_when_it_opens() {
        use std::sync::atomic::{AtomicU32, Ordering};
        let (c, ctl) = windowed();
        let mut r = RunResults::default();
        let calls = AtomicU32::new(0);
        // Closed (12:00) for the first reads, then 22:00 (open).
        let clock = || {
            if calls.fetch_add(1, Ordering::SeqCst) < 4 {
                12 * 60
            } else {
                22 * 60
            }
        };
        // Observe the wait state from another task while waiting.
        let watcher = {
            let ctl = ctl.clone();
            tokio::spawn(async move {
                for _ in 0..200 {
                    if let Some(t) = ctl.with_live(|l| l.window_opens_at().map(str::to_string)) {
                        return Some(t);
                    }
                    tokio::time::sleep(TICK).await;
                }
                None
            })
        };
        wait_for_window(&c, &mut r, clock, std::time::Duration::from_millis(20))
            .await
            .unwrap();
        assert!(calls.load(Ordering::SeqCst) >= 5);
        assert_eq!(watcher.await.unwrap().as_deref(), Some("22:00"));
        assert_eq!(
            ctl.with_live(|l| l.window_opens_at().map(str::to_string)),
            None
        );
        assert_eq!(r.warnings.len(), 1);
        assert!(r.warnings[0].contains("22:00-05:00"));
        // A second wait in the same run does not repeat the warning.
        let calls2 = AtomicU32::new(0);
        wait_for_window(
            &c,
            &mut r,
            || {
                if calls2.fetch_add(1, Ordering::SeqCst) < 2 {
                    600
                } else {
                    1380
                }
            },
            TICK,
        )
        .await
        .unwrap();
        assert_eq!(r.warnings.len(), 1);
    }

    #[tokio::test]
    async fn a_stop_ends_the_wait_for_the_window() {
        let (c, ctl) = windowed();
        let mut r = RunResults::default();
        let stopper = {
            let ctl = ctl.clone();
            tokio::spawn(async move {
                tokio::time::sleep(std::time::Duration::from_millis(30)).await;
                ctl.stop();
            })
        };
        let out = wait_for_window(&c, &mut r, || 12 * 60, TICK).await;
        stopper.await.unwrap();
        assert!(matches!(out, Err(RunError::Cancelled)));
        assert_eq!(
            ctl.with_live(|l| l.window_opens_at().map(str::to_string)),
            None
        );
    }

    #[tokio::test]
    async fn a_stopped_run_returns_cancelled_and_never_connects() {
        // The mock profile points at a port nothing listens on: a run that
        // tried to connect would fail with a database error instead.
        let (mut c, ctl) = stopped_config();
        let closed = |i: &mut Input| {
            if let Input::Live(p) = i {
                p.port = 1;
            }
        };
        closed(&mut c.old);
        closed(&mut c.new);
        ctl.stop();
        assert!(matches!(run(&c).await, Err(RunError::Cancelled)));
        // The file-against-live path honours it the same way.
        let file = crate::fingerprint_file::FingerprintFile {
            format_version: crate::fingerprint_file::FINGERPRINT_VERSION,
            tool_version: String::new(),
            normalise_rules_version: Some(crate::core::normalise::NORMALISE_RULES_VERSION),
            mapping_hash: c.mapping.mapping_hash.clone(),
            server: crate::fingerprint_file::CapturedServer {
                version_num: 170000,
                version_string: "17".into(),
                encoding: "UTF8".into(),
                collation: "C".into(),
                sql_ascii: false,
            },
            tables: vec![],
            inventory: None,
            sha256: String::new(),
        };
        let fl = RunConfig {
            old: Input::File(file),
            ..c.clone()
        };
        assert!(matches!(run(&fl).await, Err(RunError::Cancelled)));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn the_settle_wait_wakes_on_a_stop() {
        let (c, ctl) = stopped_config();
        let started = Instant::now();
        let (r, _) = tokio::join!(settle(&c, std::time::Duration::from_secs(30)), async {
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            ctl.stop();
        });
        assert!(matches!(r, Err(RunError::Cancelled)));
        assert!(started.elapsed() < std::time::Duration::from_secs(2));
        // Without a stop it waits the full time.
        let started = Instant::now();
        settle(&mock_config(), std::time::Duration::from_millis(120))
            .await
            .unwrap();
        assert!(started.elapsed() >= std::time::Duration::from_millis(120));
    }

    // --- Huge-table ranges and app-side hashing on a real table (T067) ---

    #[test]
    fn split_thresholds() {
        assert_eq!(split_parts_for(0), 0);
        assert_eq!(split_parts_for(SPLIT_MIN_ROWS - 1), 0);
        assert_eq!(split_parts_for(SPLIT_MIN_ROWS), 2);
        assert_eq!(split_parts_for(5_500_000), 5);
        assert_eq!(split_parts_for(i64::MAX), SPLIT_MAX_PARTS as u32);
        assert_eq!(HashMode::of(true), HashMode::Server);
        assert_eq!(HashMode::of(false), HashMode::App);
        assert!(app_hash_warning("old", HashMode::Server).is_none());
        let w = app_hash_warning("new", HashMode::App).unwrap();
        assert!(w.starts_with("the new database cannot compute md5"), "{w}");
    }

    fn spec_with(key_types: &[(&str, &str, &str)]) -> crate::plan::TableSpec {
        use crate::core::normalise::{ColumnMapping, PgType};
        crate::plan::TableSpec {
            columns: key_types
                .iter()
                .map(|(n, o, nw)| ColumnMapping {
                    old_name: (*n).into(),
                    new_name: format!("{n}_new"),
                    old_type: PgType::parse(o),
                    new_type: PgType::parse(nw),
                    options: ColumnOptions::default(),
                    control_total: false,
                })
                .collect(),
            key: vec![0],
            old_qualified: "s.t".into(),
            old_schema: "s".into(),
            old_name: "t".into(),
            new_qualified: "s.t".into(),
            new_schema: "s".into(),
            new_name: "t".into(),
            level: CheckLevel::FullValues,
            bucket_digits: 3,
            ignored_columns: BTreeMap::new(),
            dropped: Vec::new(),
            added: Vec::new(),
            coverage_complete: true,
            uncovered: Vec::new(),
        }
    }

    #[test]
    fn the_split_column_is_a_number_or_date_key_of_one_type() {
        assert_eq!(
            split_column(&spec_with(&[("id", "bigint", "bigint")])),
            Some(("id".into(), "id_new".into(), RangeType::Int8))
        );
        assert_eq!(
            split_column(&spec_with(&[("d", "date", "date")])).map(|c| c.2),
            Some(RangeType::Date)
        );
        // Never text, and never a key whose type differs between the sides.
        assert_eq!(split_column(&spec_with(&[("k", "text", "text")])), None);
        assert_eq!(
            split_column(&spec_with(&[("k", "integer", "bigint")])),
            None
        );
        let mut keyless = spec_with(&[("id", "bigint", "bigint")]);
        keyless.key.clear();
        assert_eq!(split_column(&keyless), None);
    }

    fn demo_profile() -> Option<ConnProfile> {
        let url = match std::env::var("DMUX_TEST_DEMO_OLD_URL") {
            Ok(u) => u,
            Err(_) => {
                assert!(
                    std::env::var("CI").as_deref() != Ok("true"),
                    "CI=true but DMUX_TEST_DEMO_OLD_URL is not set"
                );
                return None;
            }
        };
        let rest = url.strip_prefix("postgres://")?;
        let (creds, hostdb) = rest.split_once('@')?;
        let (user, pass) = creds.split_once(':')?;
        let (hostport, db) = hostdb.split_once('/')?;
        let (host, port) = hostport.split_once(':')?;
        Some(ConnProfile::new(
            host,
            port.parse().ok()?,
            db,
            user,
            pass.to_string(),
            crate::db::TlsMode::Disable("local test database".to_string()),
        ))
    }

    /// The range path on a real table: split into parts, scanned on several
    /// sessions at once, summed, equals the whole-table scan, in both hashing
    /// modes; and the app-side whole scan equals the server's.
    #[tokio::test]
    async fn a_split_table_sums_to_its_whole_scan_on_a_real_table() {
        let Some(profile) = demo_profile() else {
            return;
        };
        let config = RunConfig {
            allow_superuser: true,
            ..RunConfig::new(Input::Live(profile.clone()), Input::Live(profile.clone()))
        };
        let mut s = ReadOnlySession::open(&profile, true).await.unwrap();
        let live = live_table(&mut s, "demo_bank", "transactions")
            .await
            .unwrap()
            .unwrap();
        let spec = build_table_spec(
            &live,
            &live,
            SpecInputs {
                new_schema: "demo_bank",
                new_name: "transactions",
                level: CheckLevel::FullValues,
                mapping: &Mapping::new(),
                control_total_columns: &BTreeSet::new(),
                column_options: &BTreeMap::new(),
                estimated_rows: 20_000,
            },
        );
        let plan = crate::plan::core_table_plan(&spec).unwrap();
        let src = Source::Table {
            schema: "demo_bank".into(),
            name: "transactions".into(),
        };
        let whole = scan_fingerprint(&mut s, &plan, Side::Old, &src, "m", HashMode::Server, None)
            .await
            .unwrap();
        assert!(whole.rows > 1000);
        let app_whole = scan_fingerprint(&mut s, &plan, Side::Old, &src, "m", HashMode::App, None)
            .await
            .unwrap();
        assert_eq!(
            app_whole, whole,
            "app-side hashing must equal the server on a real table"
        );
        let (col, _, ty) = split_column(&spec).expect("transactions splits on its key");
        let cuts = read_cuts(&mut s, &src, &col, ty, 7).await.unwrap();
        let parts = range_parts(&cuts).unwrap();
        for (mode, workers) in [
            (HashMode::Server, 3),
            (HashMode::App, 2),
            (HashMode::Server, 1),
        ] {
            let filters: Vec<RangeFilter> = parts
                .iter()
                .map(|p| RangeFilter {
                    column: col.clone(),
                    ty,
                    part: p.clone(),
                })
                .collect();
            let merged = scan_side_in_ranges(
                &config,
                &profile,
                &mut s,
                &plan,
                Side::Old,
                &src,
                "m",
                mode,
                filters,
                workers,
            )
            .await
            .unwrap();
            assert_eq!(merged, whole, "{mode:?} with {workers} sessions");
        }
    }
}
