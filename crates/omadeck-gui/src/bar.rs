//! omadeck's widget for the Omarchy bar: an Omarchy shell plugin that shows the deck's status
//! and opens this app when clicked. Its files are built into the app and written into the
//! user's Omarchy config, so a system package never has to touch anyone's home folder.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};
use omadeck_core::paths;

/// The plugin's id, which is also its folder name under `~/.config/omarchy/plugins`.
pub const ID: &str = "dev.omadeck";

const MANIFEST: &str = include_str!("../../../packaging/omarchy-bar/manifest.json");
const WIDGET: &str = include_str!("../../../packaging/omarchy-bar/Widget.qml");

/// Runs a command and returns what it printed; tests pass a fake one.
pub type Runner<'a> = &'a dyn Fn(&[&str]) -> Result<String>;

/// Runs commands for real.
pub fn run_command(argv: &[&str]) -> Result<String> {
    let out = Command::new(argv[0]).args(&argv[1..]).output().with_context(|| format!("running {}", argv[0]))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        bail!("{}", err.trim().lines().last().unwrap_or("failed"));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Whether this desktop has the Omarchy shell to put a widget in.
pub fn available() -> bool {
    on_path("omarchy") && on_path("omarchy-shell") && paths::omarchy_config_dir().is_dir()
}

fn on_path(bin: &str) -> bool {
    std::env::var_os("PATH").is_some_and(|p| std::env::split_paths(&p).any(|d| d.join(bin).is_file()))
}

/// `~/.config/omarchy/plugins/dev.omadeck`
pub fn plugin_dir() -> PathBuf {
    paths::omarchy_config_dir().join("plugins").join(ID)
}

/// The `omadeckd` the widget should run: the one installed next to this app, so the widget
/// doesn't depend on the shell's PATH.
pub fn daemon_path() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|d| d.join("omadeckd")))
        .filter(|p| p.is_file())
        .unwrap_or_else(|| PathBuf::from("omadeckd"))
}

/// The plugin's files, ready to write.
fn files(omadeckd: &Path) -> [(&'static str, String); 2] {
    [
        ("manifest.json", MANIFEST.replace("@VERSION@", env!("CARGO_PKG_VERSION"))),
        ("Widget.qml", WIDGET.replace("@OMADECKD@", &omadeckd.to_string_lossy())),
    ]
}

/// Write the plugin into `dir`, rewriting only files that changed (Omarchy reloads a plugin
/// whenever one of its files is saved). Returns whether anything was written.
pub fn install(dir: &Path, omadeckd: &Path) -> Result<bool> {
    std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    let mut changed = false;
    for (name, body) in files(omadeckd) {
        let path = dir.join(name);
        if std::fs::read_to_string(&path).ok().as_deref() != Some(body.as_str()) {
            std::fs::write(&path, body).with_context(|| format!("writing {}", path.display()))?;
            changed = true;
        }
    }
    Ok(changed)
}

/// Put the widget on the bar: Omarchy finds the new plugin, then turns it on.
pub fn enable(run: Runner) -> Result<()> {
    run(&["omarchy-shell", "shell", "rescanPlugins"])?;
    run(&["omarchy", "plugin", "enable", ID])?;
    Ok(())
}

/// Take the widget off the bar. Its files stay, so turning it back on is instant.
pub fn disable(run: Runner) -> Result<()> {
    run(&["omarchy", "plugin", "disable", ID])?;
    Ok(())
}

/// Whether the widget is on the bar right now.
pub fn enabled(run: Runner) -> Result<bool> {
    let list: serde_json::Value =
        serde_json::from_str(&run(&["omarchy", "plugin", "list", "--json"])?).context("reading the plugin list")?;
    Ok(list.as_array().into_iter().flatten().any(|p| p["id"] == ID && p["enabled"] == true))
}

/// Remembers that the user was asked about the widget, so they're asked only once.
pub fn asked_marker() -> PathBuf {
    paths::state_dir().join("bar-asked")
}

/// Ask on first run only when there's an Omarchy bar and the question hasn't been asked before.
pub fn should_ask(available: bool, marker: &Path) -> bool {
    available && !marker.exists()
}

pub fn remember_asked(marker: &Path) {
    if let Some(dir) = marker.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(marker, "");
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    fn temp(name: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("omadeck-test-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        let _ = std::fs::remove_file(&p);
        p
    }

    #[test]
    fn install_writes_the_manifest_and_widget_with_the_daemon_path() {
        let dir = temp("plugin");
        assert!(install(&dir, Path::new("/opt/x/omadeckd")).unwrap());
        let widget = std::fs::read_to_string(dir.join("Widget.qml")).unwrap();
        let manifest = std::fs::read_to_string(dir.join("manifest.json")).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
        assert!(widget.contains(r#"command: ["/opt/x/omadeckd", "status", "--follow"]"#), "{widget}");
        assert!(!widget.contains('@') && !manifest.contains('@'), "every placeholder is filled in");
        let m: serde_json::Value = serde_json::from_str(&manifest).unwrap();
        assert_eq!(m["version"], env!("CARGO_PKG_VERSION"));
    }

    #[test]
    fn install_leaves_unchanged_files_alone() {
        let dir = temp("plugin-again");
        let daemon = Path::new("/usr/bin/omadeckd");
        assert!(install(&dir, daemon).unwrap());
        let before = std::fs::metadata(dir.join("Widget.qml")).unwrap().modified().unwrap();
        assert!(!install(&dir, daemon).unwrap(), "nothing to rewrite, so Omarchy needn't reload it");
        let after = std::fs::metadata(dir.join("Widget.qml")).unwrap().modified().unwrap();
        assert!(install(&dir, Path::new("/elsewhere/omadeckd")).unwrap(), "a moved daemon is picked up");
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(before, after);
    }

    #[test]
    fn the_bundled_manifest_is_valid_json_with_the_required_fields() {
        let m: serde_json::Value = serde_json::from_str(&files(Path::new("omadeckd"))[0].1).unwrap();
        assert_eq!(m["schemaVersion"], 1);
        assert_eq!(m["id"], ID);
        assert_eq!(m["kinds"], serde_json::json!(["bar-widget"]));
        assert_eq!(m["entryPoints"]["barWidget"], "Widget.qml");
        assert_eq!(m["barWidget"]["defaultSection"], "right");
    }

    #[test]
    fn enable_and_disable_run_the_right_commands() {
        let calls = RefCell::new(Vec::new());
        let fake = |argv: &[&str]| -> Result<String> {
            calls.borrow_mut().push(argv.join(" "));
            Ok("ok".into())
        };
        enable(&fake).unwrap();
        disable(&fake).unwrap();
        assert_eq!(
            *calls.borrow(),
            [
                "omarchy-shell shell rescanPlugins",
                "omarchy plugin enable dev.omadeck",
                "omarchy plugin disable dev.omadeck"
            ]
        );
    }

    #[test]
    fn a_failed_command_is_reported() {
        let fails = |_: &[&str]| -> Result<String> { bail!("plugin 'dev.omadeck' is not known") };
        assert!(enable(&fails).is_err());
        assert!(enabled(&fails).is_err());
    }

    #[test]
    fn enabled_reads_omarchy_plugin_list() {
        let list = include_str!("fixtures/plugin-list.json");
        assert!(enabled(&|_: &[&str]| Ok(list.to_string())).unwrap());
        let off = list.replace(
            r#""id":"dev.omadeck","name":"omadeck","kinds":["bar-widget"],"enabled":true"#,
            r#""id":"dev.omadeck","name":"omadeck","kinds":["bar-widget"],"enabled":false"#,
        );
        assert_ne!(off, list, "the fixture changed shape");
        assert!(!enabled(&|_: &[&str]| Ok(off.clone())).unwrap());
        assert!(!enabled(&|_: &[&str]| Ok("[]".into())).unwrap(), "not installed is off");
    }

    #[test]
    fn first_run_asks_once() {
        let marker = temp("bar-asked");
        assert!(should_ask(true, &marker));
        assert!(!should_ask(false, &marker), "no Omarchy bar, no question");
        remember_asked(&marker);
        assert!(!should_ask(true, &marker), "asked already");
        let _ = std::fs::remove_file(&marker);
    }
}
