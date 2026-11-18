//! Custom checks (Python hooks) in the desktop app (T074).
//!
//! The hooks engine is `dmux-hooks`; this module is the app's thin layer over
//! it.  What it adds:
//!
//! * the settings (on/off, scripts folder, Python interpreter, time limit),
//!   kept in the app's SQLite store.  The folder and the interpreter are
//!   chosen with native dialogs opened from Rust; the webview never sends a
//!   path that something is later executed from;
//! * a folder scan that lists each script and the reason a writable or
//!   foreign-owned one is refused;
//! * [`run_custom_checks`]: after the engine run finishes, run the usable
//!   scripts on the results exactly as `dmux run --hooks` does, build the same
//!   self-verifying envelope, and fold the findings into the verdict so a
//!   failing custom check never leaves a PASS showing.
//!
//! A hook is **not sandboxed**: it runs with the user's full permissions.  It
//! never receives a password or a connection (`dmux-hooks` strips both from
//! the results it hands over and from the environment it starts with).

use std::path::{Path, PathBuf};
use std::time::Duration;

use dmux_engine::results::{sha256_of, RunResults, Verdict};
use dmux_hooks::{HookError, HookResult, HookSpec, Severity};
use dmux_report::verify::{displayed_verdict, verify_value, HookEvidence};
use serde::Serialize;

use crate::error::CommandError;
use crate::store::Store;

/// The honest warning, shown wherever custom checks are on and before each run
/// that uses them.  The UI carries the same words (see `ui_warning_text` in the
/// settings tests).
pub const HOOK_WARNING: &str =
    "A custom check runs with your full permissions. Only run scripts you trust.";

pub const DEFAULT_TIMEOUT_SECS: u64 = 60;
pub const MIN_TIMEOUT_SECS: u64 = 1;
pub const MAX_TIMEOUT_SECS: u64 = 600;

const KEY_ENABLED: &str = "hooks_enabled";
const KEY_FOLDER: &str = "hooks_folder";
const KEY_PYTHON: &str = "hooks_python";
const KEY_TIMEOUT: &str = "hooks_timeout_secs";

/// The stored custom-check settings.  Off by default.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HookSettings {
    pub enabled: bool,
    pub folder: Option<PathBuf>,
    pub python: Option<PathBuf>,
    pub timeout_secs: u64,
}

impl Default for HookSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            folder: None,
            python: None,
            timeout_secs: DEFAULT_TIMEOUT_SECS,
        }
    }
}

impl HookSettings {
    pub fn load(store: &Store) -> Result<Self, CommandError> {
        let enabled = store.get_setting(KEY_ENABLED)?.as_deref() == Some("true");
        let folder = store
            .get_setting(KEY_FOLDER)?
            .filter(|s| !s.is_empty())
            .map(PathBuf::from);
        let python = store
            .get_setting(KEY_PYTHON)?
            .filter(|s| !s.is_empty())
            .map(PathBuf::from);
        let timeout_secs = store
            .get_setting(KEY_TIMEOUT)?
            .and_then(|s| s.parse::<u64>().ok())
            .map(clamp_timeout)
            .unwrap_or(DEFAULT_TIMEOUT_SECS);
        Ok(Self {
            enabled,
            folder,
            python,
            timeout_secs,
        })
    }

    pub fn save_enabled(store: &Store, enabled: bool) -> Result<(), CommandError> {
        store.set_setting(KEY_ENABLED, Some(if enabled { "true" } else { "false" }))
    }

    pub fn save_timeout(store: &Store, secs: u64) -> Result<(), CommandError> {
        if !(MIN_TIMEOUT_SECS..=MAX_TIMEOUT_SECS).contains(&secs) {
            return Err(CommandError::InvalidInput {
                message: format!(
                    "the time limit must be between {MIN_TIMEOUT_SECS} and {MAX_TIMEOUT_SECS} seconds"
                ),
            });
        }
        store.set_setting(KEY_TIMEOUT, Some(&secs.to_string()))
    }

    pub fn save_folder(store: &Store, folder: Option<&Path>) -> Result<(), CommandError> {
        store.set_setting(KEY_FOLDER, folder.map(|p| p.to_string_lossy()).as_deref())
    }

    pub fn save_python(store: &Store, python: Option<&Path>) -> Result<(), CommandError> {
        store.set_setting(KEY_PYTHON, python.map(|p| p.to_string_lossy()).as_deref())
    }
}

fn clamp_timeout(secs: u64) -> u64 {
    secs.clamp(MIN_TIMEOUT_SECS, MAX_TIMEOUT_SECS)
}

// ---------------------------------------------------------------------------
// Interpreter.
// ---------------------------------------------------------------------------

/// Where the interpreter comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PythonSource {
    /// Chosen by the user with the file dialog.
    Chosen,
    /// Shipped with the app (a `python/` folder in its resource directory).
    Bundled,
    /// Found on the machine, like the command-line tool does.
    Auto,
}

/// A Python shipped next to the app (the Windows build ships its own), if any.
pub fn bundled_interpreter(resource_dir: &Path) -> Option<PathBuf> {
    let base = resource_dir.join("python");
    [
        "python.exe",
        "bin/python3",
        "bin/python",
        "python3",
        "python",
    ]
    .iter()
    .map(|rel| base.join(rel))
    .find(|p| p.is_file())
}

/// The interpreter to use, its source, and whether one was actually found.
/// With no chosen or bundled one, the engine's own discovery applies
/// (`DMUX_PYTHON`, then `python3`, then `python` on `PATH`).
pub fn resolve_interpreter(
    settings: &HookSettings,
    bundled: Option<&Path>,
) -> (Option<PathBuf>, PythonSource) {
    if let Some(chosen) = &settings.python {
        return (Some(chosen.clone()), PythonSource::Chosen);
    }
    if let Some(b) = bundled {
        return (Some(b.to_path_buf()), PythonSource::Bundled);
    }
    (None, PythonSource::Auto)
}

/// The interpreter that would run now, if one can be found.
pub fn found_interpreter(
    settings: &HookSettings,
    bundled: Option<&Path>,
) -> Option<(PathBuf, PythonSource)> {
    let (explicit, source) = resolve_interpreter(settings, bundled);
    let spec = HookSpec {
        interpreter: explicit,
        ..HookSpec::default()
    };
    dmux_hooks::discover_interpreter(&spec)
        .ok()
        .map(|p| (p, source))
}

// ---------------------------------------------------------------------------
// Folder scan.
// ---------------------------------------------------------------------------

/// One script in the folder.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct ScriptEntry {
    pub name: String,
    /// False when Dmux will not run it; `reason` says why in plain words.
    pub usable: bool,
    pub reason: Option<String>,
}

/// What a folder holds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct FolderScan {
    /// Set when the whole folder is refused or unreadable.
    pub folder_problem: Option<String>,
    pub scripts: Vec<ScriptEntry>,
}

/// A refusal in plain words (no path: the user knows which folder).
fn plain_reason(err: &HookError) -> String {
    let text = err.to_string();
    if text.contains("writable") {
        "other users on this computer can change it; make it writable only by you".to_string()
    } else if text.contains("not owned") {
        "it belongs to a different user".to_string()
    } else {
        "it cannot be read".to_string()
    }
}

/// Remove control and invisible characters from text that came from the file
/// system or from a script, so it cannot disguise itself in the UI.
fn clean(text: &str) -> String {
    text.chars()
        .filter(|c| {
            !c.is_control()
                && !matches!(
                    c,
                    '\u{200B}'..='\u{200F}'
                        | '\u{202A}'..='\u{202E}'
                        | '\u{2060}'..='\u{2064}'
                        | '\u{2066}'..='\u{2069}'
                        | '\u{FEFF}'
                )
        })
        .collect()
}

/// List the `.py` files in `dir` and say which Dmux will refuse.
pub fn scan_folder(dir: &Path) -> FolderScan {
    if !dir.is_dir() {
        return FolderScan {
            folder_problem: Some("the folder no longer exists".to_string()),
            scripts: Vec::new(),
        };
    }
    if let Err(e) = dmux_hooks::check_hook_path_permissions(dir) {
        return FolderScan {
            folder_problem: Some(format!("the folder is refused: {}", plain_reason(&e))),
            scripts: Vec::new(),
        };
    }
    let entries = match std::fs::read_dir(dir) {
        Ok(rd) => rd,
        Err(_) => {
            return FolderScan {
                folder_problem: Some("the folder cannot be read".to_string()),
                scripts: Vec::new(),
            }
        }
    };
    let mut paths: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "py") && p.is_file())
        .collect();
    paths.sort();
    let scripts = paths
        .iter()
        .map(|p| {
            let name = clean(&p.file_name().unwrap_or_default().to_string_lossy());
            match dmux_hooks::check_hook_path_permissions(p) {
                Ok(()) => ScriptEntry {
                    name,
                    usable: true,
                    reason: None,
                },
                Err(e) => ScriptEntry {
                    name,
                    usable: false,
                    reason: Some(plain_reason(&e)),
                },
            }
        })
        .collect();
    FolderScan {
        folder_problem: None,
        scripts,
    }
}

// ---------------------------------------------------------------------------
// Running.
// ---------------------------------------------------------------------------

/// What a run needs to run its custom checks, captured when the run starts so
/// a settings change mid-run cannot alter it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HookPlan {
    pub folder: Option<PathBuf>,
    pub interpreter: Option<PathBuf>,
    pub timeout: Duration,
}

/// The plan for a new run: `None` when custom checks are off.
pub fn plan_from_settings(settings: &HookSettings, bundled: Option<&Path>) -> Option<HookPlan> {
    if !settings.enabled {
        return None;
    }
    let (interpreter, _) = resolve_interpreter(settings, bundled);
    Some(HookPlan {
        folder: settings.folder.clone(),
        interpreter,
        timeout: Duration::from_secs(clamp_timeout(settings.timeout_secs)),
    })
}

/// The plan for a run that is about to start, from the stored settings.
pub fn plan_for_new_run(state: &crate::state::AppState) -> Result<Option<HookPlan>, CommandError> {
    let settings = HookSettings::load(&*state.store()?)?;
    Ok(plan_from_settings(
        &settings,
        state.bundled_python().as_deref(),
    ))
}

/// Called by the drive task when the engine returns: run the run's custom
/// checks (if it has any) and record their outcome for `generation`.  A failed
/// engine run has nothing to check.
pub fn after_run(
    manager: &crate::run_manager::RunManager,
    run_id: &str,
    generation: u64,
    outcome: &Result<RunResults, dmux_engine::run::RunError>,
) {
    let (Some(plan), Ok(results)) = (manager.hook_plan(run_id), outcome) else {
        return;
    };
    manager.set_hook_run(run_id, generation, run_custom_checks(&plan, results));
}

/// The outcome of the custom checks for one run.
#[derive(Debug, Clone)]
pub struct HookRun {
    /// The results with any custom-check verdict change folded in (hash
    /// recomputed), as the envelope carries them.
    pub results: RunResults,
    /// `{ results, hooks, envelope_sha256 }`: what the report renders from.
    pub envelope: serde_json::Value,
    pub evidence: HookEvidence,
}

impl HookRun {
    /// The verdict to show: the engine's, worsened by any custom finding.
    pub fn displayed_verdict(&self) -> Verdict {
        displayed_verdict(&self.results, &self.evidence)
    }
}

fn not_run(name: &str, why: &str) -> (String, HookResult) {
    (
        name.to_string(),
        HookResult {
            script_hash: "(not run)".to_string(),
            findings: Vec::new(),
            warnings: vec![why.to_string()],
        },
    )
}

/// Run the plan's scripts on the results.  Blocking: call it from a blocking
/// thread (the drive task already is one).
///
/// Every way custom checks can go wrong (no folder, refused folder or script,
/// no Python, a timeout, bad output) becomes a recorded warning that lifts a
/// clean PASS to WARNING; a `fail` finding turns PASS or WARNING into FAIL.
/// Nothing here can raise a verdict toward PASS.
pub fn run_custom_checks(plan: &HookPlan, results: &RunResults) -> HookRun {
    let results_value = serde_json::to_value(results).unwrap_or_default();
    let mut ran: Vec<(String, HookResult)> = Vec::new();

    match &plan.folder {
        None => ran.push(not_run(
            "custom checks",
            "custom checks are on but no scripts folder is chosen",
        )),
        Some(dir) => {
            let scan = scan_folder(dir);
            if let Some(problem) = &scan.folder_problem {
                ran.push(not_run("custom checks", problem));
            } else if scan.scripts.is_empty() {
                ran.push(not_run(
                    "custom checks",
                    "custom checks are on but the folder has no .py scripts",
                ));
            }
            for entry in &scan.scripts {
                if !entry.usable {
                    let why = format!("not run: {}", entry.reason.as_deref().unwrap_or("refused"));
                    ran.push(not_run(&entry.name, &why));
                    continue;
                }
                let spec = HookSpec {
                    script: dir.join(&entry.name),
                    interpreter: plan.interpreter.clone(),
                    timeout: plan.timeout,
                    ..HookSpec::default()
                };
                match dmux_hooks::run_hook(&spec, &results_value) {
                    Ok(res) => ran.push((entry.name.clone(), res)),
                    Err(HookError::NoInterpreter) => ran.push(not_run(
                        &entry.name,
                        "not run: no Python was found on this computer",
                    )),
                    Err(e) => ran.push(not_run(
                        &entry.name,
                        &format!("not run: {}", clean(&e.to_string())),
                    )),
                }
            }
        }
    }

    let hook_results: Vec<&HookResult> = ran.iter().map(|(_, r)| r).collect();
    let has_fail = hook_results
        .iter()
        .any(|r| r.findings.iter().any(|f| f.severity == Severity::Fail));
    let has_warning = hook_results.iter().any(|r| !r.warnings.is_empty());

    // Fold into a typed copy so `results_sha256` covers exactly what is
    // written (the same steps as `dmux run --hooks`).
    let mut folded = results.clone();
    apply_hook_verdict(&mut folded, has_fail, has_warning, &hook_results);
    folded.results_sha256 = sha256_of(&folded);
    let folded_value = serde_json::to_value(&folded).unwrap_or_default();
    let pairs: Vec<(String, &HookResult)> = ran.iter().map(|(n, r)| (n.clone(), r)).collect();
    let envelope = dmux_hooks::build_envelope(&folded_value, &pairs);

    match verify_value(&envelope) {
        Ok(v) => HookRun {
            results: v.results,
            evidence: v.hooks.unwrap_or_default(),
            envelope,
        },
        Err(_) => {
            // Cannot happen for an envelope built here; if it ever does, the
            // run must not read as clean.
            let mut bad = folded;
            bad.verdict = bad.verdict.worst(Verdict::ToolFault);
            bad.warnings
                .push("the custom-check evidence could not be verified".to_string());
            bad.results_sha256 = sha256_of(&bad);
            let value = serde_json::to_value(&bad).unwrap_or_default();
            let envelope = dmux_hooks::build_envelope(&value, &pairs);
            HookRun {
                results: bad,
                evidence: HookEvidence::default(),
                envelope,
            }
        }
    }
}

/// Fold the hook outcome into the typed results (the same rules as the
/// command-line tool): a `fail` finding turns PASS or WARNING into FAIL; a hook
/// warning turns a PASS into WARNING.  Neither lowers a graver verdict.
fn apply_hook_verdict(
    results: &mut RunResults,
    has_fail: bool,
    has_warning: bool,
    hook_results: &[&HookResult],
) {
    if has_fail {
        if matches!(results.verdict, Verdict::Pass | Verdict::Warning) {
            results.verdict = Verdict::Fail;
        }
        for r in hook_results {
            for f in r.findings.iter().filter(|f| f.severity == Severity::Fail) {
                results
                    .warnings
                    .push(format!("custom check {} failed", clean(&f.id)));
            }
        }
    } else if has_warning && results.verdict == Verdict::Pass {
        results.verdict = Verdict::Warning;
        results
            .warnings
            .push("a custom check did not run cleanly; see the custom checks section".to_string());
    }
}

// ---------------------------------------------------------------------------
// Contract shape.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct CustomFindingDto {
    pub id: String,
    /// `info`, `warning` or `fail`.
    pub severity: String,
    pub message: String,
    pub table: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct CustomScriptDto {
    pub name: String,
    pub sha256: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct CustomChecksDto {
    pub findings: Vec<CustomFindingDto>,
    pub scripts: Vec<CustomScriptDto>,
    pub warnings: Vec<String>,
}

impl CustomChecksDto {
    pub fn from_run(run: &HookRun) -> Self {
        Self {
            findings: run
                .evidence
                .custom_checks
                .iter()
                .map(|f| CustomFindingDto {
                    id: clean(&f.id),
                    severity: f.severity.clone(),
                    message: clean(&f.message),
                    table: f.table.as_deref().map(clean),
                })
                .collect(),
            scripts: run
                .evidence
                .scripts
                .iter()
                .map(|s| CustomScriptDto {
                    name: clean(&s.script),
                    sha256: clean(&s.sha256),
                })
                .collect(),
            warnings: run.evidence.warnings.iter().map(|w| clean(w)).collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dmux_engine::results::{finalise, CheckLevel, TableResult, RESULTS_VERSION};
    use std::collections::BTreeMap;

    fn clean_results() -> RunResults {
        let table = TableResult {
            table: "demo_bank.accounts".into(),
            level: CheckLevel::FullValues,
            verdict: Verdict::Pass,
            old_rows: Some(3),
            new_rows: Some(3),
            columns_covered: Some(vec!["id".into()]),
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
            verdict: Verdict::Pass,
            coverage_line:
                "1 of 1 tables fully checked, 0 counts only, 0 structure only, 0 ignored".into(),
            fully_checked: 1,
            tables: BTreeMap::from([("demo_bank.accounts".into(), table)]),
            ..RunResults::default()
        })
    }

    /// Python is needed for the tests that run a script: skip cleanly when it
    /// is missing, except under CI where it must be there.
    fn python_or_skip() -> bool {
        let found = dmux_hooks::discover_interpreter(&HookSpec::default()).is_ok();
        if !found {
            assert!(
                std::env::var("CI").is_err(),
                "python3 is required when CI is set"
            );
            eprintln!("skipping: no Python on this machine");
        }
        found
    }

    #[cfg(unix)]
    fn write_script(dir: &Path, name: &str, body: &str, mode: u32) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let p = dir.join(name);
        std::fs::write(&p, body).unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(mode)).unwrap();
        p
    }

    #[cfg(unix)]
    fn private_dir() -> tempfile::TempDir {
        use std::os::unix::fs::PermissionsExt;
        let d = tempfile::tempdir().unwrap();
        std::fs::set_permissions(d.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        d
    }

    fn plan(dir: &Path, secs: u64) -> HookPlan {
        HookPlan {
            folder: Some(dir.to_path_buf()),
            interpreter: None,
            timeout: Duration::from_secs(secs),
        }
    }

    const FAIL_HOOK: &str = r#"
import json, sys
json.load(sys.stdin)
print(json.dumps({"findings": [{"id": "balance-sum", "severity": "fail", "table": "demo_bank.accounts", "message": "totals differ by 12"}]}))
"#;

    #[test]
    fn the_warning_is_the_agreed_sentence() {
        assert_eq!(
            HOOK_WARNING,
            "A custom check runs with your full permissions. Only run scripts you trust."
        );
    }

    #[test]
    fn custom_checks_are_off_by_default_and_the_plan_is_none() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("d.db")).unwrap();
        let s = HookSettings::load(&store).unwrap();
        assert!(!s.enabled);
        assert_eq!(s.timeout_secs, DEFAULT_TIMEOUT_SECS);
        assert!(plan_from_settings(&s, None).is_none());
    }

    #[test]
    fn settings_persist_and_the_timeout_is_bounded() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("d.db")).unwrap();
        HookSettings::save_enabled(&store, true).unwrap();
        HookSettings::save_timeout(&store, 5).unwrap();
        HookSettings::save_folder(&store, Some(Path::new("/scripts"))).unwrap();
        HookSettings::save_python(&store, Some(Path::new("/usr/bin/python3"))).unwrap();
        let s = HookSettings::load(&store).unwrap();
        assert!(s.enabled);
        assert_eq!(s.timeout_secs, 5);
        assert_eq!(s.folder.as_deref(), Some(Path::new("/scripts")));
        assert_eq!(s.python.as_deref(), Some(Path::new("/usr/bin/python3")));
        assert!(HookSettings::save_timeout(&store, 0).is_err());
        assert!(HookSettings::save_timeout(&store, MAX_TIMEOUT_SECS + 1).is_err());
        // A damaged stored value falls back to the default.
        store.set_setting(KEY_TIMEOUT, Some("soon")).unwrap();
        assert_eq!(
            HookSettings::load(&store).unwrap().timeout_secs,
            DEFAULT_TIMEOUT_SECS
        );
        let p = plan_from_settings(&s, None).unwrap();
        assert_eq!(p.timeout, Duration::from_secs(5));
    }

    #[test]
    fn a_chosen_interpreter_beats_a_bundled_one_beats_auto() {
        let mut s = HookSettings::default();
        assert_eq!(resolve_interpreter(&s, None), (None, PythonSource::Auto));
        let bundled = Path::new("/app/python/bin/python3");
        assert_eq!(
            resolve_interpreter(&s, Some(bundled)),
            (Some(bundled.to_path_buf()), PythonSource::Bundled)
        );
        s.python = Some(PathBuf::from("/opt/py"));
        assert_eq!(
            resolve_interpreter(&s, Some(bundled)),
            (Some(PathBuf::from("/opt/py")), PythonSource::Chosen)
        );
    }

    #[test]
    fn a_python_folder_in_the_resource_dir_is_found() {
        let dir = tempfile::tempdir().unwrap();
        assert!(bundled_interpreter(dir.path()).is_none());
        std::fs::create_dir_all(dir.path().join("python/bin")).unwrap();
        std::fs::write(dir.path().join("python/bin/python3"), "").unwrap();
        assert_eq!(
            bundled_interpreter(dir.path()),
            Some(dir.path().join("python/bin/python3"))
        );
    }

    #[cfg(unix)]
    #[test]
    fn writable_and_open_scripts_are_listed_as_refused_with_a_reason() {
        let dir = private_dir();
        write_script(dir.path(), "good.py", "print(1)", 0o700);
        write_script(dir.path(), "shared.py", "print(1)", 0o770);
        write_script(dir.path(), "open.py", "print(1)", 0o666);
        write_script(dir.path(), "notes.txt", "x", 0o600);
        let scan = scan_folder(dir.path());
        assert_eq!(scan.folder_problem, None);
        let by_name: BTreeMap<_, _> = scan.scripts.iter().map(|s| (s.name.as_str(), s)).collect();
        assert_eq!(by_name.len(), 3, "only .py files are listed");
        assert!(by_name["good.py"].usable);
        for bad in ["shared.py", "open.py"] {
            assert!(!by_name[bad].usable, "{bad}");
            assert!(by_name[bad]
                .reason
                .as_deref()
                .unwrap()
                .contains("can change it"));
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_world_writable_folder_is_refused_whole() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o777)).unwrap();
        write_script(dir.path(), "a.py", "print(1)", 0o700);
        let scan = scan_folder(dir.path());
        assert!(scan.folder_problem.unwrap().contains("refused"));
        assert!(scan.scripts.is_empty());
        let missing = scan_folder(&dir.path().join("gone"));
        assert!(missing.folder_problem.unwrap().contains("no longer exists"));
    }

    #[cfg(unix)]
    #[test]
    fn a_writable_script_is_not_run_and_the_run_is_not_a_clean_pass() {
        let dir = private_dir();
        let marker = dir.path().join("ran.txt");
        let body = format!(
            "open({:?}, 'w').write('ran')\nprint('{{\"findings\": []}}')",
            marker.to_string_lossy()
        );
        write_script(dir.path(), "evil.py", &body, 0o777);
        let run = run_custom_checks(&plan(dir.path(), 10), &clean_results());
        assert!(!marker.exists(), "a refused script must never execute");
        assert_eq!(run.results.verdict, Verdict::Warning);
        assert_eq!(run.displayed_verdict(), Verdict::Warning);
        assert!(run
            .evidence
            .warnings
            .iter()
            .any(|w| w.contains("evil.py") && w.contains("can change it")));
    }

    #[cfg(unix)]
    #[test]
    fn a_failing_finding_reaches_the_results_and_never_leaves_a_pass() {
        if !python_or_skip() {
            return;
        }
        let dir = private_dir();
        write_script(dir.path(), "balance.py", FAIL_HOOK, 0o700);
        let input = clean_results();
        assert_eq!(input.verdict, Verdict::Pass);
        let run = run_custom_checks(&plan(dir.path(), 30), &input);
        assert_eq!(run.results.verdict, Verdict::Fail);
        assert_eq!(run.displayed_verdict(), Verdict::Fail);
        assert_eq!(run.evidence.custom_checks.len(), 1);
        assert_eq!(run.evidence.custom_checks[0].id, "balance-sum");
        assert_eq!(run.evidence.scripts[0].script, "balance.py");
        assert_eq!(run.evidence.scripts[0].sha256.len(), 64);
        // The envelope verifies and carries the same evidence the report reads.
        let v = verify_value(&run.envelope).expect("envelope verifies");
        assert_eq!(v.hooks.unwrap().custom_checks.len(), 1);
        let dto = CustomChecksDto::from_run(&run);
        assert_eq!(dto.findings[0].severity, "fail");
    }

    #[cfg(unix)]
    #[test]
    fn info_and_warning_findings_never_show_pass_either() {
        if !python_or_skip() {
            return;
        }
        let dir = private_dir();
        write_script(
            dir.path(),
            "note.py",
            "import json\nprint(json.dumps({'findings': [{'id': 'n', 'severity': 'info', 'message': 'looks fine'}]}))",
            0o700,
        );
        let run = run_custom_checks(&plan(dir.path(), 30), &clean_results());
        // The engine verdict stays PASS; what is displayed must not.
        assert_eq!(run.results.verdict, Verdict::Pass);
        assert_eq!(run.displayed_verdict(), Verdict::Warning);
    }

    #[cfg(unix)]
    #[test]
    fn a_hook_cannot_lift_a_failed_run_to_pass() {
        if !python_or_skip() {
            return;
        }
        let dir = private_dir();
        write_script(
            dir.path(),
            "ok.py",
            "import json\nprint(json.dumps({'findings': []}))",
            0o700,
        );
        let mut bad = clean_results();
        bad.verdict = Verdict::NeedsReview;
        bad.tables.get_mut("demo_bank.accounts").unwrap().verdict = Verdict::NeedsReview;
        let bad = finalise(bad);
        let run = run_custom_checks(&plan(dir.path(), 30), &bad);
        assert_eq!(run.displayed_verdict(), Verdict::NeedsReview);
    }

    #[cfg(unix)]
    #[test]
    fn a_hanging_hook_is_stopped_by_the_time_limit() {
        if !python_or_skip() {
            return;
        }
        let dir = private_dir();
        write_script(
            dir.path(),
            "hang.py",
            "import time\ntime.sleep(60)\n",
            0o700,
        );
        let started = std::time::Instant::now();
        let run = run_custom_checks(&plan(dir.path(), 1), &clean_results());
        assert!(started.elapsed() < Duration::from_secs(20));
        assert!(run
            .evidence
            .warnings
            .iter()
            .any(|w| w.contains("timed out")));
        assert_eq!(run.displayed_verdict(), Verdict::Warning);
    }

    #[cfg(unix)]
    #[test]
    fn scripts_get_no_connection_details_and_no_secret_environment() {
        if !python_or_skip() {
            return;
        }
        std::env::set_var("DMUX_TEST_HOOK_SECRET_MARKER", "x");
        std::env::set_var("PGPASSWORD_MARKER_T074", "x");
        let dir = private_dir();
        write_script(
            dir.path(),
            "peek.py",
            r#"
import json, os, sys
payload = sys.stdin.read()
leaks = []
if "connections" in payload: leaks.append("connections key")
if "sentinel-reason-t074" in payload: leaks.append("connection details")
for k in os.environ:
    u = k.upper()
    if u.startswith("DMUX_") or u.startswith("PG") and u != "PATH" or "PASSWORD" in u or "SECRET" in u or "TOKEN" in u:
        leaks.append("env " + k)
print(json.dumps({"findings": [{"id": "leak", "severity": "fail", "message": "saw " + ", ".join(leaks)}] if leaks else []}))
"#,
            0o700,
        );
        let mut input = clean_results();
        input.connections.old.plaintext_reason = Some("sentinel-reason-t074".to_string());
        input.connections.new.superuser_reason = Some("sentinel-reason-t074".to_string());
        let input = finalise(input);
        let run = run_custom_checks(&plan(dir.path(), 30), &input);
        assert!(
            run.evidence.custom_checks.is_empty(),
            "the script saw something it must not: {:?}",
            run.evidence.custom_checks
        );
        assert_eq!(run.results.verdict, Verdict::Pass);
        std::env::remove_var("DMUX_TEST_HOOK_SECRET_MARKER");
        std::env::remove_var("PGPASSWORD_MARKER_T074");
    }

    #[test]
    fn no_folder_or_an_empty_folder_is_a_warning_not_a_silent_pass() {
        let none = HookPlan {
            folder: None,
            interpreter: None,
            timeout: Duration::from_secs(5),
        };
        let run = run_custom_checks(&none, &clean_results());
        assert_eq!(run.displayed_verdict(), Verdict::Warning);
        #[cfg(unix)]
        {
            let dir = private_dir();
            let run = run_custom_checks(&plan(dir.path(), 5), &clean_results());
            assert_eq!(run.displayed_verdict(), Verdict::Warning);
        }
    }

    #[test]
    fn control_and_bidi_characters_are_stripped_from_displayed_text() {
        assert_eq!(clean("a\u{202E}b\u{0007}c\nd"), "abcd");
    }
}
