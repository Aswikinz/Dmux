//! `dmux compare` over two saved fingerprint files (PLAN.md 3.3).
//!
//! Without a live old database, Dmux reports which tables and how many buckets
//! differ, but cannot list the rows (no drill-down).  This path therefore only
//! compares bucket summaries and control totals, and still refuses to PASS on
//! a missing table, a mapping-hash mismatch, or a truncated/needs-review side.

use std::collections::BTreeMap;

use crate::core::compare::{compare, SideOutcome};
use crate::core::fingerprint::{fingerprint, Fingerprint, Side, Source, TablePlan};
use crate::core::normalise::{ColumnMapping, ColumnOptions};
use crate::db::{probe_server, ConnProfile, ReadOnlySession};
use crate::fingerprint_file::{CapturedTable, FingerprintFile};
use crate::results::{fresh_run_id, CheckLevel, RunResults, TableResult, Verdict, RESULTS_VERSION};

use super::{run_err, RunError, RunOutcome};

/// Compare two fingerprint files (no live database on either side).
pub fn compare_two_files(
    old: &FingerprintFile,
    new: &FingerprintFile,
) -> Result<RunOutcome, RunError> {
    let mapping_hash = old.mapping_hash.clone();
    if old.mapping_hash.is_empty()
        || new.mapping_hash.is_empty()
        || old.mapping_hash != new.mapping_hash
    {
        return Err(RunError::Compare(
            crate::core::compare::CompareError::MappingHash,
        ));
    }

    // Map both files by old_qualified name: the capture records a table's old
    // identity, so matching is by that key.
    let old_by: BTreeMap<_, _> = old
        .tables
        .iter()
        .map(|t| (t.old_qualified.clone(), t))
        .collect();
    let new_by: BTreeMap<_, _> = new
        .tables
        .iter()
        .map(|t| (t.old_qualified.clone(), t))
        .collect();

    let mut results = RunResults {
        results_version: RESULTS_VERSION,
        run_id: fresh_run_id(),
        tool_version: crate::version().to_string(),
        mapping_hash: mapping_hash.clone(),
        verdict: Verdict::Pass,
        coverage_line: String::new(),
        fully_checked: 0,
        counts_only: 0,
        ignored: 0,
        tables: BTreeMap::new(),
        ignored_items: Vec::new(),
        changed_during_check: Vec::new(),
        warnings: Vec::new(),
        wal_start_lsn: None,
        wal_end_lsn: None,
        results_sha256: String::new(),
    };

    if old.server.sql_ascii {
        results
            .warnings
            .push("old capture is SQL_ASCII (P19)".to_string());
    }
    if new.server.sql_ascii {
        results
            .warnings
            .push("new capture is SQL_ASCII (P19)".to_string());
    }

    // Missing tables are reported, never passed.
    for name in old_by.keys() {
        if !new_by.contains_key(name) {
            insert_verdict(
                &mut results,
                name,
                old_by[name].level,
                Verdict::Fail,
                Some(old_by[name].rows),
                None,
                &old_by[name].columns,
                None,
                None,
                None,
                None,
                None,
                None,
                vec!["table missing in the new capture".to_string()],
            );
        }
    }
    for name in new_by.keys() {
        if !old_by.contains_key(name) {
            insert_verdict(
                &mut results,
                name,
                new_by[name].level,
                Verdict::Fail,
                None,
                Some(new_by[name].rows),
                &new_by[name].columns,
                None,
                None,
                None,
                None,
                None,
                None,
                vec!["table missing in the old capture".to_string()],
            );
        }
    }

    // Compare every table present on both sides.
    for (name, old_t) in &old_by {
        let Some(new_t) = new_by.get(name) else {
            continue;
        };
        compare_captured(&mut results, old_t, new_t, &mapping_hash, name)?;
    }

    let verdicts: Vec<Verdict> = results.tables.values().map(|t| t.verdict).collect();
    results.verdict = crate::results::overall_verdict(&verdicts);
    results = crate::results::apply_coverage(results);
    results = crate::results::finalise(results);

    let exit_code = match results.verdict {
        Verdict::Pass => 0,
        Verdict::Warning | Verdict::NeedsReview => 2,
        _ => 1,
    };
    Ok(RunOutcome { results, exit_code })
}

fn compare_captured(
    results: &mut RunResults,
    old: &CapturedTable,
    new: &CapturedTable,
    mapping_hash: &str,
    name: &str,
) -> Result<(), RunError> {
    // Rebuild the core fingerprints from the captured summaries.
    let to_fp = |t: &CapturedTable| Fingerprint {
        bucket_digits: t.bucket_digits,
        keyed: t.keyed,
        mapping_hash: mapping_hash.to_string(),
        rows: t.rows,
        buckets: t.buckets.clone(),
        control_totals: t.control_totals.clone(),
        truncated: false,
        needs_review: t.needs_review,
    };

    // A plan mismatch (different bucket digits or column/key set) is a tool
    // fault, never a PASS.
    if old.bucket_digits != new.bucket_digits
        || old.columns != new.columns
        || old.key_columns != new.key_columns
        || old.keyed != new.keyed
    {
        insert_verdict(
            results,
            name,
            old.level,
            Verdict::ToolFault,
            Some(old.rows),
            Some(new.rows),
            &old.columns,
            None,
            None,
            None,
            None,
            None,
            None,
            vec!["the two captures were built from incompatible plans".to_string()],
        );
        return Ok(());
    }

    let ofp = to_fp(old);
    let nfp = to_fp(new);
    let comparison = compare(SideOutcome::Ok(&ofp), SideOutcome::Ok(&nfp))?;

    let verdict = crate::results::from_core(comparison.verdict);
    let mut reasons = Vec::new();
    if old.needs_review || new.needs_review {
        reasons
            .push("a column was compared by a printed form that is not provably exact".to_string());
    }
    if old.rows != new.rows {
        reasons.push(format!("row count {} -> {}", old.rows, new.rows));
    }
    if old.control_totals != new.control_totals {
        reasons.push("control totals differ".to_string());
    }
    if !comparison.differing.is_empty()
        || !comparison.missing.is_empty()
        || !comparison.extra.is_empty()
    {
        reasons.push(format!(
            "{} differing, {} missing, {} extra buckets",
            comparison.differing.len(),
            comparison.missing.len(),
            comparison.extra.len()
        ));
    }

    insert_verdict(
        results,
        name,
        old.level,
        verdict,
        Some(old.rows),
        Some(new.rows),
        &old.columns,
        Some(comparison.differing.len()),
        Some(comparison.missing.len()),
        Some(comparison.extra.len()),
        None,
        None,
        None,
        reasons,
    );
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn insert_verdict(
    results: &mut RunResults,
    name: &str,
    level: CheckLevel,
    verdict: Verdict,
    old_rows: Option<u64>,
    new_rows: Option<u64>,
    columns_covered: &[String],
    differing: Option<usize>,
    missing: Option<usize>,
    extra: Option<usize>,
    missing_rows: Option<usize>,
    extra_rows: Option<usize>,
    changed_rows: Option<usize>,
    reasons: Vec<String>,
) {
    match level {
        CheckLevel::FullValues => results.fully_checked += 1,
        CheckLevel::CountsAndTotals => results.counts_only += 1,
        CheckLevel::Structure => {}
    }
    results.tables.insert(
        name.to_string(),
        TableResult {
            table: name.to_string(),
            level,
            verdict,
            old_rows,
            new_rows,
            columns_covered: Some(columns_covered.to_vec()),
            differing_buckets: differing,
            missing_buckets: missing,
            extra_buckets: extra,
            missing_rows,
            extra_rows,
            changed_rows,
            reasons,
        },
    );
}

/// Compare a saved fingerprint file (the old side) against a live database (the
/// new side).  The live side is scanned with the exact plan the file captured,
/// and bucket summaries are compared; without a live old side there is no
/// drill-down or spot check (PLAN 3.3).
pub async fn compare_file_to_live(
    old: &FingerprintFile,
    new_profile: &ConnProfile,
    allow_superuser: bool,
) -> Result<RunOutcome, RunError> {
    let mapping_hash = old.mapping_hash.clone();
    let mut new_session = ReadOnlySession::open(new_profile, allow_superuser).await?;
    let new_info = probe_server(new_profile, allow_superuser).await?;

    let mut results = RunResults {
        results_version: RESULTS_VERSION,
        run_id: fresh_run_id(),
        tool_version: crate::version().to_string(),
        mapping_hash: mapping_hash.clone(),
        verdict: Verdict::Pass,
        coverage_line: String::new(),
        fully_checked: 0,
        counts_only: 0,
        ignored: 0,
        tables: BTreeMap::new(),
        ignored_items: Vec::new(),
        changed_during_check: Vec::new(),
        warnings: Vec::new(),
        wal_start_lsn: None,
        wal_end_lsn: None,
        results_sha256: String::new(),
    };
    if old.server.sql_ascii {
        results
            .warnings
            .push("old capture is SQL_ASCII (P19)".to_string());
    }
    if new_info.sql_ascii {
        results
            .warnings
            .push("new database is SQL_ASCII (P19)".to_string());
    }

    for t in &old.tables {
        // Split the qualified name.
        let (schema, name) = match t.old_qualified.split_once('.') {
            Some((s, n)) => (s.to_string(), n.to_string()),
            None => {
                insert_verdict(
                    &mut results,
                    &t.old_qualified,
                    t.level,
                    Verdict::ToolFault,
                    Some(t.rows),
                    None,
                    &t.columns,
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                    vec!["captured table name is not schema-qualified".to_string()],
                );
                continue;
            }
        };

        // Read the live columns and resolve types.
        let live = new_session
            .with_read_only("5s", |to| {
                let s = schema.clone();
                let n = name.clone();
                Box::pin(async move {
                    crate::plan::read_live_table(&*to, &s, &n)
                        .await
                        .map_err(RunError::from)
                        .map_err(run_err)
                })
            })
            .await?;

        // Build the plan: old types from the capture, new types from the live
        // catalogue, keyed by the captured key columns.
        let live_by_name: BTreeMap<_, _> =
            live.columns.iter().map(|c| (c.name.clone(), c)).collect();
        let mut columns: Vec<ColumnMapping> = Vec::new();
        let mut key = Vec::new();
        let key_set: std::collections::BTreeSet<String> = t.key_columns.iter().cloned().collect();
        for (i, col_name) in t.columns.iter().enumerate() {
            let Some(new_c) = live_by_name.get(col_name) else {
                // Column missing on the live side: a structure difference.
                insert_verdict(
                    &mut results,
                    &t.old_qualified,
                    t.level,
                    Verdict::Fail,
                    Some(t.rows),
                    None,
                    &t.columns,
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                    vec![format!("column {col_name} missing in the live database")],
                );
                continue;
            };
            let old_type = t
                .column_types
                .get(i)
                .cloned()
                .unwrap_or(new_c.resolved_type.clone());
            columns.push(ColumnMapping {
                old_name: col_name.clone(),
                new_name: col_name.clone(),
                old_type,
                new_type: new_c.resolved_type.clone(),
                options: ColumnOptions::default(),
                control_total: false,
            });
            if key_set.contains(col_name) {
                key.push(columns.len() - 1);
            }
        }

        let Some(plan) = TablePlan::new(
            columns
                .iter()
                .map(crate::core::normalise::plan_column)
                .collect::<Result<Vec<_>, _>>()?,
            key,
            t.bucket_digits,
        )
        .ok() else {
            continue;
        };
        let src = Source::Table { schema, name };
        let fp = new_session
            .with_read_only("300s", |to| {
                let p = plan.clone();
                let s = src.clone();
                let mh = mapping_hash.clone();
                Box::pin(async move {
                    fingerprint(&*to, &p, Side::New, &s, &mh)
                        .await
                        .map_err(RunError::from)
                        .map_err(run_err)
                })
            })
            .await?;

        let ofp = Fingerprint {
            bucket_digits: t.bucket_digits,
            keyed: t.keyed,
            mapping_hash: mapping_hash.clone(),
            rows: t.rows,
            buckets: t.buckets.clone(),
            control_totals: t.control_totals.clone(),
            truncated: false,
            needs_review: t.needs_review,
        };
        let comparison = compare(SideOutcome::Ok(&ofp), SideOutcome::Ok(&fp))?;
        let verdict = crate::results::from_core(comparison.verdict);
        let mut reasons = Vec::new();
        if t.needs_review {
            reasons.push("a column was compared by an inexact printed form".to_string());
        }
        if t.rows != fp.rows {
            reasons.push(format!("row count {} -> {}", t.rows, fp.rows));
        }
        if t.control_totals != fp.control_totals {
            reasons.push("control totals differ".to_string());
        }
        if !comparison.differing.is_empty()
            || !comparison.missing.is_empty()
            || !comparison.extra.is_empty()
        {
            reasons.push(format!(
                "{} differing, {} missing, {} extra buckets",
                comparison.differing.len(),
                comparison.missing.len(),
                comparison.extra.len()
            ));
        }
        insert_verdict(
            &mut results,
            &t.old_qualified,
            t.level,
            verdict,
            Some(t.rows),
            Some(fp.rows),
            &t.columns,
            Some(comparison.differing.len()),
            Some(comparison.missing.len()),
            Some(comparison.extra.len()),
            None,
            None,
            None,
            reasons,
        );
    }

    let verdicts: Vec<Verdict> = results.tables.values().map(|t| t.verdict).collect();
    results.verdict = crate::results::overall_verdict(&verdicts);
    results = crate::results::apply_coverage(results);
    results = crate::results::finalise(results);
    let exit_code = match results.verdict {
        Verdict::Pass => 0,
        Verdict::Warning | Verdict::NeedsReview => 2,
        _ => 1,
    };
    Ok(RunOutcome { results, exit_code })
}
