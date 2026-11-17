//! Verify a results file before it is rendered (PLAN.md 2.4, 2.3).
//!
//! The engine stamps [`RunResults::results_sha256`] — a SHA-256 of the canonical
//! results JSON — at the end of a run.  Before rendering a report, `dmux report`
//! re-reads the file and recomputes that digest **with the engine's own
//! [`sha256_of`]** over the *typed* v2 [`RunResults`], never a re-implementation
//! and never a raw-JSON hash.  Any field that survives that check (structure
//! findings, drill-down detail, definition hash, in-scope/structure-only tallies)
//! is exactly the value that was signed, so a forged finding is refused, not
//! rendered.
//!
//! Results files come in two shapes:
//!
//! - a plain [`RunResults`] JSON (from `dmux compare` / `dmux run`); verify the
//!   `results_sha256` field against the canonical form;
//! - an envelope `{ results, hooks: { custom_checks, scripts, warnings },
//!   envelope_sha256 }` (written by `dmux run --hooks`, T027): the envelope's
//!   own digest is checked by [`dmux_hooks::verify_envelope`], and the nested
//!   results must ALSO pass [`sha256_of`], so a forged finding or a forged
//!   results field is caught.

use std::path::Path;

use dmux_engine::results::{sha256_of, RunResults, Verdict};

/// One hook finding carried in an envelope's `hooks.custom_checks` (T027).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HookFinding {
    pub id: String,
    pub severity: String,
    pub message: String,
    pub table: Option<String>,
}

/// One hook script's recorded name + SHA-256 (the code that actually ran).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HookScript {
    pub script: String,
    pub sha256: String,
}

/// Hook evidence recovered from a verified envelope.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HookEvidence {
    pub custom_checks: Vec<HookFinding>,
    pub scripts: Vec<HookScript>,
    pub warnings: Vec<String>,
}

/// A results file that passed verification, with its digest, the parsed typed
/// results, and (for an envelope) the hook evidence.
#[derive(Debug)]
pub struct Verification {
    pub results: RunResults,
    /// The digest printed on the report's first page: `results_sha256` for a
    /// plain file, or the envelope digest for an envelope.
    pub sha256: String,
    /// Hook evidence, present only when the file was an envelope.
    pub hooks: Option<HookEvidence>,
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
    #[error("envelope integrity check failed: {0}")]
    Envelope(String),
    #[error(
        "results file is internally inconsistent: the overall verdict is PASS \
         but a table or structure finding is not PASS; refusing to render"
    )]
    InconsistentVerdict,
}

/// Read and verify a results file (either shape).
pub fn verify_results_file(path: &Path) -> Result<Verification, VerifyError> {
    let bytes = std::fs::read(path)?;
    let value: serde_json::Value = serde_json::from_slice(&bytes)?;
    verify_value(&value)
}

/// Verify a parsed JSON value that is either a plain results object or a
/// T027 hook envelope.
pub fn verify_value(value: &serde_json::Value) -> Result<Verification, VerifyError> {
    if is_envelope(value) {
        verify_envelope(value)
    } else {
        verify_plain(value)
    }
}

/// Verify a plain (non-envelope) results file.
fn verify_plain(value: &serde_json::Value) -> Result<Verification, VerifyError> {
    // Deserialize into the typed v2 RunResults first: a forged field that does
    // not fit the schema fails now, and the digest is computed over the typed
    // value that will actually be rendered.
    let results: RunResults = serde_json::from_value(value.clone())?;
    let recomputed = sha256_of(&results);
    let recorded = results.results_sha256.clone();
    check_digest(&recorded, &recomputed)?;
    check_verdict_consistency(&results)?;
    Ok(Verification {
        results,
        sha256: recomputed,
        hooks: None,
    })
}

/// Verify a T027 hook envelope: `{ results, hooks, envelope_sha256 }`.
///
/// Both layers are checked — the envelope digest (binding the hook evidence to
/// the results) and the inner results' own [`sha256_of`] (binding the results
/// values) — so neither a forged finding nor a forged results field can reach
/// the renderer.
fn verify_envelope(value: &serde_json::Value) -> Result<Verification, VerifyError> {
    verify_envelope_bytes(value).map_err(VerifyError::Envelope)?;

    let results_value = value
        .get("results")
        .cloned()
        .ok_or(VerifyError::EnvelopeMissingResults)?;
    let results: RunResults = serde_json::from_value(results_value)?;
    let results_recomputed = sha256_of(&results);
    check_digest(&results.results_sha256, &results_recomputed)?;
    check_verdict_consistency(&results)?;

    let hooks = parse_hook_evidence(value.get("hooks"));
    let sha256 = value
        .get("envelope_sha256")
        .and_then(|v| v.as_str())
        .ok_or(VerifyError::EnvelopeMissingDigest)?
        .to_string();

    Ok(Verification {
        results,
        sha256,
        hooks: Some(hooks),
    })
}

/// Verify an envelope's own `envelope_sha256`, delegating to the single source of
/// truth in `dmux-hooks` so the report and the writer can never drift apart.
pub fn verify_envelope_bytes(value: &serde_json::Value) -> Result<(), String> {
    dmux_hooks::verify_envelope(value)
}

fn check_digest(recorded: &str, recomputed: &str) -> Result<(), VerifyError> {
    if recorded.is_empty() {
        return Err(VerifyError::MissingDigest);
    }
    if recorded != recomputed {
        return Err(VerifyError::Tampered {
            recorded: recorded.to_string(),
            recomputed: recomputed.to_string(),
        });
    }
    Ok(())
}

/// Fail-closed guard: the engine never writes PASS alongside a non-PASS table or
/// structure finding, so such a combination means the file was tampered (or a
/// writer bug).  Refuse to render rather than sign a PASS that hides a failure.
fn check_verdict_consistency(results: &RunResults) -> Result<(), VerifyError> {
    if !results.verdict.is_pass() {
        return Ok(()); // only a PASS is capable of hiding a gap
    }
    let any_table_bad = results.tables.values().any(|t| !t.verdict.is_pass());
    let any_finding_bad = results
        .structure_findings
        .iter()
        .any(|f| !f.verdict.is_pass());
    if any_table_bad || any_finding_bad {
        return Err(VerifyError::InconsistentVerdict);
    }
    Ok(())
}

/// The displayed overall verdict of an envelope:
/// `worse(inner results verdict, every hook finding folded in)`.
///
/// A `fail` finding forces the run off PASS; an `info` or `warning` finding
/// contributes `Warning` (never PASS, never hides a failure).  This matters
/// because `dmux hook` writes findings *without* mutating the nested results
/// verdict (T027), so the report must fold hooks in itself.
pub fn displayed_verdict(results: &RunResults, hooks: &HookEvidence) -> Verdict {
    let mut worst = results.verdict;
    for f in &hooks.custom_checks {
        let v = match f.severity.as_str() {
            "fail" => Verdict::Fail,
            "warning" => Verdict::Warning,
            _ => Verdict::Warning, // "info" folds to Warning: still not PASS
        };
        worst = worst.worst(v);
    }
    worst
}

fn is_envelope(value: &serde_json::Value) -> bool {
    value
        .as_object()
        .is_some_and(|o| o.contains_key("results") && o.contains_key("envelope_sha256"))
}

fn parse_hook_evidence(hooks: Option<&serde_json::Value>) -> HookEvidence {
    let Some(hooks) = hooks.and_then(|h| h.as_object()) else {
        return HookEvidence::default();
    };
    let custom_checks = hooks
        .get("custom_checks")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|f| {
                    Some(HookFinding {
                        id: f.get("id")?.as_str()?.to_string(),
                        severity: f.get("severity")?.as_str()?.to_string(),
                        message: f.get("message")?.as_str()?.to_string(),
                        table: f.get("table").and_then(|t| t.as_str()).map(str::to_string),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    let scripts = hooks
        .get("scripts")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|s| {
                    Some(HookScript {
                        script: s.get("script")?.as_str()?.to_string(),
                        sha256: s.get("sha256")?.as_str()?.to_string(),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    let warnings = hooks
        .get("warnings")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|w| w.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    HookEvidence {
        custom_checks,
        scripts,
        warnings,
    }
}

/// Verify an already-parsed [`RunResults`] payload (plain shape only).
pub fn verify_results(results: &RunResults) -> Result<Verification, VerifyError> {
    let value = serde_json::to_value(results)?;
    verify_plain(&value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use dmux_engine::results::{
        finalise, CheckLevel::FullValues, RunResults, StructureFinding, TableResult, Verdict,
        RESULTS_VERSION,
    };
    use std::collections::BTreeMap;

    fn results() -> RunResults {
        RunResults {
            results_version: RESULTS_VERSION,
            run_id: "run-1".into(),
            tool_version: "0.1.0".into(),
            mapping_hash: "mh".into(),
            verdict: Verdict::Pass,
            coverage_line:
                "1 of 1 tables fully checked, 0 counts only, 0 structure only, 0 ignored".into(),
            fully_checked: 1,
            counts_only: 0,
            ignored: 0,
            tables: BTreeMap::from([(
                "s.t".into(),
                TableResult {
                    table: "s.t".into(),
                    level: FullValues,
                    verdict: Verdict::Pass,
                    old_rows: Some(1),
                    new_rows: Some(1),
                    columns_covered: Some(vec!["id".into()]),
                    differing_buckets: Some(0),
                    missing_buckets: Some(0),
                    extra_buckets: Some(0),
                    missing_rows: Some(0),
                    extra_rows: Some(0),
                    changed_rows: Some(0),
                    reasons: vec![],
                },
            )]),
            ignored_items: vec![],
            changed_during_check: vec![],
            warnings: vec![],
            ..RunResults::default()
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
        r.verdict = Verdict::Fail;
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

    #[test]
    fn a_forged_structure_finding_is_refused() {
        // A v2 file whose results_sha256 was computed WITHOUT the finding: the
        // forged finding must not survive verification.
        let finalised = finalise(results());
        let mut v = serde_json::to_value(finalised).unwrap();
        v["structure_findings"] = serde_json::json!([
            { "object": "s.t.idx", "path": ["s","t","idx"], "verdict": "FAIL", "message": "forged" }
        ]);
        let err = verify_value(&v).unwrap_err();
        assert!(matches!(err, VerifyError::Tampered { .. }), "got {err}");
    }

    #[test]
    fn a_legit_structure_finding_verifies_and_survives() {
        let mut r = results();
        r.structure_findings = vec![StructureFinding {
            object: "s.t.idx".into(),
            path: vec!["s".into(), "t".into(), "idx".into()],
            verdict: Verdict::Fail,
            message: "missing index".into(),
        }];
        r.verdict = Verdict::Fail;
        let finalised = finalise(r);
        let v = verify_results(&finalised).unwrap();
        assert_eq!(v.results.structure_findings.len(), 1);
        assert_eq!(v.results.structure_findings[0].message, "missing index");
    }

    #[test]
    fn a_pass_with_a_bad_table_is_refused() {
        // Fail-closed: PASS plus a non-PASS table cannot be signed off.
        let mut r = finalise(results());
        r.verdict = Verdict::Pass;
        r.tables.get_mut("s.t").unwrap().verdict = Verdict::Fail;
        let r = finalise(r);
        match verify_results(&r).unwrap_err() {
            VerifyError::InconsistentVerdict => {}
            other => panic!("expected InconsistentVerdict, got {other}"),
        }
    }

    /// Build a T027-style envelope, stamped exactly as the CLI does.
    fn envelope(results: &RunResults, checks: serde_json::Value) -> serde_json::Value {
        let mut env = serde_json::Map::new();
        env.insert("results".into(), serde_json::to_value(results).unwrap());
        env.insert(
            "hooks".into(),
            serde_json::json!({
                "custom_checks": checks,
                "scripts": [{"script": "hook.py", "sha256": "ab".repeat(32)}],
                "warnings": [],
            }),
        );
        // Stamp the digest with the same function the writer uses, so the test
        // exercises the real `dmux_hooks` algorithm end to end.
        let envelope_value = serde_json::Value::Object(env.clone());
        let digest = dmux_hooks::envelope_sha256(&envelope_value);
        env.insert("envelope_sha256".into(), serde_json::json!(digest));
        serde_json::Value::Object(env)
    }

    #[test]
    fn an_envelope_verifies_and_exposes_hook_evidence() {
        let env = envelope(
            &finalise(results()),
            serde_json::json!([
                { "id": "c1", "severity": "fail", "message": "bad thing", "source": "custom_checks" }
            ]),
        );
        let v = verify_value(&env).unwrap();
        let hooks = v.hooks.as_ref().unwrap();
        assert_eq!(hooks.custom_checks.len(), 1);
        assert_eq!(hooks.custom_checks[0].severity, "fail");
        assert_eq!(hooks.scripts.len(), 1);
        assert_eq!(hooks.scripts[0].sha256.len(), 64);
    }

    #[test]
    fn a_tampered_envelope_is_refused() {
        let mut env = envelope(&finalise(results()), serde_json::json!([]));
        env["hooks"]["custom_checks"] = serde_json::json!([
            { "id": "x", "severity": "fail", "message": "injected" }
        ]);
        assert!(matches!(
            verify_value(&env),
            Err(VerifyError::Envelope(_) | VerifyError::Tampered { .. })
        ));
    }

    #[test]
    fn displayed_verdict_folds_a_hook_fail_off_pass() {
        let r = results(); // veridct PASS
        let hooks = HookEvidence {
            custom_checks: vec![HookFinding {
                id: "c".into(),
                severity: "fail".into(),
                message: "m".into(),
                table: None,
            }],
            ..Default::default()
        };
        assert_eq!(displayed_verdict(&r, &hooks), Verdict::Fail);

        let hook_warning = HookEvidence {
            custom_checks: vec![HookFinding {
                id: "c".into(),
                severity: "warning".into(),
                message: "m".into(),
                table: None,
            }],
            ..Default::default()
        };
        assert_eq!(displayed_verdict(&r, &hook_warning), Verdict::Warning);
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
