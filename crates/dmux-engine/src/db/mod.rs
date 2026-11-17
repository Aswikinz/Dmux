//! Database layer: connections, read-only transactions, server probing,
//! cancelling statements, and the canonical `SET LOCAL` block.
//!
//! Every engine query must go through [`readonly::with_readonly_txn`].

pub mod cancel;
pub mod config;
pub mod connect;
pub mod errors;
pub mod probe;
pub mod readonly;

pub use cancel::CancelToken;
pub use config::{ConnProfile, TlsMode};
pub use connect::connect;
pub use errors::DbError;
pub use probe::{probe_server, ServerInfo};
pub use readonly::with_readonly_txn;
