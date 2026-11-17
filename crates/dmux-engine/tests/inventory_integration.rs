//! Integration tests for inventory collection against live PostgreSQL mocks.
//!
//! These tests read only from the catalog and are gated by the presence of
//! `DMUX_TEST_OLD_URL` and `DMUX_TEST_NEW_URL`.  When unset outside CI they are
//! skipped; when unset in CI (CI=true) they fail, matching the core policy.

use dmux_engine::db::{ConnProfile, DbError, ReadOnlySession, TlsMode};
use dmux_engine::inventory::{collect_inventory, DatabaseInventory, InventoryError};

fn env_url(name: &str) -> Option<String> {
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

fn profile_from_url(url: &str) -> ConnProfile {
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

/// Collect an inventory over the read-only wrapper (the same path the CLI uses).
async fn collect(url: &str) -> DatabaseInventory {
    let profile = profile_from_url(url);
    let mut session = ReadOnlySession::open(&profile, true)
        .await
        .expect("connect should succeed");
    session
        .with_read_only("30s", |txn| {
            Box::pin(async move { collect_inventory(txn).await.map_err(inventory_to_db_error) })
        })
        .await
        .expect("inventory should collect")
}

fn inventory_to_db_error(e: InventoryError) -> DbError {
    DbError::Postgres {
        message: e.to_string(),
    }
}

#[tokio::test]
async fn old_database_inventory_collects_successfully() {
    let Some(url) = env_url("DMUX_TEST_OLD_URL") else {
        return;
    };
    let inventory = collect(&url).await;
    assert!(!inventory.server_version.is_empty());
    assert!(inventory.server_version_num >= 90400);
}

#[tokio::test]
async fn new_database_inventory_collects_successfully() {
    let Some(url) = env_url("DMUX_TEST_NEW_URL") else {
        return;
    };
    let inventory = collect(&url).await;
    assert!(!inventory.server_version.is_empty());
    assert!(inventory.server_version_num >= 100000);
}

#[tokio::test]
async fn inventories_roundtrip_through_json() {
    let (Some(old_url), Some(new_url)) =
        (env_url("DMUX_TEST_OLD_URL"), env_url("DMUX_TEST_NEW_URL"))
    else {
        return;
    };

    let old_inv = collect(&old_url).await;
    let new_inv = collect(&new_url).await;

    let old_json = serde_json::to_string(&old_inv).unwrap();
    let new_json = serde_json::to_string(&new_inv).unwrap();

    let old_parsed: DatabaseInventory = serde_json::from_str(&old_json).unwrap();
    let new_parsed: DatabaseInventory = serde_json::from_str(&new_json).unwrap();

    assert_eq!(old_inv, old_parsed);
    assert_eq!(new_inv, new_parsed);

    // Basic sanity: both should have a public schema.
    assert!(old_inv.schemas.iter().any(|s| s.name == "public"));
    assert!(new_inv.schemas.iter().any(|s| s.name == "public"));
}
