//! omadeckd — owns the Stream Deck: paints key faces, runs actions on press, follows
//! config and Omarchy theme changes live, and survives unplug/replug.

mod hold;
mod hub;
mod nav;
mod scan;
mod status;
mod worker;

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use notify::{RecursiveMode, Watcher};
use omadeck_core::device::{self, DeviceStateReader, DeviceStateUpdate, HidApi, Kind, StreamDeck};
use omadeck_core::ipc::{Command, Event};
use omadeck_core::paths::FileStamp;
use omadeck_core::{Action, Config, DeckInfo, Renderer, action, paths};

use hold::Hold;
use hub::Hub;
use nav::Nav;
use scan::ScanLog;
use worker::Worker;

const SCAN_INTERVAL: Duration = Duration::from_secs(2);
const READ_TIMEOUT: Duration = Duration::from_millis(100);
const RELOAD_DEBOUNCE: Duration = Duration::from_millis(150);
/// How often to double-check the config by stat, for changes inotify can't see.
const STAMP_INTERVAL: Duration = Duration::from_secs(1);
/// How long a key's release action gets to finish when the daemon stops.
const RELEASE_AT_SHUTDOWN: Duration = Duration::from_secs(2);

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
                eprintln!("usage: omadeckd press KEY [PAGE]   (keys are numbered from 0)");
                std::process::exit(2);
            };
            let page = args.next();
            let cfg = load_config();
            let Some(keys) = cfg.page(page.as_deref()) else {
                eprintln!("no page named {}", page.unwrap_or_default());
                std::process::exit(1);
            };
            let act = keys.get(i).map(|k| k.action.clone()).unwrap_or_default();
            println!("key {i}: {}", act.summary());
            if let Err(e) = action::run(&act) {
                eprintln!("{e:#}");
                std::process::exit(1);
            }
            // A press from the command line is a tap: let go at once, so nothing stays held.
            if let Some(release) = keys.get(i).and_then(|k| k.on_release.clone()).filter(|a| !a.is_navigation()) {
                println!("key {i} released: {}", release.summary());
                if let Err(e) = action::run(&release) {
                    eprintln!("{e:#}");
                    std::process::exit(1);
                }
            }
            // Give the detached child a moment to start before we exit.
            std::thread::sleep(Duration::from_millis(300));
            return;
        }
        Some("snapshot") => {
            let out = args.next().unwrap_or_else(|| "omadeck.png".into());
            if let Err(e) = snapshot(Path::new(&out), args.next().as_deref()) {
                eprintln!("snapshot failed: {e:#}");
                std::process::exit(1);
            }
            println!("wrote {out}");
            return;
        }
        Some("status") => {
            let follow = args.next().as_deref() == Some("--follow");
            let socket = match paths::socket_path() {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("{e:#}");
                    std::process::exit(1);
                }
            };
            if follow {
                use std::io::Write;
                let mut out = std::io::stdout();
                status::follow(&socket, SCAN_INTERVAL, |line| writeln!(out, "{line}").is_ok());
            } else {
                println!("{}", status::once(&socket).to_line());
            }
            return;
        }
        Some("-V" | "--version") => {
            println!("omadeckd {}", env!("CARGO_PKG_VERSION"));
            return;
        }
        Some(_) => {
            eprintln!(
                "usage: omadeckd [run | list | press KEY [PAGE] | snapshot FILE.png [PAGE] | status [--follow] | --version]\n\nRuns the omadeck Stream Deck daemon."
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
    let socket = paths::socket_path()?;
    if std::os::unix::net::UnixStream::connect(&socket).is_ok() {
        anyhow::bail!("another omadeckd is already running ({})", socket.display());
    }

    let term = Arc::new(AtomicBool::new(false));
    for sig in [signal_hook::consts::SIGTERM, signal_hook::consts::SIGINT, signal_hook::consts::SIGHUP] {
        signal_hook::flag::register(sig, term.clone())?;
    }

    let hub = Hub::start(socket)?;
    let worker = Worker::start().context("starting the action worker")?;
    let mut hold = Hold::default();

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
    let mut dir_stamp = FileStamp::of(&paths::config_dir());
    let mut cfg_stamp = FileStamp::of(&paths::config_file());
    let mut last_stamp_check = Instant::now();
    let omarchy = paths::omarchy_current_dir();
    if omarchy.is_dir() {
        watcher.watch(&omarchy, RecursiveMode::NonRecursive)?;
    }
    if paths::icons_dir().is_dir() {
        let _ = watcher.watch(&paths::icons_dir(), RecursiveMode::NonRecursive);
    }

    let mut cfg = load_config();
    let mut renderer = Renderer::new(&cfg.style);
    let mut nav = Nav::default();
    // A page the configurator asked for that isn't in the config yet (it saves a moment later).
    let mut wanted_page: Option<String> = None;
    log::info!("theme: {}", renderer.theme.name);

    let mut hid = device::new_hidapi().context("initialising hidapi")?;
    let mut deck: Option<Deck> = None;
    let mut last_scan: Option<Instant> = None;
    let mut reload_at: Option<Instant> = None;
    let mut scan_log = ScanLog::default();

    log::info!("omadeckd {} ready; config {}", env!("CARGO_PKG_VERSION"), paths::config_file().display());

    while !term.load(Ordering::Relaxed) {
        // Coalesce bursts of filesystem events into one reload.
        while rx.try_recv().is_ok() {
            reload_at = Some(Instant::now() + RELOAD_DEBOUNCE);
        }
        // inotify misses a symlinked config's target, and its watch dies with a config
        // directory that's deleted and recreated (restoring a backup), so also compare stamps.
        if last_stamp_check.elapsed() >= STAMP_INTERVAL {
            last_stamp_check = Instant::now();
            let d = FileStamp::of(&paths::config_dir());
            // The directory's mtime moves on every save; only its identity matters here.
            if !FileStamp::same_file(d, dir_stamp) {
                dir_stamp = d;
                let _ = watcher.unwatch(&paths::config_dir());
                if d.is_some() {
                    log::info!("config directory was replaced; watching the new one");
                    let _ = watcher.watch(&paths::config_dir(), RecursiveMode::NonRecursive);
                    if paths::icons_dir().is_dir() {
                        let _ = watcher.unwatch(&paths::icons_dir());
                        let _ = watcher.watch(&paths::icons_dir(), RecursiveMode::NonRecursive);
                    }
                }
            }
            let c = FileStamp::of(&paths::config_file());
            if c != cfg_stamp {
                cfg_stamp = c;
                reload_at.get_or_insert(Instant::now());
            }
            // `omarchy font set` only rewrites fontconfig's user config.
            if renderer.font_changed() {
                reload_at.get_or_insert(Instant::now());
            }
        }
        if reload_at.is_some_and(|t| Instant::now() >= t) {
            reload_at = None;
            // A typo while hand-editing shouldn't wipe the deck: keep the last good config.
            match Config::load() {
                Ok(new_cfg) => cfg = new_cfg,
                Err(e) => log::error!("{e:#}; keeping the previous config"),
            }
            renderer.update_style(&cfg.style);
            renderer.reload_theme();
            renderer.clear_cache();
            log::info!("reloaded config (theme: {})", renderer.theme.name);
            let mut moved = nav.retain_existing(&cfg);
            if let Some(p) = wanted_page.take_if(|p| cfg.has_page(p)) {
                moved |= nav.go(Some(&p));
            }
            if moved {
                hub.publish(Event::PageChanged { page: nav.current().map(str::to_string) });
            }
            if let Some(d) = &deck
                && let Err(e) = paint_all(d, &cfg, nav.current(), &mut renderer)
            {
                log::warn!("repaint failed: {e}");
            }
        }

        // Only the latest page request counts, so flicking through pages in the configurator
        // doesn't echo a trail of stale page changes back to it.
        let mut show = None;
        for cmd in hub.commands() {
            match cmd {
                Command::ShowPage { page } => show = Some(page),
            }
        }
        if let Some(page) = show {
            wanted_page = None;
            match page {
                Some(p) if !cfg.has_page(&p) => wanted_page = Some(p),
                page => {
                    if nav.go(page.as_deref()) {
                        show_page(deck.as_ref(), &cfg, &nav, &mut renderer, &hub);
                    }
                }
            }
        }

        for (i, message) in worker.failures() {
            log::warn!("key {i}: {message}");
            hub.publish(Event::ActionFailed { index: i, message });
        }

        let Some(d) = &deck else {
            if last_scan.is_none_or(|t| t.elapsed() >= SCAN_INTERVAL) {
                last_scan = Some(Instant::now());
                let event = match connect(&mut hid, &cfg, nav.current(), &mut renderer) {
                    Scan::Opened(d) => {
                        scan_log.cleared();
                        let ev = Event::Connected { deck: d.info.clone() };
                        deck = Some(d);
                        ev
                    }
                    Scan::NoDeck => {
                        scan_log.cleared();
                        Event::Disconnected
                    }
                    Scan::Failed { problem, model } => {
                        if scan_log.failed(&problem) {
                            log::warn!("{problem}");
                        }
                        match model {
                            Some(model) => Event::CantOpen { model },
                            None => Event::Disconnected,
                        }
                    }
                };
                hub.publish(event);
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
                            paint(d, &cfg, nav.current(), &mut renderer, i, true);
                            hub.publish(Event::KeyDown { index: i, page: nav.current().map(str::to_string) });
                            let k = cfg.key(nav.current(), i);
                            if let Some(missed) = hold.press(i, k.and_then(|k| k.on_release.as_ref())) {
                                fire(&worker, &hub, i, &missed);
                            }
                            let Some(k) = k else { continue };
                            log::info!("key {i}: {}", k.action.summary());
                            let moved = match &k.action {
                                Action::Page { page } if cfg.has_page(page) => Ok(nav.go(Some(page))),
                                Action::Page { page } => Err(anyhow::anyhow!("there's no page named “{page}”")),
                                Action::Back => Ok(nav.back()),
                                a => {
                                    fire(&worker, &hub, i, a);
                                    Ok(false)
                                }
                            };
                            match moved {
                                Ok(true) => show_page(Some(d), &cfg, &nav, &mut renderer, &hub),
                                Ok(false) => {}
                                Err(e) => {
                                    log::warn!("key {i}: {e:#}");
                                    hub.publish(Event::ActionFailed { index: i, message: format!("{e:#}") });
                                }
                            }
                        }
                        DeviceStateUpdate::ButtonUp(i) => {
                            paint(d, &cfg, nav.current(), &mut renderer, i, false);
                            hub.publish(Event::KeyUp { index: i });
                            if let Some(a) = hold.release(i) {
                                log::info!("key {i} released: {}", a.summary());
                                fire(&worker, &hub, i, &a);
                            }
                        }
                        _ => {}
                    }
                }
            }
            Err(_) if term.load(Ordering::Relaxed) => break,
            Err(e) => {
                log::warn!("lost {} ({e}); waiting for it to come back", d.info.model);
                for (i, a) in hold.drain() {
                    log::info!("key {i} was held when the deck went away; releasing: {}", a.summary());
                    fire(&worker, &hub, i, &a);
                }
                deck = None;
                last_scan = None;
                hub.publish(Event::Disconnected);
            }
        }
    }

    log::info!("shutting down");
    // Wait for these to finish: systemd kills whatever is still in the service once we exit,
    // and a launch takes a few milliseconds to leave it.
    for (i, a) in hold.drain() {
        log::info!("key {i} was held at shutdown; releasing: {}", a.summary());
        if let Err(e) = action::run_and_wait(&a, RELEASE_AT_SHUTDOWN) {
            log::warn!("key {i}: {e:#}");
        }
    }
    if let Some(d) = deck {
        // Hand the deck back to its idle logo screen.
        let _ = d.dev.reset();
    }
    hub.shutdown();
    Ok(())
}

/// Run key `i`'s action, reporting a failure to the log and the configurator. Hyprland actions
/// go to the worker: `hyprctl` waits for Hyprland's answer, and the deck shouldn't.
fn fire(worker: &Worker, hub: &Hub, i: u8, a: &Action) {
    if let Action::Hyprland { .. } = a {
        worker.run(i, a.clone());
    } else if let Err(e) = action::run(a) {
        log::warn!("key {i}: {e:#}");
        hub.publish(Event::ActionFailed { index: i, message: format!("{e:#}") });
    }
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

/// What one look for a deck found.
enum Scan {
    Opened(Deck),
    NoDeck,
    /// Something went wrong; `model` is set when a deck is plugged in but couldn't be opened.
    Failed {
        problem: String,
        model: Option<String>,
    },
}

// elgato-streamdeck hands out its reader via `Arc<Self>`; everything stays on this thread.
#[allow(clippy::arc_with_non_send_sync)]
fn connect(hid: &mut HidApi, cfg: &Config, page: Option<&str>, renderer: &mut Renderer) -> Scan {
    if let Err(e) = device::refresh_device_list(hid) {
        return Scan::Failed { problem: format!("hidapi refresh: {e}"), model: None };
    }
    let Some((kind, serial)) = device::list_devices(hid).into_iter().find(|(k, _)| k.is_visual()) else {
        return Scan::NoDeck;
    };
    let dev = match StreamDeck::connect(hid, kind, &serial) {
        Ok(d) => Arc::new(d),
        Err(e) => {
            let model = device::model_name(kind).to_string();
            let problem = format!("found {model} but could not open it: {e} (is the udev rule installed?)");
            return Scan::Failed { problem, model: Some(model) };
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
    if let Err(e) = paint_all(&deck, cfg, page, renderer) {
        log::warn!("initial paint failed: {e}");
    }
    Scan::Opened(deck)
}

/// Repaint the deck after `nav` moved to another page, and tell the configurators.
fn show_page(d: Option<&Deck>, cfg: &Config, nav: &Nav, renderer: &mut Renderer, hub: &Hub) {
    log::info!("page: {}", nav.current().unwrap_or(omadeck_core::HOME));
    if let Some(d) = d
        && let Err(e) = paint_all(d, cfg, nav.current(), renderer)
    {
        log::warn!("repaint failed: {e}");
    }
    hub.publish(Event::PageChanged { page: nav.current().map(str::to_string) });
}

fn paint_all(d: &Deck, cfg: &Config, page: Option<&str>, renderer: &mut Renderer) -> Result<()> {
    d.dev.set_brightness(cfg.brightness)?;
    for i in 0..d.kind.key_count() {
        write_key(d, cfg, page, renderer, i, false)?;
    }
    d.dev.flush()?;
    Ok(())
}

fn paint(d: &Deck, cfg: &Config, page: Option<&str>, renderer: &mut Renderer, i: u8, pressed: bool) {
    if let Err(e) = write_key(d, cfg, page, renderer, i, pressed).and_then(|_| Ok(d.dev.flush()?)) {
        log::warn!("paint key {i}: {e}");
    }
}

fn write_key(d: &Deck, cfg: &Config, page: Option<&str>, renderer: &mut Renderer, i: u8, pressed: bool) -> Result<()> {
    let face = renderer.render(cfg.key(page, i), &cfg.style, d.info.key_size, pressed);
    d.dev.set_button_image(i, image::DynamicImage::ImageRgba8(face))?;
    Ok(())
}

/// Render the configured layout to a PNG, the way it looks on the hardware.
fn snapshot(out: &Path, page: Option<&str>) -> Result<()> {
    let cfg = Config::load()?;
    let keys = cfg.page(page).with_context(|| format!("no page named {}", page.unwrap_or_default()))?;
    let info = device::detect().into_iter().next().unwrap_or_else(DeckInfo::fallback);
    let mut renderer = Renderer::new(&cfg.style);
    let size = 144u32;
    let gap = 18u32;
    let (w, h) = (info.cols as u32 * (size + gap) + gap, info.rows as u32 * (size + gap) + gap);
    let mut canvas = image::RgbaImage::from_pixel(w, h, image::Rgba([10, 10, 12, 255]));
    for i in 0..info.key_count() {
        let face = renderer.render(keys.get(i), &cfg.style, size, false);
        let (r, c) = (i as u32 / info.cols as u32, i as u32 % info.cols as u32);
        image::imageops::overlay(&mut canvas, &face, (gap + c * (size + gap)) as i64, (gap + r * (size + gap)) as i64);
    }
    canvas.save(out)?;
    Ok(())
}
