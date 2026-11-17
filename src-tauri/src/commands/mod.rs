//! The command handlers the frontend invokes.
//!
//! Three areas:
//! - [`profiles`]: save/list/delete connection profiles and their keychain
//!   passwords (never the password in SQLite).
//! - [`gateway`]: the structure-half engine operations — probe a connection,
//!   collect inventories, diff them, and build/export/import the mapping.
//! - [`data`]: the data-half engine operations — build a check plan, run,
//!   poll progress, pause/cancel, and fetch results (PLAN.md 4.4-4.7).

pub mod data;
pub mod gateway;
pub mod profiles;
