//! Bucket-by-bucket comparison of two fingerprints (PLAN.md 3.1 steps 5 and 7).
//!
//! PASS needs every one of: both sides scanned without error, neither side
//! truncated, equal row counts, equal control totals and every bucket equal.

use super::fingerprint::{BucketSummary, Fingerprint};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// The stop rule needs at least this many differing buckets (decision 17).
pub const WIDE_MIN_BUCKETS: usize = 64;

/// Buckets drilled into as a sample when a table is widely different.
pub const WIDE_SAMPLE_BUCKETS: usize = 16;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Verdict {
    Pass,
    Fail,
    /// More than 10% of buckets differ; full drill-down is skipped.
    WidelyDifferent,
    /// The data matched, but a column was compared by a printed form that is
    /// not provably exact; a person must review it. Never a PASS.
    NeedsReview,
    /// No decision possible (a side errored or is incomplete). Never a PASS.
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum NotPassReason {
    SideErrored,
    Truncated,
    CountsDiffer,
    BucketsDiffer,
    ControlTotalsDiffer,
    NeedsReview,
}

/// One side's outcome: a fingerprint, or a scan that failed.
#[derive(Debug, Clone, Copy)]
pub enum SideOutcome<'a> {
    Ok(&'a Fingerprint),
    Errored,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CompareError {
    #[error("the fingerprints use different bucket counts")]
    BucketCount,
    #[error("the fingerprints were built from different mappings")]
    MappingHash,
    #[error("one fingerprint is keyed and the other is not")]
    Keyed,
    #[error("the fingerprints have different numbers of control totals")]
    ControlTotals,
    #[error("a fingerprint is inconsistent: {0}")]
    Inconsistent(&'static str),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Comparison {
    pub verdict: Verdict,
    pub reasons: Vec<NotPassReason>,
    pub equal_buckets: usize,
    /// In both, with different summaries.
    pub differing: Vec<u32>,
    /// Only in the old fingerprint.
    pub missing: Vec<u32>,
    /// Only in the new fingerprint.
    pub extra: Vec<u32>,
    pub old_rows: Option<u64>,
    pub new_rows: Option<u64>,
}

impl Comparison {
    pub fn is_pass(&self) -> bool {
        self.verdict == Verdict::Pass
    }

    /// Every non-equal bucket, ascending.
    pub fn non_equal(&self) -> Vec<u32> {
        let mut all: Vec<u32> = [&self.differing, &self.missing, &self.extra]
            .into_iter()
            .flatten()
            .copied()
            .collect();
        all.sort_unstable();
        all
    }

    /// The buckets to drill into: all of them, or a sample when widely different.
    pub fn drilldown_buckets(&self) -> Vec<u32> {
        let mut all = self.non_equal();
        if self.verdict == Verdict::WidelyDifferent {
            all.truncate(WIDE_SAMPLE_BUCKETS);
        }
        all
    }
}

fn index(fp: &Fingerprint) -> Result<BTreeMap<u32, &BucketSummary>, CompareError> {
    let mut map = BTreeMap::new();
    let mut rows: u64 = 0;
    for b in &fp.buckets {
        if map.insert(b.bucket, b).is_some() {
            return Err(CompareError::Inconsistent("duplicate bucket"));
        }
        rows = rows
            .checked_add(b.n)
            .ok_or(CompareError::Inconsistent("row count overflow"))?;
    }
    if rows != fp.rows {
        return Err(CompareError::Inconsistent(
            "row count does not match buckets",
        ));
    }
    Ok(map)
}

pub fn compare(old: SideOutcome<'_>, new: SideOutcome<'_>) -> Result<Comparison, CompareError> {
    let (o, n) = match (old, new) {
        (SideOutcome::Ok(o), SideOutcome::Ok(n)) => (o, n),
        (o, n) => {
            let rows = |s: SideOutcome<'_>| match s {
                SideOutcome::Ok(f) => Some(f.rows),
                SideOutcome::Errored => None,
            };
            return Ok(Comparison {
                verdict: Verdict::Error,
                reasons: vec![NotPassReason::SideErrored],
                equal_buckets: 0,
                differing: vec![],
                missing: vec![],
                extra: vec![],
                old_rows: rows(o),
                new_rows: rows(n),
            });
        }
    };
    if o.bucket_digits != n.bucket_digits {
        return Err(CompareError::BucketCount);
    }
    if o.mapping_hash.is_empty() || o.mapping_hash != n.mapping_hash {
        return Err(CompareError::MappingHash);
    }
    if o.keyed != n.keyed {
        return Err(CompareError::Keyed);
    }
    if o.control_totals.len() != n.control_totals.len() {
        return Err(CompareError::ControlTotals);
    }
    let (oi, ni) = (index(o)?, index(n)?);
    let (mut equal, mut differing, mut missing) = (0, vec![], vec![]);
    for (b, os) in &oi {
        match ni.get(b) {
            Some(ns) if ns == os => equal += 1,
            Some(_) => differing.push(*b),
            None => missing.push(*b),
        }
    }
    let extra: Vec<u32> = ni.keys().filter(|b| !oi.contains_key(b)).copied().collect();

    let mut reasons = Vec::new();
    if o.truncated || n.truncated {
        reasons.push(NotPassReason::Truncated);
    }
    if o.rows != n.rows {
        reasons.push(NotPassReason::CountsDiffer);
    }
    let non_equal = differing.len() + missing.len() + extra.len();
    if non_equal > 0 {
        reasons.push(NotPassReason::BucketsDiffer);
    }
    if o.control_totals != n.control_totals {
        reasons.push(NotPassReason::ControlTotalsDiffer);
    }
    if o.needs_review || n.needs_review {
        reasons.push(NotPassReason::NeedsReview);
    }
    let verdict = if reasons.contains(&NotPassReason::Truncated) {
        Verdict::Error
    } else if non_equal >= WIDE_MIN_BUCKETS && non_equal * 10 > equal + non_equal {
        Verdict::WidelyDifferent
    } else if reasons.is_empty() {
        Verdict::Pass
    } else if reasons == [NotPassReason::NeedsReview] {
        Verdict::NeedsReview
    } else {
        Verdict::Fail
    };
    Ok(Comparison {
        verdict,
        reasons,
        equal_buckets: equal,
        differing,
        missing,
        extra,
        old_rows: Some(o.rows),
        new_rows: Some(n.rows),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::fingerprint::Decimal;
    use proptest::prelude::*;

    fn b(bucket: u32, n: u64, s0: u128) -> BucketSummary {
        BucketSummary {
            bucket,
            n,
            s: [s0, 0, 0, 0],
        }
    }

    fn fp(buckets: Vec<BucketSummary>) -> Fingerprint {
        Fingerprint {
            bucket_digits: 3,
            keyed: true,
            mapping_hash: "m".into(),
            rows: buckets.iter().map(|b| b.n).sum(),
            buckets,
            control_totals: vec![],
            truncated: false,
            needs_review: false,
        }
    }

    fn cmp(o: &Fingerprint, n: &Fingerprint) -> Comparison {
        compare(SideOutcome::Ok(o), SideOutcome::Ok(n)).unwrap()
    }

    fn many(k: u32) -> Vec<BucketSummary> {
        (0..k).map(|i| b(i, 1, u128::from(i))).collect()
    }

    #[test]
    fn equal_fingerprints_pass() {
        let a = fp(many(20));
        let c = cmp(&a, &a.clone());
        assert!(c.is_pass());
        assert_eq!(c.equal_buckets, 20);
        assert!(c.drilldown_buckets().is_empty());
        let empty = fp(vec![]);
        assert!(cmp(&empty, &empty).is_pass());
    }

    #[test]
    fn differences_are_classified() {
        let o = fp(many(40));
        let mut nb = many(40);
        nb[3].s[2] = 7;
        nb.remove(5);
        nb.push(b(100, 1, 0));
        let c = cmp(&o, &fp(nb));
        assert_eq!(c.verdict, Verdict::Fail);
        assert_eq!(c.differing, vec![3]);
        assert_eq!(c.missing, vec![5]);
        assert_eq!(c.extra, vec![100]);
        assert_eq!(c.drilldown_buckets(), vec![3, 5, 100]);
        assert!(c.reasons.contains(&NotPassReason::BucketsDiffer));
    }

    #[test]
    fn widely_different_stop_rule() {
        // Decision 17: more than 10% AND at least 64 differing buckets.
        let o = fp(many(100));
        let mut nb = many(100);
        for x in nb.iter_mut().take(63) {
            x.s[1] = 1;
        }
        let c = cmp(&o, &fp(nb.clone()));
        assert_eq!(c.verdict, Verdict::Fail); // 63% but only 63 buckets
        assert_eq!(c.drilldown_buckets().len(), 63); // full drill-down
        nb[63].s[1] = 1;
        let c = cmp(&o, &fp(nb));
        assert_eq!(c.verdict, Verdict::WidelyDifferent); // 64
        assert_eq!(c.drilldown_buckets().len(), WIDE_SAMPLE_BUCKETS);
        // 64 differing but exactly 10% of 640 buckets: not widely different.
        let o = fp(many(640));
        let mut nb = many(640);
        for x in nb.iter_mut().take(64) {
            x.s[1] = 1;
        }
        assert_eq!(cmp(&o, &fp(nb.clone())).verdict, Verdict::Fail);
        nb[64].s[1] = 1;
        assert_eq!(cmp(&o, &fp(nb)).verdict, Verdict::WidelyDifferent);
    }

    #[test]
    fn missing_or_extra_buckets_alone_count_towards_the_stop_rule() {
        let base = fp(many(100));
        // 64 buckets only in old, 64 only in new, 64 differing: each alone trips it.
        let mut missing = base.clone();
        missing.buckets.drain(..64);
        missing.rows -= 64;
        let c = cmp(&base, &missing);
        assert_eq!(c.verdict, Verdict::WidelyDifferent);
        assert_eq!(c.missing.len(), 64);
        let c = cmp(&missing, &base);
        assert_eq!(c.verdict, Verdict::WidelyDifferent);
        assert_eq!(c.extra.len(), 64);
        let mut mixed = base.clone();
        mixed.buckets.drain(..32);
        mixed.buckets.extend((1000..1032).map(|i| b(i, 1, 0)));
        let c = cmp(&base, &mixed);
        assert_eq!((c.missing.len(), c.extra.len()), (32, 32));
        assert_eq!(c.verdict, Verdict::WidelyDifferent);
        assert!(c.reasons.contains(&NotPassReason::BucketsDiffer));
    }

    #[test]
    fn needs_review_is_never_a_pass() {
        let a = fp(many(5));
        let mut r = a.clone();
        r.needs_review = true;
        let c = cmp(&a, &r);
        assert_eq!(c.verdict, Verdict::NeedsReview);
        assert_eq!(c.reasons, vec![NotPassReason::NeedsReview]);
        assert_eq!(cmp(&r, &a).verdict, Verdict::NeedsReview);
        let mut d = r.clone();
        d.buckets[0].s[0] += 1;
        assert_eq!(cmp(&r, &d).verdict, Verdict::Fail);
    }

    #[test]
    fn never_pass_when_a_side_errored_or_truncated() {
        let a = fp(many(3));
        for (o, n) in [
            (SideOutcome::Errored, SideOutcome::Ok(&a)),
            (SideOutcome::Ok(&a), SideOutcome::Errored),
            (SideOutcome::Errored, SideOutcome::Errored),
        ] {
            let c = compare(o, n).unwrap();
            assert_eq!(c.verdict, Verdict::Error);
            assert_eq!(c.reasons, vec![NotPassReason::SideErrored]);
        }
        let mut t = a.clone();
        t.truncated = true;
        let c = cmp(&a, &t);
        assert_eq!(c.verdict, Verdict::Error);
        assert_eq!(c.reasons, vec![NotPassReason::Truncated]);
    }

    #[test]
    fn counts_and_totals() {
        let o = fp(vec![b(1, 2, 5)]);
        let n = fp(vec![b(1, 3, 5)]);
        let c = cmp(&o, &n);
        assert!(c.reasons.contains(&NotPassReason::CountsDiffer));
        assert_ne!(c.verdict, Verdict::Pass);
        let mut o = fp(many(20));
        let mut n = o.clone();
        o.control_totals = vec![Decimal::parse("1.10").unwrap()];
        n.control_totals = vec![Decimal::parse("1.1").unwrap()];
        assert!(cmp(&o, &n).is_pass());
        n.control_totals = vec![Decimal::parse("1.2").unwrap()];
        let c = cmp(&o, &n);
        assert_eq!(c.verdict, Verdict::Fail);
        assert_eq!(c.reasons, vec![NotPassReason::ControlTotalsDiffer]);
    }

    #[test]
    fn incompatible_fingerprints_are_errors() {
        let a = fp(many(2));
        let ok = SideOutcome::Ok;
        let mut x = a.clone();
        x.bucket_digits = 4;
        assert_eq!(compare(ok(&a), ok(&x)), Err(CompareError::BucketCount));
        let mut x = a.clone();
        x.mapping_hash = "other".into();
        assert_eq!(compare(ok(&a), ok(&x)), Err(CompareError::MappingHash));
        let mut e = a.clone();
        e.mapping_hash.clear();
        assert_eq!(compare(ok(&e), ok(&e)), Err(CompareError::MappingHash));
        let mut x = a.clone();
        x.keyed = false;
        assert_eq!(compare(ok(&a), ok(&x)), Err(CompareError::Keyed));
        let mut x = a.clone();
        x.control_totals = vec![Decimal::ZERO];
        assert_eq!(compare(ok(&a), ok(&x)), Err(CompareError::ControlTotals));
        let mut x = a.clone();
        x.rows += 1;
        assert!(matches!(
            compare(ok(&a), ok(&x)),
            Err(CompareError::Inconsistent(_))
        ));
        let mut x = a.clone();
        x.buckets.push(x.buckets[0]);
        x.rows += 1;
        assert!(matches!(
            compare(ok(&x), ok(&a)),
            Err(CompareError::Inconsistent(_))
        ));
        let mut x = a.clone();
        x.buckets[0].n = u64::MAX;
        assert!(compare(ok(&a), ok(&x)).is_err());
        assert!(!CompareError::Keyed.to_string().is_empty());
    }

    proptest! {
        #[test]
        fn pass_only_when_identical(
            sums in prop::collection::vec((1u64..5, 0u128..1000), 0..30),
            flip in any::<prop::sample::Index>(),
            delta in 1u128..5,
        ) {
            let o = fp(sums.iter().enumerate().map(|(i, (n, s))| b(i as u32, *n, *s)).collect());
            prop_assert!(cmp(&o, &o.clone()).is_pass());
            if !sums.is_empty() {
                let mut n = o.clone();
                let i = flip.index(sums.len());
                n.buckets[i].s[0] += delta;
                let c = cmp(&o, &n);
                prop_assert!(!c.is_pass());
                prop_assert_eq!(c.differing, vec![i as u32]);
                let r = cmp(&n, &o);
                prop_assert!(!r.is_pass());
            }
        }
    }
}
