//! The command handlers the frontend invokes.
//!
//! Two areas:
//! - [`profiles`]: save/list/delete connection profiles and their keychain
//!   passwords (never the password in SQLite).
//! - [`gateway`]: the structure-half engine operations — probe a connection,
//!   collect inventories, diff them, and build/export/import the mapping.

pub mod gateway;
pub mod profiles;
