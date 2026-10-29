//! `omadeckd status`: the deck's state as one line of Waybar-style JSON
//! (`{"text", "tooltip", "class"}`), for the Omarchy bar widget and Waybar. With `--follow` it
//! prints a new line whenever the state changes, and keeps going while the daemon restarts.

use std::path::Path;
use std::time::Duration;

use omadeck_core::HOME;
use omadeck_core::ipc::{self, Event};

/// Nerd Font "view-module": two rows of keys, like a Stream Deck.
const GLYPH: &str = "\u{f0573}";

/// What the bar shows. `Default` is "the service isn't running".
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Status {
    running: bool,
    /// The connected deck's model.
    deck: Option<String>,
    /// A deck that's plugged in but can't be opened.
    cant_open: Option<String>,
    page: Option<String>,
}

impl Status {
    pub fn apply(&mut self, ev: &Event) {
        match ev {
            Event::Connected { deck } => {
                self.running = true;
                self.deck = Some(deck.model.clone());
                self.cant_open = None;
            }
            Event::Disconnected => {
                self.running = true;
                self.deck = None;
                self.cant_open = None;
            }
            Event::CantOpen { model } => {
                self.running = true;
                self.deck = None;
                self.cant_open = Some(model.clone());
            }
            Event::PageChanged { page } => {
                self.running = true;
                self.page = page.clone();
            }
            _ => {}
        }
    }

    pub fn to_line(&self) -> String {
        let (class, tooltip) = match (&self.deck, self.running) {
            (_, false) => ("stopped", "omadeck: the service isn't running".to_string()),
            (None, true) if self.cant_open.is_some() => (
                "cant_open",
                format!(
                    "omadeck: can't open the {}. Replug it, or install the udev rule",
                    self.cant_open.as_deref().unwrap_or_default()
                ),
            ),
            (None, true) => ("disconnected", "omadeck: no Stream Deck connected".to_string()),
            (Some(model), true) => {
                ("connected", format!("omadeck: {model}, page {}", self.page.as_deref().unwrap_or(HOME)))
            }
        };
        serde_json::json!({ "text": GLYPH, "tooltip": tooltip, "class": class }).to_string()
    }
}

/// The status right now, from the daemon listening on `socket`.
pub fn once(socket: &Path) -> Status {
    let mut status = Status::default();
    if let Ok((_sender, events)) = ipc::connect_at(socket) {
        // A new client is sent the deck's state, then its page.
        for ev in events.take(2) {
            status.apply(&ev);
        }
    }
    status
}

/// Emit the status now and every time it changes, reconnecting every `retry` while the daemon
/// is away. Returns when `emit` returns false (whoever was reading has gone).
pub fn follow(socket: &Path, retry: Duration, mut emit: impl FnMut(&str) -> bool) {
    let mut last = String::new();
    let mut show = |status: &Status| -> bool {
        let line = status.to_line();
        if line == last {
            return true;
        }
        last = line;
        emit(&last)
    };
    loop {
        let mut status = Status::default();
        if let Ok((_sender, events)) = ipc::connect_at(socket) {
            let mut events = events.into_iter();
            // The greeting is two messages, the deck then its page: show them as one, so the
            // bar doesn't flash Home on the way to the real page.
            events.by_ref().take(2).for_each(|ev| status.apply(&ev));
            if !show(&status) {
                return;
            }
            for ev in events {
                status.apply(&ev);
                if !show(&status) {
                    return;
                }
            }
        }
        // The daemon went away, or isn't running yet.
        if !show(&Status::default()) {
            return;
        }
        std::thread::sleep(retry);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use omadeck_core::DeckInfo;
    use std::io::Write;
    use std::os::unix::net::UnixListener;
    use std::sync::mpsc;

    fn deck() -> DeckInfo {
        DeckInfo { model: "Stream Deck (V2)".into(), serial: "X".into(), rows: 3, cols: 5, key_size: 72 }
    }

    fn parse(line: &str) -> (String, String) {
        let v: serde_json::Value = serde_json::from_str(line).unwrap();
        assert_eq!(v["text"], GLYPH);
        (v["class"].as_str().unwrap().into(), v["tooltip"].as_str().unwrap().into())
    }

    #[test]
    fn status_lines_for_each_state() {
        let mut s = Status::default();
        assert_eq!(parse(&s.to_line()), ("stopped".into(), "omadeck: the service isn't running".into()));
        s.apply(&Event::Disconnected);
        assert_eq!(parse(&s.to_line()), ("disconnected".into(), "omadeck: no Stream Deck connected".into()));
        s.apply(&Event::Connected { deck: deck() });
        assert_eq!(parse(&s.to_line()), ("connected".into(), "omadeck: Stream Deck (V2), page Home".into()));
        s.apply(&Event::Disconnected);
        assert_eq!(parse(&s.to_line()).0, "disconnected", "unplugging is noticed");
        s.apply(&Event::CantOpen { model: "Stream Deck (V2)".into() });
        assert_eq!(
            parse(&s.to_line()),
            (
                "cant_open".into(),
                "omadeck: can't open the Stream Deck (V2). Replug it, or install the udev rule".into()
            )
        );
        s.apply(&Event::Connected { deck: deck() });
        assert_eq!(parse(&s.to_line()).0, "connected", "fixed");
    }

    #[test]
    fn page_changes_update_the_tooltip() {
        let mut s = Status::default();
        s.apply(&Event::Connected { deck: deck() });
        s.apply(&Event::PageChanged { page: Some("Media".into()) });
        assert_eq!(parse(&s.to_line()).1, "omadeck: Stream Deck (V2), page Media");
        s.apply(&Event::KeyDown { index: 3, page: None });
        assert_eq!(parse(&s.to_line()).1, "omadeck: Stream Deck (V2), page Media", "key presses change nothing");
    }

    /// A stand-in daemon on `socket`: sends `events` to one client, then goes away.
    fn daemon(socket: &Path, events: Vec<Event>) -> std::thread::JoinHandle<()> {
        let _ = std::fs::remove_file(socket);
        let listener = UnixListener::bind(socket).unwrap();
        let socket = socket.to_path_buf();
        std::thread::spawn(move || {
            let (mut client, _) = listener.accept().unwrap();
            for ev in events {
                client.write_all(ev.to_line().as_bytes()).unwrap();
            }
            drop(client);
            drop(listener);
            let _ = std::fs::remove_file(&socket);
        })
    }

    #[test]
    fn follow_prints_a_line_per_change_and_survives_a_daemon_restart() {
        let socket = std::env::temp_dir().join(format!("omadeck-test-{}-status.sock", std::process::id()));
        let first = daemon(
            &socket,
            vec![Event::Disconnected, Event::PageChanged { page: None }, Event::Connected { deck: deck() }],
        );
        let (tx, rx) = mpsc::channel();
        let path = socket.clone();
        std::thread::spawn(move || follow(&path, Duration::from_millis(20), |line| tx.send(line.to_string()).is_ok()));

        let next = || rx.recv_timeout(Duration::from_secs(5)).expect("no status line");
        assert_eq!(parse(&next()).0, "disconnected");
        assert_eq!(parse(&next()).0, "connected");
        first.join().unwrap();
        assert_eq!(parse(&next()).0, "stopped", "the daemon went away");

        // Its greeting: the deck, then the page it's on.
        let second =
            daemon(&socket, vec![Event::Connected { deck: deck() }, Event::PageChanged { page: Some("Media".into()) }]);
        assert_eq!(
            parse(&next()),
            ("connected".into(), "omadeck: Stream Deck (V2), page Media".into()),
            "came back after the restart, straight to the real page (no flash of Home)"
        );
        second.join().unwrap();
        drop(rx);
    }
}
