//! Hızlı Bakış resim çözücü: image paketi → resvg (SVG) → WIC (HEIC, AVIF, JXL, RAW…) → RAW'daki
//! gömülü JPEG → PSD birleşik görüntü → kabuk küçük resmi.
// Generated from rf-win's public API by a one-off stub generator, then edited by hand.
// Functions return safe defaults until they get a Linux implementation.
#![allow(unused)]
use std::io::Cursor;
use std::path::Path;
use anyhow::{Context, anyhow};
use image::{AnimationDecoder, DynamicImage, ImageDecoder, RgbaImage};
use crate::thumbs::Bgra;
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
/// Windows Imaging Component: kurulu codec'ler (HEIC, AVIF, JXL, JPEG XR, kamera RAW…).
fn wic(path: &Path, max_side: u32) -> anyhow::Result<Decoded> {
    Err(anyhow::anyhow!("rf-linux: `wic` is not implemented on Linux yet").into())
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
    if !RAW_EXTENSIONS.contains(&ext.as_str()) {
        if let Ok(r) = image::ImageReader::open(path).and_then(|r| r.with_guessed_format()) {
            if let Ok(mut d) = r.into_decoder() {
                let (w, h) = d.dimensions();
                let o = d.exif_metadata().ok().flatten().map_or(1, |raw| exif_fields(&raw).0);
                return Some(if (5..=8).contains(&o) { (h, w) } else { (w, h) });
            }
        }
    }
    // Windows asks WIC here (HEIC, AVIF, JXL, RAW); on Linux their size comes with the decode
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
    if let Ok(d) = wic(path, max_side) {
        return Ok(d);
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
    let t = crate::thumbs::get_image(
        path,
        max_side.min(1024),
        crate::thumbs::Kind::Thumbnail,
    )?;
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
}
