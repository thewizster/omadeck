//! Tiny protocol between the daemon and the configurator: newline-delimited JSON over a
//! Unix socket at `$XDG_RUNTIME_DIR/omadeck.sock`. The daemon sends events; clients may
//! send commands back.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::time::Duration;

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
    /// A deck is plugged in but can't be opened: usually the udev rule is missing, or another
    /// program has it. Sent on connect while that's the case.
    CantOpen {
        model: String,
    },
    /// Sent on connect and whenever the deck shows another page (`None` is home).
    PageChanged {
        page: Option<String>,
    },
    KeyDown {
        index: u8,
        /// The page the key was on.
        #[serde(default)]
        page: Option<String>,
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "command", rename_all = "snake_case")]
pub enum Command {
    /// Show a page on the deck (`None` is home), e.g. the one being edited.
    ShowPage { page: Option<String> },
}

fn line<T: Serialize>(v: &T) -> String {
    let mut s = serde_json::to_string(v).expect("message serializes");
    s.push('\n');
    s
}

impl Event {
    pub fn to_line(&self) -> String {
        line(self)
    }
}

/// The sending half of a connection to the daemon.
pub struct Sender(UnixStream);

impl Sender {
    pub fn send(&mut self, cmd: &Command) -> Result<()> {
        Ok(self.0.write_all(line(cmd).as_bytes())?)
    }
}

/// Connect to a running daemon: a sender for commands, and its events until it goes away.
pub fn connect() -> Result<(Sender, impl Iterator<Item = Event>)> {
    connect_at(&paths::socket_path()?)
}

/// `connect`, to the daemon listening on `socket`.
pub fn connect_at(socket: &std::path::Path) -> Result<(Sender, impl Iterator<Item = Event> + use<>)> {
    let stream = UnixStream::connect(socket)?;
    let writer = stream.try_clone()?;
    writer.set_write_timeout(Some(Duration::from_millis(200)))?;
    let events = BufReader::new(stream).lines().map_while(|l| l.ok()).filter_map(|l| serde_json::from_str(&l).ok());
    Ok((Sender(writer), events))
}

/// Read commands sent by a client until it disconnects.
pub fn commands(stream: UnixStream) -> impl Iterator<Item = Command> {
    BufReader::new(stream).lines().map_while(|l| l.ok()).filter_map(|l| serde_json::from_str(&l).ok())
}
