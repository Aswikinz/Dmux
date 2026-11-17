//! Managed Tauri state shared by every command.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use crate::error::CommandError;
use crate::run_manager::RunManager;
use crate::secrets::{Keychain, PasswordStore};
use crate::store::Store;

/// App-wide state, managed once at startup.
pub struct AppState {
    /// The password store guarding secrets (the OS keychain in production).
    pub keychain: Mutex<Box<dyn PasswordStore>>,
    /// The SQLite profile store, opened at startup.
    pub store: Mutex<Store>,
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
        let store = Store::open(&data_dir.join("dmux.db"))?;
        Ok(Self {
            keychain: Mutex::new(keychain),
            store: Mutex::new(store),
            runs: Arc::new(RunManager::default()),
            data_dir,
        })
    }

    /// A guard over the profile store (locks the store mutex).
    pub fn store(&self) -> Result<std::sync::MutexGuard<'_, Store>, CommandError> {
        self.store
            .lock()
            .map_err(|_| CommandError::internal("store lock poisoned"))
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

/// The resume-store directory for the data half (the engine keeps its per-table
/// results in a SQLite file here, keyed by run definition so resume is safe).
pub fn resume_dir(state: &AppState) -> PathBuf {
    state.data_dir.clone()
}
