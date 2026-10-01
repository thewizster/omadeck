//! Renders a key face (background + icon + label) to an RGBA bitmap.
//!
//! The same renderer drives both the physical deck and the previews in the configurator,
//! so what you see in the app is exactly what lands on the hardware.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use ab_glyph::{point, Font, FontVec, PxScale, ScaleFont};
use image::imageops::{self, FilterType};
use image::{Rgba, RgbaImage};

use crate::config::{KeyConfig, Style};
use crate::paths;
use crate::theme::{Rgb, Theme};

pub struct Renderer {
    font: Option<FontVec>,
    font_pattern: Option<String>,
    pub theme: Theme,
    icons: HashMap<(PathBuf, u32), Option<RgbaImage>>,
}

impl Renderer {
    pub fn new(style: &Style) -> Self {
        let mut r = Self { font: None, font_pattern: None, theme: Theme::default(), icons: HashMap::new() };
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

    pub fn reload_theme(&mut self) {
        self.theme = Theme::load_omarchy();
    }

    /// Drop cached icons (call after the config changes; files may have been replaced).
    pub fn clear_cache(&mut self) {
        self.icons.clear();
    }

    pub fn background(&self, key: Option<&KeyConfig>, style: &Style) -> Rgb {
        key.and_then(|k| k.background.as_deref())
            .and_then(Rgb::parse)
            .or_else(|| style.background.as_deref().and_then(Rgb::parse))
            .unwrap_or(if style.follow_theme { self.theme.background } else { Rgb(0, 0, 0) })
    }

    fn label_color(&self, key: Option<&KeyConfig>, style: &Style, bg: Rgb) -> Rgb {
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

        let icon = key.icon.as_deref().and_then(|p| {
            let box_px = if want_label { (s * 0.62) as u32 } else { (s * 0.80) as u32 };
            self.icon(&paths::resolve(p), box_px)
        });

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
        if let Some(hit) = self.icons.get(&k) {
            return hit.clone();
        }
        let loaded = load_icon(path, box_px);
        if loaded.is_none() {
            log::warn!("could not load icon {}", path.display());
        }
        self.icons.insert(k, loaded.clone());
        loaded
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
        let mut px = px;
        let min_px = img.width() as f32 * 0.11;
        while px > min_px && Self::text_width(font, px, text) > max_w {
            px -= 0.5;
        }
        let mut text = text.to_string();
        if Self::text_width(font, px, &text) > max_w {
            let chars: Vec<char> = text.chars().collect();
            for n in (1..chars.len()).rev() {
                let cand = format!("{}…", chars[..n].iter().collect::<String>().trim_end());
                if Self::text_width(font, px, &cand) <= max_w || n == 1 {
                    text = cand;
                    break;
                }
            }
        }
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
fn draw_text(img: &mut RgbaImage, font: &FontVec, px: f32, x: f32, baseline: f32, text: &str, color: Rgb, opacity: f32) {
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
