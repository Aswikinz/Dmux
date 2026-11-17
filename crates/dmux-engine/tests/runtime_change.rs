//! Runtime case r01 (PLAN 8.2 row 13): a row updated while the check runs must
//! end as CHANGED_DURING_CHECK, never PASS.
//!
//! This uses the `concurrent_update.sh` harness against `demo_bank` on the
//! 9.4 -> 18 matrix pair (ports 5435/5437), and is gated behind
//! `DMUX_TEST_RUNTIME=1` only (not `CI`), so it never disturbs the shared
//! 5433/5434 readers.  It belongs in the nightly workflow.  `concurrent_update.sh` WRITES to demo_bank.notes on the
//! new side on purpose; it is harness code guarded to local mocks.

use std::process::Command;

use dmux_engine::db::{ConnProfile, TlsMode};
use dmux_engine::mapping::Mapping;
use dmux_engine::results::{CheckLevel, RunResults, Verdict};
use dmux_engine::run::{run, Input, RunConfig};

fn enabled() -> bool {
    matches!(
        std::env::var("DMUX_TEST_RUNTIME").as_deref(),
        Ok("1") | Ok("true")
    )
}

/// The mock database password, taken from the environment's `DMUX_TEST_*` URL
/// so no credential literal sits in the source (CodeQL
/// rust/hard-coded-cryptographic-value).  The password is the same on every
/// throwaway mock (old/new/9.4/18); we read it from the demo URL when present
/// and otherwise assemble a non-literal value at runtime.
fn mock_password() -> String {
    for var in [
        "DMUX_TEST_DEMO_OLD_URL",
        "DMUX_TEST_OLD_URL",
        "DMUX_TEST_NEW_URL",
    ] {
        if let Ok(url) = std::env::var(var) {
            if let Some(creds) = url
                .strip_prefix("postgres://")
                .and_then(|u| u.split('@').next())
            {
                if let Some((_user, pass)) = creds.rsplit_once(':') {
                    return pass.to_string();
                }
            }
        }
    }
    // No env URL (e.g. a local manual run): a runtime-only value with no
    // compile-time-constant content, so no credential literal exists at all.
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("{:032x}", (std::process::id() as u128) << 96 ^ nanos)
}

fn profile(port: u16) -> ConnProfile {
    ConnProfile::new(
        "127.0.0.1",
        port,
        "demo_bank",
        "dmux_dev",
        mock_password(),
        TlsMode::Disable("mock database has no TLS".to_string()),
    )
}

async fn run_engine(old_port: u16, new_port: u16) -> RunResults {
    let config = RunConfig {
        old: Input::Live(profile(old_port)),
        new: Input::Live(profile(new_port)),
        mapping: Mapping::new(),
        default_level: CheckLevel::FullValues,
        allow_superuser: true,
        ..RunConfig::default()
    };
    run(&config).await.expect("run should succeed").results
}

#[tokio::test]
async fn concurrent_update_is_changed_during_check_never_pass() {
    if !enabled() {
        eprintln!("skipped: set DMUX_TEST_RUNTIME=1 to run the concurrent-update case");
        return;
    }

    // Start the writer against the new side (18) before running the engine.  The
    // blocking spawn and the later reap both run off the async worker thread
    // (`spawn_blocking`) so the test never blocks the async context.
    let script = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../testdata/demo_bank/concurrent_update.sh");
    let child = tokio::task::spawn_blocking(move || {
        Command::new(script)
            .args(["8", "5437"])
            .spawn()
            .expect("spawn concurrent_update.sh")
    })
    .await
    .expect("spawn task");

    // Give the writer a moment to begin updating, then run the engine.
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    let results = run_engine(5435, 5437).await;
    let _ = tokio::task::spawn_blocking(move || {
        let mut child = child;
        let _ = child.kill();
        let _ = child.wait();
    })
    .await;

    // The notes table must be flagged changed (or the run must not PASS).
    let notes = results.tables.get("demo_bank.notes");
    let notes_changed = matches!(notes.map(|t| t.verdict), Some(Verdict::ChangedDuringCheck));
    assert!(
        notes_changed || results.verdict != Verdict::Pass,
        "a concurrent update must never leave a PASS, but notes={:?} overall={:?}",
        notes.map(|t| t.verdict),
        results.verdict
    );
    assert!(
        !results.changed_during_check.is_empty(),
        "changed_during_check must list the moving table"
    );
    assert_ne!(results.verdict, Verdict::Pass);
}
