//! In-progress run registry: one slot of state per background run.
//!
//! A run is driven by one blocking task at a time (the *drive task*), running
//! the engine's `run::run` with a [`RunControl`] in its config.  The rules this
//! module enforces, all under one lock:
//!
//! * **Phases move one way.**  `Running -> Done | Failed | Cancelled | Paused`;
//!   a user resume moves `Paused -> Running`; a cancel may also end a paused
//!   run (`Paused -> Cancelled`).  `Done`, `Failed` and `Cancelled` are final.
//!   Every change is a compare-and-set ([`RunManager::transition`]), so nothing
//!   can overwrite a phase it did not expect.
//! * **The drive task cannot overwrite a stop.**  Each drive attempt gets a
//!   generation number.  [`RunManager::finish`] applies the engine's outcome
//!   only to the current generation and only while the run is still
//!   `Running`; a paused or cancelled run keeps its phase, and its outcome is
//!   discarded (a stopped run has no results).
//! * **Stopping is real.**  Pause and cancel call [`RunManager::stop`], which
//!   sets the engine's stop flag and returns the cancel tokens of every session
//!   the engine registered, so the caller cancels the server-side queries.
//! * **Never two drive tasks.**  [`RunManager::begin_resume`] flips
//!   `Paused -> Running`, installs a fresh control and hands back the previous
//!   task so the caller joins it before spawning the next; a second resume
//!   while one is in progress is refused.
//! * **Results only from success.**  [`RunManager::results`] returns the
//!   engine's results only when the phase is `Done` and the engine returned
//!   `Ok`.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};

use dmux_engine::results::RunResults;
use dmux_engine::run::{RunConfig, RunControl, RunError};
use tauri::async_runtime::JoinHandle;

use crate::error::{from_run_error, CommandError};

/// The lifecycle state the UI reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunPhase {
    Running,
    Paused,
    Done,
    Failed,
    Cancelled,
}

impl RunPhase {
    /// The transitions this module allows.
    pub fn may_become(self, to: RunPhase) -> bool {
        matches!(
            (self, to),
            (
                RunPhase::Running,
                RunPhase::Done | RunPhase::Failed | RunPhase::Cancelled | RunPhase::Paused
            ) | (RunPhase::Paused, RunPhase::Running | RunPhase::Cancelled)
        )
    }
}

/// One run.
pub struct RunHandle {
    /// The definition's `RunConfig` without a control (each drive attempt gets
    /// its own [`RunControl`]).
    pub config: RunConfig,
    /// The resume-store path the engine persists per-table results into.
    pub resume_store: Option<PathBuf>,
    phase: RunPhase,
    control: Arc<RunControl>,
    /// The current drive attempt.
    generation: u64,
    /// The generation whose drive task has returned (`finish` was called).
    finished_generation: Option<u64>,
    task: Option<JoinHandle<()>>,
    /// A resume is joining the previous task and has not spawned yet.
    resuming: bool,
    /// Set only by `finish` while `Running`: the engine's results or its error.
    outcome: Option<Result<RunResults, CommandError>>,
}

impl RunHandle {
    /// A new run in phase `Running` that has not been driven yet.
    pub fn new(config: RunConfig, resume_store: Option<PathBuf>) -> Self {
        Self {
            config,
            resume_store,
            phase: RunPhase::Running,
            control: Arc::new(RunControl::new()),
            generation: 0,
            finished_generation: None,
            task: None,
            resuming: false,
            outcome: None,
        }
    }
}

/// What a drive task needs: its generation and the config with its control.
pub struct DriveTicket {
    pub generation: u64,
    pub config: RunConfig,
}

/// Called once with a run's id and results when it becomes `Done` (used to
/// persist the run history).
pub type DoneHook = Arc<dyn Fn(&str, &RunResults) + Send + Sync>;

/// The registry the Tauri commands read and update.
#[derive(Default)]
pub struct RunManager {
    runs: Mutex<HashMap<String, RunHandle>>,
    on_done: Mutex<Option<DoneHook>>,
}

impl RunManager {
    fn lock(&self) -> MutexGuard<'_, HashMap<String, RunHandle>> {
        self.runs
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Install the hook that runs when a run finishes successfully.
    pub fn set_on_done(&self, hook: DoneHook) {
        *self
            .on_done
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(hook);
    }

    /// A fresh app-level run id (safe as a file name).
    pub fn fresh_id() -> String {
        dmux_engine::results::fresh_run_id()
    }

    /// Register a run under `id`.
    pub fn insert(&self, id: &str, handle: RunHandle) {
        self.lock().insert(id.to_string(), handle);
    }

    /// A run's phase.
    pub fn phase(&self, id: &str) -> Option<RunPhase> {
        self.lock().get(id).map(|h| h.phase)
    }

    /// Compare-and-set: move the run from one of `from` to `to`, if that is an
    /// allowed transition.  `Ok(previous)` on success, `Err(current)` (or
    /// `Err(None)` for an unknown run) otherwise.
    pub fn transition(
        &self,
        id: &str,
        from: &[RunPhase],
        to: RunPhase,
    ) -> Result<RunPhase, Option<RunPhase>> {
        let mut runs = self.lock();
        let Some(h) = runs.get_mut(id) else {
            return Err(None);
        };
        let now = h.phase;
        if from.contains(&now) && now.may_become(to) {
            h.phase = to;
            Ok(now)
        } else {
            Err(Some(now))
        }
    }

    /// Start the first drive attempt of a new run (phase must be `Running`).
    pub fn begin_first(&self, id: &str) -> Option<DriveTicket> {
        let mut runs = self.lock();
        let h = runs.get_mut(id)?;
        if h.phase != RunPhase::Running || h.generation != 0 {
            return None;
        }
        Some(Self::next_attempt(h))
    }

    fn next_attempt(h: &mut RunHandle) -> DriveTicket {
        h.generation += 1;
        h.control = Arc::new(RunControl::new());
        let mut config = h.config.clone();
        config.control = Some(Arc::clone(&h.control));
        DriveTicket {
            generation: h.generation,
            config,
        }
    }

    /// Resume a paused run: `Paused -> Running` with a fresh control and a new
    /// generation, atomically.  Returns the ticket and the previous drive task,
    /// which the caller must join before spawning (never two drive tasks).
    pub fn begin_resume(
        &self,
        id: &str,
    ) -> Result<(DriveTicket, Option<JoinHandle<()>>), CommandError> {
        let mut runs = self.lock();
        let Some(h) = runs.get_mut(id) else {
            return Err(CommandError::RunNotFound {
                run_id: id.to_string(),
            });
        };
        if h.resuming {
            return Err(CommandError::RunState {
                message: "the run is already being resumed".to_string(),
            });
        }
        if h.phase != RunPhase::Paused {
            return Err(CommandError::RunState {
                message: format!("only a paused run can resume (it is {:?})", h.phase),
            });
        }
        h.phase = RunPhase::Running;
        h.resuming = true;
        let previous = h.task.take();
        Ok((Self::next_attempt(h), previous))
    }

    /// The resume finished joining.  True when `generation` is still current
    /// and the run is still `Running`, so the caller should spawn it.
    pub fn end_resume(&self, id: &str, generation: u64) -> bool {
        let mut runs = self.lock();
        let Some(h) = runs.get_mut(id) else {
            return false;
        };
        h.resuming = false;
        h.generation == generation && h.phase == RunPhase::Running
    }

    /// Keep the drive task of `generation` (ignored for a stale generation).
    pub fn set_task(&self, id: &str, generation: u64, task: JoinHandle<()>) {
        if let Some(h) = self.lock().get_mut(id) {
            if h.generation == generation {
                h.task = Some(task);
            }
        }
    }

    /// Wait for the current drive task to return (no-op when there is none).
    #[cfg(test)]
    pub async fn join(&self, id: &str) {
        let task = self.lock().get_mut(id).and_then(|h| h.task.take());
        if let Some(task) = task {
            let _ = task.await;
        }
    }

    /// The drive task of `generation` reports the engine's outcome.  Applied
    /// only for the current generation while `Running`; returns the phase
    /// after the call.
    pub fn finish(
        &self,
        id: &str,
        generation: u64,
        outcome: Result<RunResults, RunError>,
    ) -> Option<RunPhase> {
        let mut runs = self.lock();
        let h = runs.get_mut(id)?;
        if h.finished_generation.is_none_or(|g| g < generation) {
            h.finished_generation = Some(generation);
        }
        let mut done: Option<RunResults> = None;
        if h.generation == generation && h.phase == RunPhase::Running {
            match outcome {
                Ok(results) => {
                    h.phase = RunPhase::Done;
                    done = Some(results.clone());
                    h.outcome = Some(Ok(results));
                }
                Err(e) => {
                    // An engine stop while Running cannot come from this
                    // manager (pause/cancel change the phase first); it is
                    // still never a success.
                    h.phase = RunPhase::Failed;
                    h.outcome = Some(Err(from_run_error(e)));
                }
            }
        }
        let phase = h.phase;
        drop(runs);
        if let Some(results) = done {
            let hook = self
                .on_done
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .clone();
            if let Some(hook) = hook {
                hook(id, &results);
            }
        }
        Some(phase)
    }

    /// Whether the drive task of `generation` has returned.
    pub fn drive_returned(&self, id: &str, generation: u64) -> bool {
        self.lock()
            .get(id)
            .and_then(|h| h.finished_generation)
            .is_some_and(|g| g >= generation)
    }

    /// Set the engine's stop flag for the current attempt.  Returns that
    /// attempt's generation and control: `control.tokens()` are the cancel
    /// tokens of every session the engine registered, to cancel their server
    /// queries (again, if needed, until the attempt's drive task returns).
    pub fn stop(&self, id: &str) -> Option<(u64, Arc<RunControl>)> {
        let runs = self.lock();
        let h = runs.get(id)?;
        h.control.stop();
        Some((h.generation, Arc::clone(&h.control)))
    }

    /// The error of a failed run.
    pub fn error(&self, id: &str) -> Option<CommandError> {
        match self.lock().get(id) {
            Some(h) if h.phase == RunPhase::Failed => match &h.outcome {
                Some(Err(e)) => Some(e.clone()),
                _ => Some(CommandError::RunFailed {
                    message: "the run failed without a recorded reason".to_string(),
                }),
            },
            _ => None,
        }
    }

    /// The engine's results: only for a `Done` run whose engine returned `Ok`.
    pub fn results(&self, id: &str) -> Result<RunResults, CommandError> {
        let runs = self.lock();
        let Some(h) = runs.get(id) else {
            return Err(CommandError::RunNotFound {
                run_id: id.to_string(),
            });
        };
        match (h.phase, &h.outcome) {
            (RunPhase::Done, Some(Ok(results))) => Ok(results.clone()),
            (RunPhase::Failed, Some(Err(e))) => Err(e.clone()),
            (RunPhase::Cancelled | RunPhase::Paused, _) => Err(CommandError::RunStopped {
                message: format!(
                    "the run is {}; a stopped run has no results",
                    if h.phase == RunPhase::Paused {
                        "paused"
                    } else {
                        "cancelled"
                    }
                ),
            }),
            _ => Err(CommandError::ResultsNotAvailable {
                message: "the run has not finished; results are not available yet".to_string(),
            }),
        }
    }

    /// The cloneable pieces of a run (config without control, resume path).
    pub fn snapshot(&self, id: &str) -> Option<(RunConfig, Option<PathBuf>)> {
        self.lock()
            .get(id)
            .map(|h| (h.config.clone(), h.resume_store.clone()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dmux_engine::run::Input;

    fn config() -> RunConfig {
        // Never connected: these tests drive the state machine only.
        let profile = |port| {
            Input::Live(dmux_engine::db::ConnProfile::new(
                "127.0.0.1",
                port,
                "unused",
                "unused",
                String::new(),
                dmux_engine::db::TlsMode::Disable("state machine test".to_string()),
            ))
        };
        RunConfig::new(profile(1), profile(1))
    }

    fn manager_with_run() -> (RunManager, String) {
        let m = RunManager::default();
        let id = RunManager::fresh_id();
        m.insert(&id, RunHandle::new(config(), None));
        (m, id)
    }

    fn results() -> RunResults {
        RunResults::default()
    }

    #[test]
    fn transitions_are_one_way() {
        use RunPhase::*;
        for to in [Done, Failed, Cancelled, Paused] {
            assert!(Running.may_become(to));
        }
        assert!(Paused.may_become(Running) && Paused.may_become(Cancelled));
        assert!(!Paused.may_become(Done) && !Paused.may_become(Failed));
        for from in [Done, Failed, Cancelled] {
            for to in [Running, Paused, Done, Failed, Cancelled] {
                assert!(!from.may_become(to), "{from:?} -> {to:?}");
            }
        }
    }

    #[test]
    fn a_cancelled_run_stays_cancelled_when_its_task_finishes() {
        // The gate's finding: the zombie task set Done unconditionally, so a
        // cancelled run could show PASS.
        let (m, id) = manager_with_run();
        let t = m.begin_first(&id).unwrap();
        assert!(t.config.control.is_some(), "the engine gets a stop switch");
        m.transition(
            &id,
            &[RunPhase::Running, RunPhase::Paused],
            RunPhase::Cancelled,
        )
        .unwrap();
        let (g, ctl) = m.stop(&id).unwrap();
        assert_eq!(g, t.generation);
        assert!(
            ctl.tokens().is_empty(),
            "no session registered in this test"
        );
        assert!(t.config.control.as_ref().unwrap().is_stopped());
        assert_eq!(
            m.finish(&id, t.generation, Ok(results())),
            Some(RunPhase::Cancelled)
        );
        assert!(matches!(
            m.results(&id),
            Err(CommandError::RunStopped { .. })
        ));
        // Nothing moves a final phase.
        assert!(m
            .transition(&id, &[RunPhase::Cancelled], RunPhase::Running)
            .is_err());
        assert_eq!(m.phase(&id), Some(RunPhase::Cancelled));
        assert!(m.drive_returned(&id, t.generation));
    }

    #[test]
    fn a_paused_run_stays_paused_and_has_no_results() {
        let (m, id) = manager_with_run();
        let t = m.begin_first(&id).unwrap();
        m.transition(&id, &[RunPhase::Running], RunPhase::Paused)
            .unwrap();
        m.stop(&id);
        assert_eq!(
            m.finish(&id, t.generation, Err(RunError::Cancelled)),
            Some(RunPhase::Paused)
        );
        assert!(matches!(
            m.results(&id),
            Err(CommandError::RunStopped { .. })
        ));
        assert!(m.error(&id).is_none());
    }

    #[test]
    fn an_engine_error_is_failed_with_the_typed_error() {
        let (m, id) = manager_with_run();
        let t = m.begin_first(&id).unwrap();
        let out = m.finish(
            &id,
            t.generation,
            Err(RunError::Db(dmux_engine::db::DbError::SuperuserRefused)),
        );
        assert_eq!(out, Some(RunPhase::Failed));
        assert!(matches!(m.error(&id), Some(CommandError::SuperuserRefused)));
        assert!(matches!(
            m.results(&id),
            Err(CommandError::SuperuserRefused)
        ));
        // A later cancel cannot turn a failure into anything else.
        assert!(m
            .transition(
                &id,
                &[RunPhase::Running, RunPhase::Paused],
                RunPhase::Cancelled
            )
            .is_err());
    }

    #[test]
    fn results_only_after_done_and_ok() {
        let (m, id) = manager_with_run();
        assert!(matches!(
            m.results(&id),
            Err(CommandError::ResultsNotAvailable { .. })
        ));
        let t = m.begin_first(&id).unwrap();
        assert!(m.begin_first(&id).is_none(), "one first attempt only");
        assert_eq!(
            m.finish(&id, t.generation, Ok(results())),
            Some(RunPhase::Done)
        );
        assert!(m.results(&id).is_ok());
        assert!(m.results(&id).is_ok(), "results can be read again");
        assert!(matches!(
            m.results("nope"),
            Err(CommandError::RunNotFound { .. })
        ));
    }

    #[test]
    fn a_stale_drive_task_cannot_touch_the_resumed_run() {
        let (m, id) = manager_with_run();
        let first = m.begin_first(&id).unwrap();
        m.transition(&id, &[RunPhase::Running], RunPhase::Paused)
            .unwrap();
        m.stop(&id);
        let (second, previous) = m.begin_resume(&id).unwrap();
        assert!(previous.is_none());
        assert!(second.generation > first.generation);
        assert!(
            !second.config.control.as_ref().unwrap().is_stopped(),
            "a resume gets a fresh stop switch"
        );
        assert!(first.config.control.as_ref().unwrap().is_stopped());
        // The paused attempt returns late (Cancelled by its stop): ignored.
        assert_eq!(
            m.finish(&id, first.generation, Err(RunError::Cancelled)),
            Some(RunPhase::Running)
        );
        assert!(m.end_resume(&id, second.generation));
        assert_eq!(
            m.finish(&id, second.generation, Ok(results())),
            Some(RunPhase::Done)
        );
    }

    #[test]
    fn resume_is_refused_unless_paused_and_never_twice() {
        let (m, id) = manager_with_run();
        let _t = m.begin_first(&id).unwrap();
        assert!(matches!(
            m.begin_resume(&id),
            Err(CommandError::RunState { .. })
        ));
        m.transition(&id, &[RunPhase::Running], RunPhase::Paused)
            .unwrap();
        let (t2, _) = m.begin_resume(&id).unwrap();
        // While the first resume is joining, a pause and a second resume must
        // not start another drive task.
        m.transition(&id, &[RunPhase::Running], RunPhase::Paused)
            .unwrap();
        assert!(matches!(
            m.begin_resume(&id),
            Err(CommandError::RunState { .. })
        ));
        assert!(
            !m.end_resume(&id, t2.generation),
            "paused again: do not spawn"
        );
        assert!(m.begin_resume(&id).is_ok(), "the next resume may proceed");
    }

    #[test]
    fn a_cancel_during_a_resume_stops_the_new_attempt() {
        let (m, id) = manager_with_run();
        let _t = m.begin_first(&id).unwrap();
        m.transition(&id, &[RunPhase::Running], RunPhase::Paused)
            .unwrap();
        let (t2, _) = m.begin_resume(&id).unwrap();
        m.transition(
            &id,
            &[RunPhase::Running, RunPhase::Paused],
            RunPhase::Cancelled,
        )
        .unwrap();
        m.stop(&id);
        assert!(t2.config.control.as_ref().unwrap().is_stopped());
        assert!(!m.end_resume(&id, t2.generation));
        assert_eq!(m.phase(&id), Some(RunPhase::Cancelled));
    }
}
