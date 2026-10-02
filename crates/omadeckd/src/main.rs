//! omadeckd — owns the Stream Deck: paints key faces, runs actions on press, follows
//! config and Omarchy theme changes live, and survives unplug/replug.

mod hub;

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use notify::{RecursiveMode, Watcher};
use omadeck_core::device::{self, DeviceStateReader, DeviceStateUpdate, HidApi, Kind, StreamDeck};
use omadeck_core::ipc::Event;
use omadeck_core::{Config, DeckInfo, Renderer, action, paths};

use hub::Hub;

const SCAN_INTERVAL: Duration = Duration::from_secs(2);
const READ_TIMEOUT: Duration = Duration::from_millis(100);
const RELOAD_DEBOUNCE: Duration = Duration::from_millis(150);

struct Deck {
    dev: Arc<StreamDeck>,
    reader: Arc<DeviceStateReader>,
    kind: Kind,
    info: DeckInfo,
}

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).format_timestamp(None).init();

    let mut args = std::env::args().skip(1);
    match args.next().as_deref() {
        None | Some("run") => {}
        Some("list") => {
            let decks = device::detect();
            if decks.is_empty() {
                println!("No Stream Deck found.");
            }
            for d in decks {
                println!("{}  serial={}  {}x{} keys  {}px", d.model, d.serial, d.cols, d.rows, d.key_size);
            }
            return;
        }
        Some("press") => {
            let Some(i) = args.next().and_then(|v| v.parse::<u8>().ok()) else {
                eprintln!("usage: omadeckd press KEY   (keys are numbered from 0)");
                std::process::exit(2);
            };
            let cfg = load_config();
            let act = cfg.key(i).map(|k| k.action.clone()).unwrap_or_default();
            println!("key {i}: {}", act.summary());
            if let Err(e) = action::run(&act) {
                eprintln!("{e:#}");
                std::process::exit(1);
            }
            // Give the detached child a moment to start before we exit.
            std::thread::sleep(Duration::from_millis(300));
            return;
        }
        Some("snapshot") => {
            let out = args.next().unwrap_or_else(|| "omadeck.png".into());
            if let Err(e) = snapshot(Path::new(&out)) {
                eprintln!("snapshot failed: {e:#}");
                std::process::exit(1);
            }
            println!("wrote {out}");
            return;
        }
        Some("-V" | "--version") => {
            println!("omadeckd {}", env!("CARGO_PKG_VERSION"));
            return;
        }
        Some(_) => {
            eprintln!(
                "usage: omadeckd [run | list | press KEY | snapshot FILE.png | --version]\n\nRuns the omadeck Stream Deck daemon."
            );
            std::process::exit(2);
        }
    }

    if let Err(e) = run() {
        log::error!("{e:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    if std::os::unix::net::UnixStream::connect(paths::socket_path()).is_ok() {
        anyhow::bail!("another omadeckd is already running ({})", paths::socket_path().display());
    }

    let term = Arc::new(AtomicBool::new(false));
    for sig in [signal_hook::consts::SIGTERM, signal_hook::consts::SIGINT, signal_hook::consts::SIGHUP] {
        signal_hook::flag::register(sig, term.clone())?;
    }

    let hub = Hub::start(paths::socket_path())?;

    // Watch the config dir and Omarchy's theme state for changes.
    std::fs::create_dir_all(paths::config_dir())?;
    let (tx, rx) = mpsc::channel::<()>();
    let mut watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
        if let Ok(ev) = res
            && !matches!(ev.kind, notify::EventKind::Access(_))
            && ev.paths.iter().any(|p| relevant(p))
        {
            let _ = tx.send(());
        }
    })?;
    watcher.watch(&paths::config_dir(), RecursiveMode::NonRecursive)?;
    let omarchy = paths::omarchy_current_dir();
    if omarchy.is_dir() {
        watcher.watch(&omarchy, RecursiveMode::NonRecursive)?;
    }
    if paths::icons_dir().is_dir() {
        let _ = watcher.watch(&paths::icons_dir(), RecursiveMode::NonRecursive);
    }

    let mut cfg = load_config();
    let mut renderer = Renderer::new(&cfg.style);
    log::info!("theme: {}", renderer.theme.name);

    let mut hid = device::new_hidapi().context("initialising hidapi")?;
    let mut deck: Option<Deck> = None;
    let mut last_scan: Option<Instant> = None;
    let mut reload_at: Option<Instant> = None;

    log::info!("omadeckd {} ready; config {}", env!("CARGO_PKG_VERSION"), paths::config_file().display());

    while !term.load(Ordering::Relaxed) {
        // Coalesce bursts of filesystem events into one reload.
        while rx.try_recv().is_ok() {
            reload_at = Some(Instant::now() + RELOAD_DEBOUNCE);
        }
        if reload_at.is_some_and(|t| Instant::now() >= t) {
            reload_at = None;
            let new_cfg = load_config();
            renderer.update_style(&new_cfg.style);
            renderer.reload_theme();
            renderer.clear_cache();
            cfg = new_cfg;
            log::info!("reloaded config (theme: {})", renderer.theme.name);
            if let Some(d) = &deck
                && let Err(e) = paint_all(d, &cfg, &mut renderer)
            {
                log::warn!("repaint failed: {e}");
            }
        }

        let Some(d) = &deck else {
            if last_scan.is_none_or(|t| t.elapsed() >= SCAN_INTERVAL) {
                last_scan = Some(Instant::now());
                deck = connect(&mut hid, &cfg, &mut renderer);
                hub.publish(match &deck {
                    Some(d) => Event::Connected { deck: d.info.clone() },
                    None => Event::Disconnected,
                });
            } else {
                std::thread::sleep(READ_TIMEOUT);
            }
            continue;
        };

        match d.reader.read(Some(READ_TIMEOUT)) {
            Ok(updates) => {
                for u in updates {
                    match u {
                        DeviceStateUpdate::ButtonDown(i) => {
                            paint(d, &cfg, &mut renderer, i, true);
                            hub.publish(Event::KeyDown { index: i });
                            if let Some(k) = cfg.key(i) {
                                log::info!("key {i}: {}", k.action.summary());
                                if let Err(e) = action::run(&k.action) {
                                    log::warn!("key {i}: {e:#}");
                                    hub.publish(Event::ActionFailed { index: i, message: format!("{e:#}") });
                                }
                            }
                        }
                        DeviceStateUpdate::ButtonUp(i) => {
                            paint(d, &cfg, &mut renderer, i, false);
                            hub.publish(Event::KeyUp { index: i });
                        }
                        _ => {}
                    }
                }
            }
            Err(_) if term.load(Ordering::Relaxed) => break,
            Err(e) => {
                log::warn!("lost {} ({e}); waiting for it to come back", d.info.model);
                deck = None;
                last_scan = None;
                hub.publish(Event::Disconnected);
            }
        }
    }

    log::info!("shutting down");
    if let Some(d) = deck {
        // Hand the deck back to its idle logo screen.
        let _ = d.dev.reset();
    }
    hub.shutdown();
    Ok(())
}

fn relevant(p: &Path) -> bool {
    let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
    if p.starts_with(paths::omarchy_current_dir()) {
        return name == "theme.name" || name == "theme";
    }
    !name.starts_with('.')
}

fn load_config() -> Config {
    match Config::load() {
        Ok(c) => c,
        Err(e) => {
            log::error!("{e:#}; using an empty config");
            Config::default()
        }
    }
}

// elgato-streamdeck hands out its reader via `Arc<Self>`; everything stays on this thread.
#[allow(clippy::arc_with_non_send_sync)]
fn connect(hid: &mut HidApi, cfg: &Config, renderer: &mut Renderer) -> Option<Deck> {
    if let Err(e) = device::refresh_device_list(hid) {
        log::warn!("hidapi refresh: {e}");
        return None;
    }
    let (kind, serial) = device::list_devices(hid).into_iter().find(|(k, _)| k.is_visual())?;
    let dev = match StreamDeck::connect(hid, kind, &serial) {
        Ok(d) => Arc::new(d),
        Err(e) => {
            log::warn!("found {} but could not open it: {e} (is the udev rule installed?)", device::model_name(kind));
            return None;
        }
    };
    let reader = dev.get_reader();
    let deck = Deck { dev, reader, kind, info: DeckInfo::new(kind, &serial) };
    log::info!(
        "connected {} (serial {}, firmware {})",
        deck.info.model,
        serial,
        deck.dev.firmware_version().unwrap_or_default()
    );
    if let Err(e) = paint_all(&deck, cfg, renderer) {
        log::warn!("initial paint failed: {e}");
    }
    Some(deck)
}

fn paint_all(d: &Deck, cfg: &Config, renderer: &mut Renderer) -> Result<()> {
    d.dev.set_brightness(cfg.brightness)?;
    for i in 0..d.kind.key_count() {
        write_key(d, cfg, renderer, i, false)?;
    }
    d.dev.flush()?;
    Ok(())
}

fn paint(d: &Deck, cfg: &Config, renderer: &mut Renderer, i: u8, pressed: bool) {
    if let Err(e) = write_key(d, cfg, renderer, i, pressed).and_then(|_| Ok(d.dev.flush()?)) {
        log::warn!("paint key {i}: {e}");
    }
}

fn write_key(d: &Deck, cfg: &Config, renderer: &mut Renderer, i: u8, pressed: bool) -> Result<()> {
    let face = renderer.render(cfg.key(i), &cfg.style, d.info.key_size, pressed);
    d.dev.set_button_image(i, image::DynamicImage::ImageRgba8(face))?;
    Ok(())
}

/// Render the configured layout to a PNG, the way it looks on the hardware.
fn snapshot(out: &Path) -> Result<()> {
    let cfg = Config::load()?;
    let info = device::detect().into_iter().next().unwrap_or_else(DeckInfo::fallback);
    let mut renderer = Renderer::new(&cfg.style);
    let size = 144u32;
    let gap = 18u32;
    let (w, h) = (info.cols as u32 * (size + gap) + gap, info.rows as u32 * (size + gap) + gap);
    let mut canvas = image::RgbaImage::from_pixel(w, h, image::Rgba([10, 10, 12, 255]));
    for i in 0..info.key_count() {
        let face = renderer.render(cfg.key(i), &cfg.style, size, false);
        let (r, c) = (i as u32 / info.cols as u32, i as u32 % info.cols as u32);
        image::imageops::overlay(&mut canvas, &face, (gap + c * (size + gap)) as i64, (gap + r * (size + gap)) as i64);
    }
    canvas.save(out)?;
    Ok(())
}
