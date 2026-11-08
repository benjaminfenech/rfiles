//! Explorer-compatible file clipboard: CF_HDROP + "Preferred DropEffect".
// Generated from rf-win's public API by a one-off stub generator, then edited by hand.
// Functions return safe defaults until they get a Linux implementation.
#![allow(unused)]
use std::path::PathBuf;
use crate::fileops::Transfer;
use crate::wide::wide;
const DROPEFFECT_COPY: u32 = 1;
const DROPEFFECT_MOVE: u32 = 2;
struct Guard;
impl Drop for Guard {
    fn drop(&mut self) {}
}
fn open(owner: crate::Opaque) -> anyhow::Result<Guard> {
    Err(anyhow::anyhow!("rf-linux: `open` is not implemented on Linux yet").into())
}
unsafe fn alloc(bytes: &[u8]) -> anyhow::Result<crate::Opaque> {
    Err(anyhow::anyhow!("rf-linux: `alloc` is not implemented on Linux yet").into())
}
/// Empties the clipboard (so a paste can't pick up what was there before).
pub fn clear(owner: crate::Opaque) -> anyhow::Result<()> {
    Err(anyhow::anyhow!("rf-linux: `clear` is not implemented on Linux yet").into())
}
/// Changes every time the clipboard's content changes.
pub fn sequence() -> u32 {
    Default::default()
}
/// Puts files on the clipboard for copy or cut. `owner` must be a real window (any thread).
pub fn set_files(
    owner: crate::Opaque,
    paths: &[PathBuf],
    kind: Transfer,
) -> anyhow::Result<()> {
    Err(anyhow::anyhow!("rf-linux: `set_files` is not implemented on Linux yet").into())
}
pub fn has_files() -> bool {
    false
}
/// Reads files + intended operation (cut → Move) from the clipboard.
pub fn get_files(
    owner: crate::Opaque,
) -> anyhow::Result<Option<(Vec<PathBuf>, Transfer)>> {
    Err(anyhow::anyhow!("rf-linux: `get_files` is not implemented on Linux yet").into())
}
