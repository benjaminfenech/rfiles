//! File-type icons the freedesktop way: the type comes from shared-mime-info (by extension),
//! the icon from the user's icon theme per the Icon Theme Specification (inherited themes,
//! then hicolor, then /usr/share/pixmaps). SVGs are drawn with resvg at the size asked for.
//!
//! Everything is read once and cached; lookups after the first are hash-map hits.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use crate::thumbs::Bgra;

/// `$XDG_DATA_HOME` then `$XDG_DATA_DIRS` (defaults per the Base Directory spec).
pub(crate) fn data_dirs() -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Some(d) = dirs::data_dir() {
        out.push(d);
    }
    let sys = std::env::var("XDG_DATA_DIRS").ok().filter(|s| !s.is_empty()).unwrap_or_else(|| "/usr/local/share:/usr/share".into());
    out.extend(std::env::split_paths(&sys));
    out
}

// ---------------------------------------------------------------------------------------------
// MIME types
// ---------------------------------------------------------------------------------------------

struct MimeDb {
    /// lower-case extension → (weight, type)
    by_ext: HashMap<String, (u32, String)>,
    /// type → generic icon name
    generic: HashMap<String, String>,
    /// type → icon name override
    icons: HashMap<String, String>,
}

fn mime_db() -> &'static MimeDb {
    static DB: OnceLock<MimeDb> = OnceLock::new();
    DB.get_or_init(|| {
        let mut db = MimeDb { by_ext: HashMap::new(), generic: HashMap::new(), icons: HashMap::new() };
        // lower-priority directories first, so the user's own files win
        for dir in data_dirs().iter().rev() {
            let mime = dir.join("mime");
            if let Ok(text) = std::fs::read_to_string(mime.join("globs2")) {
                for line in text.lines().filter(|l| !l.starts_with('#')) {
                    let mut f = line.split(':');
                    let (Some(w), Some(ty), Some(glob)) = (f.next(), f.next(), f.next()) else { continue };
                    // only plain "*.ext" globs: the app's icon cache is per extension
                    let Some(ext) = glob.strip_prefix("*.") else { continue };
                    if ext.contains(['*', '?', '[']) {
                        continue;
                    }
                    let w: u32 = w.parse().unwrap_or(50);
                    let e = db.by_ext.entry(ext.to_lowercase()).or_insert((0, String::new()));
                    if w > e.0 || e.1.is_empty() {
                        *e = (w, ty.to_string());
                    }
                }
            }
            for (file, map) in [("generic-icons", &mut db.generic), ("icons", &mut db.icons)] {
                if let Ok(text) = std::fs::read_to_string(mime.join(file)) {
                    for line in text.lines() {
                        if let Some((ty, icon)) = line.split_once(':') {
                            map.insert(ty.to_string(), icon.to_string());
                        }
                    }
                }
            }
        }
        db
    })
}

/// The MIME type for a file extension (`"png"` → `"image/png"`), if shared-mime-info knows it.
pub fn mime_for_ext(ext: &str) -> Option<&'static str> {
    mime_db().by_ext.get(&ext.to_lowercase()).map(|(_, t)| t.as_str())
}

/// Icon names to try for a MIME type, most specific first.
pub fn icon_names_for_mime(mime: &str) -> Vec<String> {
    let db = mime_db();
    let mut out = Vec::new();
    if let Some(i) = db.icons.get(mime) {
        out.push(i.clone());
    }
    out.push(mime.replace('/', "-"));
    if let Some(g) = db.generic.get(mime) {
        out.push(g.clone());
    }
    let media = mime.split('/').next().unwrap_or("");
    out.push(format!("{media}-x-generic"));
    out
}

// ---------------------------------------------------------------------------------------------
// Icon themes
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq)]
enum DirKind {
    Fixed,
    Scalable,
    Threshold,
}

#[derive(Debug, Clone)]
struct SubDir {
    rel: String,
    size: u32,
    scale: u32,
    kind: DirKind,
    min: u32,
    max: u32,
    threshold: u32,
}

impl SubDir {
    /// Icon Theme Spec, DirectoryMatchesSize.
    fn matches(&self, size: u32) -> bool {
        match self.kind {
            DirKind::Fixed => self.size == size,
            DirKind::Scalable => self.min <= size && size <= self.max,
            DirKind::Threshold => self.size.abs_diff(size) <= self.threshold,
        }
    }

    /// Icon Theme Spec, DirectorySizeDistance.
    fn distance(&self, size: u32) -> u32 {
        match self.kind {
            DirKind::Fixed => self.size.abs_diff(size),
            DirKind::Scalable if size < self.min => self.min - size,
            DirKind::Scalable if size > self.max => size - self.max,
            DirKind::Scalable => 0,
            DirKind::Threshold if size + self.threshold < self.size => self.size - self.threshold - size,
            DirKind::Threshold if size > self.size + self.threshold => size - self.size - self.threshold,
            DirKind::Threshold => 0,
        }
    }
}

#[derive(Debug)]
struct Theme {
    /// Every `icons/<name>` folder that exists, in data-dir order.
    bases: Vec<PathBuf>,
    subdirs: Vec<SubDir>,
    inherits: Vec<String>,
}

fn icon_bases() -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Some(h) = dirs::home_dir() {
        out.push(h.join(".icons"));
    }
    out.extend(data_dirs().into_iter().map(|d| d.join("icons")));
    out
}

fn load_theme(name: &str) -> Option<Theme> {
    let bases: Vec<PathBuf> = icon_bases().into_iter().map(|b| b.join(name)).filter(|p| p.is_dir()).collect();
    let index = bases.iter().map(|b| b.join("index.theme")).find(|p| p.is_file())?;
    let text = std::fs::read_to_string(index).ok()?;
    // a tiny INI reader: sections of key=value
    let mut sections: HashMap<String, HashMap<String, String>> = HashMap::new();
    let mut cur = String::new();
    for line in text.lines().map(str::trim) {
        if let Some(s) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            cur = s.to_string();
        } else if let Some((k, v)) = line.split_once('=') {
            sections.entry(cur.clone()).or_default().insert(k.trim().to_string(), v.trim().to_string());
        }
    }
    let head = sections.get("Icon Theme")?;
    let list = |k: &str| head.get(k).map(|v| v.split(',').map(str::trim).filter(|s| !s.is_empty()).map(String::from).collect::<Vec<_>>()).unwrap_or_default();
    let mut subdirs = Vec::new();
    for rel in list("Directories").into_iter().chain(list("ScaledDirectories")) {
        let Some(s) = sections.get(&rel) else { continue };
        let num = |k: &str, d: u32| s.get(k).and_then(|v| v.parse().ok()).unwrap_or(d);
        let size = num("Size", 0);
        if size == 0 {
            continue;
        }
        let kind = match s.get("Type").map(String::as_str) {
            Some("Fixed") => DirKind::Fixed,
            Some("Scalable") => DirKind::Scalable,
            _ => DirKind::Threshold,
        };
        subdirs.push(SubDir { rel, size, scale: num("Scale", 1), kind, min: num("MinSize", size), max: num("MaxSize", size), threshold: num("Threshold", 2) });
    }
    Some(Theme { bases, subdirs, inherits: list("Inherits") })
}

/// The icon theme the desktop uses: `RUSTYFILES_ICON_THEME`, KDE's `kdeglobals` on KDE, the
/// GTK settings files, then GNOME's setting, else hicolor.
pub fn current_theme_name() -> String {
    if let Ok(t) = std::env::var("RUSTYFILES_ICON_THEME") {
        if !t.is_empty() {
            return t;
        }
    }
    let config = dirs::config_dir().unwrap_or_default();
    let ini_value = |file: &Path, section: &str, key: &str| -> Option<String> {
        let text = std::fs::read_to_string(file).ok()?;
        let mut in_section = false;
        for line in text.lines().map(str::trim) {
            if line.starts_with('[') {
                in_section = line == format!("[{section}]");
            } else if in_section {
                if let Some((k, v)) = line.split_once('=') {
                    if k.trim() == key {
                        let v = v.trim().trim_matches('"');
                        return (!v.is_empty()).then(|| v.to_string());
                    }
                }
            }
        }
        None
    };
    let kde = || ini_value(&config.join("kdeglobals"), "Icons", "Theme");
    let gtk = || {
        ["gtk-4.0/settings.ini", "gtk-3.0/settings.ini"]
            .iter()
            .find_map(|f| ini_value(&config.join(f), "Settings", "gtk-icon-theme-name"))
    };
    let gnome = || {
        let out = crate::sandbox::tool("gsettings", &[]).args(["get", "org.gnome.desktop.interface", "icon-theme"]).output().ok()?;
        let v = String::from_utf8(out.stdout).ok()?.trim().trim_matches('\'').to_string();
        (out.status.success() && !v.is_empty()).then_some(v)
    };
    let on_kde = std::env::var("XDG_CURRENT_DESKTOP").is_ok_and(|d| d.to_uppercase().contains("KDE"));
    let found = if on_kde { kde().or_else(gtk) } else { gtk().or_else(kde) };
    found.or_else(gnome).unwrap_or_else(|| "hicolor".into())
}

/// The current theme, the themes it inherits (depth first, each once), then hicolor.
fn theme_chain() -> &'static Vec<Theme> {
    static CHAIN: OnceLock<Vec<Theme>> = OnceLock::new();
    CHAIN.get_or_init(|| {
        let mut out = Vec::new();
        let mut seen = std::collections::HashSet::new();
        let mut stack = vec![current_theme_name()];
        // common fallbacks before hicolor: most themes leave mime types to these
        let tail = ["Adwaita", "breeze", "hicolor"];
        while let Some(name) = stack.pop() {
            if !seen.insert(name.clone()) {
                continue;
            }
            if let Some(t) = load_theme(&name) {
                for i in t.inherits.iter().rev() {
                    stack.push(i.clone());
                }
                out.push(t);
            }
            if stack.is_empty() {
                if let Some(next) = tail.iter().find(|t| !seen.contains(**t)) {
                    stack.push(next.to_string());
                }
            }
        }
        out
    })
}

const EXTS: [&str; 2] = ["svg", "png"];

fn find_in_theme(t: &Theme, name: &str, size: u32) -> Option<PathBuf> {
    // an exact size match first (scale 1, then any scale)
    for scale_one in [true, false] {
        for d in t.subdirs.iter().filter(|d| d.matches(size) && (d.scale == 1) == scale_one) {
            for base in &t.bases {
                for ext in EXTS {
                    let p = base.join(&d.rel).join(format!("{name}.{ext}"));
                    if p.is_file() {
                        return Some(p);
                    }
                }
            }
        }
    }
    // else the closest size; scalable wins ties (it draws sharp at any size)
    let mut best: Option<(u32, bool, PathBuf)> = None;
    for d in &t.subdirs {
        for base in &t.bases {
            for ext in EXTS {
                let p = base.join(&d.rel).join(format!("{name}.{ext}"));
                if !p.is_file() {
                    continue;
                }
                let dist = d.distance(size * d.scale.max(1)) / d.scale.max(1);
                let svg = ext == "svg";
                if best.as_ref().is_none_or(|(bd, bsvg, _)| dist < *bd || (dist == *bd && svg && !bsvg)) {
                    best = Some((dist, svg, p));
                }
            }
        }
    }
    best.map(|b| b.2)
}

/// The file for icon `name` at `size` pixels, from the current theme chain or pixmaps.
pub fn find_icon(name: &str, size: u32) -> Option<PathBuf> {
    static CACHE: OnceLock<Mutex<HashMap<(String, u32), Option<PathBuf>>>> = OnceLock::new();
    let cache = CACHE.get_or_init(Default::default);
    if let Some(hit) = cache.lock().unwrap().get(&(name.to_string(), size)) {
        return hit.clone();
    }
    let found = theme_chain().iter().find_map(|t| find_in_theme(t, name, size)).or_else(|| {
        data_dirs().iter().flat_map(|d| EXTS.map(|e| d.join("pixmaps").join(format!("{name}.{e}")))).find(|p| p.is_file())
    });
    cache.lock().unwrap().insert((name.to_string(), size), found.clone());
    found
}

/// The first of `names` the theme has, drawn at `size`.
pub fn render_first(names: &[String], size: u32) -> Option<Bgra> {
    names.iter().find_map(|n| find_icon(n, size)).and_then(|p| render_file(&p, size))
}

/// Draws an icon file (SVG or bitmap) to fit `size` × `size`, keeping its aspect ratio.
pub fn render_file(path: &Path, size: u32) -> Option<Bgra> {
    let is_svg = path.extension().is_some_and(|e| e.eq_ignore_ascii_case("svg") || e.eq_ignore_ascii_case("svgz"));
    if is_svg {
        return render_svg(&std::fs::read(path).ok()?, size);
    }
    let img = image::open(path).ok()?;
    let img = if img.width() == size || img.height() == size {
        img
    } else {
        img.resize(size, size, image::imageops::FilterType::Lanczos3)
    };
    Some(crate::thumbs::rgba_to_bgra(img.into_rgba8()))
}

fn render_svg(data: &[u8], size: u32) -> Option<Bgra> {
    let opt = resvg::usvg::Options::default();
    let tree = resvg::usvg::Tree::from_data(data, &opt).ok()?;
    let s = tree.size();
    let scale = size as f32 / s.width().max(s.height());
    let (w, h) = (((s.width() * scale).round() as u32).max(1), ((s.height() * scale).round() as u32).max(1));
    let mut pixmap = resvg::tiny_skia::Pixmap::new(w, h)?;
    resvg::render(&tree, resvg::tiny_skia::Transform::from_scale(scale, scale), &mut pixmap.as_mut());
    // tiny-skia is premultiplied RGBA; Bgra is straight alpha
    let mut data = pixmap.take();
    for px in data.chunks_exact_mut(4) {
        let a = px[3] as u32;
        if a != 0 && a != 255 {
            for c in &mut px[..3] {
                *c = ((*c as u32 * 255 + a / 2) / a).min(255) as u8;
            }
        }
        px.swap(0, 2);
    }
    Some(Bgra { width: w, height: h, data })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mime_types_from_globs() {
        // shared-mime-info is on every desktop Linux system
        if !Path::new("/usr/share/mime/globs2").exists() {
            return;
        }
        assert_eq!(mime_for_ext("png"), Some("image/png"));
        assert_eq!(mime_for_ext("PDF"), Some("application/pdf"));
        assert_eq!(mime_for_ext("definitely-not-a-type"), None);
        let names = icon_names_for_mime("application/pdf");
        assert_eq!(names.first().map(String::as_str), Some("application-pdf"));
        assert_eq!(names.last().map(String::as_str), Some("application-x-generic"));
    }

    #[test]
    fn size_matching_follows_the_spec() {
        let fixed = SubDir { rel: "16".into(), size: 16, scale: 1, kind: DirKind::Fixed, min: 16, max: 16, threshold: 2 };
        let scal = SubDir { rel: "s".into(), size: 16, scale: 1, kind: DirKind::Scalable, min: 8, max: 512, threshold: 2 };
        let thr = SubDir { rel: "t".into(), size: 32, scale: 1, kind: DirKind::Threshold, min: 32, max: 32, threshold: 2 };
        assert!(fixed.matches(16) && !fixed.matches(32));
        assert!(scal.matches(32) && !scal.matches(600));
        assert!(thr.matches(34) && !thr.matches(36));
        assert_eq!(fixed.distance(32), 16);
        assert_eq!(scal.distance(600), 88);
        assert_eq!(thr.distance(40), 6);
    }

    #[test]
    fn finds_and_draws_a_folder_icon() {
        // every theme chain ends in hicolor; "folder" lives in the desktop's theme or Adwaita
        let Some(p) = find_icon("folder", 32) else { return };
        let b = render_file(&p, 32).expect("drawable");
        assert!(b.width <= 32 && b.height <= 32 && (b.width == 32 || b.height == 32), "{}x{}", b.width, b.height);
        assert_eq!(b.data.len(), (b.width * b.height * 4) as usize);
        assert!(b.data.chunks_exact(4).any(|px| px[3] > 0), "not empty");
    }
}
