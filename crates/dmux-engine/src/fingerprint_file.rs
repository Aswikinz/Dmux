//! The fingerprint file format produced by `dmux capture` and consumed by
//! `dmux compare` (PLAN.md 3.3).
//!
//! It is **versioned JSON**, optionally gzip-compressed, carrying the tool
//! version, server info, mapping hash, one per-table plan plus the bucket
//! summaries, needs_review and counts/control totals, and a SHA-256 over the
//! canonical content.
//!
//! Parsing is **untrusted input**: unknown fields are rejected at every level,
//! the file is size-limited, the format version is checked, the SHA-256 is
//! verified before anything else, and the mapping hash plus bucket-count and
//! control-total-count equalities are checked again at compare time.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::io::Read;
use thiserror::Error;

use crate::core::fingerprint::BucketSummary;
use crate::core::fingerprint::Decimal;
use crate::results::CheckLevel;

/// Current fingerprint-file format version.
pub const FINGERPRINT_VERSION: u32 = 1;

/// Maximum accepted fingerprint file size (uncompressed), in bytes.
pub const MAX_FINGERPRINT_BYTES: usize = 512 * 1024 * 1024;

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
    pub level: CheckLevel,
    pub keyed: bool,
    pub bucket_digits: u8,
    /// Column names in mapping order.
    pub columns: Vec<String>,
    pub key_columns: Vec<String>,
    /// The resolved column types (in `columns` order), captured so a saved file
    /// can be compared against a live database with the exact same plan.
    pub column_types: Vec<crate::core::normalise::PgType>,
    pub rows: u64,
    pub buckets: Vec<BucketSummary>,
    pub control_totals: Vec<Decimal>,
    pub needs_review: bool,
}

impl CapturedTable {
    /// The bucket count implied by `bucket_digits` (2^(4*digits)); the parse
    /// step checks every bucket index is below it.
    pub fn expected_bucket_count(&self) -> u64 {
        1u64 << (4 * self.bucket_digits as u32)
    }
}

/// The whole fingerprint file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct FingerprintFile {
    pub format_version: u32,
    pub tool_version: String,
    pub mapping_hash: String,
    pub server: CapturedServer,
    pub tables: Vec<CapturedTable>,
    /// SHA-256 over the canonical content (every field except this one), set by
    /// [`FingerprintFile::write`] and verified by [`FingerprintFile::read`].
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
}

impl<'a> From<&'a FingerprintFile> for Hashable<'a> {
    fn from(f: &'a FingerprintFile) -> Self {
        Hashable {
            format_version: f.format_version,
            tool_version: &f.tool_version,
            mapping_hash: &f.mapping_hash,
            server: &f.server,
            tables: &f.tables,
        }
    }
}

/// Canonical JSON (sorted keys, compact) of every field except `sha256`, then
/// SHA-256 of that byte string.
fn compute_sha256(f: &FingerprintFile) -> String {
    let canonical = serde_json::to_string(&Hashable::from(f)).unwrap_or_default();
    hex::encode(Sha256::digest(canonical.as_bytes()))
}

impl FingerprintFile {
    /// Verify the stored hash against the content, then re-verify field-level
    /// invariants needed before any comparison: every table has buckets within
    /// range, and the table count is consistent.
    pub fn verify(&self) -> Result<(), FingerprintFileError> {
        let computed = compute_sha256(self);
        if computed != self.sha256 {
            return Err(FingerprintFileError::HashMismatch {
                expected: self.sha256.clone(),
                actual: computed,
            });
        }
        if self.format_version != FINGERPRINT_VERSION {
            return Err(FingerprintFileError::UnsupportedVersion(
                self.format_version,
            ));
        }
        for t in &self.tables {
            let limit = t.expected_bucket_count();
            for b in &t.buckets {
                if u64::from(b.bucket) >= limit {
                    return Err(FingerprintFileError::MissingSummary(format!(
                        "bucket {} out of range for {} digits",
                        b.bucket, t.bucket_digits
                    )));
                }
            }
        }
        Ok(())
    }

    /// Serialise to canonical JSON with a fresh hash.
    pub fn to_json(mut self) -> Result<Vec<u8>, FingerprintFileError> {
        self.sha256 = compute_sha256(&self);
        serde_json::to_vec(&self).map_err(|e| FingerprintFileError::Json(e.to_string()))
    }

    /// Write to `path`, gzip-compressing when the name ends in `.gz`.
    pub fn write_to(&self, path: &std::path::Path) -> Result<(), FingerprintFileError> {
        let bytes =
            serde_json::to_vec(self).map_err(|e| FingerprintFileError::Json(e.to_string()))?;
        if path.extension().and_then(|e| e.to_str()) == Some("gz") {
            let mut enc = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
            std::io::Write::write_all(&mut enc, &bytes)
                .map_err(|e| FingerprintFileError::Io(e.to_string()))?;
            let compressed = enc
                .finish()
                .map_err(|e| FingerprintFileError::Io(e.to_string()))?;
            std::fs::write(path, compressed).map_err(|e| FingerprintFileError::Io(e.to_string()))
        } else {
            std::fs::write(path, &bytes).map_err(|e| FingerprintFileError::Io(e.to_string()))
        }
    }

    /// Load from `path` (or raw bytes), rejecting untrusted input.  The hash is
    /// verified against the requester.  Gzip is transparently detected by the
    /// magic bytes, not the extension.
    pub fn from_bytes(bytes: &[u8]) -> Result<FingerprintFile, FingerprintFileError> {
        let bytes = if bytes.starts_with(&[0x1f, 0x8b]) {
            let dec = flate2::read::GzDecoder::new(bytes);
            let mut out = Vec::new();
            // Bound the decompressed size too: refuse to inflate beyond the limit.
            dec.take((MAX_FINGERPRINT_BYTES as u64) + 1)
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
        reject_unknown_fields(&raw)?;

        let file: FingerprintFile = serde_json::from_slice(&bytes)
            .map_err(|e| FingerprintFileError::Json(e.to_string()))?;

        if file.format_version != FINGERPRINT_VERSION {
            return Err(FingerprintFileError::UnsupportedVersion(
                file.format_version,
            ));
        }
        file.verify()?;
        Ok(file)
    }

    /// Load from a path.
    pub fn read_from(path: &std::path::Path) -> Result<FingerprintFile, FingerprintFileError> {
        let bytes = std::fs::read(path).map_err(|e| FingerprintFileError::Io(e.to_string()))?;
        Self::from_bytes(&bytes)
    }
}

/// Reject unknown fields at the top level, so a hostile or merely newer file
/// cannot carry fields we would silently ignore.  Nested structs are guarded by
/// their own `deny_unknown_fields` attributes.
fn reject_unknown_fields(value: &serde_json::Value) -> Result<(), FingerprintFileError> {
    let serde_json::Value::Object(map) = value else {
        return Err(FingerprintFileError::Json(
            "fingerprint file must be a JSON object".to_string(),
        ));
    };
    const KNOWN: &[&str] = &[
        "format_version",
        "tool_version",
        "mapping_hash",
        "server",
        "tables",
        "sha256",
    ];
    let unknown: Vec<String> = map
        .keys()
        .filter(|k| !KNOWN.contains(&k.as_str()))
        .cloned()
        .collect();
    if !unknown.is_empty() {
        return Err(FingerprintFileError::UnknownFields(unknown));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::fingerprint::{BucketSummary, Decimal};

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
            tables: vec![CapturedTable {
                old_qualified: "demo_bank.accounts".into(),
                level: CheckLevel::FullValues,
                keyed: true,
                bucket_digits: 3,
                columns: vec!["account_id".into()],
                key_columns: vec!["account_id".into()],
                column_types: vec![crate::core::normalise::PgType::Int8],
                rows: 1,
                buckets: vec![BucketSummary {
                    bucket: 0,
                    n: 1,
                    s: [1, 2, 3, 4],
                }],
                control_totals: vec![Decimal::parse("1.0").unwrap()],
                needs_review: false,
            }],
            sha256: String::new(),
        }
    }

    #[test]
    fn round_trips_and_verifies() {
        let f = sample();
        let json = f.to_json().unwrap();
        let back = FingerprintFile::from_bytes(&json).unwrap();
        assert_eq!(back, {
            let mut s = sample();
            s.sha256 = back.sha256.clone();
            s
        });
    }

    #[test]
    fn rejects_tampering() {
        let f = sample();
        let mut json = f.to_json().unwrap();
        // Flip a byte in the middle of the JSON body.
        let mid = json.len() / 2;
        json[mid] ^= 0xff;
        match FingerprintFile::from_bytes(&json) {
            Err(FingerprintFileError::HashMismatch { .. }) | Err(FingerprintFileError::Json(_)) => {
            }
            other => panic!("expected hash mismatch or json error, got {other:?}"),
        }
    }

    #[test]
    fn rejects_unknown_field() {
        let f = sample();
        let json = f.to_json().unwrap();
        let mut val: serde_json::Value = serde_json::from_slice(&json).unwrap();
        val.as_object_mut()
            .unwrap()
            .insert("extra".into(), serde_json::json!(1));
        let bytes = serde_json::to_vec(&val).unwrap();
        assert!(matches!(
            FingerprintFile::from_bytes(&bytes),
            Err(FingerprintFileError::UnknownFields(_))
        ));
    }

    #[test]
    fn rejects_out_of_range_bucket() {
        let mut f = sample();
        f.tables[0].bucket_digits = 1; // 16 buckets
        f.tables[0].buckets[0].bucket = 16; // out of range
        let json = f.to_json().unwrap();
        assert!(FingerprintFile::from_bytes(&json).is_err());
    }
}
