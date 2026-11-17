//! The fingerprint query (PLAN.md 3.1) in the exact shape of
//! `testdata/phase0/fingerprint.sql` and `fingerprint_nokey.sql`, and exact
//! parsing of its output.

use super::normalise::{ColumnPlan, PgType};
use super::{check_session, text_rows, CoreError};
use crate::ident::{quote_ident, quote_qualified};
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

    /// The key column indexes (empty for a table without a key).
    pub(crate) fn key(&self) -> &[usize] {
        &self.key
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

    pub(crate) fn controls(&self, side: Side) -> Vec<&str> {
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
        self.hashed_select_where(side, source, extras, None)
    }

    /// [`Self::hashed_select`] restricted by a range predicate (`WHERE ...`).
    pub(crate) fn hashed_select_where(
        &self,
        side: Side,
        source: &Source,
        extras: &[String],
        filter: Option<&str>,
    ) -> Result<String, CoreError> {
        let mut items = Vec::new();
        if self.is_keyed() {
            items.push(format!("{} AS kh", row_hash(&self.key_exprs(side))));
        }
        items.push(format!("{} AS rh", row_hash(&self.exprs(side))));
        items.extend(extras.iter().cloned());
        let filter = filter.map(|f| format!(" WHERE {f}")).unwrap_or_default();
        Ok(format!(
            "SELECT {} FROM {}{filter}",
            items.join(", "),
            source.sql_from_item()?
        ))
    }

    /// The fingerprint query for one side.
    pub fn fingerprint_sql(&self, side: Side, source: &Source) -> Result<String, CoreError> {
        self.fingerprint_sql_where(side, source, None)
    }

    /// The fingerprint query restricted to one range of rows, with every output
    /// column cast to text so it can run with bound parameters (the extended
    /// protocol) and parse exactly like the unrestricted query.
    pub fn fingerprint_sql_range(
        &self,
        side: Side,
        source: &Source,
        filter: &str,
    ) -> Result<String, CoreError> {
        let inner = self.fingerprint_sql_where(side, source, Some(filter))?;
        let mut cols = vec!["bucket::text".to_string(), "n::text".to_string()];
        cols.extend((1..=4).map(|i| format!("s{i}::text")));
        cols.extend((1..=self.control_count()).map(|i| format!("c{i}::text")));
        Ok(format!(
            "SELECT {} FROM ({inner}) f ORDER BY f.bucket",
            cols.join(", ")
        ))
    }

    fn fingerprint_sql_where(
        &self,
        side: Side,
        source: &Source,
        filter: Option<&str>,
    ) -> Result<String, CoreError> {
        let controls = self.controls(side);
        let inner_extra: Vec<String> = controls
            .iter()
            .enumerate()
            .map(|(i, c)| format!("{c} AS c{}", i + 1))
            .collect();
        let inner = self.hashed_select_where(side, source, &inner_extra, filter)?;
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

/// The column types a huge table may be split on (PLAN.md 3.1): numbers and
/// dates only, never text, because text order differs between servers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RangeType {
    Int2,
    Int4,
    Int8,
    Numeric,
    Date,
    Timestamp,
    Timestamptz,
}

impl RangeType {
    /// The type a range column may be split on, from its declared type.
    pub fn of(ty: &PgType) -> Option<RangeType> {
        Some(match ty.base() {
            PgType::Int2 => RangeType::Int2,
            PgType::Int4 => RangeType::Int4,
            PgType::Int8 => RangeType::Int8,
            PgType::Numeric(_) => RangeType::Numeric,
            PgType::Date => RangeType::Date,
            PgType::Timestamp => RangeType::Timestamp,
            PgType::Timestamptz => RangeType::Timestamptz,
            _ => return None,
        })
    }

    /// The SQL type name a bound text parameter is cast to.  A fixed list, so
    /// nothing user-supplied ever reaches the SQL text.
    pub fn sql(self) -> &'static str {
        match self {
            RangeType::Int2 => "int2",
            RangeType::Int4 => "int4",
            RangeType::Int8 => "int8",
            RangeType::Numeric => "numeric",
            RangeType::Date => "date",
            RangeType::Timestamp => "timestamp",
            RangeType::Timestamptz => "timestamptz",
        }
    }
}

/// One part of a split table.  The parts made by [`range_parts`] cover every
/// row exactly once: below the first cut, between consecutive cuts (half
/// open), from the last cut up, and the rows whose split column is NULL.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RangePart {
    Below(String),
    Between(String, String),
    From(String),
    Null,
}

/// The parts for strictly increasing `cuts` (in the column's own order, as
/// the server returned them).  No cuts means no split: `None`.
pub fn range_parts(cuts: &[String]) -> Option<Vec<RangePart>> {
    let (first, last) = (cuts.first()?, cuts.last()?);
    let mut parts = vec![RangePart::Below(first.clone())];
    for w in cuts.windows(2) {
        parts.push(RangePart::Between(w[0].clone(), w[1].clone()));
    }
    parts.push(RangePart::From(last.clone()));
    parts.push(RangePart::Null);
    Some(parts)
}

/// A range predicate on one side: the column (this side's name), its type and
/// the part.  Values are bound as text and cast by the server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RangeFilter {
    pub column: String,
    pub ty: RangeType,
    pub part: RangePart,
}

impl RangeFilter {
    /// The `WHERE` predicate and its parameters (`$1`, `$2`).
    pub fn sql(&self) -> Result<(String, Vec<String>), CoreError> {
        let c = quote_ident(&self.column)?;
        let t = self.ty.sql();
        Ok(match &self.part {
            RangePart::Below(b) => (format!("{c} < $1::text::{t}"), vec![b.clone()]),
            RangePart::Between(a, b) => (
                format!("{c} >= $1::text::{t} AND {c} < $2::text::{t}"),
                vec![a.clone(), b.clone()],
            ),
            RangePart::From(a) => (format!("{c} >= $1::text::{t}"), vec![a.clone()]),
            RangePart::Null => (format!("{c} IS NULL"), Vec::new()),
        })
    }
}

/// The cut points for splitting a table on `column` into about `parts` equal
/// value ranges, computed by the server from the column's min and max (cheap
/// on an indexed key) and returned in the column's own order, distinct.  The
/// sort is on the typed value (`d.v`), never on the text output: text order
/// ("100" < "16") would make the ranges overlap.
/// `$1` is the number of parts.  Never ordered as text.
pub fn range_cuts_sql(source: &Source, column: &str, ty: RangeType) -> Result<String, CoreError> {
    let c = quote_ident(column)?;
    let from = source.sql_from_item()?;
    let t = ty.sql();
    // Equal-width steps between min and max.  Integer and numeric columns step
    // through numeric (no overflow); dates by whole days; timestamps by
    // interval.  The cut is cast back to the column's own type.
    let step = match ty {
        RangeType::Int2 | RangeType::Int4 | RangeType::Int8 => {
            format!("floor(lo::numeric + (hi::numeric - lo::numeric) * g / $1::int)::{t}")
        }
        RangeType::Numeric => "(lo + (hi - lo) * g / $1::int)".to_string(),
        RangeType::Date => "(lo + ((hi - lo) * g / $1::int))".to_string(),
        RangeType::Timestamp | RangeType::Timestamptz => {
            "(lo + (hi - lo) * (g::float8 / $1::int))".to_string()
        }
    };
    Ok(format!(
        "SELECT d.v::text AS cut FROM (SELECT DISTINCT {step}::{t} AS v \
         FROM (SELECT min({c}) AS lo, max({c}) AS hi FROM {from}) m, \
         generate_series(1, $1::int - 1) AS g WHERE lo IS NOT NULL) d ORDER BY d.v"
    ))
}

/// Fetch the cut points (see [`range_cuts_sql`]).
pub async fn range_cuts(
    client: &impl GenericClient,
    source: &Source,
    column: &str,
    ty: RangeType,
    parts: u32,
) -> Result<Vec<String>, CoreError> {
    if parts < 2 {
        return Ok(Vec::new());
    }
    let sql = range_cuts_sql(source, column, ty)?;
    check_session(client).await?;
    let n = i32::try_from(parts).map_err(|_| CoreError::Plan("too many ranges"))?;
    let rows = client.query(sql.as_str(), &[&n]).await?;
    let cuts: Vec<String> = rows
        .iter()
        .map(|r| r.try_get::<_, Option<String>>(0))
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .flatten()
        .collect();
    Ok(cuts)
}

/// Scan one range of one side (server-side md5) and return its fingerprint.
pub async fn fingerprint_range(
    client: &impl GenericClient,
    plan: &TablePlan,
    side: Side,
    source: &Source,
    range: &RangeFilter,
    mapping_hash: &str,
) -> Result<Fingerprint, CoreError> {
    let (filter, params) = range.sql()?;
    let sql = plan.fingerprint_sql_range(side, source, &filter)?;
    let refs: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> = params
        .iter()
        .map(|p| p as &(dyn tokio_postgres::types::ToSql + Sync))
        .collect();
    check_session(client).await?;
    let rows = client.query(sql.as_str(), &refs).await?;
    let text: Vec<Vec<Option<String>>> = rows
        .iter()
        .map(|r| {
            (0..r.len())
                .map(|i| r.try_get::<_, Option<String>>(i))
                .collect::<Result<Vec<_>, _>>()
        })
        .collect::<Result<_, _>>()?;
    let (buckets, control_totals, total) =
        parse_summary(&text, plan.bucket_digits, plan.control_count())?;
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

/// Sum the fingerprints of a table's parts into the table's fingerprint.
/// Bucket sums are additive (PLAN.md 3.1), so for parts that cover every row
/// exactly once the result equals the unsplit scan.  Parts must agree on
/// bucket width, keying, mapping and control count; any truncated part makes
/// the whole truncated, and any part needing review makes the whole need it.
pub fn merge_fingerprints(parts: &[Fingerprint]) -> Result<Fingerprint, CoreError> {
    let first = parts.first().ok_or(CoreError::Plan("no parts to merge"))?;
    let mut buckets: std::collections::BTreeMap<u32, (u64, [u128; 4])> =
        std::collections::BTreeMap::new();
    let mut totals = first.control_totals.clone();
    let mut rows: u64 = 0;
    let mut truncated = false;
    let mut needs_review = false;
    for (i, p) in parts.iter().enumerate() {
        if p.bucket_digits != first.bucket_digits
            || p.keyed != first.keyed
            || p.mapping_hash != first.mapping_hash
            || p.control_totals.len() != first.control_totals.len()
        {
            return Err(CoreError::Plan("range parts do not match"));
        }
        truncated |= p.truncated;
        needs_review |= p.needs_review;
        rows = rows
            .checked_add(p.rows)
            .ok_or(CoreError::Overflow("row count"))?;
        if i > 0 {
            for (t, v) in totals.iter_mut().zip(&p.control_totals) {
                *t = t
                    .checked_add(*v)
                    .ok_or(CoreError::Overflow("control total"))?;
            }
        }
        for b in &p.buckets {
            let e = buckets.entry(b.bucket).or_insert((0, [0; 4]));
            e.0 =
                e.0.checked_add(b.n)
                    .ok_or(CoreError::Overflow("row count"))?;
            for (acc, v) in e.1.iter_mut().zip(b.s) {
                *acc = acc.checked_add(v).ok_or(CoreError::Overflow("hash sum"))?;
            }
        }
    }
    Ok(Fingerprint {
        bucket_digits: first.bucket_digits,
        keyed: first.keyed,
        mapping_hash: first.mapping_hash.clone(),
        rows,
        buckets: buckets
            .into_iter()
            .map(|(bucket, (n, s))| BucketSummary { bucket, n, s })
            .collect(),
        control_totals: totals,
        truncated,
        needs_review,
    })
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
///
/// Deserialisation is hand-written so a crafted fingerprint file cannot set
/// `scale` beyond [`MAX_DECIMAL_SCALE`]: a huge scale would make [`Decimal::reduced`]
/// loop ~4·10⁹ times and [`std::fmt::Display`] allocate gigabytes.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct Decimal {
    mantissa: i128,
    scale: u32,
}

const MAX_DECIMAL_SCALE: u32 = 38;

impl<'de> Deserialize<'de> for Decimal {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        use serde::de::Error;

        #[derive(Deserialize)]
        struct Raw {
            mantissa: i128,
            scale: u32,
        }
        let raw = Raw::deserialize(deserializer)?;
        if raw.scale > MAX_DECIMAL_SCALE {
            return Err(Error::custom("decimal scale exceeds the supported maximum"));
        }
        Ok(Decimal {
            mantissa: raw.mantissa,
            scale: raw.scale,
        })
    }
}

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

    /// Which parts an integer value (None = NULL) falls into, by the same
    /// half-open rules the SQL uses.
    fn parts_holding(parts: &[RangePart], v: Option<i64>) -> usize {
        let n = |s: &str| s.parse::<i64>().unwrap();
        parts
            .iter()
            .filter(|p| match (p, v) {
                (RangePart::Null, None) => true,
                (RangePart::Null, Some(_)) | (_, None) => false,
                (RangePart::Below(b), Some(x)) => x < n(b),
                (RangePart::Between(a, b), Some(x)) => n(a) <= x && x < n(b),
                (RangePart::From(a), Some(x)) => x >= n(a),
            })
            .count()
    }

    proptest! {
        /// Every value, and NULL, falls in exactly one part.
        #[test]
        fn range_parts_cover_every_value_exactly_once(
            mut cuts in proptest::collection::btree_set(-1000i64..1000, 1..20),
            values in proptest::collection::vec(proptest::option::of(-2000i64..2000), 0..200),
        ) {
            let cuts: Vec<String> = std::mem::take(&mut cuts).into_iter().map(|c| c.to_string()).collect();
            let parts = range_parts(&cuts).unwrap();
            prop_assert_eq!(parts.len(), cuts.len() + 2);
            for v in values.into_iter().chain([None, Some(i64::MIN), Some(i64::MAX)]) {
                prop_assert_eq!(parts_holding(&parts, v), 1, "value {:?}", v);
            }
        }
    }

    #[test]
    fn no_cuts_means_no_split() {
        assert!(range_parts(&[]).is_none());
        assert_eq!(
            range_parts(&["5".into()]).unwrap(),
            vec![
                RangePart::Below("5".into()),
                RangePart::From("5".into()),
                RangePart::Null
            ]
        );
    }

    #[test]
    fn range_filters_bind_values_and_quote_the_column() {
        let f = |part| RangeFilter {
            column: "posted \"at".into(),
            ty: RangeType::Date,
            part,
        };
        let (sql, p) = f(RangePart::Between("2020-01-01".into(), "2021-01-01".into()))
            .sql()
            .unwrap();
        assert_eq!(
            sql,
            "\"posted \"\"at\" >= $1::text::date AND \"posted \"\"at\" < $2::text::date"
        );
        assert_eq!(p, vec!["2020-01-01".to_string(), "2021-01-01".to_string()]);
        let (sql, p) = f(RangePart::Null).sql().unwrap();
        assert_eq!(sql, "\"posted \"\"at\" IS NULL");
        assert!(p.is_empty());
        assert_eq!(
            f(RangePart::Below("x".into())).sql().unwrap().0,
            "\"posted \"\"at\" < $1::text::date"
        );
        assert_eq!(
            f(RangePart::From("x".into())).sql().unwrap().0,
            "\"posted \"\"at\" >= $1::text::date"
        );
        // Only numbers and dates may be split on, never text.
        assert_eq!(RangeType::of(&PgType::Text), None);
        assert_eq!(RangeType::of(&PgType::Varchar), None);
        assert_eq!(RangeType::of(&PgType::Int8), Some(RangeType::Int8));
        assert_eq!(
            RangeType::of(&PgType::Timestamptz).map(RangeType::sql),
            Some("timestamptz")
        );
    }

    #[test]
    fn cut_query_sorts_by_the_typed_value() {
        let src = Source::Table {
            schema: "s".into(),
            name: "t".into(),
        };
        for ty in [
            RangeType::Int4,
            RangeType::Numeric,
            RangeType::Date,
            RangeType::Timestamp,
        ] {
            let sql = range_cuts_sql(&src, "k", ty).unwrap();
            assert!(sql.ends_with("ORDER BY d.v"), "{sql}");
            assert!(
                sql.contains("min(\"k\")") && sql.contains("max(\"k\")"),
                "{sql}"
            );
        }
    }

    fn part(buckets: &[(u32, u64)], total: &str) -> Fingerprint {
        Fingerprint {
            bucket_digits: 2,
            keyed: true,
            mapping_hash: "m".into(),
            rows: buckets.iter().map(|b| b.1).sum(),
            buckets: buckets
                .iter()
                .map(|(b, n)| BucketSummary {
                    bucket: *b,
                    n: *n,
                    s: [*n as u128; 4],
                })
                .collect(),
            control_totals: vec![Decimal::parse(total).unwrap()],
            truncated: false,
            needs_review: false,
        }
    }

    #[test]
    fn merging_sums_buckets_rows_and_totals() {
        let m = merge_fingerprints(&[
            part(&[(1, 2), (5, 1)], "1.5"),
            part(&[(1, 3), (9, 4)], "-0.5"),
        ])
        .unwrap();
        assert_eq!(m.rows, 10);
        assert_eq!(
            m.buckets,
            vec![
                BucketSummary {
                    bucket: 1,
                    n: 5,
                    s: [5; 4]
                },
                BucketSummary {
                    bucket: 5,
                    n: 1,
                    s: [1; 4]
                },
                BucketSummary {
                    bucket: 9,
                    n: 4,
                    s: [4; 4]
                },
            ]
        );
        assert_eq!(m.control_totals, vec![Decimal::parse("1").unwrap()]);
        assert!(!m.truncated && !m.needs_review);
        // One truncated or review-needing part taints the whole.
        let mut t = part(&[], "0");
        t.truncated = true;
        assert!(
            merge_fingerprints(&[part(&[(1, 1)], "0"), t])
                .unwrap()
                .truncated
        );
        let mut r = part(&[], "0");
        r.needs_review = true;
        assert!(
            merge_fingerprints(&[r, part(&[(1, 1)], "0")])
                .unwrap()
                .needs_review
        );
        // Mismatched parts are refused, and nothing to merge is an error.
        assert!(merge_fingerprints(&[]).is_err());
        let mut other = part(&[], "0");
        other.bucket_digits = 3;
        assert!(merge_fingerprints(&[part(&[], "0"), other]).is_err());
        let mut keyless = part(&[], "0");
        keyless.keyed = false;
        assert!(merge_fingerprints(&[part(&[], "0"), keyless]).is_err());
        let mut mh = part(&[], "0");
        mh.mapping_hash = "x".into();
        assert!(merge_fingerprints(&[part(&[], "0"), mh]).is_err());
        let mut ct = part(&[], "0");
        ct.control_totals.clear();
        assert!(merge_fingerprints(&[part(&[], "0"), ct]).is_err());
    }

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
    fn decimal_deserialise_enforces_scale_limit() {
        // A crafted file with scale = u32::MAX must be rejected fast, before
        // any reduced() loop or Display allocation could run away.
        let bad = r#"{"mantissa":0,"scale":4294967295}"#;
        assert!(serde_json::from_str::<Decimal>(bad).is_err());
        assert!(serde_json::from_str::<Decimal>(r#"{"mantissa":0,"scale":39}"#).is_err());
        // The boundary value and a normal value still round-trip.
        let ok: Decimal = serde_json::from_str(r#"{"mantissa":123,"scale":38}"#).unwrap();
        assert_eq!(
            ok,
            Decimal {
                mantissa: 123,
                scale: 38
            }
        );
        let ok: Decimal = serde_json::from_str(r#"{"mantissa":-1250,"scale":2}"#).unwrap();
        assert_eq!(ok.to_string(), "-12.50");
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
