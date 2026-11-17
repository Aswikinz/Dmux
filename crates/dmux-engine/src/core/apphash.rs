//! App-side hashing (PLAN.md 2.2): for a server that cannot run `md5()`
//! (FIPS mode), Dmux fetches each row's normalised values and computes the
//! very same row hash, key hash, bucket and slice sums in the app.
//!
//! The result is **bit-identical** to the server path, so one side may hash in
//! the app while the other hashes on its server, and a saved capture compares
//! with either.  Exactness rests on three things, each tested against real
//! servers in `db_tests`:
//!
//! 1. The values are the same normalised expressions the server path hashes
//!    (`TablePlan::exprs`), fetched as text over a UTF-8 client connection, which
//!    is what `convert_to(..., 'UTF8')` produces on the server.  The one
//!    expression that itself calls `md5()` (a `bytea` column, `md5("c")`) is
//!    fetched as `encode("c", 'hex')` and hashed here instead.
//! 2. [`record_text`] reproduces PostgreSQL's `record_out` for text fields:
//!    NULL prints as nothing; a value is double-quoted when it is empty or
//!    contains `"` `\` `(` `)` `,` or an ASCII white-space character, with `"`
//!    and `\` doubled inside the quotes.  This is the C-locale rule; a server
//!    whose locale classes a non-ASCII character as white space would quote it
//!    where Dmux does not.  The texts then differ, which can only ever produce
//!    a difference (FAIL), never hide one: the encoding is unambiguous either
//!    way.
//! 3. md5, the bucket (the first `d` hex digits of the key hash) and the four
//!    32-bit big-endian slices of the row hash, exactly as the SQL computes them.
//!
//! Rows are streamed, so a huge table never sits in memory.

use std::collections::{BTreeMap, HashMap, HashSet};

use futures_util::{pin_mut, TryStreamExt};
use md5::{Digest, Md5};
use tokio_postgres::types::ToSql;
use tokio_postgres::GenericClient;

use super::drilldown::RowRef;
use super::fingerprint::{
    BucketSummary, Decimal, Fingerprint, RangeFilter, Side, Source, TablePlan,
};
use super::{check_session, CoreError};

/// md5 of `data` as 16 bytes.
pub fn md5_digest(data: &[u8]) -> [u8; 16] {
    Md5::digest(data).into()
}

/// md5 of `data` as 32 lowercase hex characters, like PostgreSQL's `md5()`.
pub fn md5_hex(data: &[u8]) -> String {
    hex::encode(md5_digest(data))
}

/// PostgreSQL `record_out` for a row of text fields (see the module docs).
pub fn record_text(fields: &[Option<String>]) -> String {
    let mut out = String::from("(");
    for (i, f) in fields.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        let Some(v) = f else { continue };
        let quote = v.is_empty()
            || v.chars().any(|c| {
                matches!(
                    c,
                    '"' | '\\' | '(' | ')' | ',' | ' ' | '\t' | '\n' | '\r' | '\u{b}' | '\u{c}'
                )
            });
        if !quote {
            out.push_str(v);
            continue;
        }
        out.push('"');
        for c in v.chars() {
            if c == '"' || c == '\\' {
                out.push(c);
            }
            out.push(c);
        }
        out.push('"');
    }
    out.push(')');
    out
}

/// The bucket of a hash: its first `digits` hex digits as an integer, like
/// `('x' || substr(h, 1, d))::bit(4d)::int`.
pub fn bucket_of(digest: &[u8; 16], digits: u8) -> u32 {
    let mut b: u32 = 0;
    for i in 0..usize::from(digits) {
        let byte = digest[i / 2];
        let nibble = if i % 2 == 0 { byte >> 4 } else { byte & 0x0f };
        b = (b << 4) | u32::from(nibble);
    }
    b
}

/// The four 32-bit big-endian slices of a row hash, like
/// `('x' || substr(rh, k, 8))::bit(32)::bigint` for k = 1, 9, 17, 25.
pub fn slices(digest: &[u8; 16]) -> [u32; 4] {
    let mut s = [0u32; 4];
    for (j, slot) in s.iter_mut().enumerate() {
        *slot = u32::from_be_bytes([
            digest[4 * j],
            digest[4 * j + 1],
            digest[4 * j + 2],
            digest[4 * j + 3],
        ]);
    }
    s
}

/// The quoted column of a normalised expression that hashes it server-side
/// (`md5("c")`, the `bytea` rule), or `None` for every other expression.
fn md5_column(expr: &str) -> Option<&str> {
    let inner = expr.strip_prefix("md5(")?.strip_suffix(')')?;
    // Exactly one quoted identifier: `"..."` whose inside has only doubled
    // quotes, so `md5("a") || md5("b")` and anything else is refused.
    let body = inner.strip_prefix('"')?.strip_suffix('"')?;
    (!body.replace("\"\"", "").contains('"')).then_some(inner)
}

/// The expression fetched in app mode, and whether its value must be hashed
/// here (hex-decoded, then md5) to equal the server-path value.
fn app_expr(expr: &str) -> (String, bool) {
    match md5_column(expr) {
        Some(col) => (format!("encode({col}, 'hex')"), true),
        None => (expr.to_string(), false),
    }
}

/// How one side's values are fetched and turned back into normalised values.
struct AppColumns {
    /// The `ARRAY[...]::text[]` item.
    array_sql: String,
    /// Per column: hash this value in the app.
    hashed: Vec<bool>,
}

impl AppColumns {
    fn new(plan: &TablePlan, side: Side) -> Self {
        let (exprs, hashed): (Vec<String>, Vec<bool>) =
            plan.exprs(side).into_iter().map(app_expr).unzip();
        AppColumns {
            array_sql: format!("ARRAY[{}]::text[]", exprs.join(", ")),
            hashed,
        }
    }

    /// The normalised values the server path would have hashed.
    fn normalise(&self, raw: Vec<Option<String>>) -> Result<Vec<Option<String>>, CoreError> {
        if raw.len() != self.hashed.len() {
            return Err(CoreError::Malformed("column values"));
        }
        raw.into_iter()
            .zip(&self.hashed)
            .map(|(v, hashed)| match (v, hashed) {
                (Some(h), true) => hex::decode(h)
                    .map(|bytes| Some(md5_hex(&bytes)))
                    .map_err(|_| CoreError::Malformed("binary value")),
                (v, _) => Ok(v),
            })
            .collect()
    }
}

/// The app-mode row query: the values array, then the control-total
/// expressions as text, optionally restricted to a range.
pub fn app_rows_sql(
    plan: &TablePlan,
    side: Side,
    source: &Source,
    filter: Option<&str>,
) -> Result<String, CoreError> {
    let cols = AppColumns::new(plan, side);
    let mut items = vec![format!("{} AS v", cols.array_sql)];
    for (i, c) in plan.controls(side).iter().enumerate() {
        items.push(format!("({c})::text AS c{}", i + 1));
    }
    let from = match source {
        Source::Table { schema, name } => crate::ident::quote_qualified(schema, name)?,
        #[cfg(test)]
        Source::Derived(sql) => (*sql).to_string(),
    };
    let filter = filter.map(|f| format!(" WHERE {f}")).unwrap_or_default();
    Ok(format!("SELECT {} FROM {from}{filter}", items.join(", ")))
}

/// The hashes of one row of normalised values.
pub struct RowHashes {
    pub key: [u8; 16],
    pub row: [u8; 16],
}

/// Hash one row: the key hash (or the row hash for a table without a key).
pub fn hash_row(plan: &TablePlan, values: &[Option<String>]) -> RowHashes {
    let row = md5_digest(record_text(values).as_bytes());
    let key = if plan.is_keyed() {
        let k: Vec<Option<String>> = plan.key().iter().map(|i| values[*i].clone()).collect();
        md5_digest(record_text(&k).as_bytes())
    } else {
        row
    };
    RowHashes { key, row }
}

/// Bucket sums built row by row, exactly as the fingerprint query groups them.
pub struct Accumulator {
    digits: u8,
    buckets: BTreeMap<u32, (u64, [u128; 4])>,
    totals: Vec<Decimal>,
    rows: u64,
}

impl Accumulator {
    pub fn new(digits: u8, controls: usize) -> Self {
        Accumulator {
            digits,
            buckets: BTreeMap::new(),
            totals: vec![Decimal::ZERO; controls],
            rows: 0,
        }
    }

    /// Add one row's hashes and control values (NULLs are skipped, like `sum`).
    pub fn add(&mut self, h: &RowHashes, controls: &[Option<String>]) -> Result<(), CoreError> {
        if controls.len() != self.totals.len() {
            return Err(CoreError::Malformed("control total"));
        }
        let e = self
            .buckets
            .entry(bucket_of(&h.key, self.digits))
            .or_insert((0, [0; 4]));
        e.0 = e.0.checked_add(1).ok_or(CoreError::Overflow("row count"))?;
        for (acc, v) in e.1.iter_mut().zip(slices(&h.row)) {
            *acc = acc
                .checked_add(u128::from(v))
                .ok_or(CoreError::Overflow("hash sum"))?;
        }
        for (t, v) in self.totals.iter_mut().zip(controls) {
            if let Some(v) = v {
                let d = Decimal::parse(v).ok_or(CoreError::Malformed("control total"))?;
                *t = t
                    .checked_add(d)
                    .ok_or(CoreError::Overflow("control total"))?;
            }
        }
        self.rows = self
            .rows
            .checked_add(1)
            .ok_or(CoreError::Overflow("row count"))?;
        Ok(())
    }

    /// The bucket summaries (ascending), control totals and row count.
    pub fn finish(self) -> (Vec<BucketSummary>, Vec<Decimal>, u64) {
        let buckets = self
            .buckets
            .into_iter()
            .map(|(bucket, (n, s))| BucketSummary { bucket, n, s })
            .collect();
        (buckets, self.totals, self.rows)
    }
}

fn params(range: Option<&RangeFilter>) -> Result<(Option<String>, Vec<String>), CoreError> {
    match range {
        Some(r) => r.sql().map(|(f, p)| (Some(f), p)),
        None => Ok((None, Vec::new())),
    }
}

/// Stream the app-mode rows of one side, calling `each` with the normalised
/// values and the control values of every row.
async fn stream_rows<F>(
    client: &impl GenericClient,
    plan: &TablePlan,
    side: Side,
    source: &Source,
    range: Option<&RangeFilter>,
    mut each: F,
) -> Result<(), CoreError>
where
    F: FnMut(Vec<Option<String>>, Vec<Option<String>>) -> Result<(), CoreError>,
{
    let (filter, values) = params(range)?;
    let sql = app_rows_sql(plan, side, source, filter.as_deref())?;
    let cols = AppColumns::new(plan, side);
    let controls = plan.controls(side).len();
    let refs: Vec<&(dyn ToSql + Sync)> = values.iter().map(|p| p as &(dyn ToSql + Sync)).collect();
    check_session(client).await?;
    let stream = client.query_raw(sql.as_str(), refs).await?;
    pin_mut!(stream);
    while let Some(row) = stream.try_next().await? {
        let raw: Vec<Option<String>> = row.try_get(0)?;
        let c: Vec<Option<String>> = (0..controls)
            .map(|i| row.try_get::<_, Option<String>>(1 + i))
            .collect::<Result<_, _>>()?;
        each(cols.normalise(raw)?, c)?;
    }
    Ok(())
}

/// Scan one side, or one range of it, hashing in the app.  Same result as
/// [`super::fingerprint::fingerprint`] / [`super::fingerprint::fingerprint_range`].
pub async fn fingerprint_app(
    client: &impl GenericClient,
    plan: &TablePlan,
    side: Side,
    source: &Source,
    range: Option<&RangeFilter>,
    mapping_hash: &str,
) -> Result<Fingerprint, CoreError> {
    let mut acc = Accumulator::new(plan.bucket_digits(), plan.control_count());
    stream_rows(client, plan, side, source, range, |values, controls| {
        acc.add(&hash_row(plan, &values), &controls)
    })
    .await?;
    let (buckets, control_totals, rows) = acc.finish();
    Ok(Fingerprint {
        bucket_digits: plan.bucket_digits(),
        keyed: plan.is_keyed(),
        mapping_hash: mapping_hash.to_string(),
        rows,
        buckets,
        control_totals,
        truncated: false,
        needs_review: plan.needs_review(),
    })
}

/// The drill-down rows of `buckets` on one side, hashing in the app.  Same
/// result as [`super::drilldown::fetch_rows`].
pub async fn fetch_rows_app(
    client: &impl GenericClient,
    plan: &TablePlan,
    side: Side,
    source: &Source,
    buckets: &[u32],
) -> Result<Vec<RowRef>, CoreError> {
    let wanted: HashSet<u32> = buckets.iter().copied().collect();
    let mut out = Vec::new();
    stream_rows(client, plan, side, source, None, |values, _| {
        let h = hash_row(plan, &values);
        let bucket = bucket_of(&h.key, plan.bucket_digits());
        if wanted.contains(&bucket) {
            let key = if plan.is_keyed() {
                plan.key().iter().map(|i| values[*i].clone()).collect()
            } else {
                Vec::new()
            };
            out.push(RowRef {
                bucket,
                key_hash: hex::encode(h.key),
                key,
                row_hash: hex::encode(h.row),
            });
        }
        Ok(())
    })
    .await?;
    Ok(out)
}

/// The normalised values of the rows whose key hash is in `hashes`, hashing
/// in the app.  Same result as the server-side value query.
pub async fn values_by_key_hash_app(
    client: &impl GenericClient,
    plan: &TablePlan,
    side: Side,
    source: &Source,
    hashes: &[String],
) -> Result<HashMap<String, Vec<Option<String>>>, CoreError> {
    if !plan.is_keyed() {
        return Err(CoreError::Plan("column detail needs a key"));
    }
    let wanted: HashSet<&str> = hashes.iter().map(String::as_str).collect();
    let mut out = HashMap::new();
    stream_rows(client, plan, side, source, None, |values, _| {
        let kh = hex::encode(hash_row(plan, &values).key);
        if wanted.contains(kh.as_str()) {
            out.insert(kh, values);
        }
        Ok(())
    })
    .await?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::fingerprint::tests::accounts_plan;
    use crate::core::model;
    use proptest::prelude::*;

    #[test]
    fn md5_matches_known_vectors_and_the_model() {
        assert_eq!(md5_hex(b""), "d41d8cd98f00b204e9800998ecf8427e");
        assert_eq!(md5_hex(b"abc"), "900150983cd24fb0d6963f7d28e17f72");
        for s in ["", "(1,abc)", "é", "\u{0}x"] {
            assert_eq!(md5_hex(s.as_bytes()), model::md5_hex(s.as_bytes()));
        }
    }

    #[test]
    fn record_text_quotes_like_record_out() {
        let r = |v: &[Option<&str>]| {
            record_text(&v.iter().map(|x| x.map(str::to_string)).collect::<Vec<_>>())
        };
        assert_eq!(r(&[Some("a"), None, Some("b")]), "(a,,b)");
        assert_eq!(r(&[Some("")]), "(\"\")");
        assert_eq!(r(&[Some("a b")]), "(\"a b\")");
        assert_eq!(r(&[Some("x\"y\\z")]), "(\"x\"\"y\\\\z\")");
        assert_eq!(r(&[Some("(,)")]), "(\"(,)\")");
        assert_eq!(r(&[Some("é")]), "(é)");
        assert_eq!(r(&[]), "()");
        assert_eq!(r(&[None]), "()");
    }

    #[test]
    fn bucket_and_slices_follow_the_sql() {
        let d: [u8; 16] = [
            0xab, 0xcd, 0xef, 0x01, 0x23, 0x45, 0x67, 0x89, 0, 0, 0, 1, 0xff, 0xff, 0xff, 0xff,
        ];
        assert_eq!(bucket_of(&d, 1), 0xa);
        assert_eq!(bucket_of(&d, 3), 0xabc);
        assert_eq!(bucket_of(&d, 4), 0xabcd);
        assert_eq!(bucket_of(&d, 7), 0xabcdef0);
        assert_eq!(slices(&d), [0xabcdef01, 0x23456789, 1, 0xffff_ffff]);
    }

    #[test]
    fn only_the_bytea_rule_is_hashed_in_the_app() {
        assert_eq!(
            app_expr("md5(\"doc\")"),
            ("encode(\"doc\", 'hex')".to_string(), true)
        );
        assert_eq!(
            app_expr("md5(\"we\"\"ird\")"),
            ("encode(\"we\"\"ird\", 'hex')".to_string(), true)
        );
        for e in [
            "\"c\"::text",
            "encode(float8send(\"c\"), 'hex')",
            "coalesce(\"c\"::text, '')",
            "md5(x)",
            "md5(\"a\") || md5(\"b\")",
            "md5(\")",
        ] {
            assert!(!app_expr(e).1, "{e}");
        }
        // The planner emits md5(...) for bytea and nothing else.
        let plan = accounts_plan();
        let hashed: Vec<bool> = AppColumns::new(&plan, Side::Old).hashed;
        let names: Vec<&str> = crate::core::fingerprint::tests::ACCOUNTS
            .iter()
            .map(|(n, _)| *n)
            .collect();
        for (n, h) in names.iter().zip(hashed) {
            assert_eq!(h, *n == "doc_blob", "{n}");
        }
    }

    #[test]
    fn normalise_hashes_binary_values_and_keeps_nulls() {
        let cols = AppColumns {
            array_sql: String::new(),
            hashed: vec![false, true, true],
        };
        let out = cols
            .normalise(vec![Some("x".into()), Some("616263".into()), None])
            .unwrap();
        assert_eq!(
            out,
            vec![
                Some("x".into()),
                Some("900150983cd24fb0d6963f7d28e17f72".into()),
                None
            ]
        );
        assert!(cols.normalise(vec![None]).is_err());
        assert!(cols.normalise(vec![None, Some("zz".into()), None]).is_err());
    }

    #[test]
    fn app_rows_sql_shape() {
        let plan = accounts_plan();
        let src = Source::Table {
            schema: "demo_bank".into(),
            name: "accounts".into(),
        };
        let sql = app_rows_sql(&plan, Side::Old, &src, None).unwrap();
        assert!(sql.starts_with("SELECT ARRAY["), "{sql}");
        assert!(sql.contains("encode(\"doc_blob\", 'hex')"), "{sql}");
        assert!(
            !sql.contains("md5("),
            "no server-side md5 in app mode: {sql}"
        );
        assert!(sql.ends_with("FROM \"demo_bank\".\"accounts\""), "{sql}");
        let filtered = app_rows_sql(&plan, Side::Old, &src, Some("\"a\" IS NULL")).unwrap();
        assert!(filtered.ends_with(" WHERE \"a\" IS NULL"), "{filtered}");
    }

    proptest! {
        /// The accumulator gives exactly the model's summaries (the model is the
        /// independent oracle the database tests already check against Postgres).
        #[test]
        fn accumulator_matches_the_model(
            rows in proptest::collection::vec(
                proptest::collection::vec(proptest::option::of("[ -~é,()\"\\\\]{0,6}"), 3),
                0..60,
            ),
            keyed in any::<bool>(),
            digits in 1u8..=4,
        ) {
            let cols: Vec<_> = (0..3)
                .map(|i| crate::core::normalise::plan_column(
                    &crate::core::fingerprint::tests::mapping(&format!("c{i}"), "text"),
                ).unwrap())
                .collect();
            let key = if keyed { vec![0] } else { Vec::new() };
            let plan = TablePlan::new(cols, key.clone(), digits).unwrap();
            let mut acc = Accumulator::new(digits, 0);
            for r in &rows {
                acc.add(&hash_row(&plan, r), &[]).unwrap();
            }
            let (buckets, _, n) = acc.finish();
            prop_assert_eq!(n, rows.len() as u64);
            prop_assert_eq!(buckets, model::summarise(&rows, &key, digits));
        }
    }

    #[test]
    fn controls_are_summed_exactly_and_nulls_skipped() {
        let mut acc = Accumulator::new(3, 1);
        let h = RowHashes {
            key: [0; 16],
            row: [0; 16],
        };
        acc.add(&h, &[Some("1.10".into())]).unwrap();
        acc.add(&h, &[None]).unwrap();
        acc.add(&h, &[Some("-0.1".into())]).unwrap();
        let (_, totals, rows) = acc.finish();
        assert_eq!(rows, 3);
        assert_eq!(totals, vec![Decimal::parse("1").unwrap()]);
        let mut bad = Accumulator::new(3, 1);
        assert!(bad.add(&h, &[Some("NaN".into())]).is_err());
        assert!(bad.add(&h, &[]).is_err());
    }
}
