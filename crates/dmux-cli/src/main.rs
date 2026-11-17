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
use dmux_engine::results::CheckLevel;
use dmux_engine::run::{capture, compare_files, run, Input, RunConfig};
use dmux_engine::schema_diff::{compare_inventories, DiffStatus};

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

    emit_results(args.out.as_deref(), &outcome.results);
    ExitCode::from(outcome.exit_code)
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
}

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
        let tls_mode = parse_tls_mode(&c.tls, &c.reason)?;
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
        export_keys: def.export_keys,
        resume_store: None,
    })
}

#[cfg(test)]
mod tests {
    use super::{
        allow_superuser_requires_reason, load_definition, parse_level, parse_tls_mode, Cli,
        Command, DiffArgs, EXIT_FAIL, EXIT_PASS, EXIT_WARN,
    };
    use clap::Parser;
    use dmux_engine::db::TlsMode;
    use dmux_engine::results::CheckLevel;

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
}
