//! Keeps [`crate::nameindex::Index`]es of chosen roots (the home folder, local data disks)
//! ready for searching.
//!
//! Linux version of upstream's `rf-win/src/indexer.rs`, same API. [`start`] loads each root's
//! saved index (usable at once), then walks the root again in the background and swaps the
//! fresh index in: changes made while the app wasn't running are picked up that way. Indexes are
//! saved after a build and at most every ten minutes while they change. [`stop`] ends it all.
//!
//! **Watching.** inotify watches one folder at a time, and a recursive watch (the `notify`
//! crate's) would walk the whole tree a second time and watch caches and `.git` folders the
//! index leaves out. So the walk itself adds a watch on each folder it indexes, just before
//! listing it ([`crate::nameindex::Index::build_with`]); changed folders are listed again once
//! a second. Watches are limited per user (`/proc/sys/fs/inotify/max_user_watches`, shared
//! with every other program): the index takes at most half of what's left. A root with more
//! folders than that, or whose event queue overflowed, is walked again every 30 minutes
//! instead of missing changes for good.

use std::collections::{HashMap, HashSet};
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use parking_lot::{Mutex, RwLock};

use crate::nameindex::Index;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    /// Nothing usable yet: the first walk is running.
    Building,
    /// Searchable (a saved index may be refreshing behind it).
    Ready,
    /// The root couldn't be indexed.
    Failed,
}

/// One inotify instance: the folders of one root.
struct Watches {
    fd: libc::c_int,
    paths: Mutex<HashMap<libc::c_int, PathBuf>>,
    /// Watches held by all roots, and how many they may hold.
    used: Arc<AtomicUsize>,
    budget: usize,
    /// Some folder went unwatched (budget spent, or the kernel refused).
    partial: AtomicBool,
    /// Events were lost (the kernel's queue overflowed).
    overflow: AtomicBool,
}

const MASK: u32 = libc::IN_CREATE | libc::IN_DELETE | libc::IN_MOVED_FROM | libc::IN_MOVED_TO | libc::IN_ONLYDIR | libc::IN_DONT_FOLLOW | libc::IN_EXCL_UNLINK;

impl Watches {
    fn new(used: Arc<AtomicUsize>, budget: usize) -> Option<Watches> {
        // SAFETY: plain syscall
        let fd = unsafe { libc::inotify_init1(libc::IN_NONBLOCK | libc::IN_CLOEXEC) };
        if fd < 0 {
            log::info!("indexer: inotify: {}", std::io::Error::last_os_error());
            return None;
        }
        Some(Watches { fd, paths: Mutex::new(HashMap::new()), used, budget, partial: AtomicBool::new(false), overflow: AtomicBool::new(false) })
    }

    /// Watches `dir` (again: a renamed folder keeps its watch, under its new path).
    fn add(&self, dir: &Path) {
        let mut paths = self.paths.lock();
        if self.used.load(Ordering::Relaxed) >= self.budget {
            self.partial.store(true, Ordering::Relaxed);
            return;
        }
        let Ok(c) = std::ffi::CString::new(dir.as_os_str().as_bytes()) else { return };
        // SAFETY: valid fd and C string
        let wd = unsafe { libc::inotify_add_watch(self.fd, c.as_ptr(), MASK) };
        if wd < 0 {
            let e = std::io::Error::last_os_error();
            match e.raw_os_error() {
                Some(libc::EACCES) | Some(libc::ENOENT) | Some(libc::ENOTDIR) => {}
                _ => {
                    if !self.partial.swap(true, Ordering::Relaxed) {
                        log::info!("indexer: no more watches ({e}): changes deeper down are picked up by walking again");
                    }
                }
            }
            return;
        }
        if paths.insert(wd, dir.to_path_buf()).is_none() {
            self.used.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// Waits up to `ms` for events and adds the folders they happened in to `dirty`.
    fn read(&self, ms: i32, dirty: &Mutex<HashSet<PathBuf>>, ignore: &Path) {
        let mut pfd = libc::pollfd { fd: self.fd, events: libc::POLLIN, revents: 0 };
        // SAFETY: one valid pollfd
        if unsafe { libc::poll(&mut pfd, 1, ms) } <= 0 {
            return;
        }
        let mut buf = vec![0u8; 64 << 10];
        loop {
            // SAFETY: the buffer is ours and that long
            let n = unsafe { libc::read(self.fd, buf.as_mut_ptr().cast(), buf.len()) };
            if n <= 0 {
                return;
            }
            let mut d = dirty.lock();
            let mut paths = self.paths.lock();
            let mut off = 0usize;
            let head = std::mem::size_of::<libc::inotify_event>();
            while off + head <= n as usize {
                // SAFETY: the kernel writes whole events; read unaligned
                let ev: libc::inotify_event = unsafe { std::ptr::read_unaligned(buf.as_ptr().add(off).cast()) };
                off += head + ev.len as usize;
                if ev.mask & libc::IN_Q_OVERFLOW != 0 {
                    self.overflow.store(true, Ordering::Relaxed);
                    continue;
                }
                if ev.mask & libc::IN_IGNORED != 0 {
                    // the folder is gone (its parent's event says so too)
                    if paths.remove(&ev.wd).is_some() {
                        self.used.fetch_sub(1, Ordering::Relaxed);
                    }
                    continue;
                }
                if let Some(p) = paths.get(&ev.wd) {
                    if !p.starts_with(ignore) {
                        d.insert(p.clone());
                    }
                }
            }
        }
    }
}

impl Drop for Watches {
    fn drop(&mut self) {
        self.used.fetch_sub(self.paths.get_mut().len(), Ordering::Relaxed);
        // SAFETY: our fd, closed once (this drops its watches too)
        unsafe {
            libc::close(self.fd);
        }
    }
}

struct Slot {
    root: PathBuf,
    file: PathBuf,
    index: RwLock<Option<Index>>,
    state: AtomicU8,
    dirty: Mutex<HashSet<PathBuf>>,
    changed_since_save: AtomicBool,
    watches: Option<Watches>,
}

struct Service {
    slots: Vec<Arc<Slot>>,
    cancel: Arc<AtomicBool>,
}

static SERVICE: OnceLock<Mutex<Option<Service>>> = OnceLock::new();

fn service() -> &'static Mutex<Option<Service>> {
    SERVICE.get_or_init(|| Mutex::new(None))
}

fn file_for(dir: &Path, root: &Path) -> PathBuf {
    let tag: String = root.to_string_lossy().chars().map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '_' }).collect();
    let tag = tag.trim_matches('_');
    dir.join(format!("{}.rfidx", if tag.is_empty() { "root" } else { tag }))
}

/// Half of the inotify watches this user has left (other programs' included, roughly: the
/// kernel doesn't say how many are in use).
fn watch_budget() -> usize {
    let max = std::fs::read_to_string("/proc/sys/fs/inotify/max_user_watches").ok().and_then(|s| s.trim().parse::<usize>().ok()).unwrap_or(8192);
    max / 2
}

/// Indexes `roots`, saving the indexes in `dir`. Replaces a running set.
pub fn start(roots: Vec<PathBuf>, dir: PathBuf) {
    stop();
    let _ = std::fs::create_dir_all(&dir);
    let cancel = Arc::new(AtomicBool::new(false));
    let used = Arc::new(AtomicUsize::new(0));
    let budget = watch_budget();
    let mut slots = Vec::new();
    for root in roots {
        slots.push(Arc::new(Slot {
            file: file_for(&dir, &root),
            root,
            index: RwLock::new(None),
            state: AtomicU8::new(State::Building as u8),
            dirty: Mutex::new(HashSet::new()),
            changed_since_save: AtomicBool::new(false),
            watches: Watches::new(used.clone(), budget),
        }));
    }
    for slot in &slots {
        let (s, c, ignore) = (slot.clone(), cancel.clone(), dir.clone());
        // changed folders, collected from the first watch on (a walk running meanwhile misses
        // nothing: the fresh index is refreshed with them once it's swapped in)
        let _ = std::thread::Builder::new().name("rf-index-watch".into()).spawn(move || {
            while !c.load(Ordering::Relaxed) {
                match &s.watches {
                    Some(w) => w.read(500, &s.dirty, &ignore),
                    None => std::thread::sleep(Duration::from_millis(500)),
                }
            }
        });
        let (slot, cancel) = (slot.clone(), cancel.clone());
        let _ = std::thread::Builder::new().name("rf-indexer".into()).spawn(move || run(slot, cancel));
    }
    *service().lock() = Some(Service { slots, cancel });
}

/// Stops indexing (the indexes are dropped; saved files stay).
pub fn stop() {
    if let Some(s) = service().lock().take() {
        s.cancel.store(true, Ordering::Relaxed);
        for slot in &s.slots {
            save(slot);
        }
    }
}

fn save(slot: &Slot) {
    if !slot.changed_since_save.swap(false, Ordering::Relaxed) {
        return;
    }
    let mut guard = slot.index.write();
    if let Some(ix) = guard.as_mut() {
        if ix.len() > 0 && ix.len() - ix.live() > ix.len() / 5 {
            ix.compact();
        }
        if let Err(e) = ix.save(&slot.file) {
            log::info!("indexer: saving {}: {e}", slot.file.display());
        }
    }
}

/// Walks the root (watching each folder) and swaps the result in.
fn rebuild(slot: &Slot, cancel: &AtomicBool) {
    let t = Instant::now();
    let watch = |p: &Path| {
        if let Some(w) = &slot.watches {
            w.add(p);
        }
    };
    let fresh = Index::build_with(&slot.root, cancel, Some(&watch));
    if cancel.load(Ordering::Relaxed) {
        return;
    }
    let watched = slot.watches.as_ref().map_or(0, |w| w.paths.lock().len());
    log::info!("indexer: {} walked in {:.1} s, {} entries, {} MB, {watched} folders watched", slot.root.display(), t.elapsed().as_secs_f32(), fresh.live(), fresh.memory() >> 20);
    if fresh.is_empty() && slot.index.read().is_none() {
        slot.state.store(State::Failed as u8, Ordering::Relaxed);
    } else {
        *slot.index.write() = Some(fresh);
        slot.state.store(State::Ready as u8, Ordering::Relaxed);
        slot.changed_since_save.store(true, Ordering::Relaxed);
        save(slot);
    }
}

fn run(slot: Arc<Slot>, cancel: Arc<AtomicBool>) {
    // 1. the saved index, searchable at once
    match Index::load(&slot.file) {
        Ok(ix) if ix.root() == slot.root => {
            log::info!("indexer: {} loaded, {} entries", slot.root.display(), ix.live());
            *slot.index.write() = Some(ix);
            slot.state.store(State::Ready as u8, Ordering::Relaxed);
        }
        _ => {}
    }
    // 2. a fresh walk (catches what changed while the app was closed; watches every folder)
    rebuild(&slot, &cancel);
    // 3. keep up with changes
    let mut last_save = Instant::now();
    let mut last_walk = Instant::now();
    while !cancel.load(Ordering::Relaxed) {
        std::thread::sleep(Duration::from_secs(1));
        // lost events, or folders without a watch: only a walk sees everything
        if let Some(w) = &slot.watches {
            let lost = w.overflow.swap(false, Ordering::Relaxed);
            if lost || (w.partial.load(Ordering::Relaxed) && last_walk.elapsed() > Duration::from_secs(1800)) {
                log::info!("indexer: {} walked again ({})", slot.root.display(), if lost { "events were lost" } else { "not every folder is watched" });
                w.partial.store(false, Ordering::Relaxed);
                slot.dirty.lock().clear();
                rebuild(&slot, &cancel);
                last_walk = Instant::now();
                continue;
            }
        }
        let dirs: Vec<PathBuf> = slot.dirty.lock().drain().collect();
        if !dirs.is_empty() {
            let watch = |p: &Path| {
                if let Some(w) = &slot.watches {
                    w.add(p);
                }
            };
            if let Some(ix) = slot.index.write().as_mut() {
                ix.refresh_dirs_with(&dirs, &cancel, Some(&watch));
            }
            slot.changed_since_save.store(true, Ordering::Relaxed);
        }
        if last_save.elapsed() > Duration::from_secs(600) {
            save(&slot);
            last_save = Instant::now();
        }
    }
}

/// Searches the index whose root holds `under`: `None` when no ready index covers it (search
/// the disk then).
pub fn search(pattern: &str, under: &Path, include_hidden: bool, max: usize) -> Option<Vec<PathBuf>> {
    let slot = {
        let g = service().lock();
        let s = g.as_ref()?;
        // the innermost root (a data disk mounted inside the home folder)
        s.slots.iter().filter(|s| under.starts_with(&s.root)).max_by_key(|s| s.root.components().count()).cloned()?
    };
    if slot.state.load(Ordering::Relaxed) != State::Ready as u8 {
        return None;
    }
    let ix = slot.index.read();
    ix.as_ref()?.search(pattern, Some(under), include_hidden, max)
}

/// (root, state, live entries, bytes in memory) per root.
pub fn status() -> Vec<(PathBuf, State, usize, usize)> {
    let g = service().lock();
    let Some(s) = g.as_ref() else { return Vec::new() };
    s.slots
        .iter()
        .map(|slot| {
            let state = match slot.state.load(Ordering::Relaxed) {
                x if x == State::Ready as u8 => State::Ready,
                x if x == State::Failed as u8 => State::Failed,
                _ => State::Building,
            };
            let (live, mem) = slot.index.try_read().and_then(|i| i.as_ref().map(|i| (i.live(), i.memory()))).unwrap_or((0, 0));
            (slot.root.clone(), state, live, mem)
        })
        .collect()
}

/// Local disk file systems (a FUSE, network or pseudo file system is never one).
const LOCAL_FS: &[&str] = &["ext2", "ext3", "ext4", "btrfs", "xfs", "f2fs", "bcachefs", "zfs", "jfs", "reiserfs", "nilfs2", "ntfs3", "ntfs", "exfat", "vfat"];

/// The folders worth indexing: the home folder, and fixed local disks mounted outside it and
/// outside the system's own folders (`/mnt/data`, a second disk at `/home/shared`…). Network
/// shares, FUSE mounts (cloud drives) and removable disks stay out: walking them would download
/// files or wake disks. The system itself (`/`, `/usr`) isn't indexed: a search there walks.
pub fn local_roots() -> Vec<PathBuf> {
    let home = dirs::home_dir().filter(|h| h.is_dir() && h != Path::new("/"));
    let mut roots: Vec<PathBuf> = home.iter().cloned().collect();
    for d in crate::drives::list_drives() {
        if d.kind != crate::drives::DriveKind::Fixed || !LOCAL_FS.contains(&d.fs.as_str()) || d.root == Path::new("/") {
            continue;
        }
        // /home holding the home folder: the home folder is enough (others' aren't readable)
        if home.as_ref().is_some_and(|h| h.starts_with(&d.root)) {
            continue;
        }
        roots.push(d.root);
    }
    // a root inside another is covered by it
    let all = roots.clone();
    roots.retain(|r| !all.iter().any(|o| o != r && r.starts_with(o)));
    roots.sort();
    roots.dedup();
    roots
}

pub fn running() -> bool {
    service().lock().is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wait(what: &str, f: impl Fn() -> bool) {
        let t = Instant::now();
        while !f() {
            assert!(t.elapsed() < Duration::from_secs(10), "timed out waiting for {what}");
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    fn names(v: &[PathBuf]) -> Vec<String> {
        let mut n: Vec<String> = v.iter().map(|p| p.file_name().unwrap().to_string_lossy().into_owned()).collect();
        n.sort();
        n
    }

    /// One test for the whole service (it is a process-wide singleton).
    #[test]
    fn indexes_watches_and_keeps_up() {
        let base = std::env::temp_dir().join(format!("rf-indexer-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let (root, store) = (base.join("ev"), base.join("store"));
        std::fs::create_dir_all(root.join("Belgeler/2026")).unwrap();
        std::fs::write(root.join("Belgeler/rapor.pdf"), b"x").unwrap();
        start(vec![root.clone()], store.clone());
        wait("the first walk", || status().first().is_some_and(|s| s.1 == State::Ready));
        assert!(running());
        assert_eq!(names(&search("rapor", &root, false, 100).unwrap()), ["rapor.pdf"]);
        // outside every root: the disk is walked
        assert!(search("rapor", &base, false, 100).is_none());

        // new files, a new folder with contents, a rename, a delete: all followed
        std::fs::write(root.join("Belgeler/2026/rapor yeni.md"), b"x").unwrap();
        std::fs::create_dir_all(root.join("Proje/src")).unwrap();
        std::fs::write(root.join("Proje/src/rapor.rs"), b"x").unwrap();
        wait("new entries", || search("rapor", &root, false, 100).is_some_and(|v| names(&v) == ["rapor yeni.md", "rapor.pdf", "rapor.rs"]));
        // a file inside the new folder after it was walked: its watch came from the walk
        std::fs::write(root.join("Proje/src/rapor2.rs"), b"x").unwrap();
        wait("a file in a new folder", || search("rapor2", &root, false, 100).is_some_and(|v| v.len() == 1));
        std::fs::rename(root.join("Proje"), root.join("Proje eski")).unwrap();
        wait("a renamed folder", || search("rapor.rs", &root, false, 100).is_some_and(|v| v == [root.join("Proje eski/src/rapor.rs")]));
        // the renamed folder's watches moved with it
        std::fs::write(root.join("Proje eski/src/rapor3.rs"), b"x").unwrap();
        wait("a file in a renamed folder", || search("rapor3", &root, false, 100).is_some_and(|v| v == [root.join("Proje eski/src/rapor3.rs")]));
        std::fs::remove_file(root.join("Belgeler/rapor.pdf")).unwrap();
        wait("a deleted file", || search("rapor.pdf", &root, false, 100).is_some_and(|v| v.is_empty()));

        // saved on stop, loaded (searchable at once) on the next start
        stop();
        assert!(!running());
        assert!(file_for(&store, &root).exists());
        start(vec![root.clone()], store.clone());
        wait("the saved index", || status().first().is_some_and(|s| s.1 == State::Ready));
        assert_eq!(names(&search("rapor", &root, false, 100).unwrap()), ["rapor yeni.md", "rapor.rs", "rapor2.rs", "rapor3.rs"]);
        stop();
        std::fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn roots_are_home_and_data_disks() {
        let roots = local_roots();
        if let Some(h) = dirs::home_dir() {
            assert!(roots.contains(&h));
        }
        assert!(!roots.iter().any(|r| r == Path::new("/")));
        for r in &roots {
            assert!(!roots.iter().any(|o| o != r && r.starts_with(o)), "{} is inside another root", r.display());
        }
    }

    #[test]
    fn index_files_are_named_after_their_root() {
        assert_eq!(file_for(Path::new("/x"), Path::new("/home/me")), Path::new("/x/home_me.rfidx"));
        assert_eq!(file_for(Path::new("/x"), Path::new("/")), Path::new("/x/root.rfidx"));
    }
}
