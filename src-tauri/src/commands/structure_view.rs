//! The side-by-side structure view's rows (PLAN 4.2).
//!
//! The engine's schema diff lists only the differences.  A side-by-side view
//! also needs the identical objects, so this module aligns the two collected
//! inventories by name into flat rows: one row per schema, table, column,
//! index, constraint, view, sequence, function or trigger, with the old
//! definition on the left and the new one on the right.  A missing side is
//! `None`, which the UI draws as an empty gap.
//!
//! **Display-only** (audit L7): nothing a verdict or a report depends on reads
//! this.  It carries names, types and definitions, never row values.  Order is
//! by name, never by catalogue position.

use std::collections::BTreeMap;

use serde::Serialize;

use dmux_engine::inventory::{Column, Constraint, DatabaseInventory, Index, Schema, Table};
use dmux_engine::schema_diff::{compare_inventories, SchemaDiff};

/// One aligned row.  `status` is `same`, `missing`, `added` or `changed`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct StructureRowDto {
    pub id: String,
    pub depth: u8,
    pub kind: &'static str,
    pub name: String,
    pub status: &'static str,
    pub old: Option<String>,
    pub new: Option<String>,
    pub expandable: bool,
}

/// The diff plus the aligned rows and what each server is.  The diff fields
/// are flattened, so older readers of the diff keep working.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct StructureCompare {
    #[serde(flatten)]
    pub diff: SchemaDiff,
    pub rows: Vec<StructureRowDto>,
    pub old_server_version: String,
    pub new_server_version: String,
}

fn status_of(old: &Option<String>, new: &Option<String>) -> &'static str {
    match (old, new) {
        (Some(a), Some(b)) if a == b => "same",
        (Some(_), Some(_)) => "changed",
        (Some(_), None) => "missing",
        (None, Some(_)) => "added",
        (None, None) => "same",
    }
}

fn column_text(c: &Column) -> String {
    let mut s = c.type_name.clone();
    if !c.nullable {
        s.push_str(" not null");
    }
    if let Some(d) = &c.default {
        s.push_str(" default ");
        s.push_str(d);
    }
    s
}

fn constraint_text(c: &Constraint) -> String {
    let mut s = c.definition.clone();
    if !c.validated {
        s.push_str(" (not validated)");
    }
    s
}

fn index_text(i: &Index) -> String {
    let mut s = i.definition.clone();
    if !i.valid {
        s.push_str(" (invalid)");
    }
    s
}

/// Align two name-keyed lists into rows (sorted by name).
#[allow(clippy::too_many_arguments)]
fn align<T>(
    rows: &mut Vec<StructureRowDto>,
    parent: &str,
    depth: u8,
    kind: &'static str,
    old: &[T],
    new: &[T],
    key: impl Fn(&T) -> String,
    text: impl Fn(&T) -> String,
) {
    let o: BTreeMap<String, String> = old.iter().map(|x| (key(x), text(x))).collect();
    let n: BTreeMap<String, String> = new.iter().map(|x| (key(x), text(x))).collect();
    let mut names: Vec<&String> = o.keys().chain(n.keys()).collect();
    names.sort();
    names.dedup();
    for name in names {
        let (ov, nv) = (o.get(name).cloned(), n.get(name).cloned());
        rows.push(StructureRowDto {
            id: format!("{parent}/{kind}:{name}"),
            depth,
            kind,
            name: name.clone(),
            status: status_of(&ov, &nv),
            old: ov,
            new: nv,
            expandable: false,
        });
    }
}

fn table_summary(t: &Table) -> String {
    format!(
        "{} columns, {} indexes, {} constraints",
        t.columns.len(),
        t.indexes.len(),
        t.constraints.len()
    )
}

fn push_tables(rows: &mut Vec<StructureRowDto>, parent: &str, old: &Schema, new: &Schema) {
    let o: BTreeMap<&str, &Table> = old.tables.iter().map(|t| (t.name.as_str(), t)).collect();
    let n: BTreeMap<&str, &Table> = new.tables.iter().map(|t| (t.name.as_str(), t)).collect();
    let mut names: Vec<&str> = o.keys().chain(n.keys()).copied().collect();
    names.sort_unstable();
    names.dedup();
    for name in names {
        let id = format!("{parent}/table:{name}");
        let (ot, nt) = (o.get(name), n.get(name));
        let (ov, nv) = (ot.map(|t| table_summary(t)), nt.map(|t| table_summary(t)));
        match (ot, nt) {
            (Some(a), Some(b)) => {
                rows.push(StructureRowDto {
                    id: id.clone(),
                    depth: 1,
                    kind: "table",
                    name: name.to_string(),
                    // The table row itself exists on both sides; its children
                    // carry the differences.
                    status: "same",
                    old: ov,
                    new: nv,
                    expandable: true,
                });
                align(
                    rows,
                    &id,
                    2,
                    "column",
                    &a.columns,
                    &b.columns,
                    |c| c.name.clone(),
                    column_text,
                );
                align(
                    rows,
                    &id,
                    2,
                    "constraint",
                    &a.constraints,
                    &b.constraints,
                    |c| c.name.clone(),
                    constraint_text,
                );
                align(
                    rows,
                    &id,
                    2,
                    "index",
                    &a.indexes,
                    &b.indexes,
                    |i| i.name.clone(),
                    index_text,
                );
            }
            _ => rows.push(StructureRowDto {
                id,
                depth: 1,
                kind: "table",
                name: name.to_string(),
                status: status_of(&ov, &nv),
                old: ov,
                new: nv,
                expandable: false,
            }),
        }
    }
}

/// Build the aligned rows for two inventories.
pub fn structure_rows(old: &DatabaseInventory, new: &DatabaseInventory) -> Vec<StructureRowDto> {
    let mut rows = Vec::new();

    let ext = |inv: &DatabaseInventory| -> BTreeMap<String, String> {
        inv.extensions
            .iter()
            .map(|e| {
                (
                    e.name.clone(),
                    format!("version {} in {}", e.version, e.schema),
                )
            })
            .collect()
    };
    let (oe, ne) = (ext(old), ext(new));
    let mut names: Vec<&String> = oe.keys().chain(ne.keys()).collect();
    names.sort();
    names.dedup();
    for name in names {
        let (ov, nv) = (oe.get(name).cloned(), ne.get(name).cloned());
        rows.push(StructureRowDto {
            id: format!("extension:{name}"),
            depth: 0,
            kind: "extension",
            name: name.clone(),
            status: status_of(&ov, &nv),
            old: ov,
            new: nv,
            expandable: false,
        });
    }

    let o: BTreeMap<&str, &Schema> = old.schemas.iter().map(|s| (s.name.as_str(), s)).collect();
    let n: BTreeMap<&str, &Schema> = new.schemas.iter().map(|s| (s.name.as_str(), s)).collect();
    let mut schemas: Vec<&str> = o.keys().chain(n.keys()).copied().collect();
    schemas.sort_unstable();
    schemas.dedup();
    let empty = |name: &str| Schema {
        name: name.to_string(),
        owner: String::new(),
        tables: Vec::new(),
        views: Vec::new(),
        materialized_views: Vec::new(),
        functions: Vec::new(),
        triggers: Vec::new(),
        sequences: Vec::new(),
    };
    for name in schemas {
        let id = format!("schema:{name}");
        let (os, ns) = (o.get(name), n.get(name));
        let (ov, nv) = (
            os.map(|_| "present".to_string()),
            ns.map(|_| "present".to_string()),
        );
        rows.push(StructureRowDto {
            id: id.clone(),
            depth: 0,
            kind: "schema",
            name: name.to_string(),
            status: status_of(&ov, &nv),
            old: ov,
            new: nv,
            expandable: true,
        });
        // A schema missing on one side lists its objects as gaps on that side.
        let (a, b) = (
            os.map(|s| (*s).clone()).unwrap_or_else(|| empty(name)),
            ns.map(|s| (*s).clone()).unwrap_or_else(|| empty(name)),
        );
        push_tables(&mut rows, &id, &a, &b);
        align(
            &mut rows,
            &id,
            1,
            "view",
            &a.views,
            &b.views,
            |v| v.name.clone(),
            |v| v.definition.clone(),
        );
        align(
            &mut rows,
            &id,
            1,
            "view",
            &a.materialized_views,
            &b.materialized_views,
            |v| format!("{} (materialized)", v.name),
            |v| v.definition.clone(),
        );
        align(
            &mut rows,
            &id,
            1,
            "sequence",
            &a.sequences,
            &b.sequences,
            |s| s.name.clone(),
            |s| {
                format!(
                    "{}, increment {}, cycle {}",
                    s.data_type,
                    s.increment.map_or("?".to_string(), |v| v.to_string()),
                    s.cycle.map_or("?".to_string(), |v| v.to_string())
                )
            },
        );
        align(
            &mut rows,
            &id,
            1,
            "function",
            &a.functions,
            &b.functions,
            |f| format!("{}({})", f.name, f.identity_arguments),
            |f| {
                format!(
                    "{}, body fingerprint {}",
                    f.language,
                    f.definition_hash.chars().take(12).collect::<String>()
                )
            },
        );
        align(
            &mut rows,
            &id,
            1,
            "trigger",
            &a.triggers,
            &b.triggers,
            |t| format!("{}.{}", t.table_name, t.name),
            |t| {
                format!(
                    "{} {} {}, {}",
                    t.timing, t.event_manipulation, t.action_orientation, t.enabled
                )
            },
        );
    }
    rows
}

/// Compare two inventories: the engine's diff plus the aligned rows.
pub fn compare(old: &DatabaseInventory, new: &DatabaseInventory) -> StructureCompare {
    StructureCompare {
        diff: compare_inventories(old, new),
        rows: structure_rows(old, new),
        old_server_version: old.server_version.clone(),
        new_server_version: new.server_version.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn col(name: &str, ty: &str) -> Column {
        Column {
            name: name.into(),
            type_name: ty.into(),
            length: None,
            nullable: true,
            default: None,
        }
    }

    fn table(name: &str, cols: Vec<Column>) -> Table {
        Table {
            name: name.into(),
            owner: String::new(),
            has_oids: false,
            is_partitioned: false,
            columns: cols,
            constraints: Vec::new(),
            indexes: Vec::new(),
            partitions: Vec::new(),
            inherited_from: Vec::new(),
            n_live_tup: 0,
            size_bytes: 0,
            last_analyze: None,
            stats_missing: false,
            warnings: Vec::new(),
        }
    }

    fn inv(tables: Vec<Table>) -> DatabaseInventory {
        DatabaseInventory {
            server_version: "17.0".into(),
            server_version_num: 170000,
            encoding: "UTF8".into(),
            collation: "C".into(),
            ctype: "C".into(),
            lc_monetary: "C".into(),
            schemas: vec![Schema {
                name: "public".into(),
                owner: String::new(),
                tables,
                views: Vec::new(),
                materialized_views: Vec::new(),
                functions: Vec::new(),
                triggers: Vec::new(),
                sequences: Vec::new(),
            }],
            extensions: Vec::new(),
            large_object_count: 0,
            warnings: Vec::new(),
        }
    }

    fn row<'a>(rows: &'a [StructureRowDto], id: &str) -> &'a StructureRowDto {
        rows.iter().find(|r| r.id == id).unwrap()
    }

    #[test]
    fn aligns_same_missing_added_and_changed_by_name() {
        let old = inv(vec![
            table(
                "accounts",
                vec![
                    col("id", "integer"),
                    col("balance", "numeric(18,2)"),
                    col("gone", "text"),
                ],
            ),
            table("legacy", vec![col("x", "text")]),
        ]);
        let new = inv(vec![
            // Column order differs: it must not matter.
            table(
                "accounts",
                vec![
                    col("balance", "numeric(19,2)"),
                    col("id", "integer"),
                    col("extra", "text"),
                ],
            ),
            table("fresh", vec![]),
        ]);
        let rows = structure_rows(&old, &new);
        let base = "schema:public/table:accounts";
        assert_eq!(row(&rows, &format!("{base}/column:id")).status, "same");
        let bal = row(&rows, &format!("{base}/column:balance"));
        assert_eq!(bal.status, "changed");
        assert_eq!(bal.old.as_deref(), Some("numeric(18,2)"));
        assert_eq!(bal.new.as_deref(), Some("numeric(19,2)"));
        let gone = row(&rows, &format!("{base}/column:gone"));
        assert_eq!((gone.status, gone.new.is_none()), ("missing", true));
        let extra = row(&rows, &format!("{base}/column:extra"));
        assert_eq!((extra.status, extra.old.is_none()), ("added", true));
        assert_eq!(row(&rows, "schema:public/table:legacy").status, "missing");
        assert_eq!(row(&rows, "schema:public/table:fresh").status, "added");
        assert!(!row(&rows, "schema:public/table:legacy").expandable);
        assert!(row(&rows, base).expandable);
    }

    #[test]
    fn rows_do_not_depend_on_input_order() {
        let a = inv(vec![
            table("b", vec![col("y", "text"), col("x", "text")]),
            table("a", vec![]),
        ]);
        let b = inv(vec![
            table("a", vec![]),
            table("b", vec![col("x", "text"), col("y", "text")]),
        ]);
        assert_eq!(
            structure_rows(&a, &b),
            structure_rows(&b, &a)
                .into_iter()
                .map(|mut r| {
                    std::mem::swap(&mut r.old, &mut r.new);
                    r
                })
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn compare_flattens_the_diff_and_names_both_servers() {
        let c = compare(&inv(vec![]), &inv(vec![]));
        let v = serde_json::to_value(&c).unwrap();
        assert!(
            v.get("entries").is_some(),
            "the diff stays at the top level"
        );
        assert!(v.get("rows").is_some());
        assert_eq!(v["old_server_version"], "17.0");
    }
}
