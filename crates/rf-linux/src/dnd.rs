//! Drag files out of rustyfiles (to Explorer, the desktop, other apps — or back into
//! rustyfiles itself) with the standard shell data object and drag image.
// Generated from rf-win's public API by a one-off stub generator, then edited by hand.
// Functions return safe defaults until they get a Linux implementation.
#![allow(unused)]
use std::path::{Path, PathBuf};
use crate::fileops::Transfer;
use crate::wide::wide;
struct Pidl(*mut crate::Opaque);
impl Drop for Pidl {
    fn drop(&mut self) {}
}
fn parse(path: &Path) -> anyhow::Result<Pidl> {
    Err(anyhow::anyhow!("rf-linux: `parse` is not implemented on Linux yet").into())
}
/// Shell data object for items that share one parent folder.
pub fn data_object(items: &[PathBuf]) -> anyhow::Result<crate::Opaque> {
    Err(
        anyhow::anyhow!("rf-linux: `data_object` is not implemented on Linux yet").into(),
    )
}
/// Runs a modal OLE drag on the calling thread (must be the shell STA thread — see `sta::run`).
/// `ui_thread` is the id of the thread that owns the window where the drag started; its input
/// state is attached so this thread can take mouse capture while the button is still down.
pub fn drag_out(
    owner: crate::Opaque,
    ui_thread: u32,
    items: &[PathBuf],
) -> anyhow::Result<crate::Opaque> {
    Err(anyhow::anyhow!("rf-linux: `drag_out` is not implemented on Linux yet").into())
}
/// Explorer's default for a plain drag: same volume → move, otherwise copy.
/// Ctrl forces copy, Shift forces move.
pub fn default_transfer(
    sources: &[PathBuf],
    dest_dir: &Path,
    ctrl: bool,
    shift: bool,
) -> Transfer {
    if ctrl {
        return Transfer::Copy;
    }
    if shift {
        return Transfer::Move;
    }
    let vol = |p: &Path| {
        p.components().next().map(|c| c.as_os_str().to_string_lossy().to_lowercase())
    };
    match sources.first() {
        Some(s) if vol(s) == vol(dest_dir) => Transfer::Move,
        _ => Transfer::Copy,
    }
}
/// True when dropping `sources` into `dest_dir` makes no sense (onto itself or its own child).
pub fn is_invalid_drop(sources: &[PathBuf], dest_dir: &Path) -> bool {
    sources.iter().any(|s| dest_dir == s || dest_dir.starts_with(s))
        || sources.iter().all(|s| s.parent() == Some(dest_dir))
}
