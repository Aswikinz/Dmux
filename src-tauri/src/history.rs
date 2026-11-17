//! Run history, opened results and the run log (PLAN 1, 2.3, 4.7, 4.8).
//!
//! When a run finishes, its finalised results JSON is written to its own file
//! under `<app data>/history/` (mode 0600 on Unix, in a 0700 directory) and
//! its metadata goes into the SQLite store.  Results carry counts, hashes and
//! names only, never row values, but they are still a confidential audit
//! record, so they are never world-readable.
//!
//! Anything shown from a file (a history entry, or a file the user opens with
//! the native dialog) is **verified first**, exactly like `dmux report` does
//! ([`dmux_report::verify::verify_value`]): the SHA-256 is recomputed over the
//! typed results, a PASS that hides a failing table is refused, and a hook
//! envelope is checked too.  Only verified results are registered in app
//! state, under a fresh id; the webview never supplies results.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use dmux_engine::results::{RunResults, Verdict};
use dmux_report::verify::{HookEvidence, Verification};

use crate::error::CommandError;
use crate::run_manager::RunManager;
use crate::store::{HistoryRow, Store};

/// The largest results file the app will read (a results file holds counts and
/// names; anything bigger is not one).
pub const MAX_RESULTS_FILE_BYTES: u64 = 64 * 1024 * 1024;

/// How many opened results stay registered in memory at once.
const MAX_OPENED: usize = 8;

/// What the app remembers about a run while it is in progress.
#[derive(Debug, Clone)]
pub struct RunMeta {
    pub started_at: i64,
    pub old_label: String,
    pub new_label: String,
}

/// The shared table of in-progress run metadata.
pub type RunMetaTable = Arc<Mutex<HashMap<String, RunMeta>>>;

/// Verified results registered in app state (from history or an opened file).
#[derive(Debug, Clone)]
pub struct Opened {
    pub results: RunResults,
    pub hooks: Option<HookEvidence>,
    pub sha256: String,
}

impl Opened {
    pub fn from_verification(v: Verification) -> Self {
        Self {
            results: v.results,
            hooks: v.hooks,
            sha256: v.sha256,
        }
    }

    /// Rebuild the verified shape the report renderer takes.
    pub fn to_verification(&self) -> Verification {
        Verification {
            results: self.results.clone(),
            sha256: self.sha256.clone(),
            hooks: self.hooks.clone(),
        }
    }

    /// The overall verdict to show: for a hook envelope, the worse of the
    /// results verdict and every hook finding (never a PASS hiding a failed
    /// custom check).
    pub fn displayed_verdict(&self) -> Verdict {
        match &self.hooks {
            Some(h) => dmux_report::verify::displayed_verdict(&self.results, h),
            None => self.results.verdict,
        }
    }
}

/// A small bounded table of opened results, keyed by a fresh run id.
#[derive(Debug, Default)]
pub struct OpenedResults {
    entries: Vec<(String, Opened)>,
}

impl OpenedResults {
    pub fn insert(&mut self, id: String, opened: Opened) {
        self.entries.retain(|(k, _)| k != &id);
        self.entries.push((id, opened));
        while self.entries.len() > MAX_OPENED {
            self.entries.remove(0);
        }
    }

    pub fn get(&self, id: &str) -> Option<Opened> {
        self.entries
            .iter()
            .find(|(k, _)| k == id)
            .map(|(_, o)| o.clone())
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.entries.len()
    }
}

/// The results-file directory under the app data dir.
pub fn history_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("history")
}

/// An id that can safely name a file (alphanumerics and hyphens only).
pub fn is_safe_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 128 && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}

/// Unix seconds now.
pub fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

/// The history file of a run.
pub fn results_path(dir: &Path, id: &str) -> Result<PathBuf, CommandError> {
    if !is_safe_id(id) {
        return Err(CommandError::InvalidInput {
            message: "that is not a valid history entry".to_string(),
        });
    }
    Ok(dir.join(format!("{id}.json")))
}

fn store_err(what: &str, e: &std::io::Error) -> CommandError {
    CommandError::Store {
        message: format!("{what}: {e}"),
    }
}

/// Create the history directory (mode 0700 on Unix).
fn ensure_dir(dir: &Path) -> Result<(), CommandError> {
    std::fs::create_dir_all(dir).map_err(|e| store_err("cannot create the history folder", &e))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700));
    }
    Ok(())
}

/// Write `bytes` to `path` as a new private file (0600 on Unix): a temporary
/// file in the same folder, then a rename, so a crash never leaves a half
/// results file under the final name.
pub fn write_private_file(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    let _ = std::fs::remove_file(&tmp);
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let result = (|| {
        let mut file = opts.open(&tmp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        std::fs::rename(&tmp, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

/// Persist one finished run: results file first, then the metadata row, so a
/// row never points at a missing file.
pub fn persist_finished(
    store: &Mutex<Store>,
    dir: &Path,
    meta: Option<&RunMeta>,
    run_id: &str,
    results: &RunResults,
) -> Result<(), CommandError> {
    let path = results_path(dir, run_id)?;
    ensure_dir(dir)?;
    let bytes = serde_json::to_vec_pretty(results).map_err(|e| CommandError::Store {
        message: format!("cannot serialise the results: {e}"),
    })?;
    write_private_file(&path, &bytes).map_err(|e| store_err("cannot save the results", &e))?;
    let now = now_secs();
    let row = HistoryRow {
        id: run_id.to_string(),
        started_at: meta.map_or(now, |m| m.started_at),
        ended_at: now,
        verdict: verdict_word(results.verdict).to_string(),
        options_applied: results.options_applied,
        options_recorded: dmux_engine::results::options_recorded(results),
        coverage_line: results.coverage_line.clone(),
        old_label: meta.map(|m| m.old_label.clone()).unwrap_or_default(),
        new_label: meta.map(|m| m.new_label.clone()).unwrap_or_default(),
        mapping_hash: results.mapping_hash.clone(),
        definition_hash: results.definition_hash.clone(),
        results_sha256: results.results_sha256.clone(),
    };
    let guard = store
        .lock()
        .map_err(|_| CommandError::internal("store lock poisoned"))?;
    if let Err(e) = guard.history_insert(&row) {
        let _ = std::fs::remove_file(&path);
        return Err(e);
    }
    Ok(())
}

/// The contract's verdict strings (verbatim vocabulary).
pub fn verdict_word(v: Verdict) -> &'static str {
    match v {
        Verdict::Pass => "PASS",
        Verdict::Fail => "FAIL",
        Verdict::WidelyDifferent => "WIDELY_DIFFERENT",
        Verdict::NeedsReview => "NEEDS_REVIEW",
        Verdict::ChangedDuringCheck => "CHANGED_DURING_CHECK",
        Verdict::ToolFault => "TOOL_FAULT",
        Verdict::Warning => "WARNING",
    }
}

/// Read a results file (size-limited) and verify it like `dmux report` does.
/// Every refusal is a plain message that names no row data and, for a
/// tampered file, no digest.
pub fn read_verified(path: &Path) -> Result<Verification, CommandError> {
    let refuse = |message: &str| CommandError::Report {
        message: message.to_string(),
    };
    let meta =
        std::fs::metadata(path).map_err(|_| refuse("that results file could not be opened"))?;
    if !meta.is_file() {
        return Err(refuse("that is not a results file"));
    }
    if meta.len() > MAX_RESULTS_FILE_BYTES {
        return Err(refuse(
            "that file is too large to be a Dmux results file (the limit is 64 MiB)",
        ));
    }
    let bytes = {
        use std::io::Read;
        let file = std::fs::File::open(path)
            .map_err(|_| refuse("that results file could not be opened"))?;
        // Bounded read: the file may grow after the size check.
        let mut buf = Vec::new();
        file.take(MAX_RESULTS_FILE_BYTES + 1)
            .read_to_end(&mut buf)
            .map_err(|_| refuse("that results file could not be read"))?;
        buf
    };
    if bytes.len() as u64 > MAX_RESULTS_FILE_BYTES {
        return Err(refuse(
            "that file is too large to be a Dmux results file (the limit is 64 MiB)",
        ));
    }
    verify_bytes(&bytes)
}

/// Verify results bytes (plain results or a hook envelope).
pub fn verify_bytes(bytes: &[u8]) -> Result<Verification, CommandError> {
    use dmux_report::verify::VerifyError;
    let value: serde_json::Value =
        serde_json::from_slice(bytes).map_err(|_| CommandError::Report {
            message: "that file is not a Dmux results file".to_string(),
        })?;
    dmux_report::verify::verify_value(&value).map_err(|e| {
        let message = match e {
            VerifyError::Tampered { .. } => {
                "this results file has been changed since Dmux wrote it, so it cannot be \
                 trusted and is not shown"
            }
            VerifyError::MissingDigest => {
                "this results file was never finalised (it has no integrity digest), so it is \
                 not shown"
            }
            VerifyError::InconsistentVerdict => {
                "this results file says PASS while a table or structure finding does not, so \
                 it cannot be trusted and is not shown"
            }
            VerifyError::Envelope(_)
            | VerifyError::EnvelopeMissingResults
            | VerifyError::EnvelopeMissingDigest => {
                "this results file's custom-check envelope failed its integrity check, so it \
                 is not shown"
            }
            VerifyError::Json(_) | VerifyError::Io(_) => "that file is not a Dmux results file",
        };
        CommandError::Report {
            message: message.to_string(),
        }
    })
}

/// Delete every stored results file and every history row.  Profiles are kept.
/// Returns the number of history rows removed.  Files go first, and any
/// results file in the folder is removed (including a half-written leftover),
/// so no row can survive its file and no file survives its row.
pub fn purge(store: &Mutex<Store>, dir: &Path) -> Result<usize, CommandError> {
    if dir.is_dir() {
        let entries =
            std::fs::read_dir(dir).map_err(|e| store_err("cannot read the history folder", &e))?;
        for entry in entries {
            let entry = entry.map_err(|e| store_err("cannot read the history folder", &e))?;
            let path = entry.path();
            if path.is_file() {
                std::fs::remove_file(&path)
                    .map_err(|e| store_err("cannot delete a stored results file", &e))?;
            }
        }
    }
    store
        .lock()
        .map_err(|_| CommandError::internal("store lock poisoned"))?
        .history_clear()
}

/// A fresh id for results registered from a file or from history.
pub fn fresh_opened_id() -> String {
    RunManager::fresh_id()
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use dmux_engine::results::{finalise, CheckLevel, RunResults, TableResult, RESULTS_VERSION};
    use std::collections::BTreeMap;

    pub(crate) fn sample_results(verdict: Verdict) -> RunResults {
        let table = TableResult {
            table: "demo_bank.accounts".into(),
            level: CheckLevel::FullValues,
            verdict,
            old_rows: Some(3),
            new_rows: Some(3),
            columns_covered: Some(vec!["id".into(), "balance".into()]),
            differing_buckets: Some(0),
            missing_buckets: Some(0),
            extra_buckets: Some(0),
            missing_rows: Some(0),
            extra_rows: Some(0),
            changed_rows: Some(0),
            reasons: vec![],
            column_notes: vec![],
        };
        finalise(RunResults {
            results_version: RESULTS_VERSION,
            run_id: "run-1".into(),
            tool_version: "0.3.0".into(),
            mapping_hash: "mh".into(),
            definition_hash: "dh".into(),
            verdict,
            coverage_line:
                "1 of 1 tables fully checked, 0 counts only, 0 structure only, 0 ignored".into(),
            fully_checked: 1,
            tables: BTreeMap::from([("demo_bank.accounts".into(), table)]),
            ..RunResults::default()
        })
    }

    fn store(dir: &Path) -> Mutex<Store> {
        Mutex::new(Store::open(&dir.join("dmux.db")).unwrap())
    }

    fn meta() -> RunMeta {
        RunMeta {
            started_at: 1_000,
            old_label: "Old database".into(),
            new_label: "New database".into(),
        }
    }

    #[test]
    fn persists_metadata_and_a_results_file_that_verifies() {
        let dir = tempfile::tempdir().unwrap();
        let st = store(dir.path());
        let hist = history_dir(dir.path());
        let results = sample_results(Verdict::Pass);
        persist_finished(&st, &hist, Some(&meta()), "run-a-b-c", &results).unwrap();

        let rows = st.lock().unwrap().history_list().unwrap();
        assert_eq!(rows.len(), 1);
        let row = &rows[0];
        assert_eq!(row.id, "run-a-b-c");
        assert_eq!(row.verdict, "PASS");
        assert_eq!(row.started_at, 1_000);
        assert!(row.ended_at >= row.started_at);
        assert_eq!(row.old_label, "Old database");
        assert_eq!(row.mapping_hash, "mh");
        assert_eq!(row.definition_hash, "dh");
        assert_eq!(row.results_sha256, results.results_sha256);
        assert!(row.coverage_line.starts_with("1 of 1 tables"));

        let v = read_verified(&results_path(&hist, "run-a-b-c").unwrap()).unwrap();
        assert_eq!(v.sha256, results.results_sha256);
        assert_eq!(v.results.verdict, Verdict::Pass);
    }

    #[cfg(unix)]
    #[test]
    fn results_files_are_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let st = store(dir.path());
        let hist = history_dir(dir.path());
        persist_finished(&st, &hist, None, "run-x", &sample_results(Verdict::Fail)).unwrap();
        let file_mode = std::fs::metadata(hist.join("run-x.json"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(file_mode, 0o600);
        let dir_mode = std::fs::metadata(&hist).unwrap().permissions().mode() & 0o777;
        assert_eq!(dir_mode, 0o700);
        // A rewrite keeps the mode too, and leaves no temporary file behind.
        persist_finished(&st, &hist, None, "run-x", &sample_results(Verdict::Fail)).unwrap();
        let names: Vec<_> = std::fs::read_dir(&hist)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
            .collect();
        assert_eq!(names, ["run-x.json"]);
    }

    #[test]
    fn the_results_file_holds_no_row_values_only_summary_fields() {
        let dir = tempfile::tempdir().unwrap();
        let st = store(dir.path());
        let hist = history_dir(dir.path());
        persist_finished(&st, &hist, None, "run-y", &sample_results(Verdict::Pass)).unwrap();
        let text = std::fs::read_to_string(hist.join("run-y.json")).unwrap();
        // Names and counts are there; there is no password or connection field.
        assert!(text.contains("demo_bank.accounts"));
        for forbidden in ["password", "host", "postgres://"] {
            assert!(!text.contains(forbidden), "{forbidden}");
        }
    }

    #[test]
    fn a_tampered_file_is_refused_with_a_plain_message() {
        let dir = tempfile::tempdir().unwrap();
        let st = store(dir.path());
        let hist = history_dir(dir.path());
        persist_finished(&st, &hist, None, "run-t", &sample_results(Verdict::Fail)).unwrap();
        let path = hist.join("run-t.json");
        let text = std::fs::read_to_string(&path).unwrap();
        // Flip the overall verdict: the digest no longer matches.
        let edited = text.replacen("\"verdict\": \"fail\"", "\"verdict\": \"pass\"", 1);
        assert_ne!(text, edited, "the fixture must contain the overall verdict");
        std::fs::write(&path, edited).unwrap();
        let err = read_verified(&path).unwrap_err();
        let CommandError::Report { message } = err else {
            panic!("expected a Report error");
        };
        assert!(message.contains("changed since Dmux wrote it"), "{message}");
        assert!(!message.contains(&sample_results(Verdict::Fail).results_sha256));
    }

    #[test]
    fn an_unfinalised_or_inconsistent_pass_is_refused() {
        // No digest at all.
        let mut raw = sample_results(Verdict::Pass);
        raw.results_sha256 = String::new();
        let bytes = serde_json::to_vec(&raw).unwrap();
        let CommandError::Report { message } = verify_bytes(&bytes).unwrap_err() else {
            panic!()
        };
        assert!(message.contains("never finalised"), "{message}");

        // An honest digest over a PASS that hides a failing table.
        let mut lying = sample_results(Verdict::Fail);
        lying.verdict = Verdict::Pass;
        let lying = finalise(lying);
        let bytes = serde_json::to_vec(&lying).unwrap();
        let CommandError::Report { message } = verify_bytes(&bytes).unwrap_err() else {
            panic!()
        };
        assert!(message.contains("says PASS"), "{message}");
    }

    #[test]
    fn something_that_is_not_results_is_refused() {
        for bytes in [&b"not json"[..], b"[1,2,3]", b"{\"hello\":1}"] {
            assert!(verify_bytes(bytes).is_err());
        }
    }

    #[test]
    fn an_oversized_file_is_refused_without_being_read() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("big.json");
        let file = std::fs::File::create(&path).unwrap();
        file.set_len(MAX_RESULTS_FILE_BYTES + 1).unwrap(); // sparse
        let CommandError::Report { message } = read_verified(&path).unwrap_err() else {
            panic!()
        };
        assert!(message.contains("too large"), "{message}");
        // A missing file and a folder are plain refusals as well.
        assert!(read_verified(&dir.path().join("nope.json")).is_err());
        assert!(read_verified(dir.path()).is_err());
    }

    #[test]
    fn an_envelope_with_a_failed_custom_check_never_displays_pass() {
        let results = sample_results(Verdict::Pass);
        let opened = Opened {
            results: results.clone(),
            hooks: Some(HookEvidence {
                custom_checks: vec![dmux_report::verify::HookFinding {
                    id: "c1".into(),
                    severity: "fail".into(),
                    message: "a custom check failed".into(),
                    table: None,
                }],
                ..HookEvidence::default()
            }),
            sha256: "e".into(),
        };
        assert_eq!(opened.displayed_verdict(), Verdict::Fail);
        let plain = Opened {
            hooks: None,
            ..opened
        };
        assert_eq!(plain.displayed_verdict(), Verdict::Pass);
    }

    #[test]
    fn purge_removes_every_results_file_and_row_but_keeps_profiles() {
        let dir = tempfile::tempdir().unwrap();
        let st = store(dir.path());
        let hist = history_dir(dir.path());
        st.lock()
            .unwrap()
            .save(&crate::store::StoredProfile {
                id: "keep".into(),
                label: "Kept".into(),
                host: "h".into(),
                port: 5432,
                database: "d".into(),
                user: "u".into(),
                encryption: "verify".into(),
                tls_fingerprint: None,
                plaintext_reason: None,
                ca_file: None,
            })
            .unwrap();
        for id in ["run-1", "run-2", "run-3"] {
            persist_finished(&st, &hist, None, id, &sample_results(Verdict::Pass)).unwrap();
        }
        // A stray leftover in the folder goes too.
        std::fs::write(hist.join("run-9.json.tmp"), b"half").unwrap();
        assert_eq!(std::fs::read_dir(&hist).unwrap().count(), 4);

        assert_eq!(purge(&st, &hist).unwrap(), 3);
        assert_eq!(
            std::fs::read_dir(&hist).unwrap().count(),
            0,
            "files are gone"
        );
        let guard = st.lock().unwrap();
        assert!(guard.history_list().unwrap().is_empty());
        assert_eq!(guard.list().unwrap().len(), 1, "profiles are kept");
        drop(guard);
        // Purging an empty or missing history is fine.
        assert_eq!(purge(&st, &hist).unwrap(), 0);
        assert_eq!(purge(&st, &dir.path().join("none")).unwrap(), 0);
    }

    #[test]
    fn ids_that_could_name_another_path_are_refused() {
        let dir = Path::new("/x");
        for bad in ["", "../a", "a/b", "a.b", "a b"] {
            assert!(results_path(dir, bad).is_err(), "{bad}");
        }
        assert!(results_path(dir, "run-ab-12").is_ok());
    }

    #[test]
    fn opened_results_are_bounded() {
        let mut table = OpenedResults::default();
        let opened = Opened {
            results: sample_results(Verdict::Pass),
            hooks: None,
            sha256: "s".into(),
        };
        for i in 0..(MAX_OPENED + 3) {
            table.insert(format!("id-{i}"), opened.clone());
        }
        assert_eq!(table.len(), MAX_OPENED);
        assert!(table.get("id-0").is_none());
        assert!(table.get(&format!("id-{}", MAX_OPENED + 2)).is_some());
        table.clear();
        assert_eq!(table.len(), 0);
    }
}
