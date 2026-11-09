//! Windows shell folders that aren't on the file system (an MTP phone under This PC, Libraries,
//! Network), listed through the shell namespace (upstream, after 0.4.0). Linux has no shell
//! namespace: phones and cameras show up as mounted folders (gvfs / kio-fuse MTP), so nothing
//! here is ever browsable and no `Location::Shell` is created. Same API as rf-win's.

use std::path::{Path, PathBuf};

/// "This PC" (a Windows name: never resolved on Linux).
pub const THIS_PC: &str = "::{20D04FE0-3AEA-1069-A2D8-08002B30309D}";

/// One child of a shell folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShellChild {
    pub name: String,
    pub parsing: String,
    pub is_folder: bool,
    pub fs_path: Option<PathBuf>,
    pub size: Option<u64>,
    /// Unix time in milliseconds.
    pub modified_ms: Option<i64>,
}

/// A resolved shell folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Place {
    pub parsing: String,
    pub name: String,
    pub fs_path: Option<PathBuf>,
}

pub fn list(_parsing: &str, _cancel: &dyn Fn() -> bool, _on_batch: &mut dyn FnMut(Vec<ShellChild>)) -> anyhow::Result<usize> {
    anyhow::bail!("shell folders exist on Windows only")
}

pub fn display_name(_parsing: &str) -> Option<String> {
    None
}

pub fn parent(_parsing: &str) -> Option<Place> {
    None
}

pub fn is_applet(_parsing: &str) -> bool {
    false
}

/// Linux: never (see the module comment).
pub fn browsable(_s: &str) -> Option<Place> {
    None
}

/// Linux: none (phones are mounted folders, listed with the drives).
pub fn devices() -> Vec<ShellChild> {
    Vec::new()
}

pub fn copy_out(_sources: &[String], _dest: &Path) -> anyhow::Result<bool> {
    anyhow::bail!("shell folders exist on Windows only")
}

pub fn open(_parsing: &str) -> anyhow::Result<()> {
    anyhow::bail!("shell folders exist on Windows only")
}

/// Last part of a parsing name: `...\Documents` → `Documents`.
pub fn last_segment(parsing: &str) -> &str {
    let t = parsing.trim_end_matches('\\');
    t.rsplit('\\').next().unwrap_or(t)
}
