//! Drill-down into differing buckets (PLAN.md 3.1 step 6).
//!
//! A second scan returns the key hash, key values and row hash of just the
//! rows in the differing buckets (bound as an `int[]`, 9.4-safe). Rows are
//! matched in the app by key hash, never ordered by text on a server (P24).
//! Row values in these types are user data: `Debug` hides them.

use super::fingerprint::{Side, Source, TablePlan};
use super::{check_session, CoreError};
use std::collections::{BTreeMap, HashMap};
use tokio_postgres::GenericClient;

/// Changed rows per table whose column values are fetched and compared.
pub const CHANGED_DETAIL_LIMIT: usize = 100;

#[derive(Clone, PartialEq, Eq)]
pub struct RowRef {
    pub bucket: u32,
    /// For a table without a key this is the row hash.
    pub key_hash: String,
    /// Normalised key values; empty for a table without a key.
    pub key: Vec<Option<String>>,
    pub row_hash: String,
}

impl std::fmt::Debug for RowRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RowRef")
            .field("bucket", &self.bucket)
            .field("key_hash", &self.key_hash)
            .field("key", &"<hidden>")
            .finish()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Drilldown {
    /// In old, not in new.
    pub missing: Vec<RowRef>,
    /// In new, not in old.
    pub extra: Vec<RowRef>,
    /// Same key, different row hash: (old, new).
    pub changed: Vec<(RowRef, RowRef)>,
    /// Key hashes seen more than once on a side with differing rows.
    pub duplicate_keys: Vec<String>,
}

impl Drilldown {
    /// False when buckets differed but no row difference was found: the data
    /// moved between scans or the tool is wrong. Report as a tool fault.
    pub fn explains_difference(&self) -> bool {
        !(self.missing.is_empty()
            && self.extra.is_empty()
            && self.changed.is_empty()
            && self.duplicate_keys.is_empty())
    }
}

/// The drill-down query for one side; `$1` is the `int[]` of buckets.
pub fn rows_sql(plan: &TablePlan, side: Side, source: &Source) -> Result<String, CoreError> {
    let (extras, kh) = if plan.is_keyed() {
        let keys = plan.key_exprs(side).join(", ");
        (vec![format!("ARRAY[{keys}]::text[] AS kv")], "kh")
    } else {
        (vec!["'{}'::text[] AS kv".to_string()], "rh")
    };
    let inner = plan.hashed_select(side, source, &extras)?;
    Ok(format!(
        "SELECT bucket, kh, kv, rh FROM (SELECT {} AS bucket, {kh} AS kh, kv, rh FROM ({inner}) q) q2 WHERE bucket = ANY($1)",
        plan.bucket_expr(kh)
    ))
}

/// The value query for one side; `$1` is the `text[]` of key hashes.
pub fn values_sql(plan: &TablePlan, side: Side, source: &Source) -> Result<String, CoreError> {
    if !plan.is_keyed() {
        return Err(CoreError::Plan("column detail needs a key"));
    }
    let vals = format!("ARRAY[{}]::text[] AS vals", plan.exprs(side).join(", "));
    let inner = plan.hashed_select(side, source, &[vals])?;
    Ok(format!(
        "SELECT kh, vals FROM ({inner}) q WHERE kh = ANY($1)"
    ))
}

/// Fetch key and row hashes of the rows in `buckets` on one side.
pub async fn fetch_rows(
    client: &impl GenericClient,
    plan: &TablePlan,
    side: Side,
    source: &Source,
    buckets: &[u32],
) -> Result<Vec<RowRef>, CoreError> {
    let sql = rows_sql(plan, side, source)?;
    let ids: Vec<i32> = buckets
        .iter()
        .map(|b| i32::try_from(*b).map_err(|_| CoreError::Plan("bucket out of range")))
        .collect::<Result<_, _>>()?;
    check_session(client).await?;
    let rows = client.query(sql.as_str(), &[&ids]).await?;
    rows.iter()
        .map(|r| {
            let bucket: i32 = r.try_get(0)?;
            Ok(RowRef {
                bucket: u32::try_from(bucket).map_err(|_| CoreError::Malformed("bucket"))?,
                key_hash: r.try_get(1)?,
                key: r.try_get(2)?,
                row_hash: r.try_get(3)?,
            })
        })
        .collect()
}

/// Match the two sides' rows. Keyed tables match by key hash; tables without
/// a key are compared as counted sets of row hashes.
pub fn classify(old: Vec<RowRef>, new: Vec<RowRef>) -> Drilldown {
    let mut groups: BTreeMap<String, (Vec<RowRef>, Vec<RowRef>)> = BTreeMap::new();
    for r in old {
        groups.entry(r.key_hash.clone()).or_default().0.push(r);
    }
    for r in new {
        groups.entry(r.key_hash.clone()).or_default().1.push(r);
    }
    let mut d = Drilldown::default();
    for (kh, (mut o, mut n)) in groups {
        if o.len() == 1 && n.len() == 1 {
            if o[0].row_hash != n[0].row_hash {
                d.changed.push((o.remove(0), n.remove(0)));
            }
            continue;
        }
        let rh = |v: &[RowRef]| {
            let mut h: Vec<&str> = v.iter().map(|r| r.row_hash.as_str()).collect();
            h.sort_unstable();
            h.into_iter().map(str::to_owned).collect::<Vec<_>>()
        };
        if rh(&o) == rh(&n) {
            continue;
        }
        let keyless = o.iter().chain(&n).all(|r| r.row_hash == kh);
        if !keyless && !o.is_empty() && !n.is_empty() {
            d.duplicate_keys.push(kh);
            continue;
        }
        // Counted set: surplus copies on one side are missing or extra rows.
        let (lo, ln) = (o.len(), n.len());
        d.missing.extend(o.into_iter().skip(ln));
        d.extra.extend(n.into_iter().skip(lo));
    }
    d
}

/// One changed row with both normalised versions and the differing columns.
#[derive(Clone, PartialEq, Eq)]
pub struct ChangedRow {
    pub key_hash: String,
    pub key: Vec<Option<String>>,
    /// Indexes into the table plan's columns.
    pub columns: Vec<usize>,
    pub old: Vec<Option<String>>,
    pub new: Vec<Option<String>>,
}

impl std::fmt::Debug for ChangedRow {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ChangedRow")
            .field("key_hash", &self.key_hash)
            .field("columns", &self.columns)
            .field("values", &"<hidden>")
            .finish()
    }
}

async fn fetch_values(
    client: &impl GenericClient,
    plan: &TablePlan,
    side: Side,
    source: &Source,
    hashes: &[String],
) -> Result<HashMap<String, Vec<Option<String>>>, CoreError> {
    let sql = values_sql(plan, side, source)?;
    check_session(client).await?;
    let rows = client.query(sql.as_str(), &[&hashes]).await?;
    let mut out = HashMap::new();
    for r in rows {
        let vals: Vec<Option<String>> = r.try_get(1)?;
        if vals.len() != plan.columns().len() {
            return Err(CoreError::Malformed("column values"));
        }
        out.insert(r.try_get(0)?, vals);
    }
    Ok(out)
}

/// For up to [`CHANGED_DETAIL_LIMIT`] changed rows, fetch both versions and
/// name the columns that differ. A changed row whose values cannot be found
/// again, or whose values all match, is an error: the data moved.
pub async fn changed_detail(
    old_client: &impl GenericClient,
    new_client: &impl GenericClient,
    plan: &TablePlan,
    old_source: &Source,
    new_source: &Source,
    changed: &[(RowRef, RowRef)],
) -> Result<Vec<ChangedRow>, CoreError> {
    let take = &changed[..changed.len().min(CHANGED_DETAIL_LIMIT)];
    let hashes: Vec<String> = take.iter().map(|(o, _)| o.key_hash.clone()).collect();
    let ov = fetch_values(old_client, plan, Side::Old, old_source, &hashes).await?;
    let nv = fetch_values(new_client, plan, Side::New, new_source, &hashes).await?;
    take.iter()
        .map(|(o, _)| {
            let (Some(a), Some(b)) = (ov.get(&o.key_hash), nv.get(&o.key_hash)) else {
                return Err(CoreError::Malformed("changed row vanished"));
            };
            let columns: Vec<usize> = (0..a.len()).filter(|i| a[*i] != b[*i]).collect();
            if columns.is_empty() {
                return Err(CoreError::Malformed("changed row has no differing column"));
            }
            Ok(ChangedRow {
                key_hash: o.key_hash.clone(),
                key: o.key.clone(),
                columns,
                old: a.clone(),
                new: b.clone(),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::fingerprint::tests::{accounts_plan, canon};

    fn r(kh: &str, rh: &str) -> RowRef {
        RowRef {
            bucket: 1,
            key_hash: kh.into(),
            key: vec![Some("secret".into())],
            row_hash: rh.into(),
        }
    }

    #[test]
    fn classifies_keyed_rows() {
        let old = vec![r("a", "1"), r("b", "2"), r("c", "3"), r("e", "5")];
        let new = vec![r("a", "1"), r("b", "9"), r("d", "4"), r("e", "5")];
        let d = classify(old, new);
        assert_eq!(d.missing, vec![r("c", "3")]);
        assert_eq!(d.extra, vec![r("d", "4")]);
        assert_eq!(d.changed, vec![(r("b", "2"), r("b", "9"))]);
        assert!(d.explains_difference());
    }

    #[test]
    fn keyless_rows_are_a_counted_set() {
        let old = vec![r("x", "x"), r("x", "x"), r("y", "y")];
        let new = vec![r("x", "x"), r("y", "y"), r("y", "y"), r("z", "z")];
        let d = classify(old, new);
        assert_eq!(d.missing, vec![r("x", "x")]);
        assert_eq!(d.extra.len(), 2);
        assert!(d.changed.is_empty());
    }

    #[test]
    fn duplicate_keys_and_unexplained() {
        let d = classify(vec![r("a", "1"), r("a", "2")], vec![r("a", "1")]);
        assert_eq!(d.duplicate_keys, vec!["a".to_string()]);
        let d = classify(
            vec![r("a", "1"), r("a", "2")],
            vec![r("a", "2"), r("a", "1")],
        );
        assert!(!d.explains_difference());
        let d = classify(vec![r("a", "1"), r("a", "2")], vec![]);
        assert_eq!(d.missing.len(), 2);
    }

    #[test]
    fn each_kind_of_difference_explains_a_bucket() {
        let one = |f: fn(&mut Drilldown)| {
            let mut d = Drilldown::default();
            f(&mut d);
            d.explains_difference()
        };
        assert!(!one(|_| {}));
        assert!(one(|d| d.missing.push(r("a", "1"))));
        assert!(one(|d| d.extra.push(r("a", "1"))));
        assert!(one(|d| d.changed.push((r("a", "1"), r("a", "2")))));
        assert!(one(|d| d.duplicate_keys.push("a".into())));
    }

    #[test]
    fn debug_hides_row_data() {
        let s = format!("{:?}", r("a", "1"));
        assert!(!s.contains("secret"));
        assert!(s.contains("RowRef") && s.contains("<hidden>") && s.contains("bucket: 1"));
        let c = ChangedRow {
            key_hash: "k".into(),
            key: vec![Some("secret".into())],
            columns: vec![1],
            old: vec![Some("secret".into())],
            new: vec![None],
        };
        let s = format!("{c:?}");
        assert!(!s.contains("secret"));
        assert!(s.contains("ChangedRow") && s.contains("<hidden>") && s.contains("columns: [1]"));
    }

    #[test]
    fn drilldown_sql_shape() {
        let p = accounts_plan();
        let src = Source::Table {
            schema: "s".into(),
            name: "t".into(),
        };
        let sql = rows_sql(&p, Side::Old, &src).unwrap();
        assert!(canon(&sql).contains("ARRAY[\"acct_id\"::text, \"bank_code\"::text]::text[] AS kv"));
        assert!(sql.ends_with("WHERE bucket = ANY($1)"));
        assert!(sql.contains("('x' || substr(kh, 1, 3))::bit(12)::int AS bucket"));
        assert!(values_sql(&p, Side::New, &src)
            .unwrap()
            .ends_with("WHERE kh = ANY($1)"));
        let keyless = TablePlan::new(p.columns().to_vec(), vec![], 3).unwrap();
        let sql = rows_sql(&keyless, Side::Old, &src).unwrap();
        assert!(sql.contains("rh AS kh, kv, rh"));
        assert!(values_sql(&keyless, Side::Old, &src).is_err());
    }
}
