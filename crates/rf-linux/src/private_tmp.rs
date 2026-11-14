//! rfiles' own temporary files: archive members opened from DirectLook or "Open with",
//! thumbnailer output, new files for archives, and the like.
//!
//! They live under `~/.cache/rfiles/tmp`, created readable by you only, rather than in the
//! shared `/tmp`: there other users could read them (members of password-protected archives
//! included) or create the folder or file first and own it.

use std::io;
use std::os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};

/// The root: `~/.cache/rfiles/tmp`; without a home folder, `$XDG_RUNTIME_DIR/rfiles`, then
/// `/tmp/rfiles-<uid>` (refused unless it's yours and private).
pub fn root() -> PathBuf {
    if let Some(c) = dirs::cache_dir() {
        return c.join("rfiles").join("tmp");
    }
    if let Some(r) = dirs::runtime_dir() {
        return r.join("rfiles");
    }
    std::env::temp_dir().join(format!("rfiles-{}", unsafe { libc::geteuid() }))
}

/// [`root`], created private to you.
pub fn root_dir() -> io::Result<PathBuf> {
    let root = root();
    make_private(&root)?;
    Ok(root)
}

/// Whether `path` is a folder (not a symlink) that belongs to you.
pub fn is_own_dir(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok_and(|m| m.is_dir() && m.uid() == unsafe { libc::geteuid() })
}

/// `root()/<name>`, created (with its parents) private to you. Errors when an existing folder
/// along the way, from the root down, belongs to someone else or is open to others.
pub fn dir(name: &str) -> io::Result<PathBuf> {
    let d = root_dir()?.join(name);
    make_private(&d)?;
    Ok(d)
}

fn make_private(d: &Path) -> io::Result<()> {
    match std::fs::DirBuilder::new().recursive(true).mode(0o700).create(d) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
        Err(e) => return Err(e),
    }
    // symlink_metadata: a symlink planted in place of the folder is refused, not followed
    let m = std::fs::symlink_metadata(d)?;
    if !m.is_dir() || m.uid() != unsafe { libc::geteuid() } {
        return Err(io::Error::new(io::ErrorKind::PermissionDenied, format!("{} isn't a folder of yours", d.display())));
    }
    if m.mode() & 0o077 != 0 {
        // ours but left open (an older rfiles, a umask): close it
        std::fs::set_permissions(d, std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn private_and_reused() {
        let base = std::env::temp_dir().join(format!("rf-private-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let d = base.join("a").join("b");
        make_private(&d).unwrap();
        assert_eq!(std::fs::metadata(&d).unwrap().mode() & 0o777, 0o700);
        // opened up later: closed again
        std::fs::set_permissions(&d, std::fs::Permissions::from_mode(0o755)).unwrap();
        make_private(&d).unwrap();
        assert_eq!(std::fs::metadata(&d).unwrap().mode() & 0o777, 0o700);
        // a symlink in its place is refused
        let link = base.join("link");
        std::os::unix::fs::symlink(&d, &link).unwrap();
        assert!(make_private(&link).is_err());
        let _ = std::fs::remove_dir_all(&base);
    }
}
