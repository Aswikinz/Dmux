//! The fingerprint query (PLAN.md 3.1) in the exact shape of
//! `testdata/phase0/fingerprint.sql` and `fingerprint_nokey.sql`, and exact
//! parsing of its output.

use super::normalise::ColumnPlan;
use super::{check_session, text_rows, CoreError};
use crate::ident::quote_qualified;
use serde::{Deserialize, Serialize};
use tokio_postgres::GenericClient;

pub const DEFAULT_BUCKET_DIGITS: u8 = 3;
pub const MAX_BUCKET_DIGITS: u8 = 7;
pub const TARGET_ROWS_PER_BUCKET: u64 = 10_000;
const SLICE_MAX: u128 = 0xffff_ffff;

/// Hex digits of the key hash used as the bucket: 3 (4,096 buckets) by
/// default, more for huge tables so buckets stay near 10,000 rows.
pub fn bucket_digits_for(estimated_rows: u64) -> u8 {
    let mut d = DEFAULT_BUCKET_DIGITS;
    while d < MAX_BUCKET_DIGITS && estimated_rows > TARGET_ROWS_PER_BUCKET << (4 * d) {
        d += 1;
    }
    d
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Old,
    New,
}

/// What one side scans.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    Table {
        schema: String,
        name: String,
    },
    /// Test only: a fixed derived relation (with alias) written in the test source.
    #[cfg(test)]
    Derived(&'static str),
}

impl Source {
    fn sql_from_item(&self) -> Result<String, CoreError> {
        match self {
            Source::Table { schema, name } => Ok(quote_qualified(schema, name)?),
            #[cfg(test)]
            Source::Derived(sql) => Ok((*sql).to_string()),
        }
    }
}

/// The mapped columns of one table in mapping order, the key column indexes
/// (empty for a table without a key) and the bucket width.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TablePlan {
    columns: Vec<ColumnPlan>,
    key: Vec<usize>,
    bucket_digits: u8,
}

impl TablePlan {
    pub fn new(
        columns: Vec<ColumnPlan>,
        key: Vec<usize>,
        bucket_digits: u8,
    ) -> Result<Self, CoreError> {
        if columns.is_empty() {
            return Err(CoreError::Plan("no mapped columns"));
        }
        if !(1..=MAX_BUCKET_DIGITS).contains(&bucket_digits) {
            return Err(CoreError::Plan("bucket digits out of range"));
        }
        for (i, k) in key.iter().enumerate() {
            if *k >= columns.len() || key[..i].contains(k) {
                return Err(CoreError::Plan("bad key column index"));
            }
        }
        Ok(TablePlan {
            columns,
            key,
            bucket_digits,
        })
    }

    pub fn columns(&self) -> &[ColumnPlan] {
        &self.columns
    }

    pub fn bucket_digits(&self) -> u8 {
        self.bucket_digits
    }

    pub fn is_keyed(&self) -> bool {
        !self.key.is_empty()
    }

    pub(crate) fn exprs(&self, side: Side) -> Vec<&str> {
        self.columns
            .iter()
            .map(|c| match side {
                Side::Old => c.old_expr.as_str(),
                Side::New => c.new_expr.as_str(),
            })
            .collect()
    }

    pub(crate) fn key_exprs(&self, side: Side) -> Vec<&str> {
        let all = self.exprs(side);
        self.key.iter().map(|k| all[*k]).collect()
    }

    fn controls(&self, side: Side) -> Vec<&str> {
        self.columns
            .iter()
            .filter_map(|c| match side {
                Side::Old => c.old_control.as_deref(),
                Side::New => c.new_control.as_deref(),
            })
            .collect()
    }

    /// Some column is compared by a printed form that is not provably exact.
    pub fn needs_review(&self) -> bool {
        self.columns.iter().any(ColumnPlan::needs_review)
    }

    pub fn control_count(&self) -> usize {
        self.controls(Side::Old).len()
    }

    pub(crate) fn bucket_expr(&self, hash_col: &str) -> String {
        let d = self.bucket_digits;
        format!("('x' || substr({hash_col}, 1, {d}))::bit({})::int", 4 * d)
    }

    /// Inner select: `kh` (keyed only), `rh`, then any extra items.
    pub(crate) fn hashed_select(
        &self,
        side: Side,
        source: &Source,
        extras: &[String],
    ) -> Result<String, CoreError> {
        let mut items = Vec::new();
        if self.is_keyed() {
            items.push(format!("{} AS kh", row_hash(&self.key_exprs(side))));
        }
        items.push(format!("{} AS rh", row_hash(&self.exprs(side))));
        items.extend(extras.iter().cloned());
        Ok(format!(
            "SELECT {} FROM {}",
            items.join(", "),
            source.sql_from_item()?
        ))
    }

    /// The fingerprint query for one side.
    pub fn fingerprint_sql(&self, side: Side, source: &Source) -> Result<String, CoreError> {
        let controls = self.controls(side);
        let inner_extra: Vec<String> = controls
            .iter()
            .enumerate()
            .map(|(i, c)| format!("{c} AS c{}", i + 1))
            .collect();
        let inner = self.hashed_select(side, source, &inner_extra)?;
        let bucket = self.bucket_expr(if self.is_keyed() { "kh" } else { "rh" });
        let mut items = vec![format!("{bucket} AS bucket"), "count(*) AS n".to_string()];
        for (i, start) in [1, 9, 17, 25].iter().enumerate() {
            items.push(format!(
                "sum(('x' || substr(rh, {start}, 8))::bit(32)::bigint) AS s{}",
                i + 1
            ));
        }
        for i in 1..=controls.len() {
            items.push(format!("sum(c{i}) AS c{i}"));
        }
        Ok(format!(
            "SELECT {} FROM ({inner}) q GROUP BY 1 ORDER BY 1",
            items.join(", ")
        ))
    }
}

fn row_hash(exprs: &[&str]) -> String {
    format!("md5(convert_to(ROW({})::text, 'UTF8'))", exprs.join(", "))
}

/// One bucket: row count and the four 32-bit slice sums of its row hashes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct BucketSummary {
    pub bucket: u32,
    pub n: u64,
    pub s: [u128; 4],
}

/// An exact decimal for control totals. Equality is by value (1.10 == 1.1).
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Decimal {
    mantissa: i128,
    scale: u32,
}

const MAX_DECIMAL_SCALE: u32 = 38;

impl Decimal {
    pub const ZERO: Decimal = Decimal {
        mantissa: 0,
        scale: 0,
    };

    /// Parse Postgres numeric output (`-123.4500`). NaN and anything else is rejected.
    pub fn parse(s: &str) -> Option<Decimal> {
        let (neg, body) = match s.strip_prefix('-') {
            Some(b) => (true, b),
            None => (false, s),
        };
        let (int, frac) = match body.split_once('.') {
            Some((i, f)) if !f.is_empty() => (i, f),
            Some(_) => return None,
            None => (body, ""),
        };
        let digits = |p: &str| p.bytes().all(|b| b.is_ascii_digit());
        if int.is_empty() || !digits(int) || !digits(frac) {
            return None;
        }
        let scale = u32::try_from(frac.len()).ok()?;
        if scale > MAX_DECIMAL_SCALE {
            return None;
        }
        let mut m: i128 = format!("{int}{frac}").parse().ok()?;
        if neg {
            m = -m;
        }
        Some(Decimal { mantissa: m, scale })
    }

    pub fn checked_add(self, o: Decimal) -> Option<Decimal> {
        let scale = self.scale.max(o.scale);
        let a = self
            .mantissa
            .checked_mul(10i128.checked_pow(scale - self.scale)?)?;
        let b = o
            .mantissa
            .checked_mul(10i128.checked_pow(scale - o.scale)?)?;
        Some(Decimal {
            mantissa: a.checked_add(b)?,
            scale,
        })
    }

    fn reduced(self) -> (i128, u32) {
        let (mut m, mut s) = (self.mantissa, self.scale);
        while s > 0 && m % 10 == 0 {
            m /= 10;
            s -= 1;
        }
        (m, s)
    }
}

impl PartialEq for Decimal {
    fn eq(&self, other: &Self) -> bool {
        self.reduced() == other.reduced()
    }
}
impl Eq for Decimal {}

impl std::fmt::Display for Decimal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let digits = self.mantissa.unsigned_abs().to_string();
        let sign = if self.mantissa < 0 { "-" } else { "" };
        let s = self.scale as usize;
        if s == 0 {
            return write!(f, "{sign}{digits}");
        }
        let padded = format!("{digits:0>width$}", width = s + 1);
        let (i, frac) = padded.split_at(padded.len() - s);
        write!(f, "{sign}{i}.{frac}")
    }
}

/// One side's fingerprint of one table.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Fingerprint {
    pub bucket_digits: u8,
    pub keyed: bool,
    /// Hash of the mapping this was built with; both sides must match.
    pub mapping_hash: String,
    pub rows: u64,
    pub buckets: Vec<BucketSummary>,
    pub control_totals: Vec<Decimal>,
    /// Set when the bucket list is known to be incomplete; never PASSes.
    pub truncated: bool,
    /// Some column needs a person's review; never a clean PASS.
    pub needs_review: bool,
}

/// Strict unsigned integer: ASCII digits only, no sign, no spaces.
pub fn parse_uint(s: &str) -> Option<u128> {
    if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    s.parse().ok()
}

/// Parse the text rows of a fingerprint query. Rejects anything malformed.
pub fn parse_summary(
    rows: &[Vec<Option<String>>],
    bucket_digits: u8,
    controls: usize,
) -> Result<(Vec<BucketSummary>, Vec<Decimal>, u64), CoreError> {
    let bucket_limit = 1u128 << (4 * u32::from(bucket_digits));
    let mut buckets: Vec<BucketSummary> = Vec::with_capacity(rows.len());
    let mut totals = vec![Decimal::ZERO; controls];
    let mut total_rows: u64 = 0;
    for row in rows {
        if row.len() != 6 + controls {
            return Err(CoreError::Malformed("column count"));
        }
        let num = |i: usize| row[i].as_deref().and_then(parse_uint);
        let bucket = num(0)
            .filter(|b| *b < bucket_limit)
            .ok_or(CoreError::Malformed("bucket"))?;
        let n = num(1)
            .and_then(|n| u64::try_from(n).ok())
            .filter(|n| *n > 0)
            .ok_or(CoreError::Malformed("row count"))?;
        let mut s = [0u128; 4];
        for (j, slot) in s.iter_mut().enumerate() {
            *slot = num(2 + j)
                .filter(|v| *v <= u128::from(n) * SLICE_MAX)
                .ok_or(CoreError::Malformed("hash sum"))?;
        }
        let bucket = bucket as u32;
        if buckets.last().is_some_and(|b| b.bucket >= bucket) {
            return Err(CoreError::Malformed("bucket order"));
        }
        for (j, t) in totals.iter_mut().enumerate() {
            if let Some(v) = row[6 + j].as_deref() {
                let d = Decimal::parse(v).ok_or(CoreError::Malformed("control total"))?;
                *t = t
                    .checked_add(d)
                    .ok_or(CoreError::Overflow("control total"))?;
            }
        }
        total_rows = total_rows
            .checked_add(n)
            .ok_or(CoreError::Overflow("row count"))?;
        buckets.push(BucketSummary { bucket, n, s });
    }
    Ok((buckets, totals, total_rows))
}

/// Scan one side and return its fingerprint. The client must be inside the
/// Dmux read-only session; this is checked first.
pub async fn fingerprint(
    client: &impl GenericClient,
    plan: &TablePlan,
    side: Side,
    source: &Source,
    mapping_hash: &str,
) -> Result<Fingerprint, CoreError> {
    let sql = plan.fingerprint_sql(side, source)?;
    check_session(client).await?;
    let rows = text_rows(client, &sql).await?;
    let (buckets, control_totals, total) =
        parse_summary(&rows, plan.bucket_digits, plan.control_count())?;
    Ok(Fingerprint {
        bucket_digits: plan.bucket_digits,
        keyed: plan.is_keyed(),
        mapping_hash: mapping_hash.to_string(),
        rows: total,
        buckets,
        control_totals,
        truncated: false,
        needs_review: plan.needs_review(),
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::core::normalise::{plan_column, ColumnMapping, ColumnOptions, PgType};
    use proptest::prelude::*;

    pub(crate) fn mapping(name: &str, ty: &str) -> ColumnMapping {
        ColumnMapping {
            old_name: name.into(),
            new_name: name.into(),
            old_type: PgType::parse(ty),
            new_type: PgType::parse(ty),
            options: ColumnOptions::default(),
            control_total: false,
        }
    }

    pub(crate) const ACCOUNTS: &[(&str, &str)] = &[
        ("acct_id", "character varying(20)"),
        ("bank_code", "character varying(10)"),
        ("holder_name", "character varying(80)"),
        ("branch_code", "character(8)"),
        ("balance", "numeric(18,4)"),
        ("rate", "double precision"),
        ("opened_at", "timestamp(0) without time zone"),
        ("doc_blob", "bytea"),
        ("remarks", "text"),
    ];

    pub(crate) fn accounts_plan() -> TablePlan {
        let cols = ACCOUNTS
            .iter()
            .map(|(n, t)| plan_column(&mapping(n, t)).unwrap())
            .collect();
        TablePlan::new(cols, vec![0, 1], DEFAULT_BUCKET_DIGITS).unwrap()
    }

    /// The SELECT of a reference file, comments removed, whitespace collapsed.
    pub(crate) fn reference_select(file: &str) -> String {
        let path = format!(
            "{}/../../testdata/phase0/{file}",
            env!("CARGO_MANIFEST_DIR")
        );
        let text = std::fs::read_to_string(path).unwrap();
        let body: String = text
            .lines()
            .map(|l| l.split("--").next().unwrap())
            .collect::<Vec<_>>()
            .join(" ");
        let start = body.find("SELECT").unwrap();
        let end = body[start..].find(';').unwrap() + start;
        canon(&body[start..end])
    }

    pub(crate) fn canon(sql: &str) -> String {
        let s = sql.split_whitespace().collect::<Vec<_>>().join(" ");
        s.replace("( ", "(").replace(" )", ")")
    }

    #[test]
    fn keyed_sql_matches_the_reference_shape() {
        let sql = accounts_plan()
            .fingerprint_sql(
                Side::Old,
                &Source::Table {
                    schema: "phase0".into(),
                    name: "accounts".into(),
                },
            )
            .unwrap();
        // Our identifiers are always quoted; the reference's are plain lowercase.
        assert_eq!(
            canon(&sql.replace('"', "")),
            reference_select("fingerprint.sql")
        );
    }

    #[test]
    fn keyless_sql_matches_the_reference_shape() {
        let cols = [
            ("event_code", "character varying(10)"),
            ("event_text", "text"),
        ]
        .iter()
        .map(|(n, t)| plan_column(&mapping(n, t)).unwrap())
        .collect();
        let plan = TablePlan::new(cols, vec![], 3).unwrap();
        assert!(!plan.is_keyed());
        let src = Source::Table {
            schema: "phase0".into(),
            name: "audit_log".into(),
        };
        let sql = plan.fingerprint_sql(Side::New, &src).unwrap();
        assert_eq!(
            canon(&sql.replace('"', "")),
            reference_select("fingerprint_nokey.sql")
        );
    }

    #[test]
    fn numscale_and_money_match_their_references() {
        let mut amt = mapping("amt", "numeric(20,4)");
        amt.new_type = PgType::parse("numeric(20,6)");
        let mut cols = vec![
            plan_column(&mapping("id", "integer")).unwrap(),
            plan_column(&amt).unwrap(),
        ];
        let src = |t: &str| Source::Table {
            schema: "phase0".into(),
            name: t.into(),
        };
        // Reference casts to numeric(20,6); we keep all integer digits: (22,6).
        let plan = TablePlan::new(cols.clone(), vec![0], 3).unwrap();
        let sql = plan.fingerprint_sql(Side::Old, &src("num_test")).unwrap();
        assert_eq!(
            canon(
                &sql.replace('"', "")
                    .replace("numeric(22,6)", "numeric(20,6)")
            ),
            reference_select("fingerprint_numscale.sql")
        );
        cols[1] = plan_column(&mapping("amt", "money")).unwrap();
        let plan = TablePlan::new(cols, vec![0], 3).unwrap();
        let sql = plan.fingerprint_sql(Side::New, &src("money_test")).unwrap();
        assert_eq!(
            canon(&sql.replace('"', "")),
            reference_select("fingerprint_money.sql")
        );
    }

    #[test]
    fn control_totals_and_digits_in_sql() {
        let mut m = mapping("amt", "numeric(20,2)");
        m.control_total = true;
        let plan = TablePlan::new(vec![plan_column(&m).unwrap()], vec![0], 4).unwrap();
        assert_eq!(plan.control_count(), 1);
        let sql = plan
            .fingerprint_sql(
                Side::Old,
                &Source::Table {
                    schema: "s".into(),
                    name: "t".into(),
                },
            )
            .unwrap();
        assert!(sql.contains("\"amt\"::numeric AS c1"));
        assert!(sql.contains("sum(c1) AS c1"));
        assert!(sql.contains("substr(kh, 1, 4))::bit(16)::int"));
        let bad = Source::Table {
            schema: String::new(),
            name: "t".into(),
        };
        assert!(plan.fingerprint_sql(Side::Old, &bad).is_err());
    }

    #[test]
    fn plan_validation() {
        let c = plan_column(&mapping("a", "text")).unwrap();
        assert!(TablePlan::new(vec![], vec![], 3).is_err());
        assert!(TablePlan::new(vec![c.clone()], vec![], 0).is_err());
        assert!(TablePlan::new(vec![c.clone()], vec![], 8).is_err());
        assert!(TablePlan::new(vec![c.clone()], vec![1], 3).is_err());
        assert!(TablePlan::new(vec![c.clone(), c.clone()], vec![0, 0], 3).is_err());
        let p = TablePlan::new(vec![c], vec![], 7).unwrap();
        assert_eq!(p.bucket_digits(), 7);
        assert_eq!(p.columns().len(), 1);
    }

    #[test]
    fn bucket_sizing() {
        assert_eq!(bucket_digits_for(0), 3);
        assert_eq!(bucket_digits_for(40_960_000), 3);
        assert_eq!(bucket_digits_for(40_960_001), 4);
        assert_eq!(bucket_digits_for(u64::MAX), MAX_BUCKET_DIGITS);
    }

    fn row(v: &[&str]) -> Vec<Option<String>> {
        v.iter().map(|s| Some(s.to_string())).collect()
    }

    #[test]
    fn parses_summaries_and_rejects_malformed() {
        let rows = vec![
            row(&["1", "2", "3", "4", "5", "8589934590"]),
            row(&["4095", "1", "0", "0", "0", "0"]),
        ];
        let (b, _, n) = parse_summary(&rows, 3, 0).unwrap();
        assert_eq!(n, 3);
        assert_eq!(b[0].s, [3, 4, 5, 8_589_934_590]);
        let bad: &[&[&str]] = &[
            &["4096", "1", "0", "0", "0", "0"],
            &["-1", "1", "0", "0", "0", "0"],
            &["1", "0", "0", "0", "0", "0"],
            &["1", "1", "4294967296", "0", "0", "0"],
            &["1", "1", "1.0", "0", "0", "0"],
            &["1", "1", " 1", "0", "0", "0"],
            &["1", "1", "+1", "0", "0", "0"],
            &["1", "1", "", "0", "0", "0"],
            &["1", "1", "1e3", "0", "0", "0"],
            &["1", "1", "0", "0", "0"],
            &["1", "99999999999999999999", "0", "0", "0", "0"],
        ];
        for r in bad {
            assert!(parse_summary(&[row(r)], 3, 0).is_err(), "{r:?}");
        }
        let mut nulls = row(&["1", "1", "0", "0", "0", "0"]);
        nulls[3] = None;
        assert!(parse_summary(&[nulls], 3, 0).is_err());
        let unordered = vec![
            row(&["2", "1", "0", "0", "0", "0"]),
            row(&["2", "1", "0", "0", "0", "0"]),
        ];
        assert_eq!(
            parse_summary(&unordered, 3, 0),
            Err(CoreError::Malformed("bucket order"))
        );
    }

    #[test]
    fn control_totals_parse_exactly() {
        let mut r1 = row(&["1", "1", "0", "0", "0", "0", "1.10"]);
        let r2 = row(&["2", "1", "0", "0", "0", "0", "-0.05"]);
        let (_, t, _) = parse_summary(&[r1.clone(), r2], 3, 1).unwrap();
        assert_eq!(t[0], Decimal::parse("1.05").unwrap());
        assert_eq!(t[0].to_string(), "1.05");
        r1[6] = None;
        let (_, t, _) = parse_summary(&[r1.clone()], 3, 1).unwrap();
        assert_eq!(t[0], Decimal::ZERO);
        r1[6] = Some("NaN".into());
        assert!(parse_summary(&[r1.clone()], 3, 1).is_err());
        let big = "9".repeat(38);
        r1[6] = Some(big.clone());
        let r3 = row(&["2", "1", "0", "0", "0", "0", &big]);
        assert_eq!(
            parse_summary(&[r1, r3], 3, 1),
            Err(CoreError::Overflow("control total"))
        );
    }

    #[test]
    fn decimal_parse_rules() {
        for bad in [
            "", "-", "1.", ".5", "1.2.3", "a", "--1", "1,0", "NaN", "Infinity",
        ] {
            assert!(Decimal::parse(bad).is_none(), "{bad}");
        }
        assert!(Decimal::parse(&format!("0.{}", "1".repeat(39))).is_none());
        assert_eq!(Decimal::parse("1.10"), Decimal::parse("1.1"));
        assert_ne!(Decimal::parse("1.10"), Decimal::parse("1.11"));
        assert_eq!(Decimal::parse("-0.005").unwrap().to_string(), "-0.005");
        assert_eq!(Decimal::parse("42").unwrap().to_string(), "42");
        assert_eq!(Decimal::ZERO.to_string(), "0");
        assert_eq!(Decimal::parse("0.00").unwrap().to_string(), "0.00");
        assert_eq!(Decimal::parse("-0.00").unwrap().to_string(), "0.00");
        let tiny = Decimal::parse(&format!("0.{}1", "0".repeat(37))).unwrap();
        assert!(tiny
            .checked_add(Decimal::parse(&"9".repeat(38)).unwrap())
            .is_none());
    }

    #[test]
    fn fingerprint_serialises_exactly() {
        let fp = Fingerprint {
            bucket_digits: 3,
            keyed: true,
            mapping_hash: "m".into(),
            rows: u64::MAX,
            buckets: vec![BucketSummary {
                bucket: 7,
                n: u64::MAX,
                s: [u128::from(u64::MAX) * SLICE_MAX, 0, 1, 2],
            }],
            control_totals: vec![Decimal::parse("-12.50").unwrap()],
            truncated: false,
            needs_review: true,
        };
        let text = serde_json::to_string(&fp).unwrap();
        let back: Fingerprint = serde_json::from_str(&text).unwrap();
        assert_eq!(back, fp);
        assert_eq!(back.control_totals[0].to_string(), "-12.50");
    }

    proptest! {
        #[test]
        fn sum_parsing_round_trips(n in 1u64..=u64::MAX, f in 0.0f64..=1.0) {
            let max = u128::from(n) * SLICE_MAX;
            let v = ((max as f64) * f) as u128;
            let v = v.min(max);
            let r = row(&["0", &n.to_string(), &v.to_string(), "0", "0", &max.to_string()]);
            let (b, _, total) = parse_summary(&[r], 3, 0).unwrap();
            prop_assert_eq!(b[0].s[0], v);
            prop_assert_eq!(b[0].s[3], max);
            prop_assert_eq!(total, n);
            prop_assert_eq!(parse_uint(&format!("{v}x")), None);
        }

        #[test]
        fn decimal_round_trips(m in any::<i64>(), scale in 0u32..12) {
            let d = Decimal { mantissa: i128::from(m), scale };
            prop_assert_eq!(Decimal::parse(&d.to_string()), Some(d));
        }
    }
}
