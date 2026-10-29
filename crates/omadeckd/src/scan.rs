//! Looking for a deck every couple of seconds: problems are logged when they change, not on
//! every scan. Without the udev rule, "found but could not open it" used to be logged every
//! 2 s, about 43,000 lines a day.

/// The last problem logged while scanning for a deck.
#[derive(Default)]
pub struct ScanLog {
    last: Option<String>,
}

impl ScanLog {
    /// A scan failed with `problem`. True when it should be logged: it's new, or different
    /// from the last one.
    pub fn failed(&mut self, problem: &str) -> bool {
        if self.last.as_deref() == Some(problem) {
            return false;
        }
        self.last = Some(problem.to_string());
        true
    }

    /// A scan worked (a deck opened, or simply none plugged in): the next problem is news.
    pub fn cleared(&mut self) {
        self.last = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NO_RULE: &str = "found Stream Deck (V2) but could not open it: permission denied";

    #[test]
    fn the_same_open_failure_is_logged_once() {
        let mut log = ScanLog::default();
        assert!(log.failed(NO_RULE));
        for _ in 0..1000 {
            assert!(!log.failed(NO_RULE), "the same problem again, 2 s later");
        }
    }

    #[test]
    fn a_new_failure_or_success_resets_the_log() {
        let mut log = ScanLog::default();
        assert!(log.failed(NO_RULE));
        assert!(log.failed("hidapi refresh: no such device"), "a different problem is news");
        assert!(log.failed(NO_RULE), "and so is the first one coming back");
        log.cleared();
        assert!(log.failed(NO_RULE), "after a success, the old problem is news again");
    }
}
