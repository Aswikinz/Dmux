//! Schema diff: compare two [`DatabaseInventory`] values by name, never by
//! position.
//!
//! The result is a serialisable tree of red/green/amber entries.

use crate::inventory::{
    Column, Constraint, DatabaseInventory, Extension, Function, Index, Sequence, Table, Trigger,
    View,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Overall status of a schema diff.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Hash)]
#[serde(rename_all = "snake_case")]
pub enum DiffStatus {
    Equal,
    Different,
    Warning,
}

/// One named object difference.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct SchemaDiffEntry {
    pub path: Vec<String>,
    pub status: DiffStatus,
    pub old_value: Option<String>,
    pub new_value: Option<String>,
    pub message: String,
    pub children: Vec<SchemaDiffEntry>,
}

impl SchemaDiffEntry {
    fn leaf(path: Vec<String>, status: DiffStatus, message: impl Into<String>) -> Self {
        Self {
            path,
            status,
            old_value: None,
            new_value: None,
            message: message.into(),
            children: Vec::new(),
        }
    }

    fn with_children(
        path: Vec<String>,
        status: DiffStatus,
        message: impl Into<String>,
        children: Vec<SchemaDiffEntry>,
    ) -> Self {
        Self {
            path,
            status,
            old_value: None,
            new_value: None,
            message: message.into(),
            children,
        }
    }
}

/// The full comparison between two inventories.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct SchemaDiff {
    pub status: DiffStatus,
    pub old_encoding: String,
    pub new_encoding: String,
    pub old_collation: String,
    pub new_collation: String,
    pub entries: Vec<SchemaDiffEntry>,
}

/// Compare `old` and `new` inventories.
pub fn compare_inventories(old: &DatabaseInventory, new: &DatabaseInventory) -> SchemaDiff {
    let mut entries = Vec::new();

    if old.encoding != new.encoding {
        entries.push(SchemaDiffEntry::leaf(
            vec!["database".to_string(), "encoding".to_string()],
            DiffStatus::Warning,
            format!("encoding changed: {} -> {}", old.encoding, new.encoding),
        ));
    }
    if old.collation != new.collation {
        entries.push(SchemaDiffEntry::leaf(
            vec!["database".to_string(), "collation".to_string()],
            DiffStatus::Warning,
            format!("collation changed: {} -> {}", old.collation, new.collation),
        ));
    }
    if old.lc_monetary != new.lc_monetary {
        entries.push(SchemaDiffEntry::leaf(
            vec!["database".to_string(), "lc_monetary".to_string()],
            DiffStatus::Warning,
            format!(
                "lc_monetary changed: {} -> {}",
                old.lc_monetary, new.lc_monetary
            ),
        ));
    }

    diff_extensions(&mut entries, &old.extensions, &new.extensions);

    if old.large_object_count != new.large_object_count {
        entries.push(SchemaDiffEntry::leaf(
            vec!["database".to_string(), "large_objects".to_string()],
            DiffStatus::Different,
            format!(
                "large object count changed: {} -> {}",
                old.large_object_count, new.large_object_count
            ),
        ));
    }

    let old_schemas: BTreeMap<_, _> = old.schemas.iter().map(|s| (s.name.clone(), s)).collect();
    let new_schemas: BTreeMap<_, _> = new.schemas.iter().map(|s| (s.name.clone(), s)).collect();

    for name in old_schemas.keys() {
        if !new_schemas.contains_key(name) {
            entries.push(SchemaDiffEntry::leaf(
                vec!["schema".to_string(), name.clone()],
                DiffStatus::Different,
                format!("schema missing in new: {name}"),
            ));
        }
    }

    for (name, new_schema) in &new_schemas {
        match old_schemas.get(name) {
            None => {
                entries.push(SchemaDiffEntry::leaf(
                    vec!["schema".to_string(), name.clone()],
                    DiffStatus::Warning,
                    format!("schema added in new: {name}"),
                ));
            }
            Some(old_schema) => {
                if let Some(diff) = diff_schema(name, old_schema, new_schema) {
                    entries.push(diff);
                }
            }
        }
    }

    let mut status = DiffStatus::Equal;
    for e in &entries {
        status = merge_status(status, e.status);
    }

    SchemaDiff {
        status,
        old_encoding: old.encoding.clone(),
        new_encoding: new.encoding.clone(),
        old_collation: old.collation.clone(),
        new_collation: new.collation.clone(),
        entries,
    }
}

fn diff_extensions(entries: &mut Vec<SchemaDiffEntry>, old: &[Extension], new: &[Extension]) {
    let old_map: BTreeMap<_, _> = old.iter().map(|e| (e.name.clone(), e)).collect();
    let new_map: BTreeMap<_, _> = new.iter().map(|e| (e.name.clone(), e)).collect();

    for name in old_map.keys() {
        if !new_map.contains_key(name) {
            entries.push(SchemaDiffEntry::leaf(
                vec![
                    "database".to_string(),
                    "extension".to_string(),
                    name.clone(),
                ],
                DiffStatus::Warning,
                format!("extension missing in new: {name}"),
            ));
        }
    }
    for (name, new_ext) in &new_map {
        match old_map.get(name) {
            None => {
                entries.push(SchemaDiffEntry::leaf(
                    vec![
                        "database".to_string(),
                        "extension".to_string(),
                        name.clone(),
                    ],
                    DiffStatus::Warning,
                    format!("extension added in new: {name}"),
                ));
            }
            Some(old_ext) => {
                if old_ext.version != new_ext.version || old_ext.schema != new_ext.schema {
                    entries.push(SchemaDiffEntry::leaf(
                        vec![
                            "database".to_string(),
                            "extension".to_string(),
                            name.clone(),
                        ],
                        DiffStatus::Warning,
                        format!("extension changed: {name}"),
                    ));
                }
            }
        }
    }
}

fn diff_schema(
    name: &str,
    old: &crate::inventory::Schema,
    new: &crate::inventory::Schema,
) -> Option<SchemaDiffEntry> {
    let path = vec!["schema".to_string(), name.to_string()];
    let mut children = Vec::new();

    diff_object_lists(
        &mut children,
        path.clone(),
        "table",
        &old.tables,
        &new.tables,
        diff_table,
    );
    diff_object_lists(
        &mut children,
        path.clone(),
        "view",
        &old.views,
        &new.views,
        diff_view,
    );
    diff_object_lists(
        &mut children,
        path.clone(),
        "materialized_view",
        &old.materialized_views,
        &new.materialized_views,
        diff_view,
    );
    diff_keyed(
        &mut children,
        path.clone(),
        "function",
        &old.functions,
        &new.functions,
        |f: &Function| {
            let label = if f.identity_arguments.is_empty() {
                f.name.clone()
            } else {
                format!("{}({})", f.name, f.identity_arguments)
            };
            ((f.name.clone(), f.identity_arguments.clone()), label)
        },
        diff_function,
    );
    diff_keyed(
        &mut children,
        path.clone(),
        "trigger",
        &old.triggers,
        &new.triggers,
        |t: &Trigger| {
            (
                (t.table_name.clone(), t.name.clone()),
                format!("{}.{}", t.table_name, t.name),
            )
        },
        diff_trigger,
    );
    diff_object_lists(
        &mut children,
        path.clone(),
        "sequence",
        &old.sequences,
        &new.sequences,
        diff_sequence,
    );

    if children.is_empty() {
        None
    } else {
        Some(SchemaDiffEntry::with_children(
            path,
            rollup_status(&children),
            format!("schema {name} has differences"),
            children,
        ))
    }
}

fn diff_table(path: Vec<String>, old: &Table, new: &Table) -> Option<SchemaDiffEntry> {
    let mut children = Vec::new();

    if old.has_oids != new.has_oids {
        children.push(SchemaDiffEntry::leaf(
            push(&path, "with_oids"),
            DiffStatus::Warning,
            format!(
                "WITH OIDS flag changed: {} -> {}",
                old.has_oids, new.has_oids
            ),
        ));
    }

    diff_column_list(&mut children, path.clone(), &old.columns, &new.columns);
    diff_named_list(
        &mut children,
        path.clone(),
        "constraint",
        &old.constraints,
        &new.constraints,
        |a, b| a.name == b.name,
        diff_constraint,
    );
    diff_named_list(
        &mut children,
        path.clone(),
        "index",
        &old.indexes,
        &new.indexes,
        |a, b| a.name == b.name,
        diff_index,
    );

    if old.partitions != new.partitions {
        children.push(SchemaDiffEntry::leaf(
            push(&path, "partitions"),
            DiffStatus::Warning,
            "partition list changed".to_string(),
        ));
    }

    if children.is_empty() {
        None
    } else {
        Some(SchemaDiffEntry::with_children(
            path,
            rollup_status(&children),
            format!("table {} changed", old.name),
            children,
        ))
    }
}

fn diff_column_list(
    children: &mut Vec<SchemaDiffEntry>,
    path: Vec<String>,
    old: &[Column],
    new: &[Column],
) {
    let old_map: BTreeMap<_, _> = old.iter().map(|c| (c.name.clone(), c)).collect();
    let new_map: BTreeMap<_, _> = new.iter().map(|c| (c.name.clone(), c)).collect();

    for name in old_map.keys() {
        if !new_map.contains_key(name) {
            children.push(SchemaDiffEntry::leaf(
                push(&path, "column")
                    .into_iter()
                    .chain(Some(name.clone()))
                    .collect(),
                DiffStatus::Different,
                format!("column missing in new: {name}"),
            ));
        }
    }

    for (name, new_col) in &new_map {
        match old_map.get(name) {
            None => {
                children.push(SchemaDiffEntry::leaf(
                    push(&path, "column")
                        .into_iter()
                        .chain(Some(name.clone()))
                        .collect(),
                    DiffStatus::Warning,
                    format!("column added in new: {name}"),
                ));
            }
            Some(old_col) => {
                if let Some(diff) = diff_column(
                    push(&path, "column")
                        .into_iter()
                        .chain(Some(name.clone()))
                        .collect(),
                    old_col,
                    new_col,
                ) {
                    children.push(diff);
                }
            }
        }
    }
}

fn diff_column(path: Vec<String>, old: &Column, new: &Column) -> Option<SchemaDiffEntry> {
    let mut messages = Vec::new();
    let mut status = DiffStatus::Equal;

    if old.type_name != new.type_name {
        let type_status = type_change_status(&old.type_name, &new.type_name);
        status = merge_status(status, type_status);
        messages.push(format!("type: {} -> {}", old.type_name, new.type_name));
    }
    if old.nullable != new.nullable {
        status = merge_status(status, DiffStatus::Warning);
        messages.push(format!("nullable: {} -> {}", old.nullable, new.nullable));
    }
    if old.default != new.default {
        status = merge_status(status, DiffStatus::Warning);
        messages.push("default changed".to_string());
    }

    if messages.is_empty() {
        None
    } else {
        Some(SchemaDiffEntry::leaf(path, status, messages.join("; ")))
    }
}

/// Classify a column type change.  Any difference in `format_type` output is at
/// least a warning (never silently equal): a `numeric(12,2)` -> `numeric(12,0)`
/// scale change must not compare equal.
fn type_change_status(old_type: &str, new_type: &str) -> DiffStatus {
    if old_type == new_type {
        return DiffStatus::Equal;
    }

    let old_base = base_type(old_type);
    let new_base = base_type(new_type);

    // char(n) <-> varchar(n): amber (q06); a one-character char is not padded.
    if (old_base == "character" && new_base == "character varying")
        || (old_base == "character varying" && new_base == "character")
    {
        return DiffStatus::Warning;
    }

    // Same base type with only a typmod change (varchar(60)->varchar(120),
    // numeric(12,2)->numeric(12,0), char(2)->char(3)): amber.
    if old_base == new_base {
        return DiffStatus::Warning;
    }

    // json <-> jsonb loses order and duplicate keys: red.
    if (old_base == "json" && new_base == "jsonb") || (old_base == "jsonb" && new_base == "json") {
        return DiffStatus::Different;
    }

    // timestamp <-> timestamptz changes interpretation under the session TZ.
    if old_base.starts_with("timestamp") && new_base.starts_with("timestamp") {
        return DiffStatus::Warning;
    }

    // float4 <-> float8 widening is exact; still a type change to flag amber.
    if (old_base == "real" && new_base == "double precision")
        || (old_base == "double precision" && new_base == "real")
    {
        return DiffStatus::Warning;
    }

    // Any other base-type change is a real change: red.
    DiffStatus::Different
}

/// Strip a parenthesised typmod from the base type name, keeping qualifiers
/// such as "without time zone": `numeric(12,2)` -> `numeric`,
/// `timestamp(0) without time zone` -> `timestamp without time zone`.
fn base_type(t: &str) -> String {
    if let Some(open) = t.find('(') {
        if let Some(rel_close) = t[open..].find(')') {
            let close = open + rel_close;
            let base = &t[..open];
            let rest = &t[close + 1..];
            return format!("{base}{rest}");
        }
    }
    t.to_string()
}

fn diff_constraint(
    path: Vec<String>,
    old: &Constraint,
    new: &Constraint,
) -> Option<SchemaDiffEntry> {
    let mut messages = Vec::new();
    if old.kind != new.kind {
        messages.push("kind changed".to_string());
    }
    if old.validated && !new.validated {
        return Some(SchemaDiffEntry::leaf(
            path,
            DiffStatus::Different,
            format!("constraint {} is not validated in new", old.name),
        ));
    }
    if old.definition != new.definition {
        messages.push("definition changed".to_string());
    }
    if messages.is_empty() {
        None
    } else {
        Some(SchemaDiffEntry::leaf(
            path,
            DiffStatus::Warning,
            messages.join("; "),
        ))
    }
}

fn diff_index(path: Vec<String>, old: &Index, new: &Index) -> Option<SchemaDiffEntry> {
    if old.valid && !new.valid {
        return Some(SchemaDiffEntry::leaf(
            path,
            DiffStatus::Different,
            format!("index {} is invalid in new", old.name),
        ));
    }
    if old.definition != new.definition {
        return Some(SchemaDiffEntry::leaf(
            path,
            DiffStatus::Warning,
            format!("index {} definition changed", old.name),
        ));
    }
    None
}

fn diff_sequence(path: Vec<String>, old: &Sequence, new: &Sequence) -> Option<SchemaDiffEntry> {
    // A sequence whose state could not be read (missing SELECT privilege) is
    // reported, never silently skipped.
    if old.unreadable || new.unreadable {
        return Some(SchemaDiffEntry::leaf(
            path,
            DiffStatus::Warning,
            format!(
                "sequence {} could not be read on one side; review required",
                old.name
            ),
        ));
    }

    // Rows migrated but the new sequence never advanced: the next value handed
    // out would collide with migrated rows.  This is a real failure.
    if old.is_called && !new.is_called {
        return Some(SchemaDiffEntry::leaf(
            path,
            DiffStatus::Different,
            format!(
                "sequence {} was advanced on the old side but never called on the new side",
                old.name
            ),
        ));
    }

    // Both never called: nothing to compare.
    if !old.is_called && !new.is_called {
        return None;
    }

    match (old.last_value, new.last_value) {
        (Some(old_val), Some(new_val)) if new_val < old_val => Some(SchemaDiffEntry::leaf(
            path,
            DiffStatus::Different,
            format!(
                "sequence {} last_value regressed: {} -> {}",
                old.name, old_val, new_val
            ),
        )),
        _ => None,
    }
}

fn diff_view(_path: Vec<String>, old: &View, new: &View) -> Option<SchemaDiffEntry> {
    if old.definition != new.definition {
        return Some(SchemaDiffEntry::leaf(
            _path,
            DiffStatus::Warning,
            format!("view {} definition differs; review required", old.name),
        ));
    }
    None
}

fn diff_function(path: Vec<String>, old: &Function, new: &Function) -> Option<SchemaDiffEntry> {
    if old.definition_hash != new.definition_hash {
        return Some(SchemaDiffEntry::leaf(
            path,
            DiffStatus::Warning,
            format!("function {} definition differs; review required", old.name),
        ));
    }
    None
}

fn diff_trigger(path: Vec<String>, old: &Trigger, new: &Trigger) -> Option<SchemaDiffEntry> {
    if old.action_statement != new.action_statement || old.enabled != new.enabled {
        return Some(SchemaDiffEntry::leaf(
            path,
            DiffStatus::Warning,
            format!("trigger {} changed", old.name),
        ));
    }
    None
}

fn diff_object_lists<T, DiffFn>(
    children: &mut Vec<SchemaDiffEntry>,
    path: Vec<String>,
    kind: &str,
    old: &[T],
    new: &[T],
    diff_fn: DiffFn,
) where
    T: NamedObject + Eq,
    DiffFn: Fn(Vec<String>, &T, &T) -> Option<SchemaDiffEntry>,
{
    diff_named_list(
        children,
        path,
        kind,
        old,
        new,
        |a, b| a.name() == b.name(),
        diff_fn,
    );
}

fn diff_keyed<T, K, DiffFn>(
    children: &mut Vec<SchemaDiffEntry>,
    path: Vec<String>,
    kind: &str,
    old: &[T],
    new: &[T],
    identity: impl Fn(&T) -> (K, String),
    diff_fn: DiffFn,
) where
    K: Ord,
    DiffFn: Fn(Vec<String>, &T, &T) -> Option<SchemaDiffEntry>,
{
    let old_map: BTreeMap<K, (&T, String)> = old
        .iter()
        .map(|o| {
            let (k, label) = identity(o);
            (k, (o, label))
        })
        .collect();
    let new_map: BTreeMap<K, (&T, String)> = new
        .iter()
        .map(|n| {
            let (k, label) = identity(n);
            (k, (n, label))
        })
        .collect();

    for (k, (_o, label)) in &old_map {
        if !new_map.contains_key(k) {
            children.push(SchemaDiffEntry::leaf(
                push(&path, kind)
                    .into_iter()
                    .chain(Some(label.clone()))
                    .collect(),
                DiffStatus::Different,
                format!("{kind} missing in new: {label}"),
            ));
        }
    }
    for (k, (n, label)) in &new_map {
        match old_map.get(k) {
            None => {
                children.push(SchemaDiffEntry::leaf(
                    push(&path, kind)
                        .into_iter()
                        .chain(Some(label.clone()))
                        .collect(),
                    DiffStatus::Warning,
                    format!("{kind} added in new: {label}"),
                ));
            }
            Some((o, _)) => {
                if let Some(d) = diff_fn(
                    push(&path, kind)
                        .into_iter()
                        .chain(Some(label.clone()))
                        .collect(),
                    o,
                    n,
                ) {
                    children.push(d);
                }
            }
        }
    }
}

fn diff_named_list<T, EqFn, DiffFn>(
    children: &mut Vec<SchemaDiffEntry>,
    path: Vec<String>,
    kind: &str,
    old: &[T],
    new: &[T],
    eq: EqFn,
    diff_fn: DiffFn,
) where
    EqFn: Fn(&T, &T) -> bool,
    DiffFn: Fn(Vec<String>, &T, &T) -> Option<SchemaDiffEntry>,
    T: NamedObject,
{
    for o in old {
        if !new.iter().any(|n| eq(o, n)) {
            children.push(SchemaDiffEntry::leaf(
                push(&path, kind)
                    .into_iter()
                    .chain(Some(o.name().to_string()))
                    .collect(),
                DiffStatus::Different,
                format!("{kind} missing in new: {}", o.name()),
            ));
        }
    }

    for n in new {
        match old.iter().find(|o| eq(o, n)) {
            None => {
                children.push(SchemaDiffEntry::leaf(
                    push(&path, kind)
                        .into_iter()
                        .chain(Some(n.name().to_string()))
                        .collect(),
                    DiffStatus::Warning,
                    format!("{kind} added in new: {}", n.name()),
                ));
            }
            Some(o) => {
                if let Some(diff) = diff_fn(
                    push(&path, kind)
                        .into_iter()
                        .chain(Some(n.name().to_string()))
                        .collect(),
                    o,
                    n,
                ) {
                    children.push(diff);
                }
            }
        }
    }
}

fn push(path: &[String], segment: &str) -> Vec<String> {
    path.iter()
        .cloned()
        .chain(Some(segment.to_string()))
        .collect()
}

fn rollup_status(children: &[SchemaDiffEntry]) -> DiffStatus {
    children
        .iter()
        .map(|c| c.status)
        .fold(DiffStatus::Equal, merge_status)
}

fn merge_status(a: DiffStatus, b: DiffStatus) -> DiffStatus {
    match (a, b) {
        (DiffStatus::Different, _) | (_, DiffStatus::Different) => DiffStatus::Different,
        (DiffStatus::Warning, _) | (_, DiffStatus::Warning) => DiffStatus::Warning,
        _ => DiffStatus::Equal,
    }
}

/// Trait used to compare named objects generically.
pub trait NamedObject {
    fn name(&self) -> &str;
}

impl NamedObject for Table {
    fn name(&self) -> &str {
        &self.name
    }
}

impl NamedObject for View {
    fn name(&self) -> &str {
        &self.name
    }
}

impl NamedObject for Sequence {
    fn name(&self) -> &str {
        &self.name
    }
}

impl NamedObject for Constraint {
    fn name(&self) -> &str {
        &self.name
    }
}

impl NamedObject for Index {
    fn name(&self) -> &str {
        &self.name
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::inventory::{
        Column, Constraint, ConstraintKind, DatabaseInventory, Index, Schema, Sequence, Table,
    };

    fn empty_inventory() -> DatabaseInventory {
        DatabaseInventory {
            server_version: String::new(),
            server_version_num: 0,
            encoding: "UTF8".to_string(),
            collation: "C".to_string(),
            ctype: "C".to_string(),
            lc_monetary: "C".to_string(),
            schemas: Vec::new(),
            extensions: Vec::new(),
            large_object_count: 0,
            warnings: Vec::new(),
        }
    }

    fn table_with_columns(name: &str, columns: Vec<Column>) -> Table {
        Table {
            name: name.to_string(),
            owner: "postgres".to_string(),
            has_oids: false,
            is_partitioned: false,
            columns,
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

    fn column(name: &str, type_name: &str) -> Column {
        Column {
            name: name.to_string(),
            type_name: type_name.to_string(),
            length: None,
            nullable: false,
            default: None,
        }
    }

    #[test]
    fn equal_inventories_have_no_differences() {
        let inv = empty_inventory();
        let diff = compare_inventories(&inv, &inv);
        assert_eq!(diff.status, DiffStatus::Equal);
        assert!(diff.entries.is_empty());
    }

    #[test]
    fn missing_table_is_different() {
        let old = DatabaseInventory {
            schemas: vec![Schema {
                name: "public".to_string(),
                owner: "postgres".to_string(),
                tables: vec![table_with_columns("accounts", Vec::new())],
                views: Vec::new(),
                materialized_views: Vec::new(),
                functions: Vec::new(),
                triggers: Vec::new(),
                sequences: Vec::new(),
            }],
            ..empty_inventory()
        };
        let new = DatabaseInventory {
            schemas: vec![Schema {
                name: "public".to_string(),
                owner: "postgres".to_string(),
                tables: Vec::new(),
                views: Vec::new(),
                materialized_views: Vec::new(),
                functions: Vec::new(),
                triggers: Vec::new(),
                sequences: Vec::new(),
            }],
            ..empty_inventory()
        };
        let diff = compare_inventories(&old, &new);
        assert_eq!(diff.status, DiffStatus::Different);
    }

    #[test]
    fn column_type_change_char_to_varchar_is_warning() {
        // Real format_type output: character(1) -> character varying(1).
        let old = table_with_columns("kyc", vec![column("result", "character(1)")]);
        let new = table_with_columns("kyc", vec![column("result", "character varying(1)")]);
        let diff = diff_table(vec!["public".to_string(), "kyc".to_string()], &old, &new);
        assert_eq!(diff.unwrap().status, DiffStatus::Warning);
    }

    #[test]
    fn varchar_widening_is_warning() {
        let old = table_with_columns("branches", vec![column("city", "character varying(60)")]);
        let new = table_with_columns("branches", vec![column("city", "character varying(120)")]);
        let diff = diff_table(
            vec!["public".to_string(), "branches".to_string()],
            &old,
            &new,
        );
        assert_eq!(diff.unwrap().status, DiffStatus::Warning);
    }

    #[test]
    fn numeric_scale_change_is_never_equal() {
        // numeric(12,2) -> numeric(12,0) must not be Equal (and is a warning).
        let old = table_with_columns("t", vec![column("amt", "numeric(12,2)")]);
        let new = table_with_columns("t", vec![column("amt", "numeric(12,0)")]);
        let diff = diff_table(vec!["public".to_string(), "t".to_string()], &old, &new);
        assert_eq!(diff.unwrap().status, DiffStatus::Warning);
    }

    #[test]
    fn json_to_jsonb_is_different() {
        let old = table_with_columns("accounts", vec![column("meta", "json")]);
        let new = table_with_columns("accounts", vec![column("meta", "jsonb")]);
        let diff = diff_table(
            vec!["public".to_string(), "accounts".to_string()],
            &old,
            &new,
        );
        assert_eq!(diff.unwrap().status, DiffStatus::Different);
    }

    #[test]
    fn different_base_type_is_different() {
        let old = table_with_columns("t", vec![column("v", "integer")]);
        let new = table_with_columns("t", vec![column("v", "text")]);
        let diff = diff_table(vec!["public".to_string(), "t".to_string()], &old, &new);
        assert_eq!(diff.unwrap().status, DiffStatus::Different);
    }

    #[test]
    fn timestamp_to_timestamptz_is_warning() {
        let old = table_with_columns("t", vec![column("ts", "timestamp without time zone")]);
        let new = table_with_columns("t", vec![column("ts", "timestamp with time zone")]);
        let diff = diff_table(vec!["public".to_string(), "t".to_string()], &old, &new);
        assert_eq!(diff.unwrap().status, DiffStatus::Warning);
    }

    #[test]
    fn invalid_index_is_different() {
        let old = Table {
            indexes: vec![Index {
                name: "idx".to_string(),
                valid: true,
                definition: "CREATE INDEX idx ON accounts(id)".to_string(),
            }],
            ..table_with_columns("accounts", Vec::new())
        };
        let new = Table {
            indexes: vec![Index {
                name: "idx".to_string(),
                valid: false,
                definition: "CREATE INDEX idx ON accounts(id)".to_string(),
            }],
            ..table_with_columns("accounts", Vec::new())
        };
        let diff = diff_table(
            vec!["public".to_string(), "accounts".to_string()],
            &old,
            &new,
        );
        assert_eq!(diff.unwrap().status, DiffStatus::Different);
    }

    #[test]
    fn unvalidated_constraint_is_different() {
        let old = Table {
            constraints: vec![Constraint {
                name: "chk".to_string(),
                kind: ConstraintKind::Check,
                validated: true,
                definition: "CHECK (id > 0)".to_string(),
            }],
            ..table_with_columns("accounts", Vec::new())
        };
        let new = Table {
            constraints: vec![Constraint {
                name: "chk".to_string(),
                kind: ConstraintKind::Check,
                validated: false,
                definition: "CHECK (id > 0)".to_string(),
            }],
            ..table_with_columns("accounts", Vec::new())
        };
        let diff = diff_table(
            vec!["public".to_string(), "accounts".to_string()],
            &old,
            &new,
        );
        assert_eq!(diff.unwrap().status, DiffStatus::Different);
    }

    #[test]
    fn sequence_regression_is_different() {
        let old = Sequence {
            name: "seq".to_string(),
            data_type: "bigint".to_string(),
            last_value: Some(100),
            is_called: true,
            ..Sequence::default()
        };
        let new = Sequence {
            name: "seq".to_string(),
            data_type: "bigint".to_string(),
            last_value: Some(50),
            is_called: true,
            ..Sequence::default()
        };
        let diff = diff_sequence(vec!["public".to_string(), "seq".to_string()], &old, &new);
        assert_eq!(diff.unwrap().status, DiffStatus::Different);
    }

    #[test]
    fn sequence_advanced_old_but_never_called_new_is_different() {
        // The classic "rows migrated, sequence not advanced" bug: the new side
        // has the rows but its sequence was never used, so the next value would
        // collide.  Must be a FAIL, not a warning.
        let old = Sequence {
            name: "seq".to_string(),
            data_type: "bigint".to_string(),
            last_value: Some(5000),
            is_called: true,
            ..Sequence::default()
        };
        let new = Sequence {
            name: "seq".to_string(),
            data_type: "bigint".to_string(),
            last_value: None,
            is_called: false,
            ..Sequence::default()
        };
        let diff = diff_sequence(vec!["public".to_string(), "seq".to_string()], &old, &new);
        assert_eq!(diff.unwrap().status, DiffStatus::Different);
    }

    #[test]
    fn sequence_not_called_on_either_side_is_not_a_regression() {
        // A never-called sequence on both sides must not be reported even though
        // its last_value is absent.
        let old = Sequence {
            name: "seq".to_string(),
            data_type: "bigint".to_string(),
            is_called: false,
            ..Sequence::default()
        };
        let new = Sequence {
            name: "seq".to_string(),
            data_type: "bigint".to_string(),
            is_called: false,
            ..Sequence::default()
        };
        let diff = diff_sequence(vec!["public".to_string(), "seq".to_string()], &old, &new);
        assert!(diff.is_none());
    }

    #[test]
    fn unreadable_sequence_is_warning_not_silent() {
        let old = Sequence {
            name: "seq".to_string(),
            data_type: "bigint".to_string(),
            last_value: Some(10),
            is_called: true,
            unreadable: true,
            ..Sequence::default()
        };
        let new = Sequence {
            name: "seq".to_string(),
            data_type: "bigint".to_string(),
            last_value: Some(10),
            is_called: true,
            ..Sequence::default()
        };
        let diff = diff_sequence(vec!["public".to_string(), "seq".to_string()], &old, &new);
        assert_eq!(diff.unwrap().status, DiffStatus::Warning);
    }

    #[test]
    fn large_object_count_change_is_different() {
        let old = DatabaseInventory {
            large_object_count: 1,
            ..empty_inventory()
        };
        let new = DatabaseInventory {
            large_object_count: 0,
            ..empty_inventory()
        };
        let diff = compare_inventories(&old, &new);
        assert_eq!(diff.status, DiffStatus::Different);
    }

    #[test]
    fn lc_monetary_change_is_warning() {
        let old = DatabaseInventory {
            lc_monetary: "C".to_string(),
            ..empty_inventory()
        };
        let new = DatabaseInventory {
            lc_monetary: "en_US.utf8".to_string(),
            ..empty_inventory()
        };
        let diff = compare_inventories(&old, &new);
        assert_eq!(diff.status, DiffStatus::Warning);
    }

    #[test]
    fn diff_is_order_independent() {
        let mut old = empty_inventory();
        old.schemas.push(Schema {
            name: "public".to_string(),
            owner: "postgres".to_string(),
            tables: vec![
                table_with_columns("b", Vec::new()),
                table_with_columns("a", Vec::new()),
            ],
            views: Vec::new(),
            materialized_views: Vec::new(),
            functions: Vec::new(),
            triggers: Vec::new(),
            sequences: Vec::new(),
        });

        let mut new = empty_inventory();
        new.schemas.push(Schema {
            name: "public".to_string(),
            owner: "postgres".to_string(),
            tables: vec![
                table_with_columns("a", Vec::new()),
                table_with_columns("b", Vec::new()),
            ],
            views: Vec::new(),
            materialized_views: Vec::new(),
            functions: Vec::new(),
            triggers: Vec::new(),
            sequences: Vec::new(),
        });

        let diff = compare_inventories(&old, &new);
        assert_eq!(diff.status, DiffStatus::Equal);
    }
}
