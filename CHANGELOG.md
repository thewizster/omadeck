# Changelog

All notable changes to omadeck are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project uses
[Semantic Versioning](https://semver.org/).

## [Unreleased]

### Fixed
- A config from a newer omadeck no longer blanks the deck. A key whose action type this
  version doesn't know is drawn as usual, says it needs a newer omadeck when pressed, and is
  kept as written. Settings this version doesn't know are no longer dropped when the app
  saves.
- A stalled Hyprland could freeze the deck: Hyprland actions waited on `hyprctl` with no time
  limit, on the same loop that reads the keys. They now run in order on their own thread, and
  `hyprctl` gets two seconds to answer. Thanks to
  [@nishantpillai5](https://github.com/nishantpillai5) for spotting this.
- A configurator window that stopped reading could hold up the deck, and once dropped it was
  never told, so it stayed silently disconnected. Each window now has its own writer; one that
  falls far behind is disconnected cleanly and reconnects. Also spotted by
  [@nishantpillai5](https://github.com/nishantpillai5).

## [0.2.0] - 2026-10-18

### Added
- Pages and folders. Extra pages live in `[[page]]` tables (top-level keys stay the Home page),
  with two new actions: `page` opens a page by name and `back` returns to the previous one.
  The configurator gets a page switcher (new, rename, delete), and "New page" on a key builds a
  folder with a Back key in place. The deck follows the page you're editing, and the app
  follows page changes made on the deck.
- `omadeckd press KEY [PAGE]` and `omadeckd snapshot FILE [PAGE]` take an optional page.
- Undo for deleting a page or clearing a key, and backups of `config.toml` in
  `~/.local/state/omadeck/backups` each time the app opens and before a page is deleted
  (the newest 20 are kept).
- The app warns when a page has no Back key, so the deck would get stuck on it, and offers to
  add one.

### Fixed
- A colour value with a non-ASCII character (in the config or the Omarchy theme) crashed the
  daemon, and systemd restarted it into the same crash.
- Websites, scripts and terminals opened from the deck ran inside the omadeckd service, so
  restarting or upgrading it closed them. They now get their own scope (via `uwsm-app`, or
  `systemd-run --scope` without it), like launched apps.
- The configurator now picks up edits made to `config.toml` while it's open instead of
  overwriting them on its next save.
- A config the configurator can't parse is no longer renamed to `config.toml.broken` and
  replaced with an empty one. The app shows the error and won't save until the file is fixed.
- A site icon or file chosen for a key no longer lands on another page's key if you switch
  pages before it finishes.
- Changing the system font (`omarchy font set`) now updates key labels without a restart, and
  icon glyphs no longer vanish when the new font has no Nerd Font symbols.
- Live reload keeps working after the config folder is deleted and recreated, and with a
  symlinked `config.toml`. Saving from the configurator keeps the symlink.

## [0.1.0] - 2026-10-02

The first public release.

### Added
- `omadeckd`, a small Rust daemon (systemd user service) that drives the Stream Deck:
  hotplug, live config reload, and a last-good-config fallback when a hand edit has a typo.
- `omadeck`, a GTK4/libadwaita configurator: click a key to edit it, drag to rearrange,
  press a physical key to jump to it, test any action with ▶.
- Five action types: launch app (via `uwsm-app`), run script (optionally in a terminal),
  play sound (PipeWire), open website, and Hyprland dispatchers (Lua or classic syntax).
- Key faces with images (PNG/JPEG/SVG/WebP/…), app icons from your icon theme,
  site favicons, or Nerd Font glyphs; labels in your Omarchy font.
- Omarchy theme integration: key colours follow the active theme and repaint on
  `omarchy theme set`.
- First-run starter layout exercising every action type.
- CLI: `omadeckd list`, `omadeckd press N`, `omadeckd snapshot FILE.png`.
- Installer, udev rule, desktop entry, AUR packages (`omadeck`, `omadeck-bin`) and a project website.

[Unreleased]: https://github.com/thewizster/omadeck/compare/v0.2.0...HEAD
[0.2.0]: https://github.com/thewizster/omadeck/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/thewizster/omadeck/releases/tag/v0.1.0
