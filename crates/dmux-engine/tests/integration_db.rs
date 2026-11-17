//! Integration tests against live PostgreSQL mock databases.
//!
//! These tests are gated on the `DMUX_TEST_OLD_URL` and `DMUX_TEST_NEW_URL`
//! environment variables.  When unset the tests skip with a message.

use dmux_engine::db::{
    connect, probe_server, with_readonly_txn, ConnProfile, DbError, ServerInfo, TlsMode,
};

fn test_profile_from_url(url: &str) -> ConnProfile {
    // Expects postgres://user:password@host:port/dbname
    let url = url
        .strip_prefix("postgres://")
        .expect("url must be postgres");
    let (creds, rest) = url
        .split_once('@')
        .expect("url must contain user:pass@host");
    let (user, password) = creds.split_once(':').expect("url must contain user:pass");
    let (host_port, dbname) = rest.split_once('/').unwrap_or((rest, ""));
    let (host, port_str) = host_port
        .split_once(':')
        .expect("url must contain host:port");
    let port = port_str.parse().expect("port must be numeric");
    ConnProfile::new(
        host,
        port,
        dbname,
        user,
        password,
        TlsMode::Disable("integration test mock database has no TLS".to_string()),
    )
}

fn old_url() -> Option<String> {
    std::env::var("DMUX_TEST_OLD_URL").ok()
}

#[allow(dead_code)]
fn new_url() -> Option<String> {
    std::env::var("DMUX_TEST_NEW_URL").ok()
}

#[tokio::test]
async fn probe_reports_server_info() {
    let Some(url) = old_url() else {
        eprintln!("skipping probe_reports_server_info: DMUX_TEST_OLD_URL not set");
        return;
    };
    let profile = test_profile_from_url(&url);
    let info: ServerInfo = probe_server(&profile).await.expect("probe should succeed");
    assert!(info.version_num > 0);
    assert!(!info.version_string.is_empty());
    assert!(!info.encoding.is_empty());
    assert!(!info.collation.is_empty());
}

#[tokio::test]
async fn readonly_txn_rejects_writes() {
    let Some(url) = old_url() else {
        eprintln!("skipping readonly_txn_rejects_writes: DMUX_TEST_OLD_URL not set");
        return;
    };
    let profile = test_profile_from_url(&url);
    let mut client = connect(&profile).await.expect("connect should succeed");

    let result: Result<(), DbError> =
        with_readonly_txn(&mut client, "5s", |_txn| Box::pin(async move { Ok(()) })).await;
    assert!(result.is_ok(), "a read-only SELECT should succeed");

    let write_result: Result<(), DbError> = with_readonly_txn(&mut client, "5s", |txn| {
        Box::pin(async move {
            txn.execute("CREATE TABLE dmux_readonly_violation_test(id int)", &[])
                .await
                .map_err(|e| DbError::Postgres {
                    message: e.to_string(),
                })?;
            Ok(())
        })
    })
    .await;
    assert!(
        write_result.is_err(),
        "a write inside a read-only transaction must fail"
    );
}

#[tokio::test]
async fn probe_mock_login_is_superuser() {
    let Some(url) = old_url() else {
        eprintln!("skipping probe_mock_login_is_superuser: DMUX_TEST_OLD_URL not set");
        return;
    };
    let profile = test_profile_from_url(&url);
    let info = probe_server(&profile).await.expect("probe should succeed");
    assert!(
        info.is_superuser,
        "the mock login is documented as a superuser"
    );
}
