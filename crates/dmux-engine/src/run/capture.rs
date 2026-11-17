//! `dmux capture`: save one database's fingerprints to a file (PLAN.md 3.3).
//!
//! The capture is a [`FingerprintFile`]: versioned, hashed, untrusted-input
//! parsed.  It records each table's full plan (columns sorted by name, resolved
//! types, options, key and control-total columns, bucket width) so a later
//! comparison scans the other side with exactly the same plan.  The mapping is
//! fixed at capture time (its hash is recorded); an in-place upgrade keeps
//! every name, so the identity mapping works.  The file also carries the
//! database's full structural inventory (PLAN.md 3.3).
//!
//! Levels: FullValues stores bucket summaries; CountsAndTotals stores the row
//! count and control totals only.  Structure is refused: a fingerprint file
//! holds data summaries, and structure is compared live (`dmux run`) or from
//! inventories (`dmux diff`).

use std::collections::{BTreeMap, BTreeSet};

use crate::core::fingerprint::{Side, Source};
use crate::core::normalise::{plan_column, ColumnOptions};
use crate::db::{probe_server, ConnProfile, ReadOnlySession};
use crate::fingerprint_file::{
    CapturedServer, CapturedTable, FingerprintFile, FINGERPRINT_VERSION,
};
use crate::mapping::Mapping;
use crate::plan::{build_table_spec, core_table_plan, SpecInputs};
use crate::results::CheckLevel;

use super::{inventory, live_table, scan_counts, scan_fingerprint, RunError};

/// What to capture.
#[derive(Debug, Clone)]
pub struct CaptureOptions<'a> {
    pub level: CheckLevel,
    pub control_total_columns: &'a BTreeSet<String>,
    /// Per-column options, by `schema.table.column`.
    pub column_options: &'a BTreeMap<String, ColumnOptions>,
    pub mapping: &'a Mapping,
    pub allow_superuser: bool,
}

/// Capture one database's fingerprints for every in-scope table.  A table
/// that cannot be planned or scanned stops the capture with an error naming
/// it: a capture never silently leaves a table out.
pub async fn capture(
    profile: &ConnProfile,
    opts: &CaptureOptions<'_>,
) -> Result<FingerprintFile, RunError> {
    if opts.level == CheckLevel::Structure {
        return Err(RunError::Config(
            "a fingerprint file holds data summaries; use `dmux run` or `dmux diff` for structure"
                .into(),
        ));
    }
    let info = probe_server(profile, opts.allow_superuser).await?;
    let mut session = ReadOnlySession::open(profile, opts.allow_superuser).await?;
    let inv = inventory(&mut session).await?;
    let ignored_tables: BTreeSet<&str> = opts
        .mapping
        .ignored
        .iter()
        .filter(|i| i.path.matches('.').count() == 1)
        .map(|i| i.path.as_str())
        .collect();

    let mut captured = Vec::new();
    for s in &inv.schemas {
        for t in &s.tables {
            let path = format!("{}.{}", s.name, t.name);
            if ignored_tables.contains(path.as_str()) {
                continue;
            }
            let fail = |why: String| RunError::Config(format!("cannot capture {path}: {why}"));
            let live = live_table(&mut session, &s.name, &t.name)
                .await
                .map_err(fail)?
                .ok_or_else(|| fail("the table disappeared during the capture".into()))?;
            let spec = build_table_spec(
                &live,
                &live,
                SpecInputs {
                    new_schema: &s.name,
                    new_name: &t.name,
                    level: opts.level,
                    mapping: opts.mapping,
                    control_total_columns: opts.control_total_columns,
                    column_options: opts.column_options,
                    estimated_rows: t.n_live_tup.max(0) as u64,
                },
            );
            if spec.columns.is_empty() {
                return Err(fail("no column to capture".into()));
            }
            let mut plans = Vec::with_capacity(spec.columns.len());
            for m in &spec.columns {
                plans.push(plan_column(m).map_err(|e| fail(e.to_string()))?);
            }
            let control_columns: Vec<String> = spec
                .columns
                .iter()
                .zip(plans.iter())
                .filter(|(_, p)| p.old_control.is_some())
                .map(|(m, _)| m.old_name.clone())
                .collect();

            let (rows, buckets, control_totals, needs_review, bucket_digits) = match opts.level {
                CheckLevel::CountsAndTotals => {
                    let controls: Vec<String> =
                        plans.iter().filter_map(|p| p.old_control.clone()).collect();
                    let (rows, totals) = scan_counts(&mut session, &s.name, &t.name, &controls)
                        .await
                        .map_err(fail)?;
                    (rows, Vec::new(), totals, false, spec.bucket_digits)
                }
                _ => {
                    let plan = core_table_plan(&spec).map_err(|e| fail(e.to_string()))?;
                    let src = Source::Table {
                        schema: s.name.clone(),
                        name: t.name.clone(),
                    };
                    let fp = scan_fingerprint(
                        &mut session,
                        &plan,
                        Side::Old,
                        &src,
                        &opts.mapping.mapping_hash,
                    )
                    .await
                    .map_err(fail)?;
                    (
                        fp.rows,
                        fp.buckets,
                        fp.control_totals,
                        fp.needs_review,
                        fp.bucket_digits,
                    )
                }
            };

            captured.push(CapturedTable {
                old_qualified: path,
                level: opts.level,
                keyed: spec.is_keyed(),
                bucket_digits,
                columns: spec.columns_covered(),
                key_columns: spec
                    .key
                    .iter()
                    .map(|i| spec.columns[*i].old_name.clone())
                    .collect(),
                column_types: spec.columns.iter().map(|c| c.old_type.clone()).collect(),
                options: spec.columns.iter().map(|c| c.options.clone()).collect(),
                control_columns,
                rows,
                buckets,
                control_totals,
                needs_review,
            });
        }
    }

    let file = FingerprintFile {
        format_version: FINGERPRINT_VERSION,
        tool_version: crate::version().to_string(),
        mapping_hash: opts.mapping.mapping_hash.clone(),
        server: CapturedServer {
            version_num: info.version_num,
            version_string: info.version_string,
            encoding: info.encoding,
            collation: info.collation,
            sql_ascii: info.sql_ascii,
        },
        tables: captured,
        // The full structure, so a saved-file comparison checks indexes,
        // constraints, sequences and large objects like a live run.
        inventory: Some(Box::new(inv)),
        sha256: String::new(),
    };
    // A capture must itself be a valid file before anyone relies on it.
    file.validate()
        .map_err(|e| RunError::Config(format!("the capture is not consistent: {e}")))?;
    Ok(file)
}
