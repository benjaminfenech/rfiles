//! Hızlı Bakış resim çözücü: image paketi → resvg (SVG) → jxl-oxide (JXL) → avifdec / heif-dec
//! (AVIF, HEIC) → RAW'daki gömülü JPEG → PSD birleşik görüntü → kabuk küçük resmi.
//!
//! On Linux, JPEG XL decodes in-process (pure Rust); AVIF and HEIC go through libavif's and
//! libheif's command-line decoders when installed, so building needs no system codec library.
// Generated from rf-win's public API by a one-off stub generator, then edited by hand.
// Functions return safe defaults until they get a Linux implementation.
#![allow(unused)]
use std::io::Cursor;
use std::path::Path;
use std::process::Command;
use std::time::Duration;
use anyhow::{Context, anyhow};
use image::{AnimationDecoder, DynamicImage, ImageDecoder, RgbaImage};
use crate::thumbs::Bgra;
use crate::tool::{NotInstalled, path_arg, run};
use super::heif;
/// Çözülmüş resim: kareler (BGRA, süre ms), yön (EXIF 1–8), EXIF alanları.
pub struct Decoded {
    pub frames: Vec<(Bgra, u32)>,
    /// EXIF yönü (1 = düz). Kareler zaten bu yöne göre döndürülmüş gelir.
    pub orientation: u8,
    pub exif: Vec<(String, String)>,
    /// Asıl boyut (döndürülmüş, küçültülmemiş).
    pub width: u32,
    pub height: u32,
}
const RAW_EXTENSIONS: &[&str] = &[
    "arw",
    "cr2",
    "cr3",
    "crw",
    "dng",
    "erf",
    "kdc",
    "mef",
    "mos",
    "mrw",
    "nef",
    "nrw",
    "orf",
    "pef",
    "raf",
    "raw",
    "rw2",
    "rwl",
    "sr2",
    "srf",
    "srw",
    "x3f",
];
fn to_bgra(img: RgbaImage) -> Bgra {
    let (width, height) = img.dimensions();
    let mut data = img.into_raw();
    for px in data.chunks_exact_mut(4) {
        px.swap(0, 2);
    }
    Bgra { width, height, data }
}
/// En uzun kenarı `max_side`'ı aşmayacak boyut.
fn fit(w: u32, h: u32, max_side: u32) -> (u32, u32) {
    let m = w.max(h);
    if m <= max_side || m == 0 {
        return (w, h);
    }
    let s = max_side as f64 / m as f64;
    (((w as f64 * s).round() as u32).max(1), ((h as f64 * s).round() as u32).max(1))
}
fn shrink(img: RgbaImage, max_side: u32) -> RgbaImage {
    let (w, h) = img.dimensions();
    let (nw, nh) = fit(w, h, max_side);
    if (nw, nh) == (w, h) {
        img
    } else {
        image::imageops::resize(&img, nw, nh, image::imageops::FilterType::Triangle)
    }
}
fn exif_fields(raw: &[u8]) -> (u8, Vec<(String, String)>) {
    let Ok(ex) = exif::Reader::new().read_raw(raw.to_vec()) else {
        return (1, Vec::new())
    };
    let orientation = ex
        .get_field(exif::Tag::Orientation, exif::In::PRIMARY)
        .and_then(|f| f.value.get_uint(0))
        .map_or(1, |v| v.clamp(1, 8) as u8);
    let fields = ex
        .fields()
        .filter(|f| {
            !matches!(f.tag, exif::Tag::MakerNote | exif::Tag::UserComment)
                && f.tag.description().is_some()
        })
        .map(|f| (
            f.tag.to_string(),
            f.display_value().with_unit(&ex).to_string().trim_matches('"').to_string(),
        ))
        .filter(|(_, v)| !v.is_empty() && v.len() < 200)
        .collect();
    (orientation, fields)
}
fn orient(img: DynamicImage, o: u8) -> DynamicImage {
    match o {
        2 => img.fliph(),
        3 => img.rotate180(),
        4 => img.flipv(),
        5 => img.rotate90().fliph(),
        6 => img.rotate90(),
        7 => img.rotate270().fliph(),
        8 => img.rotate270(),
        _ => img,
    }
}
fn frames_of<'a>(
    frames: image::Frames<'a>,
    max_side: u32,
) -> anyhow::Result<Vec<(Bgra, u32)>> {
    let mut out = Vec::new();
    for f in frames {
        let f = f?;
        let (n, d) = f.delay().numer_denom_ms();
        let ms = if d == 0 { 100 } else { (n / d).max(20) };
        out.push((to_bgra(shrink(f.into_buffer(), max_side)), ms));
    }
    Ok(out)
}
/// Animasyonlu GIF / APNG / WebP: birden çok kare varsa onları döner.
fn animated(
    bytes: &[u8],
    format: image::ImageFormat,
    max_side: u32,
) -> Option<Vec<(Bgra, u32)>> {
    use image::codecs::{gif::GifDecoder, png::PngDecoder, webp::WebPDecoder};
    let frames = match format {
        image::ImageFormat::Gif => {
            frames_of(GifDecoder::new(Cursor::new(bytes)).ok()?.into_frames(), max_side)
                .ok()?
        }
        image::ImageFormat::Png => {
            let d = PngDecoder::new(Cursor::new(bytes)).ok()?;
            if !d.is_apng().ok()? {
                return None;
            }
            frames_of(d.apng().ok()?.into_frames(), max_side).ok()?
        }
        image::ImageFormat::WebP => {
            let d = WebPDecoder::new(Cursor::new(bytes)).ok()?;
            if !d.has_animation() {
                return None;
            }
            frames_of(d.into_frames(), max_side).ok()?
        }
        _ => return None,
    };
    (frames.len() > 1).then_some(frames)
}
fn with_image_crate(bytes: &[u8], max_side: u32) -> anyhow::Result<Decoded> {
    let reader = image::ImageReader::new(Cursor::new(bytes)).with_guessed_format()?;
    let format = reader.format().ok_or_else(|| anyhow!("unknown format"))?;
    if let Some(frames) = animated(bytes, format, max_side) {
        let (width, height) = (frames[0].0.width, frames[0].0.height);
        return Ok(Decoded {
            frames,
            orientation: 1,
            exif: Vec::new(),
            width,
            height,
        });
    }
    let mut dec = reader.into_decoder()?;
    let (orientation, exif) = dec
        .exif_metadata()
        .ok()
        .flatten()
        .map(|r| exif_fields(&r))
        .unwrap_or((1, Vec::new()));
    let img = orient(DynamicImage::from_decoder(dec)?, orientation);
    let (width, height) = (img.width(), img.height());
    Ok(Decoded {
        frames: vec![(to_bgra(shrink(img.into_rgba8(), max_side)), 0)],
        orientation,
        exif,
        width,
        height,
    })
}
fn svg(bytes: &[u8], max_side: u32) -> anyhow::Result<Decoded> {
    use resvg::{tiny_skia, usvg};
    let mut opt = usvg::Options::default();
    // sistem yazı tipleri yalnız metin içeren SVG'de gerekir; yüklemesi pahalı, bir kez yapılır
    if bytes.windows(5).any(|w| w == b"<text") {
        static FONTS: std::sync::OnceLock<std::sync::Arc<usvg::fontdb::Database>> = std::sync::OnceLock::new();
        opt.fontdb = FONTS
            .get_or_init(|| {
                let mut db = usvg::fontdb::Database::new();
                db.load_system_fonts();
                std::sync::Arc::new(db)
            })
            .clone();
    }
    let tree = usvg::Tree::from_data(bytes, &opt)?;
    let size = tree.size();
    let (w, h) = (size.width().max(1.0), size.height().max(1.0));
    // küçük SVG'ler net görünsün diye en az 1024 piksele büyütülür
    let target = max_side.min(4096).max(1) as f32;
    let s = (target.min(w.max(h).max(1024.0)) / w.max(h)).max(0.01);
    let (pw, ph) = ((w * s).ceil() as u32, (h * s).ceil() as u32);
    let mut pix = tiny_skia::Pixmap::new(pw.max(1), ph.max(1)).ok_or_else(|| anyhow!("svg size"))?;
    resvg::render(&tree, tiny_skia::Transform::from_scale(s, s), &mut pix.as_mut());
    // tiny-skia önceden çarpılmış RGBA verir: düz alfaya çevir
    let mut data = pix.take();
    for px in data.chunks_exact_mut(4) {
        let a = px[3] as u32;
        if a > 0 && a < 255 {
            for c in &mut px[..3] {
                *c = ((*c as u32 * 255 + a / 2) / a).min(255) as u8;
            }
        }
        px.swap(0, 2);
    }
    Ok(Decoded { frames: vec![(Bgra { width: pw, height: ph, data }, 0)], orientation: 1, exif: Vec::new(), width: w.round() as u32, height: h.round() as u32 })
}
/// HEIF containers: HEIC (HEVC) and AVIF (AV1).
const HEIF_EXTENSIONS: &[&str] = &["heic", "heif", "hif", "avif"];

/// How long an external decoder may take for one image.
const TOOL_TIMEOUT: Duration = Duration::from_secs(30);

/// No decoder for this format is installed; the message says what to install.
#[derive(Debug)]
pub struct MissingDecoder(pub &'static str);

impl std::fmt::Display for MissingDecoder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0)
    }
}

impl std::error::Error for MissingDecoder {}

fn one_frame(img: RgbaImage, exif: Vec<(String, String)>, max_side: u32) -> Decoded {
    let (width, height) = img.dimensions();
    Decoded { frames: vec![(to_bgra(shrink(img, max_side)), 0)], orientation: 1, exif, width, height }
}

/// EXIF fields of a HEIF file (HEIC, AVIF). Not applied: the decoders turn the image with the
/// container's own rotation, which HEIF says wins over EXIF's.
fn heif_exif(bytes: &[u8]) -> Vec<(String, String)> {
    heif::exif(bytes).map(|t| exif_fields(t).1).unwrap_or_default()
}

/// JPEG XL in pure Rust; jxl-oxide turns the image upright itself.
fn jxl(bytes: &[u8], max_side: u32) -> anyhow::Result<Decoded> {
    let mut dec = jxl_oxide::integration::JxlDecoder::new(Cursor::new(bytes))?;
    let exif = dec.exif_metadata().ok().flatten().map(|r| exif_fields(&r).1).unwrap_or_default();
    let img = DynamicImage::from_decoder(dec)?.into_rgba8();
    Ok(one_frame(img, exif, max_side))
}

/// AVIF / HEIC through libavif's or libheif's command-line decoder, into a PNG in a private
/// temporary folder. Both apply the container's rotation and mirroring.
fn heif_tools(path: &Path, ext: &str, bytes: &[u8], max_side: u32) -> anyhow::Result<Decoded> {
    // heif-convert is heif-dec's name before libheif 1.17
    let tools: &[(&str, &[&str])] = if ext == "avif" {
        &[("avifdec", &["--png-compress", "0", "-d", "8"]), ("heif-dec", &[]), ("heif-convert", &[])]
    } else {
        &[("heif-dec", &[]), ("heif-convert", &[])]
    };
    static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("rustyfiles-look-{}-{n}", std::process::id()));
    {
        use std::os::unix::fs::DirBuilderExt;
        std::fs::DirBuilder::new().mode(0o700).create(&dir).with_context(|| dir.display().to_string())?;
    }
    let result = (|| {
        let out = dir.join("out.png");
        let mut last: Option<anyhow::Error> = None;
        for (prog, args) in tools {
            let mut cmd = crate::sandbox::tool(prog, &[]);
            cmd.args(*args).arg(path_arg(path)).arg(&out);
            match run(cmd, TOOL_TIMEOUT) {
                Err(e) if e.is::<NotInstalled>() => continue,
                Err(e) => last = Some(e),
                Ok(r) if !r.ok => last = Some(anyhow!("{prog}: {}", String::from_utf8_lossy(&r.stderr).trim())),
                Ok(_) => match written_image(&dir) {
                    Ok(mut img) => {
                        if *prog == "avifdec" && !avifdec_transforms() {
                            img = upright(img, bytes);
                        }
                        return Ok(one_frame(img, heif_exif(bytes), max_side));
                    }
                    Err(e) => last = Some(e.context(*prog)),
                },
            }
        }
        Err(last.unwrap_or_else(|| {
            MissingDecoder(if ext == "avif" {
                "To preview AVIF images at full size, install libavif (avifdec) or libheif (heif-dec)."
            } else {
                "To preview HEIC images, install libheif (it provides heif-dec)."
            })
            .into()
        }))
    })();
    let _ = std::fs::remove_dir_all(&dir);
    result
}

/// Whether `avifdec` applies the rotation and mirroring itself (libavif 1.4.0 and later).
fn avifdec_transforms() -> bool {
    static ANSWER: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ANSWER.get_or_init(|| {
        let mut cmd = crate::sandbox::tool("avifdec", &[]);
        cmd.arg("--version");
        let ran = run(cmd, Duration::from_secs(5)).ok();
        let text = ran.map(|r| String::from_utf8_lossy(&r.stdout).into_owned()).unwrap_or_default();
        avifdec_version_transforms(&text)
    })
}

/// From `avifdec --version` output ("Version: 1.0.4 (dav1d …)"): 1.4.0 or later. Unreadable
/// counts as new.
fn avifdec_version_transforms(text: &str) -> bool {
    let Some(v) = text.lines().find_map(|l| l.trim().strip_prefix("Version:")) else { return true };
    let mut parts = v.trim().split(|c: char| !c.is_ascii_digit()).map(|n| n.parse::<u32>().unwrap_or(0));
    let (major, minor) = (parts.next().unwrap_or(0), parts.next().unwrap_or(0));
    (major, minor) >= (1, 4)
}

/// Applies the container's `irot`, then `imir` (HEIF's order), to an image decoded as stored.
fn upright(img: RgbaImage, bytes: &[u8]) -> RgbaImage {
    use image::imageops;
    let Some(p) = heif::primary(bytes) else { return img };
    // already turned (a different tool, or a newer one than its version says)
    if (img.width(), img.height()) == p.shown_size() && p.quarter_turns % 2 == 1 {
        return img;
    }
    let img = match p.quarter_turns {
        1 => imageops::rotate270(&img),
        2 => imageops::rotate180(&img),
        3 => imageops::rotate90(&img),
        _ => img,
    };
    match p.mirror {
        Some(0) => imageops::flip_horizontal(&img),
        Some(_) => imageops::flip_vertical(&img),
        None => img,
    }
}

/// The decoded image a tool left in `dir`: `out.png`, or when it numbered several top-level
/// images (`out-1.png`, …), the largest one; depth and other auxiliary images are skipped.
fn written_image(dir: &Path) -> anyhow::Result<RgbaImage> {
    let main = dir.join("out.png");
    if main.exists() {
        return Ok(image::open(&main)?.into_rgba8());
    }
    let mut best: Option<(u64, std::path::PathBuf)> = None;
    for e in std::fs::read_dir(dir)?.flatten() {
        let p = e.path();
        let name = e.file_name().to_string_lossy().to_ascii_lowercase();
        if !name.ends_with(".png") || name.contains("depth") || name.contains("aux") || name.contains("urn:") {
            continue;
        }
        if let Ok((w, h)) = image::image_dimensions(&p) {
            let area = w as u64 * h as u64;
            if best.as_ref().is_none_or(|(a, _)| area > *a) {
                best = Some((area, p));
            }
        }
    }
    let (_, p) = best.context("the decoder wrote no image")?;
    Ok(image::open(&p)?.into_rgba8())
}
/// Kamera RAW dosyasının içine gömülü en büyük JPEG önizleme.
fn raw_embedded_jpeg(bytes: &[u8], max_side: u32) -> anyhow::Result<Decoded> {
    let mut best: Option<(usize, u64)> = None;
    let mut i = 0;
    while i + 3 < bytes.len() {
        if bytes[i] == 0xFF && bytes[i + 1] == 0xD8 && bytes[i + 2] == 0xFF {
            let r = image::ImageReader::with_format(
                Cursor::new(&bytes[i..]),
                image::ImageFormat::Jpeg,
            );
            if let Ok((w, h)) = r.into_dimensions() {
                let area = w as u64 * h as u64;
                if best.is_none_or(|(_, a)| area > a) {
                    best = Some((i, area));
                }
            }
        }
        i += 1;
    }
    let (start, _) = best.context("no embedded JPEG")?;
    with_image_crate(&bytes[start..], max_side)
}
fn psd(bytes: &[u8], max_side: u32) -> anyhow::Result<Decoded> {
    let p = psd::Psd::from_bytes(bytes).map_err(|e| anyhow!("psd: {e}"))?;
    let img = RgbaImage::from_raw(p.width(), p.height(), p.rgba())
        .context("psd pixels")?;
    let (width, height) = img.dimensions();
    Ok(Decoded {
        frames: vec![(to_bgra(shrink(img, max_side)), 0)],
        orientation: 1,
        exif: Vec::new(),
        width,
        height,
    })
}
/// Resmin (EXIF yönüne göre döndürülmüş) boyutu; yalnız başlık okunur, hızlıdır.
/// The image's size (EXIF rotation applied) without decoding it: from the header for what
/// the image crate reads, SVG's own size.
pub fn dimensions(path: &Path) -> Option<(u32, u32)> {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    if ext == "svg" || ext == "svgz" {
        let bytes = std::fs::read(path).ok()?;
        let tree = resvg::usvg::Tree::from_data(&bytes, &resvg::usvg::Options::default()).ok()?;
        return Some((tree.size().width().round() as u32, tree.size().height().round() as u32));
    }
    if HEIF_EXTENSIONS.contains(&ext.as_str()) {
        use std::io::Read;
        // the `meta` box comes first in practice; 1 MiB is plenty
        let mut head = Vec::new();
        std::fs::File::open(path).ok()?.take(1 << 20).read_to_end(&mut head).ok()?;
        return heif::primary_size(&head);
    }
    if ext == "jxl" {
        let f = std::io::BufReader::new(std::fs::File::open(path).ok()?);
        return jxl_oxide::integration::JxlDecoder::new(f).ok().map(|d| d.dimensions());
    }
    if !RAW_EXTENSIONS.contains(&ext.as_str()) {
        if let Ok(r) = image::ImageReader::open(path).and_then(|r| r.with_guessed_format()) {
            if let Ok(mut d) = r.into_decoder() {
                let (w, h) = d.dimensions();
                let o = d.exif_metadata().ok().flatten().map_or(1, |raw| exif_fields(&raw).0);
                return Some(if (5..=8).contains(&o) { (h, w) } else { (w, h) });
            }
        }
    }
    // Windows asks WIC here for RAW too; on Linux a RAW file's size comes with the decode
    None
}
/// Resmi tam çözünürlükte (en uzun kenar en çok `max_side`) çözer; sırayla dener.
pub fn decode_full(path: &Path, max_side: u32) -> anyhow::Result<Decoded> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let bytes = std::fs::read(path).with_context(|| path.display().to_string())?;
    if ext == "svg" || ext == "svgz" {
        return svg(&bytes, max_side);
    }
    let raw = RAW_EXTENSIONS.contains(&ext.as_str());
    if !raw {
        if let Ok(d) = with_image_crate(&bytes, max_side) {
            return Ok(d);
        }
    }
    if ext == "jxl" {
        match jxl(&bytes, max_side) {
            Ok(d) => return Ok(d),
            Err(e) => log::debug!("jxl {}: {e:#}", path.display()),
        }
    }
    if HEIF_EXTENSIONS.contains(&ext.as_str()) {
        match heif_tools(path, &ext, &bytes, max_side) {
            Ok(d) => return Ok(d),
            Err(e) if e.is::<MissingDecoder>() => {
                // a real thumbnail (at most 1024 px) beats the hint; the file-type icon doesn't
                return crate::xdg_thumbs::thumbnail(path, max_side.min(1024))
                    .map(|img| one_frame(img, heif_exif(&bytes), max_side))
                    .map_err(|_| e);
            }
            Err(e) => log::debug!("heif {}: {e:#}", path.display()),
        }
    }
    if raw {
        if let Ok(d) = raw_embedded_jpeg(&bytes, max_side) {
            return Ok(d);
        }
    }
    if ext == "psd" || ext == "psb" {
        if let Ok(d) = psd(&bytes, max_side) {
            return Ok(d);
        }
    }
    let t = crate::thumbs::get_image(path, max_side.min(1024), crate::thumbs::Kind::Thumbnail)?;
    let (width, height) = (t.width, t.height);
    Ok(Decoded {
        frames: vec![(t, 0)],
        orientation: 1,
        exif: Vec::new(),
        width,
        height,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_without_decoding_and_svg() {
        let d = std::env::temp_dir().join(format!("rf-look-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        let png = d.join("a.png");
        RgbaImage::from_pixel(300, 120, image::Rgba([1, 2, 3, 255])).save(&png).unwrap();
        assert_eq!(dimensions(&png), Some((300, 120)));
        let svg_path = d.join("b.svg");
        std::fs::write(&svg_path, r#"<svg xmlns="http://www.w3.org/2000/svg" width="400" height="100"><rect width="400" height="100" fill="red"/></svg>"#).unwrap();
        assert_eq!(dimensions(&svg_path), Some((400, 100)));
        let full = decode_full(&svg_path, 2000).unwrap();
        assert_eq!((full.width, full.height), (400, 100));
        let (f, _) = &full.frames[0];
        assert!(f.width >= 400 && f.data.chunks_exact(4).any(|p| p == [0, 0, 255, 255]), "red, as BGRA");
        let _ = std::fs::remove_dir_all(&d);
    }

    fn testdata(name: &str) -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata").join(name)
    }

    /// BGRA pixel at (x, y), as RGB.
    fn rgb(b: &Bgra, x: u32, y: u32) -> [u8; 3] {
        let p = &b.data[((y * b.width + x) * 4) as usize..][..4];
        [p[2], p[1], p[0]]
    }

    fn near(a: [u8; 3], b: [u8; 3]) -> bool {
        a.iter().zip(b).all(|(x, y)| x.abs_diff(y) < 40)
    }

    fn have(prog: &str) -> bool {
        Command::new(prog).arg("--version").stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).status().is_ok()
    }

    // small.png: 60 x 40, white, with a red 20 x 10 block in the top-left corner

    #[test]
    fn jpeg_xl_in_process() {
        let p = testdata("small.jxl");
        assert_eq!(dimensions(&p), Some((60, 40)));
        let d = decode_full(&p, 2000).unwrap();
        assert_eq!((d.width, d.height), (60, 40));
        let f = &d.frames[0].0;
        assert!(near(rgb(f, 2, 2), [255, 0, 0]) && near(rgb(f, 50, 30), [255, 255, 255]));
    }

    #[test]
    fn avif_turned_a_quarter() {
        // rotated.avif is small.png with irot 1 (a quarter turn anticlockwise): 40 x 60, red
        // block bottom-left
        let p = testdata("rotated.avif");
        assert_eq!(dimensions(&p), Some((40, 60)));
        if !have("avifdec") && !have("heif-dec") {
            eprintln!("no AVIF decoder installed; skipped");
            return;
        }
        let d = decode_full(&p, 2000).unwrap();
        assert_eq!((d.width, d.height), (40, 60));
        let f = &d.frames[0].0;
        assert!(near(rgb(f, 2, 57), [255, 0, 0]) && near(rgb(f, 2, 2), [255, 255, 255]));
    }

    #[test]
    fn old_avifdec_is_turned_here() {
        assert!(!avifdec_version_transforms("Version: 1.0.4 (dav1d [dec]:1.4.1)\nlibyuv : available"));
        assert!(avifdec_version_transforms("Version: 1.4.2 (dav1d [dec]:1.5.4)"));
        assert!(avifdec_version_transforms("Version: 2.0.0"));
        assert!(avifdec_version_transforms("no version line"));
        // rotated.avif decoded as stored (60 x 40, red top-left), turned upright here
        let stored = image::open(testdata("small.png")).unwrap().into_rgba8();
        let bytes = std::fs::read(testdata("rotated.avif")).unwrap();
        let up = upright(stored, &bytes);
        assert_eq!(up.dimensions(), (40, 60));
        assert_eq!(up.get_pixel(2, 57).0, [255, 0, 0, 255]);
        assert_eq!(up.get_pixel(2, 2).0, [255, 255, 255, 255]);
    }

    #[test]
    fn heic_through_libheif() {
        if !have("heif-enc") || !have("heif-dec") {
            eprintln!("libheif's tools not installed; skipped");
            return;
        }
        let dir = std::env::temp_dir().join(format!("rf-heic-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("small.heic");
        let ok = Command::new("heif-enc").arg("-q").arg("90").arg(testdata("small.png")).arg("-o").arg(&p).status().unwrap();
        if !ok.success() {
            // libheif without its HEVC encoder plugin
            eprintln!("heif-enc can't encode HEIC here; skipped");
            return;
        }
        assert_eq!(dimensions(&p), Some((60, 40)));
        let d = decode_full(&p, 2000).unwrap();
        assert_eq!((d.width, d.height), (60, 40));
        assert!(near(rgb(&d.frames[0].0, 2, 2), [255, 0, 0]));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
