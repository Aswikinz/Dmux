//! In-progress run registry: one slot of state per background run.
//!
//! The engine's `run::run` is monolithic and has no run-level pause/cancel hook
//! (its [`dmux_engine::db::CancelToken`] cancels one *statement*, and is private
//! to the run internals).  The app therefore records a run's lifecycle itself:
//!
//! * [`RunHandle::task`] is the blocking task driving the engine.  **Cancel**
//!   aborts that task, which drops the engine's live sessions and their
//!   connections; Postgres cancels every in-flight statement of a dropped
//!   connection, so server queries genuinely stop.
//! * **Pause/Resume** moves the same handle between states.  Because the engine
//!   cannot suspend mid-run, "pause" aborts the task (marking the run paused);
//!   "resume" starts the *same* engine run again with the *same* resume store,
//!   so completed tables are reused (the engine's store re-checks every saved
//!   table against the counters/epoch/replica marks before reuse).  Nothing is
//!   ever fabricated: progress is read back from the engine's resume store.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;

use dmux_engine::results::RunResults;
use dmux_engine::run::RunConfig;
use tauri::async_runtime::JoinHandle;

/// The lifecycle state the UI reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunPhase {
    Running,
    Paused,
    Done,
    Cancelled,
}

/// A cancellable result handle for one run.
pub struct RunHandle {
    /// The definition's `RunConfig` (kept so resume can re-spawn it).
    pub config: RunConfig,
    /// The resume-store path the engine persists per-table results into.
    pub resume_store: Option<PathBuf>,
    /// The background task output, once finished.
    pub task: Option<JoinHandle<Result<RunResults, String>>>,
    pub phase: RunPhase,
}

/// The registry the Tauri commands read and update.
#[derive(Default)]
pub struct RunManager {
    runs: Mutex<HashMap<String, RunHandle>>,
}

impl RunManager {
    /// Register a new run and return its (app-level) run id.
    pub fn insert(&self, handle: RunHandle) -> String {
        let id = dmux_engine::results::fresh_run_id();
        self.runs
            .lock()
            .expect("run map poisoned")
            .insert(id.clone(), handle);
        id
    }

    /// A lock-scoped read of one run's phase.
    pub fn phase(&self, id: &str) -> Option<RunPhase> {
        self.runs
            .lock()
            .expect("run map poisoned")
            .get(id)
            .map(|h| h.phase)
    }

    /// Update a run's phase; true when the run exists.
    pub fn set_phase(&self, id: &str, phase: RunPhase) -> bool {
        match self.runs.lock().expect("run map poisoned").get_mut(id) {
            Some(h) => {
                h.phase = phase;
                true
            }
            None => false,
        }
    }

    /// Abort the in-flight drive task (cancel / pause).  No-op when there is no
    /// task or it has already finished.
    pub fn abort(&self, id: &str) -> bool {
        match self.runs.lock().expect("run map poisoned").get_mut(id) {
            Some(h) => {
                if let Some(task) = &h.task {
                    task.abort();
                }
                true
            }
            None => false,
        }
    }

    /// Read the cloneable pieces of a run (config + resume path), or `None`.
    pub fn snapshot(&self, id: &str) -> Option<(RunConfig, Option<PathBuf>)> {
        self.runs
            .lock()
            .expect("run map poisoned")
            .get(id)
            .map(|h| (h.config.clone(), h.resume_store.clone()))
    }

    /// Replace the drive task after a resume re-spawn.
    pub fn set_task(&self, id: &str, task: JoinHandle<Result<RunResults, String>>) -> bool {
        match self.runs.lock().expect("run map poisoned").get_mut(id) {
            Some(h) => {
                h.task = Some(task);
                true
            }
            None => false,
        }
    }

    /// Grab the finished task handle so the caller can await its result.
    pub fn take_task(&self, id: &str) -> Option<JoinHandle<Result<RunResults, String>>> {
        self.runs
            .lock()
            .expect("run map poisoned")
            .get_mut(id)
            .and_then(|h| h.task.take())
    }
}
