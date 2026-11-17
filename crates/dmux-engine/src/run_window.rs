//! The run window: an optional daily time range outside which a run waits
//! (PLAN 4.6, "a run window can be set, such as 22:00 to 05:00").
//!
//! The window is local clock time, to the minute.  It may cross midnight
//! (`22:00-05:00`).  The start minute is inside the window and the end minute is
//! outside it, so `22:00-05:00` runs from 22:00:00 up to 04:59:59.  The run
//! only ever waits between tables (see `run.rs`), never in the middle of a
//! scan, so no half-finished table result exists.

use std::fmt;

/// Minutes in a day.
const DAY: u32 = 24 * 60;

/// A daily window, as minutes after local midnight.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RunWindow {
    start: u32,
    end: u32,
}

/// Why a window text was refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RunWindowError {
    #[error("a run window looks like 22:00-05:00 (start-end, 24-hour clock); got \"{0}\"")]
    Format(String),
    #[error("a run window's start and end must be different times; got \"{0}\"")]
    Empty(String),
}

impl RunWindow {
    /// A window from two clock times.  `None` for an hour over 23, a minute over 59, or equal times.
    pub fn new(start_hour: u32, start_minute: u32, end_hour: u32, end_minute: u32) -> Option<Self> {
        if start_hour > 23 || end_hour > 23 || start_minute > 59 || end_minute > 59 {
            return None;
        }
        let w = Self {
            start: start_hour * 60 + start_minute,
            end: end_hour * 60 + end_minute,
        };
        (w.start != w.end).then_some(w)
    }

    /// Parse `HH:MM-HH:MM`.  The text is untrusted (a definition file).
    pub fn parse(text: &str) -> Result<Self, RunWindowError> {
        let bad = || RunWindowError::Format(text.chars().take(40).collect());
        let (a, b) = text.trim().split_once('-').ok_or_else(bad)?;
        let clock = |t: &str| -> Option<(u32, u32)> {
            let (h, m) = t.trim().split_once(':')?;
            let ok = |s: &str| (1..=2).contains(&s.len()) && s.bytes().all(|c| c.is_ascii_digit());
            if !ok(h) || m.len() != 2 || !ok(m) {
                return None;
            }
            Some((h.parse().ok()?, m.parse().ok()?))
        };
        let ((sh, sm), (eh, em)) = clock(a).zip(clock(b)).ok_or_else(bad)?;
        if sh > 23 || eh > 23 || sm > 59 || em > 59 {
            return Err(bad());
        }
        Self::new(sh, sm, eh, em)
            .ok_or_else(|| RunWindowError::Empty(text.chars().take(40).collect()))
    }

    /// Whether `minute_of_day` (0..1440, local time) is inside the window.
    pub fn contains(&self, minute_of_day: u32) -> bool {
        let m = minute_of_day % DAY;
        if self.start < self.end {
            m >= self.start && m < self.end
        } else {
            // Crosses midnight.
            m >= self.start || m < self.end
        }
    }

    /// The start time as `HH:MM` (when the window opens).
    pub fn opens_at(&self) -> String {
        format!("{:02}:{:02}", self.start / 60, self.start % 60)
    }

    /// Whole minutes from `minute_of_day` until the window next opens; 0 when
    /// it is open now.
    pub fn minutes_until_open(&self, minute_of_day: u32) -> u32 {
        let m = minute_of_day % DAY;
        if self.contains(m) {
            0
        } else {
            (self.start + DAY - m) % DAY
        }
    }
}

impl fmt::Display for RunWindow {
    /// The text form accepted by [`RunWindow::parse`].
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{:02}:{:02}-{:02}:{:02}",
            self.start / 60,
            self.start % 60,
            self.end / 60,
            self.end % 60
        )
    }
}

/// The local wall-clock minute of the day now (0..1440).  The OS time zone is
/// used; if it cannot be read, jiff falls back to UTC rather than failing.
pub fn local_minute_of_day() -> u32 {
    let now = jiff::Zoned::now();
    now.hour() as u32 * 60 + now.minute() as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    fn w(t: &str) -> RunWindow {
        RunWindow::parse(t).unwrap()
    }

    #[test]
    fn a_same_day_window_includes_its_start_and_excludes_its_end() {
        let win = w("09:00-17:30");
        assert!(!win.contains(8 * 60 + 59));
        assert!(win.contains(9 * 60));
        assert!(win.contains(12 * 60));
        assert!(win.contains(17 * 60 + 29));
        assert!(!win.contains(17 * 60 + 30));
        assert!(!win.contains(0));
    }

    #[test]
    fn a_window_may_cross_midnight() {
        let win = w("22:00-05:00");
        assert!(win.contains(22 * 60));
        assert!(win.contains(23 * 60 + 59));
        assert!(win.contains(0));
        assert!(win.contains(4 * 60 + 59));
        assert!(!win.contains(5 * 60));
        assert!(!win.contains(12 * 60));
        assert!(!win.contains(21 * 60 + 59));
    }

    #[test]
    fn minutes_until_the_window_opens() {
        let win = w("22:00-05:00");
        assert_eq!(win.minutes_until_open(23 * 60), 0);
        assert_eq!(win.minutes_until_open(5 * 60), 17 * 60);
        assert_eq!(win.minutes_until_open(21 * 60 + 30), 30);
        let day = w("09:00-10:00");
        assert_eq!(day.minutes_until_open(10 * 60), 23 * 60);
        assert_eq!(day.minutes_until_open(8 * 60), 60);
    }

    #[test]
    fn parsing_accepts_one_digit_hours_and_spaces_and_round_trips() {
        assert_eq!(w(" 9:05 - 17:00 ").to_string(), "09:05-17:00");
        assert_eq!(w("22:00-05:00").opens_at(), "22:00");
        assert_eq!(w(&w("23:59-00:01").to_string()), w("23:59-00:01"));
    }

    #[test]
    fn bad_windows_are_refused_with_the_expected_format() {
        for bad in [
            "",
            "22:00",
            "22:00-",
            "-05:00",
            "24:00-05:00",
            "22:60-05:00",
            "22:0-05:00",
            "ab:cd-ef:gh",
            "22:00-05:00-06:00",
            "22.00-05.00",
            "+2:00-05:00",
            "٢٢:00-05:00",
        ] {
            let e = RunWindow::parse(bad).unwrap_err();
            assert!(matches!(e, RunWindowError::Format(_)), "{bad:?} gave {e:?}");
            assert!(e.to_string().contains("22:00-05:00"));
        }
        assert!(matches!(
            RunWindow::parse("08:00-08:00").unwrap_err(),
            RunWindowError::Empty(_)
        ));
        assert!(RunWindow::new(24, 0, 5, 0).is_none());
        assert!(RunWindow::new(1, 0, 1, 0).is_none());
    }

    #[test]
    fn the_local_clock_is_a_minute_of_the_day() {
        assert!(local_minute_of_day() < DAY);
    }
}
