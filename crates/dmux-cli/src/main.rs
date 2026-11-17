//! `dmux` command-line runner — Phase 0 skeleton.
//!
//! Exit codes follow PLAN.md 4.8: 0 pass, 1 fail, 2 warnings. Every subcommand
//! is a stub in Phase 0: it prints a clear message and exits 2, so a scripted
//! run can never mistake the skeleton for a passing verification.

use std::process::ExitCode;

use clap::{Parser, Subcommand};
use dmux_engine::db::{probe_server, ConnProfile, TlsMode};

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
    /// Save one database's fingerprints to a file (not implemented).
    Capture,
    /// Compare two fingerprint files, or a file and a live database (not implemented).
    Compare,
    /// Run a saved verification definition (not implemented).
    Run { definition: String },
    /// Test a database connection and report server information.
    Probe(ProbeArgs),
}

#[derive(Parser, Debug)]
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

impl ProbeArgs {
    fn tls_mode(&self) -> Result<TlsMode, String> {
        if self.tls == "verify" {
            return Ok(TlsMode::VerifyFull);
        }
        if self.tls == "disable" {
            let reason = self
                .reason
                .clone()
                .ok_or("--reason <text> is required for --tls disable")?;
            return Ok(TlsMode::Disable(reason));
        }
        let prefix = "fingerprint:";
        if let Some(hex) = self.tls.strip_prefix(prefix) {
            if hex.is_empty() {
                return Err("--tls fingerprint:<hex> is missing the hex value".to_string());
            }
            return Ok(TlsMode::TrustFingerprint(hex.to_string()));
        }
        Err(format!(
            "unknown --tls value: {} (expected verify, fingerprint:<hex>, or disable)",
            self.tls
        ))
    }
}

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();
    match cli.command {
        Command::Probe(args) => run_probe(args).await,
        Command::Capture | Command::Compare | Command::Run { .. } => {
            eprintln!("dmux: this command is not implemented in Phase 0");
            ExitCode::from(EXIT_WARN)
        }
    }
}

async fn run_probe(args: ProbeArgs) -> ExitCode {
    let tls_mode = match args.tls_mode() {
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

    if args.allow_superuser && args.reason.is_none() {
        eprintln!("configuration error: --allow-superuser requires --reason <text>");
        return ExitCode::from(EXIT_FAIL);
    }

    let profile = ConnProfile::new(args.host, args.port, args.db, args.user, password, tls_mode);

    let info = match probe_server(&profile).await {
        Ok(i) => i,
        Err(e) => {
            eprintln!("probe failed: {e}");
            return ExitCode::from(EXIT_FAIL);
        }
    };

    let json = serde_json::to_string(&info).unwrap_or_else(|_| "{}".to_string());
    println!("{json}");

    if info.is_superuser && !args.allow_superuser {
        eprintln!("error: superuser login refused; use --allow-superuser --reason <text>");
        return ExitCode::from(EXIT_FAIL);
    }

    let mut warnings = Vec::new();
    if info.is_superuser && args.allow_superuser {
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

#[cfg(test)]
mod tests {
    use super::{EXIT_FAIL, EXIT_PASS, EXIT_WARN};

    #[test]
    fn exit_codes_match_the_plan() {
        assert_eq!(EXIT_PASS, 0);
        assert_eq!(EXIT_FAIL, 1);
        assert_eq!(EXIT_WARN, 2);
    }
}
