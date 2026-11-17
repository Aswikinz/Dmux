//! The command handlers the frontend invokes.
//!
//! Three areas:
//! - [`profiles`]: save/list/delete connection profiles and their keychain
//!   passwords (never the password in SQLite).
//! - [`gateway`]: the structure-half engine operations — probe a connection,
//!   collect inventories, diff them, and build/export/import the mapping.
//! - [`data`]: the data-half engine operations — build a check plan, run,
//!   poll progress, pause/cancel, and fetch results (PLAN.md 4.4-4.7).
//! - [`history`]: the local run history, opening a results file, the run log
//!   and purging history (PLAN.md 2.3, 4.8).
//! - [`report`]: export the finished run's PDF or Excel report through a
//!   native save dialog (PLAN.md 4.7).

pub mod data;
pub mod gateway;
pub mod history;
pub mod mapping_edit;
pub mod profiles;
pub mod report;
pub mod settings;
pub mod structure_view;
