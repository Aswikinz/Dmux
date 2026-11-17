//! Schema diff: compare two [`DatabaseInventory`] values by name, never by
//! position.
//!
//! The result is a serialisable tree of red/green/amber entries.

use crate::inventory::{
    Column, Constraint, DatabaseInventory, Index, Sequence, Table, Trigger, View,
};
use serde::{Deserialize, Serialize};

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

    let old_schemas: std::collections::BTreeMap<_, _> =
        old.schemas.iter().map(|s| (s.name.clone(), s)).collect();
    let new_schemas: std::collections::BTreeMap<_, _> =
        new.schemas.iter().map(|s| (s.name.clone(), s)).collect();

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
    diff_object_lists(
        &mut children,
        path.clone(),
        "function",
        &old.functions,
        &new.functions,
        diff_function,
    );
    diff_object_lists(
        &mut children,
        path.clone(),
        "trigger",
        &old.triggers,
        &new.triggers,
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
    let old_map: std::collections::BTreeMap<_, _> =
        old.iter().map(|c| (c.name.clone(), c)).collect();
    let new_map: std::collections::BTreeMap<_, _> =
        new.iter().map(|c| (c.name.clone(), c)).collect();

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
    if old.length != new.length {
        status = merge_status(status, DiffStatus::Warning);
        messages.push(format!("length: {:?} -> {:?}", old.length, new.length));
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

fn type_change_status(old_type: &str, new_type: &str) -> DiffStatus {
    let old_norm = normalise_type(old_type);
    let new_norm = normalise_type(new_type);

    if old_norm == new_norm {
        return DiffStatus::Equal;
    }

    // char(n) -> varchar(n): allowed, amber.
    if old_type.starts_with("char") && new_type.starts_with("varchar") {
        return DiffStatus::Warning;
    }

    // json -> jsonb: flagged because semantics change.
    if old_type == "json" && new_type == "jsonb" {
        return DiffStatus::Different;
    }

    // varchar(50) -> varchar(100): amber warning (widening).
    if old_type.starts_with("varchar") && new_type.starts_with("varchar") {
        return DiffStatus::Warning;
    }

    // numeric(p,s) -> numeric(p',s'): amber warning.
    if old_type.starts_with("numeric") && new_type.starts_with("numeric") {
        return DiffStatus::Warning;
    }

    DiffStatus::Different
}

fn normalise_type(t: &str) -> String {
    // Drop length/precision for comparison when the base type matches.
    if let Some(paren) = t.find('(') {
        t[..paren].to_string()
    } else {
        t.to_string()
    }
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
    if old.value_skipped || new.value_skipped {
        return None;
    }
    if let (Some(o), Some(n)) = (&old.last_value, &new.last_value) {
        let o_val: i64 = o.parse().ok()?;
        let n_val: i64 = n.parse().ok()?;
        if n_val < o_val {
            return Some(SchemaDiffEntry::leaf(
                path,
                DiffStatus::Different,
                format!(
                    "sequence {} last_value regressed: {} -> {}",
                    old.name, o_val, n_val
                ),
            ));
        }
    }
    None
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

fn diff_function(
    path: Vec<String>,
    old: &crate::inventory::Function,
    new: &crate::inventory::Function,
) -> Option<SchemaDiffEntry> {
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

impl NamedObject for crate::inventory::Function {
    fn name(&self) -> &str {
        &self.name
    }
}

impl NamedObject for Trigger {
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
        let old = table_with_columns(
            "accounts",
            vec![Column {
                name: "status".to_string(),
                type_name: "char(1)".to_string(),
                length: Some(1),
                nullable: false,
                default: None,
            }],
        );
        let new = table_with_columns(
            "accounts",
            vec![Column {
                name: "status".to_string(),
                type_name: "varchar(1)".to_string(),
                length: Some(1),
                nullable: false,
                default: None,
            }],
        );
        let diff = diff_table(
            vec!["public".to_string(), "accounts".to_string()],
            &old,
            &new,
        );
        assert!(diff.is_some());
        assert_eq!(diff.unwrap().status, DiffStatus::Warning);
    }

    #[test]
    fn json_to_jsonb_is_different() {
        let old = table_with_columns(
            "accounts",
            vec![Column {
                name: "meta".to_string(),
                type_name: "json".to_string(),
                length: None,
                nullable: true,
                default: None,
            }],
        );
        let new = table_with_columns(
            "accounts",
            vec![Column {
                name: "meta".to_string(),
                type_name: "jsonb".to_string(),
                length: None,
                nullable: true,
                default: None,
            }],
        );
        let diff = diff_table(
            vec!["public".to_string(), "accounts".to_string()],
            &old,
            &new,
        );
        assert_eq!(diff.unwrap().status, DiffStatus::Different);
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
            last_value: Some("100".to_string()),
            value_skipped: false,
            ..Sequence::default()
        };
        let new = Sequence {
            name: "seq".to_string(),
            data_type: "bigint".to_string(),
            last_value: Some("50".to_string()),
            value_skipped: false,
            ..Sequence::default()
        };
        let diff = diff_sequence(vec!["public".to_string(), "seq".to_string()], &old, &new);
        assert!(diff.is_some());
        assert_eq!(diff.unwrap().status, DiffStatus::Different);
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
