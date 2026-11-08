//! File operations for Linux. rf-win routes these through the Windows shell's copy engine;
//! here they are a small engine of our own, and the place `fastcopy` hands what it leaves
//! out (items containing symlinks, pasting into the same folder, files that failed there),
//! and what runs when the fast engine is turned off in Settings.
//!
//! * Copies keep permissions and times; symlinks are copied as links (never followed).
//! * Moves are renames on one file system, else a copy and then removal of the source.
//! * Names that exist at the destination are put to `Tracker::ask` (without it, both kept).
//! * Delete goes to the freedesktop Trash ([`crate::recycle`]) or, permanently, through
//!   [`crate::fastdelete`].
//!
//! Blocking: call from a worker thread.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::bail;

use crate::fastcopy::{Conflict, Resolve};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Transfer {
    Copy,
    Move,
}

/// Copy or move into `dest_dir`. Pasting into the same folder makes "x (2)" names.
pub fn transfer(kind: Transfer, sources: &[PathBuf], dest_dir: &Path) -> anyhow::Result<bool> {
    transfer_tracked(kind, sources, dest_dir, Arc::new(Tracker::new(Box::new(|_, _, _| {}), Default::default())))
}

/// Delete to the Trash (`permanent = false`) or permanently.
pub fn delete(paths: &[PathBuf], permanent: bool) -> anyhow::Result<bool> {
    delete_tracked(paths, permanent, Arc::new(Tracker::new(Box::new(|_, _, _| {}), Default::default())))
}

/// Renames `path` to `new_name` in its folder; fails when the name is taken.
pub fn rename(path: &Path, new_name: &str) -> anyhow::Result<bool> {
    anyhow::ensure!(!new_name.is_empty() && !new_name.contains('/') && new_name != "." && new_name != "..", "invalid name: {new_name}");
    let to = path.with_file_name(new_name);
    if to == path {
        return Ok(true);
    }
    match rename_no_replace(path, &to) {
        Ok(()) => Ok(true),
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => bail!("Bu adla bir öğe zaten var"),
        Err(e) => Err(e.into()),
    }
}

/// Rename that fails instead of replacing an existing target (`std::fs::rename` replaces on
/// Linux): `renameat2(RENAME_NOREPLACE)`, or a check first where the file system lacks it.
pub fn rename_no_replace(from: &Path, to: &Path) -> std::io::Result<()> {
    use std::os::unix::ffi::OsStrExt;
    let c = |p: &Path| std::ffi::CString::new(p.as_os_str().as_bytes()).map_err(|_| std::io::Error::from(std::io::ErrorKind::InvalidInput));
    let (a, b) = (c(from)?, c(to)?);
    // SAFETY: valid C strings; AT_FDCWD resolves them like open() would
    let r = unsafe { libc::renameat2(libc::AT_FDCWD, a.as_ptr(), libc::AT_FDCWD, b.as_ptr(), libc::RENAME_NOREPLACE) };
    if r == 0 {
        return Ok(());
    }
    let e = std::io::Error::last_os_error();
    match e.raw_os_error() {
        // not supported by this file system (some FUSE / network ones): check, then rename
        Some(libc::EINVAL | libc::ENOSYS | libc::EOPNOTSUPP) => {
            if std::fs::symlink_metadata(to).is_ok() {
                return Err(std::io::Error::from(std::io::ErrorKind::AlreadyExists));
            }
            std::fs::rename(from, to)
        }
        _ => Err(e),
    }
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
    if std::fs::symlink_metadata(&first).is_err() {
        return first;
    }
    (2..).map(|i| dir.join(format!("{stem} ({i}){ext}"))).find(|p| std::fs::symlink_metadata(p).is_err()).unwrap()
}

// ---------------------------------------------------------------------------------------------
// Tracked operations (in-app Status Center): progress callback + cancellation. Same Tracker
// as rf-win.
// ---------------------------------------------------------------------------------------------

/// Progress reporting for one operation. `on_progress(total, done, current_item)`;
/// `total`/`done` are abstract work units (use done/total as a fraction).
pub struct Tracker {
    pub on_progress: Box<dyn Fn(u32, u32, Option<String>) + Send + Sync>,
    pub cancel: Arc<AtomicBool>,
    /// (source, result) for every item finished: the new copy/moved location, or the item's
    /// path inside the Trash for deletes. Used to build undo records.
    pub results: std::sync::Mutex<Vec<(PathBuf, PathBuf)>>,
    /// The engines wait while this is set.
    pub paused: Arc<AtomicBool>,
    /// Copy speed limit in bytes per second; 0 = none (fast engine only).
    pub limit: std::sync::atomic::AtomicU64,
    /// Check copies after writing them (fast engine only); see [`Tracker::set_verify`].
    pub verify: std::sync::atomic::AtomicU8,
    /// Asked (on the worker thread, blocking) what to do with names that already exist at the
    /// destination; `None` cancels the operation. Without it, both are kept.
    #[allow(clippy::type_complexity)]
    pub ask: Option<Box<dyn Fn(&[crate::fastcopy::Conflict]) -> Option<Vec<crate::fastcopy::Resolve>> + Send + Sync>>,
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

    pub fn new(on_progress: Box<dyn Fn(u32, u32, Option<String>) + Send + Sync>, cancel: Arc<AtomicBool>) -> Self {
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

    fn cancelled(&self) -> bool {
        while self.paused.load(Ordering::Relaxed) && !self.cancel.load(Ordering::Relaxed) {
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        self.cancel.load(Ordering::Relaxed)
    }
}

/// Byte progress of one fallback operation, reported as permille.
struct Progress<'a> {
    tracker: &'a Tracker,
    total: u64,
    done: u64,
}

impl Progress<'_> {
    fn add(&mut self, bytes: u64, name: &Path) {
        self.done += bytes;
        let permille = (self.done.min(self.total) as u128 * 1000 / self.total.max(1) as u128) as u32;
        (self.tracker.on_progress)(1000, permille, name.file_name().map(|n| n.to_string_lossy().into_owned()));
    }
}

/// Bytes under `p` (at least 1 per item, so empty files and folders move the bar).
fn weight(p: &Path) -> u64 {
    let Ok(m) = std::fs::symlink_metadata(p) else { return 1 };
    if !m.is_dir() {
        return m.len().max(1);
    }
    let mut total = 1;
    for e in std::fs::read_dir(p).into_iter().flatten().flatten() {
        total += weight(&e.path());
    }
    total
}

/// "name (2).ext", "name (3).ext", … : the first name free in `dir`.
fn free_name(dst: &Path, is_dir: bool) -> PathBuf {
    let dir = dst.parent().unwrap_or(Path::new(""));
    let name = dst.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let (stem, ext) = match name.rfind('.') {
        Some(i) if i > 0 && !is_dir => (name[..i].to_string(), name[i..].to_string()),
        _ => (name.clone(), String::new()),
    };
    (2..).map(|n| dir.join(format!("{stem} ({n}){ext}"))).find(|p| std::fs::symlink_metadata(p).is_err()).expect("a free name")
}

/// Copies `src` to the new path `dst`: files with their permissions and times, folders
/// recursively, symlinks as symlinks. `Ok(false)` when cancelled (the partial item is removed).
fn copy_item(src: &Path, dst: &Path, p: &mut Progress) -> std::io::Result<bool> {
    if p.tracker.cancelled() {
        return Ok(false);
    }
    let m = std::fs::symlink_metadata(src)?;
    let ft = m.file_type();
    if ft.is_symlink() {
        std::os::unix::fs::symlink(std::fs::read_link(src)?, dst)?;
        p.add(1, src);
    } else if ft.is_dir() {
        std::fs::create_dir(dst)?;
        for e in std::fs::read_dir(src)? {
            let e = e?;
            if !copy_item(&e.path(), &dst.join(e.file_name()), p)? {
                return Ok(false);
            }
        }
        let _ = std::fs::set_permissions(dst, m.permissions());
        if let (Ok(mt), Ok(at), Ok(d)) = (m.modified(), m.accessed(), std::fs::File::open(dst)) {
            let _ = d.set_times(std::fs::FileTimes::new().set_modified(mt).set_accessed(at));
        }
        p.add(1, src);
    } else {
        if std::fs::symlink_metadata(dst).is_ok() {
            return Err(std::io::Error::from(std::io::ErrorKind::AlreadyExists));
        }
        // std::fs::copy keeps the permissions and uses copy_file_range where it can
        if let Err(e) = std::fs::copy(src, dst) {
            let _ = std::fs::remove_file(dst);
            return Err(e);
        }
        if let (Ok(mt), Ok(at), Ok(f)) = (m.modified(), m.accessed(), std::fs::OpenOptions::new().write(true).open(dst)) {
            let _ = f.set_times(std::fs::FileTimes::new().set_modified(mt).set_accessed(at));
        }
        p.add(m.len().max(1), src);
    }
    Ok(true)
}

/// Removes `p` for good, symlinks as links.
fn remove_item(p: &Path) -> std::io::Result<()> {
    match std::fs::symlink_metadata(p) {
        Ok(m) if m.is_dir() => std::fs::remove_dir_all(p),
        Ok(_) => std::fs::remove_file(p),
        Err(e) => Err(e),
    }
}

/// `transfer` with in-app progress. Undo pairs land in `tracker.results`.
pub fn transfer_tracked(kind: Transfer, sources: &[PathBuf], dest_dir: &Path, tracker: Arc<Tracker>) -> anyhow::Result<bool> {
    let same_dir = sources.iter().all(|s| s.parent() == Some(dest_dir));
    if kind == Transfer::Move && same_dir {
        return Ok(true); // moving onto itself: no-op
    }
    // where each source goes, and which names clash
    let mut plan: Vec<(PathBuf, PathBuf)> = Vec::new();
    let mut clashes: Vec<(usize, Conflict)> = Vec::new();
    for s in sources {
        let Some(name) = s.file_name() else { bail!("{}: not a file or folder", s.display()) };
        let meta = std::fs::symlink_metadata(s)?;
        if meta.is_dir() && dest_dir.starts_with(s) {
            bail!("A folder can't be copied or moved into itself: {}", s.display());
        }
        let mut dst = dest_dir.join(name);
        if s.parent() == Some(dest_dir) {
            // a copy next to the original
            dst = free_name(&dst, meta.is_dir());
        } else if let Ok(d) = std::fs::symlink_metadata(&dst) {
            clashes.push((
                plan.len(),
                Conflict {
                    src: s.clone(),
                    dst: dst.clone(),
                    src_dir: meta.is_dir(),
                    dst_dir: d.is_dir(),
                    src_size: if meta.is_dir() { 0 } else { meta.len() },
                    dst_size: if d.is_dir() { 0 } else { d.len() },
                    src_time: meta.modified().ok(),
                    dst_time: d.modified().ok(),
                },
            ));
        }
        plan.push((s.clone(), dst));
    }
    // one question for all clashes
    let mut replace = vec![false; plan.len()];
    let mut skip = vec![false; plan.len()];
    if !clashes.is_empty() {
        let list: Vec<Conflict> = clashes.iter().map(|(_, c)| c.clone()).collect();
        let answers = match &tracker.ask {
            Some(ask) => ask(&list),
            None => Some(vec![Resolve::KeepBoth; list.len()]),
        };
        let Some(answers) = answers else { return Ok(false) };
        for ((i, c), r) in clashes.iter().zip(answers) {
            match r {
                Resolve::Skip => skip[*i] = true,
                // a file can't replace a folder (or the other way round): keep both instead
                Resolve::Replace if !c.mismatched() => replace[*i] = true,
                _ => plan[*i].1 = free_name(&plan[*i].1, c.src_dir),
            }
        }
    }
    let mut p = Progress { total: plan.iter().enumerate().filter(|(i, _)| !skip[*i]).map(|(_, (s, _))| weight(s)).sum::<u64>().max(1), done: 0, tracker: &tracker };
    for (i, (src, dst)) in plan.iter().enumerate() {
        if skip[i] {
            continue;
        }
        if tracker.cancelled() {
            return Ok(false);
        }
        if replace[i] {
            remove_item(dst)?;
        }
        if kind == Transfer::Move {
            match rename_no_replace(src, dst) {
                Ok(()) => {
                    p.add(weight(dst), src);
                    tracker.results.lock().unwrap().push((src.clone(), dst.clone()));
                    continue;
                }
                // another file system: copy, then remove the source
                Err(e) if e.raw_os_error() == Some(libc::EXDEV) => {}
                Err(e) => return Err(anyhow::Error::from(e).context(src.display().to_string())),
            }
        }
        match copy_item(src, dst, &mut p) {
            Ok(true) => {}
            Ok(false) => {
                let _ = remove_item(dst);
                return Ok(false);
            }
            Err(e) => {
                let _ = remove_item(dst);
                return Err(anyhow::Error::from(e).context(src.display().to_string()));
            }
        }
        if kind == Transfer::Move {
            remove_item(src).map_err(|e| anyhow::Error::from(e).context(format!("copied, but the original stays: {}", src.display())))?;
        }
        tracker.results.lock().unwrap().push((src.clone(), dst.clone()));
    }
    (tracker.on_progress)(1000, 1000, None);
    Ok(true)
}

/// `delete` with in-app progress. Trashed items' new paths land in `tracker.results`.
pub fn delete_tracked(paths: &[PathBuf], permanent: bool, tracker: Arc<Tracker>) -> anyhow::Result<bool> {
    if permanent {
        let threads = crate::fastdelete::threads_for(paths);
        return crate::fastdelete::delete_permanent(paths, tracker, threads);
    }
    let mut failed = Vec::new();
    for (i, p) in paths.iter().enumerate() {
        if tracker.cancelled() {
            return Ok(false);
        }
        (tracker.on_progress)(1000, (i * 1000 / paths.len().max(1)) as u32, p.file_name().map(|n| n.to_string_lossy().into_owned()));
        match crate::recycle::trash(p) {
            Ok(to) => tracker.results.lock().unwrap().push((p.clone(), to)),
            Err(e) => {
                log::warn!("trash {}: {e:#}", p.display());
                failed.push((p.clone(), e));
            }
        }
    }
    (tracker.on_progress)(1000, 1000, None);
    if let Some((p, e)) = failed.first() {
        bail!("Couldn't move {} item(s) to the Trash ({}: {e:#})", failed.len(), p.display());
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base(name: &str) -> PathBuf {
        let b = std::env::temp_dir().join(format!("rf-ops-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&b);
        std::fs::create_dir_all(&b).unwrap();
        b
    }

    fn tracker() -> Arc<Tracker> {
        Arc::new(Tracker::new(Box::new(|_, _, _| {}), Arc::new(AtomicBool::new(false))))
    }

    #[test]
    fn unique_rename_and_same_folder_copy() {
        let d = base("unique");
        let a = new_folder(&d).unwrap();
        let b = new_folder(&d).unwrap();
        assert!(a.ends_with("Yeni klasör") && b.ends_with("Yeni klasör (2)"));
        let f = new_text_file(&d).unwrap();
        assert!(rename(&f, "x.txt").unwrap());
        assert!(rename(&a, "x.txt").is_err(), "taken name refused");
        assert!(rename(&d.join("x.txt"), "X.txt").unwrap(), "case-only rename");
        assert!(transfer(Transfer::Copy, &[d.join("X.txt")], &d).unwrap());
        assert!(d.join("X (2).txt").exists());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn copies_symlinks_as_links_and_refuses_self_nesting() {
        let d = base("links");
        std::fs::create_dir_all(d.join("src/alt")).unwrap();
        std::fs::write(d.join("hedef.txt"), b"dokunma").unwrap();
        std::os::unix::fs::symlink(d.join("hedef.txt"), d.join("src/alt/link")).unwrap();
        std::fs::create_dir(d.join("out")).unwrap();
        let t = tracker();
        assert!(transfer_tracked(Transfer::Copy, &[d.join("src")], &d.join("out"), t.clone()).unwrap());
        let link = d.join("out/src/alt/link");
        assert!(std::fs::symlink_metadata(&link).unwrap().file_type().is_symlink());
        assert_eq!(std::fs::read_link(&link).unwrap(), d.join("hedef.txt"));
        assert_eq!(t.results.lock().unwrap().clone(), vec![(d.join("src"), d.join("out/src"))]);
        assert!(transfer(Transfer::Copy, &[d.join("src")], &d.join("src/alt")).is_err());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn move_with_conflicts() {
        let d = base("move");
        std::fs::create_dir_all(d.join("a")).unwrap();
        std::fs::create_dir_all(d.join("b")).unwrap();
        std::fs::write(d.join("a/aynı.txt"), b"yeni").unwrap();
        std::fs::write(d.join("b/aynı.txt"), b"eski").unwrap();
        // keep both (no ask): the old one stays, the moved one gets "(2)"
        assert!(transfer(Transfer::Move, &[d.join("a/aynı.txt")], &d.join("b")).unwrap());
        assert_eq!(std::fs::read(d.join("b/aynı (2).txt")).unwrap(), b"yeni");
        assert!(!d.join("a/aynı.txt").exists());
        // replace
        std::fs::write(d.join("a/aynı.txt"), b"daha yeni").unwrap();
        let mut t = Tracker::new(Box::new(|_, _, _| {}), Default::default());
        t.ask = Some(Box::new(|c| Some(vec![Resolve::Replace; c.len()])));
        assert!(transfer_tracked(Transfer::Move, &[d.join("a/aynı.txt")], &d.join("b"), Arc::new(t)).unwrap());
        assert_eq!(std::fs::read(d.join("b/aynı.txt")).unwrap(), b"daha yeni");
        let _ = std::fs::remove_dir_all(&d);
    }
}
