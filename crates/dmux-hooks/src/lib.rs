//! Python hooks (PLAN.md 4.9).
//!
//! A *hook* is a user-supplied Python script that reads the run's results as
//! JSON on stdin and writes extra findings, as JSON, on stdout.  The findings
//! appear in the report marked as custom checks.
//!
//! Scripts are **untrusted input** (PLAN.md 6.3 / AGENTS.md rule 2), so this
//! module treats them as hostile.  It guarantees:
//!
//! - **No secrets.** The child receives no inherited environment except a tiny
//!   allow-list (PATH, LANG, LC_ALL, and SYSTEMROOT on Windows).  `PG*`,
//!   `DMUX_*`, `*PASSWORD*`, `*TOKEN*` and `*SECRET*` are never passed through,
//!   and the results payload is scrubbed of password/token/connection keys.
//! - **No row data.** The results JSON carries counts, hashes and names only
//!   (PLAN.md 6.6); the payload writer scrubs any sensitive key as defence in
//!   depth.
//! - **A time limit.** The interpreter is killed when it exceeds the limit
//!   (default 60 s), along with any children, via a Unix process group (Windows:
//!   a new process group plus `taskkill /F /T`).
//! - **Bounded, validated output.** stdout is capped at [`MAX_OUTPUT_BYTES`]
//!   and parsed against a strict schema with `deny_unknown_fields`; every
//!   finding's message length and control characters are checked.
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

/// Defence-in-depth cap on the results payload written to a hook's stdin.
const STDIN_HEADER_BYTES: usize = 0x10_00_00;

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
}

impl Default for HookSpec {
    fn default() -> Self {
        Self {
            script: PathBuf::new(),
            interpreter: None,
            timeout: DEFAULT_TIMEOUT,
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

/// Serialise the results we hand a hook, scrubbed of any sensitive key.
///
/// The payload is deliberately small and secret-free (PLAN.md 6.6): run id,
/// tool version, mapping hash, verdict vocabulary, coverage line, per-table
/// verdicts with counts and reasons — never a password or a row value.  The
/// scrub pass is defence in depth against a future schema field.
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
}

/// Compute the SHA-256 hex digest of the script file's bytes (PLAN 4.9).
pub fn hash_script(path: &Path) -> Result<String, HookError> {
    let bytes = std::fs::read(path)
        .map_err(|e| HookError::ReadScript(path.display().to_string(), e.to_string()))?;
    Ok(hex::encode(sha2::Sha256::digest(&bytes)))
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

/// Run one hook and return its findings plus non-fatal warnings.
///
/// Failures inside become warnings on the [`HookResult`] (never an abort of the
/// run).  `Err` is reserved for setup problems the caller cannot recover from
/// (missing script or interpreter).
pub fn run_hook(spec: &HookSpec, results: &serde_json::Value) -> Result<HookResult, HookError> {
    if !spec.script.is_file() {
        return Err(HookError::ScriptNotFound(spec.script.display().to_string()));
    }
    let script_hash = hash_script(&spec.script)?;
    let interpreter = discover_interpreter(spec)?;

    // Resolve the script to an absolute path: the child runs in a fresh temp
    // working directory, so a relative script path would no longer resolve.
    let script_abs = spec
        .script
        .canonicalize()
        .unwrap_or_else(|_| spec.script.clone());

    let payload = serde_json::to_vec(&results_payload(results))
        .map_err(|e| HookError::Runtime(e.to_string()))?;

    // A fresh temp working directory isolates the script from the caller's cwd
    // and from any stray files.
    let workdir = tempfile::tempdir().map_err(|e| HookError::Runtime(e.to_string()))?;

    let start = std::time::Instant::now();
    let mut warnings = Vec::new();
    let mut findings = Vec::new();

    match spawn_and_collect(
        interpreter,
        &script_abs,
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
    }

    Ok(HookResult {
        script_hash,
        findings,
        warnings,
    })
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
    let mut out: String = s.chars().take(LIMIT).collect();
    if s.chars().count() > LIMIT {
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
    Ok(())
}

pub fn contains_control(s: &str) -> bool {
    s.chars().any(|c| c.is_control())
}

enum Collect {
    Findings(Vec<Finding>),
    BadOutput(String),
    TimedOut { limit_ms: u64, stderr: String },
    Spawn(String),
    ExitNoFindings { code: Option<i32>, stderr: String },
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
    cmd.arg(script);
    cmd.args(["-u"]);
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

    // Write stdin (bounded), then close it so the script sees EOF.
    {
        use std::io::Write;
        if let Some(mut stdin) = child.stdin.take() {
            let to_write = payload.len().min(STDIN_HEADER_BYTES);
            let _ = stdin.write_all(&payload[..to_write]);
        }
    }

    // Drain stdout and stderr on their own threads (bounded).  Each thread sends
    // its bytes over a channel when the pipe closes, so the main loop can see
    // "child + both pipes done" without ever blocking on a join while a child
    // that forked a grandchild keeps a pipe open.
    let (stdout_tx, stdout_rx) = std::sync::mpsc::channel::<Vec<u8>>();
    let (stderr_tx, stderr_rx) = std::sync::mpsc::channel::<Vec<u8>>();
    let stdout_thread = spawn_reader(child.stdout.take(), MAX_OUTPUT_BYTES, stdout_tx);
    let stderr_thread = spawn_reader(child.stderr.take(), MAX_OUTPUT_BYTES, stderr_tx);

    let limit_ms = spec.timeout.as_millis() as u64;
    let mut status = None;
    let mut stdout_bytes = None;
    let mut stderr_bytes = None;

    loop {
        // Reap the parent's status when it exits.
        if status.is_none() {
            match child.try_wait() {
                Ok(Some(st)) => status = Some(st),
                Ok(None) => {}
                Err(_) => status = Some(std::process::ExitStatus::default()),
            }
        }
        // Collect any finished reader.
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

        // Done when the parent exited and both pipes closed.
        if status.is_some() && stdout_bytes.is_some() && stderr_bytes.is_some() {
            let _ = stdout_thread.join();
            let _ = stderr_thread.join();
            break;
        }

        // Time limit: kill the whole process group (children included), then
        // drain whatever the readers have already captured.
        if start.elapsed() >= spec.timeout {
            kill_group(&mut child);
            // Give the pipes a moment to close after the kill.
            let deadline = std::time::Instant::now() + Duration::from_millis(500);
            while std::time::Instant::now() < deadline {
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
            let stderr = stderr_bytes.unwrap_or_default();
            return Collect::TimedOut {
                limit_ms,
                stderr: String::from_utf8_lossy(&stderr).into_owned(),
            };
        }

        std::thread::sleep(Duration::from_millis(10));
    }

    let stdout_bytes = stdout_bytes.unwrap_or_default();
    let stderr_bytes = stderr_bytes.unwrap_or_default();
    let stderr = String::from_utf8_lossy(&stderr_bytes).into_owned();

    match parse_stdout(&stdout_bytes) {
        Ok(findings) => Collect::Findings(findings),
        Err(_) => {
            let code = status.and_then(|s| s.code());
            if code == Some(0) || code.is_none() {
                Collect::BadOutput(stderr)
            } else {
                Collect::ExitNoFindings { code, stderr }
            }
        }
    }
}

fn spawn_reader<T: std::io::Read + Send + 'static>(
    from: Option<T>,
    cap: usize,
    tx: std::sync::mpsc::Sender<Vec<u8>>,
) -> std::thread::JoinHandle<()> {
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
    })
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
}
