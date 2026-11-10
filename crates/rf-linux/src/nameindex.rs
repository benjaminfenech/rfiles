//! A name index of whole folders: searching it takes milliseconds where walking the disk takes
//! seconds to minutes.
//!
//! Linux version of upstream's `rf-win/src/nameindex.rs` (rustyfiles 0.4.0, with the name
//! table of upstream `c8e1e57`), kept in the same order, function by function, so an upstream
//! change can be carried over by comparing the two files. It isn't shared with rf-win because
//! the parts that differ run through all of it:
//!
//! - **Names are bytes.** The name table holds each name's raw bytes (Linux names needn't be
//!   UTF-8); paths are rebuilt from them exactly, and matching reads them lossily.
//! - **Case-sensitive folders.** Upstream finds a folder by its case-folded name ("Foo" and
//!   "foo" are one folder on NTFS); here by its exact bytes, that is by its name id. Matching a
//!   search pattern is still case-insensitive (Turkish İ/ı included), as in the disk walk.
//! - **Listing** with `read_dir` and the entry's type (no `stat`): hidden = a leading dot,
//!   symlinks (upstream's reparse points) are listed, never entered.
//! - **Mounts** inside a root (a FUSE cloud folder in `~`, a network share, another disk) are
//!   listed but not walked, and marked so a search inside one walks the disk instead.
//! - **Exclusions**: Linux's caches and build folders, matched case-sensitively.
//! - **The folder searched** isn't one of its own hits (as in the disk walk).
//! - **Watching**: [`Index::build_with`] / [`Index::refresh_dirs_with`] call back for each
//!   folder walked, so the indexer can watch exactly the folders it indexes (inotify watches
//!   one folder at a time).
//! - **No file ids.** Upstream also keeps each folder's NTFS file id for following the change
//!   journal (`5bc4945`); Linux follows changes with inotify (`indexer.rs`) and keeps none.
//!
//! **Layout.** One node per file or folder, struct-of-arrays: parent id, name id, flags: 9
//! bytes per entry. Names are kept once each in a name table, as most of them repeat (a home
//! folder's `README.md`, `index.js`, `mod.rs`…). Node 0 is the root folder itself. Ids follow
//! the walk, so a folder's entries are adjacent.
//!
//! **Building** walks the root in parallel (nice 10, like the search walk). Workers reserve id
//! ranges from a counter, add each folder's names to the shared name table as they list it, and
//! hand their listings (name ids) to the caller, which lays them out by id once the walk ends.
//!
//! **Searching** matches each different name once, then picks the entries carrying a matching
//! one, both on all cores: ASCII names are compared case-insensitively without allocating; names
//! or patterns with other letters go through the same folding the walk search uses. A hit counts
//! only if no folder above it was removed (and, unless hidden files are wanted, none is hidden),
//! so removing a folder never has to touch its contents.
//!
//! **Keeping up** ([`Index::refresh_dirs`]): a changed folder is listed again and compared with
//! its indexed entries by name: gone ones are marked removed, new ones appended (new folders
//! walked), the rest updated. [`Index::compact`] drops removed entries and their contents.
//!
//! **Saved** to a file ([`Index::save`] / [`Index::load`]) so a start is searchable at once.
//! Format 2 is read straight into the arrays; format 1 files (a name per entry) convert on
//! loading.

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

/// A hash set of ids whose keys live elsewhere (in the index arrays): 4 bytes a slot instead of
/// a stored key. Open addressing with linear probing; the caller hashes and compares through
/// the ids, and gives the hash of any id when slots move (growing, removing).
struct IdTable {
    slots: Vec<u32>,
    len: usize,
}

impl IdTable {
    fn new() -> Self {
        IdTable { slots: Vec::new(), len: 0 }
    }

    fn mask(&self) -> usize {
        self.slots.len() - 1
    }

    /// The id stored for `hash` that `is` accepts.
    fn find(&self, hash: u64, is: impl Fn(u32) -> bool) -> Option<u32> {
        if self.slots.is_empty() {
            return None;
        }
        let mut at = hash as usize & self.mask();
        loop {
            match self.slots[at] {
                NONE => return None,
                id if is(id) => return Some(id),
                _ => at = (at + 1) & self.mask(),
            }
        }
    }

    /// Adds `id` (not already in) under `hash`; `hash_of` rehashes the others when growing.
    fn insert(&mut self, hash: u64, id: u32, hash_of: impl Fn(u32) -> u64) {
        if (self.len + 1) * 8 > self.slots.len() * 7 {
            self.grow((self.len + 1).max(16), &hash_of);
        }
        let mut at = hash as usize & self.mask();
        while self.slots[at] != NONE {
            at = (at + 1) & self.mask();
        }
        self.slots[at] = id;
        self.len += 1;
    }

    /// Room for `n` ids (power-of-two slots, at most 7/8 full).
    fn grow(&mut self, n: usize, hash_of: &impl Fn(u32) -> u64) {
        let size = (n * 8 / 7 + 1).next_power_of_two().max(16);
        if size <= self.slots.len() {
            return;
        }
        let old = std::mem::replace(&mut self.slots, vec![NONE; size]);
        let mask = size - 1;
        for id in old.into_iter().filter(|&id| id != NONE) {
            let mut at = hash_of(id) as usize & mask;
            while self.slots[at] != NONE {
                at = (at + 1) & mask;
            }
            self.slots[at] = id;
        }
    }

    /// Takes `id` out (found under `hash`), moving later entries of its run back so lookups
    /// never stop at a hole.
    fn remove(&mut self, hash: u64, id: u32, hash_of: impl Fn(u32) -> u64) {
        if self.slots.is_empty() {
            return;
        }
        let mask = self.mask();
        let mut at = hash as usize & mask;
        loop {
            match self.slots[at] {
                NONE => return,
                x if x == id => break,
                _ => at = (at + 1) & mask,
            }
        }
        self.slots[at] = NONE;
        self.len -= 1;
        let mut hole = at;
        let mut next = (at + 1) & mask;
        while self.slots[next] != NONE {
            let home = hash_of(self.slots[next]) as usize & mask;
            // the entry may fill the hole if its home isn't between the hole and its place
            let between = if hole <= next { hole < home && home <= next } else { hole < home || home <= next };
            if !between {
                self.slots[hole] = self.slots[next];
                self.slots[next] = NONE;
                hole = next;
            }
            next = (next + 1) & mask;
        }
    }

    fn clear(&mut self) {
        self.slots = Vec::new();
        self.len = 0;
    }

    fn memory(&self) -> usize {
        self.slots.capacity() * 4
    }
}

pub struct Index {
    root: PathBuf,
    parent: Vec<u32>,
    /// Each entry's name, an id into `names`.
    name_id: Vec<u32>,
    flags: Vec<u8>,
    names: Names,
    /// Folders by (parent, name id): finding a folder from a path when it changes.
    dirs: IdTable,
    removed: usize,
}

/// Every different name once: raw bytes in one pool, name `k` is `pool[off[k]..off[k + 1]]`.
/// Ids are unique (two ids never hold the same bytes), so a folder's exact name is its id.
struct Names {
    pool: Vec<u8>,
    off: Vec<u32>,
    /// Name ids by the hash of their bytes.
    lookup: IdTable,
}

impl Names {
    fn new() -> Self {
        Names { pool: Vec::new(), off: vec![0], lookup: IdTable::new() }
    }

    fn len(&self) -> usize {
        self.off.len() - 1
    }

    fn get(&self, k: u32) -> &[u8] {
        &self.pool[self.off[k as usize] as usize..self.off[k as usize + 1] as usize]
    }

    fn hash(name: &[u8]) -> u64 {
        // FNV-1a: short names, no allocation, good spread over the table's low bits once mixed
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        for &b in name {
            h = (h ^ b as u64).wrapping_mul(0x0000_0100_0000_01b3);
        }
        h ^ (h >> 29)
    }

    /// The id of `name`, if held.
    fn find(&self, name: &[u8]) -> Option<u32> {
        self.lookup.find(Self::hash(name), |k| self.get(k) == name)
    }

    /// The id of `name`, added if new.
    fn intern(&mut self, name: &[u8]) -> u32 {
        self.intern_hashed(name, Self::hash(name))
    }

    /// [`Self::intern`] with `name`'s hash already worked out (outside the walk's lock).
    fn intern_hashed(&mut self, name: &[u8], h: u64) -> u32 {
        let (pool, off) = (&self.pool, &self.off);
        let get = |k: u32| &pool[off[k as usize] as usize..off[k as usize + 1] as usize];
        if let Some(k) = self.lookup.find(h, |k| get(k) == name) {
            return k;
        }
        let k = self.len() as u32;
        self.pool.extend_from_slice(name);
        self.off.push(self.pool.len() as u32);
        let (pool, off) = (&self.pool, &self.off);
        self.lookup.insert(h, k, |j| Self::hash(&pool[off[j as usize] as usize..off[j as usize + 1] as usize]));
        k
    }

    /// The lookup of names read from a file. A name stored twice keeps its first id: the
    /// returned map (old id → id kept) is given only then.
    fn rebuild_lookup(&mut self) -> Option<Vec<u32>> {
        let (pool, off) = (&self.pool, &self.off);
        let get = |k: u32| &pool[off[k as usize] as usize..off[k as usize + 1] as usize];
        let mut t = IdTable::new();
        t.grow(self.off.len() - 1, &|j| Self::hash(get(j)));
        let mut remap: Option<Vec<u32>> = None;
        for k in 0..(self.off.len() - 1) as u32 {
            let name = get(k);
            let h = Self::hash(name);
            match t.find(h, |j| get(j) == name) {
                None => t.insert(h, k, |j| Self::hash(get(j))),
                // ids must be unique: folders are found by them
                Some(first) => remap.get_or_insert_with(|| (0..(off.len() - 1) as u32).collect())[k as usize] = first,
            }
        }
        self.lookup = t;
        remap
    }

    fn memory(&self) -> usize {
        self.pool.capacity() + self.off.capacity() * 4 + self.lookup.memory()
    }

    fn shrink(&mut self) {
        self.pool.shrink_to_fit();
        self.off.shrink_to_fit();
    }
}

/// One folder's listing from a build worker: entries `base..base + ids.len()`, their names
/// already in the shared name table (raw names of a whole tree would be held twice).
struct Chunk {
    base: u32,
    parent: u32,
    ids: Vec<u32>,
    flags: Vec<u8>,
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
    names: &'a Mutex<Names>,
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
    let ids: Vec<u32> = {
        // hashed before the lock (8 workers share it), then one lock per folder
        let mut off = 0usize;
        let spans: Vec<(usize, usize, u64)> = lens
            .iter()
            .map(|&len| {
                let (o, e) = (off, off + len as usize);
                off = e;
                (o, e, Names::hash(&names[o..e]))
            })
            .collect();
        let mut table = b.names.lock().unwrap();
        spans.iter().map(|&(o, e, h)| table.intern_hashed(&names[o..e], h)).collect()
    };
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
    b.chunks.lock().unwrap().push(Chunk { base, parent: id, ids, flags });
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

    /// Bytes held (arrays, names and lookups). Upstream's figure also counts its folder file
    /// ids (`b593628`); Linux keeps none.
    pub fn memory(&self) -> usize {
        self.parent.capacity() * 4 + self.name_id.capacity() * 4 + self.flags.capacity() + self.names.memory() + self.dirs.memory()
    }

    /// Different names held (each stored once).
    pub fn distinct_names(&self) -> usize {
        self.names.len()
    }

    fn name(&self, i: u32) -> &[u8] {
        self.names.get(self.name_id[i as usize])
    }

    /// Gives back what the arrays reserved for growing (after a build or a compaction).
    fn shrink(&mut self) {
        self.parent.shrink_to_fit();
        self.name_id.shrink_to_fit();
        self.flags.shrink_to_fit();
        self.names.shrink();
    }

    fn empty(root: PathBuf) -> Index {
        let mut names = Names::new();
        names.intern(b"");
        Index { root, parent: vec![NONE], name_id: vec![0], flags: vec![DIR], names, dirs: IdTable::new(), removed: 0 }
    }

    /// Walks `root` (blocking; `cancel` stops it, returning what was found so far).
    pub fn build(root: &Path, cancel: &AtomicBool) -> Index {
        Self::build_with(root, cancel, None)
    }

    /// [`Index::build`], calling `on_dir` with each folder whose contents get indexed.
    pub fn build_with(root: &Path, cancel: &AtomicBool, on_dir: OnDir<'_>) -> Index {
        let next = AtomicU32::new(1);
        let chunks = Mutex::new(Vec::new());
        let mut table = Names::new();
        table.intern(b"");
        let names = Mutex::new(table);
        let mounts = mount_points();
        let b = Build { next: &next, chunks: &chunks, names: &names, cancel, mounts: &mounts, on_dir };
        pool().scope(|sc| walk(sc, &b, root.to_path_buf(), 0));
        let total = next.load(Ordering::Relaxed) as usize;
        let mut chunks = chunks.into_inner().unwrap();
        chunks.sort_unstable_by_key(|c| c.base);
        let mut ix = Index::empty(root.to_path_buf());
        ix.names = names.into_inner().unwrap();
        ix.parent.resize(total, NONE);
        ix.name_id.resize(total, 0);
        ix.flags.resize(total, REMOVED);
        // each chunk's listing goes as soon as it is laid out
        for c in chunks {
            for (k, (&id, &f)) in c.ids.iter().zip(&c.flags).enumerate() {
                let i = c.base as usize + k;
                ix.parent[i] = c.parent;
                ix.name_id[i] = id;
                ix.flags[i] = f;
            }
        }
        // ids reserved by a cancelled walk but never filled stay removed
        ix.removed = ix.flags.iter().filter(|&&f| f & REMOVED != 0).count();
        ix.index_dirs();
        ix.shrink();
        ix
    }

    /// The key a folder is found by: its parent and its name (exact bytes: its name id).
    fn dir_hash(parent: u32, name_id: u32) -> u64 {
        ((name_id as u64).wrapping_mul(0xff51_afd7_ed55_8ccd) ^ (parent as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15)).rotate_left(17)
    }

    fn dir_hash_of(&self, i: u32) -> u64 {
        Self::dir_hash(self.parent[i as usize], self.name_id[i as usize])
    }

    fn index_dirs(&mut self) {
        self.dirs.clear();
        let dirs: Vec<u32> = (1..self.parent.len() as u32).filter(|&i| self.flags[i as usize] & (DIR | REMOVED) == DIR).collect();
        let mut t = IdTable::new();
        t.grow(dirs.len(), &|i| self.dir_hash_of(i));
        // the table is big enough: no rehashing while filling it
        for &i in &dirs {
            t.insert(self.dir_hash_of(i), i, |_| unreachable!("sized for every folder"));
        }
        self.dirs = t;
    }

    /// The folder named `name_id` directly in `parent`.
    fn child_dir(&self, parent: u32, name_id: u32) -> Option<u32> {
        self.dirs.find(Self::dir_hash(parent, name_id), |i| self.parent[i as usize] == parent && self.name_id[i as usize] == name_id)
    }

    /// The node of `path` (a folder at or under the root), if indexed.
    fn node_of(&self, path: &Path) -> Option<u32> {
        let rel = path.strip_prefix(&self.root).ok()?;
        let mut id = 0u32;
        for c in rel.components() {
            let std::path::Component::Normal(n) = c else { continue };
            id = self.child_dir(id, self.names.find(n.as_bytes())?)?;
        }
        Some(id)
    }

    /// Whether `i` and every folder above it are present (and visible, unless `hidden`); and
    /// whether `under` (if given) is one of them.
    fn valid(&self, mut i: u32, hidden: bool, under: Option<u32>) -> bool {
        let mut inside = under.is_none_or(|u| u == 0);
        // hidden folders count below the search's folder only: searching inside a hidden folder
        // (one the user opened) finds what is in it
        let mut above_under = false;
        while i != 0 && i != NONE {
            let f = self.flags[i as usize];
            if Some(i) == under {
                inside = true;
                above_under = true;
            }
            if f & REMOVED != 0 || (!hidden && !above_under && f & HIDDEN != 0) {
                return false;
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

    /// The node `under` names (`None` inside: the root); `None` outside: a folder the index
    /// leaves out, another mount, or a folder it doesn't have (the disk is walked then).
    fn under_node(&self, under: Option<&Path>) -> Option<Option<u32>> {
        Some(match under {
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
        })
    }

    /// Which different names match `pattern` (a bit per name id), and whether any does.
    fn name_matches(&self, pattern: &str) -> (Vec<u64>, bool) {
        let m = Matcher::new(pattern);
        let ascii = pattern.trim().to_ascii_lowercase();
        let fast = !ascii.is_empty() && ascii.is_ascii() && !ascii.contains(['*', '?']);
        let needle = ascii.as_bytes();
        let threads = std::thread::available_parallelism().map_or(4, |t| t.get()).clamp(1, 16);
        // each different name once: which ones match (a bit per name)
        let names = self.names.len();
        let mut matched = vec![0u64; names.div_ceil(64)];
        let any = AtomicBool::new(false);
        // whole 64-bit words per thread: each owns its part of the bit set
        let words_per = matched.len().div_ceil(threads).max(1);
        std::thread::scope(|s| {
            for (t, out) in matched.chunks_mut(words_per).enumerate() {
                let (m, any) = (&m, &any);
                s.spawn(move || {
                    let lo = t * words_per * 64;
                    for (w, word) in out.iter_mut().enumerate() {
                        for b in 0..64 {
                            let k = lo + w * 64 + b;
                            if k >= names {
                                break;
                            }
                            let name = self.names.get(k as u32);
                            let hit = if fast && name.is_ascii() {
                                name.len() >= needle.len() && name.windows(needle.len()).any(|w| w.eq_ignore_ascii_case(needle))
                            } else {
                                m.is_match(&String::from_utf8_lossy(name))
                            };
                            if hit {
                                *word |= 1 << b;
                            }
                        }
                    }
                    if out.iter().any(|&w| w != 0) {
                        any.store(true, Ordering::Relaxed);
                    }
                });
            }
        });
        (matched, any.load(Ordering::Relaxed))
    }

    /// Entries under `under` (the root when `None`) whose name matches `pattern` (substring, or
    /// a glob with `*` / `?`), at most `max`, in index order. `None` when `under` isn't indexed.
    pub fn search(&self, pattern: &str, under: Option<&Path>, include_hidden: bool, max: usize) -> Option<Vec<PathBuf>> {
        let under = self.under_node(under)?;
        let (matched, any) = self.name_matches(pattern);
        if !any {
            return Some(Vec::new());
        }
        let threads = std::thread::available_parallelism().map_or(4, |t| t.get()).clamp(1, 16);
        let matched = &matched;
        // then the entries carrying one of them
        let n = self.parent.len();
        let per = n.div_ceil(threads).max(1);
        let found = Mutex::new(Vec::<(usize, Vec<u32>)>::new());
        let stop = AtomicBool::new(false);
        std::thread::scope(|s| {
            for t in 0..threads {
                let (lo, hi) = (t * per, ((t + 1) * per).min(n));
                if lo >= hi {
                    break;
                }
                let (found, stop) = (&found, &stop);
                s.spawn(move || {
                    let mut hits = Vec::new();
                    for i in lo.max(1)..hi {
                        if stop.load(Ordering::Relaxed) {
                            break;
                        }
                        if self.flags[i] & REMOVED != 0 {
                            continue;
                        }
                        let k = self.name_id[i] as usize;
                        let hit = matched[k / 64] & (1 << (k % 64)) != 0;
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

    /// Like [`Self::search`], but the best matches first whatever their place in the index:
    /// `rank` scores a matching name (None: not wanted), the best names are kept, and their
    /// entries come back best first (shallower paths first among equals), at most `max`.
    pub fn search_ranked(&self, pattern: &str, under: Option<&Path>, include_hidden: bool, max: usize, rank: &(dyn Fn(&str) -> Option<i32> + Sync)) -> Option<Vec<(i32, PathBuf)>> {
        let under = self.under_node(under)?;
        let (matched, any) = self.name_matches(pattern);
        if !any {
            return Some(Vec::new());
        }
        // score the matching names, keep the best few hundred (a common fragment matches tens
        // of thousands; the best are what is shown)
        const KEEP_NAMES: usize = 600;
        let mut scored: Vec<(i32, u32)> = Vec::new();
        for (w, &word) in matched.iter().enumerate() {
            let mut bits = word;
            while bits != 0 {
                let b = bits.trailing_zeros() as usize;
                bits &= bits - 1;
                let k = (w * 64 + b) as u32;
                if let Some(sc) = rank(&String::from_utf8_lossy(self.names.get(k))) {
                    scored.push((sc, k));
                }
            }
        }
        if scored.is_empty() {
            return Some(Vec::new());
        }
        scored.sort_unstable_by(|a, b| b.0.cmp(&a.0));
        scored.truncate(KEEP_NAMES);
        let score_of: HashMap<u32, i32> = scored.iter().map(|&(sc, k)| (k, sc)).collect();
        let threads = std::thread::available_parallelism().map_or(4, |t| t.get()).clamp(1, 16);
        let n = self.parent.len();
        let per = n.div_ceil(threads).max(1);
        let found = Mutex::new(Vec::<(i32, u32, u32)>::new());
        std::thread::scope(|s| {
            for t in 0..threads {
                let (lo, hi) = (t * per, ((t + 1) * per).min(n));
                if lo >= hi {
                    break;
                }
                let (found, score_of) = (&found, &score_of);
                s.spawn(move || {
                    let mut hits = Vec::new();
                    for i in lo.max(1)..hi {
                        if self.flags[i] & REMOVED != 0 {
                            continue;
                        }
                        let Some(&sc) = score_of.get(&self.name_id[i]) else { continue };
                        if self.valid(i as u32, include_hidden, under) {
                            hits.push((sc, self.depth(i as u32), i as u32));
                        }
                    }
                    found.lock().unwrap().extend(hits);
                });
            }
        });
        let mut hits = found.into_inner().unwrap();
        hits.sort_unstable_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)).then(a.2.cmp(&b.2)));
        Some(hits.into_iter().take(max).map(|(sc, _, i)| (sc, self.path(i))).collect())
    }

    /// How many folders above `i` (the root's children: 1).
    fn depth(&self, mut i: u32) -> u32 {
        let mut d = 0;
        while i != 0 && i != NONE {
            d += 1;
            i = self.parent[i as usize];
        }
        d
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
                        self.remove(k);
                        now.insert(name, f);
                    }
                    None => self.remove(k),
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

    fn remove(&mut self, k: u32) {
        self.flags[k as usize] |= REMOVED;
        self.removed += 1;
        if self.flags[k as usize] & DIR != 0 {
            let h = self.dir_hash_of(k);
            let (parent_of, name_id) = (&self.parent, &self.name_id);
            self.dirs.remove(h, k, |i| Self::dir_hash(parent_of[i as usize], name_id[i as usize]));
        }
    }

    fn push(&mut self, parent: u32, name: &[u8], f: u8) -> u32 {
        let k = self.names.intern(name);
        self.push_id(parent, k, f)
    }

    /// [`Self::push`] with a name already in this index's table.
    fn push_id(&mut self, parent: u32, k: u32, f: u8) -> u32 {
        let id = self.parent.len() as u32;
        self.parent.push(parent);
        self.name_id.push(k);
        self.flags.push(f);
        if f & DIR != 0 && f & REMOVED == 0 {
            let h = self.dir_hash_of(id);
            let (parent_of, name_id) = (&self.parent, &self.name_id);
            self.dirs.insert(h, id, |i| Self::dir_hash(parent_of[i as usize], name_id[i as usize]));
        }
        id
    }

    /// Appends `sub`'s entries (not its root) under node `at`.
    fn graft(&mut self, sub: &Index, at: u32) {
        let base = self.parent.len() as u32 - 1;
        // `sub` has its own name table: each of its names is found in ours once
        let mut ids = vec![NONE; sub.names.len()];
        for i in 1..sub.parent.len() as u32 {
            let f = sub.flags[i as usize];
            let p = sub.parent[i as usize];
            let parent = match p {
                0 => at,
                NONE => NONE,
                p => base + p,
            };
            let s = sub.name_id[i as usize] as usize;
            if ids[s] == NONE {
                ids[s] = self.names.intern(sub.names.get(s as u32));
            }
            let id = self.push_id(parent, ids[s], f);
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
        // the name table stays (a name no longer used costs a few bytes)
        let names = std::mem::replace(&mut self.names, Names::new());
        let mut out = Index { root: self.root.clone(), parent: vec![NONE], name_id: vec![0], flags: vec![DIR], names, dirs: IdTable::new(), removed: 0 };
        for i in 1..n {
            let p = self.parent[i];
            if self.flags[i] & REMOVED != 0 || p == NONE || !keep[p as usize] {
                continue;
            }
            keep[i] = true;
            map[i] = out.push_id(map[p as usize], self.name_id[i], self.flags[i]);
        }
        out.shrink();
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
    /// Version 2: entries (parent, name id, flags), then the name table. Version 1 files (a
    /// name per entry) are read and converted.
    const MAGIC: &'static [u8; 8] = b"RFNIXL\0\x02";
    const MAGIC_V1: &'static [u8; 8] = b"RFNIXL\0\x01";

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
            f.write_all(&(self.names.pool.len() as u64).to_le_bytes())?;
            write_u32s(&mut f, &self.parent)?;
            write_u32s(&mut f, &self.name_id)?;
            f.write_all(&self.flags)?;
            write_u32s(&mut f, &self.names.off)?;
            f.write_all(&self.names.pool)?;
            f.flush()?;
        }
        std::fs::rename(&tmp, path)
    }

    /// Reads an index file straight into its arrays (no second copy of the file in memory).
    pub fn load(path: &Path) -> std::io::Result<Index> {
        let bad = || std::io::Error::new(std::io::ErrorKind::InvalidData, "not an index file");
        let file = std::fs::File::open(path)?;
        let size = file.metadata()?.len() as usize;
        let mut f = std::io::BufReader::with_capacity(1 << 20, file);
        let mut magic = [0u8; 8];
        f.read_exact(&mut magic).map_err(|_| bad())?;
        let v1 = &magic == Self::MAGIC_V1;
        if !v1 && &magic != Self::MAGIC {
            return Err(bad());
        }
        let rl = read_u32(&mut f)? as usize;
        // PATH_MAX
        if rl > 4096 {
            return Err(bad());
        }
        let mut root = vec![0u8; rl];
        f.read_exact(&mut root)?;
        let root = PathBuf::from(OsStr::from_bytes(&root));
        let n = read_u64(&mut f)? as usize;
        if n == 0 {
            return Err(bad());
        }
        if v1 {
            let nl = read_u64(&mut f)? as usize;
            if n.checked_mul(11).and_then(|x| x.checked_add(nl)).is_none_or(|x| x > size) {
                return Err(bad());
            }
            return Self::load_v1(&mut f, root, n, nl).map_err(|_| bad());
        }
        let k = read_u64(&mut f)? as usize;
        let pl = read_u64(&mut f)? as usize;
        // sizes from the header must fit the file before anything that big is allocated
        if n.checked_mul(9).and_then(|x| x.checked_add(k.checked_add(1)?.checked_mul(4)?)).and_then(|x| x.checked_add(pl)).is_none_or(|x| x > size) {
            return Err(bad());
        }
        let read = |f: &mut std::io::BufReader<std::fs::File>| -> std::io::Result<_> {
            let parent = read_vec_u32(f, n)?;
            let name_id = read_vec_u32(f, n)?;
            let mut flags = vec![0u8; n];
            f.read_exact(&mut flags)?;
            let off = read_vec_u32(f, k + 1)?;
            let mut pool = vec![0u8; pl];
            f.read_exact(&mut pool)?;
            Ok((parent, name_id, flags, off, pool))
        };
        let (parent, mut name_id, flags, off, pool) = read(&mut f).map_err(|_| bad())?;
        // every name inside the pool, every id and parent valid
        if off[0] != 0 || off[k] as usize != pl || off.windows(2).any(|w| w[0] > w[1]) {
            return Err(bad());
        }
        if (0..n).any(|i| name_id[i] as usize >= k || (parent[i] != NONE && parent[i] as usize >= n)) {
            return Err(bad());
        }
        let removed = flags.iter().filter(|&&f| f & REMOVED != 0).count();
        let mut names = Names { pool, off, lookup: IdTable::new() };
        if let Some(remap) = names.rebuild_lookup() {
            for id in &mut name_id {
                *id = remap[*id as usize];
            }
        }
        let mut ix = Index { root, parent, name_id, flags, names, dirs: IdTable::new(), removed };
        ix.index_dirs();
        Ok(ix)
    }

    /// A version 1 file: entries with their own name (offset, length) into a pool.
    fn load_v1(f: &mut impl Read, root: PathBuf, n: usize, nl: usize) -> std::io::Result<Index> {
        let bad = || std::io::Error::new(std::io::ErrorKind::InvalidData, "not an index file");
        let parent = read_vec_u32(f, n)?;
        let name_off = read_vec_u32(f, n)?;
        let mut name_len = vec![0u16; n];
        f.read_exact(as_bytes_mut(&mut name_len))?;
        if cfg!(target_endian = "big") {
            name_len.iter_mut().for_each(|x| *x = u16::from_le(*x));
        }
        let mut flags = vec![0u8; n];
        f.read_exact(&mut flags)?;
        let mut pool = vec![0u8; nl];
        f.read_exact(&mut pool)?;
        let mut ix = Index::empty(root);
        ix.parent = parent;
        ix.flags = flags;
        ix.name_id = Vec::with_capacity(n);
        for i in 0..n {
            let (o, l) = (name_off[i] as usize, name_len[i] as usize);
            let name = pool.get(o..o + l).ok_or_else(bad)?;
            if ix.parent[i] != NONE && ix.parent[i] as usize >= n {
                return Err(bad());
            }
            let id = ix.names.intern(name);
            ix.name_id.push(id);
        }
        ix.removed = ix.flags.iter().filter(|&&f| f & REMOVED != 0).count();
        ix.index_dirs();
        ix.shrink();
        Ok(ix)
    }
}

fn read_u32(r: &mut impl Read) -> std::io::Result<u32> {
    let mut b = [0u8; 4];
    r.read_exact(&mut b)?;
    Ok(u32::from_le_bytes(b))
}

fn read_u64(r: &mut impl Read) -> std::io::Result<u64> {
    let mut b = [0u8; 8];
    r.read_exact(&mut b)?;
    Ok(u64::from_le_bytes(b))
}

/// The arrays as they are in memory: little-endian on x86-64 and aarch64, the file's order.
fn as_bytes<T: Copy>(v: &[T]) -> &[u8] {
    // u8/u16/u32 only: no padding, every bit pattern valid
    unsafe { std::slice::from_raw_parts(v.as_ptr() as *const u8, std::mem::size_of_val(v)) }
}

fn as_bytes_mut<T: Copy>(v: &mut [T]) -> &mut [u8] {
    unsafe { std::slice::from_raw_parts_mut(v.as_mut_ptr() as *mut u8, std::mem::size_of_val(v)) }
}

/// `v` little-endian (Linux also runs on big-endian machines, unlike upstream's targets).
fn write_u32s(w: &mut impl Write, v: &[u32]) -> std::io::Result<()> {
    if cfg!(target_endian = "little") {
        return w.write_all(as_bytes(v));
    }
    for x in v {
        w.write_all(&x.to_le_bytes())?;
    }
    Ok(())
}

fn read_vec_u32(r: &mut impl Read, n: usize) -> std::io::Result<Vec<u32>> {
    let mut v = vec![0u32; n];
    r.read_exact(as_bytes_mut(&mut v))?;
    if cfg!(target_endian = "big") {
        v.iter_mut().for_each(|x| *x = u32::from_le(*x));
    }
    Ok(v)
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

    /// The ranked search returns the best names first, whatever their place in the index, and
    /// shallower paths first among equals.
    #[test]
    fn ranked_search_puts_the_best_first() {
        let root = tree("ranked");
        for i in 0..50 {
            std::fs::write(root.join(format!("eski rapor {i}.txt")), b"x").unwrap();
        }
        std::fs::create_dir_all(root.join("a/b")).unwrap();
        std::fs::write(root.join("a/b/rapor"), b"x").unwrap();
        std::fs::write(root.join("a/rapor"), b"x").unwrap();
        let ix = Index::build(&root, &AtomicBool::new(false));
        let rank = |name: &str| {
            let n = name.to_lowercase();
            if n == "rapor" { Some(1000) } else if n.starts_with("rapor") { Some(800) } else if n.contains("rapor") { Some(400) } else { None }
        };
        let got = ix.search_ranked("rapor", None, false, 3, &rank).unwrap();
        assert_eq!(got[0].1, root.join("a").join("rapor"));
        assert_eq!(got[1].1, root.join("a").join("b").join("rapor"));
        assert_eq!(got[2].0, 800); // Rapor.pdf
        std::fs::remove_dir_all(&root).unwrap();
    }

    fn names(v: &[PathBuf]) -> Vec<String> {
        let mut n: Vec<String> = v.iter().map(|p| p.file_name().unwrap().to_string_lossy().into_owned()).collect();
        n.sort();
        n
    }

    /// `ix` written in format 1 (a name per entry), as the index before the name table saved it.
    fn write_v1(ix: &Index, file: &Path) {
        let mut f = std::fs::File::create(file).unwrap();
        let r = ix.root.as_os_str().as_bytes();
        let n = ix.parent.len();
        let mut pool = Vec::new();
        let (mut offs, mut lens) = (Vec::new(), Vec::new());
        for i in 0..n as u32 {
            offs.push(pool.len() as u32);
            lens.push(ix.name(i).len() as u16);
            pool.extend_from_slice(ix.name(i));
        }
        f.write_all(Index::MAGIC_V1).unwrap();
        f.write_all(&(r.len() as u32).to_le_bytes()).unwrap();
        f.write_all(r).unwrap();
        f.write_all(&(n as u64).to_le_bytes()).unwrap();
        f.write_all(&(pool.len() as u64).to_le_bytes()).unwrap();
        for v in [&ix.parent, &offs] {
            for x in v {
                f.write_all(&x.to_le_bytes()).unwrap();
            }
        }
        for x in &lens {
            f.write_all(&x.to_le_bytes()).unwrap();
        }
        f.write_all(&ix.flags).unwrap();
        f.write_all(&pool).unwrap();
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
        // nothing matches: no second pass
        assert!(ix.search("yokböyle", None, false, 10).unwrap().is_empty());
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
        let odd_dir = OsStr::from_bytes(b"klas\xf6r");
        std::fs::create_dir_all(root.join(odd_dir)).unwrap();
        std::fs::write(root.join(odd_dir).join("içinde.txt"), b"x").unwrap();
        let ix = Index::build(&root, &AtomicBool::new(false));
        let hits = ix.search("rapor-", None, false, 10).unwrap();
        assert_eq!(hits, [root.join(odd)]);
        assert!(hits[0].exists());
        // through a save in either format, and a folder found by its bytes
        let file = root.with_extension("bytes.rfidx");
        let v1 = root.with_extension("bytes1.rfidx");
        ix.save(&file).unwrap();
        write_v1(&ix, &v1);
        for f in [&file, &v1] {
            let back = Index::load(f).unwrap();
            assert_eq!(back.search("rapor-", None, false, 10).unwrap(), [root.join(odd)]);
            assert_eq!(back.search("içinde", Some(&root.join(odd_dir)), false, 10).unwrap(), [root.join(odd_dir).join("içinde.txt")]);
            std::fs::remove_file(f).unwrap();
        }
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
        // still follows changes after compacting (folders found again by name id)
        std::fs::write(root.join("Arşiv/2026/rapor 3.txt"), b"x").unwrap();
        back.refresh_dirs(&[root.join("Arşiv/2026")], &AtomicBool::new(false));
        assert_eq!(names(&back.search("rapor", Some(&root.join("Arşiv")), false, 10).unwrap()), ["eski rapor.pdf", "rapor 3.txt"]);
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
        // and back: the old name finds the new folder, not the removed one
        std::fs::rename(root.join("Proje 2"), root.join("Proje")).unwrap();
        ix.refresh_dirs(std::slice::from_ref(&root), &AtomicBool::new(false));
        assert_eq!(ix.search("main", Some(&root.join("Proje")), false, 10).unwrap(), [root.join("Proje/main.rs")]);
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
        // searching inside the hidden folder the user opened finds what is in it
        assert_eq!(names(&ix.search("rapor", Some(&secret), false, 10).unwrap()), ["rapor gizli.txt"]);
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
        std::fs::write(&file, &b[..b.len() - 1]).unwrap();
        assert!(Index::load(&file).is_err());
        // a name id past the table
        let mut c = b.clone();
        let at = 8 + 4 + ix.root.as_os_str().len() + 24 + ix.parent.len() * 4;
        c[at..at + 4].copy_from_slice(&u32::MAX.to_le_bytes());
        std::fs::write(&file, &c).unwrap();
        assert!(Index::load(&file).is_err());
        std::fs::write(&file, b"RFNIX\0\0\x01rest").unwrap();
        assert!(Index::load(&file).is_err());
        std::fs::write(&file, b"RFN").unwrap();
        assert!(Index::load(&file).is_err());
        std::fs::remove_file(&file).unwrap();
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn id_table_finds_inserts_and_removes_like_a_set() {
        // keys collide on purpose (hash = id % 37): long runs, wrap-around, holes refilled
        let hash = |id: u32| (id % 37) as u64 * 0x9e37_79b9;
        let mut t = IdTable::new();
        let mut model = std::collections::HashSet::new();
        let mut x = 12345u32;
        for step in 0..20_000 {
            x = x.wrapping_mul(1_103_515_245).wrapping_add(12345);
            let id = (x >> 8) % 500;
            if model.contains(&id) && step % 3 != 0 {
                t.remove(hash(id), id, hash);
                model.remove(&id);
            } else if !model.contains(&id) {
                t.insert(hash(id), id, hash);
                model.insert(id);
            }
            if step % 997 == 0 {
                for probe in 0..500 {
                    assert_eq!(t.find(hash(probe), |i| i == probe).is_some(), model.contains(&probe), "step {step}, id {probe}");
                }
            }
        }
        assert_eq!(t.len, model.len());
    }

    #[test]
    fn names_are_stored_once_and_old_files_convert() {
        let root = tree("names");
        for d in ["a", "b", "c"] {
            std::fs::create_dir_all(root.join(d)).unwrap();
            std::fs::write(root.join(d).join("README.md"), b"x").unwrap();
        }
        let ix = Index::build(&root, &AtomicBool::new(false));
        assert_eq!(names(&ix.search("readme", None, false, 10).unwrap()), ["README.md"; 3]);
        // "", 3 folder names, README.md and the tree's 8 names: 13, not 16
        assert_eq!(ix.distinct_names(), 13);
        // a version 1 file (a name per entry) reads into the same index
        let file = root.with_extension("v1.rfidx");
        write_v1(&ix, &file);
        let back = Index::load(&file).unwrap();
        assert_eq!(back.distinct_names(), 13);
        assert_eq!(back.live(), ix.live());
        assert_eq!(back.search("readme", Some(&root.join("b")), false, 10).unwrap(), [root.join("b").join("README.md")]);
        // a truncated file is refused, not trusted
        let bytes = std::fs::read(&file).unwrap();
        std::fs::write(&file, &bytes[..bytes.len() - 3]).unwrap();
        assert!(Index::load(&file).is_err());
        std::fs::remove_file(&file).unwrap();
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_name_stored_twice_in_a_file_is_one_name() {
        // not written by save, but a file could hold it: folders must still be found
        let root = tree("twice");
        let ix = Index::build(&root, &AtomicBool::new(false));
        let file = root.with_extension("twice.rfidx");
        let r = ix.root.as_os_str().as_bytes();
        // the table again with every name doubled; entries point at the second copies
        let k = ix.names.len();
        let mut pool = ix.names.pool.clone();
        pool.extend_from_slice(&ix.names.pool);
        let mut off = ix.names.off.clone();
        off.extend(ix.names.off[1..].iter().map(|o| o + ix.names.pool.len() as u32));
        let ids: Vec<u32> = ix.name_id.iter().map(|&i| i + k as u32).collect();
        let mut f = std::fs::File::create(&file).unwrap();
        f.write_all(Index::MAGIC).unwrap();
        f.write_all(&(r.len() as u32).to_le_bytes()).unwrap();
        f.write_all(r).unwrap();
        for x in [ix.parent.len(), 2 * k, pool.len()] {
            f.write_all(&(x as u64).to_le_bytes()).unwrap();
        }
        write_u32s(&mut f, &ix.parent).unwrap();
        write_u32s(&mut f, &ids).unwrap();
        f.write_all(&ix.flags).unwrap();
        write_u32s(&mut f, &off).unwrap();
        f.write_all(&pool).unwrap();
        drop(f);
        let mut back = Index::load(&file).unwrap();
        assert_eq!(back.search("ışık", Some(&root.join("Proje/Çizimler")), false, 10).unwrap(), [root.join("Proje/Çizimler/ışık.png")]);
        std::fs::write(root.join("Proje/yeni.txt"), b"x").unwrap();
        back.refresh_dirs(&[root.join("Proje")], &AtomicBool::new(false));
        assert_eq!(back.search("yeni", None, false, 10).unwrap(), [root.join("Proje/yeni.txt")]);
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
