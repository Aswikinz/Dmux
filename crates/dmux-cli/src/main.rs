//! `dmux` command-line runner.
//!
//! `probe`, `inventory`, `diff`, `capture`, `compare` and `run` share the engine.
//! Exit codes follow PLAN.md 4.8: 0 pass, 1 fail, 2 warnings.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Args, Parser, Subcommand};
use dmux_engine::db::{probe_server, ConnProfile, DbError, ReadOnlySession, TlsMode};
use dmux_engine::fingerprint_file::FingerprintFile;
use dmux_engine::inventory::collect_inventory;
use dmux_engine::mapping::{from_json as mapping_from_json, Mapping};
use dmux_engine::results::{CheckLevel, RunResults, Verdict};
use dmux_engine::run::{capture, compare_files, run, Input, RunConfig};
use dmux_engine::schema_diff::{compare_inventories, DiffStatus};
use dmux_hooks::{Finding, HookSpec, Severity};

/// PLAN.md 4.8 exit codes.
const EXIT_PASS: u8 = 0;
const EXIT_FAIL: u8 = 1;
const EXIT_WARN: u8 = 2;

#[derive(Parser, Debug)]
#[command(name = "dmux")]
#[command(about = "Proves a PostgreSQL migration copied every row and value")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Test a database connection and report server information.
    Probe(ProbeArgs),
    /// Collect a structural inventory and print it as JSON.
    Inventory(InventoryArgs),
    /// Compare two inventory JSON files and print the differences as JSON.
    Diff(DiffArgs),
    /// Save one database's fingerprints to a file.
    Capture(CaptureArgs),
    /// Compare two fingerprint files (or a file and a live database).
    Compare(CompareArgs),
    /// Run a saved verification definition (TOML or JSON).
    Run(RunArgs),
    /// Run Python hooks over a results JSON file and print their findings.
    Hook(HookArgs),
}

/// Shared connection arguments for the commands that talk to a database.
#[derive(Args, Debug, Clone)]
struct ConnArgs {
    #[arg(long)]
    host: String,
    #[arg(long)]
    port: u16,
    #[arg(long)]
    db: String,
    #[arg(long)]
    user: String,
    #[arg(long, value_name = "VAR")]
    password_env: Option<String>,
    #[arg(long, value_name = "MODE", default_value = "verify")]
    tls: String,
    #[arg(long, value_name = "TEXT")]
    reason: Option<String>,
    #[arg(long)]
    allow_superuser: bool,
}

impl ConnArgs {
    fn profile(&self) -> Result<ConnProfile, String> {
        let tls_mode = parse_tls_mode(&self.tls, &self.reason)?;
        let password = match self.password_env.as_deref().map(std::env::var) {
            Some(Ok(p)) => p,
            Some(Err(_)) => {
                return Err(format!(
                    "password environment variable {} is not set",
                    self.password_env.as_deref().unwrap_or_default()
                ))
            }
            None => String::new(),
        };
        if self.allow_superuser && !allow_superuser_requires_reason(&self.reason) {
            return Err("--allow-superuser requires --reason <text>".to_string());
        }
        Ok(ConnProfile::new(
            self.host.clone(),
            self.port,
            self.db.clone(),
            self.user.clone(),
            password,
            tls_mode,
        ))
    }
}

/// Full connection arguments for the single-database commands.
#[derive(Args, Debug)]
struct ProbeArgs {
    #[command(flatten)]
    conn: ConnArgs,
}

#[derive(Args, Debug)]
struct InventoryArgs {
    #[command(flatten)]
    conn: ConnArgs,
}

#[derive(Args, Debug)]
struct DiffArgs {
    #[arg(long)]
    old: PathBuf,
    #[arg(long)]
    new: PathBuf,
}

#[derive(Args, Debug)]
struct CaptureArgs {
    #[command(flatten)]
    conn: ConnArgs,
    /// Check level for every table (structure, counts, or full-values).
    #[arg(long, default_value = "full-values")]
    level: String,
    /// A control-total column to sum (repeatable).
    #[arg(long = "control-total", value_name = "COL")]
    control_total: Vec<String>,
    /// A mapping JSON file (identity by default).
    #[arg(long, value_name = "FILE")]
    mapping: Option<PathBuf>,
    /// Output file (`.gz` compresses).
    #[arg(long, value_name = "FILE")]
    out: PathBuf,
}

#[derive(Args, Debug)]
struct CompareArgs {
    #[arg(long, value_name = "FILE")]
    old: PathBuf,
    #[arg(long, value_name = "FILE")]
    new: PathBuf,
    /// Print the results JSON to this path (stdout by default).
    #[arg(long, value_name = "FILE")]
    out: Option<PathBuf>,
}

#[derive(Args, Debug)]
struct RunArgs {
    /// The verification definition file (TOML or JSON).
    definition: String,
    /// Resume from a saved run store, skipping finished tables.
    #[arg(long)]
    resume: Option<String>,
    /// Write the results JSON to this path (stdout by default).
    #[arg(long, value_name = "FILE")]
    out: Option<PathBuf>,
    /// Run every Python hook in this directory after the run (off by default).
    #[arg(long, value_name = "DIR")]
    hooks: Option<PathBuf>,
    /// Hook time limit in seconds (default 60).
    #[arg(long, value_name = "SECS", default_value_t = 60)]
    hook_timeout: u64,
    /// Python interpreter to run hooks with (default: auto-discover python3/python).
    #[arg(long, value_name = "PATH")]
    python: Option<PathBuf>,
}

#[derive(Args, Debug)]
struct HookArgs {
    /// The run's results JSON file (`-` reads stdin).
    #[arg(value_name = "RESULTS")]
    results: String,
    /// A directory of Python hook scripts, or a single script.
    #[arg(value_name = "HOOKS")]
    hooks: PathBuf,
    /// Hook time limit in seconds (default 60).
    #[arg(long, value_name = "SECS", default_value_t = 60)]
    timeout: u64,
    /// Python interpreter (default: auto-discover python3/python).
    #[arg(long, value_name = "PATH")]
    python: Option<PathBuf>,
}

/// Parse the `--tls` mode string into a [`TlsMode`].
fn parse_tls_mode(tls: &str, reason: &Option<String>) -> Result<TlsMode, String> {
    match tls {
        "verify" => Ok(TlsMode::VerifyFull),
        "disable" => {
            let reason = reason
                .as_deref()
                .ok_or("--reason <text> is required for --tls disable")?;
            if reason.trim().is_empty() {
                return Err("--reason <text> may not be empty".to_string());
            }
            Ok(TlsMode::Disable(reason.to_string()))
        }
        _ => {
            if let Some(hex) = tls.strip_prefix("fingerprint:") {
                if hex.is_empty() {
                    return Err("--tls fingerprint:<hex> is missing the hex value".to_string());
                }
                Ok(TlsMode::TrustFingerprint(hex.to_string()))
            } else {
                Err(format!(
                    "unknown --tls value: {tls} (expected verify, fingerprint:<hex>, or disable)"
                ))
            }
        }
    }
}

fn allow_superuser_requires_reason(reason: &Option<String>) -> bool {
    reason.as_deref().is_some_and(|r| !r.trim().is_empty())
}

fn parse_level(level: &str) -> Result<CheckLevel, String> {
    match level {
        "structure" => Ok(CheckLevel::Structure),
        "counts" | "counts-and-totals" => Ok(CheckLevel::CountsAndTotals),
        "full-values" | "full_values" | "full" => Ok(CheckLevel::FullValues),
        other => Err(format!("unknown check level: {other}")),
    }
}

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();
    match cli.command {
        Command::Probe(args) => run_probe(args.conn).await,
        Command::Inventory(args) => run_inventory(args.conn).await,
        Command::Diff(args) => run_diff(args),
        Command::Capture(args) => run_capture(args).await,
        Command::Compare(args) => run_compare(args).await,
        Command::Run(args) => run_run(args).await,
        Command::Hook(args) => run_hook(args).await,
    }
}

async fn run_probe(args: ConnArgs) -> ExitCode {
    let profile = match args.profile() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("configuration error: {e}");
            return ExitCode::from(EXIT_FAIL);
        }
    };

    let info = match probe_server(&profile, args.allow_superuser).await {
        Ok(i) => i,
        Err(e) => {
            eprintln!("probe failed: {e}");
            return ExitCode::from(EXIT_FAIL);
        }
    };

    let json = serde_json::to_string(&info).unwrap_or_else(|_| "{}".to_string());
    println!("{json}");

    let mut warnings = Vec::new();
    if info.is_superuser {
        warnings.push("superuser login allowed with recorded reason");
    }
    if info.can_write {
        warnings.push("login can write to user tables");
    }
    if info.pooler_suspected {
        warnings.push("connection pooler suspected");
    }
    if info.sql_ascii {
        warnings.push("SQL_ASCII encoding; review required");
    }
    if !info.md5_available {
        warnings.push("md5 unavailable; fallback hashing will be used");
    }

    for w in &warnings {
        eprintln!("warning: {w}");
    }

    if !warnings.is_empty() {
        ExitCode::from(EXIT_WARN)
    } else {
        ExitCode::from(EXIT_PASS)
    }
}

async fn run_inventory(args: ConnArgs) -> ExitCode {
    let profile = match args.profile() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("configuration error: {e}");
            return ExitCode::from(EXIT_FAIL);
        }
    };

    let mut session = match ReadOnlySession::open(&profile, args.allow_superuser).await {
        Ok(s) => s,
        Err(e) => {
            eprintln!("dmux inventory failed: {e}");
            return ExitCode::from(EXIT_FAIL);
        }
    };

    let inventory = session
        .with_read_only("30s", |txn| {
            Box::pin(async move {
                collect_inventory(txn).await.map_err(|e| DbError::Postgres {
                    message: e.to_string(),
                })
            })
        })
        .await;

    match inventory {
        Ok(inv) => match serde_json::to_string_pretty(&inv) {
            Ok(json) => println!("{json}"),
            Err(e) => {
                eprintln!("dmux inventory failed: {e}");
                return ExitCode::from(EXIT_FAIL);
            }
        },
        Err(e) => {
            eprintln!("dmux inventory failed: {e}");
            return ExitCode::from(EXIT_FAIL);
        }
    }
    ExitCode::from(EXIT_PASS)
}

fn run_diff(args: DiffArgs) -> ExitCode {
    let result = (|| -> Result<DiffStatus, String> {
        let old_text = fs::read_to_string(&args.old).map_err(|e| e.to_string())?;
        let new_text = fs::read_to_string(&args.new).map_err(|e| e.to_string())?;
        let old = serde_json::from_str(&old_text).map_err(|e| e.to_string())?;
        let new = serde_json::from_str(&new_text).map_err(|e| e.to_string())?;
        let diff = compare_inventories(&old, &new);
        let json = serde_json::to_string_pretty(&diff).map_err(|e| e.to_string())?;
        println!("{json}");
        Ok(diff.status)
    })();

    match result {
        Ok(DiffStatus::Equal) => ExitCode::from(EXIT_PASS),
        Ok(DiffStatus::Warning) => ExitCode::from(EXIT_WARN),
        Ok(DiffStatus::Different) => ExitCode::from(EXIT_FAIL),
        Err(e) => {
            eprintln!("dmux diff failed: {e}");
            ExitCode::from(EXIT_FAIL)
        }
    }
}

async fn run_capture(args: CaptureArgs) -> ExitCode {
    let profile = match args.conn.profile() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("configuration error: {e}");
            return ExitCode::from(EXIT_FAIL);
        }
    };
    let level = match parse_level(&args.level) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("configuration error: {e}");
            return ExitCode::from(EXIT_FAIL);
        }
    };
    let mapping = match args.mapping.as_ref() {
        Some(path) => {
            let bytes = match fs::read(path) {
                Ok(b) => b,
                Err(e) => {
                    eprintln!("dmux capture failed: {e}");
                    return ExitCode::from(EXIT_FAIL);
                }
            };
            match mapping_from_json(&bytes) {
                Ok(m) => m,
                Err(e) => {
                    eprintln!("dmux capture failed: invalid mapping: {e}");
                    return ExitCode::from(EXIT_FAIL);
                }
            }
        }
        None => Mapping::new(),
    };
    let control_total: BTreeSet<String> = args.control_total.into_iter().collect();

    let file = match capture::capture(
        &profile,
        level,
        &control_total,
        &mapping,
        args.conn.allow_superuser,
    )
    .await
    {
        Ok(f) => f,
        Err(e) => {
            eprintln!("dmux capture failed: {e}");
            return ExitCode::from(EXIT_FAIL);
        }
    };

    if let Err(e) = write_fingerprint_file(&args.out, &file) {
        eprintln!("dmux capture failed: {e}");
        return ExitCode::from(EXIT_FAIL);
    }
    println!(
        "captured {} tables to {}",
        file.tables.len(),
        args.out.display()
    );
    ExitCode::from(EXIT_PASS)
}

fn write_fingerprint_file(path: &std::path::Path, file: &FingerprintFile) -> Result<(), String> {
    let bytes = file.clone().to_json().map_err(|e| e.to_string())?;
    fs::write(path, bytes).map_err(|e| e.to_string())
}

async fn run_compare(args: CompareArgs) -> ExitCode {
    let old = match FingerprintFile::read_from(&args.old) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("dmux compare failed: old capture: {e}");
            return ExitCode::from(EXIT_FAIL);
        }
    };
    let new = match FingerprintFile::read_from(&args.new) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("dmux compare failed: new capture: {e}");
            return ExitCode::from(EXIT_FAIL);
        }
    };

    let outcome = match compare_files::compare_two_files(&old, &new) {
        Ok(o) => o,
        Err(e) => {
            eprintln!("dmux compare failed: {e}");
            return ExitCode::from(EXIT_FAIL);
        }
    };

    emit_results(args.out.as_deref(), &outcome.results);
    ExitCode::from(outcome.exit_code)
}

fn emit_results(path: Option<&std::path::Path>, results: &dmux_engine::results::RunResults) {
    let json = serde_json::to_string_pretty(results).unwrap_or_else(|_| "{}".to_string());
    match path {
        Some(p) => {
            if let Err(e) = fs::write(p, &json) {
                eprintln!("dmux: failed to write results: {e}");
            }
        }
        None => println!("{json}"),
    }
}

fn emit_json_value(path: Option<&std::path::Path>, value: &serde_json::Value) {
    let json = serde_json::to_string_pretty(value).unwrap_or_else(|_| "{}".to_string());
    match path {
        Some(p) => {
            if let Err(e) = fs::write(p, &json) {
                eprintln!("dmux: failed to write results: {e}");
            }
        }
        None => println!("{json}"),
    }
}

/// Run the `dmux hook` command: read a results file (or stdin), run the given
/// hook script(s), and print their findings as JSON marked `custom_checks`.
async fn run_hook(args: HookArgs) -> ExitCode {
    let results_value = match read_results_json(&args.results) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("dmux hook failed: {e}");
            return ExitCode::from(EXIT_FAIL);
        }
    };

    let mut specs = match discover_hook_specs(&args.hooks, args.timeout, args.python.as_deref()) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("dmux hook failed: {e}");
            return ExitCode::from(EXIT_FAIL);
        }
    };
    for spec in &mut specs {
        if let Some(p) = args.python.as_deref() {
            spec.interpreter = Some(p.to_path_buf());
        }
    }

    let hook_results = match dmux_hooks::run_hooks(&specs, &results_value) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("dmux hook failed: {e}");
            return ExitCode::from(EXIT_FAIL);
        }
    };

    let envelope = envelope_results(&results_value, &specs, &hook_results);
    emit_json_value(None, &envelope);

    if hook_causes_non_pass(&hook_results) {
        ExitCode::from(EXIT_FAIL)
    } else if hook_results.iter().any(|r| !r.warnings.is_empty()) {
        ExitCode::from(EXIT_WARN)
    } else {
        ExitCode::from(EXIT_PASS)
    }
}

/// Read a results JSON value from a path, or stdin for `-`.
fn read_results_json(path: &str) -> Result<serde_json::Value, String> {
    use std::io::Read;
    let text = if path == "-" {
        let mut buf = String::new();
        std::io::stdin()
            .read_to_string(&mut buf)
            .map_err(|e| e.to_string())?;
        buf
    } else {
        fs::read_to_string(path).map_err(|e| e.to_string())?
    };
    serde_json::from_str(&text).map_err(|e| format!("results are not valid JSON: {e}"))
}

/// Collect one [`HookSpec`] per `.py` file in a directory, or a single script.
fn discover_hook_specs(
    path: &std::path::Path,
    timeout_secs: u64,
    interpreter: Option<&std::path::Path>,
) -> Result<Vec<HookSpec>, String> {
    let files: Vec<PathBuf> = if path.is_dir() {
        let mut entries: Vec<PathBuf> = fs::read_dir(path)
            .map_err(|e| format!("cannot read hooks dir {}: {e}", path.display()))?
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x == "py"))
            .collect();
        entries.sort();
        entries
    } else if path.is_file() {
        vec![path.to_path_buf()]
    } else {
        return Err(format!("hooks path not found: {}", path.display()));
    };

    if files.is_empty() {
        return Err(format!("no Python hooks found in {}", path.display()));
    }

    let mut specs = Vec::with_capacity(files.len());
    for f in files {
        specs.push(HookSpec {
            script: f,
            interpreter: interpreter.map(PathBuf::from),
            timeout: std::time::Duration::from_secs(timeout_secs),
        });
    }
    Ok(specs)
}

/// Build the JSON envelope a hook-enabled `run` / `hook` emits: the results
/// plus each script's hash and its findings, marked `custom_checks`.
fn envelope_results(
    results: &serde_json::Value,
    specs: &[HookSpec],
    hook_results: &[dmux_hooks::HookResult],
) -> serde_json::Value {
    let mut custom_checks: Vec<serde_json::Value> = Vec::new();
    let mut scripts: Vec<serde_json::Value> = Vec::new();
    let mut warnings: Vec<String> = Vec::new();

    for (spec, res) in specs.iter().zip(hook_results.iter()) {
        let script_name = spec
            .script
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| spec.script.display().to_string());
        for w in &res.warnings {
            warnings.push(format!("{script_name}: {w}"));
        }
        scripts.push(serde_json::json!({
            "script": script_name,
            "sha256": res.script_hash,
            "findings": res.findings.iter().map(finding_json).collect::<Vec<_>>(),
        }));
        for f in &res.findings {
            custom_checks.push(finding_json(f));
        }
    }

    let mut envelope = serde_json::json!({
        "results": results,
        "hooks": {
            "custom_checks": custom_checks,
            "scripts": scripts,
            "warnings": warnings,
        }
    });

    // A self-verifying SHA-256 over the whole envelope, so a reader can detect
    // tampering with either the results or the hook evidence (script hashes,
    // findings).  The digest covers everything except the field itself.
    let envelope_sha256 = dmux_hooks::envelope_sha256(&envelope);
    envelope["envelope_sha256"] = serde_json::Value::String(envelope_sha256);
    envelope
}

fn finding_json(f: &Finding) -> serde_json::Value {
    let mut v = serde_json::json!({
        "id": f.id,
        "severity": severity_str(f.severity),
        "message": f.message,
        "source": "custom_checks"
    });
    if let Some(t) = &f.table {
        v["table"] = serde_json::Value::String(t.clone());
    }
    v
}

fn severity_str(s: Severity) -> &'static str {
    match s {
        Severity::Info => "info",
        Severity::Warning => "warning",
        Severity::Fail => "fail",
    }
}

/// True when any `fail` finding means the overall verdict must not be PASS
/// (PLAN 4.9: a `fail` finding forces the verdict off PASS; info/warning do not,
/// and findings can never raise a non-PASS to PASS).
fn hook_causes_non_pass(results: &[dmux_hooks::HookResult]) -> bool {
    results
        .iter()
        .any(|r| r.findings.iter().any(|f| f.severity == Severity::Fail))
}

/// Compute the exit code a hook-enabled run should return.
///
/// A custom-check `fail` finding ⇒ FAIL (exit 1), taking precedence over any
/// hook warning.  A hook timeout / crash / invalid output is a warning that
/// lifts a clean PASS to exit 2, but never overrides an existing non-PASS
/// verdict from the comparison itself.
fn hook_exit_code(has_fail: bool, has_warning: bool, base: u8) -> u8 {
    if has_fail {
        EXIT_FAIL
    } else if has_warning && base == EXIT_PASS {
        EXIT_WARN
    } else {
        base
    }
}

async fn run_run(args: RunArgs) -> ExitCode {
    let mut config = match load_definition(&args.definition) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("dmux run failed: {e}");
            return ExitCode::from(EXIT_FAIL);
        }
    };

    config.resume_store = args.resume.map(std::path::PathBuf::from);

    let outcome = match run(&config).await {
        Ok(o) => o,
        Err(e) => {
            eprintln!("dmux run failed: {e}");
            return ExitCode::from(EXIT_FAIL);
        }
    };

    let mut exit_code = outcome.exit_code;

    // Hooks are opt-in and off by default (PLAN 4.9).
    if let Some(dir) = args.hooks.as_deref() {
        let mut specs = match discover_hook_specs(dir, args.hook_timeout, args.python.as_deref()) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("dmux run: hooks: {e}");
                return ExitCode::from(EXIT_FAIL);
            }
        };
        for spec in &mut specs {
            if let Some(p) = args.python.as_deref() {
                spec.interpreter = Some(p.to_path_buf());
            }
        }

        let results_value = serde_json::to_value(&outcome.results).unwrap_or_default();
        let hook_results = match dmux_hooks::run_hooks(&specs, &results_value) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("dmux run: hooks: {e}");
                return ExitCode::from(EXIT_FAIL);
            }
        };

        // The written verdict must match the exit code.  A fail finding ⇒
        // verdict FAIL; a hook warning (timeout / crash / invalid / uncollected
        // output) on an otherwise-PASS run ⇒ verdict WARNING with a recorded
        // reason.  Neither may ever raise a non-PASS verdict back to PASS.
        let has_fail = hook_causes_non_pass(&hook_results);
        let has_warning = hook_results.iter().any(|r| !r.warnings.is_empty());
        exit_code = hook_exit_code(has_fail, has_warning, exit_code);

        // Mutate a typed copy of the results (so results_sha256 is recomputed
        // over exactly what is written) and re-serialise.
        let mut typed = outcome.results.clone();
        apply_hook_verdict(&mut typed, has_fail, has_warning, &hook_results);

        // Recompute the results-level self-hash over the mutated content, then
        // wrap everything in the envelope with its own SHA-256 so the hook
        // evidence (script hashes, findings) is tamper-covered too.
        typed.results_sha256 = dmux_engine::results::sha256_of(&typed);
        let results_value = serde_json::to_value(&typed).unwrap_or_default();
        let envelope = envelope_results(&results_value, &specs, &hook_results);
        emit_json_value(args.out.as_deref(), &envelope);
    } else {
        emit_results(args.out.as_deref(), &outcome.results);
    }

    ExitCode::from(exit_code)
}

/// Fold the hook outcome into the typed results so the written verdict matches
/// the exit code.
///
/// - A `fail` finding turns a PASS or an ENGINE-produced WARNING into FAIL with
///   reason "custom check <id> failed" recorded per finding; a hook fail is
///   more severe than a warning, so WARNING is treated like PASS here.
/// - A hook warning (timeout / crash / invalid / uncollected output) without a
///   fail finding turns a PASS into WARNING with a recorded reason.
/// - A fail finding never lowers an existing graver verdict (FAIL, TOOL_FAULT,
///   CHANGED_DURING_CHECK, NEEDS_REVIEW, WIDELY_DIFFERENT) back toward PASS, and
///   a warning never lowers a FAIL: the hook can only force the verdict off
///   PASS, never mask a data or tool problem.
fn apply_hook_verdict(
    results: &mut RunResults,
    has_fail: bool,
    has_warning: bool,
    hook_results: &[dmux_hooks::HookResult],
) {
    if has_fail {
        // A hook `fail` must demote both a clean PASS and an engine WARNING; it
        // must never raise a graver verdict.  Treat WARNING like PASS.
        if matches!(results.verdict, Verdict::Pass | Verdict::Warning) {
            results.verdict = Verdict::Fail;
        }
        let fail_ids: Vec<String> = hook_results
            .iter()
            .flat_map(|r| r.findings.iter())
            .filter(|f| f.severity == Severity::Fail)
            .map(|f| f.id.clone())
            .collect();
        for id in fail_ids {
            results.warnings.push(format!("custom check {id} failed"));
        }
    } else if has_warning && results.verdict == Verdict::Pass {
        results.verdict = Verdict::Warning;
        results
            .warnings
            .push("a custom check hook reported a problem; see hooks.warnings".to_string());
    }
}

/// The on-disk run definition (TOML or JSON).  This is the shape `dmux run`
/// accepts; it maps onto [`RunConfig`].
#[derive(serde::Deserialize, Debug)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
struct RunDefinition {
    #[serde(default)]
    old: Option<ConnDef>,
    #[serde(default)]
    new: Option<ConnDef>,
    #[serde(default)]
    old_file: Option<String>,
    #[serde(default)]
    new_file: Option<String>,
    #[serde(default = "default_level")]
    level: String,
    #[serde(default)]
    levels: BTreeMap<String, String>,
    #[serde(default)]
    control_total: Vec<String>,
    #[serde(default)]
    mapping: Option<String>,
    #[serde(default)]
    old_parallelism: Option<usize>,
    #[serde(default)]
    new_parallelism: Option<usize>,
    #[serde(default)]
    allow_superuser: bool,
    #[serde(default)]
    reason: Option<String>,
    #[serde(default)]
    export_keys: bool,
}

fn default_level() -> String {
    "full-values".to_string()
}

#[derive(serde::Deserialize, Debug, Clone)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
struct ConnDef {
    host: String,
    port: u16,
    db: String,
    user: String,
    password: Option<String>,
    password_env: Option<String>,
    #[serde(default = "default_tls")]
    tls: String,
    reason: Option<String>,
}

fn default_tls() -> String {
    "verify".to_string()
}

fn load_definition(path: &str) -> Result<RunConfig, String> {
    let text = fs::read_to_string(path).map_err(|e| e.to_string())?;
    let def: RunDefinition = if path.ends_with(".toml") {
        toml::from_str(&text).map_err(|e| e.to_string())?
    } else {
        serde_json::from_str(&text).map_err(|e| e.to_string())?
    };

    let conn_def_to_profile = |c: &ConnDef| -> Result<ConnProfile, String> {
        let tls_mode = if c.tls == "disable" {
            TlsMode::Disable(c.reason.clone().unwrap_or_default())
        } else {
            TlsMode::VerifyFull
        };
        let password = match (&c.password, &c.password_env) {
            (Some(p), _) => p.clone(),
            (None, Some(var)) => std::env::var(var)
                .map_err(|_| format!("password environment variable {var} is not set"))?,
            (None, None) => String::new(),
        };
        Ok(ConnProfile::new(
            c.host.clone(),
            c.port,
            c.db.clone(),
            c.user.clone(),
            password,
            tls_mode,
        ))
    };

    let old = match (&def.old, &def.old_file) {
        (Some(c), _) => Input::Live(conn_def_to_profile(c)?),
        (None, Some(f)) => Input::File(
            FingerprintFile::read_from(std::path::Path::new(f)).map_err(|e| e.to_string())?,
        ),
        _ => return Err("definition must set `old` or `old_file`".to_string()),
    };
    let new = match (&def.new, &def.new_file) {
        (Some(c), _) => Input::Live(conn_def_to_profile(c)?),
        (None, Some(f)) => Input::File(
            FingerprintFile::read_from(std::path::Path::new(f)).map_err(|e| e.to_string())?,
        ),
        _ => return Err("definition must set `new` or `new_file`".to_string()),
    };

    let mapping = match def.mapping.as_ref() {
        Some(p) => {
            let bytes = fs::read(p).map_err(|e| e.to_string())?;
            mapping_from_json(&bytes).map_err(|e| e.to_string())?
        }
        None => Mapping::new(),
    };

    let default_level = parse_level(&def.level)?;
    let mut levels = BTreeMap::new();
    for (k, v) in &def.levels {
        levels.insert(k.clone(), parse_level(v)?);
    }

    if def.allow_superuser && def.reason.as_deref().is_none_or(|r| r.trim().is_empty()) {
        return Err("allow_superuser requires a non-empty reason".to_string());
    }

    Ok(RunConfig {
        old,
        new,
        mapping,
        default_level,
        levels,
        control_total_columns: def.control_total.into_iter().collect(),
        old_parallelism: def.old_parallelism.unwrap_or(2),
        new_parallelism: def.new_parallelism.unwrap_or(4),
        allow_superuser: def.allow_superuser,
        export_keys: def.export_keys,
        resume_store: None,
    })
}

/// Minimal `RunResults` for the `apply_hook_verdict` tests (the engine type,
/// but built by hand so no live run is needed).
#[cfg(test)]
fn minimal_run_results(verdict: Verdict) -> RunResults {
    RunResults {
        results_version: 1,
        run_id: "run".into(),
        tool_version: "0.0.1".into(),
        mapping_hash: "m".into(),
        verdict,
        coverage_line: String::new(),
        fully_checked: 0,
        counts_only: 0,
        ignored: 0,
        tables: BTreeMap::new(),
        ignored_items: vec![],
        changed_during_check: vec![],
        warnings: vec![],
        wal_start_lsn: None,
        wal_end_lsn: None,
        results_sha256: String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        allow_superuser_requires_reason, apply_hook_verdict, hook_causes_non_pass, hook_exit_code,
        minimal_run_results, parse_level, parse_tls_mode, Cli, Command, DiffArgs, Severity,
        EXIT_FAIL, EXIT_PASS, EXIT_WARN,
    };
    use clap::Parser;
    use dmux_engine::db::TlsMode;
    use dmux_engine::results::{CheckLevel, Verdict};

    #[test]
    fn exit_codes_match_the_plan() {
        assert_eq!(EXIT_PASS, 0);
        assert_eq!(EXIT_FAIL, 1);
        assert_eq!(EXIT_WARN, 2);
    }

    #[test]
    fn parses_levels() {
        assert_eq!(parse_level("structure").unwrap(), CheckLevel::Structure);
        assert_eq!(parse_level("counts").unwrap(), CheckLevel::CountsAndTotals);
        assert_eq!(parse_level("full-values").unwrap(), CheckLevel::FullValues);
        assert!(parse_level("bogus").is_err());
    }

    #[test]
    fn parses_stub_commands() {
        fn parse(args: &[&str]) -> Command {
            Cli::try_parse_from(std::iter::once("dmux").chain(args.iter().copied()))
                .expect("args should parse")
                .command
        }
        assert!(matches!(
            parse(&[
                "probe",
                "--host",
                "h",
                "--port",
                "5433",
                "--db",
                "b",
                "--user",
                "u",
                "--password-env",
                "P",
                "--tls",
                "verify"
            ]),
            Command::Probe(_)
        ));
        assert!(matches!(
            parse(&[
                "capture", "--host", "h", "--port", "5433", "--db", "b", "--user", "u", "--out",
                "o.json"
            ]),
            Command::Capture(_)
        ));
        assert!(matches!(
            parse(&["compare", "--old", "a", "--new", "b"]),
            Command::Compare(_)
        ));
        assert!(matches!(parse(&["run", "def.toml"]), Command::Run(_)));
    }

    #[test]
    fn tls_modes() {
        assert_eq!(
            parse_tls_mode("verify", &None).unwrap(),
            TlsMode::VerifyFull
        );
        assert!(parse_tls_mode("disable", &None).is_err());
        assert!(parse_tls_mode("disable", &Some("  ".into())).is_err());
        assert_eq!(
            parse_tls_mode("disable", &Some("lab".into())).unwrap(),
            TlsMode::Disable("lab".to_string())
        );
        assert!(parse_tls_mode("fingerprint:", &None).is_err());
        match parse_tls_mode("fingerprint:abcd", &None).unwrap() {
            TlsMode::TrustFingerprint(fp) => assert_eq!(fp, "abcd"),
            _ => panic!("expected fingerprint"),
        }
    }

    #[test]
    fn superuser_reason_required() {
        assert!(!allow_superuser_requires_reason(&None));
        assert!(!allow_superuser_requires_reason(&Some("  ".into())));
        assert!(allow_superuser_requires_reason(&Some("lab".into())));
    }

    #[test]
    fn diff_args_shape() {
        let cmd = Cli::try_parse_from(["dmux", "diff", "--old", "a.json", "--new", "b.json"])
            .expect("parse");
        match cmd.command {
            Command::Diff(DiffArgs { old, new }) => {
                assert_eq!(old, std::path::PathBuf::from("a.json"));
                assert_eq!(new, std::path::PathBuf::from("b.json"));
            }
            _ => panic!("expected diff"),
        }
    }

    #[test]
    fn run_args_accept_hooks_opt_in() {
        // Hooks are opt-in: without `--hooks` the field is None.
        let cmd = Cli::try_parse_from(["dmux", "run", "def.toml"]).expect("parse");
        match cmd.command {
            Command::Run(args) => {
                assert!(args.hooks.is_none());
                assert_eq!(args.hook_timeout, 60);
                assert!(args.python.is_none());
            }
            _ => panic!("expected run"),
        }

        let cmd = Cli::try_parse_from([
            "dmux",
            "run",
            "def.toml",
            "--hooks",
            "hooks/",
            "--hook-timeout",
            "30",
            "--python",
            "/usr/bin/python3",
        ])
        .expect("parse");
        match cmd.command {
            Command::Run(args) => {
                assert_eq!(args.hooks, Some(std::path::PathBuf::from("hooks/")));
                assert_eq!(args.hook_timeout, 30);
                assert_eq!(
                    args.python,
                    Some(std::path::PathBuf::from("/usr/bin/python3"))
                );
            }
            _ => panic!("expected run"),
        }
    }

    #[test]
    fn hook_args_shape() {
        let cmd = Cli::try_parse_from(["dmux", "hook", "results.json", "hooks/"]).expect("parse");
        match cmd.command {
            Command::Hook(args) => {
                assert_eq!(args.results, "results.json");
                assert_eq!(args.hooks, std::path::PathBuf::from("hooks/"));
                assert_eq!(args.timeout, 60);
            }
            _ => panic!("expected hook"),
        }
    }

    #[test]
    fn hook_cause_non_pass_only_on_fail() {
        use dmux_hooks::Finding;
        let info = dmux_hooks::HookResult {
            script_hash: "h".into(),
            findings: vec![Finding {
                id: "a".into(),
                severity: Severity::Info,
                table: None,
                message: "m".into(),
            }],
            warnings: vec![],
        };
        assert!(!hook_causes_non_pass(&[info]));

        let fail = dmux_hooks::HookResult {
            script_hash: "h".into(),
            findings: vec![Finding {
                id: "a".into(),
                severity: Severity::Fail,
                table: None,
                message: "m".into(),
            }],
            warnings: vec![],
        };
        assert!(hook_causes_non_pass(&[fail]));
    }

    #[test]
    fn custom_fail_forces_verdict_fail_in_written_results() {
        use dmux_hooks::Finding;
        let fail = dmux_hooks::HookResult {
            script_hash: "h".into(),
            findings: vec![Finding {
                id: "custom.total".into(),
                severity: Severity::Fail,
                table: None,
                message: "over threshold".into(),
            }],
            warnings: vec![],
        };

        // A PASS verdict must flip to FAIL with the reason recorded.
        let mut pass_results = minimal_run_results(Verdict::Pass);
        apply_hook_verdict(&mut pass_results, true, false, std::slice::from_ref(&fail));
        assert_eq!(pass_results.verdict, Verdict::Fail);
        assert!(
            pass_results
                .warnings
                .iter()
                .any(|w| w == "custom check custom.total failed"),
            "reason not recorded: {:?}",
            pass_results.warnings
        );

        // An already-FAIL verdict must never be raised back to PASS.
        let mut fail_results = minimal_run_results(Verdict::Fail);
        apply_hook_verdict(&mut fail_results, true, false, std::slice::from_ref(&fail));
        assert_eq!(fail_results.verdict, Verdict::Fail);

        // A non-PASS, non-FAIL verdict stays as-is (never lowered to PASS).
        let mut tf = minimal_run_results(Verdict::ToolFault);
        apply_hook_verdict(&mut tf, true, false, &[fail]);
        assert_eq!(tf.verdict, Verdict::ToolFault);
    }

    #[test]
    fn engine_warning_plus_custom_fail_yields_verdict_fail_and_exit_1() {
        // Starting from a results file whose ENGINE verdict is WARNING (e.g. a
        // SQL_ASCII flag), a custom-check fail must turn it into FAIL and exit 1,
        // not stay WARNING / exit 2.
        let fail = dmux_hooks::HookResult {
            script_hash: "h".into(),
            findings: vec![dmux_hooks::Finding {
                id: "custom.total".into(),
                severity: Severity::Fail,
                table: None,
                message: "over".into(),
            }],
            warnings: vec![],
        };
        let engine_warning = minimal_run_results(Verdict::Warning);
        let mut results = engine_warning.clone();
        apply_hook_verdict(&mut results, true, false, std::slice::from_ref(&fail));

        // File verdict: WARNING ⇒ FAIL.
        assert_eq!(results.verdict, Verdict::Fail);
        assert!(
            results
                .warnings
                .iter()
                .any(|w| w == "custom check custom.total failed"),
            "reason not recorded: {:?}",
            results.warnings
        );

        // Exit code: a base WARNING (2) plus a fail finding ⇒ FAIL (1).
        assert_eq!(hook_exit_code(true, false, EXIT_WARN), EXIT_FAIL);

        // The results hash must be recomputed over the mutated file and change.
        results.results_sha256 = dmux_engine::results::sha256_of(&results);
        assert_ne!(
            dmux_engine::results::sha256_of(&engine_warning),
            results.results_sha256,
            "the mutated file must carry a different digest"
        );
        assert_eq!(
            dmux_engine::results::sha256_of(&results),
            results.results_sha256
        );
    }

    #[test]
    fn warning_plus_fail_yields_verdict_fail() {
        use dmux_hooks::Finding;
        let fail = dmux_hooks::HookResult {
            script_hash: "h".into(),
            findings: vec![Finding {
                id: "custom.total".into(),
                severity: Severity::Fail,
                table: None,
                message: "over".into(),
            }],
            warnings: vec!["timed out".into()],
        };

        // WARNING + fail ⇒ file verdict FAIL (fail takes precedence); a warning
        // must never lower a FAIL.
        let mut results = minimal_run_results(Verdict::Pass);
        apply_hook_verdict(&mut results, true, true, &[fail]);
        assert_eq!(results.verdict, Verdict::Fail);
    }

    #[test]
    fn hook_warning_on_pass_yields_verdict_warning() {
        // A hook that timed out / crashed / produced invalid or uncollected
        // output on an otherwise-PASS run ⇒ file verdict WARNING with a reason.
        let warn = dmux_hooks::HookResult {
            script_hash: "h".into(),
            findings: vec![],
            warnings: vec!["hook timed out after 1000 ms".into()],
        };
        let mut results = minimal_run_results(Verdict::Pass);
        apply_hook_verdict(&mut results, false, true, &[warn]);
        assert_eq!(results.verdict, Verdict::Warning);
        assert!(
            results
                .warnings
                .iter()
                .any(|w| w.contains("custom check hook reported a problem")),
            "warning reason not recorded: {:?}",
            results.warnings
        );

        // A hook warning never downgrades an existing non-PASS verdict.
        let warn2 = dmux_hooks::HookResult {
            script_hash: "h".into(),
            findings: vec![],
            warnings: vec!["timed out".into()],
        };
        let mut fail = minimal_run_results(Verdict::Fail);
        apply_hook_verdict(&mut fail, false, true, &[warn2]);
        assert_eq!(fail.verdict, Verdict::Fail);
    }

    #[test]
    fn custom_fail_with_no_fail_findings_is_a_noop() {
        let mut results = minimal_run_results(Verdict::Pass);
        let info = dmux_hooks::HookResult {
            script_hash: "h".into(),
            findings: vec![dmux_hooks::Finding {
                id: "a".into(),
                severity: Severity::Info,
                table: None,
                message: "m".into(),
            }],
            warnings: vec![],
        };
        apply_hook_verdict(&mut results, false, false, &[info]);
        assert_eq!(results.verdict, Verdict::Pass);
    }

    #[test]
    fn warning_plus_fail_is_fail_not_warning() {
        // A fail finding must yield FAIL (1), even when warnings are present:
        // fail takes precedence over warning.
        assert_eq!(hook_exit_code(true, true, EXIT_PASS), EXIT_FAIL);
        // Fail alone ⇒ FAIL.
        assert_eq!(hook_exit_code(true, false, EXIT_PASS), EXIT_FAIL);
        // Warning alone on a clean PASS ⇒ WARN (2).
        assert_eq!(hook_exit_code(false, true, EXIT_PASS), EXIT_WARN);
        // No hook signal leaves the base code untouched.
        assert_eq!(hook_exit_code(false, false, EXIT_PASS), EXIT_PASS);
        assert_eq!(hook_exit_code(false, false, EXIT_FAIL), EXIT_FAIL);
        // A warning never overrides an existing non-PASS verdict.
        assert_eq!(hook_exit_code(false, true, EXIT_FAIL), EXIT_FAIL);
    }

    #[test]
    fn custom_fail_recomputes_the_results_hash() {
        // The mutated results must carry a results_sha256 that matches the
        // actual (mutated) content — the whole point of fix 1.
        let fail = dmux_hooks::HookResult {
            script_hash: "h".into(),
            findings: vec![dmux_hooks::Finding {
                id: "custom.total".into(),
                severity: Severity::Fail,
                table: None,
                message: "over".into(),
            }],
            warnings: vec![],
        };
        let mut results = minimal_run_results(Verdict::Pass);
        apply_hook_verdict(&mut results, true, false, &[fail]);
        // Recompute exactly as run_run does before writing.
        results.results_sha256 = dmux_engine::results::sha256_of(&results);

        // A FAIL→PASS edit must fail verification.
        let mut edited = results.clone();
        edited.verdict = Verdict::Pass;
        assert_ne!(
            dmux_engine::results::sha256_of(&edited),
            results.results_sha256,
            "editing FAIL back to PASS must change the digest"
        );
        // The untouched content still matches.
        assert_eq!(
            dmux_engine::results::sha256_of(&results),
            results.results_sha256
        );
    }
}
