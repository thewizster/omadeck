//! Broadcasts daemon events to any configurator windows connected over the Unix socket.

use std::io::Write;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context, Result};
use omadeck_core::ipc::Event;

pub struct Hub {
    path: PathBuf,
    inner: Arc<Mutex<Inner>>,
}

struct Inner {
    clients: Vec<UnixStream>,
    /// Last connect/disconnect event, replayed to new clients.
    status: Event,
}

impl Hub {
    pub fn start(path: PathBuf) -> Result<Self> {
        let _ = std::fs::remove_file(&path);
        let listener = UnixListener::bind(&path).with_context(|| format!("binding {}", path.display()))?;
        let inner = Arc::new(Mutex::new(Inner { clients: Vec::new(), status: Event::Disconnected }));
        let shared = inner.clone();
        std::thread::Builder::new().name("ipc".into()).spawn(move || {
            for stream in listener.incoming().flatten() {
                let _ = stream.set_write_timeout(Some(Duration::from_millis(200)));
                let mut g = shared.lock().unwrap();
                let mut stream = stream;
                if stream.write_all(g.status.to_line().as_bytes()).is_ok() {
                    g.clients.push(stream);
                }
            }
        })?;
        Ok(Self { path, inner })
    }

    pub fn publish(&self, ev: Event) {
        let line = ev.to_line();
        let mut g = self.inner.lock().unwrap();
        if matches!(ev, Event::Connected { .. } | Event::Disconnected) {
            g.status = ev;
        }
        g.clients.retain_mut(|c| c.write_all(line.as_bytes()).is_ok());
    }

    pub fn shutdown(self) {
        let _ = std::fs::remove_file(&self.path);
    }
}
