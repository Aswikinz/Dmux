//! Cross-check against live servers. Runs only when DMUX_TEST_OLD_URL and
//! DMUX_TEST_NEW_URL are set; reads fixed derived relations (VALUES and
//! generate_series), never a table, inside read-only transactions.

use super::compare::{compare, SideOutcome, Verdict};
use super::drilldown::{changed_detail, classify, fetch_rows};
use super::fingerprint::tests::{accounts_plan, mapping, reference_select};
use super::fingerprint::{fingerprint, parse_summary, Fingerprint, Side, Source, TablePlan};
use super::model;
use super::normalise::{plan_column, resolve_type, ColumnMapping, Geo, PgType};
use super::{text_rows, CoreError};
use tokio_postgres::{Client, IsolationLevel, NoTls, Transaction};

async fn connect(var: &str) -> Option<Client> {
    let url = std::env::var(var).ok()?;
    let (client, conn) = tokio_postgres::connect(&url, NoTls).await.expect("connect");
    tokio::spawn(conn);
    Some(client)
}

async fn both() -> Option<(Client, Client)> {
    let old = connect("DMUX_TEST_OLD_URL").await;
    let new = connect("DMUX_TEST_NEW_URL").await;
    if old.is_none() || new.is_none() {
        assert!(
            std::env::var("CI").as_deref() != Ok("true"),
            "CI=true but DMUX_TEST_OLD_URL / DMUX_TEST_NEW_URL are not set: the live cross-check must run in CI"
        );
        eprintln!("skipped: DMUX_TEST_OLD_URL / DMUX_TEST_NEW_URL not set");
    }
    Some((old?, new?))
}

/// The canonical SET LOCAL block, taken from the reference file itself.
fn settings_block() -> String {
    let path = format!(
        "{}/../../testdata/phase0/fingerprint.sql",
        env!("CARGO_MANIFEST_DIR")
    );
    std::fs::read_to_string(path)
        .unwrap()
        .lines()
        .filter(|l| l.starts_with("SET LOCAL"))
        .map(|l| l.split("--").next().unwrap().trim())
        .collect::<Vec<_>>()
        .join("\n")
}

async fn session(c: &mut Client) -> Transaction<'_> {
    let t = c
        .build_transaction()
        .isolation_level(IsolationLevel::RepeatableRead)
        .read_only(true)
        .start()
        .await
        .unwrap();
    t.batch_execute(&settings_block()).await.unwrap();
    // check_session requires row_security = off on 9.5+, but the setting does
    // not exist on 9.4 (nightly's OLD server), so it is applied conditionally.
    if version_num(&t).await >= 90500 {
        t.batch_execute("SET LOCAL row_security = off")
            .await
            .unwrap();
    }
    t
}

/// Read `server_version_num` from a live transaction.
async fn version_num(t: &Transaction<'_>) -> i32 {
    let row = t.query_one("SHOW server_version_num", &[]).await.unwrap();
    let text: String = row.get(0);
    text.parse().unwrap()
}

const ACC_COLS: &str = "accounts(acct_id, bank_code, holder_name, branch_code, balance, rate, opened_at, doc_blob, remarks)";

const ACC_A: &str = r#"(VALUES
 ('A1'::varchar(20), 'B1'::varchar(10), 'José Ñ'::varchar(80), 'S1'::char(8), 12.3400::numeric(18,4), 0.1::float8, '2020-01-02 03:04:05'::timestamp(0), '\x00ff'::bytea, NULL::text),
 ('A2', 'B1', '', 'S2', -0.0001, 1::float8/3, '1999-12-31 23:59:59', ''::bytea, ''),
 ('A3', 'B2', NULL, NULL, NULL, NULL, NULL, NULL, 'a,b(c)\"'),
 ('A4', 'B2', 'x', 'S4', 1.0000, -0.0::float8, '2024-02-29 00:00:00', '\x01', 'r')
) AS accounts(acct_id, bank_code, holder_name, branch_code, balance, rate, opened_at, doc_blob, remarks)"#;

// ACC_A with exactly one value changed: A4's balance 1 -> 1.0001.
const ACC_B: &str = r#"(VALUES
 ('A1'::varchar(20), 'B1'::varchar(10), 'José Ñ'::varchar(80), 'S1'::char(8), 12.3400::numeric(18,4), 0.1::float8, '2020-01-02 03:04:05'::timestamp(0), '\x00ff'::bytea, NULL::text),
 ('A2', 'B1', '', 'S2', -0.0001, 1::float8/3, '1999-12-31 23:59:59', ''::bytea, ''),
 ('A3', 'B2', NULL, NULL, NULL, NULL, NULL, NULL, 'a,b(c)\"'),
 ('A4', 'B2', 'x', 'S4', 1.0001, -0.0::float8, '2024-02-29 00:00:00', '\x01', 'r')
) AS accounts(acct_id, bank_code, holder_name, branch_code, balance, rate, opened_at, doc_blob, remarks)"#;

async fn fp(t: &Transaction<'_>, plan: &TablePlan, side: Side, src: &'static str) -> Fingerprint {
    fingerprint(t, plan, side, &Source::Derived(src), "m")
        .await
        .unwrap()
}

#[tokio::test]
async fn reference_and_builder_agree_on_both_servers() {
    let Some((mut oc, mut nc)) = both().await else {
        return;
    };
    assert!(ACC_A.ends_with(ACC_COLS));
    let plan = accounts_plan();
    let reference = reference_select("fingerprint.sql").replace("phase0.accounts", ACC_A);
    let mut results = Vec::new();
    for c in [&mut oc, &mut nc] {
        let t = session(c).await;
        let rows = text_rows(&t, &reference).await.unwrap();
        let (ref_buckets, _, ref_n) = parse_summary(&rows, 3, 0).unwrap();
        let ours = fp(&t, &plan, Side::Old, ACC_A).await;
        assert_eq!(ours.buckets, ref_buckets);
        assert_eq!(ours.rows, ref_n);
        assert_eq!(ours.rows, 4);
        let changed = fp(&t, &plan, Side::New, ACC_B).await;
        assert_ne!(changed.buckets, ours.buckets);
        t.rollback().await.unwrap();
        results.push((ours, changed));
    }
    assert_eq!(results[0], results[1], "old and new servers must agree");
}

#[tokio::test]
async fn one_value_change_is_found_and_drilled() {
    let Some((mut oc, mut nc)) = both().await else {
        return;
    };
    let plan = accounts_plan();
    let (to, tn) = (session(&mut oc).await, session(&mut nc).await);
    let a = fp(&to, &plan, Side::Old, ACC_A).await;
    let same = fp(&tn, &plan, Side::New, ACC_A).await;
    assert!(compare(SideOutcome::Ok(&a), SideOutcome::Ok(&same))
        .unwrap()
        .is_pass());
    let b = fp(&tn, &plan, Side::New, ACC_B).await;
    let c = compare(SideOutcome::Ok(&a), SideOutcome::Ok(&b)).unwrap();
    assert_ne!(c.verdict, Verdict::Pass);
    assert_eq!(c.differing.len(), 1);
    let buckets = c.drilldown_buckets();
    let (os, ns) = (Source::Derived(ACC_A), Source::Derived(ACC_B));
    let old_rows = fetch_rows(&to, &plan, Side::Old, &os, &buckets)
        .await
        .unwrap();
    let new_rows = fetch_rows(&tn, &plan, Side::New, &ns, &buckets)
        .await
        .unwrap();
    let d = classify(old_rows, new_rows);
    assert!(d.missing.is_empty() && d.extra.is_empty());
    assert_eq!(d.changed.len(), 1);
    assert_eq!(
        d.changed[0].0.key,
        vec![Some("A4".into()), Some("B2".into())]
    );
    let detail = changed_detail(&to, &tn, &plan, &os, &ns, &d.changed)
        .await
        .unwrap();
    assert_eq!(detail[0].columns, vec![4]);
    assert_eq!(detail[0].old[4].as_deref(), Some("1.0000"));
    assert_eq!(detail[0].new[4].as_deref(), Some("1.0001"));
}

const GEN: &str = r#"(SELECT g::text AS id, CASE WHEN g % 5 = 0 THEN NULL WHEN g % 7 = 0 THEN '' ELSE 'v ' || g || ',"(é)\' END AS v FROM generate_series(1, 300) g) AS t"#;

#[tokio::test]
async fn rust_model_matches_postgres() {
    let Some((mut oc, mut nc)) = both().await else {
        return;
    };
    let rows: Vec<Vec<Option<String>>> = (1..=300)
        .map(|g| {
            let v = match g {
                g if g % 5 == 0 => None,
                g if g % 7 == 0 => Some(String::new()),
                g => Some(format!("v {g},\"(é)\\")),
            };
            vec![Some(g.to_string()), v]
        })
        .collect();
    let cols: Vec<_> = ["id", "v"]
        .iter()
        .map(|n| plan_column(&mapping(n, "text")).unwrap())
        .collect();
    for key in [vec![0], vec![]] {
        let plan = TablePlan::new(cols.clone(), key.clone(), 2).unwrap();
        let want = model::summarise(&rows, &key, 2);
        for c in [&mut oc, &mut nc] {
            let t = session(c).await;
            assert_eq!(fp(&t, &plan, Side::Old, GEN).await.buckets, want);
        }
    }
}

const TYPES_OLD: &str = r#"(VALUES
 (1, 1.2344::numeric(20,4), 0.1::real, ARRAY[0.1, 1::float8/3], '12.34'::money, '1 year 2 mons 3 days 04:05:06'::interval, '2001-01-01 00:00:00+05:30'::timestamptz, '{"b":1,"a":2}'::jsonb, '[1,10)'::int4range, 'a0eebc99-9c0b-4ef8-bb6d-6bb9bd380a11'::uuid, true, NULL::bytea, '2020-01-02'::date, ARRAY[1, NULL]::int[]),
 (2, 1.2345, 3.5, ARRAY[-0.0::float8], '-0.01', '24 hours', '2024-06-30 12:00:00+00', '[1, {"x": null}]', 'empty', NULL, false, ''::bytea, NULL, '{}')
) AS t(id, amt, r, fa, m, iv, ts, j, rg, u, b, bin, d, ia)"#;

const TYPES_NEW: &str = r#"(VALUES
 (1, 1.2344::numeric(20,6), 0.1::real::float8, ARRAY[0.1, 1::float8/3], '12.34'::money, '1 year 2 mons 3 days 04:05:06'::interval, '2001-01-01 00:00:00+05:30'::timestamptz, '{"b":1,"a":2}'::jsonb, '[1,10)'::int4range, 'a0eebc99-9c0b-4ef8-bb6d-6bb9bd380a11'::uuid, true, NULL::bytea, '2020-01-02'::date, ARRAY[1, NULL]::int[]),
 (2, 1.2345, 3.5, ARRAY[-0.0::float8], '-0.01', '24 hours', '2024-06-30 12:00:00+00', '[1, {"x": null}]', 'empty', NULL, false, ''::bytea, NULL, '{}')
) AS t(id, amt, r, fa, m, iv, ts, j, rg, u, b, bin, d, ia)"#;

fn types_plan(key: Vec<usize>) -> TablePlan {
    let spec = [
        ("id", "integer", "integer"),
        ("amt", "numeric(20,4)", "numeric(20,6)"),
        ("r", "real", "double precision"),
        ("fa", "double precision[]", "double precision[]"),
        ("m", "money", "money"),
        ("iv", "interval", "interval"),
        ("ts", "timestamp with time zone", "timestamp with time zone"),
        ("j", "jsonb", "jsonb"),
        ("rg", "int4range", "int4range"),
        ("u", "uuid", "uuid"),
        ("b", "boolean", "boolean"),
        ("bin", "bytea", "bytea"),
        ("d", "date", "date"),
        ("ia", "integer[]", "integer[]"),
    ];
    let cols = spec
        .iter()
        .map(|(n, o, nw)| {
            let m = ColumnMapping {
                old_type: PgType::parse(o),
                new_type: PgType::parse(nw),
                control_total: *n == "amt" || *n == "m",
                ..mapping(n, o)
            };
            plan_column(&m).unwrap()
        })
        .collect();
    TablePlan::new(cols, key, 3).unwrap()
}

#[tokio::test]
async fn type_rules_agree_across_servers_and_type_changes() {
    let Some((mut oc, mut nc)) = both().await else {
        return;
    };
    for key in [vec![0], vec![]] {
        let plan = types_plan(key);
        let mut all = Vec::new();
        for c in [&mut oc, &mut nc] {
            let t = session(c).await;
            let o = fp(&t, &plan, Side::Old, TYPES_OLD).await;
            let n = fp(&t, &plan, Side::New, TYPES_NEW).await;
            assert!(compare(SideOutcome::Ok(&o), SideOutcome::Ok(&n))
                .unwrap()
                .is_pass());
            assert_eq!(o.control_totals[0].to_string(), "2.4689");
            assert_eq!(o.control_totals[1].to_string(), "12.33");
            all.push(o);
        }
        assert_eq!(all[0], all[1]);
    }
}

#[tokio::test]
async fn keyless_duplicates_are_counted() {
    let Some((mut oc, _)) = both().await else {
        return;
    };
    const ONE: &str = "(VALUES ('a'::text), ('a'), ('b')) AS t(v)";
    const TWO: &str = "(VALUES ('a'::text), ('b')) AS t(v)";
    let plan =
        TablePlan::new(vec![plan_column(&mapping("v", "text")).unwrap()], vec![], 3).unwrap();
    let t = session(&mut oc).await;
    let a = fp(&t, &plan, Side::Old, ONE).await;
    let b = fp(&t, &plan, Side::New, TWO).await;
    let c = compare(SideOutcome::Ok(&a), SideOutcome::Ok(&b)).unwrap();
    assert!(!c.is_pass());
    let bs = c.drilldown_buckets();
    let old = fetch_rows(&t, &plan, Side::Old, &Source::Derived(ONE), &bs)
        .await
        .unwrap();
    let new = fetch_rows(&t, &plan, Side::New, &Source::Derived(TWO), &bs)
        .await
        .unwrap();
    let d = classify(old, new);
    assert_eq!(d.missing.len(), 1);
    assert!(d.extra.is_empty() && d.changed.is_empty());
}

#[tokio::test]
async fn refuses_to_run_outside_the_read_only_session() {
    let Some((mut oc, _)) = both().await else {
        return;
    };
    let plan = accounts_plan();
    let src = Source::Derived(ACC_A);
    // Read-write transaction with the settings: refused before any scan.
    let t = oc.transaction().await.unwrap();
    t.batch_execute(&settings_block()).await.unwrap();
    let err = fingerprint(&t, &plan, Side::Old, &src, "m")
        .await
        .unwrap_err();
    assert_eq!(err, CoreError::UnsafeSession("transaction_read_only"));
    assert_eq!(
        fetch_rows(&t, &plan, Side::Old, &src, &[1])
            .await
            .unwrap_err(),
        CoreError::UnsafeSession("transaction_read_only")
    );
    t.rollback().await.unwrap();
    // Read-only but without the SET LOCAL block.
    let t = oc
        .build_transaction()
        .isolation_level(IsolationLevel::RepeatableRead)
        .read_only(true)
        .start()
        .await
        .unwrap();
    assert_eq!(
        fingerprint(&t, &plan, Side::Old, &src, "m")
            .await
            .unwrap_err(),
        CoreError::UnsafeSession("search_path")
    );
    t.rollback().await.unwrap();
    // Timeouts switched off (P17).
    for setting in ["statement_timeout", "lock_timeout"] {
        let t = session(&mut oc).await;
        t.batch_execute(&format!("SET LOCAL {setting} = 0"))
            .await
            .unwrap();
        assert_eq!(
            fingerprint(&t, &plan, Side::Old, &src, "m")
                .await
                .unwrap_err(),
            CoreError::UnsafeSession(setting)
        );
    }
    // A database error keeps only the SQLSTATE.
    let t = session(&mut oc).await;
    let bad = Source::Table {
        schema: "dmux_no_such_schema".into(),
        name: "t".into(),
    };
    assert_eq!(
        fingerprint(&t, &plan, Side::Old, &bad, "m")
            .await
            .unwrap_err(),
        CoreError::Db("42P01".into())
    );
}

#[tokio::test]
async fn row_security_off_is_required_on_95_plus_only() {
    let Some((mut oc, _)) = both().await else {
        return;
    };
    let plan = accounts_plan();
    let src = Source::Derived(ACC_A);
    // Everything correct except `row_security = off`, which a session could
    // omit (e.g. a hand-rolled connection or a pooler that dropped it).  On
    // 9.5+ that must be refused before any scan; on 9.4 it is not required.
    let t = oc
        .build_transaction()
        .isolation_level(IsolationLevel::RepeatableRead)
        .read_only(true)
        .start()
        .await
        .unwrap();
    t.batch_execute(&settings_block()).await.unwrap();
    let result = fingerprint(&t, &plan, Side::Old, &src, "m").await;
    if version_num(&t).await >= 90500 {
        assert_eq!(
            result.unwrap_err(),
            CoreError::UnsafeSession("row_security")
        );
    } else {
        assert!(result.is_ok(), "9.4 must not require row_security");
    }
    t.rollback().await.unwrap();
}

/// Plan `id integer` (key) + `v` of the given type on both sides.
fn one_col_plan(v: PgType) -> TablePlan {
    let m = ColumnMapping {
        old_type: v.clone(),
        new_type: v,
        ..mapping("v", "integer")
    };
    let cols = vec![
        plan_column(&mapping("id", "integer")).unwrap(),
        plan_column(&m).unwrap(),
    ];
    TablePlan::new(cols, vec![0], 3).unwrap()
}

/// `old` and `new` print alike under extra_float_digits = 0, yet must differ.
async fn must_catch(plan: &TablePlan, old: &'static str, new: &'static str, printed: &str) {
    let Some((mut oc, mut nc)) = both().await else {
        return;
    };
    for c in [&mut oc, &mut nc] {
        let t = session(c).await;
        // The trap is real: the printed forms are identical.
        let r = text_rows(&t, printed).await.unwrap();
        assert_eq!(r[0][0].as_deref(), Some("t"), "{printed}");
        let a = fp(&t, plan, Side::Old, old).await;
        let b = fp(&t, plan, Side::New, new).await;
        assert!(compare(SideOutcome::Ok(&a), SideOutcome::Ok(&a.clone()))
            .unwrap()
            .is_pass());
        assert_eq!(
            compare(SideOutcome::Ok(&a), SideOutcome::Ok(&b))
                .unwrap()
                .verdict,
            Verdict::Fail
        );
    }
}

#[tokio::test]
async fn one_ulp_inside_a_float8_array_is_caught() {
    must_catch(
        &one_col_plan(PgType::parse("double precision[]")),
        "(VALUES (1, ARRAY[0.1::float8, 2])) AS t(id, v)",
        "(VALUES (1, ARRAY['0.10000000000000002'::float8, 2])) AS t(id, v)",
        "SELECT ARRAY[0.1::float8]::text = ARRAY['0.10000000000000002'::float8]::text",
    )
    .await;
}

#[tokio::test]
async fn one_ulp_inside_a_point_is_caught() {
    must_catch(
        &one_col_plan(PgType::Geometric(Geo::Point)),
        "(VALUES (1, point(0.1, 1))) AS t(id, v)",
        "(VALUES (1, point('0.10000000000000002'::float8, 1))) AS t(id, v)",
        "SELECT point(0.1, 1)::text = point('0.10000000000000002'::float8, 1)::text",
    )
    .await;
}

#[tokio::test]
async fn resolver_reads_the_catalog() {
    let Some((mut oc, mut nc)) = both().await else {
        return;
    };
    for c in [&mut oc, &mut nc] {
        let t = session(c).await;
        let oid = |name: &'static str| {
            let t = &t;
            async move {
                let r = t
                    .query_one("SELECT $1::text::regtype::oid", &[&name])
                    .await
                    .unwrap();
                r.get::<_, u32>(0)
            }
        };
        let r = |o, m| resolve_type(&t, o, m);
        assert_eq!(
            r(oid("float8[]").await, -1).await.unwrap(),
            PgType::Array(Box::new(PgType::Float8))
        );
        assert_eq!(
            r(oid("numeric").await, (10 << 16) + 2 + 4).await.unwrap(),
            PgType::Numeric(Some((10, 2)))
        );
        assert_eq!(
            r(oid("polygon").await, -1).await.unwrap(),
            PgType::Geometric(Geo::Polygon)
        );
        let dom = r(oid("information_schema.cardinal_number").await, -1)
            .await
            .unwrap();
        assert_eq!(dom.base(), &PgType::Int4);
        assert!(matches!(dom, PgType::Domain { .. }));
        // point has typelem float8 and box has typelem point: still not arrays.
        for (n, g) in [
            ("point", Geo::Point),
            ("box", Geo::Box),
            ("line", Geo::Line),
        ] {
            assert_eq!(r(oid(n).await, -1).await.unwrap(), PgType::Geometric(g));
        }
        let nr = r(oid("numrange").await, -1).await.unwrap();
        assert!(matches!(nr, PgType::Range { ref sub, .. } if **sub == PgType::Numeric(None)));
        // Arrays of domains (PG 11+) resolve element-wise to the domain's base.
        let arr: u32 = t
            .query_one(
                "SELECT typarray FROM pg_type WHERE oid = 'information_schema.cardinal_number'::regtype",
                &[],
            )
            .await
            .unwrap()
            .get(0);
        if arr != 0 {
            let a = r(arr, -1).await.unwrap();
            let PgType::Array(ref e) = a else {
                panic!("array expected");
            };
            assert_eq!(e.base(), &PgType::Int4);
            assert!(matches!(**e, PgType::Domain { .. }));
        }
        let rng = r(oid("int4range").await, -1).await.unwrap();
        assert!(matches!(rng, PgType::Range { ref sub, .. } if **sub == PgType::Int4));
        // pg_class's row type holds reltuples real: printed form is not exact.
        let comp = r(oid("pg_class").await, -1).await.unwrap();
        let PgType::Composite { ref fields, .. } = comp else {
            panic!("composite expected");
        };
        assert!(fields.contains(&PgType::Float4));
        assert!(one_col_plan(comp.clone()).needs_review());
        assert!(matches!(
            r(oid("anyelement").await, -1).await.unwrap(),
            PgType::Unknown(_)
        ));
        assert_eq!(
            r(0, -1).await.unwrap_err(),
            CoreError::Malformed("type not found")
        );
    }
}

#[tokio::test]
async fn domain_columns_use_the_base_rule_live() {
    let Some((mut oc, mut nc)) = both().await else {
        return;
    };
    const OLD: &str = "(VALUES (1, 5::information_schema.cardinal_number)) AS t(id, v)";
    const NEW: &str = "(VALUES (1, 6::information_schema.cardinal_number)) AS t(id, v)";
    for c in [&mut oc, &mut nc] {
        let t = session(c).await;
        let o: u32 = t
            .query_one(
                "SELECT 'information_schema.cardinal_number'::regtype::oid",
                &[],
            )
            .await
            .unwrap()
            .get(0);
        let plan = one_col_plan(resolve_type(&t, o, -1).await.unwrap());
        assert!(!plan.needs_review());
        let a = fp(&t, &plan, Side::Old, OLD).await;
        let b = fp(&t, &plan, Side::New, NEW).await;
        assert!(!compare(SideOutcome::Ok(&a), SideOutcome::Ok(&b))
            .unwrap()
            .is_pass());
        // A domain over float8, if the test database provides one (no DDL here).
        let found = t
            .query_opt(
                "SELECT t.oid, format_type(t.oid, NULL) FROM pg_type t \
                 WHERE t.typtype = 'd' AND t.typbasetype = 'float8'::regtype LIMIT 1",
                &[],
            )
            .await
            .unwrap();
        let Some(row) = found else {
            eprintln!(
                "notice: no domain over float8 in the test database; 1-ULP domain case not run"
            );
            continue;
        };
        let (doid, dname): (u32, String) = (row.get(0), row.get(1));
        let plan = one_col_plan(resolve_type(&t, doid, -1).await.unwrap());
        let old: &'static str =
            Box::leak(format!("(VALUES (1, 0.1::float8::{dname})) AS t(id, v)").into_boxed_str());
        let new: &'static str = Box::leak(
            format!("(VALUES (1, '0.10000000000000002'::float8::{dname})) AS t(id, v)")
                .into_boxed_str(),
        );
        let a = fp(&t, &plan, Side::Old, old).await;
        let b = fp(&t, &plan, Side::New, new).await;
        assert!(!compare(SideOutcome::Ok(&a), SideOutcome::Ok(&b))
            .unwrap()
            .is_pass());
    }
}

// --- App-side hashing (PLAN 2.2) and huge-table ranges (PLAN 3.1) ---

use super::apphash::{fetch_rows_app, fingerprint_app, values_by_key_hash_app};
use super::fingerprint::{
    fingerprint_range, merge_fingerprints, range_cuts, range_parts, RangeFilter, RangeType,
};

async fn app_fp(
    t: &Transaction<'_>,
    plan: &TablePlan,
    side: Side,
    src: &'static str,
) -> Fingerprint {
    fingerprint_app(t, plan, side, &Source::Derived(src), None, "m")
        .await
        .unwrap()
}

/// The app-side fingerprint is bit-identical to the server's, on both
/// servers, for every fixture: accents in LATIN1, quotes and commas, empty
/// versus NULL, bytea, floats, arrays, ranges, money, intervals, json and
/// control totals; keyed and keyless.
#[tokio::test]
async fn app_side_hashing_equals_the_server_on_both_servers() {
    let Some((mut oc, mut nc)) = both().await else {
        return;
    };
    let gen_cols: Vec<_> = ["id", "v"]
        .iter()
        .map(|n| plan_column(&mapping(n, "text")).unwrap())
        .collect();
    let cases: Vec<(TablePlan, Side, &'static str)> = vec![
        (accounts_plan(), Side::Old, ACC_A),
        (accounts_plan(), Side::New, ACC_B),
        (
            TablePlan::new(gen_cols.clone(), vec![0], 2).unwrap(),
            Side::Old,
            GEN,
        ),
        (TablePlan::new(gen_cols, vec![], 2).unwrap(), Side::Old, GEN),
        (types_plan(vec![0]), Side::Old, TYPES_OLD),
        (types_plan(vec![0]), Side::New, TYPES_NEW),
        (types_plan(vec![]), Side::Old, TYPES_OLD),
        (types_plan(vec![]), Side::New, TYPES_NEW),
    ];
    for c in [&mut oc, &mut nc] {
        let t = session(c).await;
        for (plan, side, src) in &cases {
            let server = fp(&t, plan, *side, src).await;
            let app = app_fp(&t, plan, *side, src).await;
            assert_eq!(
                app, server,
                "app-side hashing must equal the server for {src}"
            );
        }
        t.rollback().await.unwrap();
    }
}

/// App-side drill-down rows and changed-row values equal the server path.
#[tokio::test]
async fn app_side_drilldown_equals_the_server() {
    let Some((mut oc, mut nc)) = both().await else {
        return;
    };
    let plan = accounts_plan();
    for c in [&mut oc, &mut nc] {
        let t = session(c).await;
        let a = fp(&t, &plan, Side::Old, ACC_A).await;
        let b = fp(&t, &plan, Side::New, ACC_B).await;
        let cmp = compare(SideOutcome::Ok(&a), SideOutcome::Ok(&b)).unwrap();
        let buckets = cmp.drilldown_buckets();
        for (side, src) in [(Side::Old, ACC_A), (Side::New, ACC_B)] {
            let s = Source::Derived(src);
            let mut server = fetch_rows(&t, &plan, side, &s, &buckets).await.unwrap();
            let mut app = fetch_rows_app(&t, &plan, side, &s, &buckets).await.unwrap();
            server.sort_by(|x, y| x.key_hash.cmp(&y.key_hash));
            app.sort_by(|x, y| x.key_hash.cmp(&y.key_hash));
            assert_eq!(app, server);
            assert!(!app.is_empty());
        }
        // Changed-row values: the server path's changed_detail versus the app's.
        let old_rows = fetch_rows(&t, &plan, Side::Old, &Source::Derived(ACC_A), &buckets)
            .await
            .unwrap();
        let new_rows = fetch_rows(&t, &plan, Side::New, &Source::Derived(ACC_B), &buckets)
            .await
            .unwrap();
        let d = classify(old_rows, new_rows);
        let hashes: Vec<String> = d.changed.iter().map(|(o, _)| o.key_hash.clone()).collect();
        let detail = changed_detail(
            &t,
            &t,
            &plan,
            &Source::Derived(ACC_A),
            &Source::Derived(ACC_B),
            &d.changed,
        )
        .await
        .unwrap();
        let app_old =
            values_by_key_hash_app(&t, &plan, Side::Old, &Source::Derived(ACC_A), &hashes)
                .await
                .unwrap();
        let app_new =
            values_by_key_hash_app(&t, &plan, Side::New, &Source::Derived(ACC_B), &hashes)
                .await
                .unwrap();
        for row in &detail {
            assert_eq!(app_old[&row.key_hash], row.old);
            assert_eq!(app_new[&row.key_hash], row.new);
        }
        t.rollback().await.unwrap();
    }
}

const SPLIT: &str = r#"(SELECT g AS id, g::text AS v, date '2020-01-01' + g AS d, (g * 1.5)::numeric(10,2) AS amt, CASE WHEN g % 10 = 0 THEN NULL ELSE g END AS nk, timestamp '2021-03-04 05:06:07' + g * interval '37 minutes' AS ts, CASE WHEN g % 3 = 0 THEN 'x' END AS w FROM generate_series(1, 1000) g) AS t"#;

fn split_plan(key: Vec<usize>, digits: u8) -> TablePlan {
    let spec = [
        ("id", "integer"),
        ("v", "text"),
        ("d", "date"),
        ("amt", "numeric(10,2)"),
        ("nk", "integer"),
        ("ts", "timestamp without time zone"),
        ("w", "text"),
    ];
    let cols = spec
        .iter()
        .map(|(n, ty)| {
            plan_column(&ColumnMapping {
                control_total: *n == "amt",
                ..mapping(n, ty)
            })
            .unwrap()
        })
        .collect();
    TablePlan::new(cols, key, digits).unwrap()
}

/// Splitting a table into ranges and summing the parts gives exactly the
/// unsplit fingerprint, for integer, date, numeric, timestamp and a column
/// with NULLs, with any number of parts, in both hashing modes.
#[tokio::test]
async fn range_parts_sum_to_the_whole_table() {
    let Some((mut oc, mut nc)) = both().await else {
        return;
    };
    let src = Source::Derived(SPLIT);
    for c in [&mut oc, &mut nc] {
        let t = session(c).await;
        for key in [vec![0], vec![]] {
            let plan = split_plan(key, 2);
            let whole = fp(&t, &plan, Side::Old, SPLIT).await;
            assert_eq!(whole.rows, 1000);
            for (col, ty) in [
                ("id", RangeType::Int4),
                ("d", RangeType::Date),
                ("amt", RangeType::Numeric),
                ("nk", RangeType::Int4),
                ("ts", RangeType::Timestamp),
            ] {
                for n in [2u32, 3, 7, 64] {
                    let cuts = range_cuts(&t, &src, col, ty, n).await.unwrap();
                    assert!(!cuts.is_empty() && cuts.len() < n as usize);
                    let parts = range_parts(&cuts).unwrap();
                    let mut server_parts = Vec::new();
                    let mut app_parts = Vec::new();
                    for part in parts {
                        let r = RangeFilter {
                            column: col.into(),
                            ty,
                            part,
                        };
                        server_parts.push(
                            fingerprint_range(&t, &plan, Side::Old, &src, &r, "m")
                                .await
                                .unwrap(),
                        );
                        app_parts.push(
                            fingerprint_app(&t, &plan, Side::Old, &src, Some(&r), "m")
                                .await
                                .unwrap(),
                        );
                    }
                    let merged = merge_fingerprints(&server_parts).unwrap();
                    assert_eq!(merged, whole, "{col} in {n} parts (server)");
                    assert_eq!(
                        merge_fingerprints(&app_parts).unwrap(),
                        whole,
                        "{col} in {n} parts (app)"
                    );
                }
            }
        }
        t.rollback().await.unwrap();
    }
}

/// A column that is entirely NULL (or an empty table) gives no cuts, so the
/// table is scanned whole rather than split into parts that miss rows.
#[tokio::test]
async fn no_cuts_for_an_all_null_column() {
    let Some((mut oc, _)) = both().await else {
        return;
    };
    let t = session(&mut oc).await;
    let src = Source::Derived("(SELECT NULL::int AS k FROM generate_series(1, 5)) AS t");
    let cuts = range_cuts(&t, &src, "k", RangeType::Int4, 4).await.unwrap();
    assert!(cuts.is_empty());
    assert!(range_parts(&cuts).is_none());
    assert!(range_cuts(&t, &src, "k", RangeType::Int4, 1)
        .await
        .unwrap()
        .is_empty());
}
