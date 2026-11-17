//! Comparisons that involve a saved fingerprint file (PLAN.md 3.3): two files,
//! or one file and a live database.
//!
//! Both record [`Evidence`] and go through the same [`Builder`] and decision
//! function as the live run, so P3, P19, change detection, fail-closed merging
//! and the run rules are identical.  What a saved file cannot give is noted on
//! the table: there is no drill-down and no spot check without a live old side.
//!
//! The file's own plan (columns, types, options, key and control-total
//! columns, bucket width) is what its hashes mean, so the live side is scanned
//! with exactly that plan.  A difference the plan cannot absorb (a captured
//! column missing live, a changed type) is a TOOL_FAULT for that table, never a
//! guess; a run option the file cannot honour (a control total it did not
//! capture, different options) lowers the table to NEEDS_REVIEW.
//!
//! Structure is compared exactly as in a live run ([`structure::apply`]): the
//! file's captured inventory against the other file's, or against the live
//! database's.  Every red finding (missing index, NOT VALID constraint,
//! sequence behind, large object missing, ...) fails the run.  A file without
//! an inventory floors the run at WARNING ("structure not compared").

use std::collections::{BTreeMap, BTreeSet};

use crate::consistency::{change_detection_floor, SideChangeDetection};
use crate::core::compare::{compare, SideOutcome};
use crate::core::fingerprint::{Fingerprint, Side, Source, TablePlan};
use crate::core::normalise::{plan_column, ColumnMapping};
use crate::db::{probe_server, ConnProfile, ReadOnlySession};
use crate::fingerprint_file::{CapturedTable, FingerprintFile};
use crate::inventory::DatabaseInventory;
use crate::mapping::Mapping;
use crate::plan::{build_table_spec, unknown_keys, LiveColumn, LiveTable, SpecInputs};
use crate::results::{fresh_run_id, CheckLevel, ColumnNote, ConnectionInfo, Connections, Verdict};

use super::structure;
use super::verdict::{Builder, CountsComparison, Evidence, Outcome, Spot};
use super::{
    apply_replica, checkpoint, connection_info, connection_warnings, counters, definition_hash,
    epoch, epoch_changed, foreign_tables_set, inventory, live_table, map_table, note_paused,
    register, replica, replica_outcome, scan_counts, scan_fingerprint, settle, track_counts_on,
    wal, HashMode, Input, RunConfig, RunError, RunOutcome,
};

/// The run warning when a side has no inventory to compare.
pub const STRUCTURE_NOT_COMPARED: &str =
    "structure not compared: a fingerprint file carries no inventory";

/// The structure step of a saved-file comparison, or the WARNING floor when
/// either inventory is absent.
fn file_structure(
    b: &mut Builder,
    old: Option<&DatabaseInventory>,
    new: Option<&DatabaseInventory>,
    mapping: &Mapping,
) {
    match (old, new) {
        (Some(o), Some(n)) => {
            structure::apply(b, o, n, mapping);
        }
        _ => b.floor_run(Verdict::Warning, STRUCTURE_NOT_COMPARED),
    }
}

const NO_LIVE: &str =
    "saved fingerprints: no row-level drill-down or spot check without a live old database";

fn invalid(e: crate::fingerprint_file::FingerprintFileError) -> RunError {
    RunError::Config(format!("the fingerprint file is not usable: {e}"))
}

/// Compare two fingerprint files with a default run configuration.
pub fn compare_two_files(
    old: &FingerprintFile,
    new: &FingerprintFile,
) -> Result<RunOutcome, RunError> {
    compare_two_files_with(
        old,
        new,
        &RunConfig::new(Input::File(old.clone()), Input::File(new.clone())),
    )
}

/// Compare two fingerprint files (no live database on either side).
pub fn compare_two_files_with(
    old: &FingerprintFile,
    new: &FingerprintFile,
    config: &RunConfig,
) -> Result<RunOutcome, RunError> {
    old.validate().map_err(invalid)?;
    new.validate().map_err(invalid)?;
    if old.mapping_hash.is_empty() || old.mapping_hash != new.mapping_hash {
        return Err(RunError::Compare(
            crate::core::compare::CompareError::MappingHash,
        ));
    }

    let mut b = Builder::new(
        fresh_run_id(),
        old.mapping_hash.clone(),
        definition_hash(config),
    );
    b.set_sql_ascii(old.server.sql_ascii || new.server.sql_ascii);
    // Both inputs were capture files: record their SHA-256s for page 1 of the
    // report (audit M1).  A file built in memory has no hash, so record None.
    b.results.capture_sha256_old = (!old.sha256.is_empty()).then(|| old.sha256.clone());
    b.results.capture_sha256_new = (!new.sha256.is_empty()).then(|| new.sha256.clone());
    // Same rules version (checked by validate), other tool version: a warning.
    b.results.warnings.extend(old.tool_version_warning("old"));
    b.results.warnings.extend(new.tool_version_warning("new"));
    if old.server.sql_ascii {
        b.results
            .warnings
            .push("old capture is SQL_ASCII (P19)".into());
    }
    if new.server.sql_ascii {
        b.results
            .warnings
            .push("new capture is SQL_ASCII (P19)".into());
    }

    let old_by: BTreeMap<&str, &CapturedTable> = old
        .tables
        .iter()
        .map(|t| (t.old_qualified.as_str(), t))
        .collect();
    let new_by: BTreeMap<&str, &CapturedTable> = new
        .tables
        .iter()
        .map(|t| (t.old_qualified.as_str(), t))
        .collect();
    for (name, t) in &old_by {
        match new_by.get(name) {
            None => b.record(Evidence::new(*name, t.level, Outcome::MissingOnNew)),
            Some(n) => b.record(captured_pair(name, t, n, &old.mapping_hash)),
        }
    }
    for (name, t) in new_by.iter().filter(|(n, _)| !old_by.contains_key(*n)) {
        b.record(Evidence::new(*name, t.level, Outcome::OnlyOnNew));
    }

    // The run's mapping is used for the structure diff only when it is the one
    // the captures were made with; otherwise names are compared as they are.
    let identity = Mapping::new();
    let mapping = if config.mapping.mapping_hash == old.mapping_hash {
        &config.mapping
    } else {
        if !old.mapping_hash.is_empty() && old.mapping_hash != identity.mapping_hash {
            b.results.warnings.push(
                "the captures' mapping is not available here; structure was compared by name"
                    .into(),
            );
        }
        &identity
    };
    file_structure(
        &mut b,
        old.inventory.as_deref(),
        new.inventory.as_deref(),
        mapping,
    );
    Ok(b.finish())
}

/// The fields of a captured plan that must be equal for its hashes to compare.
fn plan_difference(o: &CapturedTable, n: &CapturedTable) -> Option<&'static str> {
    if o.level != n.level {
        Some("the two captures were made at different check levels")
    } else if o.column_types != n.column_types {
        Some("a column type differs between the captures")
    } else if o.options != n.options {
        Some("the column options differ between the captures")
    } else if o.key_columns != n.key_columns || o.keyed != n.keyed {
        Some("the key columns differ between the captures")
    } else if o.control_columns != n.control_columns {
        Some("the control-total columns differ between the captures")
    } else if o.bucket_digits != n.bucket_digits {
        Some("the captures use different bucket widths")
    } else {
        None
    }
}

fn captured_fp(t: &CapturedTable, mapping_hash: &str) -> Fingerprint {
    Fingerprint {
        bucket_digits: t.bucket_digits,
        keyed: t.keyed,
        mapping_hash: mapping_hash.to_string(),
        rows: t.rows,
        buckets: t.buckets.clone(),
        control_totals: t.control_totals.clone(),
        truncated: false,
        // Never the stored flag alone: a file is untrusted input, so the
        // flag is recomputed from its own types and options (audit H3).
        needs_review: t.effective_needs_review(),
    }
}

/// Per-column notes reconstructed from a captured table's own plan (its ticked
/// options and type warnings), in stable (column, kind) order.  The two
/// captures share one plan (enforced by [`plan_difference`]), so one side
/// yields the notes (H2 / decision 23).
fn captured_column_notes(cap: &CapturedTable) -> Vec<ColumnNote> {
    let mut notes = Vec::new();
    for (i, cname) in cap.columns.iter().enumerate() {
        let m = ColumnMapping {
            old_name: cname.clone(),
            new_name: cname.clone(),
            old_type: cap.column_types[i].clone(),
            new_type: cap.column_types[i].clone(),
            options: cap.options[i].clone(),
            control_total: cap.control_columns.contains(cname),
        };
        if let Ok(cp) = plan_column(&m) {
            notes.extend(crate::results::column_notes(cname, &cp.flags));
        }
    }
    notes.sort_by(|a, b| a.column.cmp(&b.column).then_with(|| a.kind.cmp(&b.kind)));
    notes
}

fn captured_pair(name: &str, o: &CapturedTable, n: &CapturedTable, mapping_hash: &str) -> Evidence {
    let mut e = Evidence::new(name, o.level, Outcome::Fault(String::new()));
    e.columns_covered = Some(o.columns.clone());
    e.notes.push(NO_LIVE.into());
    e.spot = Spot::NotRun("no live database".into());
    e.column_notes = captured_column_notes(o);
    if o.columns != n.columns {
        let oc: BTreeSet<&String> = o.columns.iter().collect();
        let nc: BTreeSet<&String> = n.columns.iter().collect();
        e.dropped = oc.difference(&nc).map(|c| format!("{name}.{c}")).collect();
        e.added = nc.difference(&oc).map(|c| format!("{name}.{c}")).collect();
        e.outcome = Outcome::Fault(
            "the two captures hash different column sets, so their values cannot be compared from saved files"
                .into(),
        );
        return e;
    }
    if let Some(why) = plan_difference(o, n) {
        e.outcome = Outcome::Fault(why.into());
        return e;
    }
    e.outcome = match o.level {
        CheckLevel::CountsAndTotals => Outcome::Counts(CountsComparison {
            old_rows: o.rows,
            new_rows: n.rows,
            old_totals: o.control_totals.clone(),
            new_totals: n.control_totals.clone(),
        }),
        _ => match compare(
            SideOutcome::Ok(&captured_fp(o, mapping_hash)),
            SideOutcome::Ok(&captured_fp(n, mapping_hash)),
        ) {
            Ok(c) => Outcome::Full(c),
            Err(err) => Outcome::Fault(format!("the captures cannot be compared: {err}")),
        },
    };
    e
}

/// Compare a saved fingerprint file against a live database.  `file_side` says
/// which side the file is (`Side::Old`: the file is the old database).
pub async fn compare_file_to_live(
    file: &FingerprintFile,
    file_side: Side,
    live_profile: &ConnProfile,
    config: &RunConfig,
) -> Result<RunOutcome, RunError> {
    file.validate().map_err(invalid)?;
    if file.mapping_hash != config.mapping.mapping_hash {
        return Err(RunError::Config(format!(
            "the fingerprint file was captured with mapping {}, but this run uses mapping {}",
            file.mapping_hash, config.mapping.mapping_hash
        )));
    }
    let mapping = &config.mapping;
    checkpoint(config)?;
    let info = probe_server(live_profile, config.allow_superuser).await?;
    checkpoint(config)?;
    let mut s = ReadOnlySession::open(live_profile, config.allow_superuser).await?;
    register(config, &s)?;

    let mut b = Builder::new(
        fresh_run_id(),
        file.mapping_hash.clone(),
        definition_hash(config),
    );
    b.set_sql_ascii(file.server.sql_ascii || info.sql_ascii);
    // md5 unavailable on the live server (FIPS): hash it in the app (PLAN 2.2).
    let live_mode = HashMode::of(info.md5_available);
    let live_side_name = match file_side {
        Side::Old => "new",
        Side::New => "old",
    };
    if let Some(w) = super::app_hash_warning(live_side_name, live_mode) {
        b.results.warnings.push(w);
    }
    if file.server.sql_ascii {
        b.results
            .warnings
            .push("the captured database is SQL_ASCII (P19)".into());
    }
    if info.sql_ascii {
        b.results
            .warnings
            .push("the live database is SQL_ASCII (P19)".into());
    }
    // The capture file's SHA-256 and the live side's connection-safety record
    // reach page 1 of the report (audit M1 / M5).
    if file_side == Side::Old && !file.sha256.is_empty() {
        b.results.capture_sha256_old = Some(file.sha256.clone());
    }
    if file_side == Side::New && !file.sha256.is_empty() {
        b.results.capture_sha256_new = Some(file.sha256.clone());
    }
    let file_side_name = match file_side {
        Side::Old => "old",
        Side::New => "new",
    };
    b.results
        .warnings
        .extend(file.tool_version_warning(file_side_name));
    let live_conn = connection_info(
        live_profile,
        &info,
        config.allow_superuser_reason.as_deref(),
    );
    let (old_conn, new_conn) = match file_side {
        Side::Old => (ConnectionInfo::default(), live_conn),
        Side::New => (live_conn, ConnectionInfo::default()),
    };
    connection_warnings(&mut b.results, &old_conn, &new_conn);
    b.results.connections = Connections {
        old: old_conn,
        new: new_conn,
    };
    b.results.wal_start_lsn = wal(&mut s, info.version_num).await;
    let epoch_start = epoch(&mut s).await;

    let live_inv = inventory(&mut s).await;
    checkpoint(config)?;
    let live_inv = live_inv?;
    // Change-detection availability on the live side (M3): when `track_counts`
    // is off the counters never move, and foreign tables have no counters.
    let live_track_counts = track_counts_on(&mut s).await;
    let live_foreign = foreign_tables_set(&mut s).await?;
    // Structure: the captured inventory against the live one, oriented.
    match file_side {
        Side::Old => file_structure(&mut b, file.inventory.as_deref(), Some(&live_inv), mapping),
        Side::New => file_structure(&mut b, Some(&live_inv), file.inventory.as_deref(), mapping),
    }
    let live: BTreeSet<String> = live_inv
        .schemas
        .iter()
        .flat_map(|sc| {
            sc.tables
                .iter()
                .map(move |t| format!("{}.{}", sc.name, t.name))
        })
        .collect();
    let captured: BTreeMap<&str, &CapturedTable> = file
        .tables
        .iter()
        .map(|t| (t.old_qualified.as_str(), t))
        .collect();
    // L4: a level, column-option or ignore key that names no table or column on
    // either side is a configuration error, never a silent no-op.
    let mut known_tables: BTreeSet<String> = live.clone();
    let mut known_columns: BTreeSet<String> = BTreeSet::new();
    for sc in &live_inv.schemas {
        for t in &sc.tables {
            for c in &t.columns {
                known_columns.insert(format!("{}.{}.{}", sc.name, t.name, c.name));
            }
        }
    }
    for t in &file.tables {
        known_tables.insert(t.old_qualified.clone());
        for c in &t.columns {
            known_columns.insert(format!("{}.{}", t.old_qualified, c));
        }
    }
    let unknown = unknown_keys(
        &config.levels,
        &config.column_options,
        &mapping.ignored,
        &known_tables,
        &known_columns,
        &known_tables,
        &known_columns,
    );
    if !unknown.is_empty() {
        return Err(RunError::Config(format!(
            "unknown paths in the run definition or mapping: {}",
            unknown.join(", ")
        )));
    }
    let table_ignores: BTreeMap<&str, &str> = mapping
        .ignored
        .iter()
        .filter(|i| i.path.matches('.').count() == 1)
        .map(|i| (i.path.as_str(), i.reason.as_str()))
        .collect();
    let split = |p: &str| -> (String, String) {
        let (a, c) = p.split_once('.').unwrap_or((p, ""));
        (a.to_string(), c.to_string())
    };

    // (old path, captured table, live (schema, table)).
    let mut pairs: Vec<(String, &CapturedTable, (String, String))> = Vec::new();
    let mut targets = BTreeSet::new();
    let mut mapped: Vec<(String, String)> = Vec::new();
    let old_names: Vec<String> = match file_side {
        Side::Old => captured.keys().map(|k| k.to_string()).collect(),
        Side::New => live.iter().cloned().collect(),
    };
    for old_path in &old_names {
        let (os, ot) = split(old_path);
        let (ns, nt) = map_table(mapping, &os, &ot);
        let new_path = format!("{ns}.{nt}");
        targets.insert(new_path.clone());
        if let Some(reason) = table_ignores.get(old_path.as_str()) {
            b.ignore_table(old_path, reason);
            continue;
        }
        mapped.push((old_path.clone(), new_path.clone()));
        let (cap, live_name) = match file_side {
            Side::Old => (
                captured.get(old_path.as_str()).copied(),
                live.contains(&new_path).then_some((ns, nt)),
            ),
            Side::New => (
                captured.get(new_path.as_str()).copied(),
                Some((os.clone(), ot.clone())),
            ),
        };
        match (cap, live_name) {
            (Some(c), Some(l)) => pairs.push((old_path.clone(), c, l)),
            (Some(c), None) => b.record(Evidence::new(old_path, c.level, Outcome::MissingOnNew)),
            (None, _) => b.record(Evidence::new(
                old_path,
                config.default_level,
                Outcome::MissingOnNew,
            )),
        }
    }
    // M2: refuse two old tables mapping to one new table (the mapping does not
    // say which old table the new one is, so neither may PASS against it).
    let duplicates = crate::mapping::duplicate_table_targets(&mapped);
    if !duplicates.is_empty() {
        let mapped_by_old: BTreeMap<&str, &str> = mapped
            .iter()
            .map(|(o, n)| (o.as_str(), n.as_str()))
            .collect();
        let mut kept = Vec::with_capacity(pairs.len());
        for (old_path, c, l) in pairs {
            let dup = mapped_by_old
                .get(old_path.as_str())
                .and_then(|np| duplicates.get(*np));
            if let Some(olds) = dup {
                b.record(Evidence::new(
                    &old_path,
                    c.level,
                    Outcome::Fault(format!(
                        "{} old tables ({}) map to new table {}",
                        if olds.len() == 2 { "two" } else { "multiple" },
                        olds.join(", "),
                        mapped_by_old[old_path.as_str()]
                    )),
                ));
            } else {
                kept.push((old_path, c, l));
            }
        }
        pairs = kept;
    }
    let new_only: Vec<String> = match file_side {
        Side::Old => live
            .iter()
            .filter(|p| !targets.contains(*p))
            .cloned()
            .collect(),
        Side::New => captured
            .keys()
            .filter(|p| !targets.contains(**p))
            .map(|p| p.to_string())
            .collect(),
    };
    for p in new_only {
        if let Some(reason) = table_ignores.get(p.as_str()) {
            b.ignore_table(&p, reason);
        } else {
            b.record(Evidence::new(&p, config.default_level, Outcome::OnlyOnNew));
        }
    }

    // Change detection on the live side, around all scans.
    let mut before = BTreeMap::new();
    for (path, _, (ls, lt)) in &pairs {
        checkpoint(config)?;
        before.insert(path.clone(), counters(&mut s, ls, lt).await.ok());
    }
    let live_side = match file_side {
        Side::Old => "new",
        Side::New => "old",
    };
    for (path, cap, (ls, lt)) in &pairs {
        checkpoint(config)?;
        let r_before = replica(&mut s, info.version_num).await;
        let mut e = file_live_pair(
            config,
            &mut s,
            path,
            cap,
            ls,
            lt,
            file_side,
            &file.mapping_hash,
            live_mode,
        )
        .await;
        // A stop during the scans is never recorded as the table's result.
        checkpoint(config)?;
        // Change detection unavailable on the live side (track_counts off or a
        // foreign table): floor the table at NEEDS_REVIEW (M3).  The file side
        // is a frozen capture, so only the live side's counters are in question.
        let live_cd = SideChangeDetection::of(
            live_track_counts,
            live_foreign.contains(&format!("{ls}.{lt}")),
        );
        let cd_floor = match file_side {
            Side::Old => change_detection_floor(SideChangeDetection::Ok, live_cd),
            Side::New => change_detection_floor(live_cd, SideChangeDetection::Ok),
        };
        if let Some((floor, reason)) = cd_floor {
            e.floors.push((floor, reason));
        }
        let d = replica_outcome(&r_before, &replica(&mut s, info.version_num).await);
        note_paused(&mut b.results, live_side, &d);
        apply_replica(&mut e, live_side, &d);
        b.record(e);
    }
    // Let writes made during the run reach the statistics, then sweep.
    settle(config, config.stats_settle).await?;
    let epoch_moved = epoch_changed(&epoch_start, &epoch(&mut s).await);
    if epoch_moved {
        b.results.warnings.push(
            "the live server restarted or its statistics were reset during the run; changes cannot be ruled out"
                .into(),
        );
    }
    for (path, _, (ls, lt)) in &pairs {
        checkpoint(config)?;
        let after = counters(&mut s, ls, lt).await.ok();
        let moved = match (before.get(path).cloned().flatten(), after) {
            (Some(x), Some(y)) => x.moved(&y),
            _ => true,
        };
        if epoch_moved || moved {
            b.mark_changed(path);
        }
    }
    b.results.wal_end_lsn = wal(&mut s, info.version_num).await;
    checkpoint(config)?;
    Ok(b.finish())
}

/// The captured side of a pair as a [`LiveTable`], so it can be planned with
/// the same pairing rules as two live tables.
fn as_live(cap: &CapturedTable) -> LiveTable {
    let (schema, name) = cap
        .old_qualified
        .split_once('.')
        .unwrap_or(("", &cap.old_qualified));
    LiveTable {
        schema: schema.to_string(),
        name: name.to_string(),
        columns: cap
            .columns
            .iter()
            .zip(cap.column_types.iter())
            .map(|(n, t)| LiveColumn {
                name: n.clone(),
                oid: 0,
                typmod: -1,
                nullable: true,
                attnum: 0,
                resolved_type: t.clone(),
            })
            .collect(),
        primary_key: cap.key_columns.clone(),
    }
}

#[allow(clippy::too_many_arguments)]
async fn file_live_pair(
    config: &RunConfig,
    s: &mut ReadOnlySession,
    path: &str,
    cap: &CapturedTable,
    live_schema: &str,
    live_name: &str,
    file_side: Side,
    mapping_hash: &str,
    live_mode: HashMode,
) -> Evidence {
    let level = cap.level;
    let fault = |why: String| {
        let mut e = Evidence::new(path, level, Outcome::Fault(why));
        e.notes.push(NO_LIVE.into());
        e
    };
    let live_t = match live_table(s, live_schema, live_name).await {
        Ok(Some(t)) => t,
        Ok(None) => {
            return fault("the table disappeared from the live database during the run".into())
        }
        Err(e) => return fault(format!("the live table could not be read: {e}")),
    };
    let cap_t = as_live(cap);
    let (old_t, new_t) = match file_side {
        Side::Old => (&cap_t, &live_t),
        Side::New => (&live_t, &cap_t),
    };
    let spec = build_table_spec(
        old_t,
        new_t,
        SpecInputs {
            new_schema: &new_t.schema,
            new_name: &new_t.name,
            level,
            mapping: &config.mapping,
            control_total_columns: &config.control_total_columns,
            column_options: &config.column_options,
            estimated_rows: 0,
        },
    );
    // M2: two old columns pairing with the same new column is refused.
    if let Some(dup) = crate::plan::duplicate_column_target(&spec) {
        return fault(dup.to_string());
    }
    let mut e = Evidence::new(path, level, Outcome::Fault(String::new()));
    e.dropped = spec.dropped.clone();
    e.added = spec.added.clone();
    e.ignored_columns = spec.ignored_columns.clone();
    e.columns_covered = Some(cap.columns.clone());
    e.notes.push(NO_LIVE.into());
    e.spot = Spot::NotRun("only one side is live".into());

    // Captured column name -> (live column name, live type).
    let mut live_of: BTreeMap<&str, (&str, &crate::core::normalise::PgType)> = BTreeMap::new();
    for m in &spec.columns {
        let (cap_name, live_n, live_ty) = match file_side {
            Side::Old => (m.old_name.as_str(), m.new_name.as_str(), &m.new_type),
            Side::New => (m.new_name.as_str(), m.old_name.as_str(), &m.old_type),
        };
        live_of.insert(cap_name, (live_n, live_ty));
    }

    let mut cols = Vec::with_capacity(cap.columns.len());
    for (i, cname) in cap.columns.iter().enumerate() {
        let Some((lname, lty)) = live_of.get(cname.as_str()) else {
            e.outcome = Outcome::Fault(format!(
                "the saved fingerprint covers column {cname}, which has no counterpart in the live database"
            ));
            return e;
        };
        if **lty != cap.column_types[i] {
            e.outcome = Outcome::Fault(format!(
                "the type of column {cname} changed since the capture; a saved fingerprint cannot compare it (run both sides live)"
            ));
            return e;
        }
        cols.push(ColumnMapping {
            old_name: lname.to_string(),
            new_name: lname.to_string(),
            old_type: (*lty).clone(),
            new_type: (*lty).clone(),
            options: cap.options[i].clone(),
            control_total: cap.control_columns.contains(cname),
        });
        // Run options the file cannot honour lower the table, with the reason.
        let old_name = match file_side {
            Side::Old => cname.as_str(),
            Side::New => lname,
        };
        let opt_path = format!("{}.{}.{}", old_t.schema, old_t.name, old_name);
        if let Some(wanted) = config.column_options.get(&opt_path) {
            if *wanted != cap.options[i] {
                e.floors.push((
                    Verdict::NeedsReview,
                    format!("options for column {cname} differ from the capture; the capture's options were used"),
                ));
            }
        }
        if config.control_total_columns.contains(old_name) && !cap.control_columns.contains(cname) {
            e.floors.push((
                Verdict::NeedsReview,
                format!("a control total on column {cname} was requested but the saved fingerprint does not carry it"),
            ));
        }
    }
    let key: Vec<usize> = cap
        .columns
        .iter()
        .enumerate()
        .filter(|(_, c)| cap.key_columns.contains(c))
        .map(|(i, _)| i)
        .collect();
    let mut plans = Vec::with_capacity(cols.len());
    let mut column_notes = Vec::with_capacity(cols.len());
    for (c, cname) in cols.iter().zip(cap.columns.iter()) {
        match plan_column(c) {
            Ok(p) => {
                column_notes.extend(crate::results::column_notes(cname, &p.flags));
                plans.push(p);
            }
            Err(err) => {
                e.outcome =
                    Outcome::Fault(format!("column {} cannot be planned: {err}", c.old_name));
                return e;
            }
        }
    }
    column_notes.sort_by(|a, b| a.column.cmp(&b.column).then_with(|| a.kind.cmp(&b.kind)));
    e.column_notes = column_notes;
    let controls: Vec<String> = plans.iter().filter_map(|p| p.old_control.clone()).collect();
    let src = Source::Table {
        schema: live_schema.to_string(),
        name: live_name.to_string(),
    };
    let orient = |captured: Fingerprint, live: Fingerprint| match file_side {
        Side::Old => (captured, live),
        Side::New => (live, captured),
    };

    e.outcome = match level {
        CheckLevel::CountsAndTotals => {
            match scan_counts(s, live_schema, live_name, &controls).await {
                Ok((rows, totals)) => {
                    let (c, l) = ((cap.rows, cap.control_totals.clone()), (rows, totals));
                    let ((orows, ot), (nrows, nt)) = match file_side {
                        Side::Old => (c, l),
                        Side::New => (l, c),
                    };
                    Outcome::Counts(CountsComparison {
                        old_rows: orows,
                        new_rows: nrows,
                        old_totals: ot,
                        new_totals: nt,
                    })
                }
                Err(err) => Outcome::Fault(format!("count scan failed: {err}")),
            }
        }
        _ => {
            let plan = match TablePlan::new(plans, key, cap.bucket_digits) {
                Ok(p) => p,
                Err(err) => {
                    e.outcome = Outcome::Fault(format!("the plan cannot be built: {err}"));
                    return e;
                }
            };
            match scan_fingerprint(s, &plan, Side::Old, &src, mapping_hash, live_mode, None).await {
                Ok(live_fp) => {
                    let (o, n) = orient(captured_fp(cap, mapping_hash), live_fp);
                    match compare(SideOutcome::Ok(&o), SideOutcome::Ok(&n)) {
                        Ok(c) => Outcome::Full(c),
                        Err(err) => {
                            Outcome::Fault(format!("the fingerprints cannot be compared: {err}"))
                        }
                    }
                }
                Err(err) => Outcome::Fault(format!("fingerprint scan failed: {err}")),
            }
        }
    };
    e
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::fingerprint::{BucketSummary, Decimal};
    use crate::core::normalise::{ColumnOptions, PgType};
    use crate::fingerprint_file::{CapturedServer, FINGERPRINT_VERSION};

    fn table(name: &str, rows: u64) -> CapturedTable {
        CapturedTable {
            old_qualified: name.into(),
            level: CheckLevel::FullValues,
            keyed: true,
            bucket_digits: 3,
            columns: vec!["id".into(), "v".into()],
            key_columns: vec!["id".into()],
            column_types: vec![PgType::Int4, PgType::Text],
            options: vec![ColumnOptions::default(), ColumnOptions::default()],
            control_columns: vec![],
            rows,
            buckets: if rows == 0 {
                vec![]
            } else {
                vec![BucketSummary {
                    bucket: 1,
                    n: rows,
                    s: [1, 2, 3, 4],
                }]
            },
            control_totals: vec![],
            needs_review: false,
        }
    }

    fn file(tables: Vec<CapturedTable>) -> FingerprintFile {
        FingerprintFile {
            format_version: FINGERPRINT_VERSION,
            tool_version: crate::version().into(),
            normalise_rules_version: Some(crate::core::normalise::NORMALISE_RULES_VERSION),
            mapping_hash: crate::mapping::Mapping::new().mapping_hash,
            server: CapturedServer {
                version_num: 170000,
                version_string: "17".into(),
                encoding: "UTF8".into(),
                collation: "C".into(),
                sql_ascii: false,
            },
            inventory: Some(Box::new(crate::fingerprint_file::test_inventory::of(
                &tables
                    .iter()
                    .map(|t| t.old_qualified.as_str())
                    .collect::<Vec<_>>(),
            ))),
            tables,
            sha256: String::new(),
        }
    }

    fn run(old: Vec<CapturedTable>, new: Vec<CapturedTable>) -> RunOutcome {
        compare_two_files(&file(old), &file(new)).unwrap()
    }

    #[test]
    fn two_empty_captures_are_not_a_pass() {
        // Fix 1: `tables: []` on both sides used to be PASS with exit 0.
        let out = run(vec![], vec![]);
        assert_eq!(out.results.verdict, Verdict::Warning);
        assert_eq!(out.exit_code, 2);
    }

    #[test]
    fn identical_captures_pass() {
        let out = run(vec![table("s.a", 3)], vec![table("s.a", 3)]);
        assert_eq!(out.results.verdict, Verdict::Pass);
        assert_eq!(out.exit_code, 0);
        assert_eq!(
            out.results.coverage_line,
            "1 of 1 tables fully checked, 0 counts only, 0 structure only, 0 ignored"
        );
    }

    #[test]
    fn missing_extra_and_differing_tables() {
        let out = run(
            vec![table("s.a", 3), table("s.gone", 1)],
            vec![table("s.a", 2), table("s.new", 1)],
        );
        let t = &out.results.tables;
        assert_eq!(t["s.a"].verdict, Verdict::Fail);
        assert_eq!(t["s.gone"].verdict, Verdict::Fail);
        assert_eq!(t["s.new"].verdict, Verdict::Warning);
        assert_eq!(
            out.results.fully_checked, 1,
            "missing and new-only tables are not checked"
        );
    }

    #[test]
    fn a_dropped_column_between_captures_is_a_tool_fault_listing_the_gap() {
        let mut n = table("s.a", 3);
        n.columns = vec!["id".into(), "w".into()];
        let out = run(vec![table("s.a", 3)], vec![n]);
        let t = &out.results.tables["s.a"];
        assert_eq!(t.verdict, Verdict::ToolFault);
        assert!(t
            .reasons
            .iter()
            .any(|r| r.contains("s.a.v has no counterpart")));
    }

    #[test]
    fn incompatible_plans_are_tool_faults() {
        let mut n = table("s.a", 3);
        n.column_types[1] = PgType::Varchar;
        assert_eq!(
            run(vec![table("s.a", 3)], vec![n]).results.tables["s.a"].verdict,
            Verdict::ToolFault
        );
        let mut n = table("s.a", 3);
        n.level = CheckLevel::CountsAndTotals;
        n.buckets.clear();
        assert_eq!(
            run(vec![table("s.a", 3)], vec![n]).results.tables["s.a"].verdict,
            Verdict::ToolFault
        );
    }

    #[test]
    fn counts_captures_compare_counts_and_totals() {
        let counts = |rows: u64, total: &str| CapturedTable {
            level: CheckLevel::CountsAndTotals,
            buckets: vec![],
            control_columns: vec!["v".into()],
            control_totals: vec![Decimal::parse(total).unwrap()],
            ..table("s.a", rows)
        };
        let out = run(vec![counts(3, "1.0")], vec![counts(3, "1")]);
        assert_eq!(out.results.verdict, Verdict::Pass);
        assert_eq!(out.results.counts_only, 1);
        let out = run(vec![counts(3, "1.0")], vec![counts(3, "1.01")]);
        assert_eq!(out.results.verdict, Verdict::Fail);
    }

    #[test]
    fn sql_ascii_captures_are_never_a_clean_pass() {
        let mut f = file(vec![table("s.a", 3)]);
        f.server.sql_ascii = true;
        let out = compare_two_files(&f, &file(vec![table("s.a", 3)])).unwrap();
        assert_eq!(out.results.verdict, Verdict::NeedsReview);
    }

    #[test]
    fn invalid_files_and_mapping_mismatches_are_rejected() {
        let mut bad = table("s.a", 3);
        bad.rows = 4;
        assert!(matches!(
            compare_two_files(&file(vec![bad]), &file(vec![])),
            Err(RunError::Config(_))
        ));
        let mut other = file(vec![]);
        other.mapping_hash = "something else".into();
        assert!(compare_two_files(&file(vec![]), &other).is_err());
    }

    #[test]
    fn a_new_capture_lacking_an_index_is_not_a_pass() {
        // A: saved-file comparisons used to skip structure entirely, so this
        // pair passed with exit 0.
        let old = file(vec![table("s.a", 3)]);
        let mut new = file(vec![table("s.a", 3)]);
        new.inventory.as_mut().unwrap().schemas[0].tables[0]
            .indexes
            .clear();
        let out = compare_two_files(&old, &new).unwrap();
        assert_eq!(out.results.verdict, Verdict::Fail);
        assert_eq!(out.exit_code, 1);
        let f = out
            .results
            .structure_findings
            .iter()
            .find(|f| f.object == "s.a.ix_a")
            .expect("the missing index is a finding");
        assert_eq!(f.verdict, Verdict::Fail);
        // The data itself still compares equal.
        assert_eq!(out.results.tables["s.a"].verdict, Verdict::Pass);
    }

    #[test]
    fn red_structure_in_captures_fails_the_run_like_live() {
        let old = file(vec![table("s.a", 3)]);
        // A constraint that is NOT VALID on the new side.
        let mut new = file(vec![table("s.a", 3)]);
        new.inventory.as_mut().unwrap().schemas[0].tables[0].constraints[0].validated = false;
        assert_eq!(
            compare_two_files(&old, &new).unwrap().results.verdict,
            Verdict::Fail
        );
        // A large object lost.
        let mut old_lo = file(vec![table("s.a", 3)]);
        old_lo.inventory.as_mut().unwrap().large_object_count = 1;
        let out = compare_two_files(&old_lo, &file(vec![table("s.a", 3)])).unwrap();
        assert_eq!(out.results.verdict, Verdict::Fail);
        assert!(out
            .results
            .structure_findings
            .iter()
            .any(|f| f.object == "database.large_objects"));
    }

    /// Audit H3: a table with an `Unknown` type whose `needs_review` was edited
    /// to `false` (hash recomputed) must never PASS file-against-file.
    #[test]
    fn a_forged_needs_review_false_never_passes() {
        let forged = || {
            let mut t = table("s.a", 3);
            t.column_types[1] = PgType::Unknown("public.geometry".into());
            t.needs_review = false;
            t
        };
        // Through the parser, as a real attack would arrive: refused.
        let bytes = file(vec![forged()]).to_json().unwrap();
        let parsed = FingerprintFile::from_bytes(&bytes);
        assert!(matches!(
            parsed,
            Err(crate::fingerprint_file::FingerprintFileError::NeedsReviewCleared { .. })
        ));
        // Built in memory and handed straight to the compare: refused too.
        let r = compare_two_files(&file(vec![forged()]), &file(vec![forged()]));
        assert!(
            matches!(&r, Err(RunError::Config(m)) if m.contains("needs no review")),
            "{r:?}"
        );
        // Honestly flagged, the same pair is NEEDS_REVIEW, never PASS.
        let mut honest = forged();
        honest.needs_review = true;
        let out = compare_two_files(&file(vec![honest.clone()]), &file(vec![honest])).unwrap();
        assert_eq!(out.results.tables["s.a"].verdict, Verdict::NeedsReview);
        assert_ne!(out.results.verdict, Verdict::Pass);
    }

    #[test]
    fn the_captured_fingerprint_uses_the_recomputed_flag() {
        let mut t = table("s.a", 3);
        t.column_types[1] = PgType::Unknown("x".into());
        t.needs_review = false;
        assert!(captured_fp(&t, "m").needs_review);
        assert!(!captured_fp(&table("s.a", 3), "m").needs_review);
    }

    #[test]
    fn captures_from_other_or_missing_rules_versions_are_refused() {
        let mut other = file(vec![table("s.a", 3)]);
        other.normalise_rules_version = Some(crate::core::normalise::NORMALISE_RULES_VERSION + 1);
        for (o, n) in [
            (other.clone(), file(vec![table("s.a", 3)])),
            (file(vec![table("s.a", 3)]), other),
        ] {
            let r = compare_two_files(&o, &n);
            assert!(
                matches!(&r, Err(RunError::Config(m)) if m.contains("normalisation rules version")),
                "{r:?}"
            );
        }
        let mut missing = file(vec![table("s.a", 3)]);
        missing.normalise_rules_version = None;
        let r = compare_two_files(&missing, &file(vec![table("s.a", 3)]));
        assert!(
            matches!(&r, Err(RunError::Config(m)) if m.contains("does not record its normalisation rules version")),
            "{r:?}"
        );
    }

    #[test]
    fn a_different_tool_version_is_a_run_warning() {
        let mut old = file(vec![table("s.a", 3)]);
        old.tool_version = "0.0.1".into();
        let out = compare_two_files(&old, &file(vec![table("s.a", 3)])).unwrap();
        let w: Vec<&String> = out
            .results
            .warnings
            .iter()
            .filter(|w| w.contains("capture was made by Dmux"))
            .collect();
        assert_eq!(w.len(), 1, "{:?}", out.results.warnings);
        assert!(w[0].contains("old capture was made by Dmux 0.0.1"));
        // The rules match, so the data still compares (decision 23: a
        // different tool version alone is recorded, not refused).
        assert_eq!(out.results.tables["s.a"].verdict, Verdict::Pass);
        // The same tool version gives no such warning.
        let same = run(vec![table("s.a", 3)], vec![table("s.a", 3)]);
        assert!(!same
            .results
            .warnings
            .iter()
            .any(|w| w.contains("capture was made by Dmux")));
    }

    #[test]
    fn a_capture_without_an_inventory_floors_the_run_at_warning() {
        let mut new = file(vec![table("s.a", 3)]);
        new.inventory = None;
        let out = compare_two_files(&file(vec![table("s.a", 3)]), &new).unwrap();
        assert_eq!(out.results.verdict, Verdict::Warning);
        assert_eq!(out.exit_code, 2);
        assert!(out
            .results
            .warnings
            .iter()
            .any(|w| w == STRUCTURE_NOT_COMPARED));
    }
}
