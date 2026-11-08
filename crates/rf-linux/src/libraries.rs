//! Windows Libraries (Documents, Music, Pictures, Videos, user-created ones). A library is a
//! virtual merge of several folders; we resolve it to its file-system folders so the UI can show
//! the merged contents (Files only opens the default folder). Blocking: off the UI thread.
// Generated from rf-win's public API by a one-off stub generator, then edited by hand.
// Functions return safe defaults until they get a Linux implementation.
#![allow(unused)]
use std::path::PathBuf;
use crate::shell::ensure_com;
use crate::wide::wide;
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Library {
    /// The `.library-ms` file (stable identity).
    pub file: PathBuf,
    /// Localized display name ("Belgeler", "Music"…).
    pub name: String,
    /// File-system folders, in library order.
    pub folders: Vec<PathBuf>,
    /// Where new items go when the library is the target.
    pub default_save: Option<PathBuf>,
}
fn libraries_dir() -> Option<PathBuf> {
    std::env::var_os("APPDATA")
        .map(|a| PathBuf::from(a).join(r"Microsoft\Windows\Libraries"))
}
fn display(item: &crate::Opaque, sigdn: crate::Opaque) -> Option<String> {
    None
}
/// Loads one library file.
pub fn load(file: &std::path::Path) -> Option<Library> {
    None
}
/// All libraries of the current user, sorted by name. Libraries with no file-system folder
/// (empty, or only non-FS locations) are skipped.
pub fn list() -> Vec<Library> {
    let Some(dir) = libraries_dir() else { return Vec::new() };
    let Ok(rd) = std::fs::read_dir(&dir) else { return Vec::new() };
    let mut out: Vec<Library> = rd
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("library-ms")))
        .filter_map(|p| load(&p))
        .filter(|l| !l.folders.is_empty())
        .collect();
    out.sort_by(|a, b| rf_core::sort::natural_cmp(&a.name, &b.name));
    out
}
