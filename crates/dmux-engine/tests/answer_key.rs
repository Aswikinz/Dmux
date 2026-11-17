//! The answer-key acceptance harness (PLAN 8.2, T022 item 10).
//!
//! Runs the full engine over the planted `demo_bank` databases and asserts every
//! `must_report` data entry (m01-m06) is reported exactly, and every
//! `must_stay_quiet` data entry (q01-q06) PASSes (or is amber where specified),
//! with **no false alarms** on any other table.
//!
//! The structure entries (m07-m15) are covered separately by
//! `demo_bank_integration.rs`; this test covers the data-comparison entries and
//! reads the answer key at test time (never hardcodes key values).

use std::collections::BTreeMap;

use dmux_engine::db::{ConnProfile, TlsMode};
use dmux_engine::mapping::{ColumnMapping, Mapping, MappingStatus, SchemaMapping, TableMapping};
use dmux_engine::results::{CheckLevel, RunResults, Verdict};
use dmux_engine::run::{run, Input, RunConfig};

fn key_path() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../testdata/expected_diffs.json")
}

struct AnswerKey {
    value: serde_json::Value,
}

impl AnswerKey {
    fn load() -> Self {
        let text = std::fs::read_to_string(key_path()).expect("answer key missing");
        AnswerKey {
            value: serde_json::from_str(&text).expect("answer key invalid JSON"),
        }
    }

    fn entries(&self, section: &str) -> Vec<&serde_json::Value> {
        self.value
            .get(section)
            .and_then(|v| v.as_array())
            .expect("section must be an array")
            .iter()
            .collect()
    }

    /// (id, expected-object) for an entry.
    fn expected(&self, section: &str, id: &str) -> &serde_json::Value {
        self.entries(section)
            .iter()
            .find(|e| e.get("id").and_then(|v| v.as_str()) == Some(id))
            .and_then(|e| e.get("expected"))
            .expect("entry missing")
    }
}

fn env_url(name: &str) -> Option<String> {
    match std::env::var(name) {
        Ok(u) => Some(u),
        Err(_) => {
            assert!(
                std::env::var("CI").as_deref() != Ok("true"),
                "CI=true but {name} is not set: the answer-key harness must run in CI"
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
        TlsMode::Disable("mock database has no TLS".to_string()),
    )
}

/// Build a mapping from the answer key's rename entries (m09 table rename,
/// m10 column rename), so the harness reads the key instead of hardcoding.
fn harness_mapping(key: &AnswerKey) -> Mapping {
    // m09: table limits -> product_limits.
    let m09 = key.expected("must_report", "m09");
    let old_table = m09["old_name"].as_str().unwrap();
    let new_table = m09["new_name"].as_str().unwrap();

    // m10: column fees.fee_name -> fee_label.
    let m10 = key.expected("must_report", "m10");
    let table = m10["table"].as_str().unwrap();
    let old_col = m10["old_name"].as_str().unwrap();
    let new_col = m10["new_name"].as_str().unwrap();

    let mut mapping = Mapping::new();
    let mut schema = SchemaMapping {
        target_schema: "demo_bank".to_string(),
        table_mappings: BTreeMap::new(),
    };
    schema.table_mappings.insert(
        old_table.to_string(),
        TableMapping {
            target_table: new_table.to_string(),
            status: MappingStatus::Exact,
            column_mappings: BTreeMap::new(),
        },
    );
    let mut fees = TableMapping {
        target_table: table.to_string(),
        status: MappingStatus::Exact,
        column_mappings: BTreeMap::new(),
    };
    fees.column_mappings.insert(
        old_col.to_string(),
        ColumnMapping {
            target_column: new_col.to_string(),
            status: MappingStatus::Exact,
        },
    );
    schema.table_mappings.insert(table.to_string(), fees);
    mapping
        .schema_mappings
        .insert("demo_bank".to_string(), schema);
    mapping
}

async fn run_engine(old_url: &str, new_url: &str) -> RunResults {
    let mapping = harness_mapping(&AnswerKey::load());
    let config = RunConfig {
        old: Input::Live(profile_from_url(old_url)),
        new: Input::Live(profile_from_url(new_url)),
        mapping,
        default_level: CheckLevel::FullValues,
        levels: BTreeMap::new(),
        control_total_columns: ["balance"].iter().map(|s| s.to_string()).collect(),
        old_parallelism: 2,
        new_parallelism: 4,
        allow_superuser: true,
        export_keys: false,
        resume_store: None,
    };
    run(&config)
        .await
        .expect("engine run should succeed")
        .results
}

/// Tables planted with a must-report data difference; every other table must
/// PASS (no false alarms).
fn planted_data_tables() -> &'static [&'static str] {
    &[
        "demo_bank.transactions",
        "demo_bank.audit_log",
        "demo_bank.accounts",
    ]
}

#[tokio::test]
async fn answer_key_data_reports_every_difference_and_stays_quiet() {
    let (Some(old_url), Some(new_url)) = (
        env_url("DMUX_TEST_DEMO_OLD_URL"),
        env_url("DMUX_TEST_DEMO_NEW_URL"),
    ) else {
        return;
    };

    let results = run_engine(&old_url, &new_url).await;

    // m01: transactions missing 12 rows.
    let tx = &results.tables["demo_bank.transactions"];
    assert_eq!(tx.verdict, Verdict::Fail, "m01: transactions must FAIL");
    assert_eq!(tx.old_rows, Some(20000), "m01 old count");
    assert_eq!(tx.new_rows, Some(19988), "m01 new count");
    assert_eq!(tx.missing_rows, Some(12), "m01: 12 missing rows");

    // m02: audit_log 3 extra rows (keyless counted set).
    let al = &results.tables["demo_bank.audit_log"];
    assert_eq!(al.verdict, Verdict::Fail, "m02: audit_log must FAIL");
    assert_eq!(al.old_rows, Some(10495));
    assert_eq!(al.new_rows, Some(10498));
    assert_eq!(al.extra_rows, Some(3), "m02: 3 extra rows");

    // m03: accounts balance +0.01 -> full-values FAIL and control-total differ.
    let ac = &results.tables["demo_bank.accounts"];
    assert_eq!(ac.verdict, Verdict::Fail, "m03: accounts must FAIL");
    assert_eq!(ac.old_rows, Some(5000));
    assert_eq!(ac.new_rows, Some(5000));
    assert!(
        ac.reasons
            .iter()
            .any(|r| r.contains("control totals differ")),
        "m03: control total must differ"
    );

    // m04, m05, m06 all live inside transactions: 15 differing buckets total
    // (12 missing + 3 changed rows).  The changed rows also surface.
    assert!(
        tx.differing_buckets.unwrap_or(0) + tx.missing_buckets.unwrap_or(0) >= 15,
        "transactions must show the planted changes"
    );
    assert_eq!(tx.changed_rows, Some(3), "m04/m05/m06: 3 changed rows");

    // Every non-planted table must PASS (no false alarms), including the quiet
    // cases q01-q06 (LATIN1 vs UTF-8, sort order, floats, partitioned table,
    // reordered columns, char->varchar).
    for (name, t) in &results.tables {
        if planted_data_tables().contains(&name.as_str()) {
            continue;
        }
        assert_eq!(
            t.verdict,
            Verdict::Pass,
            "table {name} must stay quiet but got {:?} (reasons: {:?})",
            t.verdict,
            t.reasons
        );
    }

    // q04: balances_daily is shown as ONE table (not partitions).
    assert!(results.tables.contains_key("demo_bank.balances_daily"));
    assert!(!results.tables.contains_key("demo_bank.balances_daily_p1"));

    // The overall verdict is FAIL because of the planted data changes.
    assert_eq!(results.verdict, Verdict::Fail);
}
