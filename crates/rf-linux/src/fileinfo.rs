//! Facts for the Properties window: times, attributes, size on disk, default app. Blocking
//! (disk / registry): call off the UI thread.
// Generated from rf-win's public API by a one-off stub generator, then edited by hand.
// Functions return safe defaults until they get a Linux implementation.
use std::path::Path;
#[derive(Debug, Clone, Default)]
pub struct Info {
    pub is_dir: bool,
    pub size: u64,
    /// Allocated size (compression / sparse / cluster rounding). Files only.
    pub size_on_disk: Option<u64>,
    pub created_ms: i64,
    pub modified_ms: i64,
    pub accessed_ms: i64,
    pub attrs: u32,
    /// Friendly name of the app that opens this type ("Fotoğraflar"). Files only.
    pub opens_with: Option<String>,
}
fn ms(t: std::io::Result<std::time::SystemTime>) -> i64 {
    t.ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}
/// Windows-style attribute bits for the Properties window: read-only (no write permission
/// for anyone), hidden (dot name), directory.
fn unix_attrs(path: &Path, m: &std::fs::Metadata) -> u32 {
    let mut a = 0;
    if m.permissions().readonly() {
        a |= 0x1;
    }
    if path.file_name().is_some_and(|n| n.to_string_lossy().starts_with('.')) {
        a |= 0x2;
    }
    if m.is_dir() {
        a |= 0x10;
    }
    a
}
pub fn info(path: &Path) -> std::io::Result<Info> {
    let m = std::fs::metadata(path)?;
    let is_dir = m.is_dir();
    Ok(Info {
        is_dir,
        size: if is_dir { 0 } else { m.len() },
        size_on_disk: if is_dir { None } else { size_on_disk(path) },
        created_ms: ms(m.created()),
        modified_ms: ms(m.modified()),
        accessed_ms: ms(m.accessed()),
        attrs: unix_attrs(path, &m),
        opens_with: if is_dir { None } else { opens_with(path) },
    })
}
/// Bytes the file occupies: compressed size rounded up to whole clusters.
pub fn size_on_disk(_path: &Path) -> Option<u64> {
    None
}
/// Friendly name of the default app for the file's type.
/// The name of the app that opens this file by default (Properties: "Opens with").
pub fn opens_with(path: &Path) -> Option<String> {
    crate::xdg_apps::default_app(&crate::xdg_apps::mime_for_path(path)).map(|a| a.name.clone())
}
