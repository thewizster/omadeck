//! Images in and out: textures for previews, importing icons into the config dir,
//! resolving app icons and fetching website favicons.

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

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

/// Copy an image the user picked into the config dir so the setup stays self-contained.
pub fn import_icon(src: &Path) -> Result<String> {
    if src.starts_with(paths::icons_dir()) {
        return Ok(paths::portable(src));
    }
    let bytes = fs::read(src).with_context(|| format!("reading {}", src.display()))?;
    let ext = src.extension().and_then(|e| e.to_str()).unwrap_or("png").to_ascii_lowercase();
    let stem = src.file_stem().and_then(|s| s.to_str()).unwrap_or("icon");
    if omadeck_core::render::load_icon(src, 16).is_none() {
        bail!("{} is not an image omadeck can read", src.display());
    }
    store_icon(stem, &ext, &bytes)
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

/// Download a website's icon (blocking; call from a worker thread).
pub fn fetch_favicon(url: &str) -> Result<String> {
    let host = host_of(url).context("that doesn't look like a website address")?;
    let sources = [
        format!("https://www.google.com/s2/favicons?domain={host}&sz=256"),
        format!("https://icons.duckduckgo.com/ip3/{host}.ico"),
    ];
    let mut last = None;
    for src in sources {
        match download(&src) {
            Ok(bytes) => {
                let img = image_from(&bytes)?;
                // Upscale tiny favicons so they look decent on the key.
                let img = if img.width() < 96 {
                    image::imageops::resize(&img, 128, 128, image::imageops::FilterType::Nearest)
                } else {
                    img
                };
                let mut png = Vec::new();
                img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)?;
                return store_icon(&host, "png", &png);
            }
            Err(e) => last = Some(e),
        }
    }
    Err(last.unwrap_or_else(|| anyhow::anyhow!("no icon found")))
}

fn image_from(bytes: &[u8]) -> Result<image::RgbaImage> {
    Ok(image::load_from_memory(bytes).context("site icon is not a readable image")?.to_rgba8())
}

fn download(url: &str) -> Result<Vec<u8>> {
    let mut resp = ureq::get(url).call()?;
    let mut buf = Vec::new();
    resp.body_mut().as_reader().take(4 << 20).read_to_end(&mut buf)?;
    if buf.len() < 64 {
        bail!("empty response");
    }
    Ok(buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hosts() {
        assert_eq!(host_of("omarchy.org/x").as_deref(), Some("omarchy.org"));
        assert_eq!(host_of("https://u@a.b.com:8080/p?q").as_deref(), Some("a.b.com"));
        assert_eq!(host_of("nonsense"), None);
    }
}
