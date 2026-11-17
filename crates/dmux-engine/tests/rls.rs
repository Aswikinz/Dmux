//! Row-level security is fail-closed: a table it would hide is a TOOL_FAULT,
//! never a PASS.
//!
//! The `rls_bank` fixture (a separate Nina task) holds one table,
//! `public.branch_notes`, with ENABLE + FORCE row-level security and a policy
//! that shows a non-owner only `branch_id = 1` — and the hidden half differs
//! between old and new, so a silent filter would compare equal and PASS.
//! Dmux sets `row_security = off` on 9.5+, so PostgreSQL raises SQLSTATE 42501
//! instead of filtering, and the run must end TOOL_FAULT with the row-security
//! reason.
//!
//! Gated on `DMUX_TEST_RLS_OLD_URL` / `DMUX_TEST_RLS_NEW_URL`; skipped (with a
//! printed note) when unset, because CI does not plant `rls_bank` yet.  The
//! reader login is a non-superuser, so `allow_superuser` stays `false` and the
//! password comes from the URL, never a literal.

use dmux_engine::db::{ConnProfile, TlsMode};
use dmux_engine::results::{CheckLevel, Verdict};
use dmux_engine::run::{run, Input, RunConfig};

fn env_url(name: &str) -> Option<String> {
    match std::env::var(name) {
        Ok(u) => Some(u),
        Err(_) => {
            eprintln!("skipped: {name} not set (no rls_bank fixture)");
            None
        }
    }
}

fn profile_from_url(url: &str) -> ConnProfile {
    let url = url.strip_prefix("postgres://").expect("postgres url");
    let (creds, rest) = url.split_once('@').expect("user:pass@host");
    let (user, password) = creds.split_once(':').expect("user:pass");
    let (host_port, dbname) = rest.split_once('/').unwrap_or((rest, ""));
    let (host, port) = host_port.split_once(':').expect("host:port");
    ConnProfile::new(
        host,
        port.parse().expect("port"),
        dbname,
        user,
        password,
        TlsMode::Disable("mock database has no TLS".to_string()),
    )
}

#[tokio::test]
async fn rls_table_is_a_tool_fault_never_a_pass() {
    let (Some(old_url), Some(new_url)) = (
        env_url("DMUX_TEST_RLS_OLD_URL"),
        env_url("DMUX_TEST_RLS_NEW_URL"),
    ) else {
        return;
    };

    let mut config = RunConfig::new(
        Input::Live(profile_from_url(&old_url)),
        Input::Live(profile_from_url(&new_url)),
    );
    // Only branch_notes is read at full values; anything else stays structural
    // so the test stays focused and cheap even if the fixture grows tables.
    config.default_level = CheckLevel::Structure;
    config
        .levels
        .insert("public.branch_notes".to_string(), CheckLevel::FullValues);

    let results = run(&config).await.expect("run should complete").results;

    let table = &results.tables["public.branch_notes"];
    assert_eq!(
        table.verdict,
        Verdict::ToolFault,
        "an RLS-protected table must never PASS: {:?}",
        table.reasons
    );
    assert!(
        table
            .reasons
            .iter()
            .any(|r| r.contains("row-level security")),
        "the fault must name row-level security: {:?}",
        table.reasons
    );
    assert_eq!(
        results.verdict,
        Verdict::ToolFault,
        "the run verdict is at least TOOL_FAULT"
    );
}
