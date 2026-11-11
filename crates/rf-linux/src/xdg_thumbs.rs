//! Thumbnails per the freedesktop.org Thumbnail Managing Standard, shared with GNOME, KDE and
//! other file managers:
//!
//! 1. `~/.cache/thumbnails/{normal,large,x-large,xx-large}/<md5 of URI>.png`, valid while its
//!    `Thumb::MTime` matches the file's modification time.
//! 2. Otherwise made here: images through the `image` crate, SVGs through resvg, anything else
//!    through the thumbnailer installed for its type (`/usr/share/thumbnailers/*.thumbnailer`:
//!    ffmpegthumbnailer, evince, gsf-office…), with a time limit.
//! 3. What we make is written back to the cache. A failure is remembered in
//!    `fail/rfiles/` (until the file changes), so a broken file isn't retried on every visit.

use std::collections::HashMap;
use std::io::BufReader;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use image::RgbaImage;

/// How long an external thumbnailer may run.
const TIMEOUT: Duration = Duration::from_secs(15);

/// The cache folder for a requested size: (name, its pixel size).
fn size_class(size: u32) -> (&'static str, u32) {
    match size {
        0..=128 => ("normal", 128),
        129..=256 => ("large", 256),
        257..=512 => ("x-large", 512),
        _ => ("xx-large", 1024),
    }
}

fn cache_root() -> Option<PathBuf> {
    dirs::cache_dir().map(|c| c.join("thumbnails"))
}

/// `file://` URI the way GLib writes it (so the hash matches other apps' cache entries):
/// unreserved characters and `!$&'()*+,;=:@/` kept, everything else percent-encoded.
pub fn file_uri(path: &Path) -> String {
    use std::os::unix::ffi::OsStrExt;
    let mut out = String::from("file://");
    for &b in path.as_os_str().as_bytes() {
        if b.is_ascii_alphanumeric() || b"-._~!$&'()*+,;=:@/".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

fn md5_hex(s: &str) -> String {
    use md5::{Digest, Md5};
    Md5::digest(s.as_bytes()).iter().map(|b| format!("{b:02x}")).collect()
}

/// A cached thumbnail, if it is there and was made from this version of the file.
fn read_valid(png_path: &Path, mtime: u64) -> Option<RgbaImage> {
    let file = std::fs::File::open(png_path).ok()?;
    let reader = png::Decoder::new(BufReader::new(file)).read_info().ok()?;
    let info = reader.info();
    let stored = info
        .uncompressed_latin1_text
        .iter()
        .find(|t| t.keyword == "Thumb::MTime")
        .map(|t| t.text.clone())
        .or_else(|| info.utf8_text.iter().find(|t| t.keyword == "Thumb::MTime").and_then(|t| t.get_text().ok()))?;
    if stored.trim().parse::<u64>().ok()? != mtime {
        return None;
    }
    drop(reader);
    image::open(png_path).ok().map(|i| i.into_rgba8())
}

/// Writes a thumbnail with its `Thumb::URI` / `Thumb::MTime`, atomically (temp file + rename),
/// readable only by the user.
fn write_png(dest: &Path, img: &RgbaImage, uri: &str, mtime: u64) -> Result<()> {
    let dir = dest.parent().context("no folder")?;
    std::fs::DirBuilder::new().recursive(true).mode(0o700).create(dir)?;
    static N: AtomicU32 = AtomicU32::new(0);
    let tmp = dir.join(format!(".rfiles-{}-{}.png", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
    let r = (|| -> Result<()> {
        let f = std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(&tmp)?;
        let mut enc = png::Encoder::new(std::io::BufWriter::new(f), img.width(), img.height());
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        enc.add_text_chunk("Thumb::URI".into(), uri.into())?;
        enc.add_text_chunk("Thumb::MTime".into(), mtime.to_string())?;
        enc.add_text_chunk("Software".into(), "rfiles".into())?;
        let mut w = enc.write_header()?;
        w.write_image_data(img.as_raw())?;
        w.finish()?;
        std::fs::rename(&tmp, dest)?;
        Ok(())
    })();
    if r.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    r
}

/// Shrinks to fit `size` (never enlarges).
fn fit(img: RgbaImage, size: u32) -> RgbaImage {
    if img.width() <= size && img.height() <= size {
        return img;
    }
    image::DynamicImage::ImageRgba8(img).thumbnail(size, size).into_rgba8()
}

/// Why nothing was made.
enum NoThumb {
    /// No decoder or thumbnailer for this type: not a failure of the file.
    Unsupported,
    Failed(anyhow::Error),
}

/// mime type → thumbnailer `Exec` line, from every `thumbnailers` folder (the user's win).
fn thumbnailers() -> &'static HashMap<String, String> {
    static MAP: OnceLock<HashMap<String, String>> = OnceLock::new();
    MAP.get_or_init(|| {
        let mut map = HashMap::new();
        for dir in crate::xdg_icons::data_dirs().iter().rev() {
            let Ok(rd) = std::fs::read_dir(dir.join("thumbnailers")) else { continue };
            for e in rd.flatten() {
                let Ok(text) = std::fs::read_to_string(e.path()) else { continue };
                let get = |k: &str| text.lines().map(str::trim).find_map(|l| l.strip_prefix(k).and_then(|v| v.strip_prefix('='))).map(str::trim);
                let (Some(exec), Some(types)) = (get("Exec"), get("MimeType")) else { continue };
                if let Some(try_exec) = get("TryExec") {
                    if !in_path(try_exec) {
                        continue;
                    }
                }
                for t in types.split(';').filter(|t| !t.is_empty()) {
                    map.insert(t.to_string(), exec.to_string());
                }
            }
        }
        map
    })
}

fn in_path(prog: &str) -> bool {
    crate::sandbox::in_path(prog)
}

/// Splits an `Exec` line into arguments (double quotes group), with `%i %u %o %s %%` filled in.
fn exec_args(exec: &str, input: &Path, uri: &str, out: &Path, px: u32) -> Vec<String> {
    let mut args = Vec::new();
    let mut cur = String::new();
    let mut quoted = false;
    let mut chars = exec.chars().peekable();
    let mut started = false;
    while let Some(c) = chars.next() {
        match c {
            '"' => {
                quoted = !quoted;
                started = true;
            }
            c if c.is_whitespace() && !quoted => {
                if started {
                    args.push(std::mem::take(&mut cur));
                    started = false;
                }
            }
            '%' => {
                started = true;
                match chars.next() {
                    Some('i') => cur.push_str(&input.to_string_lossy()),
                    Some('u') => cur.push_str(uri),
                    Some('o') => cur.push_str(&out.to_string_lossy()),
                    Some('s') => cur.push_str(&px.to_string()),
                    Some('%') => cur.push('%'),
                    Some(other) => {
                        cur.push('%');
                        cur.push(other);
                    }
                    None => cur.push('%'),
                }
            }
            c => {
                cur.push(c);
                started = true;
            }
        }
    }
    if started {
        args.push(cur);
    }
    args
}

/// Runs the thumbnailer for `mime` into a temporary PNG and reads it.
fn external(path: &Path, uri: &str, mime: &str, px: u32) -> Result<RgbaImage, NoThumb> {
    let Some(exec) = thumbnailers().get(mime) else { return Err(NoThumb::Unsupported) };
    static N: AtomicU32 = AtomicU32::new(0);
    let out = std::env::temp_dir().join(format!("rfiles-thumb-{}-{}.png", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
    let args = exec_args(exec, path, uri, &out, px);
    let Some((prog, rest)) = args.split_first() else { return Err(NoThumb::Unsupported) };
    let fail = |e: anyhow::Error| NoThumb::Failed(e);
    let mut child = crate::sandbox::tool(prog, &[])
        .args(rest)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| fail(anyhow::anyhow!("{prog}: {e}")))?;
    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(s)) => break s,
            Ok(None) if started.elapsed() > TIMEOUT => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = std::fs::remove_file(&out);
                return Err(fail(anyhow::anyhow!("{prog} took longer than {TIMEOUT:?}")));
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(20)),
            Err(e) => return Err(fail(e.into())),
        }
    };
    let img = image::open(&out);
    let _ = std::fs::remove_file(&out);
    if !status.success() {
        return Err(fail(anyhow::anyhow!("{prog} exited with {status}")));
    }
    img.map(|i| fit(i.into_rgba8(), px)).map_err(|e| fail(e.into()))
}

/// Makes a thumbnail `px` pixels on its longer side, or says why not.
fn make(path: &Path, uri: &str, px: u32) -> Result<RgbaImage, NoThumb> {
    let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    if ext == "svg" || ext == "svgz" {
        return crate::xdg_icons::render_file(path, px)
            .map(|b| {
                let mut data = b.data;
                for p in data.chunks_exact_mut(4) {
                    p.swap(0, 2);
                }
                RgbaImage::from_raw(b.width, b.height, data).expect("sized buffer")
            })
            .ok_or_else(|| NoThumb::Failed(anyhow::anyhow!("unreadable SVG")));
    }
    if image::ImageFormat::from_extension(&ext).is_some() {
        let decoded = image::ImageReader::open(path).and_then(|r| r.with_guessed_format()).map_err(|e| NoThumb::Failed(e.into()))?;
        return decoded.decode().map(|i| fit(i.into_rgba8(), px)).map_err(|e| NoThumb::Failed(e.into()));
    }
    match crate::xdg_icons::mime_for_ext(&ext) {
        Some(mime) => external(path, uri, mime, px),
        None => Err(NoThumb::Unsupported),
    }
}

/// A thumbnail of `path` at most `size` pixels on its longer side.
pub fn thumbnail(path: &Path, size: u32) -> Result<RgbaImage> {
    let path = std::path::absolute(path)?;
    let meta = std::fs::metadata(&path)?;
    if !meta.is_file() {
        bail!("not a file");
    }
    let mtime = meta.modified()?.duration_since(UNIX_EPOCH)?.as_secs();
    let uri = file_uri(&path);
    let hash = md5_hex(&uri);
    let (class, px) = size_class(size);
    let root = cache_root();
    // thumbnails of the cache's own files are never cached (the spec says so)
    let cacheable = root.as_ref().is_some_and(|r| !path.starts_with(r));
    if let (Some(root), true) = (&root, cacheable) {
        if let Some(img) = read_valid(&root.join(class).join(format!("{hash}.png")), mtime) {
            return Ok(fit(img, size));
        }
        if read_valid(&root.join("fail/rfiles").join(format!("{hash}.png")), mtime).is_some() {
            bail!("failed before (file unchanged since)");
        }
    }
    match make(&path, &uri, px) {
        Ok(img) => {
            if let (Some(root), true) = (&root, cacheable) {
                if let Err(e) = write_png(&root.join(class).join(format!("{hash}.png")), &img, &uri, mtime) {
                    log::debug!("thumbnail cache: {e:#}");
                }
            }
            Ok(fit(img, size))
        }
        Err(NoThumb::Unsupported) => bail!("no thumbnailer for this type"),
        Err(NoThumb::Failed(e)) => {
            if let (Some(root), true) = (&root, cacheable) {
                let _ = write_png(&root.join("fail/rfiles").join(format!("{hash}.png")), &RgbaImage::new(1, 1), &uri, mtime);
            }
            Err(e)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uri_and_hash_like_glib() {
        assert_eq!(file_uri(Path::new("/home/me/a b/ü.png")), "file:///home/me/a%20b/%C3%BC.png");
        assert_eq!(file_uri(Path::new("/x/(1)+[2].png")), "file:///x/(1)+%5B2%5D.png");
        // the spec's example
        assert_eq!(md5_hex("file:///home/jens/photos/me.png"), "c6ee772d9e49320e97ec29a7eb5b1697");
    }

    #[test]
    fn exec_lines_expand() {
        let a = exec_args("ffmpegthumbnailer -i %i -o %o -s %s -f", Path::new("/v/a b.mp4"), "file:///v/a%20b.mp4", Path::new("/t/o.png"), 256);
        assert_eq!(a, ["ffmpegthumbnailer", "-i", "/v/a b.mp4", "-o", "/t/o.png", "-s", "256", "-f"]);
        let b = exec_args(r#"sh -c "x %u" 100%%"#, Path::new("/i"), "file:///i", Path::new("/o"), 128);
        assert_eq!(b, ["sh", "-c", "x file:///i", "100%"]);
    }

    /// Makes, caches and re-reads a thumbnail of a real image, in a private cache folder.
    #[test]
    fn caches_and_invalidates_on_change() {
        let base = std::env::temp_dir().join(format!("rf-thumbs-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        let pic = base.join("resim.png");
        RgbaImage::from_pixel(600, 300, image::Rgba([10, 200, 30, 255])).save(&pic).unwrap();
        let mtime = std::fs::metadata(&pic).unwrap().modified().unwrap().duration_since(UNIX_EPOCH).unwrap().as_secs();
        let uri = file_uri(&pic);
        let made = match make(&pic, &uri, 256) {
            Ok(i) => i,
            Err(_) => panic!("png decodes"),
        };
        assert_eq!((made.width(), made.height()), (256, 128));
        let cached = base.join("large").join(format!("{}.png", md5_hex(&uri)));
        write_png(&cached, &made, &uri, mtime).unwrap();
        assert!(read_valid(&cached, mtime).is_some());
        assert!(read_valid(&cached, mtime + 1).is_none(), "stale after the file changes");
        assert_eq!(fit(made, 64).dimensions(), (64, 32));
        let _ = std::fs::remove_dir_all(&base);
    }
}
