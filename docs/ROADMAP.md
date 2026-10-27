# Roadmap

Plans, roughly in order. Release numbers are a guide, not a promise.

## Released

- **0.2.0:** pages and folders, undo and backups, and the fixes from stress testing (see
  `CHANGELOG.md`).

## 0.3.0: hold keys and the Omarchy bar

- Release actions (below).
- The Omarchy bar widget (below).
- Forward compatibility: a key with an action type this version doesn't know is skipped with
  a warning instead of the whole config being rejected. Templates need this, since one made
  with a newer omadeck should still load in an older one.
- More fixes from stress testing as time allows.

## 0.4.0: templates

Templates and Omarchy Basics (below). Omarchy Basics needs release actions for its Talk key,
so templates come after 0.3.0.

## Release actions (hold keys)

Let a key do something when it's released as well as when it's pressed:

```toml
[[key]]
label = "Talk"
action     = { type = "app", command = "voxtype record start" }
on_release = { type = "app", command = "voxtype record stop" }
```

This makes push-to-talk work, and hold-to-mute and similar keys too.

- Start and stop must be fast. Run short commands like these directly instead of through
  `uwsm-app`, or the first word of a dictation gets cut off.
- Dictation types into the focused window. A later "Talk to agent" key could focus the agent's
  window first, then start recording.

## Omarchy bar widget

An omadeck icon on the Omarchy bar, on by default. Its tooltip says whether the deck is
connected and which page it shows, and clicking it opens the app.

- It's an Omarchy shell plugin (`manifest.json` plus a small QML widget), the same kind as
  Omarchy's own Dropbox and Tailscale widgets, so it follows the theme.
- The plugin files are built into the `omadeck` app. On first run, when the Omarchy shell is
  present, the app writes them to `~/.config/omarchy/plugins/omadeck/` and enables the widget
  with `omarchy plugin enable`. The release tarball and the packages don't change.
- Turn it off with "Show in Omarchy bar" in the app's settings, or with
  `omarchy plugin disable omadeck`. The app remembers the choice and never re-enables it.
- `scripts/uninstall.sh` removes the plugin.
- Omarchy installs third-party plugins disabled by default, so consider asking once on first
  run ("Add omadeck to your bar?") rather than enabling it silently.
- Live status may need a small `omadeckd status` command for the widget to poll.

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
