//! Folder size calculation: the details view's Size column ("Boyutu hesapla" / Settings →
//! "Klasör boyutlarını hesapla"), the preview pane and Properties.
//!
//! Directories are listed with `FindFirstFileExW` (basic info, large fetch) by a few shared
//! worker threads at below-normal CPU priority (the UI thread always wins; I/O priority stays
//! normal, background mode throttles the disk so hard that a column of folders crawls).
//! The workers serve all requested folders **round-robin**, one directory at a time: a small
//! folder clicked while a huge one is being measured still finishes in milliseconds instead of
//! queueing behind it. Reparse points (junctions, symlinks, OneDrive placeholders) are neither
//! counted nor followed. Unreadable sub-folders are skipped silently.
//!
//! Hard links are counted once per link; compressed / sparse files are measured by logical size.
//!
//! The shared cache can be kept in a file between runs ([`FolderSizeCache::persist`]). A stored
//! value is only a first guess: a folder's own date does not move when something deep inside it
//! changes, so values read from the file are *provisional*. They are shown at once
//! ([`SizeState::Provisional`]) and every provisional folder is measured again when it is asked
//! for; the fresh result replaces the guess.
// Generated from rf-win's public API by a one-off stub generator, then edited by hand.
// Functions return safe defaults until they get a Linux implementation.
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, SystemTime};
use parking_lot::{Condvar, Mutex};
pub use crate::hash::Cancelled;
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FolderSize {
    /// Sum of logical file sizes.
    pub bytes: u64,
    pub files: u64,
    /// Sub-folders (the root itself is not counted).
    pub dirs: u64,
    /// Approximate size on disk: every file rounded up to the volume cluster size. `None` when
    /// the cluster size could not be determined.
    pub allocated: Option<u64>,
}
#[derive(Default)]
struct Counters {
    bytes: AtomicU64,
    files: AtomicU64,
    dirs: AtomicU64,
    allocated: AtomicU64,
}
impl Counters {
    fn snapshot(&self, track_alloc: bool) -> FolderSize {
        FolderSize {
            bytes: self.bytes.load(Ordering::Relaxed),
            files: self.files.load(Ordering::Relaxed),
            dirs: self.dirs.load(Ordering::Relaxed),
            allocated: track_alloc.then(|| self.allocated.load(Ordering::Relaxed)),
        }
    }
}
/// Bytes per cluster of the volume holding `dir`.
fn cluster_size(_dir: &Path) -> Option<u64> {
    None
}
/// Folder sizes shared by the Size column, the preview pane and Properties.
pub fn shared_cache() -> &'static FolderSizeCache {
    static CACHE: OnceLock<FolderSizeCache> = OnceLock::new();
    CACHE.get_or_init(FolderSizeCache::default)
}
/// One folder being measured.
struct Job {
    batch: Arc<SizeBatch>,
    i: usize,
    root: PathBuf,
    /// Directories still to list (depth-first: the most recently found first).
    todo: Mutex<Vec<PathBuf>>,
    started: AtomicBool,
}
struct Sched {
    jobs: Mutex<Vec<Arc<Job>>>,
    cv: Condvar,
    next: AtomicUsize,
}
/// Measured on NVMe with 16 hardware threads: 8 workers is the sweet spot, more only contend.
fn sched() -> &'static Sched {
    static S: std::sync::OnceLock<&'static Sched> = std::sync::OnceLock::new();
    S.get_or_init(|| {
        let s: &'static Sched = Box::leak(Box::new(Sched { jobs: Mutex::new(Vec::new()), cv: Condvar::new(), next: AtomicUsize::new(0) }));
        let n = std::thread::available_parallelism().map_or(4, |n| n.get()).clamp(4, 8);
        for k in 0..n {
            let _ = std::thread::Builder::new().name(format!("rf-dirsize-{k}")).spawn(move || {
                // below normal, like the Windows build
                // SAFETY: plain syscall on the calling thread
                unsafe {
                    libc::setpriority(libc::PRIO_PROCESS, 0, 5);
                }
                worker(s)
            });
        }
        s
    })
}
/// Next (job, directory), taking the jobs in turn so every folder keeps moving.
fn take(s: &Sched) -> (Arc<Job>, PathBuf) {
    let mut jobs = s.jobs.lock();
    loop {
        let n = jobs.len();
        if n > 0 {
            let start = s.next.fetch_add(1, Ordering::Relaxed);
            for k in 0..n {
                let job = &jobs[(start + k) % n];
                if let Some(dir) = job.todo.lock().pop() {
                    return (job.clone(), dir);
                }
            }
        }
        s.cv.wait_for(&mut jobs, Duration::from_millis(250));
    }
}
fn worker(s: &'static Sched) {
    loop {
        let (job, dir) = take(s);
        process(s, &job, dir);
    }
}
fn finish(s: &Sched, job: &Job) {
    let slot = &job.batch.slots[job.i];
    if !job.batch.stop.load(Ordering::Relaxed) {
        let size = slot.c.snapshot(slot.cluster.get().copied().flatten().is_some());
        shared_cache().insert_at(&job.root, size, slot.mtime.get().copied().flatten());
    }
    slot.done.store(true, Ordering::Release);
    s.jobs.lock().retain(|j| !(Arc::ptr_eq(&j.batch, &job.batch) && j.i == job.i));
}
fn process(s: &Sched, job: &Job, dir: PathBuf) {
    let slot = &job.batch.slots[job.i];
    if job.batch.stop.load(Ordering::Relaxed) {
        let left = job.todo.lock().drain(..).count();
        if slot.pending.fetch_sub(left + 1, Ordering::AcqRel) == left + 1 {
            finish(s, job);
        }
        return;
    }
    if !job.started.swap(true, Ordering::Relaxed) {
        if let Some(c) = shared_cache().get(&job.root) {
            slot.c.bytes.store(c.bytes, Ordering::Relaxed);
            slot.c.files.store(c.files, Ordering::Relaxed);
            slot.c.dirs.store(c.dirs, Ordering::Relaxed);
            slot.c.allocated.store(c.allocated.unwrap_or(0), Ordering::Relaxed);
            let _ = slot.cluster.set(c.allocated.map(|_| 1));
            let _ = slot.mtime.set(dir_mtime(&job.root));
            slot.pending.store(0, Ordering::Release);
            finish(s, job);
            return;
        }
        let _ = slot.mtime.set(dir_mtime(&job.root));
        let _ = slot.cluster.set(cluster_size(&job.root));
    }
    let cluster = slot.cluster.get().copied().flatten();
    let (mut bytes, mut files, mut alloc) = (0u64, 0u64, 0u64);
    let mut subdirs = Vec::new();
    // Linux: plain readdir. Symlinks are never followed (they play the reparse point's
    // part), and the allocated size comes from st_blocks instead of the cluster size.
    let _ = cluster;
    match std::fs::read_dir(&dir) {
        Ok(rd) => {
            use std::os::unix::fs::MetadataExt;
            for e in rd.flatten() {
                // DirEntry::metadata does not follow symlinks on Unix
                let Ok(m) = e.metadata() else { continue };
                let ft = m.file_type();
                if ft.is_symlink() {
                    continue;
                }
                if ft.is_dir() {
                    subdirs.push(e.path());
                } else {
                    bytes += m.len();
                    files += 1;
                    alloc += m.blocks() * 512;
                }
            }
        }
        Err(e) => log::debug!("foldersize: {} failed ({e})", dir.display()),
    }
    slot.c.bytes.fetch_add(bytes, Ordering::Relaxed);
    slot.c.files.fetch_add(files, Ordering::Relaxed);
    slot.c.dirs.fetch_add(subdirs.len() as u64, Ordering::Relaxed);
    slot.c.allocated.fetch_add(alloc, Ordering::Relaxed);
    if !subdirs.is_empty() {
        slot.pending.fetch_add(subdirs.len(), Ordering::AcqRel);
        let n = subdirs.len();
        job.todo.lock().extend(subdirs);
        drop(s.jobs.lock());
        for _ in 0..n.min(8) {
            s.cv.notify_one();
        }
    }
    if slot.pending.fetch_sub(1, Ordering::AcqRel) == 1 {
        finish(s, job);
    }
}
#[derive(Default)]
struct Slot {
    c: Counters,
    /// Directories queued or being listed for this folder.
    pending: AtomicUsize,
    done: AtomicBool,
    cluster: OnceLock<Option<u64>>,
    mtime: OnceLock<Option<SystemTime>>,
    /// Value remembered from an earlier run, shown until the new measurement ends.
    seed: Option<FolderSize>,
}
/// State of one folder in a [`SizeBatch`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SizeState {
    /// Still walking; bytes counted so far.
    Partial(u64),
    /// Still being measured again, but an earlier run's result is known: show it and let the
    /// fresh one replace it silently.
    Provisional(FolderSize),
    Done(FolderSize),
}
/// Sizes of several folders, measured together by the shared workers. Each folder finishes on
/// its own; results also land in [`shared_cache`] (a folder measured before and unchanged since
/// comes straight from there).
pub struct SizeBatch {
    slots: Vec<Slot>,
    stop: AtomicBool,
}
impl SizeBatch {
    /// Starts at once and returns immediately (nothing touches the disk on the calling thread).
    pub fn start(dirs: Vec<PathBuf>) -> Arc<Self> {
        let b = Arc::new(SizeBatch {
            slots: dirs.iter().map(|d| Slot { seed: shared_cache().provisional(d), ..Default::default() }).collect(),
            stop: AtomicBool::new(false),
        });
        let s = sched();
        let mut jobs = s.jobs.lock();
        for (i, dir) in dirs.into_iter().enumerate() {
            b.slots[i].pending.store(1, Ordering::Relaxed);
            jobs.push(
                Arc::new(Job {
                    batch: b.clone(),
                    i,
                    root: dir.clone(),
                    todo: Mutex::new(vec![dir]),
                    started: AtomicBool::new(false),
                }),
            );
        }
        drop(jobs);
        s.cv.notify_all();
        b
    }
    pub fn len(&self) -> usize {
        self.slots.len()
    }
    pub fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }
    pub fn state(&self, i: usize) -> SizeState {
        let s = &self.slots[i];
        if s.done.load(Ordering::Acquire) {
            SizeState::Done(s.c.snapshot(s.cluster.get().copied().flatten().is_some()))
        } else if let Some(seed) = s.seed {
            SizeState::Provisional(seed)
        } else {
            SizeState::Partial(s.c.bytes.load(Ordering::Relaxed))
        }
    }
    /// Running totals (final once [`finished`](Self::finished)).
    pub fn totals(&self, i: usize) -> FolderSize {
        let s = &self.slots[i];
        s.c.snapshot(s.cluster.get().copied().flatten().is_some())
    }
    pub fn finished(&self) -> bool {
        self.slots.iter().all(|s| s.done.load(Ordering::Acquire))
    }
    /// Stops all walks soon; results are then not cached.
    pub fn cancel(&self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}
/// Totals for everything under `dir` (blocking: call off the UI thread). `progress` receives
/// running totals about every 100 ms on the calling thread, `cancelled` is polled there too; on
/// cancel the error downcasts to [`Cancelled`]. Not cached here: see [`FolderSizeCache`].
pub fn folder_size(
    dir: &Path,
    cancelled: &dyn Fn() -> bool,
    progress: &mut dyn FnMut(&FolderSize),
) -> anyhow::Result<FolderSize> {
    anyhow::ensure!(dir.is_dir(), "Klasör bulunamadı: {}", dir.display());
    shared_cache().invalidate(dir);
    let b = SizeBatch::start(vec![dir.to_path_buf()]);
    let mut since = std::time::Instant::now();
    while !b.finished() {
        if cancelled() {
            b.cancel();
            return Err(Cancelled.into());
        }
        std::thread::sleep(Duration::from_millis(5));
        if since.elapsed() >= Duration::from_millis(100) {
            since = std::time::Instant::now();
            progress(&b.totals(0));
        }
    }
    let total = b.totals(0);
    progress(&total);
    Ok(total)
}
struct CacheEntry {
    /// Last-write time of the root folder when computed.
    mtime: Option<SystemTime>,
    size: FolderSize,
    seq: u64,
    /// Read from the cache file, not measured in this run: good for showing, not for trusting.
    provisional: bool,
}
/// Folders kept in the cache file (the most recently used ones).
pub const PERSIST_LIMIT: usize = 20_000;
/// How long after a change the cache file is written; results arrive in bursts, so one write
/// covers many of them.
const SAVE_DELAY: Duration = Duration::from_secs(3);
/// Concurrent cache of computed folder sizes, keyed by path (Linux: case-sensitive) and validated
/// against the root folder's last-write time. The root's mtime changes when entries are added,
/// removed or renamed directly inside it, **not** when something deeper changes, so deep edits
/// show up after [`invalidate`](Self::invalidate) (e.g. from the file watcher) or a refresh.
pub struct FolderSizeCache {
    map: Mutex<HashMap<String, CacheEntry>>,
    seq: AtomicU64,
    capacity: usize,
    /// Something changed since the cache file was last written.
    dirty: Mutex<bool>,
    dirty_cv: Condvar,
}
impl Default for FolderSizeCache {
    fn default() -> Self {
        Self::new(PERSIST_LIMIT)
    }
}
/// Linux: names are case-sensitive and the separator is `/` (`/` itself stays `/`).
fn key(dir: &Path) -> String {
    let s = dir.to_string_lossy();
    let t = s.trim_end_matches('/');
    if t.is_empty() && s.starts_with('/') { "/".into() } else { t.to_string() }
}
fn dir_mtime(dir: &Path) -> Option<SystemTime> {
    std::fs::metadata(dir).and_then(|m| m.modified()).ok()
}
impl FolderSizeCache {
    pub fn new(capacity: usize) -> Self {
        Self {
            map: Mutex::new(HashMap::new()),
            seq: AtomicU64::new(1),
            capacity: capacity.max(16),
            dirty: Mutex::new(false),
            dirty_cv: Condvar::new(),
        }
    }
    fn mark_dirty(&self) {
        *self.dirty.lock() = true;
        self.dirty_cv.notify_one();
    }
    /// Cached size if it was measured in this run and the folder's mtime is unchanged. Cheap
    /// (one `stat`). Values remembered from an earlier run never answer here: see
    /// [`provisional`](Self::provisional).
    pub fn get(&self, dir: &Path) -> Option<FolderSize> {
        let mtime = dir_mtime(dir)?;
        let map = self.map.lock();
        let e = map.get(&key(dir))?;
        (!e.provisional && e.mtime == Some(mtime)).then_some(e.size)
    }
    /// The value remembered from an earlier run, if that is all there is. No disk access.
    pub fn provisional(&self, dir: &Path) -> Option<FolderSize> {
        let map = self.map.lock();
        map.get(&key(dir)).filter(|e| e.provisional).map(|e| e.size)
    }
    pub fn insert(&self, dir: &Path, size: FolderSize) {
        self.insert_at(dir, size, dir_mtime(dir));
    }
    fn insert_at(&self, dir: &Path, size: FolderSize, mtime: Option<SystemTime>) {
        {
            let mut map = self.map.lock();
            if map.len() >= self.capacity {
                // Evict the older half.
                let mut seqs: Vec<u64> = map.values().map(|e| e.seq).collect();
                seqs.sort_unstable();
                let cut = seqs[seqs.len() / 2];
                map.retain(|_, e| e.seq > cut);
            }
            map.insert(key(dir), CacheEntry { mtime, size, seq: self.seq.fetch_add(1, Ordering::Relaxed), provisional: false });
        }
        self.mark_dirty();
    }
    /// Forgets `dir` and every cached folder below it.
    pub fn invalidate(&self, dir: &Path) {
        let k = key(dir);
        let prefix = if k == "/" { k.clone() } else { format!("{k}/") };
        self.map.lock().retain(|p, _| *p != k && !p.starts_with(&prefix));
        self.mark_dirty();
    }
    pub fn clear(&self) {
        self.map.lock().clear();
        self.mark_dirty();
    }
    /// Returns the cached value, or computes (blocking, see [`folder_size`]) and stores it.
    /// Cancelled or failed computations are not cached.
    pub fn get_or_compute(
        &self,
        dir: &Path,
        cancelled: &dyn Fn() -> bool,
        progress: &mut dyn FnMut(&FolderSize),
    ) -> anyhow::Result<FolderSize> {
        if let Some(s) = self.get(dir) {
            progress(&s);
            return Ok(s);
        }
        let before = dir_mtime(dir);
        let size = folder_size(dir, cancelled, progress)?;
        self.insert_at(dir, size, before);
        Ok(size)
    }

    // --------------------------------------------------------------------- cache file

    /// Writes the `limit` most recently used folders to `path`: into a temporary file next to
    /// it first, then renamed over it, so a crash never leaves half a file.
    pub fn save_to(&self, path: &Path, limit: usize) -> std::io::Result<()> {
        let mut rows: Vec<(u64, String, FolderSize)> = {
            let map = self.map.lock();
            map.iter().map(|(k, e)| (e.seq, k.clone(), e.size)).collect()
        };
        rows.sort_unstable_by_key(|r| r.0);
        let skip = rows.len().saturating_sub(limit);
        let rows = &rows[skip..];
        let mut buf = Vec::with_capacity(16 + rows.len() * 64);
        buf.extend_from_slice(FILE_MAGIC);
        buf.extend_from_slice(&(rows.len() as u32).to_le_bytes());
        // oldest first: reading back in order keeps the recency
        for (_, k, s) in rows {
            let kb = k.as_bytes();
            let n = kb.len().min(u16::MAX as usize);
            buf.extend_from_slice(&(n as u16).to_le_bytes());
            buf.extend_from_slice(&kb[..n]);
            for v in [s.bytes, s.files, s.dirs, s.allocated.unwrap_or(NO_ALLOC)] {
                buf.extend_from_slice(&v.to_le_bytes());
            }
        }
        let sum = fnv1a(&buf);
        buf.extend_from_slice(&sum.to_le_bytes());
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension("tmp");
        std::fs::write(&tmp, &buf)?;
        std::fs::rename(&tmp, path).inspect_err(|_| {
            let _ = std::fs::remove_file(&tmp);
        })
    }

    /// Reads a cache file written by [`save_to`](Self::save_to); the values enter as provisional
    /// and never replace anything measured in this run. A missing, truncated, corrupt or
    /// foreign file is ignored. Returns how many folders were taken.
    pub fn load_from(&self, path: &Path) -> usize {
        let Ok(data) = std::fs::read(path) else { return 0 };
        let Some(rows) = parse_file(&data) else {
            log::debug!("foldersize: ignoring unreadable cache file {}", path.display());
            return 0;
        };
        let mut map = self.map.lock();
        let mut taken = 0;
        // rows are oldest first: when there are more than fit, the newest ones are kept
        let skip = rows.len().saturating_sub(self.capacity.saturating_sub(map.len()));
        for (k, size) in rows.into_iter().skip(skip) {
            if let std::collections::hash_map::Entry::Vacant(v) = map.entry(k) {
                // below everything measured in this run (those start at 1), so the next
                // eviction takes these first
                v.insert(CacheEntry { mtime: None, size, seq: 0, provisional: true });
                taken += 1;
            }
        }
        taken
    }

    /// Keeps this cache in the file `path`: reads it now (on a background thread), then writes
    /// it again a few seconds after each change, off the calling thread.
    pub fn persist(&'static self, path: PathBuf) {
        self.persist_with(path, SAVE_DELAY, PERSIST_LIMIT);
    }

    fn persist_with(&'static self, path: PathBuf, delay: Duration, limit: usize) {
        let _ = std::thread::Builder::new().name("rf-dirsize-save".into()).spawn(move || {
            crate::search::lower_priority();
            let n = self.load_from(&path);
            log::debug!("foldersize: {n} remembered sizes from {}", path.display());
            loop {
                {
                    let mut d = self.dirty.lock();
                    while !*d {
                        self.dirty_cv.wait(&mut d);
                    }
                }
                std::thread::sleep(delay);
                *self.dirty.lock() = false;
                if let Err(e) = self.save_to(&path, limit) {
                    log::debug!("foldersize: could not write {}: {e}", path.display());
                }
            }
        });
    }
}

const FILE_MAGIC: &[u8; 8] = b"RFSIZE\x00\x01";
/// Stored for "size on disk unknown".
const NO_ALLOC: u64 = u64::MAX;

fn fnv1a(b: &[u8]) -> u64 {
    b.iter().fold(0xcbf29ce484222325, |h, &x| (h ^ x as u64).wrapping_mul(0x100000001b3))
}

/// The rows of a cache file, or `None` when it is not exactly what `save_to` writes.
fn parse_file(data: &[u8]) -> Option<Vec<(String, FolderSize)>> {
    let body_len = data.len().checked_sub(8)?;
    let (body, sum) = data.split_at(body_len);
    if u64::from_le_bytes(sum.try_into().ok()?) != fnv1a(body) || body.get(..8)? != FILE_MAGIC {
        return None;
    }
    let count = u32::from_le_bytes(body.get(8..12)?.try_into().ok()?) as usize;
    let mut at: usize = 12;
    let mut take = |n: usize| -> Option<&[u8]> {
        let r = body.get(at..at.checked_add(n)?)?;
        at += n;
        Some(r)
    };
    // every row is at least 34 bytes: a huge count in a short file is rejected before allocating
    let mut rows = Vec::with_capacity(count.min(body.len() / 34));
    for _ in 0..count {
        let n = u16::from_le_bytes(take(2)?.try_into().ok()?) as usize;
        let k = String::from_utf8(take(n)?.to_vec()).ok()?;
        let mut v = [0u64; 4];
        for x in &mut v {
            *x = u64::from_le_bytes(take(8)?.try_into().ok()?);
        }
        rows.push((k, FolderSize { bytes: v[0], files: v[1], dirs: v[2], allocated: (v[3] != NO_ALLOC).then_some(v[3]) }));
    }
    (at == body.len()).then_some(rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fs(bytes: u64) -> FolderSize {
        FolderSize { bytes, files: 2, dirs: 1, allocated: Some(bytes + 100) }
    }

    fn temp_file(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("rf-fsize-file-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d.join("foldersizes.bin")
    }

    fn tree(tag: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("rf-fsize-tree-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("a/b")).unwrap();
        std::fs::write(root.join("top.bin"), vec![0u8; 1000]).unwrap();
        std::fs::write(root.join("a/b/deep.bin"), vec![0u8; 75_000]).unwrap();
        root
    }

    fn wait(b: &SizeBatch) {
        let t = std::time::Instant::now();
        while !b.finished() {
            assert!(t.elapsed() < Duration::from_secs(10), "batch never finished");
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn cache_file_round_trip() {
        let path = temp_file("rt");
        let a = FolderSizeCache::default();
        a.insert_at(Path::new("/data/One"), fs(10), None);
        a.insert_at(Path::new("/data/Two/"), FolderSize { allocated: None, ..fs(20) }, None);
        a.save_to(&path, 100).unwrap();
        assert!(!path.with_extension("tmp").exists(), "temporary file left behind");
        let b = FolderSizeCache::default();
        assert_eq!(b.load_from(&path), 2);
        // trailing slash ignored, "unknown on disk" stays unknown
        assert_eq!(b.provisional(Path::new("/data/One")), Some(fs(10)));
        assert_eq!(b.provisional(Path::new("/data/Two")).map(|s| s.allocated), Some(None));
        // remembered values never count as measured
        assert!(b.get(Path::new("/data/One")).is_none());
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    /// Linux names are case-sensitive: "One" and "one" are two folders.
    #[test]
    fn keys_are_case_sensitive() {
        let a = FolderSizeCache::default();
        a.insert_at(Path::new("/data/One"), fs(10), None);
        a.insert_at(Path::new("/data/one"), fs(20), None);
        assert_eq!(a.map.lock().len(), 2);
        assert_eq!(key(Path::new("/")), "/");
        assert_eq!(key(Path::new("/data/One/")), "/data/One");
    }

    /// Forgetting a folder forgets the folders inside it, and nothing beside it.
    #[test]
    fn invalidate_takes_the_subfolders() {
        let a = FolderSizeCache::default();
        for p in ["/data", "/data/a", "/data/a/b", "/database", "/other"] {
            a.insert_at(Path::new(p), fs(1), None);
        }
        a.invalidate(Path::new("/data"));
        let mut left: Vec<String> = a.map.lock().keys().cloned().collect();
        left.sort();
        assert_eq!(left, ["/database", "/other"]);
        a.invalidate(Path::new("/"));
        assert!(a.map.lock().is_empty());
    }

    #[test]
    fn bad_cache_files_are_ignored() {
        let path = temp_file("bad");
        let a = FolderSizeCache::default();
        a.insert_at(Path::new("/x"), fs(1), None);
        a.insert_at(Path::new("/y"), fs(2), None);
        a.save_to(&path, 100).unwrap();
        let good = std::fs::read(&path).unwrap();
        let load = |bytes: &[u8]| {
            std::fs::write(&path, bytes).unwrap();
            FolderSizeCache::default().load_from(&path)
        };
        assert_eq!(load(&good), 2);
        assert_eq!(load(&good[..good.len() - 5]), 0, "truncated");
        assert_eq!(load(&good[..10]), 0, "cut in the header");
        let mut flipped = good.clone();
        flipped[20] ^= 0xFF;
        assert_eq!(load(&flipped), 0, "corrupt");
        assert_eq!(load(b"hello, this is not a cache file"), 0, "foreign");
        assert_eq!(load(&[]), 0, "empty");
        // a huge row count in a short file must not allocate or crash
        let mut lie = good[..12].to_vec();
        lie[8..12].copy_from_slice(&u32::MAX.to_le_bytes());
        let sum = fnv1a(&lie);
        lie.extend_from_slice(&sum.to_le_bytes());
        assert_eq!(load(&lie), 0, "lying count");
        assert_eq!(FolderSizeCache::default().load_from(&path.with_file_name("missing.bin")), 0);
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn cache_file_keeps_the_most_recent() {
        let path = temp_file("bound");
        let a = FolderSizeCache::default();
        for i in 0..50u64 {
            a.insert_at(Path::new(&format!("/f{i}")), fs(i), None);
        }
        a.save_to(&path, 20).unwrap();
        let b = FolderSizeCache::default();
        assert_eq!(b.load_from(&path), 20);
        assert!(b.provisional(Path::new("/f29")).is_none());
        assert!(b.provisional(Path::new("/f30")).is_some());
        assert!(b.provisional(Path::new("/f49")).is_some());
        // a reader with less room takes the newest of what is stored
        let small = FolderSizeCache::new(16);
        assert_eq!(small.load_from(&path), 16);
        assert!(small.provisional(Path::new("/f49")).is_some() && small.provisional(Path::new("/f33")).is_none());
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    /// A remembered size is shown at once, then replaced by the fresh measurement; a change deep
    /// inside (which leaves the folder's own date alone) shows up.
    #[test]
    fn provisional_value_is_replaced_by_a_fresh_measurement() {
        let root = tree("prov");
        let path = temp_file("prov");
        let a = FolderSizeCache::default();
        let s1 = a.get_or_compute(&root, &|| false, &mut |_| {}).unwrap();
        assert_eq!(s1.bytes, 76_000);
        a.save_to(&path, 100).unwrap();
        // something deep changes while the app is closed
        std::fs::write(root.join("a/b/z2.bin"), vec![0u8; 500]).unwrap();

        let fresh = shared_cache();
        fresh.invalidate(&root);
        assert!(fresh.load_from(&path) >= 1);
        assert_eq!(fresh.provisional(&root).map(|s| s.bytes), Some(76_000));
        assert!(fresh.get(&root).is_none(), "a remembered value must not pass as measured");

        let b = SizeBatch::start(vec![root.clone()]);
        assert!(matches!(b.state(0), SizeState::Provisional(s) if s.bytes == 76_000) || matches!(b.state(0), SizeState::Done(_)));
        wait(&b);
        assert!(matches!(b.state(0), SizeState::Done(s) if s.bytes == 76_500));
        assert!(fresh.provisional(&root).is_none());
        assert_eq!(fresh.get(&root).map(|s| s.bytes), Some(76_500));
        fresh.invalidate(&root);
        std::fs::remove_dir_all(&root).unwrap();
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }
}
