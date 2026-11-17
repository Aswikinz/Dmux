//! Run and report defaults the Settings screen edits (T074).
//!
//! Each setting is stored in the app's SQLite store, has a sensible default,
//! is validated on the way in, and is read by the code that uses it:
//!
//! * `old_parallelism` / `new_parallelism`: how many scans run at once on each
//!   side.  `start_run` uses them when the request names none.
//! * `starting_level`: the check level a new plan starts from (the Choose
//!   checks screen applies it as its starting preset).
//! * `include_keys_default`: where the Report screen's "include row keys"
//!   box starts.  Off by default; the box stays an explicit choice.
//!
//! The engine has no configurable statement timeout, so there is no setting
//! for one.

use crate::error::CommandError;
use crate::store::Store;

pub const DEFAULT_OLD_PARALLELISM: usize = 2;
pub const DEFAULT_NEW_PARALLELISM: usize = 4;
pub const MIN_PARALLELISM: usize = 1;
pub const MAX_PARALLELISM: usize = 32;

const KEY_LEVEL: &str = "pref_starting_level";
const KEY_OLD_PAR: &str = "pref_old_parallelism";
const KEY_NEW_PAR: &str = "pref_new_parallelism";
const KEY_KEYS: &str = "pref_include_keys";

/// The levels a new plan may start from.
pub const STARTING_LEVELS: [&str; 4] = ["suggested", "structure", "counts", "full_values"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Prefs {
    pub starting_level: String,
    pub old_parallelism: usize,
    pub new_parallelism: usize,
    pub include_keys_default: bool,
}

impl Default for Prefs {
    fn default() -> Self {
        Self {
            starting_level: "suggested".to_string(),
            old_parallelism: DEFAULT_OLD_PARALLELISM,
            new_parallelism: DEFAULT_NEW_PARALLELISM,
            include_keys_default: false,
        }
    }
}

fn parallelism(raw: Option<String>, default: usize) -> usize {
    raw.and_then(|s| s.parse::<usize>().ok())
        .filter(|n| (MIN_PARALLELISM..=MAX_PARALLELISM).contains(n))
        .unwrap_or(default)
}

impl Prefs {
    /// Load, falling back to the default for anything missing or damaged.
    pub fn load(store: &Store) -> Result<Self, CommandError> {
        let level = store
            .get_setting(KEY_LEVEL)?
            .filter(|l| STARTING_LEVELS.contains(&l.as_str()))
            .unwrap_or_else(|| "suggested".to_string());
        Ok(Self {
            starting_level: level,
            old_parallelism: parallelism(store.get_setting(KEY_OLD_PAR)?, DEFAULT_OLD_PARALLELISM),
            new_parallelism: parallelism(store.get_setting(KEY_NEW_PAR)?, DEFAULT_NEW_PARALLELISM),
            include_keys_default: store.get_setting(KEY_KEYS)?.as_deref() == Some("true"),
        })
    }

    pub fn save_level(store: &Store, level: &str) -> Result<(), CommandError> {
        if !STARTING_LEVELS.contains(&level) {
            return Err(CommandError::InvalidInput {
                message: "unknown starting check level".to_string(),
            });
        }
        store.set_setting(KEY_LEVEL, Some(level))
    }

    pub fn save_parallelism(
        store: &Store,
        old: Option<usize>,
        new: Option<usize>,
    ) -> Result<(), CommandError> {
        for n in [old, new].into_iter().flatten() {
            if !(MIN_PARALLELISM..=MAX_PARALLELISM).contains(&n) {
                return Err(CommandError::InvalidInput {
                    message: format!(
                        "parallel scans must be between {MIN_PARALLELISM} and {MAX_PARALLELISM}"
                    ),
                });
            }
        }
        if let Some(n) = old {
            store.set_setting(KEY_OLD_PAR, Some(&n.to_string()))?;
        }
        if let Some(n) = new {
            store.set_setting(KEY_NEW_PAR, Some(&n.to_string()))?;
        }
        Ok(())
    }

    pub fn save_include_keys(store: &Store, on: bool) -> Result<(), CommandError> {
        store.set_setting(KEY_KEYS, Some(if on { "true" } else { "false" }))
    }
}

/// The parallel-scan limits for a run: the request's, else the stored ones.
pub fn effective_parallelism(
    requested_old: Option<usize>,
    requested_new: Option<usize>,
    prefs: &Prefs,
) -> (usize, usize) {
    (
        requested_old.unwrap_or(prefs.old_parallelism),
        requested_new.unwrap_or(prefs.new_parallelism),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> (tempfile::TempDir, Store) {
        let d = tempfile::tempdir().unwrap();
        let s = Store::open(&d.path().join("p.db")).unwrap();
        (d, s)
    }

    #[test]
    fn defaults_are_sensible() {
        let (_d, s) = store();
        let p = Prefs::load(&s).unwrap();
        assert_eq!(p, Prefs::default());
        assert_eq!((p.old_parallelism, p.new_parallelism), (2, 4));
        assert!(!p.include_keys_default, "row keys stay opt-in");
        assert_eq!(p.starting_level, "suggested");
    }

    #[test]
    fn values_persist_and_bad_ones_are_refused() {
        let (_d, s) = store();
        Prefs::save_level(&s, "counts").unwrap();
        Prefs::save_parallelism(&s, Some(3), Some(8)).unwrap();
        Prefs::save_include_keys(&s, true).unwrap();
        let p = Prefs::load(&s).unwrap();
        assert_eq!(p.starting_level, "counts");
        assert_eq!((p.old_parallelism, p.new_parallelism), (3, 8));
        assert!(p.include_keys_default);
        assert!(Prefs::save_level(&s, "everything").is_err());
        assert!(Prefs::save_parallelism(&s, Some(0), None).is_err());
        assert!(Prefs::save_parallelism(&s, None, Some(MAX_PARALLELISM + 1)).is_err());
        // A refused update changed nothing.
        assert_eq!(Prefs::load(&s).unwrap().old_parallelism, 3);
    }

    #[test]
    fn damaged_stored_values_fall_back_to_defaults() {
        let (_d, s) = store();
        s.set_setting(KEY_OLD_PAR, Some("0")).unwrap();
        s.set_setting(KEY_NEW_PAR, Some("many")).unwrap();
        s.set_setting(KEY_LEVEL, Some("bogus")).unwrap();
        assert_eq!(Prefs::load(&s).unwrap(), Prefs::default());
    }

    #[test]
    fn a_run_uses_the_stored_limits_unless_the_request_names_its_own() {
        let prefs = Prefs {
            old_parallelism: 3,
            new_parallelism: 6,
            ..Prefs::default()
        };
        assert_eq!(effective_parallelism(None, None, &prefs), (3, 6));
        assert_eq!(effective_parallelism(Some(1), None, &prefs), (1, 6));
    }
}
