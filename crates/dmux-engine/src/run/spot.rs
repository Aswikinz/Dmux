//! The independent spot check (PLAN.md 3.4).
//!
//! For a keyed full-values table, both sides return their 200 rows with the
//! smallest key hash (ordered with `COLLATE "C"`, so the server's text sort
//! order never matters), the union of those keys is fetched from both sides,
//! and every **normalised column value** is compared in the app.  Rows are
//! matched by their normalised key values, not by any hash, and no row hash
//! is used: the fingerprint's hashing and summing is bypassed entirely.
//!
//! The outcome is then checked against the fingerprint for the same rows.  A
//! row that differs (value, missing or extra) in a bucket the fingerprint
//! called equal, or a key whose key hash differs between the sides, is a
//! disagreement and becomes TOOL_FAULT.  A difference inside a bucket the
//! fingerprint already reported is consistent and keeps the data verdict.
//!
//! Row values are held in memory only; the outcome carries counts and bucket
//! numbers, never a value (rule 7).

use std::collections::{BTreeMap, BTreeSet};

use crate::core::fingerprint::{Side, Source, TablePlan};
use crate::core::{check_session, CoreError};
use tokio_postgres::Transaction;

use super::verdict::Spot;

/// The rows each side contributes to the sample (PLAN.md 3.4).
pub const SPOT_CHECK_ROWS: i64 = 200;

/// One sampled row: its key hash, normalised key values and normalised values.
#[derive(Clone, PartialEq, Eq)]
pub struct SpotRow {
    pub kh: String,
    pub kv: Vec<Option<String>>,
    pub vals: Vec<Option<String>>,
}

impl std::fmt::Debug for SpotRow {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SpotRow")
            .field("kh", &self.kh)
            .field("values", &"<hidden>")
            .finish()
    }
}

fn inner_select(plan: &TablePlan, side: Side, source: &Source) -> Result<String, CoreError> {
    let keys = plan.key_exprs(side).join(", ");
    let vals = plan.exprs(side).join(", ");
    plan.hashed_select(
        side,
        source,
        &[
            format!("ARRAY[{keys}]::text[] AS kv"),
            format!("ARRAY[{vals}]::text[] AS vals"),
        ],
    )
}

fn rows_of(rows: Vec<tokio_postgres::Row>, width: usize) -> Result<Vec<SpotRow>, CoreError> {
    rows.into_iter()
        .map(|r| {
            let row = SpotRow {
                kh: r.try_get(0)?,
                kv: r.try_get(1)?,
                vals: r.try_get(2)?,
            };
            if row.vals.len() != width {
                return Err(CoreError::Malformed("spot-check values"));
            }
            Ok(row)
        })
        .collect()
}

/// The side's rows with the smallest key hashes.
pub async fn sample(
    txn: &Transaction<'_>,
    plan: &TablePlan,
    side: Side,
    source: &Source,
) -> Result<Vec<SpotRow>, CoreError> {
    if !plan.is_keyed() {
        return Err(CoreError::Plan("the spot check needs a key"));
    }
    let sql = format!(
        "SELECT kh, kv, vals FROM ({}) q ORDER BY kh COLLATE \"C\" LIMIT $1",
        inner_select(plan, side, source)?
    );
    check_session(txn).await?;
    let rows = txn.query(sql.as_str(), &[&SPOT_CHECK_ROWS]).await?;
    rows_of(rows, plan.columns().len())
}

/// The side's rows whose key hash is in `khs`.
pub async fn fetch(
    txn: &Transaction<'_>,
    plan: &TablePlan,
    side: Side,
    source: &Source,
    khs: &[String],
) -> Result<Vec<SpotRow>, CoreError> {
    let sql = format!(
        "SELECT kh, kv, vals FROM ({}) q WHERE kh = ANY($1)",
        inner_select(plan, side, source)?
    );
    check_session(txn).await?;
    let rows = txn.query(sql.as_str(), &[&khs]).await?;
    rows_of(rows, plan.columns().len())
}

/// Compare the sampled rows of both sides value by value and check the result
/// against the fingerprint's non-equal buckets.
pub fn judge(
    bucket_digits: u8,
    non_equal: &BTreeSet<u32>,
    old: &[SpotRow],
    new: &[SpotRow],
) -> Spot {
    let bucket = |kh: &str| -> Option<u32> {
        kh.get(..usize::from(bucket_digits))
            .and_then(|h| u32::from_str_radix(h, 16).ok())
    };
    let group = |rows: &[SpotRow]| -> BTreeMap<Vec<Option<String>>, Vec<SpotRow>> {
        let mut m: BTreeMap<Vec<Option<String>>, Vec<SpotRow>> = BTreeMap::new();
        for r in rows {
            m.entry(r.kv.clone()).or_default().push(r.clone());
        }
        m
    };
    let (o, n) = (group(old), group(new));
    let keys: BTreeSet<&Vec<Option<String>>> = o.keys().chain(n.keys()).collect();

    let (mut compared, mut differing) = (0usize, 0usize);
    for k in keys {
        let empty = Vec::new();
        let (or, nr) = (o.get(k).unwrap_or(&empty), n.get(k).unwrap_or(&empty));
        let mut khs: BTreeSet<&str> = or.iter().chain(nr.iter()).map(|r| r.kh.as_str()).collect();
        if khs.len() != 1 {
            return Spot::Disagrees(
                "equal key values have different key hashes on the two sides".into(),
            );
        }
        let kh = khs.pop_first().unwrap_or_default();
        let Some(b) = bucket(kh) else {
            return Spot::Disagrees("a key hash is not a valid bucket prefix".into());
        };
        let mut ov: Vec<&Vec<Option<String>>> = or.iter().map(|r| &r.vals).collect();
        let mut nv: Vec<&Vec<Option<String>>> = nr.iter().map(|r| &r.vals).collect();
        ov.sort();
        nv.sort();
        if !or.is_empty() && !nr.is_empty() {
            compared += 1;
        }
        if ov != nv {
            differing += 1;
            if !non_equal.contains(&b) {
                let what = match (or.is_empty(), nr.is_empty()) {
                    (false, true) => "a row missing on the new side",
                    (true, false) => "an extra row on the new side",
                    _ => "a differing value",
                };
                return Spot::Disagrees(format!(
                    "{what} in bucket {b}, which the fingerprint called equal"
                ));
            }
        }
    }
    Spot::Agrees {
        compared,
        differing,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(kh: &str, k: &str, v: &str) -> SpotRow {
        SpotRow {
            kh: kh.into(),
            kv: vec![Some(k.into())],
            vals: vec![Some(k.into()), Some(v.into())],
        }
    }

    fn none() -> BTreeSet<u32> {
        BTreeSet::new()
    }

    #[test]
    fn equal_rows_agree() {
        let a = vec![row("00aa", "1", "x"), row("01bb", "2", "y")];
        assert_eq!(
            judge(3, &none(), &a, &a.clone()),
            Spot::Agrees {
                compared: 2,
                differing: 0
            }
        );
    }

    #[test]
    fn a_value_difference_the_fingerprint_missed_is_a_disagreement() {
        // The simulated hash/value disagreement: the fingerprint called every
        // bucket equal, yet a normalised value differs.
        let old = vec![row("000a", "1", "x")];
        let new = vec![row("000a", "1", "z")];
        assert!(
            matches!(judge(3, &none(), &old, &new), Spot::Disagrees(m) if m.contains("bucket 0"))
        );
        // The same difference inside a bucket the fingerprint reported agrees.
        let reported: BTreeSet<u32> = [0].into();
        assert_eq!(
            judge(3, &reported, &old, &new),
            Spot::Agrees {
                compared: 1,
                differing: 1
            }
        );
    }

    #[test]
    fn a_missing_row_is_consistent_only_when_its_bucket_was_reported() {
        // m01-style: a sampled key is missing on new.
        let old = vec![row("0a1f", "137", "x"), row("0b00", "2", "y")];
        let new = vec![row("0b00", "2", "y")];
        let reported: BTreeSet<u32> = [0x0a1].into();
        assert_eq!(
            judge(3, &reported, &old, &new),
            Spot::Agrees {
                compared: 1,
                differing: 1
            }
        );
        assert!(
            matches!(judge(3, &none(), &old, &new), Spot::Disagrees(m) if m.contains("missing"))
        );
        // An extra row on new, unreported.
        assert!(matches!(judge(3, &none(), &new, &old), Spot::Disagrees(m) if m.contains("extra")));
    }

    #[test]
    fn inconsistent_key_hashes_are_a_disagreement() {
        let old = vec![row("00aa", "1", "x")];
        let new = vec![row("ffaa", "1", "x")];
        assert!(
            matches!(judge(3, &none(), &old, &new), Spot::Disagrees(m) if m.contains("key hashes"))
        );
        let bad = vec![row("zz", "1", "x")];
        assert!(matches!(judge(3, &none(), &bad, &bad), Spot::Disagrees(_)));
    }

    #[test]
    fn duplicated_keys_are_compared_as_a_multiset() {
        let old = vec![row("000a", "1", "x")];
        let new = vec![row("000a", "1", "x"), row("000a", "1", "x")];
        assert!(matches!(judge(3, &none(), &old, &new), Spot::Disagrees(_)));
        let reported: BTreeSet<u32> = [0].into();
        assert!(matches!(
            judge(3, &reported, &old, &new),
            Spot::Agrees { .. }
        ));
    }

    #[test]
    fn debug_hides_values() {
        assert!(!format!("{:?}", row("00", "secret", "secret")).contains("secret"));
    }
}
