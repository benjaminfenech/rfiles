//! `.lnk` shortcuts via IShellLinkW + IPersistFile. Blocking + COM: call off the UI thread.
// Generated from rf-win's public API by a one-off stub generator, then edited by hand.
// Functions return safe defaults until they get a Linux implementation.
#![allow(unused)]
use std::path::{Path, PathBuf};
use crate::shell::ensure_com;
use crate::wide::{from_wide, wide};
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ShortcutInfo {
    /// Empty when the target is not a file-system path (e.g. a Control Panel item).
    pub target: PathBuf,
    pub args: String,
    pub working_dir: PathBuf,
    /// Icon file ("" = use the target's icon) and its resource index.
    pub icon: String,
    pub icon_index: i32,
}
/// Reads a `.lnk` file.
pub fn resolve(path: &Path) -> anyhow::Result<ShortcutInfo> {
    Err(anyhow::anyhow!("rf-linux: `resolve` is not implemented on Linux yet").into())
}
/// Creates (or overwrites) `link_path` (should end in `.lnk`) pointing at `target`.
/// The working directory defaults to the target's folder (or the target itself for folders).
pub fn create(link_path: &Path, target: &Path) -> anyhow::Result<()> {
    Err(anyhow::anyhow!("rf-linux: `create` is not implemented on Linux yet").into())
}

/// What the Properties window's Shortcut tab shows and edits: Windows `.lnk` files, which
/// Linux doesn't read (the tab isn't shown). Same fields as rf-win's.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Details {
    pub target: String,
    pub args: String,
    pub working_dir: String,
    pub comment: String,
    pub show: i32,
    pub hotkey: u16,
    pub icon: String,
    pub icon_index: i32,
    pub run_as_admin: bool,
}

pub fn details(_path: &Path) -> anyhow::Result<Details> {
    anyhow::bail!("Windows shortcuts (.lnk) aren't read on Linux")
}

pub fn save(_path: &Path, _d: &Details) -> anyhow::Result<()> {
    anyhow::bail!("Windows shortcuts (.lnk) aren't written on Linux")
}

pub fn hotkey_text(_hotkey: u16) -> String {
    String::new()
}
