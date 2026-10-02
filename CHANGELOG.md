# Changelog

All notable changes to omadeck are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project uses
[Semantic Versioning](https://semver.org/).

## [Unreleased]

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

[Unreleased]: https://github.com/thewizster/omadeck/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/thewizster/omadeck/releases/tag/v0.1.0
