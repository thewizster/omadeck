# Forward compatibility: keep what you don't understand

**Status:** 0.3.0 · **Fixes:** B8 from the 0.2 stress test

## Why

Configs travel. They're shared as templates, synced through dotfiles, and opened by an older
omadeck on another machine. Before 0.3.0, two things went wrong when a config came from a
newer version:

- A key with an action type this version doesn't know (`type = "obs_scene"`) made the whole
  file fail to load, so the deck went blank.
- Settings this version doesn't know were silently dropped the next time the app saved.

## Rules

1. **An unknown action type loads.** The key is drawn as usual (label, icon, colours), and the
   rest of the deck works.
2. **Pressing it does nothing and says why.** The daemon logs it and the app shows "this key
   needs a newer omadeck (its action is “obs_scene”)".
3. **It's kept exactly as written,** including any nested settings, every time the app saves.
4. **Unknown settings are kept too,** at every level: the top of the file, `[style]`, each
   `[[page]]` and each `[[key]]`.
5. **In the app,** the key's editor says the action comes from a newer omadeck. Choosing another
   action replaces it, with Undo. ▶ is turned off for it.
6. **Mistakes in known actions are still errors.** A `type = "url"` with no `url` is reported
   with its line number, as before, rather than being treated as unknown.
7. **Configs with nothing unknown save byte-for-byte as they did in 0.2.0.**

## How

- `Action::Unknown { kind, raw }` holds the action's table as written. `KeyConfig.action` uses
  `#[serde(with = "keep_unknown")]`: it reads the table, and if `type` isn't one of
  `Action::TYPES`, keeps it whole; otherwise it deserializes the known action as before.
- `Config`, `Style`, `Page` and `KeyConfig` each have `#[serde(flatten)] extra: toml::Table`,
  left out of the file when empty.

## Tests

In `crates/omadeck-core`, using `src/fixtures/future.toml`, a config "from a future omadeck":

| Test | Rule |
|------|------|
| `unknown_action_type_loads_and_the_rest_of_the_deck_still_works` | 1 |
| `running_an_unknown_action_says_a_newer_version_is_needed` | 2 |
| `unknown_action_survives_a_save_unchanged` | 3 |
| `unknown_fields_survive_a_save_at_every_level` | 3, 4 |
| `a_mistake_in_a_known_action_is_still_an_error` | 6 |
| `known_configs_save_exactly_as_before` (golden file `example.saved.toml`, written by 0.2.0) | 7 |
| `every_known_kind_is_listed_in_types` | keeps `TYPES` in step with the enum |

## Manual checks

- Put `future.toml` in place as your config. The deck shows Terminal and Scene; pressing Scene
  shows the "needs a newer omadeck" toast in the app.
- Select Scene in the app: the editor explains it. Change any other key, then check the file:
  `obs_scene`, `corner_radius`, `[template]` and the page's `color` are all still there.
- Change Scene to another action, then press Undo: `obs_scene` is back.
