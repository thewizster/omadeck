# Roadmap

Plans, roughly in order. Release numbers are a guide, not a promise.

## Released

- **0.2.0:** pages and folders, undo and backups, and the fixes from stress testing (see
  `CHANGELOG.md`).

## 0.3.0: hold keys and the Omarchy bar

- **Done:** release actions (below; design in `docs/design/release-actions.md`).
- **Done:** forward compatibility. A key with an action type this version doesn't know is
  drawn as usual and kept as written, instead of the whole config being rejected, and unknown
  settings survive a save. Templates need this, since one made with a newer omadeck should
  still load in an older one. Design in `docs/design/forward-compat.md`.
- **Done:** the Omarchy bar widget (below; design in `docs/design/bar-widget.md`).
- **Done:** fixes from stress testing: B11 (log spam without the udev rule), B15 (silent app
  launch failures), B10 (huge images) and B9 (very long labels).

## 0.4.0: templates

Templates and Omarchy Basics (below). Omarchy Basics needs release actions for its Talk key,
so templates come after 0.3.0.

## Release actions (hold keys)

Let a key do something when it's released as well as when it's pressed:

```toml
[[key]]
index = 2
label = "Talk"

[key.action]
type = "app"
command = "voxtype record start"

[key.on_release]
type = "app"
command = "voxtype record stop"
```

This makes push-to-talk work, and hold-to-mute and similar keys too.

- Launching through `uwsm-app` adds about 20 ms, which was measured and is fast enough: the
  first word of a dictation isn't cut off.
- Dictation types into the focused window. A later "Talk to agent" key could focus the agent's
  window first, then start recording.

## Omarchy bar widget

Done for 0.3.0; see `docs/design/bar-widget.md`. An omadeck icon on the Omarchy bar shows
whether the deck is connected and which page it's on, and opens the app when clicked. The app
asks once on first run whether to add it, and a settings switch turns it on and off. It's an
Omarchy shell plugin that runs `omadeckd status --follow`; its files are built into the app,
so the release tarball and the Omarchy package don't change.

## Templates

Ready-made pages you can add to your deck, so you can hit the ground running.

**Format.** A template is a config file holding only `[[page]]` tables, plus a header:

```toml
[template]
name = "Omarchy Basics"
description = "Your agent, dictation and everyday Omarchy keys"
author = "omadeck"
requires = ["voxtype"]   # commands it needs; adding the template says if any are missing
entry = "Omarchy"        # the page a folder key should open
```

On disk it's a folder with `template.toml` and `icons/`, which works well in git. For
downloads, the same folder zipped is one `.omadeck` file. Keys that use Nerd Font glyphs need
no icon files.

**Two operations, one preview screen:**

- *Add from template* merges pages into your config. A page whose name is taken is renamed,
  and the links inside the template follow the new name.
- *Restore backup* replaces your config with one from `~/.local/state/omadeck/backups`.
  Any backup can also be used as a template.

**Save as template** takes a page and every page it links to, so a folder tree stays
together, and copies the icons it uses.

**Safety.** A template is a set of commands that run when you press keys. Adding one shows
every command first, and nothing runs on import. Templates don't carry script files in the
first version.

**Deck sizes.** Templates are mostly made on 15-key decks. Keys that don't fit a smaller deck
spill onto an extra "more" page.

### Omarchy Basics, the first built-in template

- Back
- Agent: `omarchy-agent --pick` (your default agent, e.g. Claude Code)
- Talk: hold to dictate (`voxtype record start` / `stop`; needs release actions)
- Dictate: `voxtype record toggle`
- Omarchy menu (`omarchy-menu`) and Keybindings (`omarchy-menu-keybindings`)
- Terminal (`omarchy-launch-terminal`) and Browser (`omarchy-launch-browser`)
- Night light (`omarchy-toggle-nightlight`) and workspace keys
- Volume up and down (`omarchy audio output volume raise` / `lower`), and mute
  (`omarchy audio output volume mute-toggle`). These go through Omarchy, so its on-screen
  volume display shows, like the keyboard's volume keys. Tested on real hardware.

If voxtype isn't installed, the preview offers `omarchy-voxtype-install`. The first-run
layout becomes this template, so new users and "Add from template" get the same thing.

## Template library

- A `templates/` folder in this repo (its own repo if it grows).
- A gallery page on omadeck.app.
- "Add from URL" in the app, through the same preview.

## App templates

Templates for OBS and other apps once omadeck can talk to them. Each declares what it needs
in `requires`.
