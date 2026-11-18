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
//!
//! Two further checks close audit H3.  The file's `normalise_rules_version`
//! must equal [`NORMALISE_RULES_VERSION`]: a capture made under other rules
//! hashed its values differently and is refused (a missing version is refused
//! too).  And each full-values table's `needs_review` is recomputed from its
//! own `column_types` and `options` with the core planner: a file that claims
//! `false` where the planner says `true` has been edited and is refused.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::io::Read;
use thiserror::Error;

use crate::core::fingerprint::{BucketSummary, Decimal, MAX_BUCKET_DIGITS};
use crate::core::normalise::{
    plan_column, ColumnMapping, ColumnOptions, PgType, NORMALISE_RULES_VERSION,
};
use crate::inventory::DatabaseInventory;
use crate::results::CheckLevel;

/// Current fingerprint-file format version.  Version 3 added the full
/// structural `inventory` (PLAN.md 3.3), so saved-file comparisons check
/// structure exactly like live ones.  Version 2 added `column_types`,
/// `options` and `control_columns` to every table.
pub const FINGERPRINT_VERSION: u32 = 3;

/// Maximum accepted fingerprint file size (uncompressed), in bytes.
///
/// A table of more than about 40,000 rows holds 4,096 bucket summaries (about
/// 370 KB of JSON), and the largest tables hold up to 16 times more per extra
/// bucket digit, so a capture of a large core banking database runs to
/// gigabytes.  The owner raised the limit from 64 MiB (audit L2) to 8 GiB for
/// very large databases (decision 29).  Reading is built for that size: the
/// text is held once, each table is checked on its own, and the hash is
/// streamed, so peak memory is about twice the file (see [`FingerprintFile::from_bytes`]).
pub const MAX_FINGERPRINT_BYTES: usize = 8 * 1024 * 1024 * 1024;

/// One 32-bit slice of an md5, the most a single row adds to a bucket sum.
const SLICE_MAX: u128 = 0xffff_ffff;

/// Errors when reading or writing a fingerprint file.
#[derive(Debug, Clone, Error, PartialEq, Eq)]
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
    /// The file carries no normalisation-rules version (made before it was
    /// recorded), so what its hashes mean is unknown.
    #[error(
        "the capture does not record its normalisation rules version; capture it again with this version of Dmux (rules version {current})"
    )]
    RulesVersionMissing { current: u32 },
    /// The file was captured under other normalisation rules: its hashes are
    /// not comparable with what this version computes.
    #[error(
        "the capture was made with normalisation rules version {found}, but this version of Dmux uses {current}; capture it again with this version"
    )]
    RulesVersionMismatch { found: u32, current: u32 },
    /// A table says it needs no review, but its own column types and options
    /// say it does: the file was edited.
    #[error(
        "{table}: the file says the table needs no review, but its column types or options require review; the file has been edited"
    )]
    NeedsReviewCleared { table: String },
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
    /// As stored.  Never trusted alone: see [`CapturedTable::effective_needs_review`].
    pub needs_review: bool,
}

impl CapturedTable {
    /// Whether the planner flags any column of this table for review, from the
    /// table's own `column_types` and `options` (both sides of a capture use
    /// the same type).  Only full-values tables hash values, so a counts-only
    /// table never needs review on these grounds.  A column the planner
    /// refuses is an error.
    fn recomputed_needs_review(&self) -> Result<bool, FingerprintFileError> {
        if self.level != CheckLevel::FullValues {
            return Ok(false);
        }
        let mut any = false;
        for ((name, ty), options) in self
            .columns
            .iter()
            .zip(self.column_types.iter())
            .zip(self.options.iter())
        {
            let m = ColumnMapping {
                old_name: name.clone(),
                new_name: name.clone(),
                old_type: ty.clone(),
                new_type: ty.clone(),
                options: options.clone(),
                control_total: self.control_columns.contains(name),
            };
            let plan = plan_column(&m).map_err(|e| {
                FingerprintFileError::Invalid(format!(
                    "{}: column {name} cannot be planned: {e}",
                    self.old_qualified
                ))
            })?;
            any |= plan.needs_review();
        }
        Ok(any)
    }

    /// The stored flag OR the recomputed one: what a comparison must use.
    /// [`FingerprintFile::validate`] has already refused a file where the two
    /// disagree in the unsafe direction; this keeps the rule even for a file
    /// that skipped it.
    pub fn effective_needs_review(&self) -> bool {
        self.needs_review || self.recomputed_needs_review().unwrap_or(true)
    }

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
        if self.recomputed_needs_review()? && !self.needs_review {
            return Err(FingerprintFileError::NeedsReviewCleared {
                table: self.old_qualified.clone(),
            });
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
    /// The [`NORMALISE_RULES_VERSION`] the hashes were computed under.  Absent
    /// in files written before it was recorded; such files are refused.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub normalise_rules_version: Option<u32>,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    normalise_rules_version: Option<u32>,
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
            normalise_rules_version: f.normalise_rules_version,
            mapping_hash: &f.mapping_hash,
            server: &f.server,
            tables: &f.tables,
            inventory: &f.inventory,
        }
    }
}

/// Canonical JSON (compact, struct field order) of every field except `sha256`,
/// then SHA-256 of that byte string.  The JSON is streamed into the hasher, so
/// a large capture is never held a second time as text; the bytes hashed are
/// exactly those `serde_json::to_string` would produce.
fn compute_sha256(f: &FingerprintFile) -> String {
    let mut hasher = HashWriter(Sha256::new());
    if serde_json::to_writer(&mut hasher, &Hashable::from(f)).is_err() {
        return String::new();
    }
    hex::encode(hasher.0.finalize())
}

/// An `io::Write` that feeds a SHA-256.
struct HashWriter(Sha256);

impl std::io::Write for HashWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.update(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// The stored file, borrowed: the same fields in the same order as
/// [`FingerprintFile`]'s own serialisation, so a capture can be written
/// without cloning it.
#[derive(Serialize)]
struct Stored<'a> {
    format_version: u32,
    tool_version: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    normalise_rules_version: Option<u32>,
    mapping_hash: &'a str,
    server: &'a CapturedServer,
    tables: &'a [CapturedTable],
    inventory: &'a Option<Box<DatabaseInventory>>,
    sha256: &'a str,
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
        self.check_rules_version()?;
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
        self.check_rules_version()?;
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

    /// Refuse a file captured under other normalisation rules, or one that
    /// does not say which rules it used (decision 23).
    pub fn check_rules_version(&self) -> Result<(), FingerprintFileError> {
        match self.normalise_rules_version {
            None => Err(FingerprintFileError::RulesVersionMissing {
                current: NORMALISE_RULES_VERSION,
            }),
            Some(v) if v != NORMALISE_RULES_VERSION => {
                Err(FingerprintFileError::RulesVersionMismatch {
                    found: v,
                    current: NORMALISE_RULES_VERSION,
                })
            }
            Some(_) => Ok(()),
        }
    }

    /// A run warning when the file was written by another Dmux version whose
    /// normalisation rules match this one (decision 23): the hashes compare,
    /// but the signer should know.  `side` is "old" or "new".
    pub fn tool_version_warning(&self, side: &str) -> Option<String> {
        let running = crate::version();
        (self.tool_version != running).then(|| {
            format!(
                "the {side} capture was made by Dmux {}, and this run uses Dmux {running}; both use normalisation rules version {NORMALISE_RULES_VERSION}",
                sanitize_version(&self.tool_version)
            )
        })
    }

    /// Serialise to canonical JSON with a fresh hash.
    pub fn to_json(mut self) -> Result<Vec<u8>, FingerprintFileError> {
        self.sha256 = compute_sha256(&self);
        serde_json::to_vec(&self).map_err(|e| FingerprintFileError::Json(e.to_string()))
    }

    /// Write to `path` with a fresh hash, gzip-compressing when the name ends
    /// in `.gz`.
    ///
    /// Streamed: the capture is neither cloned nor built as one string first,
    /// so writing a capture of a very large database needs no extra memory.
    /// The bytes are exactly those of [`Self::to_json`].
    pub fn write_to(&self, path: &std::path::Path) -> Result<(), FingerprintFileError> {
        use std::io::Write;
        let io = |e: std::io::Error| FingerprintFileError::Io(e.to_string());
        let sha256 = compute_sha256(self);
        let stored = Stored {
            format_version: self.format_version,
            tool_version: &self.tool_version,
            normalise_rules_version: self.normalise_rules_version,
            mapping_hash: &self.mapping_hash,
            server: &self.server,
            tables: &self.tables,
            inventory: &self.inventory,
            sha256: &sha256,
        };
        let json = |e: serde_json::Error| FingerprintFileError::Json(e.to_string());
        let file = std::io::BufWriter::new(std::fs::File::create(path).map_err(io)?);
        if path.extension().and_then(|e| e.to_str()) == Some("gz") {
            let mut enc = flate2::write::GzEncoder::new(file, flate2::Compression::default());
            serde_json::to_writer(&mut enc, &stored).map_err(json)?;
            enc.finish().map_err(io)?.flush().map_err(io)
        } else {
            let mut file = file;
            serde_json::to_writer(&mut file, &stored).map_err(json)?;
            file.flush().map_err(io)
        }
    }

    /// Load from raw bytes, rejecting untrusted input (see the module docs).
    /// Gzip is detected by its magic bytes, not the extension.
    ///
    /// Built for captures of very large databases.  The top level is read as
    /// borrowed slices of the input (nothing copied); each table is then parsed
    /// on its own into a `serde_json::Value`, turned into its typed form and
    /// compared key by key with that form's canonical value (the unknown-field
    /// check), and the `Value` is dropped before the next table.  Peak memory is
    /// the text plus the typed file plus one table, about twice the file.
    pub fn from_bytes(bytes: &[u8]) -> Result<FingerprintFile, FingerprintFileError> {
        Self::from_bytes_within(bytes, MAX_FINGERPRINT_BYTES)
    }

    /// [`Self::from_bytes`] with an explicit size limit (tests use a small one).
    fn from_bytes_within(
        bytes: &[u8],
        limit: usize,
    ) -> Result<FingerprintFile, FingerprintFileError> {
        if bytes.len() > limit {
            return Err(FingerprintFileError::TooLarge(bytes.len()));
        }
        if bytes.starts_with(&[0x1f, 0x8b]) {
            let mut out = Vec::new();
            // Bound the inflated size too: never inflate beyond the limit.
            flate2::read::GzDecoder::new(bytes)
                .take((limit as u64) + 1)
                .read_to_end(&mut out)
                .map_err(|e| FingerprintFileError::Io(e.to_string()))?;
            if out.len() > limit {
                return Err(FingerprintFileError::TooLarge(out.len()));
            }
            Self::from_json_text(&out)
        } else {
            Self::from_json_text(bytes)
        }
    }

    /// [`Self::from_bytes`] on uncompressed JSON text.
    fn from_json_text(text: &[u8]) -> Result<FingerprintFile, FingerprintFileError> {
        use serde_json::value::RawValue;
        use serde_json::{Map, Value};
        let json = |e: serde_json::Error| FingerprintFileError::Json(e.to_string());

        let mut top: std::collections::BTreeMap<String, &RawValue> =
            serde_json::from_slice(text).map_err(json)?;

        let version = top
            .get("format_version")
            .and_then(|r| serde_json::from_str::<u64>(r.get()).ok())
            .ok_or_else(|| FingerprintFileError::Json("missing format_version".into()))?;
        if version != u64::from(FINGERPRINT_VERSION) {
            return Err(FingerprintFileError::UnsupportedVersion(
                u32::try_from(version).unwrap_or(u32::MAX),
            ));
        }

        let raw_tables = top
            .remove("tables")
            .ok_or_else(|| FingerprintFileError::Json("missing field `tables`".into()))?;
        let raw_inventory = top.remove("inventory");

        // The header: every small top-level field, typed through the whole
        // file's own (deny_unknown_fields) struct with the large parts empty.
        let mut header = Map::new();
        for (k, v) in &top {
            header.insert(
                k.clone(),
                serde_json::from_str::<Value>(v.get()).map_err(json)?,
            );
        }
        let raw_header = Value::Object(header.clone());
        header.insert("tables".into(), Value::Array(Vec::new()));
        header.insert("inventory".into(), Value::Null);
        let mut file = FingerprintFile::deserialize(&Value::Object(header)).map_err(json)?;

        // Unknown fields at any depth: every key of the input must survive a
        // round trip through the typed value.  Checked piece by piece.
        let mut unknown = Vec::new();
        let canon_header = serde_json::to_value(&file).map_err(json)?;
        unknown_keys(&raw_header, &canon_header, "", &mut unknown);
        drop(raw_header);

        let rows: Vec<&RawValue> = serde_json::from_str(raw_tables.get()).map_err(json)?;
        let mut tables = Vec::with_capacity(rows.len());
        for (i, rt) in rows.iter().enumerate() {
            let v: Value = serde_json::from_str(rt.get()).map_err(json)?;
            let t = CapturedTable::deserialize(&v).map_err(json)?;
            let canon = serde_json::to_value(&t).map_err(json)?;
            unknown_keys(&v, &canon, &format!("tables[{i}]"), &mut unknown);
            tables.push(t);
        }
        file.tables = tables;

        if let Some(ri) = raw_inventory {
            let v: Value = serde_json::from_str(ri.get()).map_err(json)?;
            let inv: Option<Box<DatabaseInventory>> = Option::deserialize(&v).map_err(json)?;
            if let Some(inv) = &inv {
                let canon = serde_json::to_value(inv).map_err(json)?;
                unknown_keys(&v, &canon, "inventory", &mut unknown);
            }
            file.inventory = inv;
        }

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

/// A tool version read from an untrusted file, made safe to print in a warning:
/// only version characters, at most 40 of them.
fn sanitize_version(v: &str) -> String {
    let kept: String = v
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '+' | '_'))
        .take(40)
        .collect();
    if kept.is_empty() {
        "(unknown version)".into()
    } else {
        kept
    }
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
            normalise_rules_version: Some(NORMALISE_RULES_VERSION),
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
    fn the_streamed_hash_and_file_are_byte_for_byte_the_old_ones() {
        let f = sample();
        // The hash every existing capture carries: SHA-256 of the compact text.
        let text = serde_json::to_string(&Hashable::from(&f)).unwrap();
        assert_eq!(
            compute_sha256(&f),
            hex::encode(Sha256::digest(text.as_bytes()))
        );
        let dir = std::env::temp_dir().join(format!("dmux-fp-stream-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let plain = dir.join("c.json");
        f.write_to(&plain).unwrap();
        assert_eq!(std::fs::read(&plain).unwrap(), f.clone().to_json().unwrap());
        let gz = dir.join("c.json.gz");
        f.write_to(&gz).unwrap();
        let mut out = Vec::new();
        flate2::read::GzDecoder::new(std::fs::File::open(&gz).unwrap())
            .read_to_end(&mut out)
            .unwrap();
        assert_eq!(out, f.clone().to_json().unwrap());
        assert_eq!(FingerprintFile::read_from(&gz).unwrap().tables, f.tables);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_file_without_tables_is_still_refused() {
        let mut v: serde_json::Value =
            serde_json::from_slice(&sample().to_json().unwrap()).unwrap();
        v.as_object_mut().unwrap().remove("tables");
        assert!(matches!(
            FingerprintFile::from_bytes(&serde_json::to_vec(&v).unwrap()),
            Err(FingerprintFileError::Json(_))
        ));
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

    /// The size checks, against a small limit so the tests stay fast.
    const SMALL: usize = 64 * 1024;

    #[test]
    fn rejects_oversized_input() {
        let big = vec![b' '; SMALL + 1];
        assert_eq!(
            FingerprintFile::from_bytes_within(&big, SMALL),
            Err(FingerprintFileError::TooLarge(SMALL + 1))
        );
        // Exactly at the limit is not refused for size (it fails as JSON).
        let at = vec![b' '; SMALL];
        assert!(matches!(
            FingerprintFile::from_bytes_within(&at, SMALL),
            Err(FingerprintFileError::Json(_))
        ));
    }

    #[test]
    fn the_size_limit_is_8_gib() {
        // Raised from 64 MiB for very large databases (decision 29).
        assert_eq!(MAX_FINGERPRINT_BYTES, 8 * 1024 * 1024 * 1024);
    }

    #[test]
    fn rejects_a_gzip_that_inflates_past_the_limit() {
        let mut enc = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::best());
        std::io::Write::write_all(&mut enc, &vec![b' '; SMALL + 1]).unwrap();
        let gz = enc.finish().unwrap();
        assert!(gz.len() < SMALL);
        assert_eq!(
            FingerprintFile::from_bytes_within(&gz, SMALL),
            Err(FingerprintFileError::TooLarge(SMALL + 1))
        );
        // Inflating to exactly the limit is not refused for size.
        let mut enc = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::best());
        std::io::Write::write_all(&mut enc, &vec![b' '; SMALL]).unwrap();
        assert!(matches!(
            FingerprintFile::from_bytes_within(&enc.finish().unwrap(), SMALL),
            Err(FingerprintFileError::Json(_))
        ));
    }

    #[test]
    fn rejects_an_oversized_file_on_disk() {
        let dir = std::env::temp_dir().join(format!("dmux-fp-big-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("big.json");
        let f = std::fs::File::create(&path).unwrap();
        f.set_len(MAX_FINGERPRINT_BYTES as u64 + 1).unwrap();
        let r = FingerprintFile::read_from(&path);
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(
            r,
            Err(FingerprintFileError::TooLarge(MAX_FINGERPRINT_BYTES + 1))
        );
    }

    #[test]
    fn a_capture_from_other_normalisation_rules_is_refused() {
        for other in [0, NORMALISE_RULES_VERSION + 1, u32::MAX] {
            let mut f = sample();
            f.normalise_rules_version = Some(other);
            // The hash is recomputed, as an editor would: still refused.
            assert_eq!(
                round(f.clone()),
                Err(FingerprintFileError::RulesVersionMismatch {
                    found: other,
                    current: NORMALISE_RULES_VERSION
                })
            );
            assert_eq!(
                f.validate(),
                Err(FingerprintFileError::RulesVersionMismatch {
                    found: other,
                    current: NORMALISE_RULES_VERSION
                })
            );
        }
    }

    #[test]
    fn a_capture_without_a_rules_version_is_refused() {
        let mut f = sample();
        f.normalise_rules_version = None;
        let json = f.clone().to_json().unwrap();
        // Written as an older Dmux would: no key at all.
        let v: serde_json::Value = serde_json::from_slice(&json).unwrap();
        assert!(v.get("normalise_rules_version").is_none());
        let missing = FingerprintFileError::RulesVersionMissing {
            current: NORMALISE_RULES_VERSION,
        };
        assert_eq!(FingerprintFile::from_bytes(&json), Err(missing.clone()));
        assert_eq!(f.validate(), Err(missing));
        // An explicit null is not a version either.
        let mut v = v;
        v["normalise_rules_version"] = serde_json::Value::Null;
        assert!(FingerprintFile::from_bytes(&serde_json::to_vec(&v).unwrap()).is_err());
    }

    #[test]
    fn the_rules_version_is_written_and_hashed() {
        let json = sample().to_json().unwrap();
        let v: serde_json::Value = serde_json::from_slice(&json).unwrap();
        assert_eq!(
            v["normalise_rules_version"],
            serde_json::json!(NORMALISE_RULES_VERSION)
        );
        // Changing it with the old hash kept is a hash mismatch, not accepted.
        let mut v = v;
        v["normalise_rules_version"] = serde_json::json!(NORMALISE_RULES_VERSION);
        v["tool_version"] = serde_json::json!("edited");
        assert!(matches!(
            FingerprintFile::from_bytes(&serde_json::to_vec(&v).unwrap()),
            Err(FingerprintFileError::HashMismatch { .. })
        ));
    }

    /// A table whose types need review, with the flag forged to `false`.
    fn forged(ty: PgType) -> FingerprintFile {
        let mut f = sample();
        f.tables[0].columns = vec!["account_id".into(), "shape".into()];
        f.tables[0].column_types = vec![PgType::Int8, ty];
        f.tables[0].control_columns.clear();
        f.tables[0].control_totals.clear();
        f.tables[0].needs_review = false;
        f
    }

    fn review_types() -> Vec<PgType> {
        vec![
            PgType::Unknown("public.geometry".into()),
            PgType::Composite {
                name: "public.pt".into(),
                fields: vec![PgType::Float8, PgType::Int4],
            },
            PgType::Range {
                name: "public.georange".into(),
                sub: Box::new(PgType::Unknown("public.geometry".into())),
            },
            PgType::Array(Box::new(PgType::Unknown("public.geometry".into()))),
            PgType::Domain {
                name: "public.d".into(),
                base: Box::new(PgType::Unknown("x".into())),
            },
        ]
    }

    #[test]
    fn a_forged_needs_review_false_is_refused() {
        for ty in review_types() {
            let f = forged(ty.clone());
            let cleared = FingerprintFileError::NeedsReviewCleared {
                table: "demo_bank.accounts".into(),
            };
            // Hash recomputed by the forger: the planner still catches it.
            assert_eq!(round(f.clone()), Err(cleared.clone()), "{ty:?}");
            assert_eq!(f.validate(), Err(cleared), "{ty:?}");
            assert!(f.tables[0].effective_needs_review(), "{ty:?}");
            // Stored honestly, the same file is accepted and needs review.
            let mut ok = f;
            ok.tables[0].needs_review = true;
            let back = round(ok).unwrap();
            assert!(back.tables[0].effective_needs_review());
        }
    }

    #[test]
    fn needs_review_is_recomputed_only_for_full_values() {
        // A counts-only table hashes no values, so its types do not matter.
        let mut f = forged(PgType::Unknown("x".into()));
        f.tables[0].level = CheckLevel::CountsAndTotals;
        f.tables[0].buckets.clear();
        f.tables[0].bucket_digits = 3;
        f.tables[0].rows = 3;
        assert!(round(f.clone()).is_ok());
        assert!(!f.tables[0].effective_needs_review());
    }

    #[test]
    fn stored_needs_review_true_is_kept_even_when_types_are_exact() {
        let mut f = sample();
        f.tables[0].needs_review = true;
        let back = round(f).unwrap();
        assert!(back.tables[0].effective_needs_review());
        // And a plain exact table stays clean.
        assert!(!sample().tables[0].effective_needs_review());
    }

    #[test]
    fn a_column_the_planner_refuses_is_invalid() {
        // A control total on a text column cannot be planned.
        let mut f = sample();
        f.tables[0].column_types[1] = PgType::Text;
        invalid(f.clone(), "column balance cannot be planned");
        // effective_needs_review fails closed when the plan cannot be built.
        assert!(f.tables[0].effective_needs_review());
    }

    #[test]
    fn tool_version_warning_only_when_versions_differ() {
        let mut f = sample();
        f.tool_version = crate::version().into();
        assert_eq!(f.tool_version_warning("old"), None);
        f.tool_version = "0.0.1".into();
        let w = f.tool_version_warning("old").unwrap();
        assert!(w.contains("old capture was made by Dmux 0.0.1"), "{w}");
        assert!(w.contains(crate::version()), "{w}");
        assert!(
            w.contains(&format!("rules version {NORMALISE_RULES_VERSION}")),
            "{w}"
        );
        // An untrusted version string never carries control or markup text.
        f.tool_version = "1.0\n<script>\u{202e}".into();
        let w = f.tool_version_warning("new").unwrap();
        assert!(w.contains("Dmux 1.0script, and"), "{w}");
        f.tool_version = "\u{0}".into();
        assert!(f
            .tool_version_warning("new")
            .unwrap()
            .contains("(unknown version)"));
        assert_eq!(sanitize_version(&"9".repeat(100)).len(), 40);
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
