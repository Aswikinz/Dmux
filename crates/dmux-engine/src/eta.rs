//! Live time remaining (PLAN 4.5/4.6): the plan's estimate for every table,
//! corrected by how fast the finished tables really ran.
//!
//! Everything here is pure arithmetic over numbers, so it is unit tested
//! without a database.  The run records the plan once (`LiveProgress::set_plan`)
//! and each finished table (`table_finished`); the desktop app asks for
//! [`LiveProgress::remaining`] on every poll.  No value is ever negative or
//! not-a-number; where nothing can be said the answer is `None`.

use std::collections::{BTreeMap, BTreeSet};
use std::time::Instant;

use crate::estimate::Estimate;

/// The slowest and fastest correction accepted.  Beyond these the measurement
/// is more likely noise (a cached table, a stalled disk) than a real speed.
const MIN_RATIO: f64 = 0.05;
const MAX_RATIO: f64 = 20.0;
/// The plan's own spread, as a fraction of the point value (see
/// `estimate_from_seconds`: half to twice).
const PLAN_LOW: f64 = 0.5;
const PLAN_HIGH: f64 = 1.0;
/// The spread kept once every table has been measured.
const MEASURED_LOW: f64 = 0.15;
const MEASURED_HIGH: f64 = 0.25;
/// Measured tables whose planned time is below this say nothing about speed.
const MIN_PLANNED_SECONDS: f64 = 1.0;
/// Ten years: a ceiling so a silly input cannot overflow.
const MAX_SECONDS: f64 = 315_360_000.0;

/// A finished table: what the plan said and what it really took.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Measured {
    pub planned_seconds: f64,
    pub actual_seconds: f64,
}

/// The remaining time: per table (low, high) in seconds, and in total.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Remaining {
    pub tables: BTreeMap<String, (u64, u64)>,
    pub total: Option<(u64, u64)>,
}

/// Seconds as a whole number: never negative, never NaN, never huge.
fn whole(x: f64) -> u64 {
    if x.is_finite() && x > 0.0 {
        x.min(MAX_SECONDS).round() as u64
    } else {
        0
    }
}

/// The plan's point value for an estimate (the estimate is half to twice it).
pub fn point_seconds(e: Estimate) -> f64 {
    // low + high = (0.5 + 2) x point.
    (e.low_seconds as f64 + e.high_seconds as f64) / 2.5
}

/// How much slower (above 1) or faster (below 1) the finished tables ran than
/// planned, and how much of the plan they cover (0..=1).  `None` until enough
/// planned time has been measured.
pub fn speed_ratio(measured: &[Measured], planned_total: f64) -> Option<(f64, f64)> {
    let planned: f64 = measured.iter().map(|m| m.planned_seconds).sum();
    let actual: f64 = measured.iter().map(|m| m.actual_seconds).sum();
    if !(planned.is_finite() && actual.is_finite())
        || planned < MIN_PLANNED_SECONDS
        || actual <= 0.0
    {
        return None;
    }
    let ratio = (actual / planned).clamp(MIN_RATIO, MAX_RATIO);
    let share = if planned_total.is_finite() && planned_total > 0.0 {
        (planned / planned_total).clamp(0.0, 1.0)
    } else {
        1.0
    };
    Some((ratio, share))
}

/// A table's planned range, corrected by the measured speed.  The spread
/// narrows from the plan's own (half to twice) towards a tight band as more of
/// the plan has been measured.
pub fn corrected(planned: Estimate, correction: Option<(f64, f64)>) -> (u64, u64) {
    let Some((ratio, share)) = correction else {
        return (
            planned.low_seconds,
            planned.high_seconds.max(planned.low_seconds),
        );
    };
    let point = point_seconds(planned) * ratio;
    let low_spread = PLAN_LOW * (1.0 - share) + MEASURED_LOW * share;
    let high_spread = PLAN_HIGH * (1.0 - share) + MEASURED_HIGH * share;
    let low = whole(point * (1.0 - low_spread));
    let high = whole(point * (1.0 + high_spread));
    (low, high.max(low))
}

/// What is left of a running table's range after `elapsed` seconds.  `None`
/// once it has run past its upper bound (the estimate was wrong; unknown).
pub fn running_remaining(range: (u64, u64), elapsed_seconds: f64) -> Option<(u64, u64)> {
    let elapsed = whole(elapsed_seconds);
    if range.1 <= elapsed {
        return None;
    }
    Some((range.0.saturating_sub(elapsed), range.1 - elapsed))
}

/// The run's live numbers, shared between the engine (writer) and the app.
#[derive(Debug, Default)]
pub struct LiveProgress {
    planned: BTreeMap<String, Estimate>,
    measured: Vec<Measured>,
    current: Option<(String, Instant)>,
    window_opens_at: Option<String>,
    notices: Vec<String>,
}

impl LiveProgress {
    /// Record the plan: the estimate of every data table.
    pub fn set_plan(&mut self, planned: BTreeMap<String, Estimate>) {
        self.planned = planned;
    }

    /// A table's scan began.
    pub fn table_started(&mut self, path: &str, at: Instant) {
        self.current = Some((path.to_string(), at));
    }

    /// A table's scan ended after `actual_seconds`.
    pub fn table_finished(&mut self, path: &str, actual_seconds: f64) {
        if self.current.as_ref().is_some_and(|(p, _)| p == path) {
            self.current = None;
        }
        if let Some(e) = self.planned.get(path) {
            if actual_seconds.is_finite() && actual_seconds >= 0.0 {
                self.measured.push(Measured {
                    planned_seconds: point_seconds(*e),
                    actual_seconds,
                });
            }
        }
    }

    /// The run is waiting for its window (`Some("22:00")` = when it opens).
    pub fn set_window_wait(&mut self, opens_at: Option<String>) {
        self.window_opens_at = opens_at;
    }

    /// When the run's window next opens, while the run waits for it.
    pub fn window_opens_at(&self) -> Option<&str> {
        self.window_opens_at.as_deref()
    }

    /// Add a plain-language notice for the person watching the run (once).
    pub fn add_notice(&mut self, text: &str) {
        if !self.notices.iter().any(|n| n == text) {
            self.notices.push(text.to_string());
        }
    }

    /// The notices added so far.
    pub fn notices(&self) -> &[String] {
        &self.notices
    }

    /// The path of the table being scanned.
    pub fn current_table(&self) -> Option<&str> {
        self.current.as_ref().map(|(p, _)| p.as_str())
    }

    /// Time remaining per unfinished table and in total.  `done` is every
    /// table the store holds as finished.  Empty when there is no plan.
    pub fn remaining(&self, done: &BTreeSet<String>, now: Instant) -> Remaining {
        let mut out = Remaining::default();
        if self.planned.is_empty() {
            return out;
        }
        let planned_total: f64 = self.planned.values().map(|e| point_seconds(*e)).sum();
        let correction = speed_ratio(&self.measured, planned_total);
        let mut low = 0u64;
        let mut high = 0u64;
        let mut any = false;
        for (path, est) in &self.planned {
            if done.contains(path) {
                continue;
            }
            let mut range = corrected(*est, correction);
            if let Some((cur, since)) = &self.current {
                if cur == path {
                    let elapsed = now.saturating_duration_since(*since).as_secs_f64();
                    match running_remaining(range, elapsed) {
                        Some(r) => range = r,
                        None => continue,
                    }
                }
            }
            out.tables.insert(path.clone(), range);
            low = low.saturating_add(range.0);
            high = high.saturating_add(range.1);
            any = true;
        }
        if any {
            out.total = Some((low, high.max(low)));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::estimate::estimate_from_seconds;
    use std::time::Duration;

    fn plan(pairs: &[(&str, f64)]) -> BTreeMap<String, Estimate> {
        pairs
            .iter()
            .map(|(p, s)| (p.to_string(), estimate_from_seconds(*s)))
            .collect()
    }

    #[test]
    fn the_point_value_recovers_the_planned_seconds() {
        let p = point_seconds(estimate_from_seconds(100.0));
        assert!((p - 100.0).abs() < 1.0, "{p}");
        assert_eq!(point_seconds(Estimate::zero()), 0.0);
    }

    #[test]
    fn without_measurements_the_plan_is_used_as_is() {
        assert_eq!(corrected(Estimate::new(5, 20), None), (5, 20));
    }

    #[test]
    fn a_slower_than_planned_run_raises_the_estimate_and_narrows_it() {
        let planned = estimate_from_seconds(100.0);
        let m = [Measured {
            planned_seconds: 100.0,
            actual_seconds: 200.0,
        }];
        let c = speed_ratio(&m, 200.0).unwrap();
        assert!((c.0 - 2.0).abs() < 1e-9 && (c.1 - 0.5).abs() < 1e-9);
        let (lo, hi) = corrected(planned, Some(c));
        // Point 200 s, spread between the plan's and the measured band.
        assert!(lo > 100 && hi < 400 && lo < hi, "{lo}..{hi}");
        // Everything measured: a tight band around the point.
        let (lo, hi) = corrected(planned, Some((2.0, 1.0)));
        assert_eq!((lo, hi), (170, 250));
    }

    #[test]
    fn a_faster_run_lowers_the_estimate() {
        let m = [Measured {
            planned_seconds: 100.0,
            actual_seconds: 25.0,
        }];
        let c = speed_ratio(&m, 100.0).unwrap();
        let (lo, hi) = corrected(estimate_from_seconds(100.0), Some(c));
        assert!(hi < 50, "{lo}..{hi}");
    }

    #[test]
    fn tiny_or_odd_measurements_are_ignored_or_clamped() {
        let tiny = [Measured {
            planned_seconds: 0.2,
            actual_seconds: 9.0,
        }];
        assert_eq!(speed_ratio(&tiny, 10.0), None);
        let zero = [Measured {
            planned_seconds: 50.0,
            actual_seconds: 0.0,
        }];
        assert_eq!(speed_ratio(&zero, 50.0), None);
        let nan = [Measured {
            planned_seconds: f64::NAN,
            actual_seconds: 1.0,
        }];
        assert_eq!(speed_ratio(&nan, 50.0), None);
        let wild = [Measured {
            planned_seconds: 10.0,
            actual_seconds: 1e9,
        }];
        assert_eq!(speed_ratio(&wild, 10.0).unwrap().0, MAX_RATIO);
        let fast = [Measured {
            planned_seconds: 10.0,
            actual_seconds: 1e-9,
        }];
        assert_eq!(speed_ratio(&fast, 10.0).unwrap().0, MIN_RATIO);
        // A bad total counts the measured part as the whole plan.
        assert_eq!(speed_ratio(&wild, f64::NAN).unwrap().1, 1.0);
    }

    #[test]
    fn a_running_table_counts_down_and_becomes_unknown_when_overdue() {
        assert_eq!(running_remaining((10, 40), 4.0), Some((6, 36)));
        assert_eq!(running_remaining((10, 40), 15.0), Some((0, 25)));
        assert_eq!(running_remaining((10, 40), 40.0), None);
        assert_eq!(running_remaining((10, 40), 900.0), None);
        assert_eq!(running_remaining((10, 40), f64::NAN), Some((10, 40)));
        assert_eq!(running_remaining((10, 40), -5.0), Some((10, 40)));
    }

    #[test]
    fn whole_never_returns_nonsense() {
        assert_eq!(whole(f64::NAN), 0);
        assert_eq!(whole(f64::INFINITY), 0);
        assert_eq!(whole(-3.0), 0);
        assert_eq!(whole(2.6), 3);
        assert_eq!(whole(1e30), MAX_SECONDS as u64);
    }

    #[test]
    fn no_plan_means_no_numbers() {
        let live = LiveProgress::default();
        assert_eq!(
            live.remaining(&BTreeSet::new(), Instant::now()),
            Remaining::default()
        );
    }

    #[test]
    fn remaining_skips_finished_tables_and_corrects_as_tables_finish() {
        let mut live = LiveProgress::default();
        live.set_plan(plan(&[("s.a", 100.0), ("s.b", 100.0), ("s.c", 100.0)]));
        let now = Instant::now();
        let before = live.remaining(&BTreeSet::new(), now);
        assert_eq!(before.tables.len(), 3);
        let (tl, th) = before.total.unwrap();
        assert!(tl < th);

        // Table a ran twice as slowly as planned.
        live.table_started("s.a", now);
        live.table_finished("s.a", 200.0);
        let done: BTreeSet<String> = ["s.a".to_string()].into();
        let after = live.remaining(&done, now);
        assert_eq!(after.tables.len(), 2);
        assert!(!after.tables.contains_key("s.a"));
        let (_, h2) = after.total.unwrap();
        // Two tables left at about 200 s each: above the first plan's two tables.
        assert!(h2 > before.tables["s.b"].1 * 2 - 100);
        let (l2, _) = after.total.unwrap();
        assert!(l2 > before.tables["s.b"].0 * 2);
    }

    #[test]
    fn the_running_table_shows_what_is_left_of_it() {
        let mut live = LiveProgress::default();
        live.set_plan(plan(&[("s.a", 100.0)]));
        let t0 = Instant::now();
        live.table_started("s.a", t0);
        assert_eq!(live.current_table(), Some("s.a"));
        let r = live.remaining(&BTreeSet::new(), t0 + Duration::from_secs(30));
        let (lo, hi) = r.tables["s.a"];
        assert_eq!((lo, hi), (20, 170));
        // Past the upper bound: unknown, so the table drops out.
        let r = live.remaining(&BTreeSet::new(), t0 + Duration::from_secs(500));
        assert!(r.tables.is_empty() && r.total.is_none());
    }

    #[test]
    fn a_table_without_a_plan_is_not_measured_and_the_window_state_is_kept() {
        let mut live = LiveProgress::default();
        live.set_plan(plan(&[("s.a", 100.0)]));
        live.table_started("s.other", Instant::now());
        live.table_finished("s.other", 5.0);
        assert!(live.measured.is_empty());
        assert_eq!(live.current_table(), None);
        live.table_finished("s.a", f64::NAN);
        assert!(live.measured.is_empty());
        assert_eq!(live.window_opens_at(), None);
        live.set_window_wait(Some("22:00".into()));
        assert_eq!(live.window_opens_at(), Some("22:00"));
        live.set_window_wait(None);
        assert_eq!(live.window_opens_at(), None);
        live.add_notice("hello");
        live.add_notice("hello");
        assert_eq!(live.notices(), ["hello".to_string()]);
    }

    #[test]
    fn all_tables_done_leaves_nothing() {
        let mut live = LiveProgress::default();
        live.set_plan(plan(&[("s.a", 10.0)]));
        let done: BTreeSet<String> = ["s.a".to_string()].into();
        let r = live.remaining(&done, Instant::now());
        assert!(r.tables.is_empty() && r.total.is_none());
    }
}
