//! Talks to configurator windows over the Unix socket: broadcasts daemon events to all of
//! them and collects the commands they send.
//!
//! Each window gets its own writer thread, so publishing never waits on a socket and a
//! window that stops reading can't hold up the deck.

use std::io::Write;
use std::net::Shutdown;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, mpsc};

use anyhow::{Context, Result};
use omadeck_core::ipc::{self, Command, Event};

pub struct Hub {
    path: PathBuf,
    inner: Arc<Mutex<Inner>>,
    commands: mpsc::Receiver<Command>,
}

/// Events a window may fall behind by before it's disconnected. It reconnects, and the
/// replay brings it up to date.
const QUEUE: usize = 256;

struct Client {
    queue: mpsc::SyncSender<String>,
    /// Shut down to unblock the writer and tell the window it was dropped.
    stream: UnixStream,
}

impl Client {
    fn open(stream: UnixStream, hello: String) -> Option<Self> {
        let (queue, lines) = mpsc::sync_channel::<String>(QUEUE);
        let mut writer = stream.try_clone().ok()?;
        std::thread::Builder::new()
            .name("ipc-writer".into())
            .spawn(move || {
                for line in lines {
                    if writer.write_all(line.as_bytes()).is_err() {
                        break;
                    }
                }
                let _ = writer.shutdown(Shutdown::Both);
            })
            .ok()?;
        queue.try_send(hello).ok()?;
        Some(Self { queue, stream })
    }

    /// False when the window is gone or too far behind.
    fn send(&self, line: &str) -> bool {
        let ok = self.queue.try_send(line.to_string()).is_ok();
        if !ok {
            let _ = self.stream.shutdown(Shutdown::Both);
        }
        ok
    }
}

struct Inner {
    clients: Vec<Client>,
    /// Last connect/disconnect event, replayed to new clients.
    status: Event,
    /// Last page change, replayed to new clients.
    page: Event,
}

impl Hub {
    pub fn start(path: PathBuf) -> Result<Self> {
        let _ = std::fs::remove_file(&path);
        let listener = UnixListener::bind(&path).with_context(|| format!("binding {}", path.display()))?;
        let inner = Arc::new(Mutex::new(Inner {
            clients: Vec::new(),
            status: Event::Disconnected,
            page: Event::PageChanged { page: None },
        }));
        let (tx, commands) = mpsc::channel();
        let shared = inner.clone();
        std::thread::Builder::new().name("ipc".into()).spawn(move || {
            for stream in listener.incoming().flatten() {
                if let Ok(reader) = stream.try_clone() {
                    let tx = tx.clone();
                    let _ = std::thread::Builder::new().name("ipc-client".into()).spawn(move || {
                        for cmd in ipc::commands(reader) {
                            if tx.send(cmd).is_err() {
                                break;
                            }
                        }
                    });
                }
                let mut g = shared.lock().unwrap();
                let hello = g.status.to_line() + &g.page.to_line();
                if let Some(c) = Client::open(stream, hello) {
                    g.clients.push(c);
                }
            }
        })?;
        Ok(Self { path, inner, commands })
    }

    pub fn publish(&self, ev: Event) {
        let line = ev.to_line();
        let mut g = self.inner.lock().unwrap();
        match ev {
            Event::Connected { .. } | Event::Disconnected | Event::CantOpen { .. } => g.status = ev,
            Event::PageChanged { .. } => g.page = ev,
            _ => {}
        }
        g.clients.retain(|c| c.send(&line));
    }

    /// Commands received since the last call.
    pub fn commands(&self) -> mpsc::TryIter<'_, Command> {
        self.commands.try_iter()
    }

    pub fn shutdown(self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Read};
    use std::time::{Duration, Instant};

    fn socket(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("omadeck-test-{}-{name}.sock", std::process::id()))
    }

    fn wait_for_clients(hub: &Hub, n: usize) {
        let deadline = Instant::now() + Duration::from_secs(2);
        while hub.inner.lock().unwrap().clients.len() != n {
            assert!(Instant::now() < deadline, "client never registered");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn a_window_that_stops_reading_cannot_hold_up_the_deck() {
        let path = socket("stuck");
        let hub = Hub::start(path.clone()).unwrap();
        let mut stuck = UnixStream::connect(&path).unwrap();
        wait_for_clients(&hub, 1);

        // Far more than the socket buffer and the queue hold, and nobody reading.
        let start = Instant::now();
        for i in 0..20_000 {
            hub.publish(Event::ActionFailed { index: 0, message: format!("{i:0>200}") });
        }
        assert!(start.elapsed() < Duration::from_secs(1), "publishing took {:?}", start.elapsed());
        assert!(hub.inner.lock().unwrap().clients.is_empty(), "the stuck window is still subscribed");

        // The window is told it was dropped: reading ends, so it reconnects.
        stuck.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
        let mut sink = Vec::new();
        stuck.read_to_end(&mut sink).expect("the socket should close, not time out");
        hub.shutdown();
    }

    #[test]
    fn a_stuck_window_does_not_cost_the_others_their_events() {
        let path = socket("two");
        let hub = Hub::start(path.clone()).unwrap();
        let _stuck = UnixStream::connect(&path).unwrap();
        wait_for_clients(&hub, 1);
        let healthy = UnixStream::connect(&path).unwrap();
        wait_for_clients(&hub, 2);

        // The healthy window reads everything as it arrives.
        let reader = std::thread::spawn(move || {
            healthy.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
            BufReader::new(healthy)
                .lines()
                .map_while(Result::ok)
                .filter(|l| l.contains("\"key_up\""))
                .take(2_000)
                .count()
        });
        for i in 0..2_000 {
            hub.publish(Event::KeyUp { index: (i % 15) as u8 });
            if i % 100 == 0 {
                // Give the healthy reader a moment so only the stuck window falls behind.
                std::thread::sleep(Duration::from_millis(1));
            }
        }
        assert_eq!(reader.join().unwrap(), 2_000, "the healthy window missed events");
        hub.shutdown();
    }

    #[test]
    fn commands_from_a_window_reach_the_daemon() {
        let path = socket("cmd");
        let hub = Hub::start(path.clone()).unwrap();
        let mut window = UnixStream::connect(&path).unwrap();
        // What the configurator's `ipc::Sender` writes.
        window.write_all(b"{\"command\":\"show_page\",\"page\":\"Media\"}\n").unwrap();

        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            if let Some(Command::ShowPage { page }) = hub.commands().next() {
                assert_eq!(page.as_deref(), Some("Media"));
                break;
            }
            assert!(Instant::now() < deadline, "the command never arrived");
            std::thread::sleep(Duration::from_millis(5));
        }
        hub.shutdown();
    }

    #[test]
    fn a_deck_that_cant_be_opened_is_replayed_to_new_windows() {
        let path = socket("cantopen");
        let hub = Hub::start(path.clone()).unwrap();
        hub.publish(Event::CantOpen { model: "Stream Deck (V2)".into() });
        let stream = UnixStream::connect(&path).unwrap();
        stream.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
        let first = BufReader::new(stream).lines().next().unwrap().unwrap();
        assert_eq!(first, Event::CantOpen { model: "Stream Deck (V2)".into() }.to_line().trim_end());
        hub.shutdown();
    }

    #[test]
    fn new_windows_get_the_current_state() {
        let path = socket("hello");
        let hub = Hub::start(path.clone()).unwrap();
        hub.publish(Event::PageChanged { page: Some("Media".into()) });
        let stream = UnixStream::connect(&path).unwrap();
        wait_for_clients(&hub, 1);
        hub.publish(Event::KeyUp { index: 3 });

        stream.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
        let lines: Vec<String> = BufReader::new(stream).lines().take(3).map(Result::unwrap).collect();
        assert_eq!(lines[0], Event::Disconnected.to_line().trim_end());
        assert_eq!(lines[1], Event::PageChanged { page: Some("Media".into()) }.to_line().trim_end());
        assert_eq!(lines[2], Event::KeyUp { index: 3 }.to_line().trim_end());
        hub.shutdown();
    }
}
