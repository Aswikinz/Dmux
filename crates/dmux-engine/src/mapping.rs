//! Mapping between old and new database objects.
//!
//! Mappings are built deterministically:
//!
//! 1. Apply explicit rules first (schema rename, prefix/suffix rules).
//! 2. Exact name matches are auto-accepted.
//! 3. Case-insensitive matches are auto-accepted only when unambiguous AND the
//!    target has not already been claimed by another old name; any ambiguity
//!    lands in the "needs attention" queue.
//! 4. Fuzzy matches are suggestions only and are **never** auto-accepted; they
//!    always go to "needs attention" with a confidence score.
//! 5. Tables or columns can be marked `ignore` with a required reason.
//!
//! The exported JSON is versioned and carries a SHA-256 hash of its canonical
//! form.  Mapping files are treated as untrusted input: they are validated,
//! size-limited and rejected if they contain unknown fields (at any nesting).

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use thiserror::Error;

/// Current mapping file format version.
pub const MAPPING_VERSION: u32 = 1;
/// Max size for a mapping *file* read from disk or pasted in, in bytes.
///
/// A mapping holds one entry per column, about 60 to 80 bytes each, so a core
/// banking schema with thousands of tables passes 10 MiB easily.  256 MiB
/// allows roughly four million columns while still refusing an absurd file
/// before it is parsed.  A mapping already in memory (one the app built and
/// the screen sends back) is checked with [`validate`], which has no byte
/// limit: there is nothing left to guard by then.
pub const MAX_MAPPING_BYTES: usize = 256 * 1024 * 1024;

/// Errors thrown when loading or validating a mapping.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum MappingError {
    #[error("mapping file too large: {0} bytes")]
    TooLarge(usize),
    #[error("JSON parse error: {0}")]
    Json(String),
    #[error("unknown fields in mapping: {0:?}")]
    UnknownFields(Vec<String>),
    #[error("mapping version {0} is not supported")]
    UnsupportedVersion(u32),
    #[error("ignore reason cannot be empty")]
    EmptyIgnoreReason,
    #[error("mapping hash mismatch: expected {expected}, got {actual}")]
    HashMismatch { expected: String, actual: String },
}

/// A saved mapping, serialisable to/from JSON.
///
/// `deny_unknown_fields` matters even though [`from_json`] also rejects unknown
/// top-level fields by hand: a `Mapping` can be deserialised directly as a
/// typed command argument (for example from the Tauri webview), where that
/// manual check never runs.  Rejecting at the serde boundary makes a typo'd
/// field an error everywhere.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct Mapping {
    pub version: u32,
    pub rules: Vec<Rule>,
    pub schema_mappings: BTreeMap<String, SchemaMapping>,
    pub ignored: Vec<IgnoredItem>,
    pub mapping_hash: String,
}

impl Mapping {
    /// Create a new empty mapping with the current version and a fresh hash.
    pub fn new() -> Self {
        Self {
            version: MAPPING_VERSION,
            rules: Vec::new(),
            schema_mappings: BTreeMap::new(),
            ignored: Vec::new(),
            mapping_hash: String::new(),
        }
        .with_fresh_hash()
    }

    /// Recompute and store the canonical hash.
    pub fn with_fresh_hash(mut self) -> Self {
        self.mapping_hash = compute_hash(&self);
        self
    }

    /// Serialise to a canonical JSON string (sorted keys, no whitespace).
    pub fn to_canonical_json(&self) -> Result<String, MappingError> {
        serde_json::to_string(self).map_err(|e| MappingError::Json(e.to_string()))
    }
}

impl Default for Mapping {
    fn default() -> Self {
        Self::new()
    }
}

/// A user-specified transformation rule, applied before automatic matching.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct Rule {
    pub kind: RuleKind,
    pub pattern: String,
    pub target: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Hash)]
#[serde(rename_all = "snake_case")]
pub enum RuleKind {
    SchemaRename,
    PrefixReplace,
    SuffixReplace,
}

/// Per-schema mapping: tables and their columns.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct SchemaMapping {
    pub target_schema: String,
    pub table_mappings: BTreeMap<String, TableMapping>,
}

/// Per-table mapping.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct TableMapping {
    pub target_table: String,
    pub status: MappingStatus,
    pub column_mappings: BTreeMap<String, ColumnMapping>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Hash, Default)]
#[serde(rename_all = "snake_case")]
pub enum MappingStatus {
    #[default]
    Unmapped,
    Exact,
    CaseInsensitive,
    Fuzzy,
    /// Paired by a rename rule (a schema, prefix or suffix rule).
    Rule,
    /// Paired by the user, by accepting a suggestion or picking a counterpart.
    Confirmed,
    Ignored,
}

/// Per-column mapping.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct ColumnMapping {
    pub target_column: String,
    pub status: MappingStatus,
}

/// A deliberately ignored table or column with a required reason.
///
/// This is the single ignore type: both table and column ignores are expressed
/// as a path plus a non-empty reason.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct IgnoredItem {
    pub path: String,
    pub reason: String,
}

/// Result of running the matching pipeline for one old-new schema pair.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MatchResult {
    /// Items successfully mapped, by old name.
    pub mapped: BTreeMap<String, TableMapping>,
    /// Items that need a human decision.
    pub needs_attention: Vec<AttentionItem>,
    /// Suggestions only; never auto-accepted.
    pub suggestions: Vec<Suggestion>,
}

/// A table or column that could not be matched automatically.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttentionItem {
    pub path: String,
    pub issue: AttentionIssue,
}

/// Why an item needs attention.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AttentionIssue {
    MissingInNew { candidates: Vec<String> },
    AmbiguousCaseInsensitive { candidates: Vec<String> },
    AddBackReview,
}

/// A fuzzy-match suggestion with a confidence score.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Suggestion {
    pub path: String,
    pub candidate: String,
    pub confidence: u8,
}

/// Pair the old table names in `old_names` with the new names in `new_names`.
///
/// `schema` is used only for building human-readable paths.
pub fn match_tables(schema: &str, old_names: &[String], new_names: &[String]) -> MatchResult {
    let mut mapped: BTreeMap<String, TableMapping> = BTreeMap::new();
    let mut needs_attention: Vec<AttentionItem> = Vec::new();
    let mut suggestions: Vec<Suggestion> = Vec::new();

    let new_set: BTreeSet<String> = new_names.iter().cloned().collect();
    let lower_new: BTreeMap<String, String> = new_names
        .iter()
        .map(|n| (n.to_lowercase(), n.clone()))
        .collect();
    // New names already claimed by an auto-accept; a second old name that maps
    // to the same target must be flagged, not silently double-mapped.
    let mut claimed: BTreeSet<String> = BTreeSet::new();

    for old in old_names {
        if new_set.contains(old) {
            mapped.insert(
                old.clone(),
                TableMapping {
                    target_table: old.clone(),
                    status: MappingStatus::Exact,
                    ..TableMapping::default()
                },
            );
            claimed.insert(old.clone());
            continue;
        }

        let lower = old.to_lowercase();
        let mut case_matches: Vec<String> = new_names
            .iter()
            .filter(|n| n.to_lowercase() == lower)
            .cloned()
            .collect();
        case_matches.sort();

        if case_matches.len() == 1 {
            let target = case_matches[0].clone();
            if claimed.contains(&target) {
                needs_attention.push(AttentionItem {
                    path: format!("{schema}.{old}"),
                    issue: AttentionIssue::AmbiguousCaseInsensitive {
                        candidates: case_matches,
                    },
                });
            } else {
                mapped.insert(
                    old.clone(),
                    TableMapping {
                        target_table: target.clone(),
                        status: MappingStatus::CaseInsensitive,
                        ..TableMapping::default()
                    },
                );
                claimed.insert(target);
            }
        } else if case_matches.len() > 1 {
            needs_attention.push(AttentionItem {
                path: format!("{schema}.{old}"),
                issue: AttentionIssue::AmbiguousCaseInsensitive {
                    candidates: case_matches,
                },
            });
        } else {
            // Try a fuzzy suggestion: longest common substring-ish via Levenshtein
            // on lower-cased names.
            let mut scored: Vec<(String, u8)> = new_names
                .iter()
                .map(|n| {
                    (
                        n.clone(),
                        fuzzy_score(&old.to_lowercase(), &n.to_lowercase()),
                    )
                })
                .filter(|(_, score)| *score > 0)
                .collect();
            scored.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));

            if let Some((candidate, confidence)) = scored.into_iter().next() {
                suggestions.push(Suggestion {
                    path: format!("{schema}.{old}"),
                    candidate,
                    confidence,
                });
            }

            needs_attention.push(AttentionItem {
                path: format!("{schema}.{old}"),
                issue: AttentionIssue::MissingInNew {
                    candidates: first_letter_candidates(&lower_new, &lower),
                },
            });
        }
    }

    // Tables present only in new.
    let old_set: BTreeSet<String> = old_names.iter().cloned().collect();
    for new in new_names {
        if !old_set.contains(new) && !mapped.values().any(|m| m.target_table == *new) {
            needs_attention.push(AttentionItem {
                path: format!("{schema}.{new}"),
                issue: AttentionIssue::AddBackReview,
            });
        }
    }

    MatchResult {
        mapped,
        needs_attention,
        suggestions,
    }
}

/// Match columns the same way as tables.
pub fn match_columns(
    schema: &str,
    table: &str,
    old_names: &[String],
    new_names: &[String],
) -> (
    BTreeMap<String, ColumnMapping>,
    Vec<AttentionItem>,
    Vec<Suggestion>,
) {
    let mut mapped: BTreeMap<String, ColumnMapping> = BTreeMap::new();
    let mut needs_attention: Vec<AttentionItem> = Vec::new();
    let mut suggestions: Vec<Suggestion> = Vec::new();

    let new_set: BTreeSet<String> = new_names.iter().cloned().collect();
    let mut claimed: BTreeSet<String> = BTreeSet::new();

    for old in old_names {
        if new_set.contains(old) {
            mapped.insert(
                old.clone(),
                ColumnMapping {
                    target_column: old.clone(),
                    status: MappingStatus::Exact,
                },
            );
            claimed.insert(old.clone());
            continue;
        }

        let lower = old.to_lowercase();
        let mut case_matches: Vec<String> = new_names
            .iter()
            .filter(|n| n.to_lowercase() == lower)
            .cloned()
            .collect();
        case_matches.sort();

        if case_matches.len() == 1 {
            let target = case_matches[0].clone();
            if claimed.contains(&target) {
                needs_attention.push(AttentionItem {
                    path: format!("{schema}.{table}.{old}"),
                    issue: AttentionIssue::AmbiguousCaseInsensitive {
                        candidates: case_matches,
                    },
                });
            } else {
                mapped.insert(
                    old.clone(),
                    ColumnMapping {
                        target_column: target.clone(),
                        status: MappingStatus::CaseInsensitive,
                    },
                );
                claimed.insert(target);
            }
        } else if case_matches.len() > 1 {
            needs_attention.push(AttentionItem {
                path: format!("{schema}.{table}.{old}"),
                issue: AttentionIssue::AmbiguousCaseInsensitive {
                    candidates: case_matches,
                },
            });
        } else {
            let mut scored: Vec<(String, u8)> = new_names
                .iter()
                .map(|n| {
                    (
                        n.clone(),
                        fuzzy_score(&old.to_lowercase(), &n.to_lowercase()),
                    )
                })
                .filter(|(_, score)| *score > 0)
                .collect();
            scored.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));

            if let Some((candidate, confidence)) = scored.into_iter().next() {
                suggestions.push(Suggestion {
                    path: format!("{schema}.{table}.{old}"),
                    candidate,
                    confidence,
                });
            }

            needs_attention.push(AttentionItem {
                path: format!("{schema}.{table}.{old}"),
                issue: AttentionIssue::MissingInNew {
                    candidates: Vec::new(),
                },
            });
        }
    }

    let old_set: BTreeSet<String> = old_names.iter().cloned().collect();
    for new in new_names {
        if !old_set.contains(new) && !mapped.values().any(|m| m.target_column == *new) {
            needs_attention.push(AttentionItem {
                path: format!("{schema}.{table}.{new}"),
                issue: AttentionIssue::AddBackReview,
            });
        }
    }

    (mapped, needs_attention, suggestions)
}

/// New names whose first character matches `lower`'s first character, without
/// ever slicing at a non-char boundary (a non-ASCII first letter must not panic).
fn first_letter_candidates(lower_new: &BTreeMap<String, String>, lower: &str) -> Vec<String> {
    let Some(first) = lower.chars().next() else {
        return Vec::new();
    };
    let mut out: BTreeSet<String> = lower_new
        .iter()
        .filter(|(k, _)| k.starts_with(first))
        .map(|(_, v)| v.clone())
        .collect();
    out.remove(lower);
    out.into_iter().collect()
}

/// Apply a rule to a table or column name.  Returns `Some(new_name)` if the
/// rule matched and produced a rename.
pub fn apply_rule(rule: &Rule, name: &str) -> Option<String> {
    match rule.kind {
        RuleKind::SchemaRename => {
            // Used by the caller to translate schema names wholesale.
            if rule.pattern.eq_ignore_ascii_case(name) {
                Some(rule.target.clone())
            } else {
                None
            }
        }
        RuleKind::PrefixReplace => name
            .strip_prefix(&rule.pattern)
            .map(|rest| format!("{}{}", rule.target, rest)),
        RuleKind::SuffixReplace => name
            .strip_suffix(&rule.pattern)
            .map(|rest| format!("{}{}", rest, rule.target)),
    }
}

/// Every new table path claimed by more than one old path, as `new -> olds`.
///
/// A many-to-one table mapping is refused (M2): two old tables can never both
/// PASS against the same new table, because the mapping does not say which old
/// table the new one really is.  The old paths are sorted for a stable report.
pub fn duplicate_table_targets(pairs: &[(String, String)]) -> BTreeMap<String, Vec<String>> {
    let mut owner: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for (old, new) in pairs {
        owner.entry(new.as_str()).or_default().push(old.as_str());
    }
    owner
        .into_iter()
        .filter(|(_, olds)| olds.len() > 1)
        .map(|(new, olds)| {
            let mut olds: Vec<String> = olds.into_iter().map(str::to_string).collect();
            olds.sort();
            (new.to_string(), olds)
        })
        .collect()
}

/// Load a mapping from JSON bytes.
///
/// Validates size, version and rejects unknown fields at any nesting level.
/// Recomputes and checks the SHA-256 hash.
pub fn from_json(bytes: &[u8]) -> Result<Mapping, MappingError> {
    check_file_size(bytes.len())?;

    // One typed parse (`deny_unknown_fields` rejects an unknown field at any
    // level).  Only when it fails is the text read again, to name unknown
    // top-level fields: a large file is never held twice in memory.
    let mapping: Mapping = match serde_json::from_slice(bytes) {
        Ok(m) => m,
        Err(e) => return Err(unknown_top_level(bytes).unwrap_or(MappingError::Json(e.to_string()))),
    };

    check_contents(&mapping)?;

    let stored_hash = mapping.mapping_hash.clone();
    let computed = compute_hash(&mapping);
    if stored_hash != computed {
        return Err(MappingError::HashMismatch {
            expected: stored_hash,
            actual: computed,
        });
    }

    Ok(mapping)
}

/// Refuse a mapping file over [`MAX_MAPPING_BYTES`] before parsing it.
pub fn check_file_size(len: usize) -> Result<(), MappingError> {
    if len > MAX_MAPPING_BYTES {
        return Err(MappingError::TooLarge(len));
    }
    Ok(())
}

/// Unknown top-level field names, when that is why a file failed to parse.
fn unknown_top_level(bytes: &[u8]) -> Option<MappingError> {
    let raw: serde_json::Value = serde_json::from_slice(bytes).ok()?;
    let known: BTreeSet<&str> = [
        "version",
        "rules",
        "schema_mappings",
        "ignored",
        "mapping_hash",
    ]
    .into_iter()
    .collect();
    let unknown: Vec<String> = raw
        .as_object()?
        .keys()
        .filter(|k| !known.contains(k.as_str()))
        .cloned()
        .collect();
    (!unknown.is_empty()).then_some(MappingError::UnknownFields(unknown))
}

/// The checks every mapping passes, from a file or from memory: a supported
/// format version and a reason on every ignored item.
fn check_contents(mapping: &Mapping) -> Result<(), MappingError> {
    for item in &mapping.ignored {
        if item.reason.trim().is_empty() {
            return Err(MappingError::EmptyIgnoreReason);
        }
    }
    if mapping.version != MAPPING_VERSION {
        return Err(MappingError::UnsupportedVersion(mapping.version));
    }
    Ok(())
}

/// Validate a mapping that is already in memory (for example one the desktop
/// app's screen sends back) with the same checks as [`from_json`], and return
/// it with a freshly computed hash: a hash supplied with it is never trusted.
/// There is no byte limit here; the size of a file is checked where a file is
/// read.
pub fn validate(mapping: &Mapping) -> Result<Mapping, MappingError> {
    check_contents(mapping)?;
    Ok(mapping.clone().with_fresh_hash())
}

/// Serialise a mapping to canonical JSON bytes with a fresh hash.
pub fn to_json(mapping: &Mapping) -> Result<Vec<u8>, MappingError> {
    let mut canonical = mapping.clone();
    canonical.mapping_hash = compute_hash(mapping);
    serde_json::to_vec(&canonical).map_err(|e| MappingError::Json(e.to_string()))
}

/// Compute the SHA-256 hash of the mapping's canonical JSON form, excluding the
/// stored `mapping_hash` field so the hash is self-verifying.
fn compute_hash(mapping: &Mapping) -> String {
    // Hash only the structural content, not the stored hash.
    let canonical = CanonicalMapping::from(mapping);
    let json = serde_json::to_string(&canonical).unwrap_or_default();
    hex::encode(Sha256::digest(json.as_bytes()))
}

/// Stripped mapping used for canonical hashing.
#[derive(Serialize)]
struct CanonicalMapping<'a> {
    version: u32,
    rules: &'a [Rule],
    schema_mappings: &'a BTreeMap<String, SchemaMapping>,
    ignored: &'a [IgnoredItem],
}

impl<'a> From<&'a Mapping> for CanonicalMapping<'a> {
    fn from(m: &'a Mapping) -> Self {
        Self {
            version: m.version,
            rules: &m.rules,
            schema_mappings: &m.schema_mappings,
            ignored: &m.ignored,
        }
    }
}

/// Simple Levenshtein-based score scaled 0-100.
fn fuzzy_score(a: &str, b: &str) -> u8 {
    let dist = levenshtein(a, b);
    let max_len = a.len().max(b.len()).max(1);
    let similarity = max_len.saturating_sub(dist);
    ((similarity * 100) / max_len) as u8
}

fn levenshtein(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let n = a.len();
    let m = b.len();
    if n == 0 {
        return m;
    }
    if m == 0 {
        return n;
    }

    let mut prev: Vec<usize> = (0..=m).collect();
    let mut curr = vec![0; m + 1];

    for i in 1..=n {
        curr[0] = i;
        for j in 1..=m {
            let cost = if a[i - 1] == b[j - 1] { 0 } else { 1 };
            curr[j] = (curr[j - 1] + 1).min(prev[j] + 1).min(prev[j - 1] + cost);
        }
        std::mem::swap(&mut prev, &mut curr);
    }

    prev[m]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A mapping the size of a large core banking schema: `tables` tables of
    /// `cols` columns, all matched by name.
    fn large_mapping(tables: usize, cols: usize) -> Mapping {
        let mut table_mappings = BTreeMap::new();
        for t in 0..tables {
            let column_mappings = (0..cols)
                .map(|c| {
                    let name = format!("column_name_{c:03}");
                    (
                        name.clone(),
                        ColumnMapping {
                            target_column: name,
                            status: MappingStatus::Exact,
                        },
                    )
                })
                .collect();
            let name = format!("table_name_{t:05}");
            table_mappings.insert(
                name.clone(),
                TableMapping {
                    target_table: name,
                    status: MappingStatus::Exact,
                    column_mappings,
                },
            );
        }
        let mut m = Mapping::new();
        m.schema_mappings.insert(
            "core".into(),
            SchemaMapping {
                target_schema: "core".into(),
                table_mappings,
            },
        );
        m.with_fresh_hash()
    }

    #[test]
    fn a_large_database_mapping_is_never_refused_for_its_size() {
        // 4,000 tables x 60 columns: well over the old 10 MiB limit.
        let m = large_mapping(4_000, 60);
        let bytes = to_json(&m).unwrap();
        assert!(bytes.len() > 10 * 1024 * 1024, "{} bytes", bytes.len());
        // In memory: checked without any byte limit.
        assert_eq!(validate(&m).unwrap(), m);
        // As a file: under the file limit, and it round-trips exactly.
        assert_eq!(from_json(&bytes).unwrap(), m);
    }

    #[test]
    fn a_file_over_the_limit_is_refused_before_parsing() {
        assert!(check_file_size(MAX_MAPPING_BYTES).is_ok());
        assert_eq!(
            check_file_size(MAX_MAPPING_BYTES + 1),
            Err(MappingError::TooLarge(MAX_MAPPING_BYTES + 1))
        );
    }

    #[test]
    fn validate_in_memory_runs_the_file_checks_and_rederives_the_hash() {
        let mut m = Mapping::new();
        m.mapping_hash = "stale".into();
        let checked = validate(&m).unwrap();
        assert_eq!(checked.mapping_hash, compute_hash(&checked));
        assert_ne!(checked.mapping_hash, "stale");

        let mut empty_reason = Mapping::new();
        empty_reason.ignored.push(IgnoredItem {
            path: "core.t".into(),
            reason: "  ".into(),
        });
        assert_eq!(
            validate(&empty_reason),
            Err(MappingError::EmptyIgnoreReason)
        );

        let mut old = Mapping::new();
        old.version = MAPPING_VERSION + 1;
        assert_eq!(
            validate(&old),
            Err(MappingError::UnsupportedVersion(MAPPING_VERSION + 1))
        );
    }

    #[test]
    fn an_unknown_nested_field_is_still_refused() {
        let json = br#"{"version":1,"rules":[],"schema_mappings":{"s":{"target_schema":"s","table_mappings":{},"extra":1}},"ignored":[],"mapping_hash":""}"#;
        assert!(matches!(from_json(json), Err(MappingError::Json(_))));
    }

    #[test]
    fn exact_match_is_auto_accepted() {
        let result = match_tables(
            "public",
            &["accounts".to_string()],
            &["accounts".to_string()],
        );
        assert_eq!(result.mapped.len(), 1);
        assert_eq!(result.mapped["accounts"].status, MappingStatus::Exact);
        assert!(result.needs_attention.is_empty());
        assert!(result.suggestions.is_empty());
    }

    #[test]
    fn case_insensitive_match_is_auto_accepted_when_unique() {
        let result = match_tables(
            "public",
            &["ACCOUNTS".to_string()],
            &["accounts".to_string()],
        );
        assert_eq!(result.mapped.len(), 1);
        assert_eq!(
            result.mapped["ACCOUNTS"].status,
            MappingStatus::CaseInsensitive
        );
        assert!(result.needs_attention.is_empty());
    }

    #[test]
    fn ambiguous_case_insensitive_needs_attention() {
        let result = match_tables(
            "public",
            &["ACCOUNTS".to_string()],
            &["accounts".to_string(), "Accounts".to_string()],
        );
        assert!(result.mapped.is_empty());
        // The ambiguous old table plus the two unmapped new tables all need attention.
        assert_eq!(result.needs_attention.len(), 3);
        assert!(result
            .needs_attention
            .iter()
            .any(|a| matches!(a.issue, AttentionIssue::AmbiguousCaseInsensitive { .. })));
    }

    #[test]
    fn two_old_mapping_to_one_new_needs_attention() {
        let result = match_tables(
            "public",
            &["Accounts".to_string(), "accounts".to_string()],
            &["ACCOUNTS".to_string()],
        );
        // Exactly one old name may claim the single target; the other is flagged.
        assert_eq!(result.mapped.len(), 1);
        assert!(result
            .needs_attention
            .iter()
            .any(|a| matches!(a.issue, AttentionIssue::AmbiguousCaseInsensitive { .. })));
    }

    #[test]
    fn non_ascii_first_letter_does_not_panic() {
        let result = match_tables(
            "public",
            &["échéance".to_string()],
            &["échéance_2".to_string()],
        );
        // The non-ASCII name finds no exact/case match; it becomes a fuzzy
        // suggestion + missing-in-new without panicking.
        assert!(result.mapped.is_empty());
        assert!(result
            .needs_attention
            .iter()
            .any(|a| matches!(a.issue, AttentionIssue::MissingInNew { .. })));
    }

    #[test]
    fn fuzzy_match_is_suggestion_only() {
        let result = match_tables(
            "public",
            &["acounts".to_string()],
            &["accounts".to_string()],
        );
        assert!(result.mapped.is_empty());
        assert_eq!(result.suggestions.len(), 1);
        assert_eq!(result.suggestions[0].candidate, "accounts");
    }

    #[test]
    fn mapping_hash_is_stable_and_self_verifying() {
        let mut mapping = Mapping::new();
        mapping.rules.push(Rule {
            kind: RuleKind::PrefixReplace,
            pattern: "old_".to_string(),
            target: "new_".to_string(),
        });
        mapping = mapping.with_fresh_hash();
        let json = to_json(&mapping).unwrap();
        let loaded = from_json(&json).unwrap();
        assert_eq!(loaded, mapping);
    }

    #[test]
    fn unknown_field_is_rejected() {
        let json = r#"{"version":1,"rules":[],"schema_mappings":{},"ignored":[],"mapping_hash":"","extra":"bad"}"#;
        let err = from_json(json.as_bytes()).unwrap_err();
        match err {
            MappingError::UnknownFields(fields) => assert!(fields.contains(&"extra".to_string())),
            _ => panic!("expected unknown fields error, got {err}"),
        }
    }

    #[test]
    fn a_mapping_with_an_unknown_top_level_field_fails_to_deserialise() {
        // A typo'd field must be an error even when the Mapping is deserialised
        // directly (for example as a typed command argument), not only through
        // from_json's manual check.
        let json = r#"{"version":1,"rules":[],"schema_mappings":{},"ignored":[],"mapping_hash":"","extra":"bad"}"#;
        assert!(serde_json::from_str::<Mapping>(json).is_err());
        // And a well-formed mapping still round-trips directly.
        let ok = r#"{"version":1,"rules":[],"schema_mappings":{},"ignored":[],"mapping_hash":""}"#;
        assert!(serde_json::from_str::<Mapping>(ok).is_ok());
    }

    #[test]
    fn duplicate_table_targets_are_detected() {
        let pairs = vec![
            ("s.x".to_string(), "s.z".to_string()),
            ("s.y".to_string(), "s.z".to_string()),
            ("s.a".to_string(), "s.b".to_string()),
        ];
        let dups = duplicate_table_targets(&pairs);
        assert_eq!(
            dups.get("s.z"),
            Some(&vec!["s.x".to_string(), "s.y".to_string()])
        );
        assert!(!dups.contains_key("s.b"));
        assert!(duplicate_table_targets(&[("s.a".into(), "s.b".into())]).is_empty());
    }

    #[test]
    fn nested_unknown_field_is_rejected() {
        let json = r#"{"version":1,"rules":[{"kind":"prefix_replace","pattern":"a","target":"b","extra":"bad"}],"schema_mappings":{},"ignored":[],"mapping_hash":""}"#;
        let err = from_json(json.as_bytes()).unwrap_err();
        assert!(matches!(err, MappingError::Json(_)));
    }

    #[test]
    fn ignore_reason_cannot_be_empty() {
        let json = r#"{"version":1,"rules":[],"schema_mappings":{},"ignored":[{"path":"x.y","reason":"   "}],"mapping_hash":""}"#;
        let err = from_json(json.as_bytes()).unwrap_err();
        assert!(matches!(err, MappingError::EmptyIgnoreReason));
    }

    #[test]
    fn rule_and_confirmed_statuses_round_trip_through_a_mapping_file() {
        let mut m = Mapping::new();
        let mut table = TableMapping {
            target_table: "accounts".into(),
            status: MappingStatus::Rule,
            ..TableMapping::default()
        };
        table.column_mappings.insert(
            "amt".into(),
            ColumnMapping {
                target_column: "amount".into(),
                status: MappingStatus::Confirmed,
            },
        );
        let mut schema = SchemaMapping {
            target_schema: "public".into(),
            ..SchemaMapping::default()
        };
        schema.table_mappings.insert("acct".into(), table);
        m.schema_mappings.insert("public".into(), schema);
        let bytes = to_json(&m).unwrap();
        let text = String::from_utf8(bytes.clone()).unwrap();
        assert!(text.contains("\"rule\"") && text.contains("\"confirmed\""));
        let back = from_json(&bytes).unwrap();
        let tm = &back.schema_mappings["public"].table_mappings["acct"];
        assert_eq!(tm.status, MappingStatus::Rule);
        assert_eq!(tm.column_mappings["amt"].status, MappingStatus::Confirmed);
    }

    #[test]
    fn apply_prefix_and_suffix_rules() {
        let prefix = Rule {
            kind: RuleKind::PrefixReplace,
            pattern: "old_".to_string(),
            target: "new_".to_string(),
        };
        assert_eq!(apply_rule(&prefix, "old_accounts").unwrap(), "new_accounts");

        let suffix = Rule {
            kind: RuleKind::SuffixReplace,
            pattern: "_old".to_string(),
            target: "_new".to_string(),
        };
        assert_eq!(apply_rule(&suffix, "accounts_old").unwrap(), "accounts_new");
    }
}

#[cfg(test)]
use proptest::prelude::*;

#[cfg(test)]
proptest! {
    #[test]
    fn fuzzy_score_is_at_most_100(a in "[a-z]{1,20}", b in "[a-z]{1,20}") {
        let score = fuzzy_score(&a, &b);
        assert!(score <= 100);
    }

    #[test]
    fn mapping_never_auto_accepts_fuzzy(
        old in "[a-z]{1,20}",
        new in "[a-z]{1,20}"
    ) {
        let result = match_tables("public", std::slice::from_ref(&old), std::slice::from_ref(&new));
        if let Some(m) = result.mapped.get(&old) {
            assert_ne!(m.status, MappingStatus::Fuzzy);
        }
    }
}
