//! XDG locations used by omadeck.

use std::env;
use std::path::{Path, PathBuf};

fn home() -> PathBuf {
    env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/"))
}

fn xdg(var: &str, fallback: &str) -> PathBuf {
    match env::var_os(var) {
        Some(v) if !v.is_empty() => PathBuf::from(v),
        _ => home().join(fallback),
    }
}

/// `~/.config/omadeck`
pub fn config_dir() -> PathBuf {
    xdg("XDG_CONFIG_HOME", ".config").join("omadeck")
}

/// `~/.local/state/omadeck`: things the app remembers that aren't settings.
pub fn state_dir() -> PathBuf {
    xdg("XDG_STATE_HOME", ".local/state").join("omadeck")
}

/// `~/.config/omarchy`: present when the Omarchy shell is set up for this user.
pub fn omarchy_config_dir() -> PathBuf {
    xdg("XDG_CONFIG_HOME", ".config").join("omarchy")
}

/// `~/.config/omadeck/config.toml`
pub fn config_file() -> PathBuf {
    config_dir().join("config.toml")
}

/// `~/.config/omadeck/icons` — icons imported through the configurator live here so the
/// whole config directory can be copied to another machine.
pub fn icons_dir() -> PathBuf {
    config_dir().join("icons")
}

/// `~/.local/state/omadeck/backups`: earlier copies of config.toml.
pub fn backups_dir() -> PathBuf {
    xdg("XDG_STATE_HOME", ".local/state").join("omadeck/backups")
}

/// `~/.config/fontconfig/fonts.conf`, which `omarchy font set` rewrites.
pub fn fontconfig_file() -> PathBuf {
    xdg("XDG_CONFIG_HOME", ".config").join("fontconfig/fonts.conf")
}

/// The current UTC time as `2026-10-17T031500Z`: sortable and safe in a file name.
pub fn timestamp() -> String {
    utc_stamp(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs()))
}

fn utc_stamp(secs: u64) -> String {
    let (days, rem) = (secs / 86400, secs % 86400);
    // Civil date from days since 1970-01-01 (Howard Hinnant's algorithm).
    let z = days as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}T{:02}{:02}{:02}Z", rem / 3600, rem % 3600 / 60, rem % 60)
}

/// Unix socket the daemon publishes events on: `$XDG_RUNTIME_DIR/omadeck.sock`.
///
/// There is deliberately no fallback: the runtime dir is private to the user (0700), while a
/// shared location such as `/tmp` would let other local users squat on or spoof the socket.
pub fn socket_path() -> anyhow::Result<PathBuf> {
    match env::var_os("XDG_RUNTIME_DIR") {
        Some(dir) if !dir.is_empty() => Ok(PathBuf::from(dir).join("omadeck.sock")),
        _ => anyhow::bail!("XDG_RUNTIME_DIR is not set; omadeck needs a private runtime directory for its socket"),
    }
}

/// Directory Omarchy keeps the active theme in.
pub fn omarchy_current_dir() -> PathBuf {
    xdg("XDG_STATE_HOME", ".local/state").join("omarchy/current")
}

/// Expand a leading `~/` and resolve paths relative to the config dir.
pub fn resolve(path: &str) -> PathBuf {
    if let Some(rest) = path.strip_prefix("~/") {
        return home().join(rest);
    }
    let p = Path::new(path);
    if p.is_absolute() { p.to_path_buf() } else { config_dir().join(p) }
}

/// Inverse of [`resolve`]: store paths inside the config dir relatively, and paths under
/// `$HOME` with `~/`, so configs stay portable.
pub fn portable(path: &Path) -> String {
    if let Ok(rel) = path.strip_prefix(config_dir()) {
        return rel.to_string_lossy().into_owned();
    }
    if let Ok(rel) = path.strip_prefix(home()) {
        return format!("~/{}", rel.to_string_lossy());
    }
    path.to_string_lossy().into_owned()
}

/// Identity and modification time of a file, following symlinks. Comparing two stamps tells
/// whether the file was edited or replaced, even when no filesystem event arrived (a symlink
/// to a file elsewhere, or a config directory that was deleted and recreated).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileStamp {
    dev: u64,
    ino: u64,
    len: u64,
    mtime: (i64, i64),
}

impl FileStamp {
    /// `None` when the file doesn't exist (or can't be read).
    pub fn of(path: &Path) -> Option<Self> {
        use std::os::unix::fs::MetadataExt;
        let m = std::fs::metadata(path).ok()?;
        Some(Self { dev: m.dev(), ino: m.ino(), len: m.len(), mtime: (m.mtime(), m.mtime_nsec()) })
    }

    /// Whether both stamps are of the same file, edited or not.
    pub fn same_file(a: Option<Self>, b: Option<Self>) -> bool {
        a.map(|s| (s.dev, s.ino)) == b.map(|s| (s.dev, s.ino))
    }
}

#[cfg(test)]
mod tests {
    use super::utc_stamp;

    #[test]
    fn stamps() {
        assert_eq!(utc_stamp(0), "1970-01-01T000000Z");
        assert_eq!(utc_stamp(951782400), "2000-02-29T000000Z");
        assert_eq!(utc_stamp(1792209096), "2026-10-17T035136Z");
        assert_eq!(utc_stamp(4107542399), "2100-02-28T235959Z");
    }
}
