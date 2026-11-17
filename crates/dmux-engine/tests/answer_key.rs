//! The answer-key acceptance harness (PLAN.md 8.2, T022 item 10).
//!
//! Runs the full engine over the planted `demo_bank` databases and grades the
//! **run results** against `testdata/expected_diffs.json`:
//!
//! * every `must_report` entry (m01–m15), data and structure alike, is found
//!   exactly — counts, keys, columns, types, sequence values and large-object
//!   counts all come from the key at test time, nothing is hard-coded;
//! * every `must_stay_quiet` entry (q01–q06) passes (amber where the key says so);
//! * every other table passes and there is no red structure finding the key
//!   does not list (no false alarms).
//!
//! Two runs: the identity mapping (the planted renames surface as unmapped
//! pairs with suggested matches, m09/m10) and a mapping built from the key's
//! m09/m10 entries (the renamed data compares clean).  The runtime case r01 is
//! `tests/runtime_change.rs` (env-gated, it writes).
//!
//! Gated on `DMUX_TEST_DEMO_OLD_URL` / `DMUX_TEST_DEMO_NEW_URL`; fails when
//! `CI=true` and they are unset.

use std::collections::{BTreeMap, BTreeSet};

use dmux_engine::core::fingerprint::Decimal;
use dmux_engine::db::{ConnProfile, TlsMode};
use dmux_engine::mapping::{ColumnMapping, Mapping, MappingStatus, SchemaMapping, TableMapping};
use dmux_engine::results::{CheckLevel, RunResults, StructureFinding, TableResult, Verdict};
use dmux_engine::run::{run, Input, RunConfig};
use serde_json::Value;

struct Key(Value);

impl Key {
    fn load() -> Key {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../testdata/expected_diffs.json");
        Key(
            serde_json::from_str(&std::fs::read_to_string(path).expect("answer key missing"))
                .expect("answer key is JSON"),
        )
    }

    fn section(&self, s: &str) -> &Vec<Value> {
        self.0[s].as_array().expect("section is an array")
    }

    fn entry(&self, id: &str) -> &Value {
        ["must_report", "must_stay_quiet", "runtime"]
            .iter()
            .flat_map(|s| self.section(s).iter())
            .find(|e| e["id"] == id)
            .unwrap_or_else(|| panic!("no entry {id}"))
    }

    fn exp(&self, id: &str) -> &Value {
        &self.entry(id)["expected"]
    }

    fn object(&self, id: &str) -> String {
        s(&self.entry(id)["object"])
    }
}

fn s(v: &Value) -> String {
    v.as_str()
        .unwrap_or_else(|| panic!("expected a string, got {v}"))
        .to_string()
}

fn u(v: &Value) -> u64 {
    v.as_u64()
        .unwrap_or_else(|| panic!("expected an integer, got {v}"))
}

/// `schema.table` of an object `schema.table[.column]`.
fn table_of(object: &str) -> String {
    object.splitn(3, '.').take(2).collect::<Vec<_>>().join(".")
}

fn schema_of(object: &str) -> String {
    object.split('.').next().unwrap().to_string()
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
    let (creds, rest) = url.split_once('@').expect("user:pass@host");
    let (user, password) = creds.split_once(':').expect("user:pass");
    let (host_port, dbname) = rest.split_once('/').unwrap_or((rest, ""));
    let (host, port) = host_port.split_once(':').expect("host:port");
    ConnProfile::new(
        host,
        port.parse().expect("numeric port"),
        dbname,
        user,
        password,
        TlsMode::Disable("mock database has no TLS".to_string()),
    )
}

/// The mapping the key implies: m09's table rename and m10's column rename.
fn key_mapping(key: &Key) -> Mapping {
    let schema = schema_of(&key.object("m09"));
    let m09 = key.exp("m09");
    let m10 = key.exp("m10");
    let mut sm = SchemaMapping {
        target_schema: schema.clone(),
        table_mappings: BTreeMap::new(),
    };
    sm.table_mappings.insert(
        s(&m09["old_name"]),
        TableMapping {
            target_table: s(&m09["new_name"]),
            status: MappingStatus::Exact,
            column_mappings: BTreeMap::new(),
        },
    );
    let mut cols = BTreeMap::new();
    cols.insert(
        s(&m10["old_name"]),
        ColumnMapping {
            target_column: s(&m10["new_name"]),
            status: MappingStatus::Exact,
        },
    );
    sm.table_mappings.insert(
        s(&m10["table"]),
        TableMapping {
            target_table: s(&m10["table"]),
            status: MappingStatus::Exact,
            column_mappings: cols,
        },
    );
    let mut m = Mapping::new();
    m.schema_mappings.insert(schema, sm);
    m.with_fresh_hash()
}

async fn run_engine(key: &Key, mapping: Mapping) -> Option<RunResults> {
    let (old, new) = (
        env_url("DMUX_TEST_DEMO_OLD_URL")?,
        env_url("DMUX_TEST_DEMO_NEW_URL")?,
    );
    // m03's control total names the column ("accounts.balance").
    let control = s(&key.exp("m03")["control_total"]["column"]);
    let control = control.rsplit('.').next().unwrap().to_string();
    let config = RunConfig {
        mapping,
        default_level: CheckLevel::FullValues,
        control_total_columns: [control].into(),
        allow_superuser: true,
        export_keys: true,
        ..RunConfig::new(
            Input::Live(profile_from_url(&old)),
            Input::Live(profile_from_url(&new)),
        )
    };
    Some(
        run(&config)
            .await
            .expect("engine run should succeed")
            .results,
    )
}

fn table<'a>(r: &'a RunResults, name: &str) -> &'a TableResult {
    r.tables
        .get(name)
        .unwrap_or_else(|| panic!("no result for table {name}; have {:?}", r.tables.keys()))
}

fn finding<'a>(r: &'a RunResults, object: &str) -> &'a StructureFinding {
    r.structure_findings
        .iter()
        .find(|f| f.object == object)
        .unwrap_or_else(|| {
            panic!(
                "no structure finding for {object}; have {:?}",
                r.structure_findings
                    .iter()
                    .map(|f| &f.object)
                    .collect::<Vec<_>>()
            )
        })
}

/// The integer key of one-column keys, as exported.
fn int_keys(keys: &[Vec<Option<String>>]) -> BTreeSet<u64> {
    keys.iter()
        .map(|k| {
            assert_eq!(k.len(), 1, "a one-column key");
            k[0].as_deref().unwrap().parse().unwrap()
        })
        .collect()
}

/// The exported changed row whose single key column equals `key`.
fn changed_columns(r: &RunResults, table: &str, key: u64) -> Vec<String> {
    r.details[table]
        .changed_rows
        .iter()
        .find(|row| {
            row.key.as_ref().map(|k| int_keys(std::slice::from_ref(k))) == Some([key].into())
        })
        .unwrap_or_else(|| panic!("no changed row {key} in {table}"))
        .columns
        .clone()
}

fn key_value(expected: &Value) -> u64 {
    let obj = expected["key"].as_object().expect("key object");
    assert_eq!(obj.len(), 1, "a one-column key");
    u(obj.values().next().unwrap())
}

/// Tables the key expects not to PASS in the identity run.
fn expected_non_pass(key: &Key) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for e in key.section("must_report") {
        let object = s(&e["object"]);
        let kind = s(&e["kind"]);
        match kind.as_str() {
            "missing_rows" | "extra_rows" | "changed_values" | "null_vs_empty"
            | "schema_missing" | "schema_added" => {
                out.insert(table_of(&object));
            }
            "rename_suggested" => {
                let x = &e["expected"];
                let schema = schema_of(&object);
                match x.get("table") {
                    Some(t) => {
                        out.insert(format!("{schema}.{}", s(t)));
                    }
                    None => {
                        out.insert(format!("{schema}.{}", s(&x["old_name"])));
                        out.insert(format!("{schema}.{}", s(&x["new_name"])));
                    }
                }
            }
            _ => {}
        }
    }
    out
}

/// Red structure findings the key lists (objects as the run names them).
fn expected_red_findings(key: &Key) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for e in key.section("must_report") {
        let object = s(&e["object"]);
        let x = &e["expected"];
        match s(&e["kind"]).as_str() {
            "schema_missing" | "sequence_behind" => {
                out.insert(object);
            }
            "rename_suggested" => {
                out.insert(match x.get("table") {
                    Some(t) => format!("{}.{}.{}", schema_of(&object), s(t), s(&x["old_name"])),
                    None => object,
                });
            }
            "index_missing" => {
                out.insert(format!("{object}.{}", s(&x["index"])));
            }
            "constraint_not_validated" => {
                out.insert(format!("{object}.{}", s(&x["constraint"])));
            }
            "large_object_missing" => {
                out.insert("database.large_objects".into());
            }
            _ => {}
        }
    }
    out
}

#[tokio::test]
async fn the_identity_run_reports_every_difference_and_stays_quiet() {
    let key = Key::load();
    let Some(r) = run_engine(&key, Mapping::new()).await else {
        return;
    };

    // m01: rows deleted from the history: counts and the exact missing keys.
    let x = key.exp("m01");
    let tx_name = key.object("m01");
    let tx = table(&r, &tx_name);
    assert_eq!(tx.verdict, Verdict::Fail, "m01");
    assert_eq!(tx.old_rows, Some(u(&x["old_count"])), "m01 old count");
    assert_eq!(tx.new_rows, Some(u(&x["new_count"])), "m01 new count");
    assert_eq!(
        tx.missing_rows,
        Some(u(&x["missing_count"]) as usize),
        "m01 missing"
    );
    let want: BTreeSet<u64> = x["missing_keys"]
        .as_array()
        .unwrap()
        .iter()
        .map(u)
        .collect();
    assert_eq!(
        int_keys(
            r.details[&tx_name]
                .missing_keys
                .as_ref()
                .expect("keys exported")
        ),
        want,
        "m01 drill-down lists exactly the missing keys"
    );
    assert!(
        tx.reasons.iter().any(|r| r.starts_with("spot check:")),
        "m01: the spot check ran and stayed consistent (not a TOOL_FAULT): {:?}",
        tx.reasons
    );

    // m02: duplicates in the keyless table, as a counted set.
    let x = key.exp("m02");
    let al = table(&r, &key.object("m02"));
    assert_eq!(al.verdict, Verdict::Fail, "m02");
    assert_eq!(al.old_rows, Some(u(&x["old_count"])));
    assert_eq!(al.new_rows, Some(u(&x["new_count"])));
    assert_eq!(
        al.extra_rows,
        Some(u(&x["extra_count"]) as usize),
        "m02 extra rows"
    );
    assert_eq!(
        x["extra_rows"].as_array().unwrap().len(),
        u(&x["extra_count"]) as usize
    );

    // m03: one balance, full values and the control total, column named.
    let x = key.exp("m03");
    let ac_name = table_of(&key.object("m03"));
    let ac = table(&r, &ac_name);
    assert_eq!(ac.verdict, Verdict::Fail, "m03");
    assert!(
        ac.reasons.iter().any(|r| r == "control totals differ"),
        "m03 control total"
    );
    assert_eq!(
        changed_columns(&r, &ac_name, key_value(x)),
        vec![s(&x["column"])],
        "m03 column"
    );
    let ct = &r.details[&ac_name].control_totals;
    let ct = ct
        .iter()
        .find(|c| {
            x["control_total"]["column"]
                .as_str()
                .unwrap()
                .ends_with(&c.column)
        })
        .expect("m03 control total exported");
    assert_eq!(
        Decimal::parse(&ct.old),
        Decimal::parse(&s(&x["control_total"]["old"])),
        "m03 old total"
    );
    assert_eq!(
        Decimal::parse(&ct.new),
        Decimal::parse(&s(&x["control_total"]["new"])),
        "m03 new total"
    );

    // m04-m06: the changed rows of the history table, each with its column;
    // exactly these rows changed.
    let mut changed_keys = BTreeSet::new();
    for id in ["m04", "m05", "m06"] {
        let x = key.exp(id);
        let t = table_of(&key.object(id));
        assert_eq!(t, tx_name, "{id} is in the history table");
        let k = key_value(x);
        changed_keys.insert(k);
        assert!(
            changed_columns(&r, &t, k).contains(&s(&x["column"])),
            "{id}: the drill-down names column {}",
            s(&x["column"])
        );
    }
    assert_eq!(
        tx.changed_rows,
        Some(changed_keys.len()),
        "exactly the planted rows changed"
    );
    let words = s(&key.exp("m05")["plain_words"]);
    assert!(
        r.details[&tx_name]
            .plain_words
            .iter()
            .any(|w| w.starts_with(&words)),
        "m05 explained in plain words: {:?}",
        r.details[&tx_name].plain_words
    );

    // m07 / m08: a dropped and an added column: structure findings, and P3
    // keeps their tables off PASS.
    let o = key.object("m07");
    assert_eq!(finding(&r, &o).verdict, Verdict::Fail, "m07 red");
    assert_eq!(
        table(&r, &table_of(&o)).verdict,
        Verdict::Fail,
        "m07: P3 fails the table"
    );
    let o = key.object("m08");
    assert_eq!(finding(&r, &o).verdict, Verdict::Warning, "m08 green/amber");
    assert_eq!(
        table(&r, &table_of(&o)).verdict,
        Verdict::NeedsReview,
        "m08: P3 review"
    );

    // m09 / m10: renames without a mapping: unpaired, with a suggested match.
    let x = key.exp("m09");
    let o = key.object("m09");
    let schema = schema_of(&o);
    assert_eq!(
        table(&r, &o).verdict,
        Verdict::Fail,
        "m09 old table missing on new"
    );
    assert_eq!(
        table(&r, &format!("{schema}.{}", s(&x["new_name"]))).verdict,
        Verdict::Warning,
        "m09 new table only on new"
    );
    assert!(
        r.suggestions
            .iter()
            .any(|g| g.path == o && g.candidate == s(&x["suggested_match"])),
        "m09 suggestion: {:?}",
        r.suggestions
    );
    let x = key.exp("m10");
    let fees = format!("{schema}.{}", s(&x["table"]));
    assert_eq!(table(&r, &fees).verdict, Verdict::Fail, "m10: P3");
    assert!(
        r.suggestions
            .iter()
            .any(|g| g.path == format!("{fees}.{}", s(&x["old_name"]))
                && g.candidate == s(&x["suggested_match"])),
        "m10 suggestion: {:?}",
        r.suggestions
    );

    // m11: varchar widened: amber, both types named; the values still pass.
    let x = key.exp("m11");
    let f = finding(&r, &key.object("m11"));
    assert_eq!(f.verdict, Verdict::Warning, "m11 amber");
    assert!(
        f.message.contains(&s(&x["old_type"])) && f.message.contains(&s(&x["new_type"])),
        "m11: {}",
        f.message
    );
    assert_eq!(
        table(&r, &table_of(&key.object("m11"))).verdict,
        Verdict::Pass,
        "m11 values"
    );

    // m12: the sequence behind its old value.
    let x = key.exp("m12");
    let f = finding(&r, &key.object("m12"));
    assert_eq!(f.verdict, Verdict::Fail, "m12");
    assert!(
        f.message.contains(&format!(
            "{} -> {}",
            u(&x["old_last_value"]),
            u(&x["new_last_value"])
        )),
        "m12: {}",
        f.message
    );

    // m13 / m14: index missing, constraint not validated.
    let x = key.exp("m13");
    let f = finding(&r, &format!("{}.{}", key.object("m13"), s(&x["index"])));
    assert_eq!(f.verdict, Verdict::Fail, "m13");
    assert!(f.message.contains("missing"), "m13: {}", f.message);
    let x = key.exp("m14");
    let f = finding(
        &r,
        &format!("{}.{}", key.object("m14"), s(&x["constraint"])),
    );
    assert_eq!(f.verdict, Verdict::Fail, "m14");
    assert!(f.message.contains("not validated"), "m14: {}", f.message);

    // m15: the large object missing.
    let x = key.exp("m15");
    let f = finding(&r, "database.large_objects");
    assert_eq!(f.verdict, Verdict::Fail, "m15");
    assert!(
        f.message
            .contains(&format!("{} -> {}", u(&x["old_count"]), u(&x["new_count"]))),
        "m15: {}",
        f.message
    );

    // q01-q03: encodings, sort order and float printing: the tables pass.
    for id in ["q01", "q02", "q03"] {
        assert_eq!(s(&key.exp(id)["verdict"]), "PASS");
        let t = table(&r, &table_of(&key.object(id)));
        assert_eq!(t.verdict, Verdict::Pass, "{id}: {:?}", t.reasons);
    }
    // q04: the partitioned table is one table with all its rows.
    let x = key.exp("q04");
    let t = table(&r, &key.object("q04"));
    assert_eq!(t.verdict, Verdict::Pass, "q04: {:?}", t.reasons);
    assert_eq!(
        (t.old_rows, t.new_rows),
        (Some(u(&x["rows"])), Some(u(&x["rows"]))),
        "q04 rows"
    );
    for p in x["new_partitions"].as_array().unwrap() {
        let name = format!("{}.{}", schema, s(p));
        assert!(
            !r.tables.contains_key(&name),
            "q04: partition {name} must not be a table"
        );
    }
    // q05: reordered columns: no finding, the table passes.
    let x = key.exp("q05");
    let o = key.object("q05");
    let t = table(&r, &o);
    assert_eq!(t.verdict, Verdict::Pass, "q05: {:?}", t.reasons);
    assert_eq!(t.new_rows, Some(u(&x["rows"])));
    assert!(
        !r.structure_findings
            .iter()
            .any(|f| f.object.starts_with(&format!("{o}."))),
        "q05: column order is not a difference"
    );
    // q06: char(1) -> varchar(1): amber type change, values pass.
    let x = key.exp("q06");
    let f = finding(&r, &key.object("q06"));
    assert_eq!(
        (s(&x["type_change"]).as_str(), f.verdict),
        ("amber", Verdict::Warning),
        "q06 amber"
    );
    assert!(
        f.message.contains(&s(&x["old_type"])) && f.message.contains(&s(&x["new_type"])),
        "q06: {}",
        f.message
    );
    let t = table(&r, &table_of(&key.object("q06")));
    assert_eq!(t.verdict, Verdict::Pass, "q06: {:?}", t.reasons);
    assert_eq!(t.new_rows, Some(u(&x["rows"])));

    // No false alarms: every other table passes...
    let non_pass = expected_non_pass(&key);
    for (name, t) in &r.tables {
        if !non_pass.contains(name) {
            assert_eq!(
                t.verdict,
                Verdict::Pass,
                "{name} must stay quiet: {:?}",
                t.reasons
            );
        }
    }
    for name in &non_pass {
        assert_ne!(
            table(&r, name).verdict,
            Verdict::Pass,
            "{name} must not pass"
        );
    }
    // ...and every red structure finding is one the key lists.
    let red: BTreeSet<String> = r
        .structure_findings
        .iter()
        .filter(|f| f.verdict == Verdict::Fail)
        .map(|f| f.object.clone())
        .collect();
    assert_eq!(red, expected_red_findings(&key), "red structure findings");

    // The run: FAIL, the coverage line counts only conclusive checks.
    assert_eq!(r.verdict, Verdict::Fail);
    assert_eq!(r.in_scope, r.tables.len() + r.ignored);
    assert_eq!(
        r.fully_checked,
        r.tables
            .values()
            .filter(|t| {
                t.reasons.iter().all(|x| {
                    x != "table missing in the new database"
                        && !x.starts_with("table exists only in the new database")
                })
            })
            .count(),
        "missing and new-only tables are not fully checked"
    );
    assert!(!r.results_sha256.is_empty());
}

#[tokio::test]
async fn the_key_mapping_pairs_the_renamed_table_and_column() {
    let key = Key::load();
    let Some(r) = run_engine(&key, key_mapping(&key)).await else {
        return;
    };
    let schema = schema_of(&key.object("m09"));
    let x = key.exp("m09");
    let old = format!("{schema}.{}", s(&x["old_name"]));
    let new = format!("{schema}.{}", s(&x["new_name"]));
    let t = table(&r, &old);
    assert_eq!(t.verdict, Verdict::Pass, "m09 mapped: {:?}", t.reasons);
    assert_eq!(t.new_rows, Some(u(&x["rows"])));
    assert!(
        !r.tables.contains_key(&new),
        "a mapped target is not a new-only table"
    );
    let x = key.exp("m10");
    let fees = format!("{schema}.{}", s(&x["table"]));
    let t = table(&r, &fees);
    assert_eq!(t.verdict, Verdict::Pass, "m10 mapped: {:?}", t.reasons);
    assert_eq!(t.new_rows, Some(u(&x["rows"])));
    assert!(!r
        .structure_findings
        .iter()
        .any(|f| f.object == old || f.object == format!("{fees}.{}", s(&x["old_name"]))));
    assert!(r.suggestions.is_empty(), "{:?}", r.suggestions);
    // The data differences are unaffected by the mapping.
    for id in ["m01", "m02", "m03"] {
        assert_eq!(
            table(&r, &table_of(&key.object(id))).verdict,
            Verdict::Fail,
            "{id}"
        );
    }
    assert_eq!(r.verdict, Verdict::Fail);
}
