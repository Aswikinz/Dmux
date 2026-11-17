//! Managed Tauri state shared by every command.

use std::path::PathBuf;
use std::sync::Mutex;

use crate::error::CommandError;
use crate::secrets::Keychain;
use crate::store::Store;

/// App-wide state, managed once at startup.
pub struct AppState {
    /// The keychain (with its availability flag) guarding secrets.
    pub keychain: Mutex<Keychain>,
    /// The SQLite profile store, opened at startup.
    pub store: Mutex<Store>,
}

impl AppState {
    /// Open the profile store (at `data_dir/dmux.db`) and build the state.
    pub fn new(data_dir: PathBuf) -> Result<Self, CommandError> {
        let store = Store::open(&data_dir.join("dmux.db"))?;
        Ok(Self {
            keychain: Mutex::new(Keychain::default()),
            store: Mutex::new(store),
        })
    }

    /// A guard over the profile store (locks the store mutex).
    pub fn store(&self) -> Result<std::sync::MutexGuard<'_, Store>, CommandError> {
        self.store
            .lock()
            .map_err(|_| CommandError::internal("store lock poisoned"))
    }

    /// A guard over the keychain.
    pub fn keychain(&self) -> Result<std::sync::MutexGuard<'_, Keychain>, CommandError> {
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
