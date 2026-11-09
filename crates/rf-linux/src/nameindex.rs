//! A name index of whole folders: searching it takes milliseconds where walking the disk takes
//! seconds to minutes.
//!
//! Linux version of upstream's `rf-win/src/nameindex.rs` (rustyfiles 0.4.0), kept in the same
//! order, function by function, so an upstream change can be carried over by comparing the two
//! files. It isn't shared with rf-win because the parts that differ run through all of it:
//!
//! - **Names are bytes.** The name pool holds each name's raw bytes (Linux names needn't be
//!   UTF-8); paths are rebuilt from them exactly, and matching reads them lossily.
//! - **Case-sensitive folders.** Upstream finds a folder by its case-folded name ("Foo" and
//!   "foo" are one folder on NTFS); here by its exact bytes. Matching a search pattern is still
//!   case-insensitive (Turkish İ/ı included), as in the disk walk.
//! - **Listing** with `read_dir` and the entry's type (no `stat`): hidden = a leading dot,
//!   symlinks (upstream's reparse points) are listed, never entered.
//! - **Mounts** inside a root (a FUSE cloud folder in `~`, a network share, another disk) are
//!   listed but not walked, and marked so a search inside one walks the disk instead.
//! - **Exclusions**: Linux's caches and build folders, matched case-sensitively.
//! - **The folder searched** isn't one of its own hits (as in the disk walk).
//! - **Watching**: [`Index::build_with`] / [`Index::refresh_dirs_with`] call back for each
//!   folder walked, so the indexer can watch exactly the folders it indexes (inotify watches
//!   one folder at a time).
//!
//! **Layout.** One node per file or folder, struct-of-arrays: parent id, name offset/length in
//! one name pool, flags. About 11 bytes plus the name per entry. Node 0 is the root folder
//! itself. Ids follow the walk, so a folder's entries are adjacent.
//!
//! **Building** walks the root in parallel (nice 10, like the search walk). Workers reserve id
//! ranges from a counter and hand their listings to the caller, which lays them out by id once
//! the walk ends.
//!
//! **Searching** scans the name pool on all cores: ASCII names are compared case-insensitively
//! without allocating; names or patterns with other letters go through the same folding the walk
//! search uses. A hit counts only if no folder above it was removed (and, unless hidden files
//! are wanted, none is hidden), so removing a folder never has to touch its contents.
//!
//! **Keeping up** ([`Index::refresh_dirs`]): a changed folder is listed again and compared with
//! its indexed entries by name: gone ones are marked removed, new ones appended (new folders
//! walked), the rest updated. [`Index::compact`] drops removed entries and their contents.
//!
//! **Saved** to a file ([`Index::save`] / [`Index::load`]) so a start doesn't walk again.

use std::collections::{HashMap, HashSet};
use std::ffi::OsStr;
use std::io::{Read, Write};
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use crate::search::{Matcher, lower_priority, virtual_dir};

const NONE: u32 = u32::MAX;
const DIR: u8 = 1;
const HIDDEN: u8 = 2;
/// A symlink: listed, never entered.
const LINK: u8 = 4;
/// A folder whose contents aren't indexed because another file system is mounted there.
const MOUNT: u8 = 8;
const REMOVED: u8 = 128;

/// Called with each folder as the walk reaches it.
pub type OnDir<'a> = Option<&'a (dyn Fn(&Path) + Sync)>;

pub struct Index {
    root: PathBuf,
    parent: Vec<u32>,
    name_off: Vec<u32>,
    name_len: Vec<u16>,
    flags: Vec<u8>,
    names: Vec<u8>,
    /// Folders by (parent, name hash): finding a folder from a path when it changes.
    dirs: HashMap<(u32, u64), u32>,
    removed: usize,
}

/// One folder's listing from a build worker: entries `base..base + lens.len()`.
struct Chunk {
    base: u32,
    parent: u32,
    names: Vec<u8>,
    lens: Vec<u16>,
    flags: Vec<u8>,
}

fn name_key(name: &[u8]) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    name.hash(&mut h);
    h.finish()
}

/// Lists `dir`: (names, lengths, flags) of every entry.
fn list(dir: &Path) -> (Vec<u8>, Vec<u16>, Vec<u8>) {
    let (mut names, mut lens, mut flags) = (Vec::new(), Vec::new(), Vec::new());
    let Ok(rd) = std::fs::read_dir(dir) else { return (names, lens, flags) };
    for de in rd.flatten() {
        let n = de.file_name();
        let b = n.as_bytes();
        // a name over 64 KB doesn't exist (NAME_MAX is 255)
        if b.len() > u16::MAX as usize {
            continue;
        }
        let mut f = 0;
        match de.file_type() {
            Ok(t) if t.is_symlink() => f |= LINK,
            Ok(t) if t.is_dir() => f |= DIR,
            _ => {}
        }
        if b.first() == Some(&b'.') {
            f |= HIDDEN;
        }
        names.extend_from_slice(b);
        lens.push(b.len() as u16);
        flags.push(f);
    }
    (names, lens, flags)
}

/// Mount points other than `/`: their folders are listed, their contents left to the disk
/// walk (another disk, a network share, a FUSE cloud folder, `/proc`…).
fn mount_points() -> HashSet<PathBuf> {
    crate::drives::mounts().into_iter().map(|(p, _, _)| p).filter(|p| p != Path::new("/")).collect()
}

struct Build<'a> {
    next: &'a AtomicU32,
    chunks: &'a Mutex<Vec<Chunk>>,
    cancel: &'a AtomicBool,
    mounts: &'a HashSet<PathBuf>,
    on_dir: OnDir<'a>,
}

/// Folders left out of the index: caches, version-control internals, package stores. They hold
/// most of a home folder's entries and are rarely searched; a search started inside one walks
/// the disk as before. Folders with a `CACHEDIR.TAG` (Cargo's `target/`, ccache…) too.
const SKIP_NAMES: &[&str] = &["node_modules", ".git", "__pycache__", ".cargo", ".rustup", ".gradle", ".cache", ".npm", ".m2"];

/// Folders the user left out (Settings → Search, or a folder's context menu).
static USER_SKIP: std::sync::RwLock<Vec<PathBuf>> = std::sync::RwLock::new(Vec::new());

/// Sets the folders the user leaves out of the index.
pub fn set_excluded(dirs: &[PathBuf]) {
    *USER_SKIP.write().unwrap_or_else(|e| e.into_inner()) = dirs.to_vec();
}

/// The built-in exclusions, for showing them.
pub fn default_excluded_names() -> &'static [&'static str] {
    SKIP_NAMES
}

pub fn skipped(dir: &Path) -> bool {
    let Some(name) = dir.file_name() else { return false };
    if SKIP_NAMES.iter().any(|s| name == *s) {
        return true;
    }
    if USER_SKIP.read().unwrap_or_else(|e| e.into_inner()).iter().any(|u| dir == u) {
        return true;
    }
    // a cache folder says so (Cargo's target/, many tools' caches): https://bford.info/cachedir/
    dir.join("CACHEDIR.TAG").exists()
}

fn walk<'s>(sc: &rayon::Scope<'s>, b: &'s Build<'s>, dir: PathBuf, id: u32) {
    if b.cancel.load(Ordering::Relaxed) {
        return;
    }
    if let Some(f) = b.on_dir {
        f(&dir);
    }
    let (names, lens, mut flags) = list(&dir);
    if lens.is_empty() {
        return;
    }
    let base = b.next.fetch_add(lens.len() as u32, Ordering::Relaxed);
    let mut off = 0usize;
    for (i, (&len, f)) in lens.iter().zip(flags.iter_mut()).enumerate() {
        let name = &names[off..off + len as usize];
        off += len as usize;
        if *f & DIR != 0 {
            let sub = dir.join(OsStr::from_bytes(name));
            if b.mounts.contains(&sub) || virtual_dir(&sub) {
                *f |= MOUNT;
                continue;
            }
            let child = base + i as u32;
            // the folder itself is listed (it can be found by name), its contents aren't
            if !skipped(&sub) {
                sc.spawn(move |sc| walk(sc, b, sub, child));
            }
        }
    }
    b.chunks.lock().unwrap().push(Chunk { base, parent: id, names, lens, flags });
}

fn pool() -> rayon::ThreadPool {
    rayon::ThreadPoolBuilder::new()
        .num_threads(std::thread::available_parallelism().map_or(4, |n| n.get()).clamp(4, 8))
        .thread_name(|i| format!("rf-index-{i}"))
        .start_handler(|_| lower_priority())
        .build()
        .expect("index thread pool")
}

impl Index {
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Entries, removed ones included.
    pub fn len(&self) -> usize {
        self.parent.len()
    }

    pub fn is_empty(&self) -> bool {
        self.parent.len() <= 1
    }

    /// Live entries (not removed).
    pub fn live(&self) -> usize {
        self.parent.len() - self.removed
    }

    /// Bytes held (arrays and names).
    pub fn memory(&self) -> usize {
        self.parent.capacity() * 4 + self.name_off.capacity() * 4 + self.name_len.capacity() * 2 + self.flags.capacity() + self.names.capacity() + self.dirs.capacity() * 24
    }

    fn name(&self, i: u32) -> &[u8] {
        let (o, l) = (self.name_off[i as usize] as usize, self.name_len[i as usize] as usize);
        &self.names[o..o + l]
    }

    /// Walks `root` (blocking; `cancel` stops it, returning what was found so far).
    pub fn build(root: &Path, cancel: &AtomicBool) -> Index {
        Self::build_with(root, cancel, None)
    }

    /// [`Index::build`], calling `on_dir` with each folder whose contents get indexed.
    pub fn build_with(root: &Path, cancel: &AtomicBool, on_dir: OnDir<'_>) -> Index {
        let next = AtomicU32::new(1);
        let chunks = Mutex::new(Vec::new());
        let mounts = mount_points();
        let b = Build { next: &next, chunks: &chunks, cancel, mounts: &mounts, on_dir };
        pool().scope(|sc| walk(sc, &b, root.to_path_buf(), 0));
        let total = next.load(Ordering::Relaxed) as usize;
        let mut chunks = chunks.into_inner().unwrap();
        chunks.sort_unstable_by_key(|c| c.base);
        let mut ix = Index {
            root: root.to_path_buf(),
            parent: vec![NONE; total],
            name_off: vec![0; total],
            name_len: vec![0; total],
            flags: vec![REMOVED; total],
            names: Vec::with_capacity(chunks.iter().map(|c| c.names.len()).sum()),
            dirs: HashMap::new(),
            removed: 0,
        };
        ix.flags[0] = DIR;
        for c in chunks {
            let mut off = 0usize;
            for (k, (&len, &f)) in c.lens.iter().zip(&c.flags).enumerate() {
                let i = c.base as usize + k;
                ix.parent[i] = c.parent;
                ix.name_off[i] = ix.names.len() as u32;
                ix.name_len[i] = len;
                ix.flags[i] = f;
                ix.names.extend_from_slice(&c.names[off..off + len as usize]);
                off += len as usize;
            }
        }
        // ids reserved by a cancelled walk but never filled stay removed
        ix.removed = ix.flags.iter().filter(|&&f| f & REMOVED != 0).count();
        ix.index_dirs();
        ix
    }

    fn index_dirs(&mut self) {
        self.dirs.clear();
        for i in 1..self.parent.len() as u32 {
            let f = self.flags[i as usize];
            if f & DIR != 0 && f & REMOVED == 0 {
                self.dirs.insert((self.parent[i as usize], name_key(self.name(i))), i);
            }
        }
    }

    /// The node of `path` (a folder at or under the root), if indexed.
    fn node_of(&self, path: &Path) -> Option<u32> {
        let rel = path.strip_prefix(&self.root).ok()?;
        let mut id = 0u32;
        for c in rel.components() {
            let std::path::Component::Normal(n) = c else { continue };
            id = *self.dirs.get(&(id, name_key(n.as_bytes())))?;
        }
        Some(id)
    }

    /// Whether `i` and every folder above it are present (and visible, unless `hidden`); and
    /// whether `under` (if given) is one of them.
    fn valid(&self, mut i: u32, hidden: bool, under: Option<u32>) -> bool {
        let mut inside = under.is_none_or(|u| u == 0);
        while i != 0 && i != NONE {
            let f = self.flags[i as usize];
            if f & REMOVED != 0 || (!hidden && f & HIDDEN != 0) {
                return false;
            }
            if Some(i) == under {
                inside = true;
            }
            i = self.parent[i as usize];
        }
        inside && i == 0
    }

    /// The full path of node `i`.
    pub fn path(&self, i: u32) -> PathBuf {
        let mut parts = Vec::new();
        let mut j = i;
        while j != 0 && j != NONE {
            parts.push(self.name(j));
            j = self.parent[j as usize];
        }
        let mut p = self.root.clone();
        for part in parts.iter().rev() {
            p.push(OsStr::from_bytes(part));
        }
        p
    }

    /// Entries under `under` (the root when `None`) whose name matches `pattern` (substring, or
    /// a glob with `*` / `?`), at most `max`, in index order. `None` when `under` isn't indexed.
    pub fn search(&self, pattern: &str, under: Option<&Path>, include_hidden: bool, max: usize) -> Option<Vec<PathBuf>> {
        let under = match under {
            Some(p) => {
                // inside a folder the index leaves out: not ours to answer (the disk is walked)
                if p.ancestors().take_while(|a| a.starts_with(&self.root) && *a != self.root.as_path()).any(skipped) {
                    return None;
                }
                let node = self.node_of(p)?;
                // inside another mount: its contents aren't here either
                let mut j = node;
                while j != 0 && j != NONE {
                    if self.flags[j as usize] & MOUNT != 0 {
                        return None;
                    }
                    j = self.parent[j as usize];
                }
                Some(node)
            }
            None => None,
        };
        let m = Matcher::new(pattern);
        let ascii = pattern.trim().to_ascii_lowercase();
        let fast = !ascii.is_empty() && ascii.is_ascii() && !ascii.contains(['*', '?']);
        let needle = ascii.as_bytes();
        let n = self.parent.len();
        let threads = std::thread::available_parallelism().map_or(4, |t| t.get()).clamp(1, 16);
        let per = n.div_ceil(threads).max(1);
        let found = Mutex::new(Vec::<(usize, Vec<u32>)>::new());
        let stop = AtomicBool::new(false);
        std::thread::scope(|s| {
            for t in 0..threads {
                let (lo, hi) = (t * per, ((t + 1) * per).min(n));
                if lo >= hi {
                    break;
                }
                let (found, stop, m) = (&found, &stop, &m);
                s.spawn(move || {
                    let mut hits = Vec::new();
                    for i in lo.max(1)..hi {
                        if stop.load(Ordering::Relaxed) {
                            break;
                        }
                        if self.flags[i] & REMOVED != 0 {
                            continue;
                        }
                        let (o, l) = (self.name_off[i] as usize, self.name_len[i] as usize);
                        let name = &self.names[o..o + l];
                        let hit = if fast && name.is_ascii() {
                            name.len() >= needle.len() && name.windows(needle.len()).any(|w| w.eq_ignore_ascii_case(needle))
                        } else {
                            m.is_match(&String::from_utf8_lossy(name))
                        };
                        // the folder searched isn't its own hit (as in the disk walk; upstream
                        // returns it)
                        if hit && Some(i as u32) != under && self.valid(i as u32, include_hidden, under) {
                            hits.push(i as u32);
                            if hits.len() >= max {
                                break;
                            }
                        }
                    }
                    found.lock().unwrap().push((lo, hits));
                });
            }
        });
        let mut parts = found.into_inner().unwrap();
        parts.sort_unstable_by_key(|p| p.0);
        Some(parts.into_iter().flat_map(|p| p.1).take(max).map(|i| self.path(i)).collect())
    }

    /// Re-reads `dirs` (folders that changed): entries gone from a folder are marked removed,
    /// new ones appended (new folders walked), the rest keep their place. Folders not in the
    /// index are skipped.
    pub fn refresh_dirs(&mut self, dirs: &[PathBuf], cancel: &AtomicBool) {
        self.refresh_dirs_with(dirs, cancel, None)
    }

    /// [`Index::refresh_dirs`], calling `on_dir` with each new folder walked.
    pub fn refresh_dirs_with(&mut self, dirs: &[PathBuf], cancel: &AtomicBool, on_dir: OnDir<'_>) {
        let nodes: Vec<(u32, &PathBuf)> = dirs.iter().filter_map(|d| self.node_of(d).map(|n| (n, d))).filter(|(n, _)| self.flags[*n as usize] & (MOUNT | REMOVED) == 0).collect();
        if nodes.is_empty() {
            return;
        }
        // their current children, in one pass
        let wanted: HashMap<u32, usize> = nodes.iter().enumerate().map(|(k, (n, _))| (*n, k)).collect();
        let mut children: Vec<Vec<u32>> = vec![Vec::new(); nodes.len()];
        for i in 1..self.parent.len() {
            if self.flags[i] & REMOVED == 0 {
                if let Some(&k) = wanted.get(&self.parent[i]) {
                    children[k].push(i as u32);
                }
            }
        }
        let mounts = mount_points();
        let mut new_dirs = Vec::new();
        for ((node, dir), kids) in nodes.iter().zip(children) {
            let (names, lens, flags) = list(dir);
            let mut now: HashMap<Vec<u8>, u8> = HashMap::new();
            let mut off = 0usize;
            for (&len, &f) in lens.iter().zip(&flags) {
                now.insert(names[off..off + len as usize].to_vec(), f);
                off += len as usize;
            }
            for k in kids {
                let name = self.name(k).to_vec();
                match now.remove(&name) {
                    // a folder keeps its mark (its contents are where they were)
                    Some(f) if (f & DIR) == (self.flags[k as usize] & DIR) => self.flags[k as usize] = f | (self.flags[k as usize] & MOUNT),
                    // a file became a folder or the other way round: a new entry
                    Some(f) => {
                        self.remove(*node, k);
                        now.insert(name, f);
                    }
                    None => self.remove(*node, k),
                }
            }
            for (name, mut f) in now {
                let path = dir.join(OsStr::from_bytes(&name));
                let walk = f & DIR != 0 && !skipped(&path);
                if f & DIR != 0 && (mounts.contains(&path) || virtual_dir(&path)) {
                    f |= MOUNT;
                }
                let id = self.push(*node, &name, f);
                if walk && f & MOUNT == 0 {
                    new_dirs.push((path, id));
                }
            }
        }
        // new folders: walk them and graft their contents on
        for (path, id) in new_dirs {
            if cancel.load(Ordering::Relaxed) {
                break;
            }
            let sub = Index::build_with(&path, cancel, on_dir);
            self.graft(&sub, id);
        }
    }

    fn remove(&mut self, parent: u32, k: u32) {
        self.flags[k as usize] |= REMOVED;
        self.removed += 1;
        if self.flags[k as usize] & DIR != 0 {
            let key = (parent, name_key(self.name(k)));
            if self.dirs.get(&key) == Some(&k) {
                self.dirs.remove(&key);
            }
        }
    }

    fn push(&mut self, parent: u32, name: &[u8], f: u8) -> u32 {
        let id = self.parent.len() as u32;
        self.parent.push(parent);
        self.name_off.push(self.names.len() as u32);
        self.name_len.push(name.len().min(u16::MAX as usize) as u16);
        self.flags.push(f);
        self.names.extend_from_slice(name);
        if f & DIR != 0 && f & REMOVED == 0 {
            self.dirs.insert((parent, name_key(name)), id);
        }
        id
    }

    /// Appends `sub`'s entries (not its root) under node `at`.
    fn graft(&mut self, sub: &Index, at: u32) {
        let base = self.parent.len() as u32 - 1;
        for i in 1..sub.parent.len() as u32 {
            let f = sub.flags[i as usize];
            let p = sub.parent[i as usize];
            let parent = match p {
                0 => at,
                NONE => NONE,
                p => base + p,
            };
            let id = self.push(parent, sub.name(i), f);
            debug_assert_eq!(id, base + i);
            if f & REMOVED != 0 {
                self.removed += 1;
            }
        }
    }

    /// Drops removed entries and everything below them; ids change.
    pub fn compact(&mut self) {
        let n = self.parent.len();
        let mut keep = vec![false; n];
        keep[0] = true;
        // parents come before their children (walk order, appends): one pass decides
        let mut map = vec![NONE; n];
        map[0] = 0;
        let mut out = Index {
            root: self.root.clone(),
            parent: vec![NONE],
            name_off: vec![0],
            name_len: vec![0],
            flags: vec![DIR],
            names: Vec::with_capacity(self.names.len()),
            dirs: HashMap::new(),
            removed: 0,
        };
        for i in 1..n {
            let p = self.parent[i];
            if self.flags[i] & REMOVED != 0 || p == NONE || !keep[p as usize] {
                continue;
            }
            keep[i] = true;
            map[i] = out.push(map[p as usize], self.name(i as u32), self.flags[i]);
        }
        *self = out;
    }

    /// The folders whose contents are indexed (the root first), for watching them.
    pub fn walked_dirs(&self) -> Vec<PathBuf> {
        let mut v = vec![self.root.clone()];
        for i in 1..self.parent.len() as u32 {
            let f = self.flags[i as usize];
            if f & DIR != 0 && f & (REMOVED | MOUNT) == 0 && self.valid(i, true, None) {
                let p = self.path(i);
                if !skipped(&p) {
                    v.push(p);
                }
            }
        }
        v
    }

    // -------------------------------------------------------------- saving

    /// Upstream's format with raw-byte names (not interchangeable with a Windows index).
    const MAGIC: &'static [u8; 8] = b"RFNIXL\0\x01";

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        let tmp = path.with_extension("tmp");
        {
            let mut f = std::io::BufWriter::with_capacity(1 << 20, std::fs::File::create(&tmp)?);
            f.write_all(Self::MAGIC)?;
            let root = self.root.as_os_str().as_bytes();
            f.write_all(&(root.len() as u32).to_le_bytes())?;
            f.write_all(root)?;
            f.write_all(&(self.parent.len() as u64).to_le_bytes())?;
            f.write_all(&(self.names.len() as u64).to_le_bytes())?;
            for v in &self.parent {
                f.write_all(&v.to_le_bytes())?;
            }
            for v in &self.name_off {
                f.write_all(&v.to_le_bytes())?;
            }
            for v in &self.name_len {
                f.write_all(&v.to_le_bytes())?;
            }
            f.write_all(&self.flags)?;
            f.write_all(&self.names)?;
            f.flush()?;
        }
        std::fs::rename(&tmp, path)
    }

    pub fn load(path: &Path) -> std::io::Result<Index> {
        let bad = || std::io::Error::new(std::io::ErrorKind::InvalidData, "not an index file");
        let mut b = Vec::new();
        std::fs::File::open(path)?.read_to_end(&mut b)?;
        let mut p = 0usize;
        let mut take = |n: usize| -> std::io::Result<&[u8]> {
            let s = b.get(p..p.checked_add(n).ok_or_else(bad)?).ok_or_else(bad)?;
            p += n;
            Ok(s)
        };
        if take(8)? != Self::MAGIC {
            return Err(bad());
        }
        let rl = u32::from_le_bytes(take(4)?.try_into().unwrap()) as usize;
        let root = PathBuf::from(OsStr::from_bytes(take(rl)?));
        let n = u64::from_le_bytes(take(8)?.try_into().unwrap()) as usize;
        let nl = u64::from_le_bytes(take(8)?.try_into().unwrap()) as usize;
        let parent: Vec<u32> = take(n.checked_mul(4).ok_or_else(bad)?)?.chunks_exact(4).map(|c| u32::from_le_bytes(c.try_into().unwrap())).collect();
        let name_off: Vec<u32> = take(n * 4)?.chunks_exact(4).map(|c| u32::from_le_bytes(c.try_into().unwrap())).collect();
        let name_len: Vec<u16> = take(n * 2)?.chunks_exact(2).map(|c| u16::from_le_bytes(c.try_into().unwrap())).collect();
        let flags = take(n)?.to_vec();
        let names = take(nl)?.to_vec();
        // every name inside the pool, every parent an earlier-or-valid id
        for i in 0..n {
            if name_off[i] as usize + name_len[i] as usize > names.len() || (parent[i] != NONE && parent[i] as usize >= n) {
                return Err(bad());
            }
        }
        if n == 0 {
            return Err(bad());
        }
        let removed = flags.iter().filter(|&&f| f & REMOVED != 0).count();
        let mut ix = Index { root, parent, name_off, name_len, flags, names, dirs: HashMap::new(), removed };
        ix.index_dirs();
        Ok(ix)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree(tag: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("rf-nameindex-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        for (dir, files) in [("", vec!["Rapor.pdf", "notlar.txt"]), ("Proje", vec!["main.rs", "RAPOR eski.docx"]), ("Proje/Çizimler", vec!["ışık.png", "İkon.svg"])] {
            let d = root.join(dir);
            std::fs::create_dir_all(&d).unwrap();
            for f in files {
                std::fs::write(d.join(f), b"x").unwrap();
            }
        }
        root
    }

    fn names(v: &[PathBuf]) -> Vec<String> {
        let mut n: Vec<String> = v.iter().map(|p| p.file_name().unwrap().to_string_lossy().into_owned()).collect();
        n.sort();
        n
    }

    #[test]
    fn builds_and_finds_case_insensitively() {
        let root = tree("find");
        let ix = Index::build(&root, &AtomicBool::new(false));
        assert_eq!(ix.live(), 9); // the root, 2 folders and 6 files
        assert_eq!(names(&ix.search("rapor", None, false, 100).unwrap()), ["RAPOR eski.docx", "Rapor.pdf"]);
        // Turkish letters fold like the walk search does
        assert_eq!(names(&ix.search("IŞIK", None, false, 100).unwrap()), ["ışık.png"]);
        assert_eq!(names(&ix.search("*.rs", None, false, 100).unwrap()), ["main.rs"]);
        // under a subfolder only
        assert_eq!(names(&ix.search("rapor", Some(&root.join("Proje")), false, 100).unwrap()), ["RAPOR eski.docx"]);
        assert!(ix.search("rapor", Some(&root.join("yok")), false, 100).is_none());
        // the full path comes back
        let p = ix.search("main", None, false, 10).unwrap();
        assert_eq!(p, [root.join("Proje").join("main.rs")]);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn folders_differing_in_case_are_two_folders() {
        let root = tree("case");
        std::fs::create_dir_all(root.join("proje")).unwrap();
        std::fs::write(root.join("proje/küçük.txt"), b"x").unwrap();
        let mut ix = Index::build(&root, &AtomicBool::new(false));
        assert_eq!(names(&ix.search("", Some(&root.join("proje")), false, 100).unwrap()), ["küçük.txt"]);
        assert_eq!(names(&ix.search("main", Some(&root.join("Proje")), false, 100).unwrap()), ["main.rs"]);
        assert!(ix.search("", Some(&root.join("PROJE")), false, 100).is_none());
        // a change in one doesn't touch the other
        std::fs::write(root.join("proje/yeni.txt"), b"x").unwrap();
        ix.refresh_dirs(&[root.join("proje")], &AtomicBool::new(false));
        assert_eq!(names(&ix.search("", Some(&root.join("proje")), false, 100).unwrap()), ["küçük.txt", "yeni.txt"]);
        assert_eq!(names(&ix.search("", Some(&root.join("Proje")), false, 100).unwrap()).len(), 5);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn names_that_are_not_utf8_keep_their_bytes() {
        let root = tree("bytes");
        let odd = OsStr::from_bytes(b"rapor-\xff\xfe.txt");
        std::fs::write(root.join(odd), b"x").unwrap();
        let ix = Index::build(&root, &AtomicBool::new(false));
        let hits = ix.search("rapor-", None, false, 10).unwrap();
        assert_eq!(hits, [root.join(odd)]);
        assert!(hits[0].exists());
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn follows_changes_and_survives_saving() {
        let root = tree("change");
        let mut ix = Index::build(&root, &AtomicBool::new(false));
        std::fs::remove_file(root.join("notlar.txt")).unwrap();
        std::fs::write(root.join("yeni rapor.md"), b"x").unwrap();
        std::fs::create_dir_all(root.join("Arşiv/2026")).unwrap();
        std::fs::write(root.join("Arşiv/2026/eski rapor.pdf"), b"x").unwrap();
        std::fs::remove_dir_all(root.join("Proje")).unwrap();
        ix.refresh_dirs(std::slice::from_ref(&root), &AtomicBool::new(false));
        assert!(ix.search("notlar", None, false, 10).unwrap().is_empty());
        // the removed folder's contents are gone without being touched one by one
        assert!(ix.search("main", None, false, 10).unwrap().is_empty());
        assert_eq!(names(&ix.search("rapor", None, false, 10).unwrap()), ["Rapor.pdf", "eski rapor.pdf", "yeni rapor.md"]);
        let file = root.with_extension("rfidx");
        ix.save(&file).unwrap();
        let mut back = Index::load(&file).unwrap();
        assert_eq!(names(&back.search("rapor", None, false, 10).unwrap()), ["Rapor.pdf", "eski rapor.pdf", "yeni rapor.md"]);
        back.compact();
        assert_eq!(back.removed, 0);
        assert_eq!(names(&back.search("rapor", None, false, 10).unwrap()), ["Rapor.pdf", "eski rapor.pdf", "yeni rapor.md"]);
        assert_eq!(back.search("eski", Some(&root.join("Arşiv")), false, 10).unwrap(), [root.join("Arşiv/2026/eski rapor.pdf")]);
        std::fs::remove_file(&file).unwrap();
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_renamed_folder_moves_its_contents() {
        let root = tree("rename");
        let mut ix = Index::build(&root, &AtomicBool::new(false));
        std::fs::rename(root.join("Proje"), root.join("Proje 2")).unwrap();
        ix.refresh_dirs(std::slice::from_ref(&root), &AtomicBool::new(false));
        assert_eq!(ix.search("main", None, false, 10).unwrap(), [root.join("Proje 2/main.rs")]);
        assert!(ix.search("", Some(&root.join("Proje")), false, 10).is_none());
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn caches_are_left_out_but_still_searchable_by_walking() {
        let root = tree("skip");
        std::fs::create_dir_all(root.join("web/node_modules/lodash")).unwrap();
        std::fs::write(root.join("web/node_modules/lodash/rapor.js"), b"x").unwrap();
        std::fs::create_dir_all(root.join("build")).unwrap();
        std::fs::write(root.join("build/CACHEDIR.TAG"), b"Signature: 8a477f597d28d172789f06886806bc55").unwrap();
        std::fs::write(root.join("build/rapor.o"), b"x").unwrap();
        // the user's own exclusion, by full path
        std::fs::create_dir_all(root.join("arsiv")).unwrap();
        std::fs::write(root.join("arsiv/rapor eski.zip"), b"x").unwrap();
        set_excluded(&[root.join("arsiv")]);
        let ix = Index::build(&root, &AtomicBool::new(false));
        set_excluded(&[]);
        // the folders are found by name, their contents aren't indexed
        assert_eq!(names(&ix.search("node_modules", None, false, 10).unwrap()), ["node_modules"]);
        assert_eq!(names(&ix.search("rapor", None, false, 10).unwrap()), ["RAPOR eski.docx", "Rapor.pdf"]);
        // a search started inside one is left to the disk walk
        assert!(ix.search("rapor", Some(&root.join("web/node_modules")), false, 10).is_none());
        assert!(ix.search("rapor", Some(&root.join("build")), false, 10).is_none());
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn dot_entries_need_asking_and_links_are_not_entered() {
        let root = tree("hidden");
        let secret = root.join(".gizli");
        std::fs::create_dir_all(&secret).unwrap();
        std::fs::write(secret.join("rapor gizli.txt"), b"x").unwrap();
        std::os::unix::fs::symlink(&root, root.join("Proje/geri")).unwrap();
        let ix = Index::build(&root, &AtomicBool::new(false));
        assert!(!names(&ix.search("gizli", None, false, 10).unwrap()).contains(&"rapor gizli.txt".to_string()));
        assert!(names(&ix.search("gizli", None, true, 10).unwrap()).contains(&"rapor gizli.txt".to_string()));
        // the link is a name, not a second copy of the tree
        assert_eq!(names(&ix.search("main", None, false, 10).unwrap()), ["main.rs"]);
        assert_eq!(names(&ix.search("geri", None, false, 10).unwrap()), ["geri"]);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn calls_back_for_every_walked_folder() {
        let root = tree("ondir");
        std::fs::create_dir_all(root.join("node_modules/x")).unwrap();
        let seen = Mutex::new(Vec::new());
        let f = |p: &Path| seen.lock().unwrap().push(p.to_path_buf());
        let mut ix = Index::build_with(&root, &AtomicBool::new(false), Some(&f));
        let mut v = seen.lock().unwrap().clone();
        v.sort();
        assert_eq!(v, [root.clone(), root.join("Proje"), root.join("Proje/Çizimler")]);
        let mut w = ix.walked_dirs();
        w.sort();
        assert_eq!(w, v);
        seen.lock().unwrap().clear();
        std::fs::create_dir_all(root.join("Yeni/Alt")).unwrap();
        ix.refresh_dirs_with(std::slice::from_ref(&root), &AtomicBool::new(false), Some(&f));
        let mut v = seen.lock().unwrap().clone();
        v.sort();
        assert_eq!(v, [root.join("Yeni"), root.join("Yeni/Alt")]);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_damaged_file_is_refused() {
        let root = tree("bad");
        let ix = Index::build(&root, &AtomicBool::new(false));
        let file = root.with_extension("rfidx");
        ix.save(&file).unwrap();
        let b = std::fs::read(&file).unwrap();
        std::fs::write(&file, &b[..b.len() / 2]).unwrap();
        assert!(Index::load(&file).is_err());
        std::fs::write(&file, b"RFNIX\0\0\x01rest").unwrap();
        assert!(Index::load(&file).is_err());
        std::fs::remove_file(&file).unwrap();
        std::fs::remove_dir_all(&root).unwrap();
    }
}

impl Index {
    /// The `n` folders holding the most entries below them (whole subtrees), largest first —
    /// for deciding what is worth indexing.
    pub fn heaviest(&self, n: usize, depth_limit: usize) -> Vec<(PathBuf, usize)> {
        let len = self.parent.len();
        let mut count = vec![0usize; len];
        // children come after their parents: one backwards pass sums subtrees
        for i in (1..len).rev() {
            if self.flags[i] & REMOVED == 0 {
                let p = self.parent[i];
                if p != NONE {
                    count[p as usize] += count[i] + 1;
                }
            }
        }
        let depth = |mut i: u32| {
            let mut d = 0;
            while i != 0 && i != NONE {
                d += 1;
                i = self.parent[i as usize];
            }
            d
        };
        let mut v: Vec<(u32, usize)> = (1..len as u32).filter(|&i| self.flags[i as usize] & DIR != 0 && depth(i) <= depth_limit).map(|i| (i, count[i as usize])).collect();
        v.sort_unstable_by(|a, b| b.1.cmp(&a.1));
        v.into_iter().take(n).map(|(i, c)| (self.path(i), c)).collect()
    }
}
