//! `dmux` command-line runner.
//!
//! `probe`, `inventory`, `diff`, `capture`, `compare` and `run` share the engine.
//! Exit codes follow PLAN.md 4.8: 0 pass, 1 fail, 2 warnings.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Args, Parser, Subcommand};
use dmux_engine::core::normalise::ColumnOptions;
use dmux_engine::db::{probe_server, ConnProfile, DbError, ReadOnlySession, TlsMode};
use dmux_engine::fingerprint_file::FingerprintFile;
use dmux_engine::inventory::collect_inventory;
use dmux_engine::mapping::{from_json as mapping_from_json, Mapping};
use dmux_engine::results::{CheckLevel, RunResults, Verdict};
use dmux_engine::run::{capture, compare_files, run, Input, RunConfig};
use dmux_engine::schema_diff::{compare_inventories, DiffStatus};
use dmux_hooks::{Finding, HookSpec, Severity};
use dmux_report::verify::verify_results_file;

/// PLAN.md 4.8 exit codes.
const EXIT_PASS: u8 = 0;
const EXIT_FAIL: u8 = 1;
const EXIT_WARN: u8 = 2;

/// Upper bound on an inventory JSON file `dmux diff` will read (audit L2).
const MAX_INVENTORY_BYTES: u64 = 64 * 1024 * 1024;

/// The honest hook warning (decision 23: hooks are not sandboxed in v1.0).
const HOOK_WARNING: &str =
    "warning: a check runs with your full permissions; only run scripts you trust";

/// Print the one-time hook warning to stderr (plain text, once per invocation).
fn warn_about_hooks() {
    eprintln!("{HOOK_WARNING}");
}

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
    /// Render the PDF summary, Excel detail and run log from a results file.
    Report(ReportArgs),
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
    /// A PEM file of extra CA certificates to trust when verifying the
    /// server (for example the bank's internal CA).  The host name is still
    /// checked.
    #[arg(long, value_name = "PATH")]
    ca_file: Option<String>,
    #[arg(long, value_name = "TEXT")]
    reason: Option<String>,
    #[arg(long)]
    allow_superuser: bool,
}

/// A connection-configuration error that carries **no secret-derived data**: each
/// variant names only a field or an error kind, and [`ConfigError::message`]
/// renders a fixed string.  A caller that prints the message can therefore never
/// leak a password or any other user-supplied value (CodeQL rust/cleartext-logging).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ConfigError {
    /// `--password-env <VAR>` names an environment variable that is not set.
    PasswordEnvUnset,
    /// `--tls disable` without a `--reason`.
    TlsDisableNeedsReason,
    /// `--tls disable` with an empty `--reason`.
    TlsReasonEmpty,
    /// `--tls fingerprint:<hex>` with an empty hex value.
    TlsFingerprintEmpty,
    /// An unrecognised `--tls` value.
    TlsUnknown,
    /// `--allow-superuser` without a non-empty `--reason`.
    SuperuserNeedsReason,
    /// `--ca-file` with an empty path.
    CaFileEmpty,
    /// `--ca-file` together with `--tls disable`: no certificate is checked.
    CaFileWithoutTls,
}

impl ConfigError {
    fn message(self) -> &'static str {
        match self {
            ConfigError::PasswordEnvUnset => "password environment variable is not set",
            ConfigError::TlsDisableNeedsReason => "--reason <text> is required for --tls disable",
            ConfigError::TlsReasonEmpty => "--reason <text> may not be empty",
            ConfigError::TlsFingerprintEmpty => "--tls fingerprint:<hex> is missing the hex value",
            ConfigError::TlsUnknown => {
                "unknown --tls value (expected verify, fingerprint:<hex>, or disable)"
            }
            ConfigError::SuperuserNeedsReason => "--allow-superuser requires --reason <text>",
            ConfigError::CaFileEmpty => "--ca-file <path> may not be empty",
            ConfigError::CaFileWithoutTls => {
                "--ca-file cannot be used with --tls disable: an unencrypted connection checks no certificate"
            }
        }
    }
}

impl ConnArgs {
    fn profile(&self) -> Result<ConnProfile, ConfigError> {
        let tls_mode = parse_tls_mode(&self.tls, &self.reason)?;
        let password = match self.password_env.as_deref().map(std::env::var) {
            Some(Ok(p)) => p,
            Some(Err(_)) => return Err(ConfigError::PasswordEnvUnset),
            None => String::new(),
        };
        if self.allow_superuser && !allow_superuser_requires_reason(&self.reason) {
            return Err(ConfigError::SuperuserNeedsReason);
        }
        let ca_file = check_ca_file(self.ca_file.as_deref(), &tls_mode)?;
        let profile = ConnProfile::new(
            self.host.clone(),
            self.port,
            self.db.clone(),
            self.user.clone(),
            password,
            tls_mode,
        );
        Ok(match ca_file {
            Some(path) => profile.with_ca_file(path),
            None => profile,
        })
    }
}

/// Validate a `--ca-file` / `ca_file` setting against the TLS mode.  The file
/// itself is read when connecting; a missing or invalid PEM is then a TLS
/// error that names only the path.
fn check_ca_file<'a>(
    ca_file: Option<&'a str>,
    tls_mode: &TlsMode,
) -> Result<Option<&'a str>, ConfigError> {
    match ca_file {
        None => Ok(None),
        Some(p) if p.trim().is_empty() => Err(ConfigError::CaFileEmpty),
        Some(_) if matches!(tls_mode, TlsMode::Disable(_)) => Err(ConfigError::CaFileWithoutTls),
        Some(p) => Ok(Some(p)),
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

/// Report arguments: render artefacts from a results file (PLAN.md 4.7).
#[derive(Args, Debug)]
struct ReportArgs {
    /// The results JSON file written by `compare` / `run` (SHA-256 verified).
    #[arg(long, value_name = "FILE")]
    results: PathBuf,
    /// Write the PDF summary to this path.
    #[arg(long, value_name = "FILE")]
    pdf: Option<PathBuf>,
    /// Write the Excel detail workbook to this path.
    #[arg(long, value_name = "FILE")]
    xlsx: Option<PathBuf>,
    /// Write the run log to this path (`.json` for JSON, text otherwise).
    #[arg(long, value_name = "FILE")]
    log: Option<PathBuf>,
    /// Include key / covered-column names in the detail (default: off).
    #[arg(long)]
    include_keys: bool,
}

/// Parse the `--tls` mode string into a [`TlsMode`].  Errors are a
/// [`ConfigError`] kind — never the raw value — so a caller never logs a
/// user-controlled string.
fn parse_tls_mode(tls: &str, reason: &Option<String>) -> Result<TlsMode, ConfigError> {
    match tls {
        "verify" => Ok(TlsMode::VerifyFull),
        "disable" => {
            let reason = reason
                .as_deref()
                .ok_or(ConfigError::TlsDisableNeedsReason)?;
            if reason.trim().is_empty() {
                return Err(ConfigError::TlsReasonEmpty);
            }
            Ok(TlsMode::Disable(reason.to_string()))
        }
        _ => {
            if let Some(hex) = tls.strip_prefix("fingerprint:") {
                if hex.is_empty() {
                    return Err(ConfigError::TlsFingerprintEmpty);
                }
                Ok(TlsMode::TrustFingerprint(hex.to_string()))
            } else {
                Err(ConfigError::TlsUnknown)
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
        Command::Report(args) => run_report(args),
        Command::Hook(args) => run_hook(args).await,
    }
}

async fn run_probe(args: ConnArgs) -> ExitCode {
    let profile = match args.profile() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("configuration error: {}", e.message());
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
            eprintln!("configuration error: {}", e.message());
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
        let old = read_inventory_json(&args.old)?;
        let new = read_inventory_json(&args.new)?;
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

/// Read one inventory JSON file, refusing anything at or over the size limit
/// before it is parsed (audit L2: unbounded input can exhaust memory).
fn read_inventory_json(
    path: &std::path::Path,
) -> Result<dmux_engine::inventory::DatabaseInventory, String> {
    let meta = fs::metadata(path).map_err(|e| e.to_string())?;
    if meta.len() > MAX_INVENTORY_BYTES {
        return Err(format!(
            "inventory file {} is {} bytes, over the {}-byte limit",
            path.display(),
            meta.len(),
            MAX_INVENTORY_BYTES
        ));
    }
    let text = fs::read_to_string(path).map_err(|e| e.to_string())?;
    serde_json::from_str(&text).map_err(|e| e.to_string())
}

async fn run_capture(args: CaptureArgs) -> ExitCode {
    let profile = match args.conn.profile() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("configuration error: {}", e.message());
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
            let bytes = match tokio::fs::read(path).await {
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

    let column_options = BTreeMap::new();
    let opts = capture::CaptureOptions {
        level,
        control_total_columns: &control_total,
        column_options: &column_options,
        mapping: &mapping,
        allow_superuser: args.conn.allow_superuser,
    };
    let file = match capture::capture(&profile, &opts).await {
        Ok(f) => f,
        Err(e) => {
            eprintln!("dmux capture failed: {e}");
            return ExitCode::from(EXIT_FAIL);
        }
    };

    if let Err(e) = write_fingerprint_file(&args.out, &file).await {
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

/// Write a fingerprint file without blocking the async runtime (Sonar S7493),
/// created with mode 0600 on Unix (audit L3).
async fn write_fingerprint_file(
    path: &std::path::Path,
    file: &FingerprintFile,
) -> Result<(), String> {
    let bytes = file.clone().to_json().map_err(|e| e.to_string())?;
    let mut opts = tokio::fs::OpenOptions::new();
    opts.create(true).write(true).truncate(true);
    #[cfg(unix)]
    {
        opts.mode(0o600);
    }
    let mut f = opts.open(path).await.map_err(|e| e.to_string())?;
    tokio::io::AsyncWriteExt::write_all(&mut f, &bytes)
        .await
        .map_err(|e| e.to_string())
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

/// Render the report artefacts from a results file, verifying its SHA-256 first
/// (refuse tampered input — PLAN.md 2.4).
fn run_report(args: ReportArgs) -> ExitCode {
    // Verify the results file before rendering anything (refuse tampered input).
    let verification = match verify_results_file(&args.results) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("dmux report: {e}");
            return ExitCode::from(EXIT_FAIL);
        }
    };

    let options = dmux_report::ReportOptions {
        include_keys: args.include_keys,
    };
    let layout = dmux_report::build_layout(&verification, &options);
    let sha256 = verification.sha256;

    if let Some(pdf) = args.pdf.as_deref() {
        if let Err(e) = dmux_report::pdf::write_summary_layout(&layout, &sha256, pdf) {
            eprintln!("dmux report: {e}");
            return ExitCode::from(EXIT_FAIL);
        }
    }
    if let Some(xlsx) = args.xlsx.as_deref() {
        if let Err(e) = dmux_report::xlsx::write_detail_layout(&layout, &sha256, xlsx) {
            eprintln!("dmux report: {e}");
            return ExitCode::from(EXIT_FAIL);
        }
    }
    if let Some(log) = args.log.as_deref() {
        if let Err(e) = dmux_report::log::write_run_log(
            &verification.results,
            verification.hooks.as_ref(),
            &options,
            log,
        ) {
            eprintln!("dmux report: {e}");
            return ExitCode::from(EXIT_FAIL);
        }
    }

    if args.pdf.is_none() && args.xlsx.is_none() && args.log.is_none() {
        eprintln!("dmux report: nothing to do (pass --pdf, --xlsx and/or --log)");
        return ExitCode::from(EXIT_FAIL);
    }

    ExitCode::from(EXIT_PASS)
}

/// Write bytes to a file, creating it with mode 0600 on Unix so results,
/// fingerprints and other CLI output never become world-readable (audit L3).
/// Preserves `fs::write`'s create/truncate overwrite behaviour.
fn write_private_file(path: &std::path::Path, contents: &[u8]) -> Result<(), String> {
    let mut opts = fs::OpenOptions::new();
    opts.create(true).write(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    opts.open(path)
        .and_then(|mut f| {
            use std::io::Write;
            f.write_all(contents)
        })
        .map_err(|e| e.to_string())
}

fn emit_results(path: Option<&std::path::Path>, results: &dmux_engine::results::RunResults) {
    let json = serde_json::to_string_pretty(results).unwrap_or_else(|_| "{}".to_string());
    match path {
        Some(p) => {
            if let Err(e) = write_private_file(p, json.as_bytes()) {
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
            if let Err(e) = write_private_file(p, json.as_bytes()) {
                eprintln!("dmux: failed to write results: {e}");
            }
        }
        None => println!("{json}"),
    }
}

/// Run the `dmux hook` command: read a results file (or stdin), run the given
/// hook script(s), and print their findings as JSON marked `custom_checks`.
async fn run_hook(args: HookArgs) -> ExitCode {
    warn_about_hooks();
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
        // A world-writable or foreign-owned hooks directory lets someone swap
        // the scripts Dmux discovers; refuse it up front (decision 23). The
        // individual script files are re-checked in `run_hook`.
        dmux_hooks::check_hook_path_permissions(path).map_err(|e| e.to_string())?;
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
        warn_about_hooks();
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
    /// Per-column options (PLAN.md 3.2), by old `schema.table.column`.
    #[serde(default)]
    options: BTreeMap<String, OptionsDef>,
    /// Wait before the final change-counter read, in milliseconds (default
    /// and minimum 2000: the statistics flush lag; see dmux_engine::consistency).
    #[serde(default)]
    stats_settle_ms: Option<u64>,
}

/// The smallest accepted `stats_settle_ms`.
const MIN_STATS_SETTLE_MS: u64 = 2000;

/// The four recorded per-column options, strictly parsed (unknown keys are
/// rejected: a definition file is untrusted input).
#[derive(serde::Deserialize, Debug, Clone, Default)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
struct OptionsDef {
    #[serde(default)]
    ignore_trailing_spaces: bool,
    #[serde(default)]
    null_equals_empty: bool,
    #[serde(default)]
    date_only: bool,
    #[serde(default)]
    round_decimals: Option<u8>,
}

impl From<OptionsDef> for ColumnOptions {
    fn from(o: OptionsDef) -> Self {
        ColumnOptions {
            ignore_trailing_spaces: o.ignore_trailing_spaces,
            null_equals_empty: o.null_equals_empty,
            date_only: o.date_only,
            round_decimals: o.round_decimals,
        }
    }
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
    password_env: Option<String>,
    #[serde(default = "default_tls")]
    tls: String,
    /// A PEM file of extra CA certificates to trust (`--ca-file`).
    #[serde(default)]
    ca_file: Option<String>,
    reason: Option<String>,
    /// A plaintext `password` is not allowed (decision 23 / PLAN 6.5).  This
    /// field exists only to turn its presence into a clear error; it is never
    /// read.  `deny_unknown_fields` would otherwise report a generic "unknown
    /// field" that does not tell the user what to do instead.
    #[serde(rename = "password", deserialize_with = "reject_password", default)]
    _password: Option<serde::de::IgnoredAny>,
}

/// Reject a plaintext `password` in a run definition with a clear message
/// (audit M7): passwords belong in `password_env` or the OS keychain, never in
/// a definition file that gets copied around jump servers.
fn reject_password<'de, D>(_de: D) -> Result<Option<serde::de::IgnoredAny>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Err(serde::de::Error::custom(
        "run definition files may not contain a password; use password_env",
    ))
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
        let tls_mode = parse_tls_mode(&c.tls, &c.reason).map_err(|e| e.message().to_string())?;
        let password = match &c.password_env {
            Some(var) => std::env::var(var)
                .map_err(|_| "password environment variable is not set".to_string())?,
            None => String::new(),
        };
        let ca_file = check_ca_file(c.ca_file.as_deref(), &tls_mode).map_err(|e| match e {
            ConfigError::CaFileEmpty => "ca_file may not be empty".to_string(),
            ConfigError::CaFileWithoutTls => "ca_file cannot be used with tls = \"disable\": \
                 an unencrypted connection checks no certificate"
                .to_string(),
            other => other.message().to_string(),
        })?;
        let profile = ConnProfile::new(
            c.host.clone(),
            c.port,
            c.db.clone(),
            c.user.clone(),
            password,
            tls_mode,
        );
        Ok(match ca_file {
            Some(path) => profile.with_ca_file(path),
            None => profile,
        })
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
    let stats_settle = match def.stats_settle_ms {
        None => dmux_engine::run::DEFAULT_STATS_SETTLE,
        Some(ms) if ms >= MIN_STATS_SETTLE_MS => std::time::Duration::from_millis(ms),
        Some(ms) => {
            return Err(format!(
                "stats_settle_ms must be at least {MIN_STATS_SETTLE_MS} (got {ms}): \
                 shorter waits can miss writes the server has not counted yet"
            ))
        }
    };
    let old_parallelism = def.old_parallelism.unwrap_or(2);
    let new_parallelism = def.new_parallelism.unwrap_or(4);
    if old_parallelism == 0 || new_parallelism == 0 {
        return Err("old_parallelism and new_parallelism must be at least 1".to_string());
    }

    Ok(RunConfig {
        old,
        new,
        mapping,
        default_level,
        levels,
        control_total_columns: def.control_total.into_iter().collect(),
        column_options: def
            .options
            .into_iter()
            .map(|(k, v)| (k, v.into()))
            .collect(),
        old_parallelism,
        new_parallelism,
        allow_superuser: def.allow_superuser,
        allow_superuser_reason: def.reason.clone(),
        export_keys: def.export_keys,
        resume_store: None,
        stats_settle,
        control: None,
    })
}

/// Minimal `RunResults` for the `apply_hook_verdict` tests (the real engine
/// type; built via `Default` so it stays correct as the results schema grows).
#[cfg(test)]
fn minimal_run_results(verdict: Verdict) -> RunResults {
    RunResults {
        verdict,
        ..RunResults::default()
    }
}

#[cfg(test)]
mod tests {
    use super::{
        allow_superuser_requires_reason, apply_hook_verdict, hook_causes_non_pass, hook_exit_code,
        load_definition, minimal_run_results, parse_level, parse_tls_mode, read_inventory_json,
        write_private_file, Cli, Command, ConfigError, ConnArgs, DiffArgs, Severity, EXIT_FAIL,
        EXIT_PASS, EXIT_WARN, MAX_INVENTORY_BYTES,
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
        assert!(matches!(
            parse(&[
                "report",
                "--results",
                "r.json",
                "--pdf",
                "out.pdf",
                "--xlsx",
                "out.xlsx",
                "--log",
                "out.txt",
                "--include-keys"
            ]),
            Command::Report(_)
        ));
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
    fn config_error_never_leaks_a_password_or_user_value() {
        // A live password lives in the environment while an unrelated config
        // error is triggered.  The rendered message must be a fixed ConfigError
        // string — never the password sentinel and never the user-supplied tls
        // value (CodeQL rust/cleartext-logging).
        const SENTINEL: &str = "SENTINEL-PASSWORD-982134";
        std::env::set_var("DMUX_TEST_SENTINEL_PW", SENTINEL);
        let args = ConnArgs {
            host: "h".into(),
            port: 5433,
            db: "d".into(),
            user: "u".into(),
            password_env: Some("DMUX_TEST_SENTINEL_PW".into()),
            tls: "SENTINEL-TLS-VALUE".into(),
            ca_file: None,
            reason: None,
            allow_superuser: false,
        };
        let err = args.profile().unwrap_err();
        assert_eq!(err, ConfigError::TlsUnknown);
        let message = err.message();
        assert!(
            !message.contains(SENTINEL),
            "password leaked into the error message: {message}"
        );
        assert!(
            !message.contains("SENTINEL-TLS-VALUE"),
            "user value leaked into the error message: {message}"
        );
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

    fn definition(name: &str, body: &str) -> Result<dmux_engine::run::RunConfig, String> {
        let dir = std::env::temp_dir().join(format!("dmux-def-{}-{name}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("def.toml");
        std::fs::write(&path, body).unwrap();
        let r = load_definition(path.to_str().unwrap());
        std::fs::remove_dir_all(&dir).unwrap();
        r
    }

    const CONNS: &str = r#"
old = { host = "127.0.0.1", port = 5433, db = "d", user = "u", tls = "disable", reason = "lab" }
new = { host = "127.0.0.1", port = 5434, db = "d", user = "u", tls = "disable", reason = "lab" }
"#;

    #[test]
    fn definition_options_reach_the_run() {
        let c = definition(
            "opts",
            &format!("{CONNS}\n[options.\"s.t.c\"]\nignore_trailing_spaces = true\n"),
        )
        .unwrap();
        assert!(c.column_options["s.t.c"].ignore_trailing_spaces);
    }

    #[test]
    fn definition_rejects_unknown_option_keys_and_zero_limits() {
        assert!(definition(
            "unknown",
            &format!("{CONNS}\n[options.\"s.t.c\"]\ntrim = true\n")
        )
        .is_err());
        assert!(definition("zero", &format!("old_parallelism = 0\n{CONNS}")).is_err());
        assert!(definition(
            "tls",
            &CONNS.replace("tls = \"disable\", reason = \"lab\"", "tls = \"bogus\"")
        )
        .is_err());
    }

    #[test]
    fn stats_settle_defaults_to_two_seconds_and_cannot_go_lower() {
        let c = definition("settle-default", CONNS).unwrap();
        assert_eq!(c.stats_settle, std::time::Duration::from_secs(2));
        let c = definition("settle-5s", &format!("stats_settle_ms = 5000\n{CONNS}")).unwrap();
        assert_eq!(c.stats_settle, std::time::Duration::from_secs(5));
        assert!(definition("settle-low", &format!("stats_settle_ms = 500\n{CONNS}")).is_err());
    }

    #[test]
    fn parses_the_ca_file_option() {
        let cli = Cli::try_parse_from([
            "dmux",
            "probe",
            "--host",
            "h",
            "--port",
            "5432",
            "--db",
            "d",
            "--user",
            "u",
            "--ca-file",
            "/etc/bank/ca.pem",
        ])
        .expect("parse");
        let Command::Probe(args) = cli.command else {
            panic!("expected probe");
        };
        assert_eq!(args.conn.ca_file.as_deref(), Some("/etc/bank/ca.pem"));
        let profile = args.conn.profile().expect("profile");
        assert_eq!(profile.ca_file.as_deref(), Some("/etc/bank/ca.pem"));
        // Without the option there is no CA file.
        let cli = Cli::try_parse_from([
            "dmux", "probe", "--host", "h", "--port", "5432", "--db", "d", "--user", "u",
        ])
        .expect("parse");
        let Command::Probe(args) = cli.command else {
            panic!("expected probe");
        };
        assert_eq!(args.conn.profile().unwrap().ca_file, None);
    }

    fn conn_args(tls: &str, reason: Option<&str>, ca_file: Option<&str>) -> ConnArgs {
        ConnArgs {
            host: "h".into(),
            port: 5432,
            db: "d".into(),
            user: "u".into(),
            password_env: None,
            tls: tls.into(),
            ca_file: ca_file.map(str::to_string),
            reason: reason.map(str::to_string),
            allow_superuser: false,
        }
    }

    #[test]
    fn ca_file_is_refused_when_empty_or_unencrypted() {
        assert_eq!(
            conn_args("verify", None, Some("  ")).profile().unwrap_err(),
            ConfigError::CaFileEmpty
        );
        assert_eq!(
            conn_args("disable", Some("lab"), Some("/ca.pem"))
                .profile()
                .unwrap_err(),
            ConfigError::CaFileWithoutTls
        );
        // A pinned fingerprint may carry a CA file too (it is simply unused).
        let p = conn_args("fingerprint:ab", None, Some("/ca.pem"))
            .profile()
            .unwrap();
        assert_eq!(p.ca_file.as_deref(), Some("/ca.pem"));
        // The messages are fixed strings.
        assert!(ConfigError::CaFileEmpty.message().contains("--ca-file"));
        assert!(ConfigError::CaFileWithoutTls
            .message()
            .contains("--tls disable"));
    }

    #[test]
    fn definition_accepts_a_ca_file() {
        let body = r#"
old = { host = "127.0.0.1", port = 5433, db = "d", user = "u", ca_file = "/etc/bank/ca.pem" }
new = { host = "127.0.0.1", port = 5434, db = "d", user = "u", tls = "disable", reason = "lab" }
"#;
        let cfg = definition("ca", body).expect("definition with ca_file parses");
        match (&cfg.old, &cfg.new) {
            (dmux_engine::run::Input::Live(old), dmux_engine::run::Input::Live(new)) => {
                assert_eq!(old.ca_file.as_deref(), Some("/etc/bank/ca.pem"));
                assert_eq!(new.ca_file, None);
            }
            _ => panic!("expected two live inputs"),
        }
        // JSON definitions take the same key.
        let dir = std::env::temp_dir().join(format!("dmux-def-{}-cajson", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("def.json");
        std::fs::write(
            &path,
            r#"{"old": {"host": "h", "port": 1, "db": "d", "user": "u", "ca_file": "/ca.pem"},
                "new": {"host": "h", "port": 2, "db": "d", "user": "u", "ca_file": "/ca.pem"}}"#,
        )
        .unwrap();
        let cfg = load_definition(path.to_str().unwrap()).expect("json parses");
        std::fs::remove_dir_all(&dir).unwrap();
        assert!(
            matches!(&cfg.new, dmux_engine::run::Input::Live(p) if p.ca_file.as_deref() == Some("/ca.pem"))
        );
    }

    #[test]
    fn definition_refuses_a_bad_ca_file_setting() {
        let empty = r#"
old = { host = "127.0.0.1", port = 5433, db = "d", user = "u", ca_file = "" }
new = { host = "127.0.0.1", port = 5434, db = "d", user = "u" }
"#;
        let err = definition("caempty", empty).unwrap_err();
        assert_eq!(err, "ca_file may not be empty");
        let plain = r#"
old = { host = "127.0.0.1", port = 5433, db = "d", user = "u", tls = "disable", reason = "lab", ca_file = "/ca.pem" }
new = { host = "127.0.0.1", port = 5434, db = "d", user = "u" }
"#;
        let err = definition("caplain", plain).unwrap_err();
        assert!(
            err.starts_with("ca_file cannot be used with tls = \"disable\""),
            "{err}"
        );
        // A misspelt key is still rejected (deny_unknown_fields).
        let typo = r#"
old = { host = "127.0.0.1", port = 5433, db = "d", user = "u", cafile = "/ca.pem" }
new = { host = "127.0.0.1", port = 5434, db = "d", user = "u" }
"#;
        assert!(definition("catypo", typo).unwrap_err().contains("cafile"));
    }

    #[test]
    fn definition_rejects_plaintext_password_with_a_clear_error() {
        // Build the password at runtime: CodeQL rust/hard-coded-cryptographic-value
        // flags a literal secret, and the value is irrelevant to the rejection.
        let password = format!("not-a-secret-{:x}", std::process::id());
        let body = format!(
            r#"old = {{ host = "127.0.0.1", port = 5433, db = "d", user = "u", password = "{password}", tls = "disable", reason = "lab" }}
new = {{ host = "127.0.0.1", port = 5434, db = "d", user = "u", tls = "disable", reason = "lab" }}
"#
        );
        let err = definition("pw", &body).unwrap_err();
        assert!(
            err.contains("may not contain a password"),
            "expected a clear password rejection, got: {err}"
        );
        assert!(
            err.contains("password_env"),
            "the error should point at password_env, got: {err}"
        );
    }

    #[test]
    fn diff_refuses_an_inventory_over_the_size_limit() {
        let dir = std::env::temp_dir().join(format!("dmux-diff-limit-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("big.json");
        // A sparse file: the logical length exceeds the limit without writing
        // 64 MiB of real data.
        let f = std::fs::File::create(&path).unwrap();
        f.set_len(MAX_INVENTORY_BYTES + 1).unwrap();
        drop(f);

        let err = read_inventory_json(&path).unwrap_err();
        assert!(
            err.contains("over the") && err.contains("limit"),
            "expected an over-limit error, got: {err}"
        );

        // A small file passes the size gate and fails on parsing instead, which
        // proves the refusal above was the size check, not a parse error.
        let small = dir.join("small.json");
        std::fs::write(&small, "not json").unwrap();
        let small_err = read_inventory_json(&small).unwrap_err();
        assert!(
            !small_err.contains("over the"),
            "small file should not trigger the size limit: {small_err}"
        );

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn output_files_are_created_mode_0600() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("dmux-cli-mode-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("results.json");
        write_private_file(&path, b"{}").unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "results file must be 0600, got {mode:o}");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
