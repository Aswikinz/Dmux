//! Python hooks (PLAN.md 4.9).
//!
//! A *hook* is a user-supplied Python script that reads the run's results as
//! JSON on stdin and writes extra findings, as JSON, on stdout.  The findings
//! appear in the report marked as custom checks.
//!
//! Scripts are **untrusted input** (PLAN.md 6.3 / AGENTS.md rule 2), but Dmux
//! does **not** sandbox them (decision 23): **a check runs with your full
//! permissions — only run scripts you trust.** A script can read files, open
//! network connections and act on the desktop session exactly as the user who
//! launched the run.  What this module does instead:
//!
//! - **No secrets in the environment.** The child receives no inherited
//!   environment except a tiny allow-list (PATH, LANG, LC_ALL, and SYSTEMROOT on
//!   Windows).  `PG*`, `DMUX_*`, `*PASSWORD*`, `*TOKEN*` and `*SECRET*` are never
//!   passed through, and the results payload is scrubbed of password/token/
//!   connection keys.  This limits accidental leakage; it does not stop a
//!   hostile script from reading those secrets itself.
//! - **No row data.** The results JSON carries counts, hashes and names only
//!   (PLAN.md 6.6); the payload writer scrubs any sensitive key as defence in
//!   depth, and drops the `missing_keys` / `extra_keys` key lists even when
//!   `export_keys` is on.
//! - **A time limit.** The interpreter is killed when it exceeds the limit
//!   (default 60 s), along with any children, via a Unix process group (Windows:
//!   a new process group plus `taskkill /F /T`).
//! - **Bounded, validated output.** stdout is capped at [`MAX_OUTPUT_BYTES`]
//!   and parsed against a strict schema with `deny_unknown_fields`; every
//!   finding's message length and control characters are checked.
//! - **Refuses unsafe scripts.** A script or hooks directory that is group- or
//!   world-writable, or not owned by the current user, is refused on Unix (see
//!   [`check_hook_path_permissions`]).
//!
//! A hook *failure* (timeout, garbage output, exit with no findings) is always
//! a recorded warning, never a crash and never a PASS-affecting verdict by
//! itself.  A hook *finding* of severity `fail` forces the overall verdict off
//! PASS; an `info` or `warning` finding does not, and findings can never raise
//! a non-PASS run to PASS.

use serde::{Deserialize, Serialize};
use sha2::Digest;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

/// Sensible default time limit for a hook (PLAN.md 4.9).
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(60);

/// Upper bound on a hook's stdout.  Anything larger is truncated and rejected.
pub const MAX_OUTPUT_BYTES: usize = 1024 * 1024;

/// Upper bound on any finding's message length (prevents a hostile script from
/// bloating the report).
pub const MAX_MESSAGE_BYTES: usize = 4096;

/// Upper bound on the results payload we are willing to hand a hook.  The
/// payload is the run's results (counts, hashes, names, per-table facts), which
/// for a very large database runs to hundreds of megabytes, so the bound was
/// raised from 1 MiB (decision 29).  Over it, the hook is not run and the run
/// says so; truncated JSON is never sent.
pub const MAX_INPUT_BYTES: usize = 1024 * 1024 * 1024;

/// One finding returned by a hook — the exact schema PLAN.md 4.9 spells out.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct Finding {
    pub id: String,
    pub severity: Severity,
    #[serde(default)]
    pub table: Option<String>,
    pub message: String,
}

/// A finding's severity.  Only `fail` affects the verdict.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Info,
    Warning,
    Fail,
}

impl std::str::FromStr for Severity {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "info" => Ok(Severity::Info),
            "warning" => Ok(Severity::Warning),
            "fail" => Ok(Severity::Fail),
            other => Err(format!("unknown severity: {other}")),
        }
    }
}

/// The strict output schema a hook must emit on stdout.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct HookOutput {
    pub findings: Vec<Finding>,
}

/// A parsed, validated hook result ready to merge into the report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HookResult {
    /// The SHA-256 hex digest of the script file (PLAN 4.9).
    pub script_hash: String,
    /// Findings the script returned, in order.
    pub findings: Vec<Finding>,
    /// Non-fatal problems encountered while running the hook.  Each one is a
    /// warning in the report, never a verdict change.
    pub warnings: Vec<String>,
}

/// Everything needed to run one hook.
#[derive(Debug, Clone)]
pub struct HookSpec {
    /// Path to the Python script (hashed, then executed).
    pub script: PathBuf,
    /// Interpreter to spawn.  `None` auto-discovers `python3` then `python`
    /// from `PATH`, or honours `DMUX_PYTHON`.
    pub interpreter: Option<PathBuf>,
    /// Time limit (default [`DEFAULT_TIMEOUT`]).
    pub timeout: Duration,
    /// The largest results payload this hook is given (default
    /// [`MAX_INPUT_BYTES`]); over it the hook is not run.
    pub max_input_bytes: usize,
}

impl Default for HookSpec {
    fn default() -> Self {
        Self {
            script: PathBuf::new(),
            interpreter: None,
            timeout: DEFAULT_TIMEOUT,
            max_input_bytes: MAX_INPUT_BYTES,
        }
    }
}

impl HookSpec {
    pub fn new(script: impl Into<PathBuf>) -> Self {
        Self {
            script: script.into(),
            ..Self::default()
        }
    }
}

/// Errors that stop hook *setup* before a script runs.
#[derive(Debug, thiserror::Error)]
pub enum HookError {
    #[error("hook script not found: {0}")]
    ScriptNotFound(String),
    #[error("failed to read hook script {0}: {1}")]
    ReadScript(String, String),
    #[error("no Python interpreter found on PATH")]
    NoInterpreter,
    #[error("hook runtime error: {0}")]
    Runtime(String),
    #[error("unsafe hook: {0}")]
    UnsafeScript(String),
}

/// How a finding folds into the run verdict.  Only `fail` forces off PASS.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerdictEffect {
    None,
    ForceNonPass,
}

impl VerdictEffect {
    pub fn of(severity: Severity) -> Self {
        match severity {
            Severity::Fail => VerdictEffect::ForceNonPass,
            _ => VerdictEffect::None,
        }
    }
}

/// Serialise the results we hand a hook, scrubbed of any sensitive key and of
/// the exported key lists.
///
/// The payload is deliberately small and secret-free (PLAN.md 6.6): run id,
/// tool version, mapping hash, verdict vocabulary, coverage line, per-table
/// verdicts with counts and reasons — never a password or a row value.  The
/// scrub pass is defence in depth against a future schema field, and it also
/// drops the `missing_keys` / `extra_keys` key lists (row keys are never sent
/// to a hook, even when `export_keys` is on; audit L10).
pub fn results_payload(results: &serde_json::Value) -> serde_json::Value {
    fn scrub(value: &mut serde_json::Value) {
        match value {
            serde_json::Value::Object(map) => {
                let keys: Vec<String> = map.keys().cloned().collect();
                for k in keys {
                    if is_sensitive_key(&k) {
                        map.remove(&k);
                    } else if let Some(v) = map.get_mut(&k) {
                        scrub(v);
                    }
                }
            }
            serde_json::Value::Array(items) => {
                for item in items {
                    scrub(item);
                }
            }
            _ => {}
        }
    }

    let mut out = results.clone();
    scrub(&mut out);
    out
}

fn is_sensitive_key(key: &str) -> bool {
    let upper = key.to_ascii_uppercase();
    (upper.contains("PASSWORD"))
        || upper.contains("TOKEN")
        || upper.contains("CONN")
        || upper.contains("DSN")
        || upper.contains("SECRET")
        || upper.contains("CREDENTIAL")
        || upper == "MISSING_KEYS"
        || upper == "EXTRA_KEYS"
}

/// Compute a self-verifying SHA-256 over the *whole* written envelope
/// (`{results, hooks}`), so a reader can detect tampering with either the
/// results or the hook evidence (script hashes, findings).  The digest covers
/// everything except the `envelope_sha256` field itself, so the field can be
/// recomputed from the file without recursing.
///
/// The results' own `results_sha256` (computed by the engine) protects the
/// results values; this envelope hash additionally binds the hook verdicts and
/// script digests to those results, closing the hole where a `fail` finding
/// could be edited away without changing `results_sha256`.
pub fn envelope_sha256(envelope: &serde_json::Value) -> String {
    let mut clone = envelope.clone();
    if let Some(obj) = clone.as_object_mut() {
        obj.remove("envelope_sha256");
    }
    // A canonical serialisation is stable for identical content (serde_json's
    // default map order is BTreeMap here), so the digest is reproducible.
    let canonical = serde_json::to_string(&clone).unwrap_or_default();
    sha256_hex(canonical.as_bytes())
}

/// The JSON form of one finding inside an envelope (marked `custom_checks`).
pub fn finding_json(f: &Finding) -> serde_json::Value {
    let severity = match f.severity {
        Severity::Info => "info",
        Severity::Warning => "warning",
        Severity::Fail => "fail",
    };
    let mut v = serde_json::json!({
        "id": f.id,
        "severity": severity,
        "message": f.message,
        "source": "custom_checks"
    });
    if let Some(t) = &f.table {
        v["table"] = serde_json::Value::String(t.clone());
    }
    v
}

/// Build the self-verifying envelope `{ results, hooks, envelope_sha256 }` that
/// `dmux run --hooks` writes and `dmux_report::verify` reads.  `scripts` pairs
/// each script's file name with its result, in run order.
pub fn build_envelope(
    results: &serde_json::Value,
    scripts: &[(String, &HookResult)],
) -> serde_json::Value {
    let mut custom_checks: Vec<serde_json::Value> = Vec::new();
    let mut script_rows: Vec<serde_json::Value> = Vec::new();
    let mut warnings: Vec<String> = Vec::new();
    for (name, res) in scripts {
        for w in &res.warnings {
            warnings.push(format!("{name}: {w}"));
        }
        script_rows.push(serde_json::json!({
            "script": name,
            "sha256": res.script_hash,
            "findings": res.findings.iter().map(finding_json).collect::<Vec<_>>(),
        }));
        for f in &res.findings {
            custom_checks.push(finding_json(f));
        }
    }
    let mut envelope = serde_json::json!({
        "results": results,
        "hooks": {
            "custom_checks": custom_checks,
            "scripts": script_rows,
            "warnings": warnings,
        }
    });
    let digest = envelope_sha256(&envelope);
    envelope["envelope_sha256"] = serde_json::Value::String(digest);
    envelope
}

/// Verify an envelope's integrity: recompute the digest and compare it to the
/// stored `envelope_sha256`.  Returns an error string describing the mismatch,
/// or `Ok(())` when the content is unchanged.
pub fn verify_envelope(envelope: &serde_json::Value) -> Result<(), String> {
    let stored = envelope
        .get("envelope_sha256")
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    if stored.is_empty() {
        return Err("envelope has no envelope_sha256".to_string());
    }
    let actual = envelope_sha256(envelope);
    if actual != stored {
        return Err("envelope_sha256 mismatch: file was modified after running".to_string());
    }
    Ok(())
}

/// Compute the SHA-256 hex digest of the script file's bytes (PLAN 4.9).
pub fn hash_script(path: &Path) -> Result<String, HookError> {
    let bytes = std::fs::read(path)
        .map_err(|e| HookError::ReadScript(path.display().to_string(), e.to_string()))?;
    Ok(sha256_hex(&bytes))
}

/// Resolve the interpreter: explicit path, then `DMUX_PYTHON`, then `python3`,
/// then `python` from `PATH`.
pub fn discover_interpreter(spec: &HookSpec) -> Result<PathBuf, HookError> {
    if let Some(explicit) = &spec.interpreter {
        if !explicit.as_os_str().is_empty() {
            return Ok(explicit.clone());
        }
    }
    if let Ok(p) = std::env::var("DMUX_PYTHON") {
        if !p.trim().is_empty() {
            return Ok(PathBuf::from(p));
        }
    }
    for candidate in ["python3", "python"] {
        if let Some(found) = find_on_path(candidate) {
            return Ok(found);
        }
    }
    Err(HookError::NoInterpreter)
}

fn find_on_path(binary: &str) -> Option<PathBuf> {
    let paths = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&paths) {
        #[cfg(unix)]
        let candidate = dir.join(binary);
        #[cfg(windows)]
        let candidate = dir.join(format!("{binary}.exe"));
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

/// The minimal environment a hook inherits.  Everything else is stripped so
/// `PG*`, `DMUX_*`, `*PASSWORD*`, `*TOKEN*` and `*SECRET*` never reach a script.
fn allowed_env() -> BTreeMap<String, String> {
    let mut env = BTreeMap::new();
    for key in ["PATH", "LANG", "LC_ALL"] {
        if let Ok(v) = std::env::var(key) {
            env.insert(key.to_string(), v);
        }
    }
    #[cfg(windows)]
    if let Ok(v) = std::env::var("SYSTEMROOT") {
        env.insert("SYSTEMROOT".to_string(), v);
    }
    env
}

/// A key that must never reach a hook (part of the env strip assertion).
pub fn is_denied_key(key: &str) -> bool {
    let upper = key.to_ascii_uppercase();
    (upper.starts_with("PG") && upper != "PATH")
        || upper.starts_with("DMUX_")
        || upper.contains("PASSWORD")
        || upper.contains("TOKEN")
        || upper.contains("SECRET")
}

/// Refuse a hook script or hooks directory that is group- or world-writable, or
/// not owned by the current user (decision 23: with no sandbox in v1.0, a
/// writable or foreign-owned hook is a privilege boundary Dmux will not cross).
///
/// On Windows there is no equivalent cheap check (ACLs are richer and there is
/// no uid), so this is a no-op there and the honest full-permissions warning
/// above still applies.
pub fn check_hook_path_permissions(path: &Path) -> Result<(), HookError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let meta = std::fs::metadata(path)
            .map_err(|e| HookError::ReadScript(path.display().to_string(), e.to_string()))?;
        if meta.mode() & 0o022 != 0 {
            return Err(HookError::UnsafeScript(format!(
                "{} is group- or world-writable; a hook must be writable only by its owner",
                path.display()
            )));
        }
        let euid = unsafe { libc::geteuid() };
        if meta.uid() != euid {
            return Err(HookError::UnsafeScript(format!(
                "{} is not owned by the current user; refusing to run it",
                path.display()
            )));
        }
    }
    #[cfg(not(unix))]
    {
        // Windows: skipped — see the doc comment above.
        let _ = path;
    }
    Ok(())
}

/// Run one hook and return its findings plus non-fatal warnings.
///
/// Failures inside become warnings on the [`HookResult`] (never an abort of the
/// run).  `Err` is reserved for setup problems the caller cannot recover from
/// (missing script or interpreter).
pub fn run_hook(spec: &HookSpec, results: &serde_json::Value) -> Result<HookResult, HookError> {
    if !spec.script.is_file() {
        return Err(HookError::ScriptNotFound(spec.script.display().to_string()));
    }
    // Refuse a script we cannot trust before reading or hashing it (decision 23).
    check_hook_path_permissions(&spec.script)?;
    // Hash the script's bytes first: this digest is the code that actually ran.
    let script_bytes = std::fs::read(&spec.script)
        .map_err(|e| HookError::ReadScript(spec.script.display().to_string(), e.to_string()))?;
    let script_hash = sha256_hex(&script_bytes);
    let interpreter = discover_interpreter(spec)?;

    let payload = serde_json::to_vec(&results_payload(results))
        .map_err(|e| HookError::Runtime(e.to_string()))?;

    // A fresh temp working directory isolates the script from the caller's cwd
    // and from any stray files.
    let workdir = tempfile::tempdir().map_err(|e| HookError::Runtime(e.to_string()))?;

    // Copy the hashed bytes into the workdir and execute THAT copy, so the
    // recorded SHA-256 is exactly the code that ran (a script on disk cannot be
    // swapped between hashing and execution).  Keep the original file name for
    // the interpreter so `__file__` and tracebacks still look sensible.
    let script_name = spec
        .script
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "hook.py".to_string());
    let script_copy = workdir.path().join(&script_name);
    std::fs::write(&script_copy, &script_bytes).map_err(|e| HookError::Runtime(e.to_string()))?;

    let start = std::time::Instant::now();
    let mut warnings = Vec::new();
    let mut findings = Vec::new();

    // Refuse a payload at or over the input cap before spawning anything: we
    // never send truncated JSON to a script.
    if payload.len() >= spec.max_input_bytes {
        warnings.push(format!(
            "hook {} not run: results payload ({} bytes) exceeds the {}-byte input limit",
            spec.script.display(),
            payload.len(),
            spec.max_input_bytes
        ));
        return Ok(HookResult {
            script_hash,
            findings,
            warnings,
        });
    }

    match spawn_and_collect(
        interpreter,
        &script_copy,
        spec,
        workdir.path(),
        &payload,
        start,
    ) {
        Collect::Findings(f) => findings = f,
        Collect::BadOutput(captured) => {
            warnings.push(format!(
                "hook {} returned invalid output (stderr captured): {}",
                spec.script.display(),
                truncate_for_log(&captured)
            ));
        }
        Collect::TimedOut { limit_ms, stderr } => {
            warnings.push(format!(
                "hook {} timed out after {} ms",
                spec.script.display(),
                limit_ms
            ));
            if !stderr.is_empty() {
                warnings.push(format!(
                    "hook {} stderr: {}",
                    spec.script.display(),
                    truncate_for_log(&stderr)
                ));
            }
        }
        Collect::Spawn(e) => {
            warnings.push(format!(
                "hook {} could not be started: {}",
                spec.script.display(),
                e
            ));
        }
        Collect::ExitNoFindings { code, stderr } => {
            warnings.push(format!(
                "hook {} exited with status {:?} and no valid findings{}{}",
                spec.script.display(),
                code,
                if stderr.is_empty() { "" } else { "; stderr: " },
                truncate_for_log(&stderr)
            ));
        }
        Collect::Uncollected(stderr) => {
            warnings.push(format!(
                "hook {} output could not be fully collected (a child process kept stdout open)",
                spec.script.display()
            ));
            if !stderr.is_empty() {
                warnings.push(format!(
                    "hook {} stderr: {}",
                    spec.script.display(),
                    truncate_for_log(&stderr)
                ));
            }
        }
    }

    Ok(HookResult {
        script_hash,
        findings,
        warnings,
    })
}

/// SHA-256 hex digest of a byte slice.
fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(sha2::Sha256::digest(bytes))
}

/// Run every hook in order against the same payload.
pub fn run_hooks(
    specs: &[HookSpec],
    results: &serde_json::Value,
) -> Result<Vec<HookResult>, HookError> {
    let mut all = Vec::with_capacity(specs.len());
    for spec in specs {
        all.push(run_hook(spec, results)?);
    }
    Ok(all)
}

fn truncate_for_log(s: &str) -> String {
    const LIMIT: usize = 512;
    // Strip control and format/bidi characters (the same classification as
    // validate_finding) so untrusted hook stderr cannot inject control
    // sequences or spoof text in warnings and the report.
    let cleaned: String = s.chars().filter(|&c| !is_unsafe_char(c)).collect();
    let mut out: String = cleaned.chars().take(LIMIT).collect();
    if cleaned.chars().count() > LIMIT {
        out.push('…');
    }
    out
}

/// Validate a parsed finding; the first violation becomes an error string.
pub fn validate_finding(f: &Finding) -> Result<(), String> {
    if f.id.is_empty() {
        return Err("finding id is empty".to_string());
    }
    if f.id.len() > 256 {
        return Err("finding id too long".to_string());
    }
    if f.message.is_empty() {
        return Err("finding message is empty".to_string());
    }
    if f.message.len() > MAX_MESSAGE_BYTES {
        return Err("finding message too long".to_string());
    }
    if let Some(t) = &f.table {
        if t.len() > 512 {
            return Err("finding table name too long".to_string());
        }
    }
    if contains_control(&f.id)
        || contains_control(&f.message)
        || f.table.as_deref().is_some_and(contains_control)
    {
        return Err("finding contains control characters".to_string());
    }
    if contains_format_or_bidi(&f.id)
        || contains_format_or_bidi(&f.message)
        || f.table.as_deref().is_some_and(contains_format_or_bidi)
    {
        return Err("finding contains Unicode format/bidi characters".to_string());
    }
    Ok(())
}

/// True when a string holds a C0/C1 control character (newline, tab, escape,
/// any of `char::is_control`).
pub fn contains_control(s: &str) -> bool {
    s.chars().any(|c| c.is_control())
}

/// True when a string holds a Unicode format, bidi or other `Cf` (format)
/// character.  These are invisible and can reorder or spoof visible text, so
/// they are rejected from untrusted findings (see PLAN.md 6.3: script output is
/// untrusted input).  This covers U+200B–U+200F, U+202A–U+202E, U+2066–U+2069,
/// U+FEFF and every other `Cf` code point.
pub fn contains_format_or_bidi(s: &str) -> bool {
    s.chars().any(is_format_or_bidi_char)
}

/// True when one character is a Unicode format, bidi or other `Cf` code point.
/// The ranges mirror the comment on [`contains_format_or_bidi`].
fn is_format_or_bidi_char(c: char) -> bool {
    // \u{200B}..\u{200F}: zero-width space, ZW non-joiner/joiner, LRM, RLM.
    // \u{202A}..\u{202E}: LRE/RLE/PDF/LRO/RLO (bidi embedding/direction).
    // \u{2066}..\u{2069}: LRI/RLI/FSI/PDI (bidi isolation).
    matches!(c, '\u{200B}'..='\u{200F}'
        | '\u{202A}'..='\u{202E}'
        | '\u{2066}'..='\u{2069}'
        | '\u{FEFF}')
        || is_format_category(c)
}

/// True when a character must be stripped from untrusted text before it reaches
/// a warning or the report: a C0/C1 control character or a Unicode format/bidi
/// code point (the same classification [`validate_finding`] rejects).
fn is_unsafe_char(c: char) -> bool {
    c.is_control() || is_format_or_bidi_char(c)
}

fn is_format_category(c: char) -> bool {
    // General category `Cf` (Format).  Rust's std has no public category API,
    // so we match the specific ranges that make up `Cf`, plus the explicit
    // code points above, to be conservative and not reject legitimate scripts.
    // Cf = U+00AD, U+0600..05, U+061C, U+06DD, U+070F, U+0890..91, U+08E2,
    //      U+180E, U+200B..0F, U+202A..2E, U+2060..64, U+2066..6F,
    //      U+FEFF, U+FFF9..FB, U+110BD, U+110CD, U+13430..38, ...
    matches!(
        c,
        '\u{00AD}'                       // soft hyphen
            | '\u{061C}'                 // Arabic letter mark
            | '\u{180E}'                 // Mongolian vowel separator
            | '\u{2060}'..='\u{2064}'    // word joiner, invisible operators
            | '\u{FEFF}'                 // zero-width no-break space (BOM)
            | '\u{FFF9}'..='\u{FFFB}'    // interlinear annotation chars
    )
}

/// Escape a value before it goes into a spreadsheet cell, so a finding's text
/// cannot be interpreted as a CSV/formula injection.
///
/// Spreadsheet apps treat a cell starting with `=`, `+`, `-`, `@`, or a tab or
/// carriage return, as a formula or directive.  Prefix such a value with `'`
/// (a text marker) so it stays inert text.  This is the helper the report
/// build (T024) calls for every untrusted field it drops into a cell; it must
/// also quote values containing `,`, `"`, `\n` per its own CSV quoting rules.
pub fn sanitize_for_spreadsheet(value: &str) -> String {
    let first = value.chars().next();
    let needs_prefix =
        matches!(first, Some('=' | '+' | '-' | '@' | '\t' | '\r')) || value.starts_with('\u{FEFF}');
    if needs_prefix {
        let mut out = String::with_capacity(value.len() + 1);
        out.push('\'');
        out.push_str(value);
        out
    } else {
        value.to_string()
    }
}

enum Collect {
    Findings(Vec<Finding>),
    BadOutput(String),
    TimedOut {
        limit_ms: u64,
        stderr: String,
    },
    Spawn(String),
    ExitNoFindings {
        code: Option<i32>,
        stderr: String,
    },
    /// The script finished (or was killed) but its stdout could not be fully
    /// collected before the drain deadline — an escaped grandchild held the
    /// pipe open.  Treated as a hook error, never as findings.
    Uncollected(String),
}

/// Spawn the interpreter, feed it the payload, cap stdout/stderr, enforce the
/// time limit, and kill the whole process group (children included) on timeout.
fn spawn_and_collect(
    interpreter: PathBuf,
    script: &Path,
    spec: &HookSpec,
    workdir: &Path,
    payload: &[u8],
    start: std::time::Instant,
) -> Collect {
    let mut cmd = Command::new(&interpreter);
    // `-I` (isolated) ignores PYTHON* env vars and user site-packages; `-u`
    // (unbuffered) makes stdout flush so we can read findings as they come.
    // Both go BEFORE the script path so they are interpreter options, never
    // part of the script's argv.
    cmd.args(["-I", "-u"]);
    cmd.arg(script);
    cmd.current_dir(workdir);
    cmd.stdin(Stdio::piped());
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());
    cmd.env_clear();
    for (k, v) in allowed_env() {
        cmd.env(k, v);
    }

    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // A new process group so a timeout can kill every child a script forks.
        cmd.process_group(0);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        cmd.creation_flags(CREATE_NEW_PROCESS_GROUP);
    }

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => return Collect::Spawn(e.to_string()),
    };

    // Drain stdout and stderr on their own threads (bounded).  Each thread sends
    // its bytes over a channel when the pipe closes, so the main loop can see
    // "child + both pipes done" without ever blocking on a join while a child
    // that forked a grandchild keeps a pipe open.  These MUST start before the
    // stdin write below: a script that never reads stdin would otherwise block
    // us on a full pipe.
    let (stdout_tx, stdout_rx) = std::sync::mpsc::channel::<Vec<u8>>();
    let (stderr_tx, stderr_rx) = std::sync::mpsc::channel::<Vec<u8>>();
    // The reader threads are fully detached: we never join them (a grandchild
    // that escaped the group kill via setsid would otherwise block a join
    // forever holding the pipe open).  We only ever read what they deposit on
    // the channels before the drain deadline.
    spawn_reader(child.stdout.take(), MAX_OUTPUT_BYTES, stdout_tx);
    spawn_reader(child.stderr.take(), MAX_OUTPUT_BYTES, stderr_tx);

    // Write stdin on a detached thread so a full pipe (a script that never reads
    // its input) can never stall the parent; the time limit below still holds
    // and kills the whole group, which unblocks this writer with EPIPE.  We
    // never join this thread: if a grandchild keeps stdin open, joining would
    // hang long after the group is killed.  Dropping the handle detaches it, and
    // dropping `stdin` here (inside the thread) closes the child's stdin.
    let stdin_payload = payload.to_vec();
    let child_stdin = child.stdin.take();
    let _stdin_thread = std::thread::spawn(move || {
        use std::io::Write;
        if let Some(mut stdin) = child_stdin {
            let _ = stdin.write_all(&stdin_payload);
        }
    });

    let limit_ms = spec.timeout.as_millis() as u64;
    let mut status = None;

    loop {
        // Reap the parent's status when it exits.
        if status.is_none() {
            match child.try_wait() {
                Ok(Some(st)) => status = Some(st),
                Ok(None) => {}
                Err(_) => status = Some(std::process::ExitStatus::default()),
            }
        }

        // Time limit: kill the whole process group (children included).
        let over = start.elapsed() >= spec.timeout;

        if status.is_some() || over {
            // Once the parent has exited (or the deadline hit), kill the whole
            // process group so an escaped grandchild holding stdin/stdout cannot
            // keep the run alive, and the readers see EOF promptly.
            kill_group(&mut child);
            // Give the pipes a brief moment to close after the kill, then drain
            // whatever the readers captured.
            let dead = std::time::Instant::now() + Duration::from_millis(500);
            let mut stdout_bytes = None;
            let mut stderr_bytes = None;
            while std::time::Instant::now() < dead {
                if stdout_bytes.is_none() {
                    if let Ok(bytes) = stdout_rx.try_recv() {
                        stdout_bytes = Some(bytes);
                    }
                }
                if stderr_bytes.is_none() {
                    if let Ok(bytes) = stderr_rx.try_recv() {
                        stderr_bytes = Some(bytes);
                    }
                }
                if stdout_bytes.is_some() && stderr_bytes.is_some() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            let _ = child.wait();
            if over {
                let stderr = stderr_bytes.unwrap_or_default();
                return Collect::TimedOut {
                    limit_ms,
                    stderr: String::from_utf8_lossy(&stderr).into_owned(),
                };
            }
            let stderr_bytes = stderr_bytes.unwrap_or_default();
            let stderr = String::from_utf8_lossy(&stderr_bytes).into_owned();
            // If stdout never made it onto the channel in time (a grandchild
            // escaped the group kill and kept stdout open), treat it as a hook
            // error: we never run a script we could not observe to completion.
            let stdout_bytes = match stdout_bytes {
                Some(bytes) => bytes,
                None => return Collect::Uncollected(stderr),
            };
            return match parse_stdout(&stdout_bytes) {
                Ok(findings) => Collect::Findings(findings),
                Err(_) => {
                    let code = status.and_then(|s| s.code());
                    if code == Some(0) || code.is_none() {
                        Collect::BadOutput(stderr)
                    } else {
                        Collect::ExitNoFindings { code, stderr }
                    }
                }
            };
        }

        std::thread::sleep(Duration::from_millis(10));
    }
}

fn spawn_reader<T: std::io::Read + Send + 'static>(
    from: Option<T>,
    cap: usize,
    tx: std::sync::mpsc::Sender<Vec<u8>>,
) {
    // Detached thread: the handle is dropped so a stuck reader (a grandchild
    // that escaped the group kill and keeps a pipe open) can never be joined
    // and block the parent.  The channel receiver is drained with a deadline
    // instead.
    std::thread::spawn(move || {
        let mut out = Vec::new();
        if let Some(mut r) = from {
            let mut buf = [0u8; 8192];
            loop {
                match r.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        if out.len() < cap {
                            let take = n.min(cap - out.len());
                            out.extend_from_slice(&buf[..take]);
                        }
                    }
                    Err(_) => break,
                }
            }
        }
        let _ = tx.send(out);
    });
}

fn parse_stdout(bytes: &[u8]) -> Result<Vec<Finding>, ()> {
    let text = String::from_utf8_lossy(bytes);
    let parsed: HookOutput = serde_json::from_str(text.trim()).map_err(|_| ())?;
    let mut findings = Vec::with_capacity(parsed.findings.len());
    for f in parsed.findings {
        if validate_finding(&f).is_err() {
            return Err(());
        }
        findings.push(f);
    }
    Ok(findings)
}

/// Kill a process group (Unix) or a process tree (Windows), swallowing errors
/// since we are already on the teardown path.
#[cfg(unix)]
fn kill_group(child: &mut std::process::Child) {
    unsafe {
        let pid = child.id() as libc::pid_t;
        // Negative pid targets the process group; the child's pgid == its pid
        // because we passed `process_group(0)` at spawn.
        libc::kill(-pid, libc::SIGKILL);
    }
}

#[cfg(windows)]
fn kill_group(child: &mut std::process::Child) {
    let pid = child.id();
    let _ = Command::new("taskkill")
        .args(["/F", "/T", "/PID"])
        .arg(pid.to_string())
        .status();
}

#[cfg(test)]
mod tests {
    #[test]
    fn build_envelope_is_self_verifying_and_carries_evidence() {
        let res = HookResult {
            script_hash: "abc".into(),
            findings: vec![Finding {
                id: "f1".into(),
                severity: Severity::Fail,
                table: Some("s.t".into()),
                message: "bad".into(),
            }],
            warnings: vec!["slow".into()],
        };
        let env = build_envelope(
            &serde_json::json!({"verdict": "PASS"}),
            &[("check.py".to_string(), &res)],
        );
        assert!(verify_envelope(&env).is_ok());
        assert_eq!(env["hooks"]["scripts"][0]["sha256"], "abc");
        assert_eq!(env["hooks"]["custom_checks"][0]["source"], "custom_checks");
        assert_eq!(env["hooks"]["warnings"][0], "check.py: slow");
        let mut forged = env.clone();
        forged["hooks"]["custom_checks"] = serde_json::json!([]);
        assert!(verify_envelope(&forged).is_err());
    }

    use super::*;

    #[test]
    fn severity_round_trips() {
        assert_eq!("info".parse::<Severity>().unwrap(), Severity::Info);
        assert_eq!("warning".parse::<Severity>().unwrap(), Severity::Warning);
        assert_eq!("fail".parse::<Severity>().unwrap(), Severity::Fail);
        assert!("bogus".parse::<Severity>().is_err());
    }

    #[test]
    fn output_schema_denies_unknown_fields() {
        assert!(serde_json::from_value::<HookOutput>(serde_json::json!({
            "findings": [], "extra": 1
        }))
        .is_err());
        assert!(serde_json::from_value::<HookOutput>(serde_json::json!({
            "findings": []
        }))
        .is_ok());
    }

    #[test]
    fn allowed_env_has_no_secrets() {
        let env = allowed_env();
        for k in env.keys() {
            assert!(!is_denied_key(k), "sensitive key leaked into hook env: {k}");
        }
    }

    #[test]
    fn denied_keys_are_detected() {
        assert!(is_denied_key("PGPASSWORD"));
        assert!(is_denied_key("DMUX_SECRET"));
        assert!(is_denied_key("api_password"));
        assert!(is_denied_key("api_token"));
        assert!(!is_denied_key("table"));
        assert!(!is_denied_key("run_id"));
    }

    #[test]
    fn sensitive_keys_are_detected() {
        assert!(is_sensitive_key("password"));
        assert!(is_sensitive_key("PGPASSWORD"));
        assert!(is_sensitive_key("api_token"));
        assert!(is_sensitive_key("dsn"));
        assert!(is_sensitive_key("connection_string"));
        assert!(is_sensitive_key("missing_keys"));
        assert!(is_sensitive_key("extra_keys"));
        assert!(!is_sensitive_key("table"));
        assert!(!is_sensitive_key("run_id"));
    }

    #[test]
    fn control_chars_rejected_in_findings() {
        assert!(contains_control("a\nb"));
        assert!(!contains_control("hello"));
        let f = Finding {
            id: "x\n".into(),
            severity: Severity::Info,
            table: None,
            message: "m".into(),
        };
        assert!(validate_finding(&f).is_err());
    }

    #[test]
    fn message_length_bounded() {
        let f = Finding {
            id: "x".into(),
            severity: Severity::Info,
            table: None,
            message: "a".repeat(MAX_MESSAGE_BYTES + 1),
        };
        assert!(validate_finding(&f).is_err());
    }

    #[test]
    fn payload_scrubs_password_keys() {
        let v = serde_json::json!({
            "run_id": "r",
            "nested": {"password": "secret", "keep": "ok"}
        });
        let scrubbed = results_payload(&v);
        assert_eq!(scrubbed["run_id"], "r");
        assert!(scrubbed["nested"]["password"].is_null());
        assert_eq!(scrubbed["nested"]["keep"], "ok");
    }

    #[test]
    fn payload_drops_key_lists_even_with_export_keys_on() {
        let v = serde_json::json!({
            "run_id": "r",
            "export_keys": true,
            "tables": {
                "public.accounts": {
                    "verdict": "FAIL",
                    "missing_keys": [["137"], ["1902"]],
                    "extra_keys": [["42"]],
                    "changed_columns": ["balance"]
                }
            }
        });
        let scrubbed = results_payload(&v);
        assert!(scrubbed["tables"]["public.accounts"]["missing_keys"].is_null());
        assert!(scrubbed["tables"]["public.accounts"]["extra_keys"].is_null());
        // Non-key detail survives the scrub.
        assert_eq!(
            scrubbed["tables"]["public.accounts"]["changed_columns"][0],
            "balance"
        );
    }

    #[cfg(unix)]
    #[test]
    fn writable_script_is_refused_and_owner_only_is_accepted() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("dmux-hooks-perm-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("hook.py");
        std::fs::write(&path, "print('x')\n").unwrap();

        // Group- and world-writable: refused.
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o666)).unwrap();
        assert!(
            check_hook_path_permissions(&path).is_err(),
            "group/world-writable script must be refused"
        );

        // Owner-only: accepted.
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        assert!(check_hook_path_permissions(&path).is_ok());

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn truncate_for_log_strips_control_and_bidi() {
        let dirty = "ok\u{202E}evil\n\u{001B}[31mred";
        let cleaned = truncate_for_log(dirty);
        assert!(cleaned.contains("ok"));
        assert!(cleaned.contains("evil"));
        assert!(cleaned.contains("red"));
        assert!(!cleaned.contains('\u{202E}'));
        assert!(!cleaned.contains('\n'));
        assert!(!cleaned.contains('\u{001B}'));
    }

    #[test]
    fn format_and_bidi_characters_are_detected() {
        // Explicit bidi/format controls.
        for c in [
            '\u{200B}', '\u{200F}', '\u{202A}', '\u{202E}', '\u{2066}', '\u{2069}', '\u{FEFF}',
        ] {
            assert!(
                contains_format_or_bidi(&c.to_string()),
                "{c:?} should be rejected"
            );
        }
        // Other Cf category code points.
        assert!(contains_format_or_bidi("\u{00AD}")); // soft hyphen
        assert!(contains_format_or_bidi("\u{061C}")); // Arabic letter mark
        assert!(contains_format_or_bidi("\u{2060}")); // word joiner
                                                      // Plain text is fine.
        assert!(!contains_format_or_bidi("normal text"));
        assert!(!contains_format_or_bidi("café")); // non-ASCII letter, not Cf
    }

    #[test]
    fn finding_with_bidi_char_is_rejected() {
        let f = Finding {
            id: "x".into(),
            severity: Severity::Info,
            table: None,
            message: "evil\u{202E}payload".into(),
        };
        assert!(validate_finding(&f).is_err());
    }

    #[test]
    fn envelope_sha256_is_self_verifying() {
        let mut env = serde_json::json!({
            "results": {"verdict": "PASS"},
            "hooks": {"custom_checks": []}
        });
        let h = envelope_sha256(&env);
        env["envelope_sha256"] = serde_json::Value::String(h);
        assert!(verify_envelope(&env).is_ok());
    }

    #[test]
    fn envelope_sha256_detects_tampering() {
        let mut env = serde_json::json!({
            "results": {"verdict": "PASS"},
            "hooks": {"custom_checks": [{"id": "x", "severity": "fail"}]}
        });
        let h = envelope_sha256(&env);
        env["envelope_sha256"] = serde_json::Value::String(h);
        assert!(verify_envelope(&env).is_ok());

        // Editing a hook finding breaks verification.
        let mut tampered = env.clone();
        tampered["hooks"]["custom_checks"][0]["severity"] =
            serde_json::Value::String("info".to_string());
        assert!(verify_envelope(&tampered).is_err());

        // Editing the verdict breaks verification.
        let mut tampered2 = env.clone();
        tampered2["results"]["verdict"] = serde_json::Value::String("FAIL".to_string());
        assert!(verify_envelope(&tampered2).is_err());
    }

    #[test]
    fn spreadsheet_sanitizer_escapes_prefixes() {
        for (raw, expected) in [
            ("=SUM(A1)", "'=SUM(A1)"),
            ("+cmd", "'+cmd"),
            ("-cmd", "'-cmd"),
            ("@cmd", "'@cmd"),
            ("\tnasty", "'\tnasty"),
            ("\rreturn", "'\rreturn"),
            ("\u{FEFF}boom", "'\u{FEFF}boom"),
            ("safe value", "safe value"),
            ("-40", "'-40"), // negative numbers still need the text marker
        ] {
            assert_eq!(sanitize_for_spreadsheet(raw), expected, "for {raw:?}");
        }
    }
}
