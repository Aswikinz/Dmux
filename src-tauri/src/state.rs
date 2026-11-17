//! Managed Tauri state shared by every command.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use crate::error::CommandError;
use crate::history::{self, Opened, OpenedResults, RunMetaTable};
use crate::run_manager::RunManager;
use crate::secrets::{Keychain, PasswordStore, SessionPasswords};
use crate::store::Store;

/// App-wide state, managed once at startup.
pub struct AppState {
    /// The password store guarding secrets (the OS keychain in production).
    pub keychain: Mutex<Box<dyn PasswordStore>>,
    /// Passwords kept in memory for this app session only, when the machine
    /// has no usable keychain (T055).  Never persisted or logged; cleared on
    /// exit.
    pub session_passwords: Mutex<SessionPasswords>,
    /// The SQLite profile store, opened at startup.
    pub store: Arc<Mutex<Store>>,
    /// Metadata of in-progress runs (start time, profile labels), used when a
    /// finished run is written to the history.
    pub run_meta: RunMetaTable,
    /// Verified results opened from history or a file, by fresh run id.
    pub opened: Mutex<OpenedResults>,
    /// The registry of in-progress runs (the data half).
    pub runs: Arc<RunManager>,
    /// The app data directory (home of the profile and resume stores).
    pub data_dir: PathBuf,
}

impl AppState {
    /// Open the profile store (at `data_dir/dmux.db`) and build the state with
    /// the OS-keychain password store, creating the app data directory first so
    /// the store always has a home.
    pub fn new(data_dir: PathBuf) -> Result<Self, CommandError> {
        Self::with_password_store(data_dir, Box::new(Keychain::default()))
    }

    /// Build the state with an injected password store (used by tests to avoid
    /// touching the real OS keychain).
    pub fn with_password_store(
        data_dir: PathBuf,
        keychain: Box<dyn PasswordStore>,
    ) -> Result<Self, CommandError> {
        std::fs::create_dir_all(&data_dir).map_err(|e| CommandError::Store {
            message: format!("cannot create app data dir: {e}"),
        })?;
        let store = Arc::new(Mutex::new(Store::open(&data_dir.join("dmux.db"))?));
        let run_meta: RunMetaTable = Arc::new(Mutex::new(HashMap::new()));
        let runs = Arc::new(RunManager::default());
        // A finished run is written to the local history (results file + row).
        {
            let store = Arc::clone(&store);
            let meta = Arc::clone(&run_meta);
            let dir = history::history_dir(&data_dir);
            runs.set_on_done(Arc::new(move |run_id, results| {
                let run_meta = meta
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .remove(run_id);
                // A failure to record history never changes the run's outcome.
                let _ = history::persist_finished(&store, &dir, run_meta.as_ref(), run_id, results);
            }));
        }
        Ok(Self {
            keychain: Mutex::new(keychain),
            session_passwords: Mutex::new(SessionPasswords::default()),
            store,
            run_meta,
            opened: Mutex::new(OpenedResults::default()),
            runs,
            data_dir,
        })
    }

    /// The folder of stored results files.
    pub fn history_dir(&self) -> PathBuf {
        history::history_dir(&self.data_dir)
    }

    /// Remember a run's start time and profile labels for its history row.
    pub fn note_run_start(&self, run_id: &str, old_label: &str, new_label: &str) {
        self.run_meta
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(
                run_id.to_string(),
                history::RunMeta {
                    started_at: history::now_secs(),
                    old_label: old_label.to_string(),
                    new_label: new_label.to_string(),
                },
            );
    }

    /// Register verified results under a fresh run id; returns the id.
    pub fn register_opened(&self, opened: Opened) -> Result<String, CommandError> {
        let id = history::fresh_opened_id();
        self.opened
            .lock()
            .map_err(|_| CommandError::internal("opened results lock poisoned"))?
            .insert(id.clone(), opened);
        Ok(id)
    }

    /// Verified results previously registered under `id`.
    pub fn opened_results(&self, id: &str) -> Result<Option<Opened>, CommandError> {
        Ok(self
            .opened
            .lock()
            .map_err(|_| CommandError::internal("opened results lock poisoned"))?
            .get(id))
    }

    /// A guard over the profile store (locks the store mutex).
    pub fn store(&self) -> Result<std::sync::MutexGuard<'_, Store>, CommandError> {
        self.store
            .lock()
            .map_err(|_| CommandError::internal("store lock poisoned"))
    }

    /// A guard over the session-only password cache.
    pub fn session_passwords(
        &self,
    ) -> Result<std::sync::MutexGuard<'_, SessionPasswords>, CommandError> {
        self.session_passwords
            .lock()
            .map_err(|_| CommandError::internal("session password lock poisoned"))
    }

    /// Drop every session password (called when the app exits).  A poisoned
    /// lock is recovered so the secrets are still zeroed.
    pub fn clear_session_passwords(&self) {
        match self.session_passwords.lock() {
            Ok(mut g) => g.clear(),
            Err(poisoned) => poisoned.into_inner().clear(),
        }
    }

    /// A guard over the password store.
    pub fn keychain(
        &self,
    ) -> Result<std::sync::MutexGuard<'_, Box<dyn PasswordStore>>, CommandError> {
        self.keychain
            .lock()
            .map_err(|_| CommandError::internal("keychain lock poisoned"))
    }
}

/// Derive the app data directory from Tauri's path resolver.
pub fn app_data_dir<R: tauri::Runtime>(app: &tauri::AppHandle<R>) -> PathBuf {
    use tauri::Manager;
    app.path()
        .app_data_dir()
        .expect("the OS must provide an app data dir")
}

/// The resume-store directory for the data half: `<app data>/resume/`.
pub fn resume_dir(state: &AppState) -> PathBuf {
    state.data_dir.join("resume")
}

/// The resume store of one run: its own SQLite file, `<app data>/resume/<run
/// id>.sqlite`, created by the engine.  Never a shared file: two runs (even of
/// the same definition) can never write into or resume from each other's
/// store.  `run_id` is an app-generated id (`run-<hex>-<hex>-<hex>`); anything
/// else is refused so it can never name a path outside the directory.
pub fn resume_store_path(state: &AppState, run_id: &str) -> Result<PathBuf, CommandError> {
    let safe = !run_id.is_empty()
        && run_id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-');
    if !safe {
        return Err(CommandError::internal("invalid run id for a resume store"));
    }
    let dir = resume_dir(state);
    std::fs::create_dir_all(&dir).map_err(|e| CommandError::Store {
        message: format!("cannot create the resume-store directory: {e}"),
    })?;
    Ok(dir.join(format!("{run_id}.sqlite")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_run_has_its_own_resume_store() {
        let dir = tempfile::tempdir().unwrap();
        let state = AppState::with_password_store(
            dir.path().to_path_buf(),
            Box::new(crate::secrets::InMemoryPasswordStore::new()),
        )
        .unwrap();
        let a = resume_store_path(&state, "run-1-2-3").unwrap();
        let b = resume_store_path(&state, "run-4-5-6").unwrap();
        assert_ne!(a, b);
        assert_eq!(a.parent().unwrap(), dir.path().join("resume"));
        assert!(a.parent().unwrap().is_dir());
        assert!(!a.ends_with("resume.db"));
        for bad in ["", "../x", "a/b", "run.db"] {
            assert!(resume_store_path(&state, bad).is_err(), "{bad}");
        }
    }
}
