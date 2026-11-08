//! Rotating images 90° like Files' "Rotate left / right".
//!
//! * JPEG (and other formats whose property handler accepts it): only the EXIF orientation
//!   tag changes — lossless, instant, honored by Explorer, Photos, browsers and our own
//!   thumbnails (they come from the shell).
//! * PNG, BMP, TIFF, GIF (single frame), ICO…: the pixels are rotated through WIC and written
//!   back in the same container format (lossless for those formats), via a temp file that
//!   replaces the original only once it is complete.
//!
//! Blocking + COM: call off the UI thread.
// Generated from rf-win's public API by a one-off stub generator, then edited by hand.
// Functions return safe defaults until they get a Linux implementation.
#![allow(unused)]
use std::path::{Path, PathBuf};
use crate::shell::ensure_com;
use crate::wide::wide;
/// Formats we can rotate.
pub fn can_rotate(p: &Path) -> bool {
    let e = p
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    matches!(
        e.as_str(), "jpg" | "jpeg" | "jfif" | "jpe" | "png" | "bmp" | "tif" | "tiff" |
        "gif" | "ico" | "heic" | "webp"
    )
}
fn uses_orientation_tag(p: &Path) -> bool {
    let e = p
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    matches!(e.as_str(), "jpg" | "jpeg" | "jfif" | "jpe" | "heic")
}
/// Next EXIF orientation after a 90° turn (mirrored variants included).
pub fn next_orientation(cur: u16, clockwise: bool) -> u16 {
    const CW: [(u16, u16); 8] = [
        (1, 6),
        (6, 3),
        (3, 8),
        (8, 1),
        (2, 7),
        (7, 4),
        (4, 5),
        (5, 2),
    ];
    let cur = if (1..=8).contains(&cur) { cur } else { 1 };
    if clockwise {
        CW.iter().find(|(a, _)| *a == cur).map_or(6, |(_, b)| *b)
    } else {
        CW.iter().find(|(_, b)| *b == cur).map_or(8, |(a, _)| *a)
    }
}
/// Rotates one image by 90° (clockwise or counter-clockwise).
pub fn rotate(path: &Path, clockwise: bool) -> anyhow::Result<()> {
    anyhow::ensure!(can_rotate(path), "Bu dosya türü döndürülemez");
    ensure_com();
    if uses_orientation_tag(path) {
        match rotate_by_tag(path, clockwise) {
            Ok(()) => return Ok(()),
            Err(e) if is_jpeg(path) => return Err(e),
            Err(_) => {}
        }
    }
    rotate_pixels(path, clockwise)
}
fn is_jpeg(p: &Path) -> bool {
    let e = p
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    matches!(e.as_str(), "jpg" | "jpeg" | "jfif" | "jpe")
}
fn rotate_by_tag(path: &Path, clockwise: bool) -> anyhow::Result<()> {
    Err(
        anyhow::anyhow!("rf-linux: `rotate_by_tag` is not implemented on Linux yet")
            .into(),
    )
}
fn rotate_pixels(path: &Path, clockwise: bool) -> anyhow::Result<()> {
    Err(
        anyhow::anyhow!("rf-linux: `rotate_pixels` is not implemented on Linux yet")
            .into(),
    )
}
