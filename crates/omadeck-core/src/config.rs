//! The on-disk configuration (`~/.config/omadeck/config.toml`).

use std::fs;
use std::io::Write;
use std::path::Path;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::paths;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Config {
    /// Panel brightness, 0–100.
    #[serde(default = "default_brightness")]
    pub brightness: u8,
    /// Global look; per-key settings override it.
    #[serde(default)]
    pub style: Style,
    /// The home page: what the deck shows at startup.
    #[serde(default, rename = "key", skip_serializing_if = "Keys::is_empty")]
    pub keys: Keys,
    /// More pages, opened from keys with a `page` action.
    #[serde(default, rename = "page", skip_serializing_if = "Vec::is_empty")]
    pub pages: Vec<Page>,
    /// Settings from a newer omadeck, kept so saving doesn't lose them.
    #[serde(flatten, default, skip_serializing_if = "toml::Table::is_empty")]
    pub extra: toml::Table,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            brightness: default_brightness(),
            style: Style::default(),
            keys: Keys::default(),
            pages: Vec::new(),
            extra: toml::Table::new(),
        }
    }
}

/// A named page of keys. Pages are flat: a "folder" is just a page with a Back key.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Page {
    pub name: String,
    #[serde(default, rename = "key", skip_serializing_if = "Keys::is_empty")]
    pub keys: Keys,
    #[serde(flatten, default, skip_serializing_if = "toml::Table::is_empty")]
    pub extra: toml::Table,
}

impl Page {
    pub fn new(name: impl Into<String>) -> Self {
        Self { name: name.into(), keys: Keys::default(), extra: toml::Table::new() }
    }
}

/// The keys on one page. Keys that aren't listed are blank.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Keys(pub Vec<KeyConfig>);

impl Keys {
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn iter(&self) -> std::slice::Iter<'_, KeyConfig> {
        self.0.iter()
    }

    pub fn get(&self, index: u8) -> Option<&KeyConfig> {
        self.0.iter().find(|k| k.index == index)
    }

    /// Get a key for editing, creating it if needed.
    pub fn get_mut(&mut self, index: u8) -> &mut KeyConfig {
        if let Some(pos) = self.0.iter().position(|k| k.index == index) {
            &mut self.0[pos]
        } else {
            self.0.push(KeyConfig::new(index));
            self.0.last_mut().unwrap()
        }
    }

    pub fn clear(&mut self, index: u8) {
        self.0.retain(|k| k.index != index);
    }

    pub fn swap(&mut self, a: u8, b: u8) {
        for k in &mut self.0 {
            if k.index == a {
                k.index = b;
            } else if k.index == b {
                k.index = a;
            }
        }
    }

    /// Drop blank keys and sort by index, for a tidy file.
    fn tidy(&mut self) {
        self.0.retain(|k| !k.is_blank());
        self.0.sort_by_key(|k| k.index);
    }
}

fn default_brightness() -> u8 {
    70
}

fn yes() -> bool {
    true
}

fn is_true(b: &bool) -> bool {
    *b
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Style {
    /// Take default colours from the active Omarchy theme (re-rendered when the theme changes).
    #[serde(default = "yes")]
    pub follow_theme: bool,
    /// Default key background, `#rrggbb`. Overrides the theme.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub background: Option<String>,
    /// Default label colour, `#rrggbb`. Overrides the theme.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label_color: Option<String>,
    /// fontconfig pattern for labels. Defaults to the Omarchy monospace font.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub font: Option<String>,
    #[serde(flatten, default, skip_serializing_if = "toml::Table::is_empty")]
    pub extra: toml::Table,
}

impl Default for Style {
    fn default() -> Self {
        Self { follow_theme: true, background: None, label_color: None, font: None, extra: toml::Table::new() }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct KeyConfig {
    /// Zero-based key index, left-to-right, top-to-bottom.
    pub index: u8,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub label: String,
    #[serde(default = "yes", skip_serializing_if = "is_true")]
    pub show_label: bool,
    /// PNG/JPEG/SVG/… path. Relative paths resolve against the config dir.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    /// A Nerd Font symbol drawn as the icon in the theme accent colour (used when `icon` is unset).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub glyph: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub background: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label_color: Option<String>,
    #[serde(default, with = "keep_unknown")]
    pub action: Action,
    /// Run when the key is let go, e.g. to stop what `action` started (push-to-talk).
    #[serde(default, skip_serializing_if = "Option::is_none", with = "keep_unknown::option")]
    pub on_release: Option<Action>,
    #[serde(flatten, default, skip_serializing_if = "toml::Table::is_empty")]
    pub extra: toml::Table,
}

impl KeyConfig {
    pub fn new(index: u8) -> Self {
        Self {
            index,
            label: String::new(),
            show_label: true,
            icon: None,
            glyph: None,
            background: None,
            label_color: None,
            action: Action::None,
            on_release: None,
            extra: toml::Table::new(),
        }
    }

    /// True when the key carries nothing worth saving.
    pub fn is_blank(&self) -> bool {
        self.label.is_empty()
            && self.icon.is_none()
            && self.glyph.is_none()
            && self.background.is_none()
            && self.label_color.is_none()
            && self.action == Action::None
            && self.on_release.is_none()
            && self.extra.is_empty()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Action {
    #[default]
    None,
    /// Launch an application: a desktop entry id (`firefox.desktop`) or a command line.
    App { command: String },
    /// Run a script or executable.
    Script {
        path: String,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        terminal: bool,
    },
    /// Play a sound file through PipeWire.
    Sound {
        file: String,
        /// 0.0–1.0
        #[serde(default, skip_serializing_if = "Option::is_none")]
        volume: Option<f32>,
    },
    /// Open a website in the default browser.
    Url { url: String },
    /// Run a Hyprland dispatcher, e.g. `workspace 3` or `togglefloating`.
    Hyprland { dispatch: String },
    /// Show another page of keys on the deck.
    Page { page: String },
    /// Return to the page shown before this one (Home if there is none).
    Back,
    /// An action from a newer omadeck. It's kept exactly as written and does nothing here.
    /// (Read and written by `keep_unknown`; serde never sees it directly.)
    #[serde(skip)]
    Unknown { kind: String, raw: toml::Table },
}

/// Serde for an action field that keeps action types this version doesn't know, instead of
/// rejecting the whole config: a key from a newer omadeck shouldn't blank the deck.
mod keep_unknown {
    use super::Action;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S: Serializer>(action: &Action, s: S) -> Result<S::Ok, S::Error> {
        match action {
            Action::Unknown { raw, .. } => raw.serialize(s),
            known => known.serialize(s),
        }
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Action, D::Error> {
        let raw = toml::Table::deserialize(d)?;
        match raw.get("type").and_then(toml::Value::as_str) {
            Some(kind) if !Action::TYPES.contains(&kind) => Ok(Action::Unknown { kind: kind.to_string(), raw }),
            // Known types, and a missing `type`, report their errors as before.
            _ => Action::deserialize(toml::Value::Table(raw)).map_err(serde::de::Error::custom),
        }
    }

    /// The same for an optional action. Only called when the field is present.
    pub mod option {
        use super::Action;
        use serde::{Deserializer, Serializer};

        pub fn serialize<S: Serializer>(action: &Option<Action>, s: S) -> Result<S::Ok, S::Error> {
            match action {
                Some(a) => super::serialize(a, s),
                None => s.serialize_none(),
            }
        }

        pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Action>, D::Error> {
            super::deserialize(d).map(Some)
        }
    }
}

impl Action {
    pub const KINDS: [&'static str; 8] =
        ["Nothing", "Launch app", "Run script", "Play sound", "Open website", "Hyprland", "Open page", "Go back"];

    /// Every `type` this version understands, in `KINDS` order.
    pub const TYPES: [&'static str; 8] = ["none", "app", "script", "sound", "url", "hyprland", "page", "back"];

    /// What `kind_index` returns for an action from a newer omadeck (not in `KINDS`).
    pub const UNKNOWN: usize = Self::KINDS.len();

    pub fn kind_index(&self) -> usize {
        match self {
            Action::None => 0,
            Action::App { .. } => 1,
            Action::Script { .. } => 2,
            Action::Sound { .. } => 3,
            Action::Url { .. } => 4,
            Action::Hyprland { .. } => 5,
            Action::Page { .. } => 6,
            Action::Back => 7,
            Action::Unknown { .. } => Self::UNKNOWN,
        }
    }

    /// Page navigation is handled by whoever owns the deck, not by `action::run`.
    pub fn is_navigation(&self) -> bool {
        matches!(self, Action::Page { .. } | Action::Back)
    }

    pub fn empty_of_kind(i: usize) -> Self {
        match i {
            1 => Action::App { command: String::new() },
            2 => Action::Script { path: String::new(), terminal: false },
            3 => Action::Sound { file: String::new(), volume: None },
            4 => Action::Url { url: String::new() },
            5 => Action::Hyprland { dispatch: String::new() },
            6 => Action::Page { page: String::new() },
            7 => Action::Back,
            _ => Action::None,
        }
    }

    pub fn summary(&self) -> String {
        match self {
            Action::None => "Does nothing".into(),
            Action::App { command } => format!("Launch {command}"),
            Action::Script { path, .. } => format!("Run {path}"),
            Action::Sound { file, .. } => format!("Play {file}"),
            Action::Url { url } => format!("Open {url}"),
            Action::Hyprland { dispatch } => format!("hyprctl dispatch {dispatch}"),
            Action::Page { page } => format!("Open page {page}"),
            Action::Back => "Go back".into(),
            Action::Unknown { kind, .. } => format!("{kind} (needs a newer omadeck)"),
        }
    }
}

impl Config {
    pub fn load() -> Result<Self> {
        Self::load_from(&paths::config_file())
    }

    /// Missing file → default config.
    pub fn load_from(path: &Path) -> Result<Self> {
        match fs::read_to_string(path) {
            Ok(s) => Self::parse(&s).with_context(|| format!("parsing {}", path.display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e).with_context(|| format!("reading {}", path.display())),
        }
    }

    pub fn parse(src: &str) -> Result<Self> {
        Ok(toml::from_str(src)?)
    }

    /// Saves and returns exactly what was written.
    pub fn save(&self) -> Result<String> {
        self.save_to(&paths::config_file())
    }

    /// Atomic write (temp file + rename) so the daemon never reads a half-written file.
    /// A symlinked config (dotfile managers) is written through, keeping the link.
    pub fn save_to(&self, path: &Path) -> Result<String> {
        let mut clean = self.clone();
        clean.keys.tidy();
        for p in &mut clean.pages {
            p.keys.tidy();
        }
        let body = format!(
            "# omadeck configuration - edit by hand or with the `omadeck` app.\n\n{}",
            toml::to_string_pretty(&clean)?
        );
        let link = fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink());
        let path = if link {
            fs::canonicalize(path).with_context(|| format!("following {}", path.display()))?
        } else {
            path.to_path_buf()
        };
        let dir = path.parent().context("config path has no parent")?;
        fs::create_dir_all(dir)?;
        let tmp = dir.join(".config.toml.tmp");
        let mut f = fs::File::create(&tmp)?;
        f.write_all(body.as_bytes())?;
        f.sync_all()?;
        fs::rename(&tmp, &path)?;
        Ok(body)
    }

    /// The keys of a page: `None` is the home page.
    pub fn page(&self, page: Option<&str>) -> Option<&Keys> {
        match page {
            None => Some(&self.keys),
            Some(name) => self.pages.iter().find(|p| p.name == name).map(|p| &p.keys),
        }
    }

    pub fn page_mut(&mut self, page: Option<&str>) -> Option<&mut Keys> {
        match page {
            None => Some(&mut self.keys),
            Some(name) => self.pages.iter_mut().find(|p| p.name == name).map(|p| &mut p.keys),
        }
    }

    pub fn key(&self, page: Option<&str>, index: u8) -> Option<&KeyConfig> {
        self.page(page)?.get(index)
    }

    pub fn has_page(&self, name: &str) -> bool {
        self.pages.iter().any(|p| p.name == name)
    }

    pub fn page_names(&self) -> Vec<String> {
        self.pages.iter().map(|p| p.name.clone()).collect()
    }

    /// Why `name` can't be used for a new or renamed page, if it can't.
    pub fn page_name_problem(&self, name: &str) -> Option<&'static str> {
        let name = name.trim();
        if name.is_empty() {
            Some("Give the page a name")
        } else if name.eq_ignore_ascii_case(HOME) {
            Some("“Home” is the main page's name")
        } else if self.has_page(name) {
            Some("There's already a page with that name")
        } else {
            None
        }
    }

    /// `base` if it's free, otherwise `base 2`, `base 3`, …
    pub fn unique_page_name(&self, base: &str) -> String {
        let base = base.trim();
        let base = if base.is_empty() || base.eq_ignore_ascii_case(HOME) { "Page" } else { base };
        (1..)
            .map(|n| if n == 1 { base.to_string() } else { format!("{base} {n}") })
            .find(|n| !self.has_page(n))
            .unwrap()
    }

    fn all_keys_mut(&mut self) -> impl Iterator<Item = &mut KeyConfig> {
        self.keys.0.iter_mut().chain(self.pages.iter_mut().flat_map(|p| p.keys.0.iter_mut()))
    }

    /// How many keys open the page `name`.
    pub fn links_to(&self, name: &str) -> usize {
        self.keys
            .iter()
            .chain(self.pages.iter().flat_map(|p| p.keys.iter()))
            .filter(|k| matches!(&k.action, Action::Page { page } if page == name))
            .count()
    }

    /// Rename a page and every key that opens it.
    pub fn rename_page(&mut self, old: &str, new: &str) {
        let new = new.trim();
        for p in &mut self.pages {
            if p.name == old {
                p.name = new.to_string();
            }
        }
        for k in self.all_keys_mut() {
            if let Action::Page { page } = &mut k.action
                && page == old
            {
                *page = new.to_string();
            }
        }
    }

    /// Delete a page. Keys that opened it keep their look but no longer do anything.
    /// Returns what [`Config::restore_page`] needs to undo it.
    pub fn remove_page(&mut self, name: &str) -> Option<RemovedPage> {
        let pos = self.pages.iter().position(|p| p.name == name)?;
        let page = self.pages.remove(pos);
        let mut links = Vec::new();
        let home = self.keys.0.iter_mut().map(|k| (None, k));
        let others = self.pages.iter_mut().flat_map(|p| p.keys.0.iter_mut().map(|k| (Some(p.name.clone()), k)));
        for (on, k) in home.chain(others) {
            if matches!(&k.action, Action::Page { page } if page == name) {
                k.action = Action::None;
                links.push((on, k.index));
            }
        }
        Some(RemovedPage { pos, page, links })
    }

    /// Undo [`Config::remove_page`]: put the page back where it was and re-link the keys
    /// that opened it, unless they've been given another job since.
    pub fn restore_page(&mut self, removed: RemovedPage) -> bool {
        let RemovedPage { pos, page, links } = removed;
        if self.has_page(&page.name) {
            return false;
        }
        let name = page.name.clone();
        self.pages.insert(pos.min(self.pages.len()), page);
        for (on, index) in links {
            if let Some(k) = self.page_mut(on.as_deref()).and_then(|keys| keys.0.iter_mut().find(|k| k.index == index))
                && k.action == Action::None
            {
                k.action = Action::Page { page: name.clone() };
            }
        }
        true
    }

    /// Whether the deck can leave `page`: it needs a Back key or a key that opens a page.
    /// Home never needs one.
    pub fn has_way_out(&self, page: Option<&str>) -> bool {
        page.is_none()
            || self
                .page(page)
                .is_none_or(|keys| keys.iter().any(|k| matches!(k.action, Action::Back | Action::Page { .. })))
    }
}

/// A deleted page, kept so the deletion can be undone.
#[derive(Debug, Clone)]
pub struct RemovedPage {
    pos: usize,
    pub page: Page,
    /// Keys (page, index) that opened it.
    links: Vec<(Option<String>, u8)>,
}

/// How many backups of config.toml to keep.
const BACKUPS: usize = 20;

/// Copy config.toml into the backups folder (`~/.local/state/omadeck/backups`), unless the
/// newest backup already matches it, and prune old ones. Returns the new backup's path.
pub fn backup() -> Result<Option<std::path::PathBuf>> {
    let Ok(body) = fs::read(paths::config_file()) else { return Ok(None) };
    let dir = paths::backups_dir();
    fs::create_dir_all(&dir)?;
    let mut old: Vec<_> = fs::read_dir(&dir)?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with("config-") && n.ends_with(".toml"))
        })
        .collect();
    old.sort();
    let mut made = None;
    if !old.last().is_some_and(|p| fs::read(p).is_ok_and(|b| b == body)) {
        let path = dir.join(format!("config-{}.toml", paths::timestamp()));
        fs::write(&path, &body)?;
        old.push(path.clone());
        made = Some(path);
    }
    let excess = old.len().saturating_sub(BACKUPS);
    for p in &old[..excess] {
        let _ = fs::remove_file(p);
    }
    Ok(made)
}

/// What the home page is called in the UI. It has no name in the config file.
pub const HOME: &str = "Home";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let mut c = Config::default();
        c.keys.get_mut(0).label = "Term".into();
        c.keys.get_mut(0).action = Action::App { command: "Alacritty.desktop".into() };
        c.keys.get_mut(3).action = Action::Sound { file: "~/a.wav".into(), volume: Some(0.5) };
        c.keys.get_mut(4).action = Action::Script { path: "x.sh".into(), terminal: true };
        c.keys.get_mut(5).action = Action::Page { page: "Media".into() };
        let mut media = Page::new("Media");
        media.keys.get_mut(0).action = Action::Back;
        media.keys.get_mut(1).label = "Play".into();
        c.pages.push(media);
        c.pages.push(Page::new("Empty"));
        let s = toml::to_string_pretty(&c).unwrap();
        let back: Config = toml::from_str(&s).unwrap();
        assert_eq!(c, back);
    }

    #[test]
    fn swap() {
        let mut c = Config::default();
        c.keys.get_mut(1).label = "a".into();
        c.keys.get_mut(2).label = "b".into();
        c.keys.swap(1, 2);
        assert_eq!(c.key(None, 1).unwrap().label, "b");
        assert_eq!(c.key(None, 2).unwrap().label, "a");
    }

    #[test]
    fn pages() {
        let mut c = Config::default();
        c.pages.push(Page::new("Media"));
        c.keys.get_mut(0).action = Action::Page { page: "Media".into() };
        c.page_mut(Some("Media")).unwrap().get_mut(2).action = Action::Page { page: "Media".into() };
        assert_eq!(c.links_to("Media"), 2);
        assert!(c.key(Some("Nope"), 0).is_none());

        assert_eq!(c.unique_page_name("Media"), "Media 2");
        assert_eq!(c.unique_page_name("home"), "Page");
        assert!(c.page_name_problem(" Media ").is_some());
        assert!(c.page_name_problem("HOME").is_some());
        assert!(c.page_name_problem("Games").is_none());

        c.rename_page("Media", "Music");
        assert!(c.has_page("Music") && !c.has_page("Media"));
        assert_eq!(c.key(None, 0).unwrap().action, Action::Page { page: "Music".into() });
        assert_eq!(c.links_to("Music"), 2);

        c.remove_page("Music");
        assert!(c.pages.is_empty());
        assert_eq!(c.key(None, 0).unwrap().action, Action::None);
    }

    #[test]
    fn save_tidies_every_page() {
        let dir = std::env::temp_dir().join(format!("omadeck-test-{}", std::process::id()));
        let path = dir.join("config.toml");
        let mut c = Config::default();
        c.keys.get_mut(4).label = "b".into();
        c.keys.get_mut(1).label = "a".into();
        c.pages.push(Page::new("P"));
        c.page_mut(Some("P")).unwrap().get_mut(3);
        c.save_to(&path).unwrap();
        let back = Config::load_from(&path).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(back.keys.iter().map(|k| k.index).collect::<Vec<_>>(), [1, 4]);
        assert!(back.page(Some("P")).unwrap().is_empty(), "blank keys are dropped, empty pages kept");
    }

    #[test]
    fn undo_page_delete() {
        let mut c = Config::default();
        for n in ["A", "B", "C"] {
            c.pages.push(Page::new(n));
        }
        c.keys.get_mut(0).action = Action::Page { page: "B".into() };
        c.page_mut(Some("A")).unwrap().get_mut(2).action = Action::Page { page: "B".into() };
        c.page_mut(Some("B")).unwrap().get_mut(1).label = "inside".into();
        let before = c.clone();

        let removed = c.remove_page("B").unwrap();
        assert!(!c.has_page("B"));
        assert_eq!(c.links_to("B"), 0);
        assert!(c.restore_page(removed));
        assert_eq!(c, before, "page back in place with its keys and links");

        // A link that was given another job meanwhile is left alone.
        let removed = c.remove_page("B").unwrap();
        c.keys.get_mut(0).action = Action::Back;
        c.restore_page(removed);
        assert_eq!(c.keys.get(0).unwrap().action, Action::Back);
        assert_eq!(c.links_to("B"), 1);
        assert!(c.remove_page("nope").is_none());
    }

    #[test]
    fn way_out() {
        let mut c = Config::default();
        c.pages.push(Page::new("P"));
        assert!(c.has_way_out(None));
        assert!(!c.has_way_out(Some("P")));
        c.page_mut(Some("P")).unwrap().get_mut(3).action = Action::Page { page: "P".into() };
        assert!(c.has_way_out(Some("P")));
    }

    #[test]
    fn save_keeps_a_symlinked_config() {
        let dir = std::env::temp_dir().join(format!("omadeck-link-{}", std::process::id()));
        let real = dir.join("dotfiles/omadeck.toml");
        let link = dir.join("config.toml");
        std::fs::create_dir_all(real.parent().unwrap()).unwrap();
        std::fs::write(&real, "").unwrap();
        std::os::unix::fs::symlink(&real, &link).unwrap();
        let mut c = Config::default();
        c.keys.get_mut(0).label = "via link".into();
        c.save_to(&link).unwrap();
        let still_link = std::fs::symlink_metadata(&link).unwrap().file_type().is_symlink();
        let back = Config::load_from(&real).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
        assert!(still_link);
        assert_eq!(back.keys.iter().next().unwrap().label, "via link");
    }

    const FUTURE: &str = include_str!("fixtures/future.toml");

    fn temp_file(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("omadeck-test-{}-{name}", std::process::id()))
    }

    #[test]
    fn unknown_action_type_loads_and_the_rest_of_the_deck_still_works() {
        let c = Config::parse(FUTURE).expect("a newer config still loads");
        assert_eq!(c.brightness, 60);
        assert_eq!(c.key(None, 0).unwrap().action, Action::App { command: "xdg-terminal-exec".into() });
        assert_eq!(c.key(None, 1).unwrap().label, "Scene");
        assert_eq!(c.key(Some("Media"), 0).unwrap().action, Action::Back);
    }

    #[test]
    fn unknown_fields_survive_a_save_at_every_level() {
        let path = temp_file("future.toml");
        Config::parse(FUTURE).unwrap().save_to(&path).unwrap();
        let saved = std::fs::read_to_string(&path).unwrap();
        let _ = std::fs::remove_file(&path);
        let want: toml::Table = toml::from_str(FUTURE).unwrap();
        let got: toml::Table = toml::from_str(&saved).unwrap();
        assert_eq!(got, want, "saved file:\n{saved}");
    }

    #[test]
    fn unknown_action_survives_a_save_unchanged() {
        let c = Config::parse(FUTURE).unwrap();
        let back = Config::parse(&toml::to_string_pretty(&c).unwrap()).unwrap();
        assert_eq!(back, c);
        let stinger = &back.key(Some("Media"), 1).unwrap().action;
        let text = toml::to_string(&back).unwrap();
        assert!(text.contains("point_ms = 900"), "nested settings kept: {stinger:?}");
    }

    #[test]
    fn every_known_kind_is_listed_in_types() {
        for (i, ty) in Action::TYPES.iter().enumerate() {
            let table = toml::Table::try_from(Action::empty_of_kind(i)).unwrap();
            assert_eq!(table["type"].as_str(), Some(*ty), "TYPES[{i}] drifted from the enum");
            assert_eq!(Action::empty_of_kind(i).kind_index(), i);
        }
    }

    #[test]
    fn a_mistake_in_a_known_action_is_still_an_error() {
        let missing_field = "[[key]]\nindex = 0\n[key.action]\ntype = \"url\"\n";
        let err = Config::parse(missing_field).unwrap_err();
        assert!(format!("{err:#}").contains("url"), "{err:#}");
        let no_type = "[[key]]\nindex = 0\n[key.action]\ncommand = \"x\"\n";
        assert!(Config::parse(no_type).is_err());
    }

    #[test]
    fn on_release_roundtrips() {
        let src = "[[key]]\nindex = 2\nlabel = \"Talk\"\n\n[key.action]\ntype = \"app\"\ncommand = \"voxtype record start\"\n\n\
                   [key.on_release]\ntype = \"app\"\ncommand = \"voxtype record stop\"\n";
        let c = Config::parse(src).unwrap();
        let k = c.key(None, 2).unwrap();
        assert_eq!(k.on_release, Some(Action::App { command: "voxtype record stop".into() }));
        assert_eq!(Config::parse(&toml::to_string_pretty(&c).unwrap()).unwrap(), c);
        // Keys without one don't grow an empty table.
        assert!(!toml::to_string(&Config::parse(FUTURE).unwrap()).unwrap().contains("on_release"));
    }

    #[test]
    fn an_unknown_release_action_is_kept_too() {
        let src = "[[key]]\nindex = 0\n[key.on_release]\ntype = \"obs_scene\"\nscene = \"Off\"\n";
        let c = Config::parse(src).unwrap();
        assert!(
            matches!(&c.key(None, 0).unwrap().on_release, Some(Action::Unknown { kind, .. }) if kind == "obs_scene")
        );
        assert!(toml::to_string(&c).unwrap().contains("scene = \"Off\""));
    }

    #[test]
    fn a_key_with_only_a_release_action_is_not_blank() {
        let mut k = KeyConfig::new(0);
        assert!(k.is_blank());
        k.on_release = Some(Action::App { command: "voxtype record stop".into() });
        assert!(!k.is_blank(), "saving would drop it");
    }

    #[test]
    fn known_configs_save_exactly_as_before() {
        let path = temp_file("example.toml");
        // 0.2.0's example config, and exactly what 0.2.0 wrote when it saved it.
        let c = Config::parse(include_str!("fixtures/v0.2.toml")).unwrap();
        let saved = c.save_to(&path).unwrap();
        let _ = std::fs::remove_file(&path);
        assert_eq!(saved, include_str!("fixtures/v0.2.saved.toml"));
    }

    #[test]
    fn example_parses() {
        let s = include_str!("../../../examples/config.toml");
        let c: Config = toml::from_str(s).unwrap();
        assert!(!c.keys.is_empty());
    }
}
