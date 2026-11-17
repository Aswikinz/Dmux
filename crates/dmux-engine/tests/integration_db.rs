//! Integration tests against live PostgreSQL mock databases.
//!
//! These tests are gated on the `DMUX_TEST_OLD_URL` and `DMUX_TEST_NEW_URL`
//! environment variables.  When unset outside CI the tests skip; when unset in
//! CI (CI=true) they fail, matching the core `db_tests.rs` policy.

use dmux_engine::db::{probe_server, ConnProfile, DbError, ReadOnlySession, ServerInfo, TlsMode};

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

/// The mock login is a documented superuser, so probes/inventory allow it.
fn require_url(name: &str) -> Option<String> {
    match std::env::var(name) {
        Ok(u) => Some(u),
        Err(_) => {
            assert!(
                std::env::var("CI").as_deref() != Ok("true"),
                "CI=true but {name} is not set: the live cross-check must run in CI"
            );
            eprintln!("skipped: {name} not set");
            None
        }
    }
}

fn old_url() -> Option<String> {
    require_url("DMUX_TEST_OLD_URL")
}

#[allow(dead_code)]
fn new_url() -> Option<String> {
    require_url("DMUX_TEST_NEW_URL")
}

#[tokio::test]
async fn probe_reports_server_info() {
    let Some(url) = old_url() else {
        return;
    };
    let profile = test_profile_from_url(&url);
    let info: ServerInfo = probe_server(&profile, true)
        .await
        .expect("probe should succeed");
    assert!(info.version_num > 0);
    assert!(!info.version_string.is_empty());
    assert!(!info.encoding.is_empty());
    assert!(!info.collation.is_empty());
}

#[tokio::test]
async fn readonly_session_rejects_writes() {
    let Some(url) = old_url() else {
        return;
    };
    let profile = test_profile_from_url(&url);
    let mut session = ReadOnlySession::open(&profile, true)
        .await
        .expect("connect should succeed");

    let result: Result<(), DbError> = session
        .with_read_only("5s", |_txn| Box::pin(async move { Ok(()) }))
        .await;
    assert!(result.is_ok(), "a read-only SELECT should succeed");

    let write_result: Result<(), DbError> = session
        .with_read_only("5s", |txn| {
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
async fn superuser_login_is_refused_without_override() {
    let Some(url) = old_url() else {
        return;
    };
    let profile = test_profile_from_url(&url);
    let result = ReadOnlySession::open(&profile, false).await;
    assert!(
        matches!(result, Err(DbError::SuperuserRefused)),
        "the mock login is a superuser and must be refused without --allow-superuser"
    );
}

#[tokio::test]
async fn probe_mock_login_is_superuser() {
    let Some(url) = old_url() else {
        return;
    };
    let profile = test_profile_from_url(&url);
    let info = probe_server(&profile, true)
        .await
        .expect("probe should succeed");
    assert!(
        info.is_superuser,
        "the mock login is documented as a superuser"
    );
}

#[tokio::test]
async fn read_only_safety_net_blocks_writes_outside_the_wrapper() {
    let Some(url) = old_url() else {
        return;
    };
    let profile = test_profile_from_url(&url);
    // Mirror the engine's config (including the read-only safety-net option) and
    // attempt a write WITHOUT going through the read-only wrapper: the
    // session-level default_transaction_read_only must still reject it.
    let mut config = tokio_postgres::Config::new();
    config
        .host(&profile.host)
        .port(profile.port)
        .dbname(&profile.dbname)
        .user(&profile.user)
        .password(&profile.password)
        .application_name("dmux")
        .ssl_mode(tokio_postgres::config::SslMode::Disable)
        .options("-c default_transaction_read_only=on");
    let (client, conn) = config.connect(tokio_postgres::NoTls).await.unwrap();
    tokio::spawn(async move {
        let _ = conn.await;
    });

    let write = client
        .execute("CREATE TABLE dmux_ro_safety_net(id int)", &[])
        .await;
    assert!(
        write.is_err(),
        "a write outside the read-only wrapper must still fail"
    );
}
