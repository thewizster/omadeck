# The Omarchy bar widget

**Status:** 0.3.0

## Why

Omarchy users look at the bar. A small omadeck icon there shows at a glance whether the deck
is connected and which page it's on, and opens the app in one click.

## Rules

1. **Asked once.** On the first run on an Omarchy desktop, the app asks "Add omadeck to your
   Omarchy bar?" with Add as the default. Omarchy installs add-on plugins turned off, so
   omadeck doesn't add itself without asking. The answer is remembered in
   `~/.local/state/omadeck/bar-asked`, and the question never comes back.
2. **Easy to turn off.** "Show in Omarchy bar" in the app's settings turns it on and off, and
   so does Omarchy's own `omarchy plugin disable dev.omadeck`. The switch reads the real state
   each time the settings open, so it agrees with whatever Omarchy says.
3. **Only on Omarchy.** Without the Omarchy shell there's no question and no switch.
4. **What it shows:** the icon, bright when a deck is connected and dimmed otherwise, with a
   tooltip ("omadeck: Stream Deck (V2), page Media", "no Stream Deck connected", or "the
   service isn't running"). It follows unplugging, page changes and daemon restarts.
5. **Upgrades refresh it.** Each time the app starts, an installed widget's files are rewritten
   if they've changed, without changing whether it's on.
6. **No package changes.** The plugin's files are built into the app, which writes them into
   `~/.config/omarchy/plugins/dev.omadeck/`. The release tarball, its layout and the Omarchy
   package (PR #801) are untouched.
7. **Uninstalling removes it.** `scripts/uninstall.sh` turns it off and deletes the folder. A
   pacman uninstall can't remove files in a home folder; the widget then has no `omadeckd` to
   run and stays hidden.

## How

- **`omadeckd status`** prints the state as one line of Waybar-style JSON
  (`{"text","tooltip","class"}`); `--follow` prints a line whenever it changes and reconnects
  every 2 s while the daemon is away. A new connection's greeting (the deck, then its page) is
  shown as one line, so the bar never flashes Home on the way to the real page.
  `crates/omadeckd/src/status.rs`.
- **The plugin** (`packaging/omarchy-bar/`): a `manifest.json` (id `dev.omadeck`, a bar widget
  on the right) and `Widget.qml`, built on Omarchy's own `BarIconButton`, like the other bar
  icons. It runs `omadeckd status --follow`, the same pattern as Omarchy's dictation
  indicator, and opens the app with `uwsm-app -- dev.omadeck.Omadeck.desktop`. The app fills
  in the full path to the `omadeckd` installed next to it, so the widget doesn't depend on the
  shell's PATH.
- **The app** (`crates/omadeck-gui/src/bar.rs`): `install` writes the files (only when they
  differ, since Omarchy reloads a plugin when its files are saved); `enable` runs
  `omarchy-shell shell rescanPlugins` then `omarchy plugin enable dev.omadeck`; `disable` and
  `enabled` use `omarchy plugin disable` and `omarchy plugin list --json`. They take the
  command runner as a parameter, so tests check the commands without running them.

## Tests

| Test | Where | Rule |
|------|-------|------|
| `status_lines_for_each_state` | `status.rs` | 4 |
| `page_changes_update_the_tooltip` | `status.rs` | 4 |
| `follow_prints_a_line_per_change_and_survives_a_daemon_restart` (stand-in daemons on a real socket) | `status.rs` | 4 |
| `first_run_asks_once` | `bar.rs` | 1, 3 |
| `enable_and_disable_run_the_right_commands` | `bar.rs` | 2 |
| `enabled_reads_omarchy_plugin_list` (fixture copied from real output) | `bar.rs` | 2 |
| `a_failed_command_is_reported` | `bar.rs` | 2 |
| `install_writes_the_manifest_and_widget_with_the_daemon_path` | `bar.rs` | 6 |
| `install_leaves_unchanged_files_alone` | `bar.rs` | 5 |
| `the_bundled_manifest_is_valid_json_with_the_required_fields` | `bar.rs` | 6 |

Also checked during development: `omarchy plugin validate` passes the installed folder;
`qmllint` reports nothing beyond Omarchy's own components, which it can't see outside the
shell; and on the development machine the widget loaded on the real bar (one per monitor) with
no QML errors, and turning it off removed it cleanly.

## Manual checks

- On first run, the question appears once. Add puts the icon on the right of the bar.
- The tooltip follows unplugging, replugging and page changes on the deck.
- Stopping the service dims the icon and the tooltip says so; starting it brings it back.
- Clicking the icon opens omadeck.
- The settings switch turns the widget off and on, and agrees with
  `omarchy plugin list` after a change made in Omarchy.
- It survives `omarchy restart shell` and a theme change, and with the bar moved to the left
  or bottom of the screen.
- After `scripts/uninstall.sh` the widget is gone.
