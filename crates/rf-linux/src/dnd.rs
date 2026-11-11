//! Drag files out of rfiles (to Explorer, the desktop, other apps — or back into
//! rfiles itself) with the standard shell data object and drag image.
// Generated from rf-win's public API by a one-off stub generator, then edited by hand.
// Functions return safe defaults until they get a Linux implementation.
use std::path::{Path, PathBuf};
use crate::fileops::Transfer;
/// Shell data object for items that share one parent folder.
pub fn data_object(_items: &[PathBuf]) -> anyhow::Result<crate::Opaque> {
    Err(
        anyhow::anyhow!("rf-linux: `data_object` is not implemented on Linux yet").into(),
    )
}
/// Runs a modal OLE drag on the calling thread (must be the shell STA thread — see `sta::run`).
/// `ui_thread` is the id of the thread that owns the window where the drag started; its input
/// state is attached so this thread can take mouse capture while the button is still down.
pub fn drag_out(
    _owner: crate::Opaque,
    _ui_thread: u32,
    _items: &[PathBuf],
) -> anyhow::Result<crate::Opaque> {
    Err(anyhow::anyhow!("rf-linux: `drag_out` is not implemented on Linux yet").into())
}
/// The default for a plain drag (Explorer's and Nautilus'): same file system → move,
/// otherwise copy. Ctrl forces copy, Shift forces move. "Same file system" is the same device
/// (rf-win compares drive letters; every Linux path starts at `/`).
pub fn default_transfer(sources: &[PathBuf], dest_dir: &Path, ctrl: bool, shift: bool) -> Transfer {
    if ctrl {
        return Transfer::Copy;
    }
    if shift {
        return Transfer::Move;
    }
    match sources.first() {
        Some(s) if same_device(s, dest_dir) => Transfer::Move,
        _ => Transfer::Copy,
    }
}

fn same_device(a: &Path, b: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    match (std::fs::symlink_metadata(a), std::fs::metadata(b)) {
        (Ok(x), Ok(y)) => x.dev() == y.dev(),
        _ => false,
    }
}

/// True when dropping `sources` into `dest_dir` makes no sense (onto itself or its own child).
pub fn is_invalid_drop(sources: &[PathBuf], dest_dir: &Path) -> bool {
    sources.iter().any(|s| dest_dir == s || dest_dir.starts_with(s))
        || sources.iter().all(|s| s.parent() == Some(dest_dir))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transfer_rules() {
        let tmp = std::env::temp_dir();
        let d = tmp.join(format!("rf-dnd-{}", std::process::id()));
        std::fs::create_dir_all(d.join("b")).unwrap();
        std::fs::write(d.join("x.txt"), b"x").unwrap();
        let src = vec![d.join("x.txt")];
        // same file system: move; another (/proc): copy, never move
        assert_eq!(default_transfer(&src, &d.join("b"), false, false), Transfer::Move);
        assert_eq!(default_transfer(&src, Path::new("/proc"), false, false), Transfer::Copy);
        assert_eq!(default_transfer(&src, Path::new("/proc"), false, true), Transfer::Move);
        assert_eq!(default_transfer(&src, &d.join("b"), true, false), Transfer::Copy);
        // a source that vanished meanwhile: copy (never a surprise move)
        assert_eq!(default_transfer(&[d.join("gone")], &d.join("b"), false, false), Transfer::Copy);
        assert!(is_invalid_drop(&src, &d));
        assert!(is_invalid_drop(&[d.clone()], &d.join("b")));
        assert!(!is_invalid_drop(&src, &d.join("b")));
        let _ = std::fs::remove_dir_all(&d);
    }
}
