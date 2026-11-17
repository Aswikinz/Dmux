//! Changing a mapping by hand: accepting a suggestion, picking a counterpart,
//! ignoring an item, and rebuilding with rename rules.
//!
//! The mapping the webview holds is a projection, and anything it sends back is
//! untrusted (audit L7).  Every change therefore goes through a command here,
//! which re-checks it against the two live databases' inventories and returns
//! the canonical, freshly hashed engine mapping.  Nothing here connects to a
//! database for writing: the inventories come from the same read-only sessions
//! as every other command.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use tauri::State;

use dmux_engine::inventory::{DatabaseInventory, Schema};
use dmux_engine::mapping::{
    self, ColumnMapping, IgnoredItem, Mapping, MappingStatus, Rule, RuleKind, SchemaMapping,
    TableMapping,
};

use super::gateway::{
    build_with_rules, collect, validate_mapping, MappingBuild, SuperuserOverride,
};
use crate::error::CommandError;
use crate::state::AppState;
use crate::types::ConnectInput;

const MAX_RULES: usize = 50;
const MAX_TEXT: usize = 128;
const MAX_REASON: usize = 500;

/// One change the user made to the mapping.  Every name is a separate field, so
/// a table or column name that contains a dot is never split.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum MappingEdit {
    /// Pair an old table with a table in the new schema it maps to.
    PairTable {
        schema: String,
        table: String,
        new_table: String,
    },
    /// Pair an old column with a column of the table's new counterpart.
    PairColumn {
        schema: String,
        table: String,
        column: String,
        new_column: String,
    },
    /// Leave an item out of the check, with a required reason.
    Ignore { path: String, reason: String },
}

fn refuse(message: impl Into<String>) -> CommandError {
    CommandError::InvalidMapping {
        message: message.into(),
    }
}

fn find_schema<'a>(inv: &'a DatabaseInventory, name: &str) -> Option<&'a Schema> {
    inv.schemas.iter().find(|s| s.name == name)
}

/// The schema an old schema maps to: its mapping, else a schema-rename rule,
/// else the same name (the same order the build and the run use).
fn target_schema_of(mapping: &Mapping, schema: &str) -> String {
    if let Some(sm) = mapping.schema_mappings.get(schema) {
        if !sm.target_schema.is_empty() {
            return sm.target_schema.clone();
        }
    }
    mapping
        .rules
        .iter()
        .find_map(|r| mapping::apply_rule(r, schema))
        .unwrap_or_else(|| schema.to_string())
}

/// Every `schema.table` and `schema.table.column` path of one inventory.
fn all_paths(inv: &DatabaseInventory) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for s in &inv.schemas {
        for t in &s.tables {
            out.insert(format!("{}.{}", s.name, t.name));
            for c in &t.columns {
                out.insert(format!("{}.{}.{}", s.name, t.name, c.name));
            }
        }
    }
    out
}

/// Refuse any rule set the engine could not apply as the user means it.
///
/// Rules apply to table names only (a schema rename renames a schema; prefix
/// and suffix rules rename tables).  Nothing a rule says can touch a column.
pub(crate) fn validate_rules(rules: &[Rule]) -> Result<(), CommandError> {
    if rules.len() > MAX_RULES {
        return Err(refuse(format!("at most {MAX_RULES} rules are allowed")));
    }
    let mut renamed_schemas = BTreeSet::new();
    for rule in rules {
        let bad_text = |t: &str| t.chars().count() > MAX_TEXT || t.chars().any(char::is_control);
        if bad_text(&rule.pattern) || bad_text(&rule.target) {
            return Err(refuse(
                "a rule's text is too long or contains control characters",
            ));
        }
        match rule.kind {
            RuleKind::SchemaRename => {
                if rule.pattern.trim().is_empty() || rule.target.trim().is_empty() {
                    return Err(refuse("a schema rename needs both an old and a new name"));
                }
                if !renamed_schemas.insert(rule.pattern.to_ascii_lowercase()) {
                    return Err(refuse(format!(
                        "schema {} is renamed by more than one rule",
                        rule.pattern
                    )));
                }
            }
            RuleKind::PrefixReplace | RuleKind::SuffixReplace => {
                if rule.pattern.is_empty() && rule.target.is_empty() {
                    return Err(refuse("a prefix or suffix rule cannot be empty"));
                }
            }
        }
    }
    Ok(())
}

/// Apply the user's edits to `mapping`, checking each against the live
/// inventories.  Returns the canonical mapping or a plain-language refusal.
pub(crate) fn apply_edits(
    old: &DatabaseInventory,
    new: &DatabaseInventory,
    mapping: &Mapping,
    edits: &[MappingEdit],
) -> Result<Mapping, CommandError> {
    let mut out = mapping.clone();
    for edit in edits {
        match edit {
            MappingEdit::PairTable {
                schema,
                table,
                new_table,
            } => pair_table(old, new, &mut out, schema, table, new_table)?,
            MappingEdit::PairColumn {
                schema,
                table,
                column,
                new_column,
            } => pair_column(old, new, &mut out, schema, table, column, new_column)?,
            MappingEdit::Ignore { path, reason } => ignore(old, new, &mut out, path, reason)?,
        }
    }
    validate_mapping(&out.with_fresh_hash())
}

fn pair_table(
    old: &DatabaseInventory,
    new: &DatabaseInventory,
    mapping: &mut Mapping,
    schema: &str,
    table: &str,
    new_table: &str,
) -> Result<(), CommandError> {
    let old_table = find_schema(old, schema)
        .and_then(|s| s.tables.iter().find(|t| t.name == table))
        .ok_or_else(|| refuse(format!("table {schema}.{table} is not in the old database")))?;
    let target_schema = target_schema_of(mapping, schema);
    let new_t = find_schema(new, &target_schema)
        .and_then(|s| s.tables.iter().find(|t| t.name == new_table))
        .ok_or_else(|| {
            refuse(format!(
                "table {target_schema}.{new_table} is not in the new database"
            ))
        })?;

    // Two old tables can never both map to one new table (M2): the mapping would
    // not say which of them the new table really is.
    let new_path = format!("{target_schema}.{new_table}");
    let mut pairs: Vec<(String, String)> = Vec::new();
    for (s, sm) in &mapping.schema_mappings {
        for (t, tm) in &sm.table_mappings {
            if s == schema && t == table {
                continue;
            }
            pairs.push((
                format!("{s}.{t}"),
                format!("{}.{}", sm.target_schema, tm.target_table),
            ));
        }
    }
    if let Some(owner) = pairs.iter().find(|(_, n)| *n == new_path) {
        return Err(refuse(format!(
            "{} is already paired with {new_path}. Two old tables cannot map to one new table: \
             unpair one of them first.",
            owner.0
        )));
    }

    let old_cols: Vec<String> = old_table.columns.iter().map(|c| c.name.clone()).collect();
    let new_cols: Vec<String> = new_t.columns.iter().map(|c| c.name.clone()).collect();
    let (columns, _, _) = mapping::match_columns(schema, table, &old_cols, &new_cols);

    let sm = mapping
        .schema_mappings
        .entry(schema.to_string())
        .or_insert_with(|| SchemaMapping {
            target_schema: target_schema.clone(),
            table_mappings: BTreeMap::new(),
        });
    sm.table_mappings.insert(
        table.to_string(),
        TableMapping {
            target_table: new_table.to_string(),
            status: MappingStatus::Confirmed,
            column_mappings: columns,
        },
    );
    let path = format!("{schema}.{table}");
    mapping.ignored.retain(|i| i.path != path);
    Ok(())
}

fn pair_column(
    old: &DatabaseInventory,
    new: &DatabaseInventory,
    mapping: &mut Mapping,
    schema: &str,
    table: &str,
    column: &str,
    new_column: &str,
) -> Result<(), CommandError> {
    let path = format!("{schema}.{table}.{column}");
    let target_table = mapping
        .schema_mappings
        .get(schema)
        .and_then(|sm| sm.table_mappings.get(table))
        .map(|tm| tm.target_table.clone())
        .ok_or_else(|| {
            refuse(format!(
                "pair table {schema}.{table} with a new table before pairing its columns"
            ))
        })?;
    let target_schema = target_schema_of(mapping, schema);
    let has_old = find_schema(old, schema)
        .and_then(|s| s.tables.iter().find(|t| t.name == table))
        .is_some_and(|t| t.columns.iter().any(|c| c.name == column));
    if !has_old {
        return Err(refuse(format!("column {path} is not in the old database")));
    }
    let has_new = find_schema(new, &target_schema)
        .and_then(|s| s.tables.iter().find(|t| t.name == target_table))
        .is_some_and(|t| t.columns.iter().any(|c| c.name == new_column));
    if !has_new {
        return Err(refuse(format!(
            "column {new_column} is not in {target_schema}.{target_table} in the new database"
        )));
    }
    let tm = mapping
        .schema_mappings
        .get_mut(schema)
        .and_then(|sm| sm.table_mappings.get_mut(table))
        .ok_or_else(|| refuse("the table is not paired"))?;
    if let Some((other, _)) = tm
        .column_mappings
        .iter()
        .find(|(old_name, cm)| old_name.as_str() != column && cm.target_column == new_column)
    {
        return Err(refuse(format!(
            "column {other} is already paired with {new_column}. Two old columns cannot map to \
             one new column: unpair one of them first."
        )));
    }
    tm.column_mappings.insert(
        column.to_string(),
        ColumnMapping {
            target_column: new_column.to_string(),
            status: MappingStatus::Confirmed,
        },
    );
    mapping.ignored.retain(|i| i.path != path);
    Ok(())
}

fn ignore(
    old: &DatabaseInventory,
    new: &DatabaseInventory,
    mapping: &mut Mapping,
    path: &str,
    reason: &str,
) -> Result<(), CommandError> {
    let reason = reason.trim();
    if reason.is_empty() {
        return Err(refuse("a reason is required to ignore an item"));
    }
    if reason.chars().count() > MAX_REASON {
        return Err(refuse("the reason for ignoring an item is too long"));
    }
    if !all_paths(old).contains(path) && !all_paths(new).contains(path) {
        return Err(refuse(format!("{path} is not in either database")));
    }
    mapping.ignored.retain(|i| i.path != path);
    mapping.ignored.push(IgnoredItem {
        path: path.to_string(),
        reason: reason.to_string(),
    });
    Ok(())
}

/// Apply edits to the mapping, after the live inventories confirm every name.
#[tauri::command]
pub async fn edit_mapping(
    old: ConnectInput,
    new: ConnectInput,
    mapping: Mapping,
    edits: Vec<MappingEdit>,
    allow_superuser: Option<SuperuserOverride>,
    state: State<'_, AppState>,
) -> Result<Mapping, CommandError> {
    let allow = allow_superuser
        .as_ref()
        .is_some_and(SuperuserOverride::allows);
    // The incoming mapping is untrusted: validate it before building on it.
    let mapping = validate_mapping(&mapping)?;
    let old_inv = collect(&old, allow, &state).await?;
    let new_inv = collect(&new, allow, &state).await?;
    apply_edits(&old_inv, &new_inv, &mapping, &edits)
}

/// How a set of rename rules would pair the tables.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub struct RulePreview {
    /// Old tables in scope.
    pub tables_total: usize,
    /// Old tables a rename rule pairs with a new table.
    pub tables_by_rules: usize,
    /// Old tables paired in all (rules, exact and case-insensitive names).
    pub tables_matched: usize,
}

pub(crate) fn preview_of(build: &MappingBuild, old: &DatabaseInventory) -> RulePreview {
    let mut by_rules = 0;
    let mut matched = 0;
    for sm in build.mapping.schema_mappings.values() {
        for tm in sm.table_mappings.values() {
            matched += 1;
            if tm.status == MappingStatus::Rule {
                by_rules += 1;
            }
        }
    }
    RulePreview {
        tables_total: old.schemas.iter().map(|s| s.tables.len()).sum(),
        tables_by_rules: by_rules,
        tables_matched: matched,
    }
}

fn build_for(
    old_inv: &DatabaseInventory,
    new_inv: &DatabaseInventory,
    rules: &[Rule],
) -> Result<MappingBuild, CommandError> {
    validate_rules(rules)?;
    let (mapping, needs_attention, suggestions) = build_with_rules(old_inv, new_inv, rules);
    Ok(MappingBuild {
        mapping,
        needs_attention,
        suggestions,
    })
}

/// Count how many tables the rules would pair, without changing anything.
#[tauri::command]
pub async fn preview_rules(
    old: ConnectInput,
    new: ConnectInput,
    rules: Vec<Rule>,
    allow_superuser: Option<SuperuserOverride>,
    state: State<'_, AppState>,
) -> Result<RulePreview, CommandError> {
    let allow = allow_superuser
        .as_ref()
        .is_some_and(SuperuserOverride::allows);
    validate_rules(&rules)?;
    let old_inv = collect(&old, allow, &state).await?;
    let new_inv = collect(&new, allow, &state).await?;
    let build = build_for(&old_inv, &new_inv, &rules)?;
    Ok(preview_of(&build, &old_inv))
}

/// Build the mapping again with these rules.  Choices made by hand are cleared:
/// the rules come first, and the screen says so before it asks.
#[tauri::command]
pub async fn rebuild_mapping(
    old: ConnectInput,
    new: ConnectInput,
    rules: Vec<Rule>,
    allow_superuser: Option<SuperuserOverride>,
    state: State<'_, AppState>,
) -> Result<MappingBuild, CommandError> {
    let allow = allow_superuser
        .as_ref()
        .is_some_and(SuperuserOverride::allows);
    validate_rules(&rules)?;
    let old_inv = collect(&old, allow, &state).await?;
    let new_inv = collect(&new, allow, &state).await?;
    build_for(&old_inv, &new_inv, &rules)
}

#[cfg(test)]
mod tests {
    use super::*;
    use dmux_engine::inventory::{Column, Table};

    fn table(name: &str, columns: &[&str]) -> Table {
        Table {
            name: name.to_string(),
            owner: "postgres".to_string(),
            has_oids: false,
            is_partitioned: false,
            columns: columns
                .iter()
                .map(|c| Column {
                    name: (*c).to_string(),
                    type_name: "text".to_string(),
                    length: None,
                    nullable: true,
                    default: None,
                })
                .collect(),
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

    fn inv(schemas: Vec<(&str, Vec<Table>)>) -> DatabaseInventory {
        DatabaseInventory {
            server_version: "17.0".to_string(),
            server_version_num: 170000,
            encoding: "UTF8".to_string(),
            collation: "C".to_string(),
            ctype: "C".to_string(),
            lc_monetary: "C".to_string(),
            schemas: schemas
                .into_iter()
                .map(|(name, tables)| Schema {
                    name: name.to_string(),
                    owner: "postgres".to_string(),
                    tables,
                    views: Vec::new(),
                    materialized_views: Vec::new(),
                    functions: Vec::new(),
                    triggers: Vec::new(),
                    sequences: Vec::new(),
                })
                .collect(),
            extensions: Vec::new(),
            large_object_count: 0,
            warnings: Vec::new(),
        }
    }

    fn rule(kind: RuleKind, pattern: &str, target: &str) -> Rule {
        Rule {
            kind,
            pattern: pattern.to_string(),
            target: target.to_string(),
        }
    }

    /// Old `accounts` has no exact partner: the new side calls it `acct`.
    fn renamed_pair() -> (DatabaseInventory, DatabaseInventory) {
        (
            inv(vec![(
                "public",
                vec![table("accounts", &["id", "amt"]), table("notes", &["id"])],
            )]),
            inv(vec![(
                "public",
                vec![
                    table("acct", &["id", "amount"]),
                    table("notes", &["id"]),
                    table("other", &["id"]),
                ],
            )]),
        )
    }

    fn built(old: &DatabaseInventory, new: &DatabaseInventory) -> Mapping {
        build_with_rules(old, new, &[]).0
    }

    #[test]
    fn accepting_a_suggestion_changes_the_engine_mapping() {
        let (old, new) = renamed_pair();
        let before = built(&old, &new);
        assert!(!before.schema_mappings["public"]
            .table_mappings
            .contains_key("accounts"));

        let after = apply_edits(
            &old,
            &new,
            &before,
            &[MappingEdit::PairTable {
                schema: "public".into(),
                table: "accounts".into(),
                new_table: "acct".into(),
            }],
        )
        .expect("a free target is accepted");
        let tm = &after.schema_mappings["public"].table_mappings["accounts"];
        assert_eq!(tm.target_table, "acct");
        assert_eq!(tm.status, MappingStatus::Confirmed);
        // The id column pairs by name; amt waits for a decision of its own.
        assert_eq!(tm.column_mappings["id"].target_column, "id");
        assert!(!tm.column_mappings.contains_key("amt"));
        assert_ne!(after.mapping_hash, before.mapping_hash);
        // The result is canonical: it passes the same check as an imported file.
        assert!(validate_mapping(&after).is_ok());

        let with_column = apply_edits(
            &old,
            &new,
            &after,
            &[MappingEdit::PairColumn {
                schema: "public".into(),
                table: "accounts".into(),
                column: "amt".into(),
                new_column: "amount".into(),
            }],
        )
        .expect("a column pair is accepted");
        let cm = &with_column.schema_mappings["public"].table_mappings["accounts"].column_mappings
            ["amt"];
        assert_eq!(cm.target_column, "amount");
        assert_eq!(cm.status, MappingStatus::Confirmed);
    }

    #[test]
    fn two_old_tables_for_one_new_table_are_refused_plainly() {
        let old = inv(vec![(
            "public",
            vec![table("accounts", &["id"]), table("acct_old", &["id"])],
        )]);
        let new = inv(vec![("public", vec![table("acct", &["id"])])]);
        let m = built(&old, &new);
        let pair = |t: &str| MappingEdit::PairTable {
            schema: "public".into(),
            table: t.into(),
            new_table: "acct".into(),
        };
        let first = apply_edits(&old, &new, &m, &[pair("accounts")]).unwrap();
        let err = apply_edits(&old, &new, &first, &[pair("acct_old")]).unwrap_err();
        let CommandError::InvalidMapping { message } = err else {
            panic!("expected InvalidMapping");
        };
        assert!(message.contains("public.accounts"), "{message}");
        assert!(message.contains("Two old tables cannot map to one new table"));
        // In one batch too.
        assert!(apply_edits(&old, &new, &m, &[pair("accounts"), pair("acct_old")]).is_err());
        // Pairing the same table twice is a re-pair, not a duplicate.
        assert!(apply_edits(&old, &new, &first, &[pair("accounts")]).is_ok());
    }

    #[test]
    fn two_old_columns_for_one_new_column_are_refused() {
        let old = inv(vec![("public", vec![table("t", &["a", "b"])])]);
        let new = inv(vec![("public", vec![table("t", &["x"])])]);
        let m = built(&old, &new);
        let pair = |c: &str| MappingEdit::PairColumn {
            schema: "public".into(),
            table: "t".into(),
            column: c.into(),
            new_column: "x".into(),
        };
        let first = apply_edits(&old, &new, &m, &[pair("a")]).unwrap();
        assert!(apply_edits(&old, &new, &first, &[pair("b")]).is_err());
    }

    #[test]
    fn edits_naming_things_that_do_not_exist_are_refused() {
        let (old, new) = renamed_pair();
        let m = built(&old, &new);
        let bad = [
            MappingEdit::PairTable {
                schema: "public".into(),
                table: "ghost".into(),
                new_table: "acct".into(),
            },
            MappingEdit::PairTable {
                schema: "public".into(),
                table: "accounts".into(),
                new_table: "ghost".into(),
            },
            MappingEdit::PairTable {
                schema: "nope".into(),
                table: "accounts".into(),
                new_table: "acct".into(),
            },
            // The table is not paired yet, so its columns cannot be.
            MappingEdit::PairColumn {
                schema: "public".into(),
                table: "accounts".into(),
                column: "amt".into(),
                new_column: "amount".into(),
            },
            MappingEdit::Ignore {
                path: "public.ghost".into(),
                reason: "x".into(),
            },
            MappingEdit::Ignore {
                path: "public.accounts".into(),
                reason: "   ".into(),
            },
        ];
        for edit in bad {
            assert!(
                apply_edits(&old, &new, &m, std::slice::from_ref(&edit)).is_err(),
                "{edit:?}"
            );
        }
    }

    #[test]
    fn ignoring_records_the_reason_and_pairing_again_clears_it() {
        let (old, new) = renamed_pair();
        let m = built(&old, &new);
        let ignored = apply_edits(
            &old,
            &new,
            &m,
            &[MappingEdit::Ignore {
                path: "public.accounts".into(),
                reason: " retired ".into(),
            }],
        )
        .unwrap();
        assert_eq!(
            ignored.ignored,
            vec![IgnoredItem {
                path: "public.accounts".into(),
                reason: "retired".into()
            }]
        );
        let paired = apply_edits(
            &old,
            &new,
            &ignored,
            &[MappingEdit::PairTable {
                schema: "public".into(),
                table: "accounts".into(),
                new_table: "acct".into(),
            }],
        )
        .unwrap();
        assert!(paired.ignored.is_empty());
    }

    #[test]
    fn a_forged_edit_kind_or_field_does_not_deserialise() {
        assert!(serde_json::from_str::<MappingEdit>(r#"{"kind":"drop_table","x":1}"#).is_err());
        assert!(serde_json::from_str::<MappingEdit>(
            r#"{"kind":"ignore","path":"a","reason":"b","extra":1}"#
        )
        .is_err());
    }

    #[test]
    fn rules_apply_to_table_names_only_and_pair_through_the_engine_rule() {
        let old = inv(vec![(
            "public",
            vec![table("tbl_accounts", &["tbl_id", "name"])],
        )]);
        let new = inv(vec![(
            "public",
            vec![table("accounts", &["tbl_id", "name"])],
        )]);
        let rules = [rule(RuleKind::PrefixReplace, "tbl_", "")];
        let (m, attention, _) = build_with_rules(&old, &new, &rules);
        let tm = &m.schema_mappings["public"].table_mappings["tbl_accounts"];
        assert_eq!(tm.target_table, "accounts");
        assert_eq!(tm.status, MappingStatus::Rule);
        // The prefix rule never renames a column: `tbl_id` still pairs with
        // `tbl_id`, not with `id`.
        assert_eq!(tm.column_mappings["tbl_id"].target_column, "tbl_id");
        assert_eq!(tm.column_mappings["tbl_id"].status, MappingStatus::Exact);
        assert!(attention.is_empty(), "{attention:?}");
        assert_eq!(m.rules, rules.to_vec());

        let preview = preview_of(
            &MappingBuild {
                mapping: m,
                needs_attention: attention,
                suggestions: Vec::new(),
            },
            &old,
        );
        assert_eq!(
            preview,
            RulePreview {
                tables_total: 1,
                tables_by_rules: 1,
                tables_matched: 1
            }
        );
    }

    #[test]
    fn a_rule_that_sends_two_old_tables_to_one_new_table_flags_the_second() {
        let old = inv(vec![(
            "public",
            vec![table("a_acct", &["id"]), table("b_acct", &["id"])],
        )]);
        let new = inv(vec![("public", vec![table("acct", &["id"])])]);
        // Both strip to `acct`.
        let rules = [
            rule(RuleKind::PrefixReplace, "a_", ""),
            rule(RuleKind::PrefixReplace, "b_", ""),
        ];
        let (m, attention, _) = build_with_rules(&old, &new, &rules);
        let mapped: Vec<_> = m.schema_mappings["public"].table_mappings.keys().collect();
        assert_eq!(mapped, vec!["a_acct"]);
        assert!(attention
            .iter()
            .any(|a| a.table.as_deref() == Some("b_acct")));
        // And a hand-built mapping with the duplicate is refused on its way in.
        let pairs: Vec<(String, String)> = vec![
            ("public.a_acct".into(), "public.acct".into()),
            ("public.b_acct".into(), "public.acct".into()),
        ];
        assert_eq!(mapping::duplicate_table_targets(&pairs).len(), 1);
    }

    #[test]
    fn schema_rename_rules_map_the_schema_and_validation_is_strict() {
        let old = inv(vec![("legacy", vec![table("t", &["id"])])]);
        let new = inv(vec![("public", vec![table("t", &["id"])])]);
        let rules = [rule(RuleKind::SchemaRename, "legacy", "public")];
        let (m, attention, _) = build_with_rules(&old, &new, &rules);
        assert!(attention.is_empty());
        assert_eq!(m.schema_mappings["legacy"].target_schema, "public");

        assert!(validate_rules(&rules).is_ok());
        assert!(validate_rules(&[rule(RuleKind::SchemaRename, "", "x")]).is_err());
        assert!(validate_rules(&[rule(RuleKind::PrefixReplace, "", "")]).is_err());
        assert!(validate_rules(&[rule(RuleKind::PrefixReplace, "a\nb", "x")]).is_err());
        assert!(validate_rules(&[
            rule(RuleKind::SchemaRename, "a", "b"),
            rule(RuleKind::SchemaRename, "A", "c"),
        ])
        .is_err());
        let many: Vec<Rule> = (0..=MAX_RULES)
            .map(|i| rule(RuleKind::PrefixReplace, &format!("p{i}"), "q"))
            .collect();
        assert!(validate_rules(&many).is_err());
    }

    #[test]
    fn suggestions_are_structured_and_never_offer_a_taken_name() {
        let old = inv(vec![(
            "public",
            vec![table("accounts", &["id"]), table("account", &["id"])],
        )]);
        // `account` pairs with `account` exactly, so `accounts` may not be
        // offered it as a counterpart.
        let new = inv(vec![(
            "public",
            vec![table("account", &["id"]), table("accountz", &["id"])],
        )]);
        let (_, _, suggestions) = build_with_rules(&old, &new, &[]);
        assert!(suggestions.iter().all(|s| s.candidate != "account"));

        let old = inv(vec![("public", vec![table("customer_master", &["id"])])]);
        let new = inv(vec![("public", vec![table("customer_masters", &["id"])])]);
        let (_, _, suggestions) = build_with_rules(&old, &new, &[]);
        let s = &suggestions[0];
        assert_eq!(s.schema, "public");
        assert_eq!(s.table.as_deref(), Some("customer_master"));
        assert_eq!(s.column, None);
        assert_eq!(s.candidate, "customer_masters");
        assert!(s.confidence > 80);
    }
}
