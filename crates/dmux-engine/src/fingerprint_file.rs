//! The fingerprint file format produced by `dmux capture` and consumed by
//! `dmux compare` / `dmux run` (PLAN.md 3.3).
//!
//! It is **versioned JSON**, optionally gzip-compressed, carrying the tool
//! version, server info, mapping hash, one per-table plan (columns, resolved
//! types, options, key and control-total columns) plus the bucket summaries,
//! needs_review and counts/control totals, and a SHA-256 over the canonical
//! content.
//!
//! Parsing is **untrusted input**.  Before any field is used, [`FingerprintFile::from_bytes`]
//! checks, in order: the size limit (compressed and inflated), the format
//! version, unknown fields **at every depth** (including inside the core's own
//! types, which do not deny unknown fields themselves), the SHA-256, and the
//! structure: no duplicate tables, no duplicate buckets, bucket indexes in
//! range, the bucket row counts adding up to the declared row count, and
//! columns, types, options, key and control-total lists consistent.  The
//! mapping hash and plan equality are checked again at compare time.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::io::Read;
use thiserror::Error;

use crate::core::fingerprint::{BucketSummary, Decimal, MAX_BUCKET_DIGITS};
use crate::core::normalise::{ColumnOptions, PgType};
use crate::inventory::DatabaseInventory;
use crate::results::CheckLevel;

/// Current fingerprint-file format version.  Version 3 added the full
/// structural `inventory` (PLAN.md 3.3), so saved-file comparisons check
/// structure exactly like live ones.  Version 2 added `column_types`,
/// `options` and `control_columns` to every table.
pub const FINGERPRINT_VERSION: u32 = 3;

/// Maximum accepted fingerprint file size (uncompressed), in bytes.
pub const MAX_FINGERPRINT_BYTES: usize = 512 * 1024 * 1024;

/// One 32-bit slice of an md5, the most a single row adds to a bucket sum.
const SLICE_MAX: u128 = 0xffff_ffff;

/// Errors when reading or writing a fingerprint file.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum FingerprintFileError {
    #[error("file too large: {0} bytes")]
    TooLarge(usize),
    #[error("unsupported format version {0}")]
    UnsupportedVersion(u32),
    #[error("unknown fields: {0:?}")]
    UnknownFields(Vec<String>),
    #[error("JSON error: {0}")]
    Json(String),
    #[error("SHA-256 mismatch: expected {expected}, got {actual}")]
    HashMismatch { expected: String, actual: String },
    #[error("I/O error: {0}")]
    Io(String),
    #[error("a fingerprint is missing its summary ({0})")]
    MissingSummary(String),
    #[error("the file carries {0} tables but its plan lists {1}")]
    TableCountMismatch(usize, usize),
    /// The file is well-formed JSON with a valid hash but its content is not a
    /// consistent fingerprint (duplicates, sums that do not add up, ...).
    #[error("invalid fingerprint file: {0}")]
    Invalid(String),
}

/// Server information carried in the fingerprint file header.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct CapturedServer {
    pub version_num: i32,
    pub version_string: String,
    pub encoding: String,
    pub collation: String,
    pub sql_ascii: bool,
}

/// One table's fingerprint as captured.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct CapturedTable {
    pub old_qualified: String,
    /// `full_values` (bucket summaries) or `counts_and_totals` (no buckets).
    /// `structure` is not valid in a fingerprint file.
    pub level: CheckLevel,
    pub keyed: bool,
    pub bucket_digits: u8,
    /// Column names in plan order (sorted by name).
    pub columns: Vec<String>,
    pub key_columns: Vec<String>,
    /// The resolved column types, in `columns` order.
    pub column_types: Vec<PgType>,
    /// The per-column options the hashes were computed with, in `columns` order.
    pub options: Vec<ColumnOptions>,
    /// The columns summed as control totals, in `control_totals` order.
    pub control_columns: Vec<String>,
    pub rows: u64,
    pub buckets: Vec<BucketSummary>,
    pub control_totals: Vec<Decimal>,
    pub needs_review: bool,
}

impl CapturedTable {
    /// The bucket count implied by `bucket_digits` (2^(4*digits)).
    pub fn expected_bucket_count(&self) -> u64 {
        1u64 << (4 * u32::from(self.bucket_digits.min(MAX_BUCKET_DIGITS)))
    }

    /// Structural checks for one table; see the module documentation.
    fn validate(&self) -> Result<(), FingerprintFileError> {
        let bad = |what: String| {
            Err(FingerprintFileError::Invalid(format!(
                "{}: {what}",
                self.old_qualified
            )))
        };
        if self.old_qualified.split_once('.').is_none() {
            return bad("table name is not schema-qualified".into());
        }
        if self.columns.is_empty() {
            return bad("no columns".into());
        }
        let unique: BTreeSet<&String> = self.columns.iter().collect();
        if unique.len() != self.columns.len() {
            return bad("duplicate column names".into());
        }
        if self.column_types.len() != self.columns.len() {
            return bad(format!(
                "{} columns but {} column types",
                self.columns.len(),
                self.column_types.len()
            ));
        }
        if self.options.len() != self.columns.len() {
            return bad(format!(
                "{} columns but {} option sets",
                self.columns.len(),
                self.options.len()
            ));
        }
        let keys: BTreeSet<&String> = self.key_columns.iter().collect();
        if keys.len() != self.key_columns.len() || !keys.iter().all(|k| unique.contains(k)) {
            return bad("key columns are duplicated or not among the columns".into());
        }
        if self.keyed == self.key_columns.is_empty() {
            return bad("keyed flag disagrees with the key columns".into());
        }
        let controls: BTreeSet<&String> = self.control_columns.iter().collect();
        if controls.len() != self.control_columns.len()
            || !controls.iter().all(|c| unique.contains(c))
        {
            return bad("control-total columns are duplicated or not among the columns".into());
        }
        if self.control_columns.len() != self.control_totals.len() {
            return bad(format!(
                "{} control-total columns but {} control totals",
                self.control_columns.len(),
                self.control_totals.len()
            ));
        }
        match self.level {
            CheckLevel::Structure => {
                bad("structure level is not valid in a fingerprint file".into())
            }
            CheckLevel::CountsAndTotals => {
                if self.buckets.is_empty() {
                    Ok(())
                } else {
                    bad("a counts-only table must not carry bucket summaries".into())
                }
            }
            CheckLevel::FullValues => self.validate_buckets(),
        }
    }

    fn validate_buckets(&self) -> Result<(), FingerprintFileError> {
        let bad = |what: String| {
            Err(FingerprintFileError::Invalid(format!(
                "{}: {what}",
                self.old_qualified
            )))
        };
        if !(1..=MAX_BUCKET_DIGITS).contains(&self.bucket_digits) {
            return bad(format!("bucket digits {} out of range", self.bucket_digits));
        }
        let limit = self.expected_bucket_count();
        let mut seen = BTreeSet::new();
        let mut total: u64 = 0;
        for b in &self.buckets {
            if u64::from(b.bucket) >= limit {
                return bad(format!(
                    "bucket {} out of range for {} digits",
                    b.bucket, self.bucket_digits
                ));
            }
            if !seen.insert(b.bucket) {
                return bad(format!("duplicate bucket {}", b.bucket));
            }
            if b.n == 0 {
                return bad(format!("bucket {} has no rows", b.bucket));
            }
            if b.s.iter().any(|s| *s > u128::from(b.n) * SLICE_MAX) {
                return bad(format!("bucket {} has an impossible hash sum", b.bucket));
            }
            total = match total.checked_add(b.n) {
                Some(t) => t,
                None => return bad("bucket row counts overflow".into()),
            };
        }
        if total != self.rows {
            return bad(format!(
                "bucket row counts add up to {total}, but the table declares {} rows",
                self.rows
            ));
        }
        Ok(())
    }
}

/// The whole fingerprint file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct FingerprintFile {
    pub format_version: u32,
    pub tool_version: String,
    pub mapping_hash: String,
    pub server: CapturedServer,
    pub tables: Vec<CapturedTable>,
    /// The database's structural inventory at capture time.  Always written
    /// by `dmux capture`; a file without one is still read, but the run is
    /// floored at WARNING ("structure not compared").
    pub inventory: Option<Box<DatabaseInventory>>,
    /// SHA-256 over the canonical content (every field except this one), set by
    /// [`FingerprintFile::to_json`] and verified by [`FingerprintFile::from_bytes`].
    pub sha256: String,
}

/// The fields actually hashed: everything except `sha256`.
#[derive(Serialize)]
struct Hashable<'a> {
    format_version: u32,
    tool_version: &'a str,
    mapping_hash: &'a str,
    server: &'a CapturedServer,
    tables: &'a [CapturedTable],
    inventory: &'a Option<Box<DatabaseInventory>>,
}

impl<'a> From<&'a FingerprintFile> for Hashable<'a> {
    fn from(f: &'a FingerprintFile) -> Self {
        Hashable {
            format_version: f.format_version,
            tool_version: &f.tool_version,
            mapping_hash: &f.mapping_hash,
            server: &f.server,
            tables: &f.tables,
            inventory: &f.inventory,
        }
    }
}

/// Canonical JSON (compact, struct field order) of every field except `sha256`,
/// then SHA-256 of that byte string.
fn compute_sha256(f: &FingerprintFile) -> String {
    let canonical = serde_json::to_string(&Hashable::from(f)).unwrap_or_default();
    hex::encode(Sha256::digest(canonical.as_bytes()))
}

impl FingerprintFile {
    /// Verify the stored hash, the version and every structural invariant.  No
    /// field of a file may be used before this has returned `Ok`.
    pub fn verify(&self) -> Result<(), FingerprintFileError> {
        if self.format_version != FINGERPRINT_VERSION {
            return Err(FingerprintFileError::UnsupportedVersion(
                self.format_version,
            ));
        }
        let computed = compute_sha256(self);
        if computed != self.sha256 {
            return Err(FingerprintFileError::HashMismatch {
                expected: self.sha256.clone(),
                actual: computed,
            });
        }
        self.validate()
    }

    /// The structural checks of [`Self::verify`] without the hash (for a file
    /// built in memory).  Every comparison path calls this before using a file.
    pub fn validate(&self) -> Result<(), FingerprintFileError> {
        if self.format_version != FINGERPRINT_VERSION {
            return Err(FingerprintFileError::UnsupportedVersion(
                self.format_version,
            ));
        }
        let mut names = BTreeSet::new();
        for t in &self.tables {
            if !names.insert(t.old_qualified.as_str()) {
                return Err(FingerprintFileError::Invalid(format!(
                    "duplicate table {}",
                    t.old_qualified
                )));
            }
            t.validate()?;
        }
        if let Some(inv) = &self.inventory {
            validate_inventory(inv, &self.tables)?;
        }
        Ok(())
    }

    /// Serialise to canonical JSON with a fresh hash.
    pub fn to_json(mut self) -> Result<Vec<u8>, FingerprintFileError> {
        self.sha256 = compute_sha256(&self);
        serde_json::to_vec(&self).map_err(|e| FingerprintFileError::Json(e.to_string()))
    }

    /// Write to `path` with a fresh hash, gzip-compressing when the name ends
    /// in `.gz`.
    pub fn write_to(&self, path: &std::path::Path) -> Result<(), FingerprintFileError> {
        let bytes = self.clone().to_json()?;
        let io = |e: std::io::Error| FingerprintFileError::Io(e.to_string());
        if path.extension().and_then(|e| e.to_str()) == Some("gz") {
            let mut enc = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
            std::io::Write::write_all(&mut enc, &bytes).map_err(io)?;
            std::fs::write(path, enc.finish().map_err(io)?).map_err(io)
        } else {
            std::fs::write(path, &bytes).map_err(io)
        }
    }

    /// Load from raw bytes, rejecting untrusted input (see the module docs).
    /// Gzip is detected by its magic bytes, not the extension.
    pub fn from_bytes(bytes: &[u8]) -> Result<FingerprintFile, FingerprintFileError> {
        if bytes.len() > MAX_FINGERPRINT_BYTES {
            return Err(FingerprintFileError::TooLarge(bytes.len()));
        }
        let bytes = if bytes.starts_with(&[0x1f, 0x8b]) {
            let mut out = Vec::new();
            // Bound the inflated size too: never inflate beyond the limit.
            flate2::read::GzDecoder::new(bytes)
                .take((MAX_FINGERPRINT_BYTES as u64) + 1)
                .read_to_end(&mut out)
                .map_err(|e| FingerprintFileError::Io(e.to_string()))?;
            out
        } else {
            bytes.to_vec()
        };
        if bytes.len() > MAX_FINGERPRINT_BYTES {
            return Err(FingerprintFileError::TooLarge(bytes.len()));
        }

        let raw: serde_json::Value = serde_json::from_slice(&bytes)
            .map_err(|e| FingerprintFileError::Json(e.to_string()))?;
        let version = raw
            .get("format_version")
            .and_then(serde_json::Value::as_u64)
            .ok_or_else(|| FingerprintFileError::Json("missing format_version".into()))?;
        if version != u64::from(FINGERPRINT_VERSION) {
            return Err(FingerprintFileError::UnsupportedVersion(
                u32::try_from(version).unwrap_or(u32::MAX),
            ));
        }

        let file: FingerprintFile = serde_json::from_slice(&bytes)
            .map_err(|e| FingerprintFileError::Json(e.to_string()))?;

        // Unknown fields at any depth: every key of the input must survive a
        // round trip through the typed value.
        let canonical: serde_json::Value = serde_json::to_vec(&file)
            .and_then(|v| serde_json::from_slice(&v))
            .map_err(|e| FingerprintFileError::Json(e.to_string()))?;
        let mut unknown = Vec::new();
        unknown_keys(&raw, &canonical, "", &mut unknown);
        if !unknown.is_empty() {
            return Err(FingerprintFileError::UnknownFields(unknown));
        }

        file.verify()?;
        Ok(file)
    }

    /// Load from a path.
    pub fn read_from(path: &std::path::Path) -> Result<FingerprintFile, FingerprintFileError> {
        let len = std::fs::metadata(path)
            .map_err(|e| FingerprintFileError::Io(e.to_string()))?
            .len();
        if len > MAX_FINGERPRINT_BYTES as u64 {
            return Err(FingerprintFileError::TooLarge(
                usize::try_from(len).unwrap_or(usize::MAX),
            ));
        }
        let bytes = std::fs::read(path).map_err(|e| FingerprintFileError::Io(e.to_string()))?;
        Self::from_bytes(&bytes)
    }
}

/// The inventory's own consistency: unique schema names, unique table and
/// column names, and every captured table present in it.
fn validate_inventory(
    inv: &DatabaseInventory,
    tables: &[CapturedTable],
) -> Result<(), FingerprintFileError> {
    let bad = |what: String| Err(FingerprintFileError::Invalid(format!("inventory: {what}")));
    let mut schemas = BTreeSet::new();
    let mut all = BTreeSet::new();
    for s in &inv.schemas {
        if !schemas.insert(s.name.as_str()) {
            return bad(format!("duplicate schema {}", s.name));
        }
        for t in &s.tables {
            if !all.insert(format!("{}.{}", s.name, t.name)) {
                return bad(format!("duplicate table {}.{}", s.name, t.name));
            }
            let mut cols = BTreeSet::new();
            if !t.columns.iter().all(|c| cols.insert(c.name.as_str())) {
                return bad(format!("duplicate column in {}.{}", s.name, t.name));
            }
        }
    }
    for t in tables {
        if !all.contains(&t.old_qualified) {
            return bad(format!(
                "captured table {} is not in the inventory",
                t.old_qualified
            ));
        }
    }
    Ok(())
}

/// Collect every object key present in `raw` but absent at the same place in
/// `canonical`.
fn unknown_keys(
    raw: &serde_json::Value,
    canonical: &serde_json::Value,
    at: &str,
    out: &mut Vec<String>,
) {
    use serde_json::Value;
    match (raw, canonical) {
        (Value::Object(r), Value::Object(c)) => {
            for (k, v) in r {
                let here = if at.is_empty() {
                    k.clone()
                } else {
                    format!("{at}.{k}")
                };
                match c.get(k) {
                    Some(cv) => unknown_keys(v, cv, &here, out),
                    None => out.push(here),
                }
            }
        }
        (Value::Array(r), Value::Array(c)) => {
            for (i, (rv, cv)) in r.iter().zip(c.iter()).enumerate() {
                unknown_keys(rv, cv, &format!("{at}[{i}]"), out);
            }
        }
        _ => {}
    }
}

/// Test helpers: a small inventory with the given `schema.table` names, each
/// with an `id` column, a primary key and one index.
#[cfg(test)]
pub(crate) mod test_inventory {
    use crate::inventory::{
        Column, Constraint, ConstraintKind, DatabaseInventory, Index, Schema, Table,
    };
    use std::collections::BTreeMap;

    pub(crate) fn table(name: &str) -> Table {
        Table {
            name: name.into(),
            owner: "o".into(),
            has_oids: false,
            is_partitioned: false,
            columns: vec![Column {
                name: "id".into(),
                type_name: "integer".into(),
                length: None,
                nullable: false,
                default: None,
            }],
            constraints: vec![Constraint {
                name: format!("{name}_pkey"),
                kind: ConstraintKind::PrimaryKey,
                validated: true,
                definition: "PRIMARY KEY (id)".into(),
            }],
            indexes: vec![Index {
                name: format!("ix_{name}"),
                valid: true,
                definition: format!("CREATE INDEX ix_{name} ON {name} (id)"),
            }],
            partitions: vec![],
            inherited_from: vec![],
            n_live_tup: 0,
            size_bytes: 0,
            last_analyze: None,
            stats_missing: false,
            warnings: vec![],
        }
    }

    pub(crate) fn of(tables: &[&str]) -> DatabaseInventory {
        let mut schemas: BTreeMap<String, Schema> = BTreeMap::new();
        for q in tables {
            let (s, t) = q.split_once('.').expect("schema.table");
            schemas
                .entry(s.to_string())
                .or_insert_with(|| Schema {
                    name: s.into(),
                    owner: "o".into(),
                    tables: vec![],
                    views: vec![],
                    materialized_views: vec![],
                    functions: vec![],
                    triggers: vec![],
                    sequences: vec![],
                })
                .tables
                .push(table(t));
        }
        DatabaseInventory {
            server_version: "17".into(),
            server_version_num: 170000,
            encoding: "UTF8".into(),
            collation: "C".into(),
            ctype: "C".into(),
            lc_monetary: "C".into(),
            schemas: schemas.into_values().collect(),
            extensions: vec![],
            large_object_count: 0,
            warnings: vec![],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::fingerprint::{BucketSummary, Decimal};

    fn table() -> CapturedTable {
        CapturedTable {
            old_qualified: "demo_bank.accounts".into(),
            level: CheckLevel::FullValues,
            keyed: true,
            bucket_digits: 3,
            columns: vec!["account_id".into(), "balance".into()],
            key_columns: vec!["account_id".into()],
            column_types: vec![PgType::Int8, PgType::Numeric(Some((20, 4)))],
            options: vec![ColumnOptions::default(), ColumnOptions::default()],
            control_columns: vec!["balance".into()],
            rows: 3,
            buckets: vec![
                BucketSummary {
                    bucket: 0,
                    n: 1,
                    s: [1, 2, 3, 4],
                },
                BucketSummary {
                    bucket: 7,
                    n: 2,
                    s: [5, 6, 7, 8],
                },
            ],
            control_totals: vec![Decimal::parse("1.0").unwrap()],
            needs_review: false,
        }
    }

    fn sample() -> FingerprintFile {
        FingerprintFile {
            format_version: FINGERPRINT_VERSION,
            tool_version: "test".to_string(),
            mapping_hash: "mhash".to_string(),
            server: CapturedServer {
                version_num: 170000,
                version_string: "17.0".into(),
                encoding: "UTF8".into(),
                collation: "en_US.utf8".into(),
                sql_ascii: false,
            },
            tables: vec![table()],
            inventory: Some(Box::new(test_inventory::of(&["demo_bank.accounts"]))),
            sha256: String::new(),
        }
    }

    /// Serialise with a correct hash, then parse: only structure can fail.
    fn round(f: FingerprintFile) -> Result<FingerprintFile, FingerprintFileError> {
        FingerprintFile::from_bytes(&f.to_json().unwrap())
    }

    fn invalid(f: FingerprintFile, needle: &str) {
        match round(f) {
            Err(FingerprintFileError::Invalid(m)) => assert!(m.contains(needle), "{m}"),
            other => panic!("expected Invalid({needle}), got {other:?}"),
        }
    }

    #[test]
    fn round_trips_and_verifies() {
        let back = round(sample()).unwrap();
        assert_eq!(back.tables, sample().tables);
    }

    #[test]
    fn round_trips_through_gzip() {
        let dir = std::env::temp_dir().join(format!("dmux-fp-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("fp.json.gz");
        sample().write_to(&path).unwrap();
        assert_eq!(
            FingerprintFile::read_from(&path).unwrap().tables,
            sample().tables
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn rejects_tampering() {
        let mut json = sample().to_json().unwrap();
        let mid = json.len() / 2;
        json[mid] ^= 0x01;
        assert!(FingerprintFile::from_bytes(&json).is_err());
        // A semantic edit with the old hash kept.
        let mut v: serde_json::Value =
            serde_json::from_slice(&sample().to_json().unwrap()).unwrap();
        v["tables"][0]["rows"] = serde_json::json!(4);
        v["tables"][0]["buckets"][1]["n"] = serde_json::json!(3);
        assert!(matches!(
            FingerprintFile::from_bytes(&serde_json::to_vec(&v).unwrap()),
            Err(FingerprintFileError::HashMismatch { .. })
        ));
    }

    #[test]
    fn rejects_unknown_fields_at_any_depth() {
        let json = sample().to_json().unwrap();
        let base: serde_json::Value = serde_json::from_slice(&json).unwrap();
        for path in [
            &[][..],
            &["tables", "0"][..],
            &["tables", "0", "buckets", "0"][..],
            &["tables", "0", "options", "0"][..],
            &["server"][..],
            &["inventory"][..],
            &["inventory", "schemas", "0", "tables", "0"][..],
            &["inventory", "schemas", "0", "tables", "0", "indexes", "0"][..],
        ] {
            let mut v = base.clone();
            let mut node = &mut v;
            for seg in path {
                node = match seg.parse::<usize>() {
                    Ok(i) => &mut node[i],
                    Err(_) => &mut node[*seg],
                };
            }
            node.as_object_mut()
                .unwrap()
                .insert("extra".into(), serde_json::json!(1));
            let r = FingerprintFile::from_bytes(&serde_json::to_vec(&v).unwrap());
            assert!(
                matches!(
                    r,
                    Err(FingerprintFileError::UnknownFields(_))
                        | Err(FingerprintFileError::Json(_))
                ),
                "extra field at {path:?} must be rejected, got {r:?}"
            );
        }
    }

    #[test]
    fn rejects_old_or_future_versions() {
        let mut v: serde_json::Value =
            serde_json::from_slice(&sample().to_json().unwrap()).unwrap();
        v["format_version"] = serde_json::json!(1);
        assert_eq!(
            FingerprintFile::from_bytes(&serde_json::to_vec(&v).unwrap()),
            Err(FingerprintFileError::UnsupportedVersion(1))
        );
    }

    #[test]
    fn rejects_oversized_input() {
        let big = vec![b' '; MAX_FINGERPRINT_BYTES + 1];
        assert!(matches!(
            FingerprintFile::from_bytes(&big),
            Err(FingerprintFileError::TooLarge(_))
        ));
    }

    #[test]
    fn rejects_duplicate_tables() {
        let mut f = sample();
        f.tables.push(table());
        match round(f) {
            Err(FingerprintFileError::Invalid(m)) => assert!(m.contains("duplicate table")),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn rejects_duplicate_buckets() {
        let mut f = sample();
        f.tables[0].buckets[1].bucket = 0;
        invalid(f, "duplicate bucket 0");
    }

    #[test]
    fn rejects_bucket_counts_that_do_not_add_up() {
        let mut f = sample();
        f.tables[0].rows = 4;
        invalid(f, "add up to 3, but the table declares 4 rows");
    }

    #[test]
    fn rejects_column_and_type_count_mismatch() {
        let mut f = sample();
        f.tables[0].column_types.pop();
        invalid(f, "2 columns but 1 column types");
        let mut f = sample();
        f.tables[0].options.pop();
        invalid(f, "option sets");
    }

    #[test]
    fn rejects_other_inconsistencies() {
        let mut f = sample();
        f.tables[0].buckets[0].bucket = 4096;
        invalid(f, "out of range");
        let mut f = sample();
        f.tables[0].key_columns = vec!["nope".into()];
        invalid(f, "key columns");
        let mut f = sample();
        f.tables[0].keyed = false;
        invalid(f, "keyed flag");
        let mut f = sample();
        f.tables[0].control_columns.clear();
        invalid(f, "control totals");
        let mut f = sample();
        f.tables[0].buckets[0].s[0] = SLICE_MAX + 1;
        invalid(f, "impossible hash sum");
        let mut f = sample();
        f.tables[0].level = CheckLevel::Structure;
        invalid(f, "structure level");
        let mut f = sample();
        f.tables[0].level = CheckLevel::CountsAndTotals;
        invalid(f, "counts-only table must not carry bucket");
        let mut f = sample();
        f.tables[0].old_qualified = "accounts".into();
        invalid(f, "schema-qualified");
    }

    #[test]
    fn carries_and_hashes_the_inventory() {
        let back = round(sample()).unwrap();
        assert_eq!(back.inventory, sample().inventory);
        // Editing the inventory with the old hash kept is tampering.
        let mut v: serde_json::Value =
            serde_json::from_slice(&sample().to_json().unwrap()).unwrap();
        v["inventory"]["schemas"][0]["tables"][0]["indexes"] = serde_json::json!([]);
        assert!(matches!(
            FingerprintFile::from_bytes(&serde_json::to_vec(&v).unwrap()),
            Err(FingerprintFileError::HashMismatch { .. })
        ));
        // Version 2 files (no inventory) are refused.
        let mut v: serde_json::Value =
            serde_json::from_slice(&sample().to_json().unwrap()).unwrap();
        v["format_version"] = serde_json::json!(2);
        assert_eq!(
            FingerprintFile::from_bytes(&serde_json::to_vec(&v).unwrap()),
            Err(FingerprintFileError::UnsupportedVersion(2))
        );
    }

    #[test]
    fn rejects_an_inconsistent_inventory() {
        let mut f = sample();
        let inv = f.inventory.as_mut().unwrap();
        let dup = inv.schemas[0].clone();
        inv.schemas.push(dup);
        invalid(f, "duplicate schema demo_bank");
        let mut f = sample();
        let t = f.inventory.as_ref().unwrap().schemas[0].tables[0].clone();
        f.inventory.as_mut().unwrap().schemas[0].tables.push(t);
        invalid(f, "duplicate table demo_bank.accounts");
        let mut f = sample();
        f.inventory = Some(Box::new(test_inventory::of(&["demo_bank.other"])));
        invalid(
            f,
            "captured table demo_bank.accounts is not in the inventory",
        );
        let mut f = sample();
        let c = f.inventory.as_ref().unwrap().schemas[0].tables[0].columns[0].clone();
        f.inventory.as_mut().unwrap().schemas[0].tables[0]
            .columns
            .push(c);
        invalid(f, "duplicate column");
    }
}
