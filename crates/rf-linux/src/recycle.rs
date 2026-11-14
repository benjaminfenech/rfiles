//! The Trash, per the freedesktop.org Trash specification (what GNOME, KDE, Xfce and
//! `gio trash` use), so items trashed here show up in other file managers and back.
//!
//! * Files on the home file system go to `$XDG_DATA_HOME/Trash` (`~/.local/share/Trash`).
//! * Files on another file system go to that file system's own trash: `$topdir/.Trash/$uid`
//!   when an admin set up a shared `.Trash` (sticky, not a link), else `$topdir/.Trash-$uid`.
//!   Nothing is ever copied across file systems.
//! * Each item is `files/NAME` plus `info/NAME.trashinfo` (original path, deletion time).
//!
//! Blocking (disk): call off the UI thread.

use std::io::Write;
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

/// One item in the Trash.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecycleItem {
    /// Original file name.
    pub name: String,
    /// Folder the item was deleted from.
    pub original_location: String,
    /// Deletion time, Unix milliseconds (UTC); 0 when unknown.
    pub deleted_ms: i64,
    pub size: u64,
    /// The item inside the trash (`…/Trash/files/NAME`); also the key used by [`restore`].
    pub parsing_path: String,
    pub is_dir: bool,
}

/// A trash folder, and the folder its relative `Path=` entries start from.
struct TrashDir {
    root: PathBuf,
    /// `None` for the home trash (absolute paths); the mount point for a per-disk trash.
    topdir: Option<PathBuf>,
}

impl TrashDir {
    fn files(&self) -> PathBuf {
        self.root.join("files")
    }
    fn info(&self) -> PathBuf {
        self.root.join("info")
    }
}

fn uid() -> u32 {
    // SAFETY: no arguments, always succeeds
    unsafe { libc::getuid() }
}

fn home_trash() -> Option<PathBuf> {
    dirs::data_dir().map(|d| d.join("Trash"))
}

/// Creates `dir` (and parents) with owner-only access when missing.
fn ensure_dir(dir: &Path) -> std::io::Result<()> {
    if dir.is_dir() {
        return Ok(());
    }
    std::fs::DirBuilder::new().recursive(true).mode(0o700).create(dir)
}

fn ensure(t: &TrashDir) -> std::io::Result<()> {
    ensure_dir(&t.files())?;
    ensure_dir(&t.info())
}

/// Device of `p`, or of its nearest existing ancestor when `p` doesn't exist yet.
fn nearest_dev(p: &Path) -> std::io::Result<u64> {
    let mut last = None;
    for a in p.ancestors() {
        match std::fs::metadata(a) {
            Ok(m) => return Ok(m.dev()),
            Err(e) => last = Some(e),
        }
    }
    Err(last.unwrap_or_else(|| std::io::Error::from(std::io::ErrorKind::NotFound)))
}

/// The trash folder for `path`: the home trash when they share a file system, else the trash
/// at the top of `path`'s own mount.
fn trash_for(path: &Path) -> Result<TrashDir> {
    let home = home_trash().context("no home folder")?;
    // the home trash's file system; on a new account the trash (even ~/.local/share) may not
    // exist yet: it will be created on the file system of its nearest existing folder
    let home_dev = nearest_dev(&home)?;
    let item_dev = std::fs::symlink_metadata(path)?.dev();
    if item_dev == home_dev {
        return Ok(TrashDir { root: home, topdir: None });
    }
    let (top, _, _) = crate::drives::mount_of(path).context("no mount found for the item")?;
    let uid = uid();
    // an admin-made shared .Trash: a real folder with the sticky bit
    let shared = top.join(".Trash");
    if let Ok(m) = std::fs::symlink_metadata(&shared) {
        if m.is_dir() && m.permissions().mode() & 0o1000 != 0 {
            let t = TrashDir { root: shared.join(uid.to_string()), topdir: Some(top.clone()) };
            if ensure(&t).is_ok() {
                return Ok(t);
            }
        }
    }
    let t = TrashDir { root: top.join(format!(".Trash-{uid}")), topdir: Some(top) };
    ensure(&t).with_context(|| format!("no trash on this drive ({})", t.root.display()))?;
    Ok(t)
}

/// Every trash folder that exists: home, then one per mounted file system.
fn all_trash_dirs() -> Vec<TrashDir> {
    let mut out = Vec::new();
    if let Some(h) = home_trash() {
        out.push(TrashDir { root: h, topdir: None });
    }
    let uid = uid();
    let mut seen = std::collections::HashSet::new();
    let mounts = crate::drives::mounts();
    for d in crate::drives::list_drives() {
        // an fstab share not mounted yet has no trash to show: looking would mount it (and
        // wait out the timeout when its server is away)
        if !seen.insert(d.root.clone()) || !crate::drives::really_mounted(&d.root, &mounts) {
            continue;
        }
        for root in [d.root.join(".Trash").join(uid.to_string()), d.root.join(format!(".Trash-{uid}"))] {
            if root.join("info").is_dir() {
                out.push(TrashDir { root, topdir: Some(d.root.clone()) });
            }
        }
    }
    out
}

/// Percent-encodes a path for `Path=` (RFC 2396 escaping, `/` kept).
fn encode(p: &Path) -> String {
    use std::os::unix::ffi::OsStrExt;
    rf_core::uri::encode(p.as_os_str().as_bytes(), b"/-_.~!*'()")
}

fn decode(s: &str) -> PathBuf {
    rf_core::uri::decode_path(s)
}

/// Reads a `.trashinfo`: (original path, deletion time in Unix ms).
fn read_info(file: &Path, t: &TrashDir) -> Option<(PathBuf, i64)> {
    let text = std::fs::read_to_string(file).ok()?;
    let mut path = None;
    let mut when = 0;
    for line in text.lines() {
        if let Some(v) = line.strip_prefix("Path=") {
            let p = decode(v.trim());
            path = Some(match (&t.topdir, p.is_absolute()) {
                (Some(top), false) => top.join(p),
                _ => p,
            });
        } else if let Some(v) = line.strip_prefix("DeletionDate=") {
            use chrono::TimeZone;
            when = chrono::NaiveDateTime::parse_from_str(v.trim(), "%Y-%m-%dT%H:%M:%S")
                .ok()
                .and_then(|n| chrono::Local.from_local_datetime(&n).earliest())
                .map_or(0, |d| d.timestamp_millis());
        }
    }
    path.map(|p| (p, when))
}

/// Moves `path` to the Trash and returns where it is now (`…/Trash/files/NAME`).
pub fn trash(path: &Path) -> Result<PathBuf> {
    let path = std::path::absolute(path)?;
    let name = path.file_name().context("can't trash a root")?.to_string_lossy().into_owned();
    let t = trash_for(&path)?;
    ensure(&t)?;
    let recorded = match &t.topdir {
        Some(top) => path.strip_prefix(top).map(Path::to_path_buf).unwrap_or_else(|_| path.clone()),
        None => path.clone(),
    };
    let body = format!(
        "[Trash Info]\nPath={}\nDeletionDate={}\n",
        encode(&recorded),
        chrono::Local::now().format("%Y-%m-%dT%H:%M:%S")
    );
    // NAME, NAME.2, NAME.3…: the info file is created exclusively, so two trashers can't
    // pick the same one
    for n in 1u32.. {
        let stem = if n == 1 { name.clone() } else { format!("{name}.{n}") };
        let info = t.info().join(format!("{stem}.trashinfo"));
        let dest = t.files().join(&stem);
        if std::fs::symlink_metadata(&dest).is_ok() {
            continue;
        }
        let mut f = match std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(&info) {
            Ok(f) => f,
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e.into()),
        };
        f.write_all(body.as_bytes())?;
        drop(f);
        return match std::fs::rename(&path, &dest) {
            Ok(()) => Ok(dest),
            Err(e) => {
                let _ = std::fs::remove_file(&info);
                Err(e).with_context(|| format!("{}", path.display()))
            }
        };
    }
    unreachable!()
}

/// Size of a trashed item, symlinks not followed.
fn size_of(p: &Path) -> u64 {
    let Ok(m) = std::fs::symlink_metadata(p) else { return 0 };
    if !m.is_dir() {
        return m.len();
    }
    let mut total = 0;
    let mut stack = vec![p.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).into_iter().flatten().flatten() {
            match e.metadata() {
                Ok(m) if m.is_dir() => stack.push(e.path()),
                Ok(m) => total += m.len(),
                Err(_) => {}
            }
        }
    }
    total
}

/// Everything in every trash, newest first.
pub fn list() -> anyhow::Result<Vec<RecycleItem>> {
    let mut out = Vec::new();
    for t in all_trash_dirs() {
        let Ok(rd) = std::fs::read_dir(t.info()) else { continue };
        for e in rd.flatten() {
            let info = e.path();
            let Some(stem) = info.file_name().and_then(|n| n.to_str()).and_then(|n| n.strip_suffix(".trashinfo")) else {
                continue;
            };
            let item = t.files().join(stem);
            let Ok(m) = std::fs::symlink_metadata(&item) else { continue };
            let Some((orig, deleted_ms)) = read_info(&info, &t) else { continue };
            out.push(RecycleItem {
                name: orig.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| stem.to_string()),
                original_location: orig.parent().map(|p| p.display().to_string()).unwrap_or_default(),
                deleted_ms,
                size: size_of(&item),
                parsing_path: item.to_string_lossy().into_owned(),
                is_dir: m.is_dir(),
            });
        }
    }
    out.sort_by_key(|i| std::cmp::Reverse(i.deleted_ms));
    Ok(out)
}

/// Puts `items` back where they came from. An item whose original name is taken again stays
/// in the Trash and is reported; errors if none could be restored.
pub fn restore(items: &[RecycleItem]) -> anyhow::Result<()> {
    if items.is_empty() {
        return Ok(());
    }
    let dirs = all_trash_dirs();
    let mut restored = 0;
    let mut taken = Vec::new();
    for it in items {
        let item = PathBuf::from(&it.parsing_path);
        let Some(t) = dirs.iter().find(|t| item.parent() == Some(t.files().as_path())) else { continue };
        let Some(stem) = item.file_name() else { continue };
        let info = t.info().join(format!("{}.trashinfo", stem.to_string_lossy()));
        let Some((orig, _)) = read_info(&info, t) else { continue };
        if std::fs::symlink_metadata(&orig).is_ok() {
            taken.push(orig);
            continue;
        }
        if let Some(parent) = orig.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        crate::fileops::rename_no_replace(&item, &orig).with_context(|| orig.display().to_string())?;
        let _ = std::fs::remove_file(&info);
        restored += 1;
    }
    if !taken.is_empty() {
        let names: Vec<String> = taken.iter().filter_map(|p| p.file_name()).map(|n| n.to_string_lossy().into_owned()).collect();
        anyhow::bail!("An item with this name already exists: {}", names.join(", "));
    }
    anyhow::ensure!(restored > 0, "Items were not found in the Trash");
    Ok(())
}

/// Removes something from a trash for good (symlinks as links).
fn remove(p: &Path) -> std::io::Result<()> {
    match std::fs::symlink_metadata(p) {
        Ok(m) if m.is_dir() => std::fs::remove_dir_all(p),
        Ok(_) => std::fs::remove_file(p),
        Err(e) => Err(e),
    }
}

/// Empties every trash. `confirm` is for the Windows build (its shell can ask); the app asks
/// before calling this.
pub fn empty(confirm: bool) -> anyhow::Result<()> {
    let _ = confirm;
    let mut failed = 0;
    for t in all_trash_dirs() {
        for dir in [t.files(), t.info()] {
            for e in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
                if remove(&e.path()).is_err() {
                    failed += 1;
                }
            }
        }
        // the size cache some file managers keep
        let _ = std::fs::remove_file(t.root.join("directorysizes"));
    }
    anyhow::ensure!(failed == 0, "{failed} items could not be removed from the Trash");
    Ok(())
}

/// `(item_count, total_bytes)` across all trashes.
pub fn info() -> anyhow::Result<(u64, u64)> {
    let items = list()?;
    Ok((items.len() as u64, items.iter().map(|i| i.size).sum()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_trash_folders_use_their_nearest_parent() {
        // a fresh account: ~/.local/share/Trash and even ~/.local/share don't exist yet
        let tmp = std::env::temp_dir();
        let missing = tmp.join(format!("rf-no-such-{}", std::process::id())).join("share/Trash");
        assert_eq!(nearest_dev(&missing).unwrap(), std::fs::metadata(&tmp).unwrap().dev());
    }

    #[test]
    fn info_files_round_trip() {
        let p = Path::new("/home/me/Belgeler/rapor %1 ü.txt");
        assert_eq!(decode(&encode(p)), p);
        assert_eq!(encode(Path::new("/a b")), "/a%20b");
    }

    /// Trashes a real file in the home trash and restores it. Uses the user's own trash (that
    /// is the point), with a unique name it cleans up.
    #[test]
    fn trash_list_restore() {
        let Some(home) = dirs::home_dir() else { return };
        // on the home file system, so the home trash is used
        let dir = home.join(format!(".cache/rf-trash-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("çöpe gidecek.txt");
        std::fs::write(&f, b"geri gel").unwrap();
        let in_trash = trash(&f).unwrap();
        assert!(!f.exists() && in_trash.exists());
        let found: Vec<RecycleItem> = list().unwrap().into_iter().filter(|i| i.parsing_path == in_trash.to_string_lossy()).collect();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].name, "çöpe gidecek.txt");
        assert_eq!(found[0].original_location, dir.display().to_string());
        assert_eq!(found[0].size, 8);
        assert!(found[0].deleted_ms > 0);
        // a new file with the same name blocks the restore; removing it lets it through
        std::fs::write(&f, b"yeni").unwrap();
        assert!(restore(&found).is_err());
        std::fs::remove_file(&f).unwrap();
        restore(&found).unwrap();
        assert_eq!(std::fs::read(&f).unwrap(), b"geri gel");
        assert!(!in_trash.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
