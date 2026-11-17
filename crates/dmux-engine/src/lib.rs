//! Dmux engine: connections (db), inventory, schema diff, mapping, and the
//! comparison core (core/: normalise, fingerprint, compare, drilldown).

pub mod core;
pub mod db;
pub mod ident;
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
