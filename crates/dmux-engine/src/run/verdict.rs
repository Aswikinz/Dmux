//! The one place a table verdict and the run verdict are decided.
//!
//! Every comparison path (live, file against live, file against file) turns
//! what it measured into an [`Evidence`] and hands it to a [`Builder`]; the
//! builder calls [`decide`] and folds the results.  Because there is only one
//! decision function, the paths cannot drift apart.
//!
//! ## Table rules, in order
//!
//! 1. **No conclusion** — a scan or plan that failed ([`Outcome::Fault`]) is
//!    TOOL_FAULT; a table missing on the new side is FAIL; a table only on the
//!    new side is WARNING (its rows are not checked).  None of these counts as
//!    checked in the coverage line.
//! 2. **Base verdict by level** — Structure: FAIL if any red structure finding
//!    for the table, else PASS (amber findings are listed; this is never a
//!    value PASS, and the coverage line reports it as "structure only").
//!    CountsAndTotals: PASS only when row counts and every control total are
//!    equal.  FullValues: the comparison core's verdict.
//! 3. **Independent checks** (FullValues) — a spot check that disagrees with the
//!    fingerprint, or a drill-down that cannot explain a differing bucket, is
//!    TOOL_FAULT.
//! 4. **Change detection** (data levels) — a table whose change counters moved
//!    is CHANGED_DURING_CHECK, replacing the data-derived verdict of rules 2–3
//!    (the data was moving, so neither a PASS nor a FAIL of it can be trusted).
//! 5. **SQL_ASCII (P19)** — a full-values PASS on a SQL_ASCII server is
//!    NEEDS_REVIEW: the encoding of the bytes cannot be proven.
//! 6. **P3 column coverage** (data levels) — every column must be paired or
//!    ignored with a reason.  An old column with no counterpart is FAIL (its
//!    values were not carried over); a new column with no counterpart is
//!    NEEDS_REVIEW (its values are not checked).  These combine with the verdict
//!    so far by taking the worse, so rule 4 can never hide them.
//! 7. **Floors** — a path may add a minimum verdict with a reason (for example a
//!    requested control total that a saved fingerprint does not carry).
//!
//! ## Run rules
//!
//! The overall verdict is the worst of every table verdict and every structure
//! finding.  If no table reached a conclusion at its level the run is at least
//! WARNING ("nothing was checked"), never PASS.  On a SQL_ASCII server a run
//! PASS becomes NEEDS_REVIEW.  Exit codes: 0 PASS; 2 WARNING, NEEDS_REVIEW and
//! CHANGED_DURING_CHECK (amber: look again); 1 FAIL, WIDELY_DIFFERENT and
//! TOOL_FAULT.

use std::collections::{BTreeMap, BTreeSet};

use crate::core::compare::{Comparison, NotPassReason};
use crate::core::fingerprint::Decimal;
use crate::results::{
    add_table, apply_coverage, finalise, from_core, overall_verdict, CheckLevel, IgnoredItem,
    RunResults, StructureFinding, TableResult, Verdict,
};

use super::RunOutcome;

/// A count-and-totals comparison (no value hashing).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CountsComparison {
    pub old_rows: u64,
    pub new_rows: u64,
    pub old_totals: Vec<Decimal>,
    pub new_totals: Vec<Decimal>,
}

/// What was measured for a table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Full(Comparison),
    Counts(CountsComparison),
    /// The structure findings that belong to this table.
    Structure(Vec<StructureFinding>),
    MissingOnNew,
    OnlyOnNew,
    /// No decision possible; the reason never carries row data.
    Fault(String),
}

/// The independent spot check's outcome (PLAN.md 3.4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Spot {
    NotRun(String),
    /// Values compared in the app agree with what the fingerprint said.
    Agrees {
        compared: usize,
        differing: usize,
    },
    /// The value comparison contradicts the fingerprint: the tool is suspect.
    Disagrees(String),
}

/// Everything [`decide`] needs for one table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Evidence {
    pub table: String,
    pub level: CheckLevel,
    pub outcome: Outcome,
    /// P3: old columns with no counterpart and no ignore.
    pub dropped: Vec<String>,
    /// P3: new columns with no counterpart and no ignore.
    pub added: Vec<String>,
    /// Columns ignored with a reason (carried into the reasons).
    pub ignored_columns: BTreeMap<String, String>,
    pub sql_ascii: bool,
    pub changed: bool,
    pub spot: Spot,
    pub drill_unexplained: bool,
    pub columns_covered: Option<Vec<String>>,
    pub missing_rows: Option<usize>,
    pub extra_rows: Option<usize>,
    pub changed_rows: Option<usize>,
    /// Informational reasons from the path (no effect on the verdict).
    pub notes: Vec<String>,
    /// Minimum verdicts with their reasons.
    pub floors: Vec<(Verdict, String)>,
}

impl Evidence {
    pub fn new(table: impl Into<String>, level: CheckLevel, outcome: Outcome) -> Self {
        Evidence {
            table: table.into(),
            level,
            outcome,
            dropped: Vec::new(),
            added: Vec::new(),
            ignored_columns: BTreeMap::new(),
            sql_ascii: false,
            changed: false,
            spot: Spot::NotRun("not a keyed full-values table".into()),
            drill_unexplained: false,
            columns_covered: None,
            missing_rows: None,
            extra_rows: None,
            changed_rows: None,
            notes: Vec::new(),
            floors: Vec::new(),
        }
    }
}

fn core_reason(r: &NotPassReason) -> &'static str {
    match r {
        NotPassReason::CountsDiffer => "row counts differ",
        NotPassReason::BucketsDiffer => "bucket summaries differ",
        NotPassReason::ControlTotalsDiffer => "control totals differ",
        NotPassReason::NeedsReview => "a column was compared by an inexact printed form",
        NotPassReason::SideErrored => "a side failed to scan",
        NotPassReason::Truncated => "a side returned a truncated scan",
    }
}

/// A verdict that rests on a finished comparison (counted in the coverage line).
pub fn is_conclusive(v: Verdict) -> bool {
    matches!(
        v,
        Verdict::Pass | Verdict::Fail | Verdict::WidelyDifferent | Verdict::NeedsReview
    )
}

/// Decide one table.  Returns the result and whether it counts as checked.
pub fn decide(e: &Evidence) -> (TableResult, bool) {
    let mut reasons: Vec<String> = Vec::new();
    let mut result = TableResult {
        table: e.table.clone(),
        level: e.level,
        verdict: Verdict::ToolFault,
        old_rows: None,
        new_rows: None,
        columns_covered: e.columns_covered.clone(),
        differing_buckets: None,
        missing_buckets: None,
        extra_buckets: None,
        missing_rows: e.missing_rows,
        extra_rows: e.extra_rows,
        changed_rows: e.changed_rows,
        reasons: Vec::new(),
    };

    // Rules 1 and 2.
    let mut can_conclude = true;
    let mut data_level = false;
    let mut v = match &e.outcome {
        Outcome::Fault(r) => {
            can_conclude = false;
            reasons.push(r.clone());
            Verdict::ToolFault
        }
        Outcome::MissingOnNew => {
            can_conclude = false;
            reasons.push("table missing in the new database".into());
            Verdict::Fail
        }
        Outcome::OnlyOnNew => {
            can_conclude = false;
            reasons.push("table exists only in the new database; its rows are not checked".into());
            Verdict::Warning
        }
        Outcome::Structure(findings) => {
            reasons.push("structure only: values were not compared".into());
            let mut v = Verdict::Pass;
            for f in findings {
                reasons.push(f.message.clone());
                if f.verdict == Verdict::Fail {
                    v = Verdict::Fail;
                }
            }
            v
        }
        Outcome::Counts(c) => {
            data_level = true;
            result.old_rows = Some(c.old_rows);
            result.new_rows = Some(c.new_rows);
            let mut v = Verdict::Pass;
            if c.old_rows != c.new_rows {
                reasons.push(format!(
                    "row counts differ: {} -> {}",
                    c.old_rows, c.new_rows
                ));
                v = Verdict::Fail;
            }
            if c.old_totals != c.new_totals {
                reasons.push("control totals differ".into());
                v = Verdict::Fail;
            }
            reasons.push("counts and totals only: values were not compared".into());
            v
        }
        Outcome::Full(c) => {
            data_level = true;
            result.old_rows = c.old_rows;
            result.new_rows = c.new_rows;
            result.differing_buckets = Some(c.differing.len());
            result.missing_buckets = Some(c.missing.len());
            result.extra_buckets = Some(c.extra.len());
            for r in &c.reasons {
                reasons.push(core_reason(r).to_string());
            }
            from_core(c.verdict)
        }
    };

    if data_level {
        // Rule 3.
        if e.level == CheckLevel::FullValues {
            match &e.spot {
                Spot::Disagrees(r) => {
                    reasons.push(format!("independent spot check disagrees with the fingerprint: {r}"));
                    v = Verdict::ToolFault;
                }
                Spot::Agrees {
                    compared,
                    differing,
                } => reasons.push(format!(
                    "spot check: {compared} rows compared value by value, {differing} differ, consistent with the fingerprint"
                )),
                Spot::NotRun(why) => reasons.push(format!("spot check not run: {why}")),
            }
        }
        if e.drill_unexplained {
            reasons.push("drill-down found no row explaining the differing buckets".into());
            v = Verdict::ToolFault;
        }
        // Rule 4.
        if e.changed {
            reasons.push("the table changed during the check".into());
            v = Verdict::ChangedDuringCheck;
        }
        // Rule 5.
        if e.sql_ascii && e.level == CheckLevel::FullValues && v == Verdict::Pass {
            reasons.push("SQL_ASCII server: the text encoding cannot be proven (P19)".into());
            v = Verdict::NeedsReview;
        }
    }

    // Rule 6 (listed even for faults, so the reader sees the gap).
    if !matches!(
        e.outcome,
        Outcome::Structure(_) | Outcome::MissingOnNew | Outcome::OnlyOnNew
    ) {
        for d in &e.dropped {
            reasons.push(format!(
                "column {d} has no counterpart in the new database and is not ignored"
            ));
            v = v.worst(Verdict::Fail);
        }
        for a in &e.added {
            reasons.push(format!(
                "column {a} exists only in the new database; its values are not checked"
            ));
            v = v.worst(Verdict::NeedsReview);
        }
    }
    for (path, why) in &e.ignored_columns {
        reasons.push(format!("column {path} ignored: {why}"));
    }

    // Rule 7.
    for (floor, why) in &e.floors {
        reasons.push(why.clone());
        v = v.worst(*floor);
    }
    reasons.extend(e.notes.iter().cloned());

    result.verdict = v;
    result.reasons = reasons;
    let checked = can_conclude && is_conclusive(v);
    (result, checked)
}

/// Folds evidence into [`RunResults`].  Shared by every comparison path.
pub struct Builder {
    pub results: RunResults,
    pending: Vec<Evidence>,
    fixed: Vec<(TableResult, bool)>,
    sql_ascii: bool,
}

impl Builder {
    pub fn new(run_id: String, mapping_hash: String, definition_hash: String) -> Self {
        Builder {
            results: RunResults {
                run_id,
                mapping_hash,
                definition_hash,
                ..RunResults::default()
            },
            pending: Vec::new(),
            fixed: Vec::new(),
            sql_ascii: false,
        }
    }

    /// Mark the run as touching a SQL_ASCII server (P19).
    pub fn set_sql_ascii(&mut self, on: bool) {
        self.sql_ascii |= on;
    }

    pub fn sql_ascii(&self) -> bool {
        self.sql_ascii
    }

    /// Record a table's evidence; decided at [`Builder::finish`].
    pub fn record(&mut self, mut e: Evidence) {
        e.sql_ascii |= self.sql_ascii;
        self.pending.push(e);
    }

    /// Record a result reused from the resume store.
    pub fn record_resumed(&mut self, result: TableResult) {
        let checked = is_conclusive(result.verdict);
        self.results.resumed_tables.push(result.table.clone());
        self.fixed.push((result, checked));
    }

    /// The table moved during the run: never a PASS.
    pub fn mark_changed(&mut self, table: &str) {
        let mut hit = false;
        for e in self.pending.iter_mut().filter(|e| e.table == table) {
            e.changed = true;
            hit = true;
        }
        for (r, checked) in self.fixed.iter_mut().filter(|(r, _)| r.table == table) {
            r.verdict = r.verdict.worst(Verdict::ChangedDuringCheck);
            r.reasons
                .push("the table changed during the check (after its stored result)".into());
            *checked = is_conclusive(r.verdict);
            hit = true;
        }
        if hit && !self.results.changed_during_check.iter().any(|t| t == table) {
            self.results.changed_during_check.push(table.to_string());
        }
    }

    /// A whole table ignored with its reason.
    pub fn ignore_table(&mut self, path: &str, reason: &str) {
        self.ignore_item(path, reason);
        self.results.ignored += 1;
    }

    /// A column or other item ignored with its reason (de-duplicated).
    pub fn ignore_item(&mut self, path: &str, reason: &str) {
        let item = IgnoredItem {
            path: path.to_string(),
            reason: reason.to_string(),
        };
        if !self.results.ignored_items.contains(&item) {
            self.results.ignored_items.push(item);
        }
    }

    pub fn has_table(&self, table: &str) -> bool {
        self.pending.iter().any(|e| e.table == table)
            || self.fixed.iter().any(|(r, _)| r.table == table)
    }

    /// Decide every table and the run.
    pub fn finish(mut self) -> RunOutcome {
        let mut not_checked: BTreeSet<String> = BTreeSet::new();
        let mut decided: Vec<(TableResult, bool)> = std::mem::take(&mut self.fixed);
        for e in &self.pending {
            decided.push(decide(e));
        }
        for (r, checked) in decided {
            if !checked {
                not_checked.insert(r.table.clone());
            }
            for (path, why) in self
                .pending
                .iter()
                .filter(|e| e.table == r.table)
                .flat_map(|e| e.ignored_columns.iter())
            {
                let item = IgnoredItem {
                    path: path.clone(),
                    reason: why.clone(),
                };
                if !self.results.ignored_items.contains(&item) {
                    self.results.ignored_items.push(item);
                }
            }
            add_table(&mut self.results, r);
        }

        let r = &mut self.results;
        let (mut full, mut counts, mut structure) = (0, 0, 0);
        for t in r.tables.values() {
            if not_checked.contains(&t.table) || !is_conclusive(t.verdict) {
                continue;
            }
            match t.level {
                CheckLevel::FullValues => full += 1,
                CheckLevel::CountsAndTotals => counts += 1,
                CheckLevel::Structure => structure += 1,
            }
        }
        r.fully_checked = full;
        r.counts_only = counts;
        r.structure_only = structure;
        r.in_scope = r.tables.len() + r.ignored;

        let mut verdicts: Vec<Verdict> = r.tables.values().map(|t| t.verdict).collect();
        verdicts.extend(r.structure_findings.iter().map(|f| f.verdict));
        let mut v = overall_verdict(&verdicts);
        if full + counts + structure == 0 {
            r.warnings
                .push("nothing was checked: no table reached a conclusion at its level".into());
            v = v.worst(Verdict::Warning);
        }
        if self.sql_ascii && v.is_pass() {
            v = Verdict::NeedsReview;
        }
        if !r.changed_during_check.is_empty() {
            r.warnings.push(format!(
                "{} table(s) changed during check",
                r.changed_during_check.len()
            ));
        }
        r.verdict = v;
        let results = finalise(apply_coverage(std::mem::take(r)));
        RunOutcome {
            exit_code: exit_code_for(results.verdict),
            results,
        }
    }
}

/// PLAN.md 4.8 exit codes (see the module documentation).
pub fn exit_code_for(v: Verdict) -> u8 {
    match v {
        Verdict::Pass => 0,
        Verdict::Warning | Verdict::NeedsReview | Verdict::ChangedDuringCheck => 2,
        Verdict::Fail | Verdict::WidelyDifferent | Verdict::ToolFault => 1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::compare::{compare, SideOutcome};
    use crate::core::fingerprint::{BucketSummary, Fingerprint};

    fn fp(buckets: Vec<(u32, u64)>) -> Fingerprint {
        Fingerprint {
            bucket_digits: 3,
            keyed: true,
            mapping_hash: "m".into(),
            rows: buckets.iter().map(|b| b.1).sum(),
            buckets: buckets
                .into_iter()
                .map(|(bucket, n)| BucketSummary {
                    bucket,
                    n,
                    s: [u128::from(bucket); 4],
                })
                .collect(),
            control_totals: vec![],
            truncated: false,
            needs_review: false,
        }
    }

    fn full(pass: bool) -> Outcome {
        let a = fp(vec![(1, 1), (2, 1)]);
        let b = if pass { a.clone() } else { fp(vec![(1, 1)]) };
        Outcome::Full(compare(SideOutcome::Ok(&a), SideOutcome::Ok(&b)).unwrap())
    }

    fn ev(outcome: Outcome) -> Evidence {
        let mut e = Evidence::new("s.t", CheckLevel::FullValues, outcome);
        e.spot = Spot::Agrees {
            compared: 2,
            differing: 0,
        };
        e
    }

    fn verdict(e: &Evidence) -> Verdict {
        decide(e).0.verdict
    }

    fn finish(evidence: Vec<Evidence>) -> RunOutcome {
        let mut b = Builder::new("r".into(), "m".into(), "d".into());
        for e in evidence {
            b.record(e);
        }
        b.finish()
    }

    #[test]
    fn an_empty_run_is_not_a_pass() {
        // Fix 1: zero tables used to be PASS with exit 0.
        let out = finish(vec![]);
        assert_eq!(out.results.verdict, Verdict::Warning);
        assert_eq!(out.exit_code, 2);
        assert!(out
            .results
            .warnings
            .iter()
            .any(|w| w.contains("nothing was checked")));
        // All ignored: still nothing checked.
        let mut b = Builder::new("r".into(), "m".into(), "d".into());
        b.ignore_table("s.t", "out of scope");
        let out = b.finish();
        assert_eq!(out.results.verdict, Verdict::Warning);
        assert_eq!(out.results.in_scope, 1);
        // Only tables that reached no conclusion: still not a PASS-able run.
        let out = finish(vec![ev(Outcome::OnlyOnNew)]);
        assert_eq!(out.results.verdict, Verdict::Warning);
        assert_eq!(out.results.fully_checked, 0);
    }

    #[test]
    fn a_dropped_column_fails_the_table_and_an_added_one_needs_review() {
        // Fix 2: unpaired columns used to be notes that never changed the verdict.
        let mut e = ev(full(true));
        e.dropped = vec!["s.t.gone".into()];
        assert_eq!(verdict(&e), Verdict::Fail);
        let mut e = ev(full(true));
        e.added = vec!["s.t.fresh".into()];
        assert_eq!(verdict(&e), Verdict::NeedsReview);
        // With the column ignored and a reason, the table passes and the reason
        // is carried into the results.
        let mut e = ev(full(true));
        e.ignored_columns
            .insert("s.t.gone".into(), "retired in the new design".into());
        let out = finish(vec![e]);
        assert_eq!(out.results.verdict, Verdict::Pass);
        assert!(out.results.tables["s.t"]
            .reasons
            .iter()
            .any(|r| r.contains("retired in the new design")));
        assert!(out.results.ignored_items.contains(&IgnoredItem {
            path: "s.t.gone".into(),
            reason: "retired in the new design".into()
        }));
    }

    #[test]
    fn change_detection_cannot_hide_a_dropped_column() {
        let mut e = ev(full(true));
        e.changed = true;
        assert_eq!(verdict(&e), Verdict::ChangedDuringCheck);
        e.dropped = vec!["s.t.gone".into()];
        assert_eq!(verdict(&e), Verdict::Fail);
    }

    #[test]
    fn a_data_failure_that_moved_is_changed_during_check() {
        let mut e = ev(full(false));
        assert_eq!(verdict(&e), Verdict::Fail);
        e.changed = true;
        assert_eq!(verdict(&e), Verdict::ChangedDuringCheck);
        e.spot = Spot::Disagrees("moved".into());
        assert_eq!(verdict(&e), Verdict::ChangedDuringCheck);
    }

    #[test]
    fn spot_disagreement_is_a_tool_fault_but_agreement_keeps_the_data_verdict() {
        let mut e = ev(full(true));
        e.spot = Spot::Disagrees("a value differs in bucket 1 the fingerprint called equal".into());
        assert_eq!(verdict(&e), Verdict::ToolFault);
        let mut e = ev(full(false));
        e.spot = Spot::Agrees {
            compared: 1,
            differing: 1,
        };
        assert_eq!(
            verdict(&e),
            Verdict::Fail,
            "a consistent spot check keeps the FAIL"
        );
    }

    #[test]
    fn sql_ascii_is_never_a_clean_pass() {
        let mut e = ev(full(true));
        e.sql_ascii = true;
        assert_eq!(verdict(&e), Verdict::NeedsReview);
        let mut b = Builder::new("r".into(), "m".into(), "d".into());
        b.set_sql_ascii(true);
        b.record(ev(full(true)));
        assert_eq!(b.finish().results.verdict, Verdict::NeedsReview);
    }

    #[test]
    fn levels_are_counted_separately() {
        let structure = Evidence::new("s.a", CheckLevel::Structure, Outcome::Structure(vec![]));
        let counts = Evidence::new(
            "s.b",
            CheckLevel::CountsAndTotals,
            Outcome::Counts(CountsComparison {
                old_rows: 3,
                new_rows: 3,
                old_totals: vec![],
                new_totals: vec![],
            }),
        );
        let missing = Evidence::new("s.c", CheckLevel::FullValues, Outcome::MissingOnNew);
        let fault = Evidence::new(
            "s.d",
            CheckLevel::FullValues,
            Outcome::Fault("SQLSTATE 42P01".into()),
        );
        let out = finish(vec![ev(full(true)), structure, counts, missing, fault]);
        let r = &out.results;
        assert_eq!(
            (r.fully_checked, r.counts_only, r.structure_only),
            (1, 1, 1)
        );
        assert_eq!(r.in_scope, 5);
        assert_eq!(
            r.coverage_line,
            "1 of 5 tables fully checked, 1 counts only, 1 structure only, 0 ignored"
        );
        assert_eq!(r.tables["s.c"].verdict, Verdict::Fail);
        assert_eq!(r.tables["s.d"].verdict, Verdict::ToolFault);
        assert_eq!(r.verdict, Verdict::ToolFault);
        assert_eq!(out.exit_code, 1);
    }

    #[test]
    fn structure_level_follows_its_findings_only() {
        let red = StructureFinding {
            object: "s.a.ix".into(),
            path: vec![],
            verdict: Verdict::Fail,
            message: "index missing in new: ix".into(),
        };
        let amber = StructureFinding {
            verdict: Verdict::Warning,
            message: "type: a -> b".into(),
            ..red.clone()
        };
        let e = Evidence::new(
            "s.a",
            CheckLevel::Structure,
            Outcome::Structure(vec![amber.clone()]),
        );
        assert_eq!(verdict(&e), Verdict::Pass);
        let e = Evidence::new(
            "s.a",
            CheckLevel::Structure,
            Outcome::Structure(vec![amber, red]),
        );
        assert_eq!(verdict(&e), Verdict::Fail);
    }

    #[test]
    fn counts_level_compares_counts_and_totals_only() {
        let mk = |o: u64, n: u64, ot: &str, nt: &str| {
            verdict(&Evidence::new(
                "s.t",
                CheckLevel::CountsAndTotals,
                Outcome::Counts(CountsComparison {
                    old_rows: o,
                    new_rows: n,
                    old_totals: vec![Decimal::parse(ot).unwrap()],
                    new_totals: vec![Decimal::parse(nt).unwrap()],
                }),
            ))
        };
        assert_eq!(mk(3, 3, "1.10", "1.1"), Verdict::Pass);
        assert_eq!(mk(3, 2, "1", "1"), Verdict::Fail);
        assert_eq!(mk(3, 3, "1.00", "1.01"), Verdict::Fail);
    }

    #[test]
    fn records_merge_fail_closed() {
        // Fix 4: a later insert for the same table must not overwrite a FAIL.
        let mut first = ev(full(true));
        first.dropped = vec!["s.t.gone".into()];
        let out = finish(vec![first, ev(full(true))]);
        assert_eq!(out.results.tables["s.t"].verdict, Verdict::Fail);
        assert_eq!(out.results.fully_checked, 1);
    }

    #[test]
    fn mark_changed_applies_to_recorded_and_resumed_tables() {
        let mut b = Builder::new("r".into(), "m".into(), "d".into());
        b.record(ev(full(false)));
        let (resumed, _) = decide(&ev(full(true)));
        b.record_resumed(TableResult {
            table: "s.u".into(),
            ..resumed
        });
        b.mark_changed("s.t");
        b.mark_changed("s.u");
        let out = b.finish();
        assert_eq!(
            out.results.tables["s.t"].verdict,
            Verdict::ChangedDuringCheck
        );
        assert_eq!(
            out.results.tables["s.u"].verdict,
            Verdict::ChangedDuringCheck
        );
        assert_eq!(out.results.changed_during_check, vec!["s.t", "s.u"]);
        assert_eq!(out.results.resumed_tables, vec!["s.u"]);
        assert_eq!(out.exit_code, 2);
    }

    #[test]
    fn a_red_structure_finding_fails_the_run() {
        let mut b = Builder::new("r".into(), "m".into(), "d".into());
        b.record(ev(full(true)));
        b.results.structure_findings.push(StructureFinding {
            object: "database.large_objects".into(),
            path: vec![],
            verdict: Verdict::Fail,
            message: "large object count changed: 1 -> 0".into(),
        });
        assert_eq!(b.finish().results.verdict, Verdict::Fail);
    }
}
