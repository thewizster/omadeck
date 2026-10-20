//! Release actions: armed when a key goes down, run when it comes back up.
//!
//! The action is captured at press time, so letting go runs what the pressed key asked for even
//! if the page changed or the config reloaded in between. Anything still held when the deck is
//! unplugged or the daemon stops is released then, so push-to-talk can't get stuck recording.

use std::collections::BTreeMap;

use omadeck_core::Action;

#[derive(Default)]
pub struct Hold {
    armed: BTreeMap<u8, Action>,
}

impl Hold {
    /// Key `key` went down; `on_release` is its release action, if any. Returns a release of the
    /// same key whose key-up never arrived, to run before this press.
    pub fn press(&mut self, key: u8, on_release: Option<&Action>) -> Option<Action> {
        let missed = self.armed.remove(&key);
        match on_release {
            None | Some(Action::None) => {}
            // Changing pages on release would fight the press: only the press navigates.
            Some(a) if a.is_navigation() => log::warn!("key {key}: a release action can't change pages; ignored"),
            Some(a) => {
                self.armed.insert(key, a.clone());
            }
        }
        missed
    }

    /// Key `key` came back up: its armed release, if any.
    pub fn release(&mut self, key: u8) -> Option<Action> {
        self.armed.remove(&key)
    }

    /// Every key still held, in key order, for when the deck goes away.
    pub fn drain(&mut self) -> Vec<(u8, Action)> {
        std::mem::take(&mut self.armed).into_iter().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app(cmd: &str) -> Action {
        Action::App { command: cmd.into() }
    }

    #[test]
    fn release_runs_the_action_armed_at_press_whatever_the_key_shows_now() {
        // The daemon looks up nothing on key-up: a page change or a reload while the key is
        // held can't swap in another key's release.
        let mut h = Hold::default();
        h.press(3, Some(&app("voxtype record stop")));
        assert_eq!(h.release(3), Some(app("voxtype record stop")));
        assert_eq!(h.release(3), None, "runs once");
    }

    #[test]
    fn keys_without_on_release_arm_nothing() {
        let mut h = Hold::default();
        assert_eq!(h.press(1, None), None);
        assert_eq!(h.press(2, Some(&Action::None)), None);
        assert_eq!(h.release(1), None);
        assert_eq!(h.release(2), None);
        assert!(h.drain().is_empty());
    }

    #[test]
    fn a_missed_key_up_releases_before_the_next_press() {
        let mut h = Hold::default();
        h.press(4, Some(&app("stop 1")));
        assert_eq!(h.press(4, Some(&app("stop 2"))), Some(app("stop 1")));
        assert_eq!(h.release(4), Some(app("stop 2")));
    }

    #[test]
    fn unplugging_releases_every_held_key() {
        let mut h = Hold::default();
        h.press(7, Some(&app("b")));
        h.press(2, Some(&app("a")));
        h.press(5, None);
        assert_eq!(h.drain(), vec![(2, app("a")), (7, app("b"))]);
        assert_eq!(h.release(2), None, "nothing left to run on a late key-up");
    }

    #[test]
    fn navigation_is_refused_as_a_release_action() {
        let mut h = Hold::default();
        h.press(0, Some(&Action::Back));
        h.press(1, Some(&Action::Page { page: "Media".into() }));
        assert_eq!(h.release(0), None);
        assert_eq!(h.release(1), None);
    }
}
