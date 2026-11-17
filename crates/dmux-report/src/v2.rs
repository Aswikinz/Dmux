//! Read results-schema version-2 extension fields from the raw results JSON.
//!
//! `RunResults` v2 (T022's fix round) adds structure findings, mapping
//! suggestions, per-table drill-down detail and a definition hash.  The report
//! renders these without needing the exact engine release that defined them: it
//! reads the raw JSON (whose integrity the verifier already checked) through
//! tolerance-for-absence accessors, so a version-1 file simply yields empty
//! lists.  This keeps the report crate decoupled from the engine's evolving
//! public types while still showing the newer fields when they are present.
//!
//! Key material (drill-down `changed_rows[].key`, `missing_keys`, `extra_keys`)
//! is only read when `include_keys` is ticked, matching PLAN.md 2.3 / security
//! rule 6: keys never enter a report by default.

use serde_json::Value;

/// One structure (inventory) difference, after the mapping is applied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StructureFinding {
    pub object: String,
    pub verdict: String,
    pub message: String,
}

/// A suggested match for an unmapped table or column.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Suggestion {
    pub path: String,
    pub candidate: String,
    pub confidence: String,
}

/// Drill-down detail for one table (values are aggregates/keys, never row data).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TableDetail {
    pub changed_columns: Vec<String>,
    pub plain_words: Vec<String>,
    /// "column: old -> new", one per differing control total.
    pub control_totals: Vec<String>,
    /// Key material (changed-row keys, missing/extra keys), only when
    /// `include_keys` is ticked.
    pub keyed_rows: Vec<String>,
}

pub fn structure_findings(raw: &Value) -> Vec<StructureFinding> {
    raw.get("structure_findings")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|it| {
                    Some(StructureFinding {
                        object: it.get("object")?.as_str()?.to_string(),
                        verdict: it.get("verdict")?.as_str()?.to_string(),
                        message: it.get("message")?.as_str()?.to_string(),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

pub fn suggestions(raw: &Value) -> Vec<Suggestion> {
    raw.get("suggestions")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|it| {
                    Some(Suggestion {
                        path: it.get("path")?.as_str()?.to_string(),
                        candidate: it.get("candidate")?.as_str()?.to_string(),
                        confidence: it
                            .get("confidence")
                            .map(|c| c.to_string())
                            .unwrap_or_default(),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

pub fn definition_hash(raw: &Value) -> String {
    raw.get("definition_hash")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

pub fn in_scope(raw: &Value) -> usize {
    raw.get("in_scope").and_then(Value::as_u64).unwrap_or(0) as usize
}

pub fn structure_only(raw: &Value) -> usize {
    raw.get("structure_only")
        .and_then(Value::as_u64)
        .unwrap_or(0) as usize
}

/// The drill-down detail for `table`, if the file carries it.
pub fn detail_for(raw: &Value, table: &str, include_keys: bool) -> Option<TableDetail> {
    let details = raw.get("details")?.as_object()?;
    let d = details.get(table)?;
    let obj = d.as_object()?;

    let mut detail = TableDetail {
        changed_columns: string_array(obj.get("changed_columns")),
        plain_words: string_array(obj.get("plain_words")),
        control_totals: obj
            .get("control_totals")
            .and_then(Value::as_array)
            .map(|arr| {
                arr.iter()
                    .filter_map(|it| {
                        Some(format!(
                            "{}: {} -> {}",
                            it.get("column")?.as_str()?,
                            it.get("old")?.as_str()?,
                            it.get("new")?.as_str()?
                        ))
                    })
                    .collect()
            })
            .unwrap_or_default(),
        keyed_rows: Vec::new(),
    };

    if include_keys {
        let mut keys = Vec::new();
        if let Some(rows) = obj.get("changed_rows").and_then(Value::as_array) {
            for r in rows {
                if let Some(key) = r.get("key").and_then(Value::as_array) {
                    keys.push(format_key(key));
                }
            }
        }
        if let Some(missing) = obj.get("missing_keys").and_then(Value::as_array) {
            for k in missing {
                if let Some(inner) = k.as_array() {
                    keys.push(format!("missing {}", format_key_inner(inner)));
                }
            }
        }
        if let Some(extra) = obj.get("extra_keys").and_then(Value::as_array) {
            for k in extra {
                if let Some(inner) = k.as_array() {
                    keys.push(format!("extra {}", format_key_inner(inner)));
                }
            }
        }
        detail.keyed_rows = keys;
    }

    Some(detail)
}

fn string_array(v: Option<&Value>) -> Vec<String> {
    v.and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

/// Render a key array of `serde_json` strings (or nulls) as `"a, b"`.
fn format_key(key: &[Value]) -> String {
    key.iter()
        .map(|v| v.as_str().unwrap_or("NULL"))
        .collect::<Vec<_>>()
        .join(", ")
}

fn format_key_inner(key: &[Value]) -> String {
    format_key(key)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn absent_fields_yield_empty_defaults() {
        let raw = serde_json::json!({ "results_version": 1 });
        assert!(structure_findings(&raw).is_empty());
        assert!(suggestions(&raw).is_empty());
        assert!(detail_for(&raw, "t", true).is_none());
        assert_eq!(in_scope(&raw), 0);
        assert_eq!(structure_only(&raw), 0);
        assert!(definition_hash(&raw).is_empty());
    }

    #[test]
    fn keys_are_gated_on_include_keys() {
        let raw = serde_json::json!({
            "details": {
                "a.t": {
                    "changed_columns": ["balance"],
                    "plain_words": ["one value changed"],
                    "changed_rows": [
                        { "key": ["123"], "columns": ["balance"] }
                    ],
                    "missing_keys": [["7"], ["8"]],
                    "extra_keys": [["9"]],
                    "control_totals": [
                        { "column": "balance", "old": "100.00", "new": "99.99" }
                    ]
                }
            }
        });

        let no_keys = detail_for(&raw, "a.t", false).unwrap();
        assert!(no_keys.keyed_rows.is_empty());
        assert_eq!(no_keys.changed_columns, vec!["balance".to_string()]);
        assert_eq!(
            no_keys.control_totals,
            vec!["balance: 100.00 -> 99.99".to_string()]
        );

        let with_keys = detail_for(&raw, "a.t", true).unwrap();
        assert_eq!(with_keys.keyed_rows.len(), 4);
        assert!(with_keys.keyed_rows[0].contains("123"));
        assert!(with_keys.keyed_rows.iter().any(|k| k.contains("missing")));
    }

    #[test]
    fn structure_findings_are_parsed() {
        let raw = serde_json::json!({
            "structure_findings": [
                { "object": "a.t.idx", "verdict": "FAIL", "message": "missing index" }
            ]
        });
        let f = structure_findings(&raw);
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].object, "a.t.idx");
        assert_eq!(f[0].verdict, "FAIL");
    }
}
