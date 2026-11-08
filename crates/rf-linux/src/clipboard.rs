//! The file clipboard (copy / cut / paste of files).
//!
//! Linux, for now: held inside rustyfiles, so copy and paste work between its windows and
//! tabs but not with other apps. Exchanging files with other file managers needs
//! `text/uri-list` plus `x-special/gnome-copied-files` on the Wayland / X11 clipboard, which
//! GPUI can't write yet; that is a follow-up (docs/LINUX_PORT.md, Phase 1).

use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU32, Ordering};

use crate::fileops::Transfer;

static FILES: Mutex<Option<(Vec<PathBuf>, Transfer)>> = Mutex::new(None);
static SEQUENCE: AtomicU32 = AtomicU32::new(1);

/// Empties the clipboard (so a paste can't pick up what was there before).
pub fn clear(owner: crate::Opaque) -> anyhow::Result<()> {
    let _ = owner;
    *FILES.lock().unwrap() = None;
    SEQUENCE.fetch_add(1, Ordering::Relaxed);
    Ok(())
}

/// Changes every time the clipboard's content changes.
pub fn sequence() -> u32 {
    SEQUENCE.load(Ordering::Relaxed)
}

/// Puts files on the clipboard for copy (`Copy`) or cut (`Move`).
pub fn set_files(owner: crate::Opaque, paths: &[PathBuf], kind: Transfer) -> anyhow::Result<()> {
    let _ = owner;
    *FILES.lock().unwrap() = (!paths.is_empty()).then(|| (paths.to_vec(), kind));
    SEQUENCE.fetch_add(1, Ordering::Relaxed);
    Ok(())
}

pub fn has_files() -> bool {
    FILES.lock().unwrap().is_some()
}

/// Files + intended operation (cut → Move).
pub fn get_files(owner: crate::Opaque) -> anyhow::Result<Option<(Vec<PathBuf>, Transfer)>> {
    let _ = owner;
    Ok(FILES.lock().unwrap().clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_get_clear() {
        let before = sequence();
        set_files(0, &[PathBuf::from("/tmp/a")], Transfer::Move).unwrap();
        assert!(has_files() && sequence() != before);
        assert_eq!(get_files(0).unwrap(), Some((vec![PathBuf::from("/tmp/a")], Transfer::Move)));
        clear(0).unwrap();
        assert!(!has_files());
        assert_eq!(get_files(0).unwrap(), None);
    }
}
