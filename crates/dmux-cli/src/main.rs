//! `dmux` command-line runner — Phase 1 structure commands.
//!
//! Exit codes follow PLAN.md 4.8: 0 pass, 1 fail, 2 warnings.  The `inventory`
//! and `diff` commands are implemented here; other subcommands remain stubs.

use dmux_engine::inventory::collect_inventory;
use dmux_engine::schema_diff::{compare_inventories, DiffStatus};
use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;
use tokio_postgres::{Client, Config, NoTls};

/// PLAN.md 4.8 exit codes.
const EXIT_PASS: u8 = 0;
const EXIT_FAIL: u8 = 1;
const EXIT_WARNING: u8 = 2;

const USAGE: &str = "\
dmux — proves a PostgreSQL migration copied every row and value

Usage: dmux <command> [args]

Commands:
  inventory <conn args>     Collect a structural inventory and print it as JSON
  diff --old <file> --new <file>
                            Compare two inventory JSON files
  capture                   Save one database's fingerprints to a file
  compare                   Compare two fingerprint files, or a file and a live database
  run <definition file>     Run a saved verification definition

Exit codes: 0 pass, 1 fail, 2 warnings.";

/// A parsed command line.
#[derive(Debug, PartialEq, Eq)]
enum Command {
    Inventory(InventoryArgs),
    Diff(DiffArgs),
    Capture,
    Compare,
    Run { definition: String },
    Help,
    Version,
}

#[derive(Debug, PartialEq, Eq, Default)]
struct InventoryArgs {
    host: String,
    port: u16,
    db: String,
    user: String,
    password_env: Option<String>,
}

#[derive(Debug, PartialEq, Eq, Default)]
struct DiffArgs {
    old: PathBuf,
    new: PathBuf,
}

impl Command {
    /// The subcommand name, used in stub messages.
    fn name(&self) -> &'static str {
        match self {
            Command::Inventory(_) => "inventory",
            Command::Diff(_) => "diff",
            Command::Capture => "capture",
            Command::Compare => "compare",
            Command::Run { .. } => "run",
            Command::Help => "help",
            Command::Version => "version",
        }
    }
}

/// Parse arguments (without the program name). Pure, so it is unit-testable.
fn parse(args: &[String]) -> Result<Command, String> {
    let Some(first) = args.first() else {
        return Err("missing command".to_string());
    };
    let rest = &args[1..];
    match first.as_str() {
        "inventory" => Ok(Command::Inventory(parse_inventory_args(rest)?)),
        "diff" => Ok(Command::Diff(parse_diff_args(rest)?)),
        "capture" | "compare" => {
            if rest.is_empty() {
                Ok(if first == "capture" {
                    Command::Capture
                } else {
                    Command::Compare
                })
            } else {
                Err(format!("unexpected argument for {first}: {}", rest[0]))
            }
        }
        "run" => match rest {
            [definition] => Ok(Command::Run {
                definition: definition.clone(),
            }),
            [] => Err("run needs a definition file: dmux run <definition file>".to_string()),
            _ => Err(format!("unexpected argument for run: {}", rest[1])),
        },
        "help" | "--help" | "-h" => Ok(Command::Help),
        "version" | "--version" | "-V" => Ok(Command::Version),
        other => Err(format!("unknown command: {other}")),
    }
}

fn parse_inventory_args(args: &[String]) -> Result<InventoryArgs, String> {
    let mut parsed = InventoryArgs {
        host: "127.0.0.1".to_string(),
        port: 5432,
        db: "bank".to_string(),
        user: "dmux_dev".to_string(),
        password_env: None,
    };

    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--host" => parsed.host = next_arg(&mut iter, "--host")?,
            "--port" => {
                parsed.port = next_arg(&mut iter, "--port")?
                    .parse::<u16>()
                    .map_err(|e| format!("--port: {e}"))?;
            }
            "--db" => parsed.db = next_arg(&mut iter, "--db")?,
            "--user" => parsed.user = next_arg(&mut iter, "--user")?,
            "--password-env" => parsed.password_env = Some(next_arg(&mut iter, "--password-env")?),
            other => return Err(format!("unknown inventory argument: {other}")),
        }
    }
    Ok(parsed)
}

fn parse_diff_args(args: &[String]) -> Result<DiffArgs, String> {
    let mut old: Option<PathBuf> = None;
    let mut new: Option<PathBuf> = None;

    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--old" => old = Some(PathBuf::from(next_arg(&mut iter, "--old")?)),
            "--new" => new = Some(PathBuf::from(next_arg(&mut iter, "--new")?)),
            other => return Err(format!("unknown diff argument: {other}")),
        }
    }

    Ok(DiffArgs {
        old: old.ok_or("--old <file> is required".to_string())?,
        new: new.ok_or("--new <file> is required".to_string())?,
    })
}

fn next_arg(iter: &mut std::slice::Iter<String>, flag: &str) -> Result<String, String> {
    iter.next()
        .cloned()
        .ok_or_else(|| format!("{flag} requires an argument"))
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match parse(&args) {
        Ok(Command::Help) => {
            println!("{USAGE}");
            ExitCode::from(EXIT_PASS)
        }
        Ok(Command::Version) => {
            println!("dmux {}", env!("CARGO_PKG_VERSION"));
            ExitCode::from(EXIT_PASS)
        }
        Ok(Command::Inventory(inv_args)) => match run_inventory(inv_args) {
            Ok(()) => ExitCode::from(EXIT_PASS),
            Err(e) => {
                eprintln!("dmux inventory failed: {e}");
                ExitCode::from(EXIT_WARNING)
            }
        },
        Ok(Command::Diff(diff_args)) => match run_diff(diff_args) {
            Ok(status) => match status {
                DiffStatus::Equal => ExitCode::from(EXIT_PASS),
                DiffStatus::Warning => ExitCode::from(EXIT_WARNING),
                DiffStatus::Different => ExitCode::from(EXIT_FAIL),
            },
            Err(e) => {
                eprintln!("dmux diff failed: {e}");
                ExitCode::from(EXIT_WARNING)
            }
        },
        Ok(command) => {
            eprintln!("dmux {}: not implemented in Phase 1", command.name());
            ExitCode::from(EXIT_WARNING)
        }
        Err(message) => {
            eprintln!("{message}");
            eprintln!();
            eprintln!("{USAGE}");
            ExitCode::from(EXIT_WARNING)
        }
    }
}

fn run_inventory(args: InventoryArgs) -> Result<(), Box<dyn std::error::Error>> {
    let runtime = tokio::runtime::Runtime::new()?;
    runtime.block_on(async {
        let client = connect(&args).await?;
        let inventory = collect_inventory(&client).await?;
        let json = serde_json::to_string_pretty(&inventory)?;
        println!("{json}");
        Ok(())
    })
}

async fn connect(args: &InventoryArgs) -> Result<Client, Box<dyn std::error::Error>> {
    let password = match &args.password_env {
        Some(var) => std::env::var(var).ok(),
        None => None,
    };

    let mut config = Config::new();
    config
        .host(&args.host)
        .port(args.port)
        .user(&args.user)
        .dbname(&args.db)
        .application_name("dmux");
    if let Some(pass) = password {
        config.password(pass);
    }

    let (client, connection) = config.connect(NoTls).await?;
    tokio::spawn(async move {
        if let Err(e) = connection.await {
            eprintln!("connection error: {e}");
        }
    });
    Ok(client)
}

fn run_diff(args: DiffArgs) -> Result<DiffStatus, Box<dyn std::error::Error>> {
    let old_text = fs::read_to_string(&args.old)?;
    let new_text = fs::read_to_string(&args.new)?;
    let old = serde_json::from_str(&old_text)?;
    let new = serde_json::from_str(&new_text)?;
    let diff = compare_inventories(&old, &new);
    let json = serde_json::to_string_pretty(&diff)?;
    println!("{json}");
    Ok(diff.status)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn parses_inventory_defaults() {
        let cmd = parse(&args(&["inventory"])).unwrap();
        match cmd {
            Command::Inventory(a) => {
                assert_eq!(a.host, "127.0.0.1");
                assert_eq!(a.port, 5432);
                assert_eq!(a.db, "bank");
                assert_eq!(a.user, "dmux_dev");
                assert!(a.password_env.is_none());
            }
            _ => panic!("expected inventory"),
        }
    }

    #[test]
    fn parses_inventory_overrides() {
        let cmd = parse(&args(&[
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
        ]))
        .unwrap();
        match cmd {
            Command::Inventory(a) => {
                assert_eq!(a.host, "db.example.com");
                assert_eq!(a.port, 5433);
                assert_eq!(a.db, "demo_bank");
                assert_eq!(a.user, "reader");
                assert_eq!(a.password_env.as_deref(), Some("DMUX_PASSWORD"));
            }
            _ => panic!("expected inventory"),
        }
    }

    #[test]
    fn parses_diff_args() {
        let cmd = parse(&args(&["diff", "--old", "a.json", "--new", "b.json"])).unwrap();
        match cmd {
            Command::Diff(a) => {
                assert_eq!(a.old, PathBuf::from("a.json"));
                assert_eq!(a.new, PathBuf::from("b.json"));
            }
            _ => panic!("expected diff"),
        }
    }

    #[test]
    fn diff_requires_both_files() {
        assert!(parse(&args(&["diff", "--old", "a.json"])).is_err());
        assert!(parse(&args(&["diff", "--new", "b.json"])).is_err());
    }

    #[test]
    fn unknown_command_is_error() {
        assert!(parse(&args(&["frobnicate"])).is_err());
    }
}
