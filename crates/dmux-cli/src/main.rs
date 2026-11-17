//! `dmux` command-line runner.
//!
//! Exit codes follow PLAN.md 4.8: 0 pass, 1 fail, 2 warnings.  `probe`,
//! `inventory` and `diff` are implemented; `capture`, `compare` and `run` are
//! Phase 0 stubs that exit 2.

use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Args, Parser, Subcommand};
use dmux_engine::db::{probe_server, ConnProfile, DbError, ReadOnlySession, TlsMode};
use dmux_engine::inventory::collect_inventory;
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
    /// Save one database's fingerprints to a file (not implemented).
    Capture,
    /// Compare two fingerprint files, or a file and a live database (not implemented).
    Compare,
    /// Run a saved verification definition (not implemented).
    Run {
        /// The verification definition file.
        definition: String,
    },
}

/// Connection arguments shared by the commands that touch a database.
#[derive(Args, Debug)]
struct ProbeArgs {
    #[arg(long)]
    host: String,
    #[arg(long)]
    port: u16,
    #[arg(long)]
    db: String,
    #[arg(long)]
    user: String,
    #[arg(long, value_name = "VAR")]
    password_env: String,
    #[arg(long, value_name = "MODE")]
    tls: String,
    #[arg(long, value_name = "TEXT")]
    reason: Option<String>,
    #[arg(long)]
    allow_superuser: bool,
}

#[derive(Args, Debug)]
struct InventoryArgs {
    #[arg(long, default_value = "127.0.0.1")]
    host: String,
    #[arg(long, default_value = "5432")]
    port: u16,
    #[arg(long, default_value = "bank")]
    db: String,
    #[arg(long, default_value = "dmux_dev")]
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

#[derive(Args, Debug)]
struct DiffArgs {
    #[arg(long)]
    old: PathBuf,
    #[arg(long)]
    new: PathBuf,
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

/// Validate that `--allow-superuser` carries a non-empty `--reason`.
fn allow_superuser_requires_reason(reason: &Option<String>) -> bool {
    reason.as_deref().is_some_and(|r| !r.trim().is_empty())
}

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();
    match cli.command {
        Command::Probe(args) => run_probe(args).await,
        Command::Inventory(args) => run_inventory(args).await,
        Command::Diff(args) => run_diff(args),
        Command::Capture | Command::Compare | Command::Run { .. } => {
            eprintln!("dmux: this command is not implemented in Phase 0");
            ExitCode::from(EXIT_WARN)
        }
    }
}

async fn run_probe(args: ProbeArgs) -> ExitCode {
    let tls_mode = match parse_tls_mode(&args.tls, &args.reason) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("configuration error: {e}");
            return ExitCode::from(EXIT_FAIL);
        }
    };

    let password = match std::env::var(&args.password_env) {
        Ok(p) => p,
        Err(_) => {
            eprintln!(
                "configuration error: password environment variable {} is not set",
                args.password_env
            );
            return ExitCode::from(EXIT_FAIL);
        }
    };

    if args.allow_superuser && !allow_superuser_requires_reason(&args.reason) {
        eprintln!("configuration error: --allow-superuser requires --reason <text>");
        return ExitCode::from(EXIT_FAIL);
    }

    let profile = ConnProfile::new(args.host, args.port, args.db, args.user, password, tls_mode);

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

async fn run_inventory(args: InventoryArgs) -> ExitCode {
    let tls_mode = match parse_tls_mode(&args.tls, &args.reason) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("configuration error: {e}");
            return ExitCode::from(EXIT_FAIL);
        }
    };

    let password = match args.password_env.as_deref().map(std::env::var) {
        Some(Ok(p)) => p,
        Some(Err(_)) => {
            eprintln!(
                "configuration error: password environment variable {} is not set",
                args.password_env.as_deref().unwrap_or_default()
            );
            return ExitCode::from(EXIT_FAIL);
        }
        None => String::new(),
    };

    if args.allow_superuser && !allow_superuser_requires_reason(&args.reason) {
        eprintln!("configuration error: --allow-superuser requires --reason <text>");
        return ExitCode::from(EXIT_FAIL);
    }

    let profile = ConnProfile::new(args.host, args.port, args.db, args.user, password, tls_mode);

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
        Ok(inv) => {
            match serde_json::to_string_pretty(&inv) {
                Ok(json) => println!("{json}"),
                Err(e) => {
                    eprintln!("dmux inventory failed: {e}");
                    return ExitCode::from(EXIT_FAIL);
                }
            }
            ExitCode::from(EXIT_PASS)
        }
        Err(e) => {
            eprintln!("dmux inventory failed: {e}");
            ExitCode::from(EXIT_FAIL)
        }
    }
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

#[cfg(test)]
mod tests {
    use super::{parse_tls_mode, Command, DiffArgs, EXIT_FAIL, EXIT_PASS, EXIT_WARN};
    use clap::Parser;
    use dmux_engine::db::TlsMode;

    #[test]
    fn exit_codes_match_the_plan() {
        assert_eq!(EXIT_PASS, 0);
        assert_eq!(EXIT_FAIL, 1);
        assert_eq!(EXIT_WARN, 2);
    }

    fn parse(args: &[&str]) -> Command {
        use super::Cli;
        Cli::try_parse_from(std::iter::once("dmux").chain(args.iter().copied()))
            .expect("args should parse")
            .command
    }

    #[test]
    fn parses_inventory_defaults() {
        match parse(&["inventory"]) {
            Command::Inventory(a) => {
                assert_eq!(a.host, "127.0.0.1");
                assert_eq!(a.port, 5432);
                assert_eq!(a.db, "bank");
                assert_eq!(a.user, "dmux_dev");
                assert!(a.password_env.is_none());
            }
            other => panic!("expected inventory, got {other:?}"),
        }
    }

    #[test]
    fn parses_inventory_overrides() {
        match parse(&[
            "inventory",
            "--host",
            "db.example.com",
            "--port",
            "5433",
            "--db",
            "demo_bank",
            "--user",
            "reader",
            "--password-env",
            "DMUX_PASSWORD",
        ]) {
            Command::Inventory(a) => {
                assert_eq!(a.host, "db.example.com");
                assert_eq!(a.port, 5433);
                assert_eq!(a.db, "demo_bank");
                assert_eq!(a.user, "reader");
                assert_eq!(a.password_env.as_deref(), Some("DMUX_PASSWORD"));
            }
            other => panic!("expected inventory, got {other:?}"),
        }
    }

    #[test]
    fn parses_diff_args() {
        match parse(&["diff", "--old", "a.json", "--new", "b.json"]) {
            Command::Diff(DiffArgs { old, new }) => {
                assert_eq!(old, std::path::PathBuf::from("a.json"));
                assert_eq!(new, std::path::PathBuf::from("b.json"));
            }
            other => panic!("expected diff, got {other:?}"),
        }
    }

    #[test]
    fn parses_probe_args() {
        match parse(&[
            "probe",
            "--host",
            "h",
            "--port",
            "5433",
            "--db",
            "bank",
            "--user",
            "u",
            "--password-env",
            "PW",
            "--tls",
            "verify",
        ]) {
            Command::Probe(a) => {
                assert_eq!(a.host, "h");
                assert_eq!(a.port, 5433);
            }
            other => panic!("expected probe, got {other:?}"),
        }
    }

    #[test]
    fn parses_stub_commands() {
        assert!(matches!(parse(&["capture"]), Command::Capture));
        assert!(matches!(parse(&["compare"]), Command::Compare));
        assert!(matches!(parse(&["run", "def.json"]), Command::Run { .. }));
    }

    #[test]
    fn tls_mode_verify() {
        assert_eq!(
            parse_tls_mode("verify", &None).unwrap(),
            TlsMode::VerifyFull
        );
    }

    #[test]
    fn tls_mode_fingerprint() {
        match parse_tls_mode("fingerprint:abcd", &None).unwrap() {
            TlsMode::TrustFingerprint(fp) => assert_eq!(fp, "abcd"),
            other => panic!("expected fingerprint, got {other:?}"),
        }
    }

    #[test]
    fn tls_mode_fingerprint_requires_hex() {
        assert!(parse_tls_mode("fingerprint:", &None).is_err());
    }

    #[test]
    fn tls_mode_disable_requires_reason() {
        assert!(parse_tls_mode("disable", &None).is_err());
        assert_eq!(
            parse_tls_mode("disable", &Some("test network".to_string())).unwrap(),
            TlsMode::Disable("test network".to_string())
        );
    }

    #[test]
    fn tls_mode_disable_rejects_empty_reason() {
        assert!(parse_tls_mode("disable", &Some("   ".to_string())).is_err());
    }

    #[test]
    fn tls_mode_unknown_is_error() {
        assert!(parse_tls_mode("bogus", &None).is_err());
    }
}
