//! Type-specific file actions from Files' context menu: set as desktop background, install a
//! font (for the current user — no administrator rights, no UAC). Blocking: off the UI thread.
// Generated from rf-win's public API by a one-off stub generator, then edited by hand.
// Functions return safe defaults until they get a Linux implementation.
#![allow(unused)]
use std::path::{Path, PathBuf};
use crate::wide::wide;
pub fn is_image(p: &Path) -> bool {
    let e = p
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    matches!(
        e.as_str(), "jpg" | "jpeg" | "png" | "bmp" | "gif" | "tif" | "tiff" | "webp" |
        "jfif" | "heic" | "avif"
    )
}
pub fn is_font(p: &Path) -> bool {
    let e = p
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    matches!(e.as_str(), "ttf" | "otf" | "ttc" | "fon")
}
/// Executables that "Run as administrator" applies to.
pub fn is_runnable(p: &Path) -> bool {
    let e = p
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    matches!(e.as_str(), "exe" | "bat" | "cmd" | "msi" | "ps1" | "com")
}
pub fn set_wallpaper(path: &Path) -> anyhow::Result<()> {
    Err(
        anyhow::anyhow!("rf-linux: `set_wallpaper` is not implemented on Linux yet")
            .into(),
    )
}
/// Installs a font for the current user: copies it into
/// `%LOCALAPPDATA%\Microsoft\Windows\Fonts`, registers it under HKCU and tells running apps.
/// Returns the installed copy.
pub fn install_font_for_user(path: &Path) -> anyhow::Result<PathBuf> {
    Err(
        anyhow::anyhow!(
            "rf-linux: `install_font_for_user` is not implemented on Linux yet"
        )
            .into(),
    )
}
