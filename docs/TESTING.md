# Testing omadeck

Every change ships with tests. We don't do strict test-driven development, but each feature
and each fix comes with a planned set of well-named tests, and anything a test can't reach
goes on the release's hands-on QA checklist.

```bash
cargo fmt --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked
```

CI runs exactly these, in an Arch Linux container with no display and no Stream Deck, so
every test has to pass there too.

## The rules

1. **Tests live next to the code** in a `#[cfg(test)] mod tests` block at the bottom of the
   file. Name each test as a sentence about behaviour, so a failure reads like a bug report:
   `a_window_that_stops_reading_cannot_hold_up_the_deck`, not `test_hub_2`.

2. **Keep decisions out of hardware and GTK code.** Put the logic in a plain function or a
   small struct, and let the deck loop or the GTK callback only call it. `nav.rs` (page
   history) and `worker.rs` (ordered Hyprland actions) are the pattern: the daemon loop is
   thin, and the rules are unit-tested.

3. **A bug fix starts with a failing test.** Write the test that reproduces the bug, check
   that it fails on the current code, then fix it. A test that passes on the broken code
   doesn't protect anything.

4. **Tests never touch the real world.** No Stream Deck, no network, no `~/.config/omadeck`,
   no real Hyprland or Omarchy shell.
   - Use temporary paths that include the process id, and the explicit-path APIs
     (`Config::load_from`, `Config::save_to`).
   - Code that runs another program takes the runner as a parameter, so a test can check
     the exact command line without running it.
   - Tests that spawn processes use only `sh`, `true` and `sleep`.

5. **Wait on deadlines, never on fixed sleeps.** Poll for the condition with a generous
   limit (seconds, not milliseconds). Before merging a test that depends on timing, run it
   ten times in a row and again with every CPU core busy.

6. **Fixtures are small files beside the tests**, read with `include_str!`. When code reads
   another program's output (for example `omarchy plugin list --json`), the fixture copies
   the real output's shape.

7. **What can't be unit-tested goes on the QA checklist.** Each release has a hands-on
   checklist run on real hardware and a real Omarchy desktop. A feature's design note lists
   its manual checks, with the expected result for each.

8. **Size.** A feature adds at least one test for each rule in its design note. A bug fix
   adds at least one regression test.

## Where things are tested

| Crate | What's covered by tests |
|-------|-------------------------|
| `omadeck-core` | Config parsing and saving (including settings from newer versions, and a byte-for-byte check against 0.2.0), page edits and undo, backups, actions' command building, finding a missing app's program, running a release and waiting for it, fitting labels (against the 0.2.0 algorithm), the icon cache, rendering helpers, the theme |
| `omadeckd` | Page history, release actions armed on press (`hold.rs`), logging scan problems once (`scan.rs`), `omadeckd status` and its reconnecting `--follow`, the action worker, the event socket (with real sockets in a temp folder) |
| `omadeck-gui` | Media helpers (including shrinking huge pictures on import), the editor's press and release slots, the Omarchy bar widget's files and commands; anything else that holds logic is moved out of GTK callbacks so it can be tested |

Hardware, USB hot-plug, the GTK interface and the Omarchy desktop are covered by the QA
checklist.
