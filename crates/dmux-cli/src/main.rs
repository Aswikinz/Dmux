//! `dmux` command-line runner — Phase 0 skeleton.
//!
//! Exit codes follow PLAN.md 4.8: 0 pass, 1 fail, 2 warnings. Every subcommand
//! is a stub in Phase 0: it prints a clear message and exits 2, so a scripted
//! run can never mistake the skeleton for a passing verification.

use std::process::ExitCode;

/// PLAN.md 4.8 exit codes.
const EXIT_PASS: u8 = 0;
// Phase 0 stubs never report a data failure; the real runner uses this from
// Phase 2 onward. Kept now so the contract is stated in one place.
#[allow(dead_code)]
const EXIT_FAIL: u8 = 1;
const EXIT_WARNING: u8 = 2;

const USAGE: &str = "\
dmux — proves a PostgreSQL migration copied every row and value

Usage: dmux <command> [args]

Commands:
  capture                 Save one database's fingerprints to a file
  compare                 Compare two fingerprint files, or a file and a live database
  run <definition file>   Run a saved verification definition

Phase 0 skeleton: every command is a stub and exits 2 (warnings).
Exit codes: 0 pass, 1 fail, 2 warnings.";

/// A parsed command line.
#[derive(Debug, PartialEq, Eq)]
enum Command {
    Capture,
    Compare,
    Run { definition: String },
    Help,
    Version,
}

impl Command {
    /// The subcommand name, used in stub messages.
    fn name(&self) -> &'static str {
        match self {
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
        Ok(command) => {
            eprintln!("dmux {}: not implemented in Phase 0", command.name());
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

#[cfg(test)]
mod tests {
    use super::{parse, Command, EXIT_FAIL, EXIT_PASS, EXIT_WARNING, USAGE};

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn parses_capture_and_compare() {
        assert_eq!(parse(&args(&["capture"])), Ok(Command::Capture));
        assert_eq!(parse(&args(&["compare"])), Ok(Command::Compare));
    }

    #[test]
    fn parses_run_with_a_definition_file() {
        assert_eq!(
            parse(&args(&["run", "run.json"])),
            Ok(Command::Run {
                definition: "run.json".to_string()
            })
        );
    }

    #[test]
    fn run_without_a_file_is_an_error() {
        assert!(parse(&args(&["run"])).is_err());
    }

    #[test]
    fn extra_arguments_are_errors() {
        assert!(parse(&args(&["capture", "extra"])).is_err());
        assert!(parse(&args(&["run", "a.json", "b.json"])).is_err());
    }

    #[test]
    fn unknown_and_missing_commands_are_errors() {
        assert!(parse(&args(&["frobnicate"])).is_err());
        assert!(parse(&args(&[])).is_err());
    }

    #[test]
    fn help_and_version_flags_parse() {
        for flag in ["help", "--help", "-h"] {
            assert_eq!(parse(&args(&[flag])), Ok(Command::Help));
        }
        for flag in ["version", "--version", "-V"] {
            assert_eq!(parse(&args(&[flag])), Ok(Command::Version));
        }
    }

    #[test]
    fn exit_codes_match_the_plan() {
        // PLAN.md 4.8: 0 pass, 1 fail, 2 warnings. Stubs exit 2, never 0.
        assert_eq!(EXIT_PASS, 0);
        assert_eq!(EXIT_FAIL, 1);
        assert_eq!(EXIT_WARNING, 2);
    }

    #[test]
    fn usage_lists_every_subcommand() {
        for name in ["capture", "compare", "run"] {
            assert!(USAGE.contains(name), "usage must mention {name}");
        }
    }

    #[test]
    fn command_names_are_the_subcommand_names() {
        assert_eq!(Command::Capture.name(), "capture");
        assert_eq!(Command::Compare.name(), "compare");
        assert_eq!(
            Command::Run {
                definition: "x".to_string()
            }
            .name(),
            "run"
        );
    }
}
