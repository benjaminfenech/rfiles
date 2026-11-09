//! Facts for the Properties window: times, attributes, size on disk, default app. Blocking
//! (disk / registry): call off the UI thread.
// Generated from rf-win's public API by a one-off stub generator, then edited by hand.
// Functions return safe defaults until they get a Linux implementation.
#![allow(unused)]
use std::path::Path;
use crate::wide::{from_wide, wide};
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
pub fn size_on_disk(path: &Path) -> Option<u64> {
    None
}
fn cluster_size(path: &Path) -> Option<u64> {
    None
}
/// Friendly name of the default app for the file's type.
/// The name of the app that opens this file by default (Properties: "Opens with").
pub fn opens_with(path: &Path) -> Option<String> {
    crate::xdg_apps::default_app(&crate::xdg_apps::mime_for_path(path)).map(|a| a.name.clone())
}
/// Sets / clears read-only and hidden, keeping the other attribute bits.
///
/// Linux: read-only takes the write permission from everyone, and clearing it gives it back to
/// the owner. Hidden is a name starting with a dot, which a flag can't change.
pub fn set_flags(path: &Path, readonly: Option<bool>, hidden: Option<bool>) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    if hidden.is_some() {
        return Err(std::io::Error::new(std::io::ErrorKind::Unsupported, "on Linux a hidden item's name starts with a dot"));
    }
    if let Some(ro) = readonly {
        let mut perm = std::fs::metadata(path)?.permissions();
        let mode = perm.mode();
        perm.set_mode(if ro { mode & !0o222 } else { mode | 0o200 });
        std::fs::set_permissions(path, perm)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn read_only_is_the_write_permission() {
        let p = std::env::temp_dir().join(format!("rf-flags-{}", std::process::id()));
        std::fs::write(&p, b"x").unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o664)).unwrap();
        set_flags(&p, Some(true), None).unwrap();
        assert_eq!(std::fs::metadata(&p).unwrap().permissions().mode() & 0o777, 0o444);
        assert_eq!(info(&p).unwrap().attrs & 0x1, 0x1, "shows as read-only");
        set_flags(&p, Some(false), None).unwrap();
        assert_eq!(std::fs::metadata(&p).unwrap().permissions().mode() & 0o777, 0o644);
        assert!(set_flags(&p, None, Some(true)).is_err(), "hidden is a dot name");
        let _ = std::fs::remove_file(&p);
    }
}
