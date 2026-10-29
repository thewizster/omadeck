//! Images in and out: textures for previews, importing icons into the config dir,
//! resolving app icons and fetching website favicons.

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use gtk::{gdk, gio, glib, prelude::*};
use omadeck_core::paths;

pub fn texture(img: omadeck_core::render::RgbaImage) -> gdk::Texture {
    let (w, h) = img.dimensions();
    let bytes = glib::Bytes::from_owned(img.into_raw());
    gdk::MemoryTexture::new(w as i32, h as i32, gdk::MemoryFormat::R8g8b8a8, &bytes, w as usize * 4).upcast()
}

fn slug(s: &str) -> String {
    let s: String = s
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '.' { c.to_ascii_lowercase() } else { '-' })
        .collect();
    let s = s.trim_matches(|c| c == '-' || c == '.').to_string();
    if s.is_empty() { "icon".into() } else { s }
}

/// A free path `icons/<stem>.<ext>`, or the existing one if it already holds identical bytes.
fn destination(stem: &str, ext: &str, bytes: &[u8]) -> PathBuf {
    let dir = paths::icons_dir();
    let stem = slug(stem);
    for n in 0.. {
        let name = if n == 0 { format!("{stem}.{ext}") } else { format!("{stem}-{n}.{ext}") };
        let p = dir.join(name);
        match fs::read(&p) {
            Ok(existing) if existing == bytes => return p,
            Ok(_) => continue,
            Err(_) => return p,
        }
    }
    unreachable!()
}

/// Store bytes under the icons dir; returns the config-relative path to save in the config.
pub fn store_icon(stem: &str, ext: &str, bytes: &[u8]) -> Result<String> {
    fs::create_dir_all(paths::icons_dir())?;
    let dest = destination(stem, ext, bytes);
    if !dest.exists() {
        fs::write(&dest, bytes)?;
    }
    Ok(paths::portable(&dest))
}

/// Imported pictures larger than this are shrunk to it. Keys are at most 96 px, so this keeps
/// plenty of detail while a phone photo goes from megabytes and a second of decoding to a small
/// PNG.
const MAX_ICON_PX: u32 = 512;

/// An imported picture as it should be stored: a raster image bigger than `MAX_ICON_PX` comes
/// back as a smaller PNG; anything else (SVGs included) comes back unchanged.
fn shrink_if_huge(bytes: &[u8], ext: &str) -> Result<(Vec<u8>, String)> {
    let unchanged = || Ok((bytes.to_vec(), ext.to_string()));
    if matches!(ext, "svg" | "svgz") {
        return unchanged();
    }
    // Only the header is read here; something that isn't an image is left for the caller to
    // reject with a clearer message.
    let reader = || image::ImageReader::new(std::io::Cursor::new(bytes)).with_guessed_format();
    let Ok((w, h)) = reader().map_err(anyhow::Error::from).and_then(|r| Ok(r.into_dimensions()?)) else {
        return unchanged();
    };
    if w.max(h) <= MAX_ICON_PX {
        return unchanged();
    }
    let img = reader()?.decode().map_err(|e| match e {
        image::ImageError::Limits(_) => anyhow::anyhow!("it's too large to import ({w}×{h})"),
        e => e.into(),
    })?;
    let small = img.resize(MAX_ICON_PX, MAX_ICON_PX, image::imageops::FilterType::Lanczos3);
    let mut out = std::io::Cursor::new(Vec::new());
    small.write_to(&mut out, image::ImageFormat::Png)?;
    Ok((out.into_inner(), "png".to_string()))
}

/// Copy an image the user picked into the config dir so the setup stays self-contained.
pub fn import_icon(src: &Path) -> Result<String> {
    if src.starts_with(paths::icons_dir()) {
        return Ok(paths::portable(src));
    }
    let bytes = fs::read(src).with_context(|| format!("reading {}", src.display()))?;
    let ext = src.extension().and_then(|e| e.to_str()).unwrap_or("png").to_ascii_lowercase();
    let stem = src.file_stem().and_then(|s| s.to_str()).unwrap_or("icon");
    let (stored, stored_ext) =
        shrink_if_huge(&bytes, &ext).with_context(|| format!("couldn't import {}", src.display()))?;
    // A shrunk picture was just decoded; anything else is checked here.
    if stored_ext == ext && stored.len() == bytes.len() && omadeck_core::render::load_icon(src, 16).is_none() {
        bail!("{} is not an image omadeck can read", src.display());
    }
    store_icon(stem, &stored_ext, &stored)
}

/// Resolve an application's icon to a file (via the icon theme) and import it.
pub fn import_app_icon(app: &gio::AppInfo) -> Option<String> {
    let icon = app.icon()?;
    let file = if let Some(fi) = icon.downcast_ref::<gio::FileIcon>() {
        fi.file().path()
    } else {
        let display = gdk::Display::default()?;
        let theme = gtk::IconTheme::for_display(&display);
        let paintable = theme.lookup_by_gicon(&icon, 256, 1, gtk::TextDirection::None, gtk::IconLookupFlags::empty());
        paintable.file().and_then(|f| f.path())
    }?;
    let name = file.file_name()?.to_string_lossy().to_string();
    if name.starts_with("image-missing") {
        return None;
    }
    let stem = app.id().map(|s| s.trim_end_matches(".desktop").to_string()).unwrap_or(name);
    let bytes = fs::read(&file).ok()?;
    let ext = file.extension().and_then(|e| e.to_str()).unwrap_or("png");
    store_icon(&stem, ext, &bytes).ok()
}

pub fn host_of(url: &str) -> Option<String> {
    let full = omadeck_core::action::normalize_url(url);
    let rest = full.split_once("://")?.1;
    let host = rest.split(['/', '?', '#']).next()?.rsplit('@').next()?;
    let host = host.split(':').next()?;
    (!host.is_empty() && host.contains('.')).then(|| host.to_string())
}

/// Where a fetched site icon came from; shown to the user.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IconSource {
    /// The website itself (its advertised icons or /favicon.ico).
    Site,
    /// DuckDuckGo's public icon service: used only when the site offers nothing usable.
    DuckDuckGo,
}

/// Download a website's icon (blocking; call from a worker thread).
///
/// Privacy: the site itself is asked first, so normally only that site sees the request.
/// DuckDuckGo's icon service is a fallback, and the caller is told when it was used.
pub fn fetch_favicon(url: &str) -> Result<(String, IconSource)> {
    let host = host_of(url).context("that doesn't look like a website address")?;
    let full = omadeck_core::action::normalize_url(url);
    let scheme = if full.starts_with("http://") { "http" } else { "https" };
    let origin = format!("{scheme}://{host}");
    let agent: ureq::Agent = ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(8))).build().into();

    let mut candidates = Vec::new();
    if let Ok(html) = download(&agent, &format!("{origin}/"), 1 << 20) {
        candidates = icon_links(&String::from_utf8_lossy(&html), &origin);
    }
    for fallback in ["/apple-touch-icon.png", "/favicon.ico"] {
        let u = format!("{origin}{fallback}");
        if !candidates.contains(&u) {
            candidates.push(u);
        }
    }
    for candidate in candidates.iter().take(8) {
        if let Ok(bytes) = download(&agent, candidate, 4 << 20)
            && let Ok(icon) = store_site_icon(&host, &bytes)
        {
            return Ok((icon, IconSource::Site));
        }
    }

    let bytes = download(&agent, &format!("https://icons.duckduckgo.com/ip3/{host}.ico"), 4 << 20)
        .context("the site offers no usable icon, and DuckDuckGo's icon service has none either")?;
    Ok((store_site_icon(&host, &bytes)?, IconSource::DuckDuckGo))
}

/// Validate downloaded icon bytes and store them (SVG as-is, rasters as PNG).
fn store_site_icon(host: &str, bytes: &[u8]) -> Result<String> {
    let head = String::from_utf8_lossy(&bytes[..bytes.len().min(512)]).to_ascii_lowercase();
    if head.contains("<svg") {
        let rel = store_icon(host, "svg", bytes)?;
        if omadeck_core::render::load_icon(&paths::resolve(&rel), 16).is_some() {
            return Ok(rel);
        }
        let _ = fs::remove_file(paths::resolve(&rel));
        bail!("unreadable SVG icon");
    }
    let img = image_from(bytes)?;
    if img.width() < 16 || img.height() < 16 {
        bail!("icon too small");
    }
    // Upscale tiny favicons so they look decent on the key.
    let img = if img.width() < 96 {
        image::imageops::resize(&img, 128, 128, image::imageops::FilterType::Nearest)
    } else {
        img
    };
    let mut png = Vec::new();
    img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)?;
    store_icon(host, "png", &png)
}

/// Icon URLs a page advertises via `<link rel="…icon…">`, best first.
fn icon_links(html: &str, origin: &str) -> Vec<String> {
    let lower = html.to_ascii_lowercase();
    let mut found: Vec<(u32, String)> = Vec::new();
    let mut at = 0;
    while let Some(pos) = lower[at..].find("<link") {
        let start = at + pos;
        let end = lower[start..].find('>').map_or(lower.len(), |e| start + e);
        at = end;
        let (tag, tag_lower) = (&html[start..end], &lower[start..end]);
        let rel = attr(tag, tag_lower, "rel").unwrap_or_default().to_ascii_lowercase();
        // mask-icon is a monochrome Safari silhouette; skip it.
        if !rel.split_whitespace().any(|r| r == "icon" || r == "apple-touch-icon") {
            continue;
        }
        let Some(url) = attr(tag, tag_lower, "href").and_then(|h| resolve_href(origin, &h)) else { continue };
        let is_svg = url.to_ascii_lowercase().ends_with(".svg")
            || attr(tag, tag_lower, "type").is_some_and(|t| t.contains("svg"));
        let size = attr(tag, tag_lower, "sizes")
            .and_then(|s| s.split_whitespace().filter_map(|d| d.split(['x', 'X']).next()?.parse::<u32>().ok()).max());
        let score = if is_svg {
            1000
        } else if rel.contains("apple-touch-icon") {
            size.unwrap_or(180)
        } else {
            size.unwrap_or(32)
        };
        found.push((score, url));
    }
    found.sort_by_key(|f| std::cmp::Reverse(f.0));
    let mut urls: Vec<String> = Vec::new();
    for (_, u) in found {
        if !urls.contains(&u) {
            urls.push(u);
        }
    }
    urls
}

/// Read one attribute from a tag. `lower` is the ASCII-lowercased tag (same byte offsets).
fn attr(tag: &str, lower: &str, name: &str) -> Option<String> {
    let mut from = 0;
    while let Some(p) = lower[from..].find(name) {
        let i = from + p;
        from = i + name.len();
        let before_ok = i == 0 || lower.as_bytes()[i - 1].is_ascii_whitespace();
        let rest = lower[from..].trim_start();
        if !before_ok || !rest.starts_with('=') {
            continue;
        }
        let vstart = from + (lower[from..].len() - rest.len()) + 1;
        let v = &tag[vstart..];
        let v = v.trim_start();
        let value = match v.chars().next()? {
            q @ ('"' | '\'') => v[1..].split(q).next()?,
            _ => v.split(|c: char| c.is_whitespace() || c == '/' || c == '>').next()?,
        };
        return Some(value.replace("&amp;", "&"));
    }
    None
}

/// Resolve an icon href against the site origin. Only http(s) URLs are followed.
fn resolve_href(origin: &str, href: &str) -> Option<String> {
    let href = href.trim();
    if href.is_empty() {
        return None;
    }
    if href.starts_with("https://") || href.starts_with("http://") {
        return Some(href.to_string());
    }
    if let Some(rest) = href.strip_prefix("//") {
        return Some(format!("https://{rest}"));
    }
    if let Some(path) = href.strip_prefix('/') {
        return Some(format!("{origin}/{path}"));
    }
    // Any other scheme (data:, javascript:, file:, …) is ignored.
    if href.split(['/', '?', '#']).next().is_some_and(|first| first.contains(':')) {
        return None;
    }
    Some(format!("{origin}/{href}"))
}

fn image_from(bytes: &[u8]) -> Result<image::RgbaImage> {
    Ok(image::load_from_memory(bytes).context("site icon is not a readable image")?.to_rgba8())
}

fn download(agent: &ureq::Agent, url: &str, limit: u64) -> Result<Vec<u8>> {
    let mut resp = agent.get(url).call()?;
    let mut buf = Vec::new();
    resp.body_mut().as_reader().take(limit).read_to_end(&mut buf)?;
    if buf.len() < 64 {
        bail!("empty response");
    }
    Ok(buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn encoded(w: u32, h: u32) -> Vec<u8> {
        let img = image::RgbaImage::from_pixel(w, h, image::Rgba([10, 120, 200, 255]));
        let mut out = std::io::Cursor::new(Vec::new());
        img.write_to(&mut out, image::ImageFormat::Png).unwrap();
        out.into_inner()
    }

    #[test]
    fn importing_a_huge_image_stores_a_small_one() {
        let (bytes, ext) = shrink_if_huge(&encoded(1600, 1200), "jpg").unwrap();
        let img = image::load_from_memory(&bytes).unwrap();
        assert_eq!(ext, "png");
        assert_eq!((img.width(), img.height()), (512, 384), "the aspect ratio is kept");
    }

    #[test]
    fn small_images_and_svgs_are_stored_as_they_are() {
        let small = encoded(256, 256);
        assert_eq!(shrink_if_huge(&small, "png").unwrap(), (small, "png".to_string()));
        let svg = b"<svg xmlns='http://www.w3.org/2000/svg' width='9000' height='9000'/>".to_vec();
        assert_eq!(shrink_if_huge(&svg, "svg").unwrap(), (svg, "svg".to_string()));
    }

    #[test]
    fn hosts() {
        assert_eq!(host_of("omarchy.org/x").as_deref(), Some("omarchy.org"));
        assert_eq!(host_of("https://u@a.b.com:8080/p?q").as_deref(), Some("a.b.com"));
        assert_eq!(host_of("nonsense"), None);
    }

    #[test]
    fn finds_icon_links() {
        let html = r#"<head>
            <link rel="stylesheet" href="/s.css">
            <LINK REL="icon" sizes="32x32" href="/fav-32.png">
            <link rel='apple-touch-icon' href='touch.png'>
            <link rel="mask-icon" href="/mask.svg">
            <link href="https://cdn.example/icon.svg" rel="icon" type="image/svg+xml"/>
            <link rel="icon" href="data:image/png;base64,AAAA">
            <link rel="shortcut icon" href="//static.example/f.ico?v=1&amp;x=2">
        </head>"#;
        assert_eq!(
            icon_links(html, "https://example.com"),
            vec![
                "https://cdn.example/icon.svg",
                "https://example.com/touch.png",
                "https://example.com/fav-32.png",
                "https://static.example/f.ico?v=1&x=2",
            ]
        );
    }

    #[test]
    fn rejects_other_schemes() {
        assert_eq!(resolve_href("https://a.b", "javascript:alert(1)"), None);
        assert_eq!(resolve_href("https://a.b", "file:///etc/passwd"), None);
        assert_eq!(resolve_href("https://a.b", "img/i.png").as_deref(), Some("https://a.b/img/i.png"));
    }

    /// Needs network: `cargo test -p omadeck-gui -- --ignored`
    #[test]
    #[ignore]
    fn favicon_fetch() {
        let dir = std::env::temp_dir().join(format!("omadeck-test-{}", std::process::id()));
        // SAFETY: single-threaded test setup before any other env access.
        unsafe { std::env::set_var("XDG_CONFIG_HOME", &dir) };
        let (rel, source) = fetch_favicon("github.com").expect("fetch");
        assert_eq!(source, IconSource::Site, "github.com serves its own icons");
        assert!(omadeck_core::render::load_icon(&paths::resolve(&rel), 64).is_some());
        let _ = fs::remove_dir_all(dir);
    }
}
