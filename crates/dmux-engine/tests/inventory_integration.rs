//! Integration tests for inventory collection against live PostgreSQL mocks.
//!
//! These tests read only from the catalog and are gated by the presence of
//! `DMUX_TEST_OLD_URL` and `DMUX_TEST_NEW_URL`.  When the variables are unset
//! the tests are skipped.

use dmux_engine::inventory::collect_inventory;
use tokio_postgres::NoTls;

fn env_url(name: &str) -> Option<String> {
    std::env::var(name).ok()
}

async fn connect(url: &str) -> tokio_postgres::Client {
    let (client, connection) = tokio_postgres::connect(url, NoTls).await.unwrap();
    tokio::spawn(async move {
        if let Err(e) = connection.await {
            eprintln!("connection error: {e}");
        }
    });
    client
}

#[tokio::test]
async fn old_database_inventory_collects_successfully() {
    let Some(url) = env_url("DMUX_TEST_OLD_URL") else {
        return;
    };
    let client = connect(&url).await;
    let inventory = collect_inventory(&client)
        .await
        .expect("inventory should collect");
    assert!(!inventory.server_version.is_empty());
    assert!(inventory.server_version_num >= 90400);
}

#[tokio::test]
async fn new_database_inventory_collects_successfully() {
    let Some(url) = env_url("DMUX_TEST_NEW_URL") else {
        return;
    };
    let client = connect(&url).await;
    let inventory = collect_inventory(&client)
        .await
        .expect("inventory should collect");
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

    let old_client = connect(&old_url).await;
    let new_client = connect(&new_url).await;

    let old_inv = collect_inventory(&old_client).await.unwrap();
    let new_inv = collect_inventory(&new_client).await.unwrap();

    let old_json = serde_json::to_string(&old_inv).unwrap();
    let new_json = serde_json::to_string(&new_inv).unwrap();

    let old_parsed: dmux_engine::inventory::DatabaseInventory =
        serde_json::from_str(&old_json).unwrap();
    let new_parsed: dmux_engine::inventory::DatabaseInventory =
        serde_json::from_str(&new_json).unwrap();

    assert_eq!(old_inv, old_parsed);
    assert_eq!(new_inv, new_parsed);

    // Basic sanity: both should have a public schema.
    assert!(old_inv.schemas.iter().any(|s| s.name == "public"));
    assert!(new_inv.schemas.iter().any(|s| s.name == "public"));
}
