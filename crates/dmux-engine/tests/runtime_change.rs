//! Runtime case r01 (PLAN 8.2 row 13): a row updated while the check runs must
//! end as CHANGED_DURING_CHECK, never PASS.
//!
//! This uses the `concurrent_update.sh` harness against `demo_bank` on the
//! 9.4 -> 18 matrix pair (ports 5435/5437) when present, and is gated behind
//! `DMUX_TEST_RUNTIME=1` (or `CI=true`), so it never disturbs the shared
//! 5433/5434 readers.  `concurrent_update.sh` WRITES to demo_bank.notes on the
//! new side on purpose; it is harness code guarded to local mocks.

use std::collections::BTreeMap;
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

fn profile(port: u16) -> ConnProfile {
    ConnProfile::new(
        "127.0.0.1",
        port,
        "demo_bank",
        "dmux_dev",
        "dmux_dev_only",
        TlsMode::Disable("mock database has no TLS".to_string()),
    )
}

async fn run_engine(old_port: u16, new_port: u16) -> RunResults {
    let config = RunConfig {
        old: Input::Live(profile(old_port)),
        new: Input::Live(profile(new_port)),
        mapping: Mapping::new(),
        default_level: CheckLevel::FullValues,
        levels: BTreeMap::new(),
        control_total_columns: Default::default(),
        old_parallelism: 2,
        new_parallelism: 4,
        allow_superuser: true,
        export_keys: false,
        resume_store: None,
    };
    run(&config).await.expect("run should succeed").results
}

#[tokio::test]
async fn concurrent_update_is_changed_during_check_never_pass() {
    if !enabled() {
        eprintln!("skipped: set DMUX_TEST_RUNTIME=1 to run the concurrent-update case");
        return;
    }

    // Start the writer against the new side (18) before running the engine.
    let script = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../testdata/demo_bank/concurrent_update.sh");
    let mut child = Command::new(script)
        .args(["8", "5437"])
        .spawn()
        .expect("spawn concurrent_update.sh");

    // Give the writer a moment to begin updating, then run the engine.
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    let results = run_engine(5435, 5437).await;
    let _ = child.kill();
    let _ = child.wait();

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
