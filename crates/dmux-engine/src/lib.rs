//! Dmux engine: the shared comparison logic behind the desktop app and the
//! command-line runner.
//!
//! Phase 1 structure work: inventory, schema diff and mapping.

pub mod inventory;
pub mod mapping;
pub mod schema_diff;

/// The engine version, taken from the crate manifest at compile time.
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

#[cfg(test)]
mod tests {
    use super::version;

    #[test]
    fn version_is_the_package_version() {
        assert_eq!(version(), env!("CARGO_PKG_VERSION"));
    }

    #[test]
    fn version_is_not_empty() {
        assert!(!version().is_empty());
    }
}
