//! Integration tests for the Python hook runner (PLAN.md 4.9).
//!
//! These tests exercise the real interpreter and the real process-group kill
//! path.  They skip with a printed notice only when no Python interpreter is
//! available, and fail (rather than skip) when `CI=true`.

use dmux_hooks::{run_hook, HookSpec, Severity};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

/// Whether Python is available on this host; the tests need it to actually run.
fn python_available() -> bool {
    for bin in ["python3", "python"] {
        if let Ok(out) = Command::new(bin).arg("--version").output() {
            if out.status.success() {
                return true;
            }
        }
    }
    false
}

/// A short-but-real results payload, secret-free.
fn sample_results() -> serde_json::Value {
    serde_json::json!({
        "results_version": 1,
        "run_id": "run-test",
        "tool_version": "0.0.1",
        "mapping_hash": "abc",
        "verdict": "PASS",
        "coverage_line": "1 tables fully checked, 0 counts only, 0 ignored",
        "tables": {
            "public.accounts": {
                "table": "public.accounts",
                "level": "full_values",
                "verdict": "PASS",
                "old_rows": 100,
                "new_rows": 100,
                "reasons": []
            }
        }
    })
}

fn require_python() {
    if python_available() {
        return;
    }
    if std::env::var("CI").map(|v| v == "true").unwrap_or(false) {
        panic!("Python interpreter is required but not available (CI=true)");
    }
    eprintln!("notice: skipping dmux-hooks tests — no Python interpreter found");
}

#[test]
fn valid_script_returns_findings() {
    if !python_available() {
        require_python();
        return;
    }
    let script = fixtures().join("valid_hook.py");
    let spec = HookSpec::new(&script);
    let result = run_hook(&spec, &sample_results()).expect("hook should run");

    assert_eq!(result.findings.len(), 2);
    assert_eq!(result.findings[0].severity, Severity::Info);
    assert_eq!(result.findings[1].severity, Severity::Warning);
    assert!(result.warnings.is_empty());

    // The script hash is recorded (PLAN 4.9).
    assert!(!result.script_hash.is_empty());
    assert_eq!(result.script_hash.len(), 64);
    // And it matches the file's actual SHA-256.
    let expected = dmux_hooks::hash_script(&script).unwrap();
    assert_eq!(result.script_hash, expected);
}

#[test]
fn hanging_script_is_killed_and_reported() {
    if !python_available() {
        require_python();
        return;
    }
    let script = fixtures().join("hang.py");
    let mut spec = HookSpec::new(&script);
    spec.timeout = Duration::from_millis(500);

    let start = std::time::Instant::now();
    let result = run_hook(&spec, &sample_results()).expect("run_hook returns a result");
    let elapsed = start.elapsed();

    // The kill fires at ~500 ms; allow generous overhead for interpreter
    // startup but far below the 999 s sleep.
    assert!(
        elapsed < Duration::from_secs(10),
        "hang was not killed in time"
    );
    assert!(result.findings.is_empty());
    assert!(
        result.warnings.iter().any(|w| w.contains("timed out")),
        "expected a timeout warning, got {:?}",
        result.warnings
    );
}

#[test]
fn forked_child_is_killed_too() {
    if !python_available() {
        require_python();
        return;
    }
    #[cfg(not(unix))]
    {
        // Forking is a Unix-only test; on Windows this fixture cannot fork.
        return;
    }
    #[cfg(unix)]
    {
        let script = fixtures().join("fork_child.py");
        let mut spec = HookSpec::new(&script);
        spec.timeout = Duration::from_millis(1000);

        // The parent exits immediately, orphaning the child.  The runner must
        // not wait until the child's 999 s sleep finishes: the process group is
        // killed when the deadline (or EOF) hits.
        let start = std::time::Instant::now();
        let result = run_hook(&spec, &sample_results()).expect("run_hook returns");
        assert!(
            start.elapsed() < Duration::from_secs(10),
            "orphaned child was not killed promptly"
        );
        // The parent produced no findings; a clean empty exit or a timeout are
        // both acceptable, but the run must have terminated promptly.
        let _ = result;
    }
}

#[test]
fn garbage_output_is_rejected() {
    if !python_available() {
        require_python();
        return;
    }
    let script = fixtures().join("garbage.py");
    let spec = HookSpec::new(&script);
    let result = run_hook(&spec, &sample_results()).expect("run_hook returns");

    assert!(result.findings.is_empty());
    assert!(
        !result.warnings.is_empty(),
        "garbage output must produce a warning"
    );
}

#[test]
fn oversized_output_is_rejected() {
    if !python_available() {
        require_python();
        return;
    }
    let script = fixtures().join("oversized.py");
    let spec = HookSpec::new(&script);
    let result = run_hook(&spec, &sample_results()).expect("run_hook returns");

    assert!(result.findings.is_empty());
    assert!(
        !result.warnings.is_empty(),
        "oversized output must be rejected with a warning"
    );
}

#[test]
fn secrets_are_stripped_from_hook_environment() {
    if !python_available() {
        require_python();
        return;
    }
    // Seed the runner's environment with a sensitive variable; the child must
    // not see it.
    let script = fixtures().join("env_probe.py");
    // We can't cleanly inject PGPASSWORD into the *parent* here, but we can set
    // it for the test process's own environment just before running.
    std::env::set_var("PGPASSWORD", "super-secret");
    std::env::set_var("DMUX_TEST_SECRET", "also-secret");

    let spec = HookSpec::new(&script);
    let result = run_hook(&spec, &sample_results()).expect("run_hook returns");

    // The probe writes "LEAK:<vars>" to stderr; a clean run has an empty list
    // so no "LEAK:" line should appear anywhere in warnings.
    for w in &result.warnings {
        assert!(
            !w.contains("LEAK:"),
            "a sensitive env var leaked to the hook: {w}"
        );
    }
    assert!(
        result.findings.is_empty(),
        "probe should return no findings"
    );
}

#[test]
fn fail_finding_is_returned_with_fail_severity() {
    if !python_available() {
        require_python();
        return;
    }
    let script = fixtures().join("fail_hook.py");
    let spec = HookSpec::new(&script);
    let result = run_hook(&spec, &sample_results()).expect("run_hook returns");

    assert_eq!(result.findings.len(), 1);
    assert_eq!(result.findings[0].severity, Severity::Fail);
    assert_eq!(
        result.findings[0].table.as_deref(),
        Some("public.transactions")
    );
    assert!(result.warnings.is_empty());
}

#[test]
fn failed_setup_is_an_error_not_a_finding() {
    // A missing script is a setup error (Err), not a quiet warning.
    let spec = HookSpec::new("/nonexistent/hook.py");
    assert!(run_hook(&spec, &sample_results()).is_err());
}

#[test]
fn severity_effect_rules() {
    use dmux_hooks::VerdictEffect;
    assert_eq!(
        VerdictEffect::of(Severity::Fail),
        VerdictEffect::ForceNonPass
    );
    assert_eq!(VerdictEffect::of(Severity::Info), VerdictEffect::None);
    assert_eq!(VerdictEffect::of(Severity::Warning), VerdictEffect::None);
}
