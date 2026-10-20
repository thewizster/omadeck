# Release actions: keys that act when you let go

**Status:** 0.3.0

## Why

Push-to-talk: hold a key to dictate to your AI agent, let go to stop. More generally, a key can
start something on press and stop it on release (hold to mute, hold to show a panel).

```toml
[[key]]
index = 2
label = "Talk"
glyph = "f036c"

[key.action]
type = "app"
command = "voxtype record start"

[key.on_release]
type = "app"
command = "voxtype record stop"
```

## Rules

1. **The release runs for the key that was pressed.** It's captured when the key goes down, so
   a page change or a config reload while the key is held can't swap in another key's release.
2. **Letting go is guaranteed.** If the deck is unplugged or the daemon stops while a key is
   held, its release runs then, so dictation can't get stuck recording.
3. **A missed key-up** (a second press with no release in between) runs the first release
   before the new press.
4. **A release can't change pages.** The app doesn't offer Open page or Back for it, and the
   daemon ignores one written by hand, with a warning.
5. **`omadeckd press KEY` is a tap:** it runs the press action, then the release action at
   once, so nothing is left held.
6. **"Nothing" on release means no release action**; the key's file has no `[key.on_release]`.
7. **Choosing an app for the release action doesn't change the key's label or icon.** The
   key's face follows what it does when pressed.

## Speed

Releases launch the same way as presses, through `uwsm-app`, so what they start keeps
running if the daemon restarts. Measured on the development machine (20 runs each), from
spawning to the command running:

| Path | Delay |
|------|-------|
| direct | 2 ms |
| `systemd-run --scope` | 6 ms |
| `uwsm-app` | 13–29 ms, median 21 ms |

A key press and release are at least ~50 ms apart, so the start always lands before the
stop, and 20 ms doesn't clip the first word of a dictation. No separate fast path is needed.

## How

- `KeyConfig.on_release: Option<Action>`, read and written like `action`, so an unknown
  release action is kept too.
- `crates/omadeckd/src/hold.rs`: `Hold` arms a release on key-down (`press`), hands it back
  on key-up (`release`), and gives back everything still held (`drain`) when the deck goes
  away or the daemon stops.
- In the app, `editor.rs` edits either action through a `Slot` (`Press` or `Release`), so one
  set of rows serves both. "When released" is its own group with its own ▶.

## Tests

| Test | Where | Rule |
|------|-------|------|
| `release_runs_the_action_armed_at_press_whatever_the_key_shows_now` | `hold.rs` | 1 |
| `unplugging_releases_every_held_key` | `hold.rs` | 2 |
| `a_missed_key_up_releases_before_the_next_press` | `hold.rs` | 3 |
| `navigation_is_refused_as_a_release_action` | `hold.rs` | 4 |
| `keys_without_on_release_arm_nothing` | `hold.rs` | 6 |
| `a_release_cant_be_set_to_change_pages` | `editor.rs` | 4 |
| `nothing_on_release_removes_the_release_action` | `editor.rs` | 6 |
| `each_slot_edits_its_own_action` | `editor.rs` | |
| `on_release_roundtrips`, `an_unknown_release_action_is_kept_too` | `config.rs` | |
| `a_key_with_only_a_release_action_is_not_blank` | `config.rs` | |

Rules 5 and 7 are checked by hand (below): they're a few lines in `main.rs` and a GTK
callback.

## Manual checks

- **Push-to-talk:** a key with `voxtype record start` / `stop`. Hold it and speak into a
  terminal or Claude; let go. The text appears, and the first word isn't cut off.
- **Unplug while holding:** recording stops.
- **`systemctl --user restart omadeckd` while holding:** recording stops.
- **Page change while holding:** a hold key on a sub-page; press it, switch page in the app,
  let go. The right release runs (check the daemon log).
- **`omadeckd press KEY`** on the hold key: it starts and stops at once.
- **Editor:** "When released" lists no Open page or Back. Its ▶ runs the release action.
  Choosing an app for it leaves the key's label and icon alone. Setting it back to Nothing
  removes `[key.on_release]` from the file.
