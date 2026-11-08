//! File operations through the Windows shell copy engine (IFileOperation): same behavior as
//! Explorer (recycle bin, undo record, conflict dialogs, elevation prompts when needed).
//! Blocking — call from a worker thread.
// Generated from rf-win's public API by a one-off stub generator, then edited by hand.
// Functions return safe defaults until they get a Linux implementation.
#![allow(unused)]
use std::path::{Path, PathBuf};
use crate::shell::ensure_com;
use crate::wide::wide;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Transfer {
    Copy,
    Move,
}
fn item(p: &Path) -> anyhow::Result<crate::Opaque> {
    Err(anyhow::anyhow!("rf-linux: `item` is not implemented on Linux yet").into())
}
fn new_op(flags: u32) -> anyhow::Result<crate::Opaque> {
    Err(anyhow::anyhow!("rf-linux: `new_op` is not implemented on Linux yet").into())
}
/// Returns Ok(true) when everything completed, Ok(false) when the user cancelled part of it.
fn perform(op: &crate::Opaque) -> anyhow::Result<bool> {
    Err(anyhow::anyhow!("rf-linux: `perform` is not implemented on Linux yet").into())
}
/// Copy or move into `dest_dir`. Pasting into the same folder makes "x - Kopya" names.
pub fn transfer(
    kind: Transfer,
    sources: &[PathBuf],
    dest_dir: &Path,
) -> anyhow::Result<bool> {
    Err(anyhow::anyhow!("rf-linux: `transfer` is not implemented on Linux yet").into())
}
/// Delete to the recycle bin (`permanent = false`) or permanently (Windows asks for confirmation).
pub fn delete(paths: &[PathBuf], permanent: bool) -> anyhow::Result<bool> {
    Err(anyhow::anyhow!("rf-linux: `delete` is not implemented on Linux yet").into())
}
pub fn rename(path: &Path, new_name: &str) -> anyhow::Result<bool> {
    Err(anyhow::anyhow!("rf-linux: `rename` is not implemented on Linux yet").into())
}
/// Plain rename that fails instead of silently replacing an existing target
/// (`std::fs::rename` replaces on Windows).
pub fn rename_no_replace(from: &Path, to: &Path) -> std::io::Result<()> {
    Err(
        std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "rf-linux: `rename_no_replace` is not implemented on Linux yet",
        ),
    )
}
/// "Yeni klasör", "Yeni klasör (2)", … — returns the created path.
pub fn new_folder(dir: &Path) -> anyhow::Result<PathBuf> {
    let p = unique_name(dir, "Yeni klasör", "");
    std::fs::create_dir(&p)?;
    Ok(p)
}
pub fn new_text_file(dir: &Path) -> anyhow::Result<PathBuf> {
    let p = unique_name(dir, "Yeni Metin Belgesi", ".txt");
    std::fs::OpenOptions::new().write(true).create_new(true).open(&p)?;
    Ok(p)
}
pub fn unique_name(dir: &Path, stem: &str, ext: &str) -> PathBuf {
    let first = dir.join(format!("{stem}{ext}"));
    if !first.exists() {
        return first;
    }
    (2..).map(|i| dir.join(format!("{stem} ({i}){ext}"))).find(|p| !p.exists()).unwrap()
}
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
/// Progress reporting for one operation. `on_progress(total, done, current_item)`;
/// `total`/`done` are the shell's abstract work units (use done/total as a fraction).
pub struct Tracker {
    pub on_progress: Box<dyn Fn(u32, u32, Option<String>) + Send + Sync>,
    pub cancel: Arc<AtomicBool>,
    /// (source, result) for every item the shell finished: the new copy/moved location, or the
    /// item's path inside the Recycle Bin for deletes. Used to build undo records.
    pub results: std::sync::Mutex<Vec<(PathBuf, PathBuf)>>,
    /// Our engine waits while this is set (the shell engine can't pause).
    pub paused: Arc<AtomicBool>,
    /// Copy speed limit in bytes per second; 0 = none (our engine only).
    pub limit: std::sync::atomic::AtomicU64,
    /// Check copies after writing them (our engine only); see [`Tracker::set_verify`].
    pub verify: std::sync::atomic::AtomicU8,
    /// Asked (on the worker thread, blocking) what to do with names that already exist at the
    /// destination; `None` cancels the operation. Without it, both are kept.
    #[allow(clippy::type_complexity)]
    pub ask: Option<
        Box<
            dyn Fn(
                &[crate::fastcopy::Conflict],
            ) -> Option<Vec<crate::fastcopy::Resolve>> + Send + Sync,
        >,
    >,
}
impl Tracker {
    pub fn set_verify(&self, v: crate::fastcopy::Verify) {
        self.verify.store(v as u8, Ordering::Relaxed);
    }
    pub fn verify_mode(&self) -> crate::fastcopy::Verify {
        match self.verify.load(Ordering::Relaxed) {
            1 => crate::fastcopy::Verify::Quick,
            2 => crate::fastcopy::Verify::Full,
            _ => crate::fastcopy::Verify::Off,
        }
    }
    pub fn new(
        on_progress: Box<dyn Fn(u32, u32, Option<String>) + Send + Sync>,
        cancel: Arc<AtomicBool>,
    ) -> Self {
        Self {
            on_progress,
            cancel,
            results: Default::default(),
            paused: Default::default(),
            limit: Default::default(),
            verify: Default::default(),
            ask: None,
        }
    }
}
fn fs_path(item: crate::Opaque) -> Option<PathBuf> {
    None
}
fn perform_tracked(op: &crate::Opaque, tracker: Arc<Tracker>) -> anyhow::Result<bool> {
    Err(
        anyhow::anyhow!("rf-linux: `perform_tracked` is not implemented on Linux yet")
            .into(),
    )
}
/// `transfer` with in-app progress (no shell progress window).
pub fn transfer_tracked(
    kind: Transfer,
    sources: &[PathBuf],
    dest_dir: &Path,
    tracker: Arc<Tracker>,
) -> anyhow::Result<bool> {
    Err(
        anyhow::anyhow!("rf-linux: `transfer_tracked` is not implemented on Linux yet")
            .into(),
    )
}
/// `delete` with in-app progress.
pub fn delete_tracked(
    paths: &[PathBuf],
    permanent: bool,
    tracker: Arc<Tracker>,
) -> anyhow::Result<bool> {
    Err(
        anyhow::anyhow!("rf-linux: `delete_tracked` is not implemented on Linux yet")
            .into(),
    )
}
