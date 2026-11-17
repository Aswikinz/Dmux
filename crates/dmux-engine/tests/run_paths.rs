//! Live tests for the run paths against the planted `demo_bank` databases:
//! exact-name table lookup (item 9), check levels (item 6), resume (item 3),
//! file against live (item 4) and P3 ignores (item 2).
//!
//! Gated on `DMUX_TEST_DEMO_OLD_URL` / `DMUX_TEST_DEMO_NEW_URL`; fails when
//! `CI=true` and they are unset.  Read-only: the resume store is a temporary
//! SQLite file, never the database.

use std::collections::BTreeMap;

use dmux_engine::db::{ConnProfile, DbError, ReadOnlySession, TlsMode};
use dmux_engine::fingerprint_file::FingerprintFile;
use dmux_engine::mapping::{IgnoredItem as MapIgnored, Mapping};
use dmux_engine::results::{CheckLevel, IgnoredItem, RunResults, Verdict};
use dmux_engine::run::capture::{capture, CaptureOptions};
use dmux_engine::run::{run, Input, RunConfig};
use dmux_engine::store::{Loaded, Store};

fn env_url(name: &str) -> Option<String> {
    match std::env::var(name) {
        Ok(u) => Some(u),
        Err(_) => {
            assert!(
                std::env::var("CI").as_deref() != Ok("true"),
                "CI=true but {name} is not set"
            );
            eprintln!("skipped: {name} not set");
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

fn profiles() -> Option<(ConnProfile, ConnProfile)> {
    Some((
        profile_from_url(&env_url("DMUX_TEST_DEMO_OLD_URL")?),
        profile_from_url(&env_url("DMUX_TEST_DEMO_NEW_URL")?),
    ))
}

fn config(old: Input, new: Input, level: CheckLevel) -> RunConfig {
    RunConfig {
        old,
        new,
        default_level: level,
        allow_superuser: true,
        ..RunConfig::default()
    }
}

fn temp(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("dmux-runpaths-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir.join("store.sqlite")
}

async fn go(c: &RunConfig) -> RunResults {
    run(c).await.expect("run should succeed").results
}

#[tokio::test]
async fn tables_are_found_by_exact_name_never_by_parsing_a_string() {
    let Some((old, _)) = profiles() else { return };
    let mut s = ReadOnlySession::open(&old, true).await.unwrap();
    let found = s
        .with_read_only("10s", |txn| {
            Box::pin(async move {
                let q = |e: dmux_engine::plan::PlanError| DbError::Postgres {
                    message: e.to_string(),
                };
                let real = dmux_engine::plan::read_live_table_opt(&*txn, "demo_bank", "accounts")
                    .await
                    .map_err(q)?;
                // Before the fix, "demo_bank.ACCOUNTS" was handed to to_regclass,
                // which folded it to demo_bank.accounts: a different table.
                let upper = dmux_engine::plan::read_live_table_opt(&*txn, "demo_bank", "ACCOUNTS")
                    .await
                    .map_err(q)?;
                // A dotted name used to be parsed as catalog.schema.table.
                let dotted =
                    dmux_engine::plan::read_live_table_opt(&*txn, "demo_bank", "accounts.balance")
                        .await
                        .map_err(q)?;
                let size = dmux_engine::estimate::table_size_bytes(&*txn, "demo_bank", "ACCOUNTS")
                    .await
                    .map_err(|e| DbError::Postgres {
                        message: e.to_string(),
                    })?;
                let real_size =
                    dmux_engine::estimate::table_size_bytes(&*txn, "demo_bank", "accounts")
                        .await
                        .map_err(|e| DbError::Postgres {
                            message: e.to_string(),
                        })?;
                Ok((
                    real.is_some(),
                    upper.is_none(),
                    dotted.is_none(),
                    size.is_none(),
                    real_size.is_some_and(|n| n > 0),
                ))
            })
        })
        .await
        .unwrap();
    assert_eq!(found, (true, true, true, true, true));
}

#[tokio::test]
async fn each_level_scans_what_it_claims_and_is_counted_as_such() {
    let Some((old, new)) = profiles() else { return };
    let mut c = config(Input::Live(old), Input::Live(new), CheckLevel::Structure);
    c.levels = BTreeMap::from([
        (
            "demo_bank.accounts".to_string(),
            CheckLevel::CountsAndTotals,
        ),
        (
            "demo_bank.audit_log".to_string(),
            CheckLevel::CountsAndTotals,
        ),
        ("demo_bank.holidays".to_string(), CheckLevel::FullValues),
    ]);
    c.control_total_columns = ["balance".to_string()].into();
    let r = go(&c).await;

    // CountsAndTotals: no value hashing, counts and control totals compared.
    let ac = &r.tables["demo_bank.accounts"];
    assert_eq!(ac.level, CheckLevel::CountsAndTotals);
    assert_eq!(
        ac.verdict,
        Verdict::Fail,
        "the 0.01 balance change moves the control total"
    );
    assert!(ac.reasons.iter().any(|x| x == "control totals differ"));
    assert!(
        ac.differing_buckets.is_none(),
        "no bucket hashing at counts level"
    );
    let al = &r.tables["demo_bank.audit_log"];
    assert_eq!(al.verdict, Verdict::Fail);
    assert!(al
        .reasons
        .iter()
        .any(|x| x.starts_with("row counts differ")));
    // Structure: no data scan; red findings fail, amber findings do not.
    let tx = &r.tables["demo_bank.transactions"];
    assert_eq!(tx.level, CheckLevel::Structure);
    assert_eq!(
        tx.verdict,
        Verdict::Fail,
        "index missing / constraint not validated"
    );
    assert!(tx.old_rows.is_none(), "no data scan at structure level");
    let br = &r.tables["demo_bank.branches"];
    assert_eq!(br.verdict, Verdict::Pass, "amber only: {:?}", br.reasons);
    assert!(br
        .reasons
        .iter()
        .any(|x| x.contains("character varying(120)")));
    // FullValues still fingerprints.
    let ho = &r.tables["demo_bank.holidays"];
    assert_eq!(ho.verdict, Verdict::Pass);
    assert!(ho.differing_buckets.is_some());

    assert_eq!((r.fully_checked, r.counts_only), (1, 2));
    // The missing renamed table is not counted at any level.
    assert_eq!(r.tables["demo_bank.limits"].verdict, Verdict::Fail);
    assert_eq!(
        r.structure_only,
        r.tables.len() - 3 - 2,
        "every other table except limits (missing) and product_limits (new only)"
    );
    assert!(r
        .coverage_line
        .contains(&format!("{} structure only", r.structure_only)));
}

#[tokio::test]
async fn resume_reuses_only_unchanged_tables_and_never_drops_one() {
    let Some((old, new)) = profiles() else { return };
    let path = temp("resume");
    let _ = std::fs::remove_file(&path);
    let mut c = config(
        Input::Live(old),
        Input::Live(new),
        CheckLevel::CountsAndTotals,
    );
    c.resume_store = Some(path.clone());

    let first = go(&c).await;
    assert!(first.resumed_tables.is_empty());
    let scanned: Vec<String> = first
        .tables
        .values()
        .filter(|t| t.old_rows.is_some())
        .map(|t| t.table.clone())
        .collect();
    assert!(scanned.len() > 30);

    // Same definition: the run id is kept and every finished table is reused.
    let second = go(&c).await;
    assert_eq!(second.run_id, first.run_id, "run id stable across resumes");
    let mut resumed = second.resumed_tables.clone();
    resumed.sort();
    assert_eq!(resumed, scanned);
    for (name, t) in &first.tables {
        assert_eq!(second.tables[name].verdict, t.verdict, "{name}");
    }

    // A stored table whose counters no longer match is rescanned; an
    // unreadable stored row is rescanned, never dropped.
    {
        let store = Store::open(&path).unwrap();
        let Loaded::Found(mut st) = store.load_table("demo_bank.accounts").unwrap() else {
            panic!("accounts was stored");
        };
        st.counters_new.n_tup_upd += 1;
        store.save_table(&st).unwrap();
        store
            .put_raw("demo_bank.branches", "{not a stored result")
            .unwrap();
    }
    let third = go(&c).await;
    assert_eq!(third.run_id, first.run_id);
    for t in ["demo_bank.accounts", "demo_bank.branches"] {
        assert!(
            !third.resumed_tables.contains(&t.to_string()),
            "{t} must be rescanned"
        );
        assert_eq!(
            third.tables[t].verdict, first.tables[t].verdict,
            "{t} still reported"
        );
        assert!(third.tables[t].old_rows.is_some());
    }
    assert_eq!(third.tables.len(), first.tables.len(), "no table dropped");

    // A different definition: nothing is reused and the run id is new.
    c.export_keys = true;
    let fourth = go(&c).await;
    assert!(fourth.resumed_tables.is_empty());
    assert_ne!(fourth.run_id, first.run_id);
    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}

#[tokio::test]
async fn a_dropped_column_ignored_with_a_reason_passes_and_carries_the_reason() {
    let Some((old, new)) = profiles() else { return };
    let mut c = config(Input::Live(old), Input::Live(new), CheckLevel::FullValues);
    c.levels = BTreeMap::new();
    let mut m = Mapping::new();
    m.ignored.push(MapIgnored {
        path: "demo_bank.user_roles.granted_by".into(),
        reason: "dropped on purpose in the new design".into(),
    });
    c.mapping = m.with_fresh_hash();
    // Only the one table matters here: everything else at structure level.
    c.default_level = CheckLevel::Structure;
    c.levels
        .insert("demo_bank.user_roles".into(), CheckLevel::FullValues);
    let r = go(&c).await;
    let t = &r.tables["demo_bank.user_roles"];
    assert_eq!(t.verdict, Verdict::Pass, "{:?}", t.reasons);
    assert!(t
        .reasons
        .iter()
        .any(|x| x.contains("dropped on purpose in the new design")));
    assert!(r.ignored_items.contains(&IgnoredItem {
        path: "demo_bank.user_roles.granted_by".into(),
        reason: "dropped on purpose in the new design".into(),
    }));
    assert!(!r
        .structure_findings
        .iter()
        .any(|f| f.object == "demo_bank.user_roles.granted_by"));
}

#[tokio::test]
async fn a_saved_fingerprint_against_a_live_database() {
    let Some((old, new)) = profiles() else { return };
    let mapping = Mapping::new();
    let controls = ["balance".to_string()].into();
    let none = BTreeMap::new();
    let opts = CaptureOptions {
        level: CheckLevel::FullValues,
        control_total_columns: &controls,
        column_options: &none,
        mapping: &mapping,
        allow_superuser: true,
    };
    // Through the file format, so the untrusted-input checks run too.
    let bytes = capture(&old, &opts).await.unwrap().to_json().unwrap();
    let file = FingerprintFile::from_bytes(&bytes).unwrap();

    let mut c = config(
        Input::File(file.clone()),
        Input::Live(new.clone()),
        CheckLevel::FullValues,
    );
    c.control_total_columns = ["balance".to_string(), "txn_count".to_string()].into();
    let r = go(&c).await;
    let v = |n: &str| r.tables[n].verdict;
    // The planted data differences.
    assert_eq!(v("demo_bank.transactions"), Verdict::Fail);
    assert_eq!(v("demo_bank.audit_log"), Verdict::Fail);
    assert_eq!(v("demo_bank.accounts"), Verdict::Fail);
    assert!(r.tables["demo_bank.accounts"]
        .reasons
        .iter()
        .any(|x| x == "control totals differ"));
    // Missing and extra tables are reported.
    assert_eq!(v("demo_bank.limits"), Verdict::Fail);
    assert_eq!(v("demo_bank.product_limits"), Verdict::Warning);
    // What a saved plan cannot absorb is an explicit TOOL_FAULT, not a skip.
    assert_eq!(
        v("demo_bank.user_roles"),
        Verdict::ToolFault,
        "captured column missing live"
    );
    assert_eq!(v("demo_bank.fees"), Verdict::ToolFault);
    assert_eq!(
        v("demo_bank.kyc_checks"),
        Verdict::ToolFault,
        "type changed since capture"
    );
    // A new live column the capture does not cover: P3.
    assert_eq!(v("demo_bank.loans"), Verdict::NeedsReview);
    // A requested control total the file does not carry lowers a passing table.
    let bd = &r.tables["demo_bank.balances_daily"];
    assert_eq!(bd.verdict, Verdict::NeedsReview, "{:?}", bd.reasons);
    assert!(bd.reasons.iter().any(|x| x.contains("does not carry it")));
    // Quiet tables pass.
    assert_eq!(v("demo_bank.holidays"), Verdict::Pass);
    assert_eq!(v("demo_bank.branches"), Verdict::Pass);
    assert_eq!(r.verdict, Verdict::ToolFault);

    // The other direction: the file is the new side, the old side is live.
    let new_bytes = capture(&new, &opts).await.unwrap().to_json().unwrap();
    let new_file = FingerprintFile::from_bytes(&new_bytes).unwrap();
    let r = go(&config(
        Input::Live(old),
        Input::File(new_file),
        CheckLevel::FullValues,
    ))
    .await;
    assert_eq!(r.tables["demo_bank.transactions"].verdict, Verdict::Fail);
    assert_eq!(r.tables["demo_bank.limits"].verdict, Verdict::Fail);
    assert_eq!(r.tables["demo_bank.holidays"].verdict, Verdict::Pass);

    // A mapping that differs from the capture's is refused outright.
    let mut c = config(Input::File(file), Input::Live(new), CheckLevel::FullValues);
    let mut m = Mapping::new();
    m.ignored.push(MapIgnored {
        path: "demo_bank.notes".into(),
        reason: "x".into(),
    });
    c.mapping = m.with_fresh_hash();
    assert!(run(&c).await.is_err());
}

#[tokio::test]
async fn a_capture_cannot_be_made_at_structure_level() {
    let Some((old, _)) = profiles() else { return };
    let mapping = Mapping::new();
    let (c, o) = (Default::default(), BTreeMap::new());
    let opts = CaptureOptions {
        level: CheckLevel::Structure,
        control_total_columns: &c,
        column_options: &o,
        mapping: &mapping,
        allow_superuser: true,
    };
    assert!(capture(&old, &opts).await.is_err());
}
