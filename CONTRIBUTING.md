# Contributing to omadeck

Thanks for helping! Bug reports, hardware reports and pull requests are all welcome.

## What's welcome right now

omadeck is young, and it's moving at a deliberate pace: every release is tested by hand on
real hardware before it ships. To keep that bar while the core settles:

- **Bug reports and hardware reports** are always welcome, and the most useful thing you can
  send. Include your deck model and the daemon log (`journalctl --user -u omadeckd`).
- **Small, focused fixes** (one bug, a few files) are welcome as pull requests.
- **New features and large changes**: please open an issue first, before writing code. The
  next releases follow [docs/ROADMAP.md](docs/ROADMAP.md), and a big change, especially to the
  daemon, means starting the hardware testing over. A large series is likely to be declined
  for now, however good it is.

This will open up as the project matures.

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

- `omadeckd snapshot out.png [PAGE]` renders a layout without touching the hardware.
- `omadeckd press 3 [PAGE]` runs key 3's action without pressing anything.
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
- Add tests the way [docs/TESTING.md](docs/TESTING.md) describes; a bug fix starts with a test that fails without it.
- User-facing change? Add a line under **Unreleased** in `CHANGELOG.md`.
- Hardware-specific change? Say which model you tested on.

## Good first issues

Look for the [`good first issue`](https://github.com/thewizster/omadeck/labels/good%20first%20issue)
label. For anything on the [roadmap](README.md#roadmap), open an issue to talk it through
before you start.
