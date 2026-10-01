//! Colours: `#rrggbb` parsing and the active Omarchy theme palette.

use std::fs;

use crate::paths;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rgb(pub u8, pub u8, pub u8);

impl Rgb {
    pub fn parse(s: &str) -> Option<Self> {
        let h = s.trim().trim_start_matches('#');
        let h = if h.len() == 8 { &h[..6] } else { h }; // ignore alpha
        if h.len() != 6 {
            return None;
        }
        let v = u32::from_str_radix(h, 16).ok()?;
        Some(Rgb((v >> 16) as u8, (v >> 8) as u8, v as u8))
    }

    pub fn hex(self) -> String {
        format!("#{:02x}{:02x}{:02x}", self.0, self.1, self.2)
    }

    pub fn luminance(self) -> f32 {
        (0.2126 * self.0 as f32 + 0.7152 * self.1 as f32 + 0.0722 * self.2 as f32) / 255.0
    }

    pub fn mix(self, other: Rgb, t: f32) -> Rgb {
        let m = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * t).round() as u8;
        Rgb(m(self.0, other.0), m(self.1, other.1), m(self.2, other.2))
    }
}

/// The handful of palette entries omadeck uses.
#[derive(Debug, Clone, PartialEq)]
pub struct Theme {
    pub name: String,
    pub background: Rgb,
    pub foreground: Rgb,
    pub accent: Rgb,
    pub muted: Rgb,
}

impl Default for Theme {
    /// Tokyo Night-ish fallback when no Omarchy theme is present.
    fn default() -> Self {
        Self {
            name: "default".into(),
            background: Rgb(0x1a, 0x1b, 0x26),
            foreground: Rgb(0xc0, 0xca, 0xf5),
            accent: Rgb(0x7a, 0xa2, 0xf7),
            muted: Rgb(0x41, 0x48, 0x68),
        }
    }
}

impl Theme {
    /// Read `~/.local/state/omarchy/current/theme/colors.toml`; falls back to defaults.
    pub fn load_omarchy() -> Self {
        let dir = paths::omarchy_current_dir();
        let mut t = Theme::default();
        if let Ok(name) = fs::read_to_string(dir.join("theme.name")) {
            t.name = name.trim().to_string();
        }
        let Ok(src) = fs::read_to_string(dir.join("theme/colors.toml")) else {
            return t;
        };
        let Ok(table) = src.parse::<toml::Table>() else {
            return t;
        };
        let get = |k: &str| table.get(k).and_then(|v| v.as_str()).and_then(Rgb::parse);
        if let Some(c) = get("background") {
            t.background = c;
        }
        if let Some(c) = get("foreground") {
            t.foreground = c;
        }
        if let Some(c) = get("accent").or_else(|| get("blue")) {
            t.accent = c;
        }
        if let Some(c) = get("muted").or_else(|| get("selection")) {
            t.muted = c;
        }
        t
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_hex() {
        assert_eq!(Rgb::parse("#7fd4e4"), Some(Rgb(0x7f, 0xd4, 0xe4)));
        assert_eq!(Rgb::parse("7fd4e4ff"), Some(Rgb(0x7f, 0xd4, 0xe4)));
        assert_eq!(Rgb::parse("#zzz"), None);
        assert_eq!(Rgb(1, 2, 255).hex(), "#0102ff");
    }
}
