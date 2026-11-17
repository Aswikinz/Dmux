//! Verify a results file before it is rendered (PLAN.md 2.4).
//!
//! The engine stamps [`RunResults::results_sha256`] — a SHA-256 of the canonical
//! results JSON — at the end of a run.  Before rendering a report, `dmux report`
//! re-reads the file and recomputes that digest over the parsed content.  If the
//! value in the file does not match, the file was edited after it was written and
//! the report refuses to render it.  This is the tamper-evident guarantee: it is
//! not tamper-proof, but it makes an unnoticed edit detectable.
//!
//! Results files come in two shapes:
//!
//! - a plain [`RunResults`] JSON (from `dmux compare` / `dmux run`); verify the
//!   `results_sha256` field against the canonical form;
//! - an envelope `{ results, hooks, envelope_sha256 }` (written by
//!   `dmux run --hooks`, T027): verify `envelope_sha256`, the SHA-256 of the
//!   envelope with its own `envelope_sha256` key removed.  This lets the report
//!   render a run that carried custom hook findings without trusting the
//!   envelope blindly.

use std::path::Path;

use dmux_engine::results::{sha256_of, RunResults};

/// A results file that passed verification, with its digest and the raw JSON.
#[derive(Debug)]
pub struct Verification {
    pub results: RunResults,
    /// The digest the plan calls "the SHA-256 of the results file" — for a plain
    /// results file this is `results_sha256`; for an envelope it is the
    /// envelope's own digest.
    pub sha256: String,
    /// The raw parsed JSON of the results object, retained so report modules can
    /// read results-schema fields that a given engine release may not expose on
    /// `RunResults` (structure findings, suggestions, drill-down detail).  Its
    /// integrity is already covered by whichever digest was verified.
    pub raw: serde_json::Value,
}

/// Why verification failed.
#[derive(Debug, thiserror::Error)]
pub enum VerifyError {
    #[error("cannot read results file: {0}")]
    Io(#[from] std::io::Error),
    #[error("results file is not valid JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error(
        "results file failed its integrity check: the recorded SHA-256 \
         ({recorded}) does not match the recomputed digest ({recomputed}); the \
         file has been edited after it was written and must not be reported on"
    )]
    Tampered {
        recorded: String,
        recomputed: String,
    },
    #[error("results file carries no SHA-256 (it was not finalised)")]
    MissingDigest,
    #[error("envelope is missing its nested results object")]
    EnvelopeMissingResults,
    #[error("envelope is missing its envelope_sha256")]
    EnvelopeMissingDigest,
}

/// Read and verify a results file (either shape).
pub fn verify_results_file(path: &Path) -> Result<Verification, VerifyError> {
    let bytes = std::fs::read(path)?;
    let value: serde_json::Value = serde_json::from_slice(&bytes)?;
    verify_value(&value)
}

/// Verify a parsed JSON value that is either a plain results object or an
/// envelope (`run --hooks` writes the envelope; `compare`/`run` write plain).
pub fn verify_value(value: &serde_json::Value) -> Result<Verification, VerifyError> {
    let envelope = is_envelope(value);
    let raw_results: serde_json::Value = if envelope {
        value
            .get("results")
            .cloned()
            .ok_or(VerifyError::EnvelopeMissingResults)?
    } else {
        value.clone()
    };
    let results: RunResults = serde_json::from_value(raw_results.clone())?;

    let (recorded, recomputed) = if envelope {
        // Remove the envelope's own signature key, serialise the rest, hash it.
        let mut closed = value.as_object().cloned().unwrap_or_default();
        closed.remove("envelope_sha256");
        let canonical =
            serde_json::to_string(&serde_json::Value::Object(closed)).unwrap_or_default();
        let recomputed = sha256_hex(canonical.as_bytes());
        let recorded = value
            .get("envelope_sha256")
            .and_then(|v| v.as_str())
            .ok_or(VerifyError::EnvelopeMissingDigest)?
            .to_string();
        (recorded, recomputed)
    } else {
        // The engine's self-verifying digest.
        let recomputed = sha256_of(&results);
        let recorded = results.results_sha256.clone();
        (recorded, recomputed)
    };

    if recorded.is_empty() {
        return Err(VerifyError::MissingDigest);
    }
    if recorded != recomputed {
        return Err(VerifyError::Tampered {
            recorded,
            recomputed,
        });
    }

    Ok(Verification {
        results,
        sha256: recomputed,
        raw: raw_results,
    })
}

fn is_envelope(value: &serde_json::Value) -> bool {
    value
        .as_object()
        .is_some_and(|o| o.contains_key("results") && o.contains_key("envelope_sha256"))
}

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::Digest;
    hex::encode(sha2::Sha256::digest(bytes))
}

/// Verify an already-parsed [`RunResults`] payload (plain shape only).
pub fn verify_results(results: &RunResults) -> Result<Verification, VerifyError> {
    let value = serde_json::to_value(results)?;
    verify_value(&value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use dmux_engine::results::{finalise, RunResults, Verdict, RESULTS_VERSION};
    use std::collections::BTreeMap;

    fn results() -> RunResults {
        RunResults {
            results_version: RESULTS_VERSION,
            run_id: "run-1".into(),
            tool_version: "0.1.0".into(),
            mapping_hash: "mh".into(),
            verdict: Verdict::Pass,
            coverage_line: "".into(),
            fully_checked: 0,
            counts_only: 0,
            ignored: 0,
            tables: BTreeMap::new(),
            ignored_items: vec![],
            changed_during_check: vec![],
            warnings: vec![],
            wal_start_lsn: None,
            wal_end_lsn: None,
            results_sha256: String::new(),
        }
    }

    #[test]
    fn a_finalised_file_verifies() {
        let v = verify_results(&finalise(results())).unwrap();
        assert_eq!(v.sha256, v.results.results_sha256);
        assert!(!v.sha256.is_empty());
    }

    #[test]
    fn an_edited_file_is_refused() {
        let mut r = finalise(results());
        r.verdict = Verdict::Fail; // a tampered edit, digest untouched
        match verify_results(&r).unwrap_err() {
            VerifyError::Tampered { .. } => {}
            other => panic!("expected Tampered, got {other}"),
        }
    }

    #[test]
    fn an_unfinalised_file_is_refused() {
        assert!(matches!(
            verify_results(&results()),
            Err(VerifyError::MissingDigest)
        ));
    }

    /// Build a T027-style envelope: the full results object + hooks, then stamp
    /// `envelope_sha256` over a copy with that key removed.
    fn envelope(results: &RunResults) -> serde_json::Value {
        let mut env = serde_json::Map::new();
        env.insert("results".into(), serde_json::to_value(results).unwrap());
        env.insert("hooks".into(), serde_json::json!([]));
        let digest = sha256_hex(
            serde_json::to_string(&serde_json::Value::Object(env.clone()))
                .unwrap()
                .as_bytes(),
        );
        env.insert("envelope_sha256".into(), serde_json::json!(digest));
        serde_json::Value::Object(env)
    }

    #[test]
    fn an_envelope_verifies_and_exposes_the_hash() {
        let env = envelope(&finalise(results()));
        let v = verify_value(&env).unwrap();
        assert!(!v.sha256.is_empty());
        assert_eq!(v.results.run_id, "run-1");
    }

    #[test]
    fn a_tampered_envelope_is_refused() {
        let mut env = envelope(&finalise(results()));
        env["results"]["verdict"] = serde_json::json!("FAIL");
        assert!(matches!(
            verify_value(&env),
            Err(VerifyError::Tampered { .. })
        ));
    }

    #[test]
    fn a_results_file_round_trips_through_disk() {
        let finalised = finalise(results());
        let dir = std::env::temp_dir().join(format!("dmux-verify-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("results.json");
        std::fs::write(&path, serde_json::to_vec_pretty(&finalised).unwrap()).unwrap();

        let v = verify_results_file(&path).unwrap();
        assert_eq!(v.results.run_id, "run-1");
        assert_eq!(v.sha256, finalised.results_sha256);
    }
}
