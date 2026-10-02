//! Tiny event feed from the daemon to the configurator: newline-delimited JSON over a
//! Unix socket at `$XDG_RUNTIME_DIR/omadeck.sock`.

use std::io::{BufRead, BufReader};
use std::os::unix::net::UnixStream;

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::device::DeckInfo;
use crate::paths;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum Event {
    /// Sent on connect and whenever a deck is attached.
    Connected {
        deck: DeckInfo,
    },
    /// Sent on connect when no deck is attached, and on unplug.
    Disconnected,
    KeyDown {
        index: u8,
    },
    KeyUp {
        index: u8,
    },
    /// An action failed to start.
    ActionFailed {
        index: u8,
        message: String,
    },
}

impl Event {
    pub fn to_line(&self) -> String {
        let mut s = serde_json::to_string(self).expect("event serializes");
        s.push('\n');
        s
    }
}

/// Connect to a running daemon and iterate its events. Ends when the daemon goes away.
pub fn subscribe() -> Result<impl Iterator<Item = Event>> {
    let stream = UnixStream::connect(paths::socket_path()?)?;
    Ok(BufReader::new(stream).lines().map_while(|l| l.ok()).filter_map(|l| serde_json::from_str(&l).ok()))
}
