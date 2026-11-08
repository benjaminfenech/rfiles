//! Recycle Bin through the shell namespace (list / restore / empty / info).
//! Blocking + COM: call off the UI thread.
// Generated from rf-win's public API by a one-off stub generator, then edited by hand.
// Functions return safe defaults until they get a Linux implementation.
#![allow(unused)]
use crate::shell::ensure_com;
/// One item in the Recycle Bin.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecycleItem {
    /// Original file name as shown by Explorer.
    pub name: String,
    /// Folder the item was deleted from.
    pub original_location: String,
    /// Deletion time, Unix milliseconds (UTC); 0 when unknown.
    pub deleted_ms: i64,
    pub size: u64,
    /// Real location inside `$Recycle.Bin\<SID>\$R...`; also the key used by [`restore`].
    pub parsing_path: String,
    pub is_dir: bool,
}
const FMTID_DISPLACED: crate::Opaque = 0;
const PKEY_DELETED_FROM: crate::Opaque = 0;
const PKEY_DATE_DELETED: crate::Opaque = 0;
const PKEY_SIZE: crate::Opaque = 0;
struct Pidl(*mut crate::Opaque);
impl Drop for Pidl {
    fn drop(&mut self) {}
}
fn bin_folder() -> anyhow::Result<crate::Opaque> {
    Err(anyhow::anyhow!("rf-linux: `bin_folder` is not implemented on Linux yet").into())
}
/// Calls `f(folder, relative_pidl)` for each item in the bin.
fn for_each_item(
    folder: &crate::Opaque,
    f: impl FnMut(&crate::Opaque, *mut crate::Opaque),
) -> anyhow::Result<()> {
    Err(
        anyhow::anyhow!("rf-linux: `for_each_item` is not implemented on Linux yet")
            .into(),
    )
}
fn display_name(
    folder: &crate::Opaque,
    pidl: *mut crate::Opaque,
    flags: crate::Opaque,
) -> String {
    Default::default()
}
fn detail(
    folder: &crate::Opaque,
    pidl: *mut crate::Opaque,
    key: &crate::Opaque,
) -> Option<crate::Opaque> {
    None
}
/// Lists everything in the Recycle Bin (all drives), in shell order.
pub fn list() -> anyhow::Result<Vec<RecycleItem>> {
    anyhow::bail!("rf-linux: the Trash is not implemented on Linux yet")
}
/// Restores `items` to their original locations by invoking the shell's "undelete" verb
/// (name-collision prompts come from the shell). Items no longer in the bin are ignored;
/// errors if none matched.
pub fn restore(items: &[RecycleItem]) -> anyhow::Result<()> {
    Err(anyhow::anyhow!("rf-linux: `restore` is not implemented on Linux yet").into())
}
/// Empties the bin on all drives. With `confirm == false` no confirmation dialog, progress UI or
/// sound is shown (the caller is expected to have asked already).
pub fn empty(confirm: bool) -> anyhow::Result<()> {
    Err(anyhow::anyhow!("rf-linux: `empty` is not implemented on Linux yet").into())
}
/// `(item_count, total_bytes)` across all drives.
pub fn info() -> anyhow::Result<(u64, u64)> {
    Err(anyhow::anyhow!("rf-linux: `info` is not implemented on Linux yet").into())
}
