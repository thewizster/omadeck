# Contributing to omadeck

Thanks for helping! Bug reports, hardware reports and pull requests are all welcome.

## Getting set up

```bash
mise use -g rust@latest                                   # or any Rust ≥ 1.85
sudo pacman -S --needed gtk4 libadwaita systemd-libs      # build dependencies on Arch/Omarchy
git clone https://github.com/thewizster/omadeck && cd omadeck
cargo build
```

Running from the build tree:

```bash
systemctl --user stop omadeckd     # only one program can own the deck
cargo run -p omadeckd              # daemon (logs to the terminal)
cargo run -p omadeck-gui           # configurator, in another terminal
```

Handy while developing:

- `omadeckd snapshot out.png` renders the current layout without touching the hardware.
- `omadeckd press 3` runs key 3's action without pressing anything.
- `OMADECK_SELECT=3 omadeck` opens the configurator with key 3 selected.
- `XDG_CONFIG_HOME=/tmp/omadeck-test cargo run -p omadeck-gui` works with a throwaway config.

## Layout

| Crate | What lives there |
|-------|------------------|
| `omadeck-core` | Config format, Omarchy theme, key renderer, actions, IPC types. No GTK. |
| `omadeckd` | The daemon: device I/O, file watching, the event socket. |
| `omadeck-gui` | The GTK4/libadwaita configurator. |

The renderer is shared on purpose: previews in the app and pixels on the deck come
from the same code, so keep drawing logic in `omadeck-core::render`.

## Before you open a PR

```bash
cargo fmt
cargo clippy --all-targets -- -D warnings
cargo test
```

- Keep changes focused; one feature or fix per PR.
- User-facing change? Add a line under **Unreleased** in `CHANGELOG.md`.
- Hardware-specific change? Say which model you tested on.

## Good first issues

Look for the [`good first issue`](https://github.com/thewizster/omadeck/labels/good%20first%20issue)
label, or pick something from the [roadmap](README.md#roadmap).
