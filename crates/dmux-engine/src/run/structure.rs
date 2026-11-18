//! The structure part of a run: the inventories of both sides, compared after
//! the mapping is applied, flattened into [`StructureFinding`]s, plus
//! suggested matches for whatever stays unpaired.
//!
//! The old inventory is renamed through the mapping first, so a table or column
//! the user mapped is compared with its counterpart instead of being reported
//! as missing and added.  Findings on ignored items become ignored items with
//! their reason.  Nothing here touches row data.

use std::collections::{BTreeMap, BTreeSet};

use crate::inventory::{DatabaseInventory, Schema};
use crate::mapping::{match_columns, match_tables, Mapping, RuleKind};
use crate::plan::column_renames;
use crate::results::{IgnoredItem, MappingSuggestion, StructureFinding, Verdict};
use crate::schema_diff::{compare_inventories, DiffStatus, SchemaDiffEntry};

use super::map_table;
use super::verdict::Builder;

/// The structure step every comparison path shares: map the old inventory,
/// diff it against the new one, record the findings (each red one fails the
/// run), the ignored items with their reasons and the suggestions.  Returns
/// the findings so structure-level tables can take theirs.
pub fn apply(
    b: &mut Builder,
    old: &DatabaseInventory,
    new: &DatabaseInventory,
    mapping: &Mapping,
) -> Vec<StructureFinding> {
    let mapped_old = map_inventory(old, mapping);
    let (kept, ignored) = findings(&mapped_old, new, mapping);
    for i in &ignored {
        b.ignore_item(&i.path, &i.reason);
    }
    b.results.structure_findings = kept.clone();
    b.results.suggestions = suggestions(&mapped_old, new, mapping);
    kept
}

/// The schema an old schema maps to.
pub fn map_schema(mapping: &Mapping, schema: &str) -> String {
    if let Some(sm) = mapping.schema_mappings.get(schema) {
        if !sm.target_schema.is_empty() {
            return sm.target_schema.clone();
        }
    }
    for rule in &mapping.rules {
        if matches!(rule.kind, RuleKind::SchemaRename) && rule.pattern.eq_ignore_ascii_case(schema)
        {
            return rule.target.clone();
        }
    }
    schema.to_string()
}

/// The old inventory with every table, column and schema renamed through the
/// mapping (types and every other property unchanged).
pub fn map_inventory(old: &DatabaseInventory, mapping: &Mapping) -> DatabaseInventory {
    let mut schemas: BTreeMap<String, Schema> = BTreeMap::new();
    for s in &old.schemas {
        let target = map_schema(mapping, &s.name);
        let entry = schemas.entry(target.clone()).or_insert_with(|| Schema {
            name: target.clone(),
            owner: s.owner.clone(),
            tables: Vec::new(),
            views: Vec::new(),
            materialized_views: Vec::new(),
            functions: Vec::new(),
            triggers: Vec::new(),
            sequences: Vec::new(),
        });
        entry.views.extend(s.views.iter().cloned());
        entry
            .materialized_views
            .extend(s.materialized_views.iter().cloned());
        entry.functions.extend(s.functions.iter().cloned());
        entry.triggers.extend(s.triggers.iter().cloned());
        entry.sequences.extend(s.sequences.iter().cloned());
        for t in &s.tables {
            let (_, new_table) = map_table(mapping, &s.name, &t.name);
            let renames = column_renames(mapping, &s.name, &t.name);
            let mut t = t.clone();
            t.name = new_table;
            for c in &mut t.columns {
                if let Some(n) = renames.get(&c.name) {
                    c.name = n.clone();
                }
            }
            t.columns.sort_by(|a, b| a.name.cmp(&b.name));
            entry.tables.push(t);
        }
    }
    for s in schemas.values_mut() {
        s.tables.sort_by(|a, b| a.name.cmp(&b.name));
    }
    DatabaseInventory {
        schemas: schemas.into_values().collect(),
        ..old.clone()
    }
}

/// `schema.table[.column...]` for a diff path; database-level paths are joined.
pub fn object_of(path: &[String]) -> String {
    if path.first().map(String::as_str) != Some("schema") || path.len() < 2 {
        return path.join(".");
    }
    let mut parts = vec![path[1].clone()];
    let mut i = 2;
    while i < path.len() {
        // [kind, name] pairs; a trailing attribute ("partitions") stands alone.
        parts.push(path.get(i + 1).cloned().unwrap_or_else(|| path[i].clone()));
        i += 2;
    }
    parts.join(".")
}

/// The (new-side) `schema.table` a finding belongs to, if it is about a table.
pub fn table_of(f: &StructureFinding) -> Option<String> {
    let p = &f.path;
    (p.len() >= 4 && p[0] == "schema" && p[2] == "table").then(|| format!("{}.{}", p[1], p[3]))
}

fn flatten(entries: &[SchemaDiffEntry], out: &mut Vec<StructureFinding>) {
    for e in entries {
        if e.children.is_empty() {
            let verdict = match e.status {
                DiffStatus::Different => Verdict::Fail,
                DiffStatus::Warning => Verdict::Warning,
                DiffStatus::Equal => continue,
            };
            out.push(StructureFinding {
                object: object_of(&e.path),
                path: e.path.clone(),
                verdict,
                message: e.message.clone(),
            });
        } else {
            flatten(&e.children, out);
        }
    }
}

/// The mapping's ignores, translated to the new-side names findings use:
/// (table ignores, column/other ignores -> (original path, reason)).
fn ignores(mapping: &Mapping) -> (BTreeSet<String>, BTreeMap<String, (String, String)>) {
    let mut tables = BTreeSet::new();
    let mut items = BTreeMap::new();
    for it in &mapping.ignored {
        let parts: Vec<&str> = it.path.split('.').collect();
        let entry = (it.path.clone(), it.reason.clone());
        items.insert(it.path.clone(), entry.clone());
        match parts.as_slice() {
            [s, t] => {
                let (ns, nt) = map_table(mapping, s, t);
                tables.insert(format!("{s}.{t}"));
                tables.insert(format!("{ns}.{nt}"));
            }
            [s, t, c] => {
                let (ns, nt) = map_table(mapping, s, t);
                let nc = column_renames(mapping, s, t)
                    .get(*c)
                    .cloned()
                    .unwrap_or_else(|| (*c).to_string());
                items.insert(format!("{ns}.{nt}.{nc}"), entry);
            }
            _ => {}
        }
    }
    (tables, items)
}

/// Every structure difference after the mapping, with findings on ignored
/// items moved to ignored items (reason carried).
pub fn findings(
    mapped_old: &DatabaseInventory,
    new: &DatabaseInventory,
    mapping: &Mapping,
) -> (Vec<StructureFinding>, Vec<IgnoredItem>) {
    let diff = compare_inventories(mapped_old, new);
    let mut all = Vec::new();
    flatten(&diff.entries, &mut all);
    let (ignored_tables, ignored_items) = ignores(mapping);
    let mut kept = Vec::new();
    let mut ignored = Vec::new();
    for f in all {
        if ignored_tables
            .iter()
            .any(|t| f.object == *t || f.object.starts_with(&format!("{t}.")))
        {
            continue; // the table-level ignore is already reported with its reason
        }
        if let Some((path, reason)) = ignored_items.get(&f.object) {
            let item = IgnoredItem {
                path: path.clone(),
                reason: reason.clone(),
            };
            if !ignored.contains(&item) {
                ignored.push(item);
            }
            continue;
        }
        kept.push(f);
    }
    (kept, ignored)
}

/// The structure findings on tables a run of chosen tables left out.  Those
/// tables are ignored (with an outside-run reason), so [`findings`] drops their
/// findings with every table-level ignore; this recovers them, so the run's
/// summary can count them and name the ones a full check would fail on
/// (decision 30).  Empty when nothing was left out.
pub fn left_out_findings(
    old: &DatabaseInventory,
    new: &DatabaseInventory,
    mapping: &Mapping,
) -> Vec<StructureFinding> {
    let left_out: BTreeSet<String> = mapping
        .ignored
        .iter()
        .filter(|i| super::is_outside_run(&i.reason))
        .filter_map(|i| {
            let (s, t) = i.path.split_once('.')?;
            (!t.contains('.')).then(|| {
                let (ns, nt) = map_table(mapping, s, t);
                format!("{ns}.{nt}")
            })
        })
        .collect();
    if left_out.is_empty() {
        return Vec::new();
    }
    // The same comparison, with only the user's own ignores applied.
    let mut own = mapping.clone();
    own.ignored.retain(|i| !super::is_outside_run(&i.reason));
    let (all, _) = findings(&map_inventory(old, &own), new, &own);
    all.into_iter()
        .filter(|f| {
            left_out
                .iter()
                .any(|t| f.object == *t || f.object.starts_with(&format!("{t}.")))
        })
        .collect()
}

/// Suggested matches for tables and columns that stay unpaired after the
/// mapping (never applied automatically).
pub fn suggestions(
    mapped_old: &DatabaseInventory,
    new: &DatabaseInventory,
    mapping: &Mapping,
) -> Vec<MappingSuggestion> {
    let (_, ignored_items) = ignores(mapping);
    let mut out = Vec::new();
    let new_by: BTreeMap<&str, &Schema> =
        new.schemas.iter().map(|s| (s.name.as_str(), s)).collect();
    for os in &mapped_old.schemas {
        let Some(ns) = new_by.get(os.name.as_str()) else {
            continue;
        };
        let old_names: BTreeSet<String> = os.tables.iter().map(|t| t.name.clone()).collect();
        let new_names: BTreeSet<String> = ns.tables.iter().map(|t| t.name.clone()).collect();
        let missing: Vec<String> = old_names.difference(&new_names).cloned().collect();
        let only_new: Vec<String> = new_names.difference(&old_names).cloned().collect();
        if !missing.is_empty() && !only_new.is_empty() {
            for s in match_tables(&os.name, &missing, &only_new).suggestions {
                out.push(MappingSuggestion {
                    path: s.path,
                    candidate: s.candidate,
                    confidence: s.confidence,
                });
            }
        }
        for ot in &os.tables {
            let Some(nt) = ns.tables.iter().find(|t| t.name == ot.name) else {
                continue;
            };
            let oc: BTreeSet<String> = ot.columns.iter().map(|c| c.name.clone()).collect();
            let nc: BTreeSet<String> = nt.columns.iter().map(|c| c.name.clone()).collect();
            let dropped: Vec<String> = oc
                .difference(&nc)
                .filter(|c| !ignored_items.contains_key(&format!("{}.{}.{c}", os.name, ot.name)))
                .cloned()
                .collect();
            let added: Vec<String> = nc.difference(&oc).cloned().collect();
            if dropped.is_empty() || added.is_empty() {
                continue;
            }
            for s in match_columns(&os.name, &ot.name, &dropped, &added).2 {
                out.push(MappingSuggestion {
                    path: s.path,
                    candidate: s.candidate,
                    confidence: s.confidence,
                });
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::inventory::{Column, Table};
    use crate::mapping::{
        ColumnMapping as MapColumn, IgnoredItem as MapIgnored, MappingStatus, SchemaMapping,
        TableMapping,
    };

    fn col(n: &str) -> Column {
        Column {
            name: n.into(),
            type_name: "text".into(),
            length: None,
            nullable: true,
            default: None,
        }
    }

    fn table(n: &str, cols: &[&str]) -> Table {
        Table {
            name: n.into(),
            owner: "o".into(),
            has_oids: false,
            is_partitioned: false,
            columns: cols.iter().map(|c| col(c)).collect(),
            constraints: vec![],
            indexes: vec![],
            partitions: vec![],
            inherited_from: vec![],
            n_live_tup: 0,
            size_bytes: 0,
            last_analyze: None,
            stats_missing: false,
            warnings: vec![],
        }
    }

    fn inv(tables: Vec<Table>) -> DatabaseInventory {
        DatabaseInventory {
            server_version: String::new(),
            server_version_num: 0,
            encoding: "UTF8".into(),
            collation: "C".into(),
            ctype: "C".into(),
            lc_monetary: "C".into(),
            schemas: vec![Schema {
                name: "s".into(),
                owner: "o".into(),
                tables,
                views: vec![],
                materialized_views: vec![],
                functions: vec![],
                triggers: vec![],
                sequences: vec![],
            }],
            extensions: vec![],
            large_object_count: 0,
            warnings: vec![],
        }
    }

    #[test]
    fn findings_on_left_out_tables_are_recovered_for_the_summary() {
        // s.loans lost a column in the new database; s.accounts is unchanged.
        let old = inv(vec![
            table("accounts", &["id"]),
            table("loans", &["id", "rate"]),
        ]);
        let new = inv(vec![table("accounts", &["id"]), table("loans", &["id"])]);
        let mut m = Mapping::new();
        m.ignored.push(MapIgnored {
            path: "s.loans".into(),
            reason: super::super::NOT_SELECTED_REASON.into(),
        });
        // The normal path drops it with the table-level ignore ...
        let (kept, _) = findings(&map_inventory(&old, &m), &new, &m);
        assert!(kept.iter().all(|f| !f.object.starts_with("s.loans")));
        // ... and this recovers it, verdict included.
        let left = left_out_findings(&old, &new, &m);
        assert_eq!(left.len(), 1, "{left:?}");
        assert_eq!(left[0].object, "s.loans.rate");
        assert_eq!(left[0].verdict, Verdict::Fail);
        // A person's own ignore is not "left out of the run": nothing recovered.
        let mut own = Mapping::new();
        own.ignored.push(MapIgnored {
            path: "s.loans".into(),
            reason: "archived before the migration".into(),
        });
        assert!(left_out_findings(&old, &new, &own).is_empty());
    }

    fn rename_mapping() -> Mapping {
        let mut m = Mapping::new();
        let mut sm = SchemaMapping {
            target_schema: "s".into(),
            table_mappings: BTreeMap::new(),
        };
        sm.table_mappings.insert(
            "limits".into(),
            TableMapping {
                target_table: "product_limits".into(),
                status: MappingStatus::Exact,
                column_mappings: BTreeMap::new(),
            },
        );
        let mut fees = TableMapping {
            target_table: "fees".into(),
            status: MappingStatus::Exact,
            column_mappings: BTreeMap::new(),
        };
        fees.column_mappings.insert(
            "fee_name".into(),
            MapColumn {
                target_column: "fee_label".into(),
                status: MappingStatus::Exact,
            },
        );
        sm.table_mappings.insert("fees".into(), fees);
        m.schema_mappings.insert("s".into(), sm);
        m
    }

    fn old() -> DatabaseInventory {
        inv(vec![
            table("fees", &["fee_id", "fee_name"]),
            table("limits", &["limit_id"]),
        ])
    }

    fn new() -> DatabaseInventory {
        inv(vec![
            table("fees", &["fee_id", "fee_label"]),
            table("product_limits", &["limit_id"]),
        ])
    }

    #[test]
    fn unmapped_renames_are_findings_with_suggestions() {
        let m = Mapping::new();
        let mapped = map_inventory(&old(), &m);
        let (f, _) = findings(&mapped, &new(), &m);
        let objects: Vec<&str> = f.iter().map(|x| x.object.as_str()).collect();
        assert!(objects.contains(&"s.limits"));
        assert!(objects.contains(&"s.fees.fee_name"));
        let red = f.iter().find(|x| x.object == "s.limits").unwrap();
        assert_eq!(red.verdict, Verdict::Fail);
        let s = suggestions(&mapped, &new(), &m);
        assert!(s
            .iter()
            .any(|x| x.path == "s.limits" && x.candidate == "product_limits"));
        assert!(s
            .iter()
            .any(|x| x.path == "s.fees.fee_name" && x.candidate == "fee_label"));
    }

    #[test]
    fn mapped_renames_are_not_findings() {
        let m = rename_mapping();
        let mapped = map_inventory(&old(), &m);
        let (f, _) = findings(&mapped, &new(), &m);
        assert!(f.is_empty(), "{f:?}");
        assert!(suggestions(&mapped, &new(), &m).is_empty());
    }

    #[test]
    fn findings_on_ignored_items_carry_the_reason() {
        let mut m = Mapping::new();
        m.ignored.push(MapIgnored {
            path: "s.fees.fee_name".into(),
            reason: "renamed by the vendor".into(),
        });
        m.ignored.push(MapIgnored {
            path: "s.limits".into(),
            reason: "retired".into(),
        });
        let mapped = map_inventory(&old(), &m);
        let (f, ignored) = findings(&mapped, &new(), &m);
        assert!(!f
            .iter()
            .any(|x| x.object == "s.fees.fee_name" || x.object == "s.limits"));
        assert_eq!(
            ignored,
            vec![IgnoredItem {
                path: "s.fees.fee_name".into(),
                reason: "renamed by the vendor".into()
            }]
        );
    }

    #[test]
    fn objects_and_tables_of_paths() {
        let p = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(
            object_of(&p(&["schema", "s", "table", "t", "column", "c"])),
            "s.t.c"
        );
        assert_eq!(
            object_of(&p(&["schema", "s", "table", "t", "partitions"])),
            "s.t.partitions"
        );
        assert_eq!(
            object_of(&p(&["database", "encoding"])),
            "database.encoding"
        );
        let f = StructureFinding {
            object: String::new(),
            path: p(&["schema", "s", "table", "t", "index", "ix"]),
            verdict: Verdict::Fail,
            message: String::new(),
        };
        assert_eq!(table_of(&f).as_deref(), Some("s.t"));
    }
}
