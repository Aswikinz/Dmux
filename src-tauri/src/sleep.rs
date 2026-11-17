//! Keep the computer awake while a run is working (PLAN 2.3: "sleep is blocked
//! during a run").
//!
//! The inhibitor sits behind a trait so the run lifecycle is testable without
//! touching the real operating system.  A drive attempt takes a hold when it
//! starts and drops it when the engine returns, which covers done, failed,
//! cancelled and paused by the user (a pause stops the engine).  A failure to
//! take the hold never fails the run: the caller records a warning.

use std::any::Any;

/// Something that can ask the operating system not to sleep.
pub trait SleepInhibitor: Send + Sync {
    /// Take a hold.  Dropping the returned value releases it.  The error text
    /// is shown to the user, so it must be plain and carry no secrets.
    fn acquire(&self) -> Result<Box<dyn Any>, String>;
}

/// The real inhibitor: systemd/desktop inhibit locks on Linux,
/// `SetThreadExecutionState` on Windows, power assertions on macOS (the
/// `keepawake` crate).  The display may still turn off.
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemInhibitor;

impl SleepInhibitor for SystemInhibitor {
    fn acquire(&self) -> Result<Box<dyn Any>, String> {
        keepawake::Builder::default()
            .sleep(true)
            .idle(true)
            .reason("A Dmux check is running")
            .app_name("Dmux")
            .app_reverse_domain("verify.dmux.desktop")
            .create()
            .map(|k| Box::new(k) as Box<dyn Any>)
            .map_err(|_| "this computer could not be stopped from going to sleep".to_string())
    }
}

/// An inhibitor that holds nothing (tests and tools that must not touch the
/// real power manager).
#[derive(Debug, Default, Clone, Copy)]
pub struct NoInhibitor;

impl SleepInhibitor for NoInhibitor {
    fn acquire(&self) -> Result<Box<dyn Any>, String> {
        Ok(Box::new(()))
    }
}

/// The warning shown (and recorded in the results) when no hold could be
/// taken.
pub const SLEEP_WARNING: &str =
    "Dmux could not stop this computer from going to sleep during the check. \
     If the computer sleeps, the check will pause; keep it awake yourself.";

#[cfg(test)]
pub mod stub {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    /// A counting inhibitor that can be told to fail.
    #[derive(Debug, Default)]
    pub struct CountingInhibitor {
        pub acquired: AtomicUsize,
        pub released: Arc<AtomicUsize>,
        pub fail: bool,
    }

    struct Hold(Arc<AtomicUsize>);
    impl Drop for Hold {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }

    impl SleepInhibitor for CountingInhibitor {
        fn acquire(&self) -> Result<Box<dyn Any>, String> {
            if self.fail {
                return Err("no session bus".to_string());
            }
            self.acquired.fetch_add(1, Ordering::SeqCst);
            Ok(Box::new(Hold(Arc::clone(&self.released))))
        }
    }

    impl CountingInhibitor {
        pub fn held(&self) -> usize {
            self.acquired.load(Ordering::SeqCst) - self.released.load(Ordering::SeqCst)
        }
    }
}
