//! Taskbar jump list (right-click on the taskbar button): "New window" / "New tab" tasks and
//! the pinned folders, like Files. Blocking + COM: call off the UI thread.
// Generated from rf-win's public API by a one-off stub generator, then edited by hand.
// Functions return safe defaults until they get a Linux implementation.
#![allow(unused)]
use std::path::{Path, PathBuf};
use crate::shell::ensure_com;
use crate::wide::wide;
/// Command-line switch of the "New window" task.
pub const NEW_WINDOW_ARG: &str = "--new-window";
fn link(
    exe: &Path,
    args: &str,
    title: &str,
    icon: Option<(&str, i32)>,
) -> anyhow::Result<crate::Opaque> {
    Err(anyhow::anyhow!("rf-linux: `link` is not implemented on Linux yet").into())
}
/// Replaces the jump list: tasks + a "pinned" category (`(title, folder)`).
pub fn set(
    exe: &Path,
    tasks: &[(&str, &str)],
    pinned_title: &str,
    pinned: &[(String, PathBuf)],
) -> anyhow::Result<()> {
    // Linux has no jump lists: nothing to do is success
    Ok(())
}
