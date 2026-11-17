//! `dmux capture`: save one database's fingerprints to a file (PLAN.md 3.3).
//!
//! The capture is a [`FingerprintFile`]: versioned, hashed, untrusted-input
//! parsed.  The mapping must be fixed at capture time (an in-place upgrade keeps
//! every name, so the identity mapping works).

use std::collections::BTreeSet;

use crate::core::fingerprint::{fingerprint, Side, Source, TablePlan};
use crate::db::{probe_server, ConnProfile, ReadOnlySession};
use crate::fingerprint_file::{
    CapturedServer, CapturedTable, FingerprintFile, FINGERPRINT_VERSION,
};
use crate::mapping::Mapping;
use crate::plan::{build_table_spec, read_live_table, row_estimate, TableSpec};
use crate::results::CheckLevel;

use super::{list_tables, map_table, RunError};

/// Capture one database's fingerprints for every in-scope table.
///
/// `profile` is the live database, `level` the check level for every table,
/// `control_total_columns` the control-total columns, `mapping` the (identity)
/// mapping, and `allow_superuser` the superuser override.
pub async fn capture(
    profile: &ConnProfile,
    level: CheckLevel,
    control_total_columns: &BTreeSet<String>,
    mapping: &Mapping,
    allow_superuser: bool,
) -> Result<FingerprintFile, RunError> {
    let info = probe_server(profile, allow_superuser).await?;
    let mut session = ReadOnlySession::open(profile, allow_superuser).await?;
    let tables = list_tables(&mut session).await?;

    // The identity mapping: every table maps to itself.  Rename rules still
    // apply so a documented rename is honoured at capture time.
    let mut captured_tables: Vec<CapturedTable> = Vec::with_capacity(tables.len());

    for (schema, table) in &tables {
        let (new_schema, new_name) = map_table(mapping, schema, table);

        let live = session
            .with_read_only("5s", |to| {
                Box::pin(async move {
                    read_live_table(&*to, schema, table)
                        .await
                        .map_err(RunError::from)
                        .map_err(super::run_err)
                })
            })
            .await?;
        let estimate = session
            .with_read_only("5s", |to| {
                Box::pin(async move {
                    row_estimate(&*to, schema, table)
                        .await
                        .map_err(RunError::from)
                        .map_err(super::run_err)
                })
            })
            .await?;

        let spec: TableSpec = build_table_spec(
            &live,
            &live,
            &new_schema,
            &new_name,
            level,
            mapping,
            control_total_columns,
            estimate,
        );

        // A capture records the plan even for tables whose coverage is
        // incomplete, so compare can report them below PASS; but the fingerprint
        // itself still needs the mapped columns only.
        let key = spec.key.clone();
        let bucket_digits = spec.bucket_digits;

        // Build column plans explicitly so a normalisation error maps to
        // CoreError (and thus RunError) rather than leaking a NormaliseError.
        let mut col_plans = Vec::with_capacity(spec.columns.len());
        for m in &spec.columns {
            col_plans.push(crate::core::normalise::plan_column(m).map_err(RunError::from)?);
        }

        let plan = if col_plans.is_empty() {
            // A table with no mapped columns cannot be fingerprinted; still
            // record it as an empty-keyless placeholder so compare flags it.
            None
        } else {
            Some(TablePlan::new(col_plans, key, bucket_digits)?)
        };

        let (rows, buckets, control_totals, needs_review, keyed, bucket_digits) = match plan {
            Some(p) => {
                let src = Source::Table {
                    schema: schema.clone(),
                    name: table.clone(),
                };
                let mh = mapping.mapping_hash.clone();
                let fp = session
                    .with_read_only("300s", |to| {
                        Box::pin(async move {
                            fingerprint(&*to, &p, Side::Old, &src, &mh)
                                .await
                                .map_err(RunError::from)
                                .map_err(super::run_err)
                        })
                    })
                    .await?;
                (
                    fp.rows,
                    fp.buckets,
                    fp.control_totals,
                    fp.needs_review,
                    fp.keyed,
                    fp.bucket_digits,
                )
            }
            None => (0u64, Vec::new(), Vec::new(), true, false, bucket_digits),
        };

        captured_tables.push(CapturedTable {
            old_qualified: format!("{schema}.{table}"),
            level,
            keyed,
            bucket_digits,
            columns: spec.columns_covered(),
            key_columns: spec
                .key
                .iter()
                .map(|i| spec.columns[*i].old_name.clone())
                .collect(),
            column_types: spec.columns.iter().map(|c| c.old_type.clone()).collect(),
            rows,
            buckets,
            control_totals,
            needs_review,
        });
    }

    Ok(FingerprintFile {
        format_version: FINGERPRINT_VERSION,
        tool_version: crate::version().to_string(),
        mapping_hash: mapping.mapping_hash.clone(),
        server: CapturedServer {
            version_num: info.version_num,
            version_string: info.version_string,
            encoding: info.encoding,
            collation: info.collation,
            sql_ascii: info.sql_ascii,
        },
        tables: captured_tables,
        sha256: String::new(),
    })
}
