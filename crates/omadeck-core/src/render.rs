//! Renders a key face (background + icon + label) to an RGBA bitmap.
//!
//! The same renderer drives both the physical deck and the previews in the configurator,
//! so what you see in the app is exactly what lands on the hardware.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use ab_glyph::{Font, FontVec, PxScale, ScaleFont, point};
use image::Rgba;
pub use image::RgbaImage;
use image::imageops::{self, FilterType};

use crate::config::{KeyConfig, Style};
use crate::paths::{self, FileStamp};
use crate::theme::{Rgb, Theme};

pub struct Renderer {
    font: Option<FontVec>,
    font_pattern: Option<String>,
    /// fontconfig's user config as of the last font lookup; `omarchy font set` rewrites it.
    fonts_stamp: Option<FileStamp>,
    /// Fonts that supply symbols the label font lacks (picking a font without Nerd Font
    /// icons shouldn't blank every glyph key).
    fallbacks: HashMap<char, Option<Arc<FontVec>>>,
    pub theme: Theme,
    /// Decoded icons, with the stamp their file had when it was read.
    icons: HashMap<(PathBuf, u32), (Option<FileStamp>, Option<RgbaImage>)>,
}

impl Renderer {
    pub fn new(style: &Style) -> Self {
        let mut r = Self {
            font: None,
            font_pattern: None,
            fonts_stamp: FileStamp::of(&paths::fontconfig_file()),
            fallbacks: HashMap::new(),
            theme: Theme::default(),
            icons: HashMap::new(),
        };
        r.update_style(style);
        r.theme = Theme::load_omarchy();
        r
    }

    /// Re-resolve the font if the style asks for a different one.
    pub fn update_style(&mut self, style: &Style) {
        if self.font.is_none() || self.font_pattern != style.font {
            self.font_pattern = style.font.clone();
            self.font = load_font(style.font.as_deref().unwrap_or("monospace"));
            if self.font.is_none() {
                log::warn!("no usable font found; labels will not be drawn");
            }
        }
    }

    /// Re-resolve the font if the system font changed (e.g. `omarchy font set`).
    /// Returns true when it did, so the caller can repaint.
    pub fn font_changed(&mut self) -> bool {
        let stamp = FileStamp::of(&paths::fontconfig_file());
        if stamp == self.fonts_stamp {
            return false;
        }
        self.fonts_stamp = stamp;
        self.font = load_font(self.font_pattern.as_deref().unwrap_or("monospace"));
        self.fallbacks.clear();
        log::info!("system font changed; reloaded the label font");
        true
    }

    pub fn reload_theme(&mut self) {
        self.theme = Theme::load_omarchy();
    }

    /// Drop cached icons whose files changed or went away (call after the config changes).
    /// Unchanged icons stay, so a save doesn't decode every image again.
    pub fn clear_cache(&mut self) {
        self.icons.retain(|(path, _), (stamp, _)| stamp.is_some() && FileStamp::of(path) == *stamp);
    }

    pub fn background(&self, key: Option<&KeyConfig>, style: &Style) -> Rgb {
        key.and_then(|k| k.background.as_deref())
            .and_then(Rgb::parse)
            .or_else(|| style.background.as_deref().and_then(Rgb::parse))
            .unwrap_or(if style.follow_theme { self.theme.background } else { Rgb(0, 0, 0) })
    }

    pub fn label_color(&self, key: Option<&KeyConfig>, style: &Style, bg: Rgb) -> Rgb {
        if let Some(c) = key.and_then(|k| k.label_color.as_deref()).and_then(Rgb::parse) {
            return c;
        }
        if let Some(c) = style.label_color.as_deref().and_then(Rgb::parse) {
            return c;
        }
        let fg = if style.follow_theme { self.theme.foreground } else { Rgb(255, 255, 255) };
        // Keep the label readable on custom backgrounds.
        if (fg.luminance() - bg.luminance()).abs() < 0.35 {
            if bg.luminance() > 0.5 { Rgb(16, 16, 16) } else { Rgb(245, 245, 245) }
        } else {
            fg
        }
    }

    /// Render one key face at `size`×`size`.
    pub fn render(&mut self, key: Option<&KeyConfig>, style: &Style, size: u32, pressed: bool) -> RgbaImage {
        let key = key.filter(|k| !k.is_blank());
        let bg = self.background(key, style);
        let face = match key {
            None => solid(size, bg.mix(Rgb(0, 0, 0), 0.55)),
            Some(k) => self.render_face(k, style, size, bg),
        };
        if pressed { self.pressed(face, bg, size) } else { face }
    }

    fn render_face(&mut self, key: &KeyConfig, style: &Style, size: u32, bg: Rgb) -> RgbaImage {
        let mut img = solid(size, bg);
        let s = size as f32;
        let label = key.label.trim();
        let want_label = key.show_label && !label.is_empty();
        let fg = self.label_color(Some(key), style, bg);

        let box_px = if want_label { (s * 0.62) as u32 } else { (s * 0.80) as u32 };
        let icon = match (key.icon.as_deref(), key.glyph.as_deref().filter(|g| !g.trim().is_empty())) {
            (Some(p), _) => self.icon(&paths::resolve(p), box_px),
            (None, Some(g)) => {
                let color = key.label_color.as_deref().and_then(Rgb::parse).unwrap_or(if style.follow_theme {
                    self.theme.accent
                } else {
                    fg
                });
                self.glyph(g, (box_px as f32 * 0.86) as u32, color)
            }
            _ => None,
        };

        match (&icon, want_label) {
            (Some(icon), true) => {
                let x = (size - icon.width()) / 2;
                let area = s * 0.70;
                let y = ((area - icon.height() as f32) / 2.0 + s * 0.04).max(0.0) as u32;
                imageops::overlay(&mut img, icon, x as i64, y as i64);
                self.draw_line(&mut img, label, s * 0.17, s * 0.86, fg, true);
            }
            (Some(icon), false) => {
                let x = (size - icon.width()) / 2;
                let y = (size - icon.height()) / 2;
                imageops::overlay(&mut img, icon, x as i64, y as i64);
            }
            (None, true) => self.draw_block(&mut img, label, fg),
            (None, false) => {}
        }
        img
    }

    /// Pressed state: face shrinks inside an accent ring for tactile feedback.
    fn pressed(&self, face: RgbaImage, bg: Rgb, size: u32) -> RgbaImage {
        let mut out = solid(size, bg.mix(Rgb(0, 0, 0), 0.4));
        let inner = (size as f32 * 0.84) as u32;
        let small = imageops::resize(&face, inner, inner, FilterType::Triangle);
        let off = ((size - inner) / 2) as i64;
        imageops::overlay(&mut out, &small, off, off);
        let ring = (size / 24).max(2);
        let a = self.theme.accent;
        let px = Rgba([a.0, a.1, a.2, 255]);
        for y in 0..size {
            for x in 0..size {
                if x < ring || y < ring || x >= size - ring || y >= size - ring {
                    out.put_pixel(x, y, px);
                }
            }
        }
        out
    }

    fn icon(&mut self, path: &Path, box_px: u32) -> Option<RgbaImage> {
        let k = (path.to_path_buf(), box_px);
        if let Some((_, hit)) = self.icons.get(&k) {
            return hit.clone();
        }
        let stamp = FileStamp::of(path);
        let loaded = load_icon(path, box_px);
        if loaded.is_none() {
            log::warn!("could not load icon {}", path.display());
        }
        self.icons.insert(k, (stamp, loaded.clone()));
        loaded
    }

    /// Rasterise a single (Nerd Font) symbol, scaled to fit a `box_px` square.
    fn glyph(&mut self, text: &str, box_px: u32, color: Rgb) -> Option<RgbaImage> {
        let c = glyph_char(text)?;
        let fallback;
        let font = match self.font.as_ref() {
            Some(f) if f.glyph_id(c).0 != 0 => f,
            _ => {
                fallback = self.fallbacks.entry(c).or_insert_with(|| load_font_with(c).map(Arc::new)).clone()?;
                &*fallback
            }
        };
        let id = font.glyph_id(c);
        let probe = font.outline_glyph(id.with_scale(PxScale::from(100.0)))?.px_bounds();
        let k = box_px as f32 / probe.width().max(probe.height()).max(1.0);
        let outline = font.outline_glyph(id.with_scale_and_position(PxScale::from(100.0 * k), point(0.0, 0.0)))?;
        let b = outline.px_bounds();
        let mut img = RgbaImage::new((b.width().ceil() as u32).max(1), (b.height().ceil() as u32).max(1));
        outline.draw(|x, y, cov| {
            if x < img.width() && y < img.height() {
                img.put_pixel(x, y, Rgba([color.0, color.1, color.2, (cov.clamp(0.0, 1.0) * 255.0) as u8]));
            }
        });
        Some(img)
    }

    // --- text -------------------------------------------------------------------------

    fn text_width(font: &FontVec, px: f32, text: &str) -> f32 {
        let sf = font.as_scaled(PxScale::from(px));
        let mut w = 0.0;
        let mut prev = None;
        for c in text.chars() {
            let id = sf.glyph_id(c);
            if let Some(p) = prev {
                w += sf.kern(p, id);
            }
            w += sf.h_advance(id);
            prev = Some(id);
        }
        w
    }

    /// Draw one centred line with its baseline-centre at `cy`, shrinking/ellipsizing to fit.
    fn draw_line(&self, img: &mut RgbaImage, text: &str, px: f32, cy: f32, color: Rgb, shadow: bool) {
        let Some(font) = &self.font else { return };
        let max_w = img.width() as f32 * 0.92;
        let min_px = img.width() as f32 * 0.11;
        let (px, text) = fit_line(text, px, min_px, max_w, |px, t| Self::text_width(font, px, t));
        let w = Self::text_width(font, px, &text);
        let sf = font.as_scaled(PxScale::from(px));
        let baseline = cy + (sf.ascent() + sf.descent()) / 2.0;
        let x = (img.width() as f32 - w) / 2.0;
        if shadow {
            draw_text(img, font, px, x + 1.0, baseline + 1.0, &text, Rgb(0, 0, 0), 0.6);
        }
        draw_text(img, font, px, x, baseline, &text, color, 1.0);
    }

    /// Label without an icon: large, centred, wrapped onto up to three lines.
    fn draw_block(&self, img: &mut RgbaImage, text: &str, color: Rgb) {
        let Some(font) = &self.font else { return };
        let s = img.width() as f32;
        let max_w = s * 0.90;
        let mut px = s * 0.26;
        let lines = loop {
            let lines = wrap(font, px, text, max_w);
            let fits = lines.iter().all(|l| Self::text_width(font, px, l) <= max_w)
                && lines.len() as f32 * px * 1.1 <= s * 0.9;
            if fits || px <= s * 0.12 {
                break lines;
            }
            px -= 0.5;
        };
        let lh = px * 1.1;
        let top = (s - lh * lines.len() as f32) / 2.0;
        for (i, line) in lines.iter().enumerate() {
            self.draw_line(img, line, px, top + lh * (i as f32 + 0.5), color, false);
        }
    }
}

/// A pasted symbol, or a codepoint written as `f075f`, `U+F075F`, `0xf075f` or `\\u{f075f}`.
pub fn glyph_char(text: &str) -> Option<char> {
    let t = text.trim();
    let hex = t
        .trim_start_matches("\\u{")
        .trim_end_matches('}')
        .trim_start_matches("U+")
        .trim_start_matches("u+")
        .trim_start_matches("0x");
    if hex.len() >= 4 && hex.len() <= 6 && hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return u32::from_str_radix(hex, 16).ok().and_then(char::from_u32);
    }
    t.chars().next()
}

fn wrap(font: &FontVec, px: f32, text: &str, max_w: f32) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    for word in text.split_whitespace() {
        match lines.last_mut() {
            Some(l) if Renderer::text_width(font, px, &format!("{l} {word}")) <= max_w => {
                l.push(' ');
                l.push_str(word);
            }
            _ => lines.push(word.to_string()),
        }
    }
    if lines.is_empty() {
        lines.push(String::new());
    }
    lines
}

#[allow(clippy::too_many_arguments)]
fn draw_text(
    img: &mut RgbaImage,
    font: &FontVec,
    px: f32,
    x: f32,
    baseline: f32,
    text: &str,
    color: Rgb,
    opacity: f32,
) {
    let scale = PxScale::from(px);
    let sf = font.as_scaled(scale);
    let mut caret = x;
    let mut prev = None;
    let (w, h) = img.dimensions();
    for c in text.chars() {
        let id = sf.glyph_id(c);
        if let Some(p) = prev {
            caret += sf.kern(p, id);
        }
        let glyph = id.with_scale_and_position(scale, point(caret, baseline));
        caret += sf.h_advance(id);
        prev = Some(id);
        let Some(outline) = font.outline_glyph(glyph) else { continue };
        let b = outline.px_bounds();
        outline.draw(|gx, gy, cov| {
            let x = b.min.x as i32 + gx as i32;
            let y = b.min.y as i32 + gy as i32;
            if x < 0 || y < 0 || x as u32 >= w || y as u32 >= h {
                return;
            }
            let a = (cov * opacity).clamp(0.0, 1.0);
            let p = img.get_pixel_mut(x as u32, y as u32);
            let blend = |d: u8, s: u8| (d as f32 * (1.0 - a) + s as f32 * a).round() as u8;
            p.0 = [blend(p.0[0], color.0), blend(p.0[1], color.1), blend(p.0[2], color.2), 255];
        });
    }
}

fn solid(size: u32, c: Rgb) -> RgbaImage {
    RgbaImage::from_pixel(size, size, Rgba([c.0, c.1, c.2, 255]))
}

/// Load a raster or SVG icon and fit it inside a `box_px` square, preserving aspect ratio.
/// Labels are cut to this many characters before fitting; a key shows far fewer.
const MAX_LABEL_CHARS: usize = 64;

/// The size and text to draw one label line with: shrink from `px` in half-pixel steps until it
/// fits `max_w` (stopping at `min_px`), then, if it still doesn't fit, cut it short with "…".
/// `width` measures a text at a size.
///
/// Both searches are binary, since a text only gets wider as it grows or its size does, and the
/// text is cut to `MAX_LABEL_CHARS` first: a key shows far fewer, and measuring a pasted page of
/// text at every step used to take seconds per key.
fn fit_line(text: &str, px: f32, min_px: f32, max_w: f32, width: impl Fn(f32, &str) -> f32) -> (f32, String) {
    let text: String = text.chars().take(MAX_LABEL_CHARS).collect();
    // Half-pixel steps down from `px`; stop at the first that fits, or once at or below `min_px`.
    let size = |step: usize| px - 0.5 * step as f32;
    let last = if px > min_px { ((px - min_px) / 0.5).ceil() as usize } else { 0 };
    let step = first_true(0, last, |k| size(k) <= min_px || width(size(k), &text) <= max_w);
    let px = size(step);
    if width(px, &text) <= max_w {
        return (px, text);
    }
    let chars: Vec<char> = text.chars().collect();
    if chars.len() < 2 {
        return (px, text);
    }
    let cut = |n: usize| format!("{}…", chars[..n].iter().collect::<String>().trim_end());
    // The longest cut that fits, or one character if none does: one past it is the first that
    // doesn't (or the whole text, which is known not to fit).
    let longest = chars.len() - 1;
    let first_too_wide = first_true(1, longest + 1, |n| n > longest || width(px, &cut(n)) > max_w);
    let n = first_too_wide.saturating_sub(1).max(1);
    (px, cut(n))
}

/// The smallest `k` in `lo..=hi` for which `pred` holds, `pred` being false and then true;
/// `hi` if it never holds before.
fn first_true(mut lo: usize, mut hi: usize, pred: impl Fn(usize) -> bool) -> usize {
    while lo < hi {
        let mid = lo + (hi - lo) / 2;
        if pred(mid) { hi = mid } else { lo = mid + 1 }
    }
    lo
}

pub fn load_icon(path: &Path, box_px: u32) -> Option<RgbaImage> {
    let is_svg = path
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("svg") || e.eq_ignore_ascii_case("svgz"));
    if is_svg {
        return load_svg(path, box_px);
    }
    let img = image::open(path).ok()?.to_rgba8();
    let (w, h) = img.dimensions();
    let scale = box_px as f32 / w.max(h) as f32;
    let (nw, nh) = (((w as f32 * scale).round() as u32).max(1), ((h as f32 * scale).round() as u32).max(1));
    Some(imageops::resize(&img, nw, nh, FilterType::Lanczos3))
}

fn load_svg(path: &Path, box_px: u32) -> Option<RgbaImage> {
    use resvg::{tiny_skia, usvg};
    let data = fs::read(path).ok()?;
    let tree = usvg::Tree::from_data(&data, &usvg::Options::default()).ok()?;
    let size = tree.size();
    let scale = box_px as f32 / size.width().max(size.height());
    let (w, h) = (((size.width() * scale).ceil() as u32).max(1), ((size.height() * scale).ceil() as u32).max(1));
    let mut pixmap = tiny_skia::Pixmap::new(w, h)?;
    resvg::render(&tree, tiny_skia::Transform::from_scale(scale, scale), &mut pixmap.as_mut());
    let mut out = RgbaImage::new(w, h);
    for (dst, src) in out.pixels_mut().zip(pixmap.pixels()) {
        let c = src.demultiply();
        *dst = Rgba([c.red(), c.green(), c.blue(), c.alpha()]);
    }
    Some(out)
}

/// Find a bold face for a fontconfig pattern (defaults to Omarchy's monospace font).
fn load_font(pattern: &str) -> Option<FontVec> {
    let candidates = [format!("{pattern}:weight=bold"), pattern.to_string(), "sans:bold".into()];
    for pat in &candidates {
        let out = Command::new("fc-match").args(["-f", "%{file}", pat]).output().ok()?;
        let file = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if file.is_empty() {
            continue;
        }
        if let Ok(data) = fs::read(&file)
            && let Ok(font) = FontVec::try_from_vec(data)
        {
            log::debug!("label font: {file}");
            return Some(font);
        }
    }
    None
}

/// Any installed font that has `c`, preferring Nerd Font symbols.
fn load_font_with(c: char) -> Option<FontVec> {
    let charset = format!(":charset={:x}", c as u32);
    for pat in [format!("Symbols Nerd Font{charset}"), format!("monospace{charset}"), charset.clone()] {
        let out = Command::new("fc-match").args(["-s", "-f", "%{file}\\n", &pat]).output().ok()?;
        // `-s` lists matches best first; take the first that really has the glyph.
        for file in String::from_utf8_lossy(&out.stdout).lines().take(8) {
            if let Ok(data) = fs::read(file.trim())
                && let Ok(font) = FontVec::try_from_vec(data)
                && font.glyph_id(c).0 != 0
            {
                log::debug!("glyph U+{:X} from {file}", c as u32);
                return Some(font);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A renderer that doesn't look at the machine's fonts or Omarchy theme.
    fn renderer() -> Renderer {
        Renderer {
            font: None,
            font_pattern: None,
            fonts_stamp: None,
            fallbacks: HashMap::new(),
            theme: Theme::default(),
            icons: HashMap::new(),
        }
    }

    fn png(path: &Path, w: u32, h: u32, rgb: [u8; 3]) {
        RgbaImage::from_pixel(w, h, image::Rgba([rgb[0], rgb[1], rgb[2], 255])).save(path).unwrap();
    }

    fn temp_png(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("omadeck-test-{}-{name}.png", std::process::id()))
    }

    /// A stand-in for measuring text with a font: each character is 0.6 em wide, a space 0.3,
    /// and the cost grows with the text's length, as the real measurement's does.
    fn width(px: f32, text: &str) -> f32 {
        text.chars().map(|c| if c == ' ' { 0.3 } else { 0.6 }).sum::<f32>() * px
    }

    /// `fit_line` as it was in 0.2.0, to compare against.
    fn fit_line_0_2(text: &str, px: f32, min_px: f32, max_w: f32) -> (f32, String) {
        let mut px = px;
        while px > min_px && width(px, text) > max_w {
            px -= 0.5;
        }
        let mut text = text.to_string();
        if width(px, &text) > max_w {
            let chars: Vec<char> = text.chars().collect();
            for n in (1..chars.len()).rev() {
                let cand = format!("{}…", chars[..n].iter().collect::<String>().trim_end());
                if width(px, &cand) <= max_w || n == 1 {
                    text = cand;
                    break;
                }
            }
        }
        (px, text)
    }

    #[test]
    fn a_20000_character_label_fits_quickly() {
        // B9: fitting was quadratic, so pasting a long text froze the app and stalled the deck.
        let label = "word ".repeat(4000);
        let start = std::time::Instant::now();
        let (_, text) = fit_line(&label, 13.0, 7.9, 66.0, width);
        let took = start.elapsed();
        assert!(text.ends_with('…'));
        assert!(took < std::time::Duration::from_millis(50), "took {took:?}");
    }

    #[test]
    fn short_labels_fit_exactly_as_before() {
        let mut labels: Vec<String> = [
            "",
            "A",
            "Vol +",
            "Terminal",
            "Workspace 1",
            "Next wallpaper",
            "Screenshot region",
            "Night light toggle",
            "a b c d e f g h i j",
            "Supercalifragilistic",
            "Ünïcödé ✓ ok",
            "trailing spaces    end",
        ]
        .map(String::from)
        .to_vec();
        labels.push("x".repeat(70));
        labels.push("word ".repeat(30));
        // Key sizes from the Mini (80) to the XL (96), with each deck's label size and limits.
        for key in [72.0_f32, 80.0, 96.0] {
            for px in [key * 0.18, key * 0.2, key * 0.25, 13.0, 14.4] {
                for label in &labels {
                    let (min_px, max_w) = (key * 0.11, key * 0.92);
                    assert_eq!(
                        fit_line(label, px, min_px, max_w, width),
                        fit_line_0_2(label, px, min_px, max_w),
                        "{label:?} at {px} px on a {key} px key"
                    );
                }
            }
        }
    }

    #[test]
    fn reload_keeps_unchanged_icons_cached() {
        // B10: every save in the app cleared the cache, so a huge icon was decoded again each time.
        let path = temp_png("kept");
        png(&path, 8, 8, [200, 0, 0]);
        let mut r = renderer();
        assert!(r.icon(&path, 32).is_some());
        r.clear_cache();
        let kept = r.icons.contains_key(&(path.clone(), 32));
        let _ = std::fs::remove_file(&path);
        assert!(kept, "an icon whose file didn't change stays cached");
    }

    #[test]
    fn a_changed_icon_file_is_reloaded() {
        let path = temp_png("changed");
        png(&path, 8, 8, [200, 0, 0]);
        let mut r = renderer();
        assert_eq!(r.icon(&path, 32).unwrap().get_pixel(16, 16)[0], 200);
        png(&path, 16, 16, [0, 0, 200]);
        r.clear_cache();
        let after = r.icon(&path, 32).unwrap();
        std::fs::remove_file(&path).unwrap();
        r.clear_cache();
        let gone = !r.icons.contains_key(&(path.clone(), 32));
        assert_eq!(after.get_pixel(16, 16)[2], 200, "the new picture is drawn");
        assert!(gone, "a deleted icon is dropped from the cache");
    }

    #[test]
    fn glyph_forms() {
        assert_eq!(glyph_char("\u{f075f}"), Some('\u{f075f}'));
        assert_eq!(glyph_char("f075f"), Some('\u{f075f}'));
        assert_eq!(glyph_char("U+F075F"), Some('\u{f075f}'));
        assert_eq!(glyph_char("\\u{f075f}"), Some('\u{f075f}'));
        assert_eq!(glyph_char("A"), Some('A'));
        assert_eq!(glyph_char(""), None);
    }
}
