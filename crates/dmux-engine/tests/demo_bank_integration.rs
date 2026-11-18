//! Integration test against the planted `demo_bank` databases.
//!
//! Loaded and planted with `testdata/demo_bank/build.sh`; the answer key is
//! `testdata/expected_diffs.json`.  This test proves the inventory + schema
//! diff report the structural entries the answer key requires (m07-m15, q04,
//! q05, q06).  Row-level differences (m01-m06) are the comparison core's job,
//! not this test's.
//!
//! Gated on `DMUX_TEST_DEMO_OLD_URL` / `DMUX_TEST_DEMO_NEW_URL`; fail (not skip)
//! when CI=true and they are unset, matching the core policy.

use dmux_engine::db::{ConnProfile, DbError, ReadOnlySession, TlsMode};
use dmux_engine::inventory::{collect_inventory, DatabaseInventory, InventoryError};
use dmux_engine::schema_diff::{compare_inventories, DiffStatus, SchemaDiffEntry};

fn env_url(name: &str) -> Option<String> {
    match std::env::var(name) {
        Ok(u) => Some(u),
        Err(_) => {
            assert!(
                std::env::var("CI").as_deref() != Ok("true"),
                "CI=true but {name} is not set: the demo_bank cross-check must run in CI"
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

async fn collect(url: &str) -> DatabaseInventory {
    let profile = profile_from_url(url);
    let mut session = ReadOnlySession::open(&profile, true)
        .await
        .expect("connect should succeed");
    session
        .with_read_only("30s", |txn| {
            Box::pin(async move {
                collect_inventory(txn)
                    .await
                    .map_err(|e: InventoryError| DbError::Postgres {
                        message: e.to_string(),
                    })
            })
        })
        .await
        .expect("inventory should collect")
}

fn flatten(entries: &[SchemaDiffEntry], out: &mut Vec<(Vec<String>, DiffStatus, String)>) {
    for e in entries {
        out.push((e.path.clone(), e.status, e.message.clone()));
        flatten(&e.children, out);
    }
}

#[tokio::test]
async fn demo_bank_reports_the_answer_key_structure() {
    let (Some(old_url), Some(new_url)) = (
        env_url("DMUX_TEST_DEMO_OLD_URL"),
        env_url("DMUX_TEST_DEMO_NEW_URL"),
    ) else {
        return;
    };

    let old_inv = collect(&old_url).await;
    let new_inv = collect(&new_url).await;
    let diff = compare_inventories(&old_inv, &new_inv);

    let mut flat: Vec<(Vec<String>, DiffStatus, String)> = Vec::new();
    flatten(&diff.entries, &mut flat);

    let status_of = |needle: &str| -> Option<DiffStatus> {
        flat.iter()
            .find(|(_, _, m)| m.contains(needle))
            .map(|(_, s, _)| *s)
    };
    let has = |needle: &str| -> bool { flat.iter().any(|(_, _, m)| m.contains(needle)) };

    // m07: dropped column user_roles.granted_by (red).
    assert_eq!(
        status_of("column missing in new: granted_by"),
        Some(DiffStatus::Different),
        "m07: dropped column must be reported"
    );

    // m08: added column loans.settlement_ref (green/amber).
    assert_eq!(
        status_of("column added in new: settlement_ref"),
        Some(DiffStatus::Warning),
        "m08: added column must be reported"
    );

    // m09: table renamed limits -> product_limits (missing + added).
    assert!(has("table missing in new: limits"), "m09: limits missing");
    assert!(
        has("table added in new: product_limits"),
        "m09: product_limits added"
    );

    // m10: column renamed fees.fee_name -> fee_label.
    assert!(
        has("column missing in new: fee_name"),
        "m10: fee_name missing"
    );
    assert!(
        has("column added in new: fee_label"),
        "m10: fee_label added"
    );

    // m11: branches.city varchar(60) -> varchar(120), amber.
    assert_eq!(
        status_of("character varying(60) -> character varying(120)"),
        Some(DiffStatus::Warning),
        "m11: varchar widening must be amber, never red"
    );

    // m12: sequence seq_gl_entry left behind (red).
    assert_eq!(
        status_of("seq_gl_entry last_value regressed"),
        Some(DiffStatus::Different),
        "m12: sequence regression must be a FAIL"
    );

    // m13: index ix_transactions_posted missing (red).
    assert_eq!(
        status_of("index missing in new: ix_transactions_posted"),
        Some(DiffStatus::Different),
        "m13: missing index must be a FAIL"
    );

    // m14: constraint ck_transactions_fee not validated (red).
    assert_eq!(
        status_of("ck_transactions_fee is not validated"),
        Some(DiffStatus::Different),
        "m14: unvalidated constraint must be a FAIL"
    );

    // m15: large object missing (red).
    assert_eq!(
        status_of("large object count changed"),
        Some(DiffStatus::Different),
        "m15: large object missing must be a FAIL"
    );

    // q04: balances_daily converted to a partitioned table, shown as ONE table.
    assert!(
        !has("table added in new: balances_daily_p1"),
        "q04: no p1 table"
    );
    assert!(
        !has("table added in new: balances_daily_p2"),
        "q04: no p2 table"
    );
    assert!(
        !has("table added in new: balances_daily_p3"),
        "q04: no p3 table"
    );
    assert!(
        !has("table missing in new: balances_daily"),
        "q04: parent present"
    );

    // q05: holidays columns reordered; order is not data, so no column diff.
    let holidays_differs = flat
        .iter()
        .any(|(p, _, _)| p.iter().any(|seg| seg == "holidays"));
    assert!(
        !holidays_differs,
        "q05: column order must not be a difference"
    );

    // q06: kyc_checks.result char(1) -> varchar(1), amber.
    assert_eq!(
        status_of("character(1) -> character varying(1)"),
        Some(DiffStatus::Warning),
        "q06: char -> varchar must be amber, never red"
    );

    // The planted data carries real structural fails, so the overall verdict is
    // Different (FAIL), not a silent PASS.
    assert_eq!(diff.status, DiffStatus::Different);
}

/// The key columns Choose checks locks (one catalogue query for the whole
/// database) are exactly the primary keys the run itself matches rows on
/// (read per table at run time), for every demo_bank table.
#[tokio::test]
async fn primary_key_columns_match_the_keys_the_run_uses() {
    let Some(old_url) = env_url("DMUX_TEST_DEMO_OLD_URL") else {
        return;
    };
    let inv = collect(&old_url).await;
    let profile = profile_from_url(&old_url);
    let mut session = ReadOnlySession::open(&profile, true)
        .await
        .expect("session opens");
    let tables: Vec<(String, String)> = inv
        .schemas
        .iter()
        .flat_map(|s| {
            s.tables
                .iter()
                .map(move |t| (s.name.clone(), t.name.clone()))
        })
        .collect();
    assert!(tables.len() > 5);
    let (keys, live) = session
        .with_read_only("60s", |txn| {
            let tables = tables.clone();
            Box::pin(async move {
                let keys = dmux_engine::inventory::primary_key_columns(&*txn)
                    .await
                    .map_err(|e| DbError::Postgres {
                        message: e.to_string(),
                    })?;
                let mut live = Vec::new();
                for (s, t) in &tables {
                    let lt = dmux_engine::plan::read_live_table(txn, s, t)
                        .await
                        .map_err(|e| DbError::Postgres {
                            message: e.to_string(),
                        })?;
                    live.push((format!("{s}.{t}"), lt.primary_key));
                }
                Ok((keys, live))
            })
        })
        .await
        .expect("keys read");
    for (path, pk) in live {
        let mut want: Vec<String> = pk;
        want.sort();
        let got: Vec<String> = keys
            .get(&path)
            .map(|s| s.iter().cloned().collect())
            .unwrap_or_default();
        assert_eq!(got, want, "{path}");
    }
}
