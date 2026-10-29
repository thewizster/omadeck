//! Executes key actions. Everything is spawned detached so a slow app never blocks the deck.

use std::os::unix::fs::PermissionsExt;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::OnceLock;
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};

use crate::config::Action;
use crate::paths;

pub fn run(action: &Action) -> Result<()> {
    match action {
        Action::None => Ok(()),
        Action::App { command } => launch_app(command),
        Action::Script { path, terminal } => run_script(path, *terminal),
        Action::Sound { file, volume } => play_sound(&paths::resolve(file), *volume),
        Action::Url { url } => open_url(url),
        Action::Hyprland { dispatch } => hypr_dispatch(dispatch),
        Action::Page { .. } | Action::Back => bail!("changing pages only works on the deck"),
        Action::Unknown { kind, .. } => bail!("this key needs a newer omadeck (its action is “{kind}”)"),
    }
}

/// Run an action and wait for it to finish, for when the daemon is about to exit. A launch
/// through `run` is still inside omadeckd's service for its first few milliseconds, and
/// systemd kills everything left in the service when the daemon exits. So app commands and
/// scripts run directly here and are waited for, up to `limit`; anything else goes through `run`.
pub fn run_and_wait(action: &Action, limit: Duration) -> Result<()> {
    let (argv, dir) = match action {
        Action::App { command } if !is_desktop_id(command.trim()) => (app_argv(command)?, home_dir()),
        Action::Script { path, terminal: false } => script_argv(path)?,
        other => return run(other),
    };
    let mut c = Command::new(&argv[0]);
    c.args(&argv[1..]).current_dir(dir);
    log::info!("exec {c:?} (waiting for it)");
    let out = output_within(c, limit)?;
    if !out.status.success() {
        bail!("{} exited with {}", argv[0], out.status);
    }
    Ok(())
}

fn which(bin: &str) -> bool {
    std::env::var_os("PATH").map(|p| std::env::split_paths(&p).any(|d| d.join(bin).is_file())).unwrap_or(false)
}

/// Omarchy launches apps through uwsm so they get their own systemd scope.
fn has_uwsm() -> bool {
    static HAS: OnceLock<bool> = OnceLock::new();
    *HAS.get_or_init(|| which("uwsm-app"))
}

/// A command that outlives the daemon. Anything started directly would stay in
/// omadeckd's cgroup, and systemd kills that whole cgroup when the service stops or
/// restarts — taking browsers, terminals and scripts with it. So run it through
/// uwsm-app (as Omarchy does) or, failing that, in a transient systemd scope.
fn detached<S: AsRef<std::ffi::OsStr>>(argv: &[S]) -> Command {
    static SCOPE: OnceLock<bool> = OnceLock::new();
    let mut c = if has_uwsm() {
        let mut c = Command::new("uwsm-app");
        c.arg("--");
        c
    } else if *SCOPE.get_or_init(|| which("systemd-run")) {
        let mut c = Command::new("systemd-run");
        c.args(["--user", "--scope", "--collect", "--quiet", "--"]);
        c
    } else {
        let mut c = Command::new(&argv[0]);
        c.args(&argv[1..]);
        return c;
    };
    c.args(argv);
    c
}

/// Spawn in its own process group with no stdio, reaping it on a helper thread.
fn spawn(mut cmd: Command) -> Result<()> {
    cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).process_group(0);
    let desc = format!("{cmd:?}");
    log::info!("exec {desc}");
    let mut child = cmd.spawn().with_context(|| format!("failed to start {desc}"))?;
    thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

fn needs_shell(s: &str) -> bool {
    s.chars().any(|c| "|&;<>()$`*?[]{}~!#".contains(c))
}

/// The program an app command would start, if it isn't installed: a desktop entry missing from
/// every `applications` folder in `data_dirs`, or a command that isn't on `path`. Commands that
/// need a shell are left to the shell.
fn missing_program(command: &str, path: Option<&std::ffi::OsStr>, data_dirs: &[std::path::PathBuf]) -> Option<String> {
    let command = command.trim();
    if is_desktop_id(command) {
        let installed = data_dirs.iter().any(|d| d.join("applications").join(command).is_file());
        return (!installed).then(|| command.to_string());
    }
    if command.is_empty() || needs_shell(command) {
        return None;
    }
    let program = shell_words::split(command).ok()?.into_iter().next()?;
    let found = if program.contains('/') {
        paths::resolve(&program).is_file()
    } else {
        path.is_some_and(|p| std::env::split_paths(p).any(|d| d.join(&program).is_file()))
    };
    (!found).then_some(program)
}

/// Where desktop entries live: `$XDG_DATA_HOME` and `$XDG_DATA_DIRS`, with their defaults.
fn data_dirs() -> Vec<std::path::PathBuf> {
    let home = std::env::var_os("XDG_DATA_HOME")
        .filter(|v| !v.is_empty())
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| home_dir().join(".local/share"));
    let rest = std::env::var_os("XDG_DATA_DIRS")
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| "/usr/local/share:/usr/share".into());
    std::iter::once(home).chain(std::env::split_paths(&rest)).collect()
}

/// A desktop entry id, e.g. "org.gnome.Nautilus.desktop".
fn is_desktop_id(command: &str) -> bool {
    command.ends_with(".desktop") && !command.contains(char::is_whitespace)
}

fn home_dir() -> std::path::PathBuf {
    std::env::var_os("HOME").unwrap_or_else(|| "/".into()).into()
}

/// The command line for an app action that isn't a desktop entry.
fn app_argv(command: &str) -> Result<Vec<String>> {
    let command = command.trim();
    if command.is_empty() {
        bail!("no application set");
    }
    Ok(if needs_shell(command) {
        vec!["sh".into(), "-c".into(), command.into()]
    } else {
        shell_words::split(command).context("could not parse command")?
    })
}

fn launch_app(command: &str) -> Result<()> {
    let command = command.trim();
    // uwsm-app always starts, so a misspelled command would otherwise fail without a word.
    if let Some(program) = missing_program(command, std::env::var_os("PATH").as_deref(), &data_dirs()) {
        bail!("“{program}” isn't installed, or isn't on your PATH");
    }
    if is_desktop_id(command) {
        if has_uwsm() {
            return spawn(detached(&[command]));
        }
        return spawn(detached(&["gtk-launch", command.trim_end_matches(".desktop")]));
    }
    let mut c = detached(&app_argv(command)?);
    c.current_dir(home_dir());
    spawn(c)
}

/// The command line for a script (run with `sh` unless it's executable), and the folder to run
/// it in.
fn script_argv(path: &str) -> Result<(Vec<String>, std::path::PathBuf)> {
    let p = paths::resolve(path.trim());
    if !p.exists() {
        bail!("script not found: {}", p.display());
    }
    let executable = p.metadata().map(|m| m.permissions().mode() & 0o111 != 0).unwrap_or(false);
    let mut argv: Vec<String> = Vec::new();
    if !executable {
        argv.push("sh".into());
    }
    argv.push(p.to_string_lossy().into_owned());
    let dir = p.parent().map(Path::to_path_buf).unwrap_or_else(home_dir);
    Ok((argv, dir))
}

fn run_script(path: &str, terminal: bool) -> Result<()> {
    let (mut argv, dir) = script_argv(path)?;
    if terminal {
        let term: &[&str] = if which("xdg-terminal-exec") { &["xdg-terminal-exec"] } else { &["alacritty", "-e"] };
        argv.splice(0..0, term.iter().map(|s| s.to_string()));
    }
    let mut c = detached(&argv);
    c.current_dir(dir);
    spawn(c)
}

fn play_sound(file: &Path, volume: Option<f32>) -> Result<()> {
    if !file.is_file() {
        bail!("sound not found: {}", file.display());
    }
    let vol = volume.map(|v| v.clamp(0.0, 1.5));
    let c = if which("pw-play") {
        let mut c = Command::new("pw-play");
        if let Some(v) = vol {
            c.arg(format!("--volume={v}"));
        }
        c.arg(file);
        c
    } else if which("paplay") {
        let mut c = Command::new("paplay");
        if let Some(v) = vol {
            c.arg(format!("--volume={}", (v * 65536.0) as u32));
        }
        c.arg(file);
        c
    } else {
        let mut c = Command::new("aplay");
        c.arg(file);
        c
    };
    spawn(c)
}

pub fn normalize_url(url: &str) -> String {
    let url = url.trim();
    if url.contains("://") || url.starts_with("mailto:") { url.to_string() } else { format!("https://{url}") }
}

fn open_url(url: &str) -> Result<()> {
    if url.trim().is_empty() {
        bail!("no URL set");
    }
    spawn(detached(&["xdg-open".to_string(), normalize_url(url)]))
}

fn hypr_dispatch(dispatch: &str) -> Result<()> {
    let d = dispatch.trim();
    if d.is_empty() {
        bail!("no dispatcher set");
    }
    // Lua dispatcher (Hyprland 0.55+ with a Lua config), e.g. hl.dsp.focus({ workspace = "3" }).
    if d.contains('(') {
        return hyprctl_dispatch(&[d.to_string()]);
    }
    // Classic syntax, e.g. `workspace 3`. Lua-config Hyprland rejects it, so translate.
    let args = shell_words::split(d).context("could not parse dispatcher")?;
    match hyprctl_dispatch(&args) {
        Ok(()) => Ok(()),
        Err(e) if e.to_string().to_lowercase().contains("lua") || e.to_string().contains("hl.") => {
            match legacy_to_lua(d) {
                Some(lua) => hyprctl_dispatch(&[lua]),
                None => bail!(
                    "this Hyprland expects Lua dispatchers, e.g. hl.dsp.focus({{ workspace = \"3\" }}) — couldn't translate `{d}`"
                ),
            }
        }
        Err(e) => Err(e),
    }
}

/// How long `hyprctl` gets to answer before it's given up on.
const HYPRCTL_TIMEOUT: Duration = Duration::from_secs(2);

/// Run `hyprctl dispatch …` and wait for its answer (it replies "ok" in a few ms).
fn hyprctl_dispatch(args: &[String]) -> Result<()> {
    log::info!("exec hyprctl dispatch {}", args.join(" "));
    let mut cmd = Command::new("hyprctl");
    cmd.arg("dispatch").args(args);
    let out = output_within(cmd, HYPRCTL_TIMEOUT).context("hyprctl")?;
    let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    let text = text.trim();
    if out.status.success() && (text.is_empty() || text == "ok") {
        Ok(())
    } else {
        let msg: Vec<&str> = text.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
        bail!("hyprctl: {}", if msg.is_empty() { "failed".to_string() } else { msg.join(" ") })
    }
}

/// `Command::output`, except that a command still running after `limit` is killed, so a
/// stalled Hyprland can't hang whoever is waiting for it.
fn output_within(mut cmd: Command, limit: Duration) -> Result<std::process::Output> {
    let mut child = cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn()?;
    let deadline = Instant::now() + limit;
    while child.try_wait()?.is_none() {
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            bail!("no answer after {} s", limit.as_secs_f32());
        }
        thread::sleep(Duration::from_millis(5));
    }
    Ok(child.wait_with_output()?)
}

fn lua_str(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

/// Translate common classic dispatchers to Hyprland's Lua API.
pub fn legacy_to_lua(dispatch: &str) -> Option<String> {
    let d = dispatch.trim();
    let (name, arg) = d.split_once(char::is_whitespace).map(|(n, a)| (n, a.trim())).unwrap_or((d, ""));
    let lua = match (name, arg) {
        ("workspace", ws) if !ws.is_empty() => format!("hl.dsp.focus({{ workspace = {} }})", lua_str(ws)),
        ("movetoworkspace", ws) if !ws.is_empty() => format!("hl.dsp.window.move({{ workspace = {} }})", lua_str(ws)),
        ("movetoworkspacesilent", ws) if !ws.is_empty() => {
            format!("hl.dsp.window.move({{ workspace = {}, follow = false }})", lua_str(ws))
        }
        ("togglespecialworkspace", n) => format!("hl.dsp.workspace.toggle_special({})", lua_str(n)),
        ("togglefloating", _) => "hl.dsp.window.float({ action = \"toggle\" })".into(),
        ("fullscreen", "1") => "hl.dsp.window.fullscreen({ mode = \"maximized\" })".into(),
        ("fullscreen", _) => "hl.dsp.window.fullscreen({ mode = \"fullscreen\" })".into(),
        ("killactive", _) => "hl.dsp.window.close()".into(),
        ("pseudo", _) => "hl.dsp.window.pseudo()".into(),
        ("movefocus", dir @ ("l" | "r" | "u" | "d")) => format!("hl.dsp.focus({{ direction = {} }})", lua_str(dir)),
        ("exec", cmd) if !cmd.is_empty() => format!("hl.dsp.exec_cmd({})", lua_str(cmd)),
        _ => return None,
    };
    Some(lua)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hypr_translation() {
        assert_eq!(legacy_to_lua("workspace 3").unwrap(), r#"hl.dsp.focus({ workspace = "3" })"#);
        assert_eq!(
            legacy_to_lua("movetoworkspacesilent 2").unwrap(),
            r#"hl.dsp.window.move({ workspace = "2", follow = false })"#
        );
        assert_eq!(legacy_to_lua("killactive").unwrap(), "hl.dsp.window.close()");
        assert_eq!(legacy_to_lua(r#"exec notify-send "hi""#).unwrap(), r#"hl.dsp.exec_cmd("notify-send \"hi\"")"#);
        assert_eq!(legacy_to_lua("bogus 1"), None);
    }

    #[test]
    fn slow_commands_are_cut_off() {
        let mut echo = Command::new("sh");
        echo.args(["-c", "echo ok"]);
        assert_eq!(output_within(echo, Duration::from_secs(2)).unwrap().stdout, b"ok\n");

        let mut hang = Command::new("sleep");
        hang.arg("5");
        let start = Instant::now();
        let err = output_within(hang, Duration::from_millis(200)).unwrap_err();
        assert!(start.elapsed() < Duration::from_secs(1), "took {:?}", start.elapsed());
        assert!(err.to_string().contains("no answer"), "{err}");
    }

    #[test]
    fn running_an_unknown_action_says_a_newer_version_is_needed() {
        let act = Action::Unknown { kind: "obs_scene".into(), raw: toml::Table::new() };
        let err = run(&act).unwrap_err().to_string();
        assert!(err.contains("newer omadeck") && err.contains("obs_scene"), "{err}");
    }

    fn marker(name: &str) -> std::path::PathBuf {
        let p = std::env::temp_dir().join(format!("omadeck-test-{}-{name}", std::process::id()));
        let _ = std::fs::remove_file(&p);
        p
    }

    #[test]
    fn a_release_at_shutdown_has_finished_when_run_and_wait_returns() {
        // systemd kills whatever is still in the service when omadeckd exits, so a release
        // started on the way out has to be done before we let go.
        let done = marker("released");
        let act = Action::App { command: format!("sh -c 'sleep 0.2; touch {}'", done.display()) };
        run_and_wait(&act, Duration::from_secs(5)).unwrap();
        let finished = done.exists();
        let _ = std::fs::remove_file(&done);
        assert!(finished, "returned before the release ran");

        let script = marker("script.sh");
        let touched = marker("from-script");
        std::fs::write(&script, format!("sleep 0.2; touch {}\n", touched.display())).unwrap();
        run_and_wait(&Action::Script { path: script.display().to_string(), terminal: false }, Duration::from_secs(5))
            .unwrap();
        let finished = touched.exists();
        let _ = std::fs::remove_file(&script);
        let _ = std::fs::remove_file(&touched);
        assert!(finished, "returned before the script ran");
    }

    #[test]
    fn a_stuck_release_cant_hold_up_shutdown() {
        let start = Instant::now();
        let res = run_and_wait(&Action::App { command: "sleep 5".into() }, Duration::from_millis(200));
        assert!(start.elapsed() < Duration::from_secs(2), "took {:?}", start.elapsed());
        assert!(res.is_err(), "a release that didn't finish is reported");
    }

    /// A fake PATH and XDG data folder, with one program and one desktop entry in them.
    struct FakeSystem {
        root: std::path::PathBuf,
    }

    impl FakeSystem {
        fn new(name: &str) -> Self {
            let root = std::env::temp_dir().join(format!("omadeck-test-{}-{name}", std::process::id()));
            let _ = std::fs::remove_dir_all(&root);
            std::fs::create_dir_all(root.join("bin")).unwrap();
            std::fs::create_dir_all(root.join("share/applications")).unwrap();
            std::fs::write(root.join("bin/foot"), "").unwrap();
            std::fs::write(root.join("share/applications/org.gnome.Nautilus.desktop"), "").unwrap();
            Self { root }
        }

        fn missing(&self, command: &str) -> Option<String> {
            let path = std::ffi::OsString::from(self.root.join("bin"));
            missing_program(command, Some(&path), &[self.root.join("share")])
        }
    }

    impl Drop for FakeSystem {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    #[test]
    fn missing_commands_are_reported() {
        let sys = FakeSystem::new("missing");
        assert_eq!(sys.missing("notarealapp"), Some("notarealapp".into()));
        assert_eq!(sys.missing("notarealapp --flag"), Some("notarealapp".into()));
        assert_eq!(sys.missing("/no/such/program"), Some("/no/such/program".into()));
        assert_eq!(sys.missing("foot"), None);
        assert_eq!(sys.missing("foot -e btop"), None, "only the program itself has to exist");
    }

    #[test]
    fn desktop_ids_are_found_in_xdg_dirs() {
        let sys = FakeSystem::new("desktop");
        assert_eq!(sys.missing("org.gnome.Nautilus.desktop"), None);
        assert_eq!(sys.missing("firefoxx.desktop"), Some("firefoxx.desktop".into()));
    }

    #[test]
    fn absolute_paths_and_shell_commands_are_not_second_guessed() {
        let sys = FakeSystem::new("shell");
        assert_eq!(sys.missing(&sys.root.join("bin/foot").display().to_string()), None);
        assert_eq!(sys.missing("notarealapp | cat"), None, "the shell reports its own errors");
        assert_eq!(sys.missing("FOO=1 $HOME/bin/x"), None);
    }

    #[test]
    fn launching_a_missing_app_says_so() {
        // Before 0.3.0 this went through uwsm-app, which always starts, so nothing was shown.
        let err = run(&Action::App { command: "omadeck-no-such-app-xyz".into() }).unwrap_err().to_string();
        assert!(err.contains("omadeck-no-such-app-xyz") && err.contains("isn't installed"), "{err}");
    }

    #[test]
    fn urls() {
        assert_eq!(normalize_url("omarchy.org"), "https://omarchy.org");
        assert_eq!(normalize_url("http://x"), "http://x");
    }
}
