//! Our own copy / move engine: many files at once with `copy_file_range` (permissions and
//! times come along; on btrfs/XFS the kernel can share extents instead of copying bytes).
//! Linux port of rf-win's engine: planning, conflicts, merging, undo and verification are the
//! same code; only the file copy, renames and volume checks differ.
//!
//! The rest of rf-win's description holds, with "shell engine" meaning `fileops`' simpler
//! engine on Linux (symlinks are copied as links there).
//!
//! rf-win's notes: Names that already exist at the destination are ours to handle too (like Files):
//! folders merge, and every clashing file is put to the user once, in one list (replace, skip
//! or keep both).
//!
//! * Moves on one volume are renames (instant); onto another volume a copy, after which each
//!   source file is deleted only once its copy checks out.
//! * "Replace" copies to a temporary name first and swaps it in at the end, so a failed or
//!   cancelled copy never costs the file that was there.
//! * Small files are spread over folders: threads writing into the same folder wait on its
//!   lock (20 000 files: ~3 400/s in folder order, ~6 500/s spread).
//! * Pause, a speed limit and byte-accurate progress run in CopyFile2's progress callback.
//!
//! Left to the shell engine, so behavior stays Explorer's where it matters: items containing
//! links / junctions, a folder into itself, paths that aren't plain file system paths, and
//! files that fail here (access denied, in use, …) — retried through the shell at the end,
//! which can ask for elevation.
//!
//! Blocking: call from a worker thread.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicI32, AtomicU64, AtomicUsize, Ordering};
use std::fs::File;
use std::io::{Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::time::{Duration, Instant, SystemTime};

use crate::fileops::{Tracker, Transfer};

/// Bytes per `copy_file_range` call: progress, pause and cancel are checked between them.
const CHUNK: usize = 8 << 20;
/// Files above this size are started first (they set the pace; small ones fill the gaps).
const BIG: u64 = 8 << 20;
/// A rename counts like this many bytes of progress.
const RENAME_WEIGHT: u64 = 64 << 10;

/// How copies are checked after writing.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
#[repr(u8)]
pub enum Verify {
    #[default]
    Off,
    /// Same size and modification time.
    Quick,
    /// Same bytes, read back from the disk (not from the cache).
    Full,
}

/// A name that already exists at the destination.
#[derive(Clone, Debug)]
pub struct Conflict {
    pub src: PathBuf,
    pub dst: PathBuf,
    pub src_dir: bool,
    pub dst_dir: bool,
    pub src_size: u64,
    pub dst_size: u64,
    pub src_time: Option<SystemTime>,
    pub dst_time: Option<SystemTime>,
}

impl Conflict {
    /// A file against a folder (or the other way round): it can't replace, only skip or keep both.
    pub fn mismatched(&self) -> bool {
        self.src_dir != self.dst_dir
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Resolve {
    Replace,
    Skip,
    KeepBoth,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Op {
    Copy,
    Rename,
}

/// One file to copy, or one item to rename into place.
#[derive(Clone, Debug)]
struct Job {
    src: PathBuf,
    dst: PathBuf,
    size: u64,
    op: Op,
    /// The destination exists and is to be replaced.
    replace: bool,
}

/// What a transfer will do, worked out before anything is written.
#[derive(Default)]
struct Plan {
    /// Folders to create (copies), parents first.
    dirs: Vec<PathBuf>,
    jobs: Vec<Job>,
    conflicts: Vec<(Conflict, Job)>,
    /// (source, destination) of every item this transfer creates, for undo.
    undo: Vec<(PathBuf, PathBuf)>,
    /// (source folder, its new copy): attributes are copied once the contents are in.
    new_dirs: Vec<(PathBuf, PathBuf)>,
    /// Source folders whose contents are taken file by file (moves remove them when empty).
    src_dirs: Vec<PathBuf>,
    /// Top-level sources for the shell engine.
    whole: Vec<PathBuf>,
    /// Destination paths this plan writes (lowercase), so "keep both" picks a free name.
    taken: HashSet<String>,
}

/// Names are case-sensitive on Linux.
fn key(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}

/// Symlinks play the part of Windows' reparse points: never followed here.
fn is_reparse(m: &std::fs::Metadata) -> bool {
    m.file_type().is_symlink()
}

/// A link or junction somewhere in the tree: the shell copes with those.
struct HasLink;

impl Plan {
    /// Plans one top-level source; on a link inside it, nothing of it is kept and it goes to
    /// the shell whole.
    fn top(&mut self, src: &Path, dest_dir: &Path, rename: bool) {
        let Some(name) = src.file_name() else {
            self.whole.push(src.to_path_buf());
            return;
        };
        let dst = dest_dir.join(name);
        let meta = match std::fs::symlink_metadata(src) {
            Ok(m) if !is_reparse(&m) => m,
            _ => {
                self.whole.push(src.to_path_buf());
                return;
            }
        };
        // a folder into itself or below itself: the shell refuses it with its own message
        if meta.is_dir() && inside(dest_dir, src) {
            self.whole.push(src.to_path_buf());
            return;
        }
        let mut sub = Plan { taken: std::mem::take(&mut self.taken), ..Default::default() };
        match sub.visit(src, &dst, &meta, rename) {
            Ok(()) => {
                self.taken = std::mem::take(&mut sub.taken);
                self.dirs.extend(sub.dirs);
                self.jobs.extend(sub.jobs);
                self.conflicts.extend(sub.conflicts);
                self.undo.extend(sub.undo);
                self.new_dirs.extend(sub.new_dirs);
                self.src_dirs.extend(sub.src_dirs);
            }
            Err(HasLink) => {
                // drop what it reserved
                let mut taken = std::mem::take(&mut sub.taken);
                for j in sub.jobs.iter().chain(sub.conflicts.iter().map(|c| &c.1)) {
                    taken.remove(&key(&j.dst));
                }
                for d in &sub.dirs {
                    taken.remove(&key(d));
                }
                self.taken = taken;
                self.whole.push(src.to_path_buf());
            }
        }
    }

    fn visit(&mut self, src: &Path, dst: &Path, meta: &std::fs::Metadata, rename: bool) -> Result<(), HasLink> {
        // already planned by another source of this transfer (same name from two folders)
        if self.taken.contains(&key(dst)) {
            let free = self.free_name(dst, meta.is_dir());
            return self.visit(src, &free, meta, rename);
        }
        let there = std::fs::symlink_metadata(dst).ok();
        match there {
            None if rename => {
                self.taken.insert(key(dst));
                self.jobs.push(Job { src: src.into(), dst: dst.into(), size: RENAME_WEIGHT, op: Op::Rename, replace: false });
                self.undo.push((src.into(), dst.into()));
            }
            None if meta.is_dir() => {
                self.new_tree(src, dst)?;
                self.undo.push((src.into(), dst.into()));
            }
            None => {
                self.taken.insert(key(dst));
                self.jobs.push(Job { src: src.into(), dst: dst.into(), size: meta.len(), op: Op::Copy, replace: false });
                self.undo.push((src.into(), dst.into()));
            }
            // folder onto folder: merge, item by item
            Some(d) if meta.is_dir() && d.is_dir() && !is_reparse(&d) => {
                self.src_dirs.push(src.into());
                let rd = std::fs::read_dir(src).map_err(|_| HasLink)?;
                for e in rd.flatten() {
                    let m = std::fs::symlink_metadata(e.path()).map_err(|_| HasLink)?;
                    if is_reparse(&m) {
                        return Err(HasLink);
                    }
                    self.visit(&e.path(), &dst.join(e.file_name()), &m, rename)?;
                }
            }
            Some(d) => {
                let c = Conflict {
                    src: src.into(),
                    dst: dst.into(),
                    src_dir: meta.is_dir(),
                    dst_dir: d.is_dir(),
                    src_size: if meta.is_dir() { 0 } else { meta.len() },
                    dst_size: if d.is_dir() { 0 } else { d.len() },
                    src_time: meta.modified().ok(),
                    dst_time: d.modified().ok(),
                };
                if meta.is_dir() {
                    // walked now, so a link inside is found before anything is asked
                    check_tree(src)?;
                }
                let size = if rename { RENAME_WEIGHT } else { meta.len() };
                self.conflicts.push((c, Job { src: src.into(), dst: dst.into(), size, op: if rename { Op::Rename } else { Op::Copy }, replace: false }));
            }
        }
        Ok(())
    }

    /// A whole new folder: its folders and files (copies).
    fn new_tree(&mut self, src: &Path, dst: &Path) -> Result<(), HasLink> {
        self.taken.insert(key(dst));
        self.dirs.push(dst.into());
        self.new_dirs.push((src.into(), dst.into()));
        let mut stack = vec![(src.to_path_buf(), dst.to_path_buf())];
        while let Some((from, to)) = stack.pop() {
            let rd = std::fs::read_dir(&from).map_err(|_| HasLink)?;
            for e in rd.flatten() {
                let m = e.metadata().map_err(|_| HasLink)?;
                if is_reparse(&m) {
                    return Err(HasLink);
                }
                let (a, b) = (e.path(), to.join(e.file_name()));
                if m.is_dir() {
                    self.dirs.push(b.clone());
                    self.new_dirs.push((a.clone(), b.clone()));
                    stack.push((a, b));
                } else {
                    self.jobs.push(Job { src: a, dst: b, size: m.len(), op: Op::Copy, replace: false });
                }
            }
        }
        Ok(())
    }

    /// Applies the user's answers to the conflicts.
    fn resolve(&mut self, answers: &[Resolve]) {
        let conflicts = std::mem::take(&mut self.conflicts);
        for ((c, mut job), r) in conflicts.into_iter().zip(answers.iter().copied()) {
            let r = if r == Resolve::Replace && c.mismatched() { Resolve::KeepBoth } else { r };
            match r {
                Resolve::Skip => {}
                Resolve::Replace => {
                    job.replace = true;
                    self.undo.push((job.src.clone(), job.dst.clone()));
                    self.jobs.push(job);
                }
                Resolve::KeepBoth => {
                    let dst = self.free_name(&c.dst, c.src_dir);
                    if job.op == Op::Rename || !c.src_dir {
                        self.taken.insert(key(&dst));
                        self.undo.push((job.src.clone(), dst.clone()));
                        self.jobs.push(Job { dst, ..job });
                    } else if self.new_tree(&c.src, &dst).is_ok() {
                        self.undo.push((c.src.clone(), dst));
                    }
                }
            }
        }
    }

    /// "name (2).ext", "name (3).ext", … : the first that neither exists nor is planned.
    fn free_name(&self, dst: &Path, is_dir: bool) -> PathBuf {
        let dir = dst.parent().unwrap_or(Path::new(""));
        let name = dst.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let (stem, ext) = match name.rfind('.') {
            Some(i) if i > 0 && !is_dir => (name[..i].to_string(), name[i..].to_string()),
            _ => (name.clone(), String::new()),
        };
        (2..)
            .map(|n| dir.join(format!("{stem} ({n}){ext}")))
            .find(|p| !self.taken.contains(&key(p)) && std::fs::symlink_metadata(p).is_err())
            .expect("a free name")
    }
}

/// Errors if there is a link or junction anywhere under `dir`.
fn check_tree(dir: &Path) -> Result<(), HasLink> {
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).map_err(|_| HasLink)?.flatten() {
            let m = e.metadata().map_err(|_| HasLink)?;
            if is_reparse(&m) {
                return Err(HasLink);
            }
            if m.is_dir() {
                stack.push(e.path());
            }
        }
    }
    Ok(())
}

/// Plans `sources` into `dest_dir`: what goes where, and what clashes.
fn plan(sources: &[PathBuf], dest_dir: &Path, rename: bool) -> Plan {
    let mut p = Plan::default();
    for s in sources {
        p.top(s, dest_dir, rename);
    }
    p
}

/// Big files first, then the rest taken from each folder in turn.
fn order(jobs: Vec<Job>) -> Vec<Job> {
    let (mut big, small): (Vec<Job>, Vec<Job>) = jobs.into_iter().partition(|j| j.size >= BIG);
    big.sort_by_key(|j| std::cmp::Reverse(j.size));
    let mut folders: Vec<std::collections::VecDeque<Job>> = Vec::new();
    let mut at: std::collections::HashMap<PathBuf, usize> = Default::default();
    for j in small {
        let parent = j.dst.parent().map(Path::to_path_buf).unwrap_or_default();
        let ix = *at.entry(parent).or_insert_with(|| {
            folders.push(Default::default());
            folders.len() - 1
        });
        folders[ix].push_back(j);
    }
    let mut out = big;
    loop {
        let before = out.len();
        for q in folders.iter_mut() {
            if let Some(j) = q.pop_front() {
                out.push(j);
            }
        }
        if out.len() == before {
            break;
        }
    }
    out
}

/// Progress, pause, speed limit and cancel, shared by the copy threads.
struct Run<'a> {
    tracker: &'a Tracker,
    total: u64,
    done: AtomicU64,
    /// Turned non-zero to stop the copies (each removes its partial file).
    cancel: AtomicI32,
    /// Speed limit clock: (since, bytes counted since then).
    clock: Mutex<(Instant, u64)>,
    last_report: Mutex<Instant>,
}

impl Run<'_> {
    fn cancelled(&self) -> bool {
        if self.tracker.cancel.load(Ordering::Relaxed) {
            self.cancel.store(1, Ordering::Relaxed);
        }
        self.cancel.load(Ordering::Relaxed) != 0
    }

    /// Blocks while paused; the paused time doesn't count against the speed limit.
    fn wait_if_paused(&self) {
        if !self.tracker.paused.load(Ordering::Relaxed) {
            return;
        }
        let from = Instant::now();
        while self.tracker.paused.load(Ordering::Relaxed) && !self.cancelled() {
            std::thread::sleep(Duration::from_millis(100));
        }
        let mut c = self.clock.lock().unwrap();
        c.0 += from.elapsed();
    }

    fn add(&self, bytes: u64, name: Option<&Path>) {
        self.done.fetch_add(bytes, Ordering::Relaxed);
        let limit = self.tracker.limit.load(Ordering::Relaxed);
        if limit > 0 {
            let wait = {
                let mut c = self.clock.lock().unwrap();
                c.1 += bytes;
                let due = Duration::from_secs_f64(c.1 as f64 / limit as f64);
                due.saturating_sub(c.0.elapsed())
            };
            if !wait.is_zero() {
                std::thread::sleep(wait.min(Duration::from_millis(500)));
            }
        }
        let mut last = self.last_report.lock().unwrap();
        if last.elapsed() >= Duration::from_millis(50) {
            *last = Instant::now();
            drop(last);
            self.report(name);
        }
    }

    fn report(&self, name: Option<&Path>) {
        let d = self.done.load(Ordering::Relaxed).min(self.total);
        let permille = (d as u128 * 1000 / self.total.max(1) as u128) as u32;
        (self.tracker.on_progress)(1000, permille, name.and_then(|n| n.file_name()).map(|n| n.to_string_lossy().into_owned()));
    }
}

fn cancelled_err() -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::Interrupted, "cancelled")
}

/// Copies the bytes of `from` into `to`, `CHUNK` at a time with `copy_file_range` (falling
/// back to read/write where the kernel or file system can't), counting progress and honoring
/// pause, speed limit and cancel between chunks. Returns the bytes copied.
fn copy_bytes(from: &File, to: &mut File, name: &Path, run: &Run) -> std::io::Result<u64> {
    let mut sent = 0u64;
    let mut kernel = true;
    let mut buf = Vec::new();
    loop {
        run.wait_if_paused();
        if run.cancelled() {
            return Err(cancelled_err());
        }
        let n = if kernel {
            // SAFETY: valid fds; null offsets use and advance the files' own positions
            let r = unsafe {
                libc::copy_file_range(from.as_raw_fd(), std::ptr::null_mut(), to.as_raw_fd(), std::ptr::null_mut(), CHUNK, 0)
            };
            if r < 0 {
                let e = std::io::Error::last_os_error();
                // not supported here (old kernel, across file systems, FUSE…): plain reads
                if sent == 0 && matches!(e.raw_os_error(), Some(libc::EXDEV | libc::EINVAL | libc::ENOSYS | libc::EOPNOTSUPP)) {
                    kernel = false;
                    continue;
                }
                return Err(e);
            }
            r as usize
        } else {
            if buf.is_empty() {
                buf = vec![0u8; 1 << 20];
            }
            let mut f = from;
            let k = f.read(&mut buf)?;
            to.write_all(&buf[..k])?;
            k
        };
        if n == 0 {
            return Ok(sent);
        }
        sent += n as u64;
        run.add(n as u64, Some(name));
    }
}

/// Copies one file: a new file (never over an existing one), its bytes, then the source's
/// permissions and times. A failed or cancelled copy leaves nothing behind.
fn copy_file(src: &Path, dst: &Path, size: u64, run: &Run) -> std::io::Result<()> {
    let from = File::open(src)?;
    let meta = from.metadata()?;
    // writable by us while it is written; the source's mode comes at the end
    let mut to = std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(dst)?;
    let r = copy_bytes(&from, &mut to, src, run).and_then(|sent| {
        // whatever wasn't counted (an empty file copies no chunk)
        run.add(size.max(1).saturating_sub(sent), None);
        let times = std::fs::FileTimes::new().set_modified(meta.modified()?).set_accessed(meta.accessed()?);
        to.set_times(times)?;
        to.set_permissions(std::fs::Permissions::from_mode(meta.permissions().mode() & 0o7777))
    });
    if r.is_err() {
        drop(to);
        let _ = std::fs::remove_file(dst);
    }
    r
}

/// Moves `from` over `to`: replacing it when asked (`rename` replaces on Linux), else failing
/// when `to` exists.
fn move_into_place(from: &Path, to: &Path, replace: bool) -> std::io::Result<()> {
    if replace { std::fs::rename(from, to) } else { crate::fileops::rename_no_replace(from, to) }
}

/// A temporary name next to `dst` for a copy that replaces it.
fn part_name(dst: &Path) -> PathBuf {
    let name = dst.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    dst.with_file_name(format!(".{name}.{}.rfpart", std::process::id()))
}

/// One job; `Err` = leave it to the fallback engine.
fn run_job(j: &Job, run: &Run) -> std::io::Result<()> {
    match j.op {
        Op::Rename => {
            let r = move_into_place(&j.src, &j.dst, j.replace);
            run.add(j.size, Some(&j.src));
            r
        }
        Op::Copy if j.replace => {
            let tmp = part_name(&j.dst);
            let _ = std::fs::remove_file(&tmp);
            copy_file(&j.src, &tmp, j.size, run)?;
            move_into_place(&tmp, &j.dst, true).inspect_err(|_| {
                let _ = std::fs::remove_file(&tmp);
            })
        }
        Op::Copy => copy_file(&j.src, &j.dst, j.size, run),
    }
}

/// What a transfer did here: undo pairs, files copied (source, copy), and what is left for
/// the shell.
#[derive(Default)]
pub struct Outcome {
    pub undo: Vec<(PathBuf, PathBuf)>,
    /// Single files that failed here: (source, destination folder).
    pub failed: Vec<(PathBuf, PathBuf)>,
    /// Top-level sources the shell should transfer whole.
    pub whole: Vec<PathBuf>,
    copied: Vec<(PathBuf, PathBuf)>,
    src_dirs: Vec<PathBuf>,
    cancelled: bool,
}

/// Plans, asks about conflicts, and runs a transfer with our engine (`rename` = a move on one
/// volume). Everything it can't do is in the returned leftovers.
fn execute(sources: &[PathBuf], dest_dir: &Path, tracker: &Tracker, threads: usize, rename: bool) -> Outcome {
    let mut p = plan(sources, dest_dir, rename);
    if !p.conflicts.is_empty() {
        let list: Vec<Conflict> = p.conflicts.iter().map(|c| c.0.clone()).collect();
        let answers = match &tracker.ask {
            Some(ask) => ask(&list),
            None => Some(vec![Resolve::KeepBoth; list.len()]),
        };
        match answers {
            Some(a) => p.resolve(&a),
            None => return Outcome { cancelled: true, ..Default::default() },
        }
    }
    for d in &p.dirs {
        if let Err(e) = std::fs::create_dir(d) {
            if e.kind() != std::io::ErrorKind::AlreadyExists {
                log::warn!("fastcopy: mkdir {}: {e}", d.display());
            }
        }
    }
    let jobs = order(std::mem::take(&mut p.jobs));
    let run = Run {
        tracker,
        total: jobs.iter().map(|j| j.size.max(1)).sum::<u64>().max(1),
        done: AtomicU64::new(0),
        cancel: AtomicI32::new(0),
        clock: Mutex::new((Instant::now(), 0)),
        last_report: Mutex::new(Instant::now() - Duration::from_secs(1)),
    };
    let next = AtomicUsize::new(0);
    let failed = Mutex::new(Vec::new());
    let done = Mutex::new(Vec::new());
    std::thread::scope(|s| {
        for _ in 0..threads.clamp(1, jobs.len().max(1)) {
            s.spawn(|| {
                loop {
                    run.wait_if_paused();
                    if run.cancelled() {
                        return;
                    }
                    let i = next.fetch_add(1, Ordering::Relaxed);
                    let Some(j) = jobs.get(i) else { return };
                    match run_job(j, &run) {
                        Ok(()) => done.lock().unwrap().push(i),
                        Err(e) => {
                            if !run.cancelled() {
                                log::info!("fastcopy: {} → fallback ({e})", j.src.display());
                                failed.lock().unwrap().push(i);
                            }
                        }
                    }
                }
            });
        }
    });
    run.report(None);
    let cancelled = run.cancelled();
    // folder modes and times once their contents are in (deepest first: setting a parent's
    // time before its children land would be undone by them)
    for (src, dst) in p.new_dirs.iter().rev() {
        if let Ok(m) = std::fs::metadata(src) {
            if let Ok(d) = File::open(dst) {
                if let (Ok(mt), Ok(at)) = (m.modified(), m.accessed()) {
                    let _ = d.set_times(std::fs::FileTimes::new().set_modified(mt).set_accessed(at));
                }
            }
            let _ = std::fs::set_permissions(dst, std::fs::Permissions::from_mode(m.permissions().mode() & 0o7777));
        }
    }
    let failed: Vec<(PathBuf, PathBuf)> = failed
        .into_inner()
        .unwrap()
        .into_iter()
        .filter_map(|i| jobs[i].dst.parent().map(|d| (jobs[i].src.clone(), d.to_path_buf())))
        .collect();
    let copied = done.into_inner().unwrap().into_iter().filter(|&i| jobs[i].op == Op::Copy).map(|i| (jobs[i].src.clone(), jobs[i].dst.clone())).collect();
    Outcome { undo: p.undo, failed, whole: p.whole, copied, src_dirs: p.src_dirs, cancelled }
}

/// Copies `sources` into `dest_dir` in parallel (conflicts as `tracker.ask` says; without it
/// both are kept). Returns the undo pairs and the leftovers for the shell.
pub fn copy(sources: &[PathBuf], dest_dir: &Path, tracker: &Tracker, threads: usize) -> Outcome {
    execute(sources, dest_dir, tracker, threads, false)
}

/// Copy or move with our engine where it can, the shell engine for the rest. Returns like
/// `fileops::transfer_tracked`; undo pairs land in `tracker.results`. A verification failure
/// is an `Err` (the copies stay).
pub fn transfer(kind: Transfer, sources: &[PathBuf], dest_dir: &Path, tracker: Arc<Tracker>) -> anyhow::Result<bool> {
    // pasting where the items already are ("x - Copy"), and anything not on a plain file
    // system path: the shell's way
    let same_dir = sources.iter().all(|s| s.parent() == Some(dest_dir));
    if same_dir || !local_fs(dest_dir) || !sources.iter().all(|s| local_fs(s)) {
        return crate::fileops::transfer_tracked(kind, sources, dest_dir, tracker);
    }
    let (here, there): (Vec<PathBuf>, Vec<PathBuf>) = sources.iter().cloned().partition(|s| kind == Transfer::Move && same_volume(s, dest_dir));
    let mut ok = true;
    if !here.is_empty() {
        let out = execute(&here, dest_dir, &tracker, 4, true);
        ok &= finish(Transfer::Move, &here, dest_dir, &tracker, out)?;
    }
    if !there.is_empty() && !tracker.cancel.load(Ordering::Relaxed) {
        let threads = threads_for(&there, dest_dir);
        let out = execute(&there, dest_dir, &tracker, threads, false);
        ok &= finish(kind, &there, dest_dir, &tracker, out)?;
    }
    for s in sources {
        if let Some(p) = s.parent() {
            crate::shell::notify_updated(p);
        }
    }
    crate::shell::notify_updated(dest_dir);
    Ok(ok && !tracker.cancel.load(Ordering::Relaxed))
}

/// After our engine: the shell for the leftovers, verification, and for moves the removal of
/// what was moved.
fn finish(kind: Transfer, sources: &[PathBuf], dest_dir: &Path, tracker: &Arc<Tracker>, out: Outcome) -> anyhow::Result<bool> {
    tracker.results.lock().unwrap().extend(out.undo.iter().cloned());
    if out.cancelled || tracker.cancel.load(Ordering::Relaxed) {
        return Ok(false);
    }
    let mut ok = true;
    if !out.whole.is_empty() {
        ok &= crate::fileops::transfer_tracked(kind, &out.whole, dest_dir, tracker.clone())?;
    }
    // failed single files: one shell call per destination folder
    let mut by_dir: Vec<(PathBuf, Vec<PathBuf>)> = Vec::new();
    for (f, d) in &out.failed {
        match by_dir.iter_mut().find(|(x, _)| x == d) {
            Some((_, v)) => v.push(f.clone()),
            None => by_dir.push((d.clone(), vec![f.clone()])),
        }
    }
    for (d, fs) in by_dir {
        // the shell reports its own results; those files' undo is covered by their folder
        let t = Arc::new(Tracker::new(Box::new(|_, _, _| {}), tracker.cancel.clone()));
        ok &= crate::fileops::transfer_tracked(kind, &fs, &d, t)?;
    }
    // checks: asked for, or a move about to delete its sources
    let mode = match (tracker.verify_mode(), kind) {
        (Verify::Off, Transfer::Move) => Verify::Quick,
        (m, _) => m,
    };
    let bad = check(&out.copied, mode, tracker);
    if tracker.cancel.load(Ordering::Relaxed) {
        return Ok(false);
    }
    if kind == Transfer::Move {
        // only sources whose copy checked out go; skipped and failed ones stay
        let bad_set: HashSet<&PathBuf> = bad.iter().collect();
        let files: Vec<PathBuf> = out.copied.iter().filter(|(s, _)| !bad_set.contains(s)).map(|(s, _)| s.clone()).collect();
        // folders emptied by the move: merged ones, and whole ones copied across volumes
        let mut dirs: Vec<PathBuf> = out.src_dirs.clone();
        for (s, _) in &out.undo {
            if s.is_dir() && sources.iter().any(|t| s.starts_with(t)) {
                walk_dirs(s, &mut dirs);
            }
        }
        dirs.sort_by_key(|d| d.components().count());
        dirs.dedup();
        if !crate::fastdelete::remove_verified(&files, &dirs) {
            log::info!("fastcopy: some sources stay (skipped, failed or changed meanwhile)");
        }
    }
    if !bad.is_empty() {
        let names: Vec<String> = bad.iter().take(5).filter_map(|p| p.file_name()).map(|n| n.to_string_lossy().into_owned()).collect();
        anyhow::bail!("{}: {} ({})", NOT_VERIFIED, bad.len(), names.join(", "));
    }
    Ok(ok)
}

/// Error text for copies that didn't verify (translated by the app).
pub const NOT_VERIFIED: &str = "Doğrulanamayan dosyalar";
/// Shown while copies are read back.
pub const VERIFYING: &str = "Doğrulanıyor…";

/// Every folder under `dir` (and `dir`).
fn walk_dirs(dir: &Path, out: &mut Vec<PathBuf>) {
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        if let Ok(rd) = std::fs::read_dir(&d) {
            for e in rd.flatten() {
                if e.file_type().is_ok_and(|t| t.is_dir()) {
                    stack.push(e.path());
                }
            }
        }
        out.push(d);
    }
}

/// Sources whose copy doesn't match.
fn check(pairs: &[(PathBuf, PathBuf)], mode: Verify, tracker: &Tracker) -> Vec<PathBuf> {
    if mode == Verify::Off || pairs.is_empty() {
        return Vec::new();
    }
    if mode == Verify::Full {
        (tracker.on_progress)(1000, 0, Some(VERIFYING.to_string()));
    }
    let next = AtomicUsize::new(0);
    let bad = Mutex::new(Vec::new());
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get()).clamp(2, 8);
    std::thread::scope(|s| {
        for _ in 0..threads {
            s.spawn(|| {
                loop {
                    if tracker.cancel.load(Ordering::Relaxed) {
                        return;
                    }
                    let i = next.fetch_add(1, Ordering::Relaxed);
                    let Some((a, b)) = pairs.get(i) else { return };
                    let same = match mode {
                        Verify::Quick => same_meta(a, b),
                        _ => same_meta(a, b) && same_bytes(a, b).unwrap_or(false),
                    };
                    if !same {
                        log::warn!("fastcopy: {} does not match {}", b.display(), a.display());
                        bad.lock().unwrap().push(a.clone());
                    }
                    if mode == Verify::Full && i % 16 == 0 {
                        (tracker.on_progress)(1000, (i * 1000 / pairs.len()) as u32, None);
                    }
                }
            });
        }
    });
    bad.into_inner().unwrap()
}

/// Same size and modification time (CopyFile2 keeps it; 2 s of slack for FAT/exFAT sticks).
fn same_meta(a: &Path, b: &Path) -> bool {
    let (Ok(ma), Ok(mb)) = (std::fs::metadata(a), std::fs::metadata(b)) else { return false };
    if ma.len() != mb.len() {
        return false;
    }
    match (ma.modified(), mb.modified()) {
        (Ok(x), Ok(y)) => x.duration_since(y).or_else(|_| y.duration_since(x)).is_ok_and(|d| d <= Duration::from_secs(2)),
        _ => false,
    }
}

/// Same bytes; the copy is read from the disk, not the page cache (synced, then its cached
/// pages dropped).
fn same_bytes(a: &Path, b: &Path) -> std::io::Result<bool> {
    const CHUNK: usize = 1 << 20;
    let mut fa = File::open(a)?;
    let mut fb = File::open(b)?;
    fb.sync_all()?;
    // SAFETY: valid fd; advice only
    unsafe {
        libc::posix_fadvise(fb.as_raw_fd(), 0, 0, libc::POSIX_FADV_DONTNEED);
        libc::posix_fadvise(fa.as_raw_fd(), 0, 0, libc::POSIX_FADV_SEQUENTIAL);
    }
    let (mut ba, mut bb) = (vec![0u8; CHUNK], vec![0u8; CHUNK]);
    loop {
        let n = read_full(&mut fb, &mut bb)?;
        let m = read_full(&mut fa, &mut ba[..n])?;
        if m != n || ba[..n] != bb[..n] {
            return Ok(false);
        }
        if n < CHUNK {
            // the copy ended: the source must too
            return Ok(read_full(&mut fa, &mut ba[..1])? == 0);
        }
    }
}

/// Reads until `buf` is full or the file ends.
fn read_full(f: &mut File, buf: &mut [u8]) -> std::io::Result<usize> {
    let mut n = 0;
    while n < buf.len() {
        let k = f.read(&mut buf[n..])?;
        if k == 0 {
            break;
        }
        n += k;
    }
    Ok(n)
}

/// `inner` is `outer` or inside it (component-wise; names are case-sensitive on Linux).
fn inside(inner: &Path, outer: &Path) -> bool {
    inner.starts_with(outer)
}

/// Same file system (device), so a move there is a rename. A bind mount of the same device
/// makes `rename` fail with EXDEV: that job then falls back to a copy.
fn same_volume(a: &Path, b: &Path) -> bool {
    match (std::fs::symlink_metadata(a), std::fs::metadata(b)) {
        (Ok(x), Ok(y)) => x.dev() == y.dev(),
        _ => false,
    }
}

/// Parallel copies pay off on SSDs; a spinning disk on either side only seeks more with
/// many at once, and network shares and USB sticks like a moderate number.
fn threads_for(sources: &[PathBuf], dest: &Path) -> usize {
    let cores = std::thread::available_parallelism().map_or(4, |n| n.get());
    let paths = sources.iter().map(PathBuf::as_path).chain(std::iter::once(dest));
    let mut slow = false;
    for p in paths {
        match crate::drives::seek_penalty(p) {
            Some(true) => return 2,
            Some(false) => {}
            None => slow = true,
        }
        slow |= crate::drives::is_removable(p);
    }
    if slow { 4 } else { cores.clamp(4, 16) }
}

/// Everything with an absolute path is a plain file system path on Linux (network shares are
/// mounted like any disk).
fn local_fs(p: &Path) -> bool {
    p.is_absolute()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicBool;

    fn tracker() -> Tracker {
        Tracker::new(Box::new(|_, _, _| {}), Arc::new(AtomicBool::new(false)))
    }

    fn answering(r: Resolve) -> Tracker {
        let mut t = tracker();
        t.ask = Some(Box::new(move |c| Some(vec![r; c.len()])));
        t
    }

    fn base(name: &str) -> PathBuf {
        let b = std::env::temp_dir().join(format!("rf-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&b);
        std::fs::create_dir_all(&b).unwrap();
        b
    }

    fn tree(root: &Path) {
        let _ = std::fs::remove_dir_all(root);
        std::fs::create_dir_all(root.join("a/b/c")).unwrap();
        std::fs::create_dir_all(root.join("boş klasör")).unwrap();
        std::fs::write(root.join("kök.txt"), b"merhaba").unwrap();
        std::fs::write(root.join("a/boş.dat"), b"").unwrap();
        let big: Vec<u8> = (0..3_000_000u32).map(|i| (i * 31 % 251) as u8).collect();
        std::fs::write(root.join("a/b/büyük.bin"), &big).unwrap();
        for i in 0..300 {
            std::fs::write(root.join(format!("a/b/c/f{i}.txt")), format!("dosya {i}").repeat(i + 1)).unwrap();
        }
    }

    fn same(a: &Path, b: &Path) {
        let ma = std::fs::metadata(a).unwrap();
        let mb = std::fs::metadata(b).unwrap();
        assert_eq!(ma.is_dir(), mb.is_dir(), "{}", b.display());
        if ma.is_dir() {
            let mut na: Vec<_> = std::fs::read_dir(a).unwrap().map(|e| e.unwrap().file_name()).collect();
            let mut nb: Vec<_> = std::fs::read_dir(b).unwrap().map(|e| e.unwrap().file_name()).collect();
            na.sort();
            nb.sort();
            assert_eq!(na, nb, "{}", b.display());
            for n in na {
                same(&a.join(&n), &b.join(&n));
            }
        } else {
            assert_eq!(std::fs::read(a).unwrap(), std::fs::read(b).unwrap(), "{}", b.display());
            assert_eq!(ma.modified().unwrap(), mb.modified().unwrap(), "{}", b.display());
        }
    }

    fn same_bytes_tree(a: &Path, b: &Path) {
        let mut stack = vec![(a.to_path_buf(), b.to_path_buf())];
        while let Some((x, y)) = stack.pop() {
            for e in std::fs::read_dir(&x).unwrap() {
                let e = e.unwrap();
                let (p, q) = (e.path(), y.join(e.file_name()));
                if p.is_dir() {
                    stack.push((p, q));
                } else {
                    assert_eq!(std::fs::read(&p).unwrap(), std::fs::read(&q).unwrap(), "{}", q.display());
                }
            }
        }
    }

    #[test]
    fn copies_a_tree_exactly() {
        let base = base("fastcopy");
        let (src, dst) = (base.join("kaynak"), base.join("hedef"));
        tree(&src);
        std::fs::create_dir_all(&dst).unwrap();
        let t = tracker();
        let out = copy(&[src.clone(), src.join("kök.txt")], &dst, &t, 8);
        assert!(out.whole.is_empty() && out.failed.is_empty());
        assert_eq!(out.undo.len(), 2);
        same(&src, &dst.join("kaynak"));
        same(&src.join("kök.txt"), &dst.join("kök.txt"));
        // again into the same place, nobody asked: the folder merges, every file gets a twin
        let out = copy(&[src.clone()], &dst, &t, 8);
        assert!(out.whole.is_empty());
        assert!(dst.join("kaynak/kök (2).txt").exists());
        assert!(dst.join("kaynak/a/b/c/f7 (2).txt").exists());
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn conflicts_replace_skip_keep_both() {
        let base = base("conflicts");
        let (src, dst) = (base.join("kaynak"), base.join("hedef"));
        std::fs::create_dir_all(src.join("alt")).unwrap();
        std::fs::create_dir_all(dst.join("kaynak/alt")).unwrap();
        std::fs::write(src.join("alt/aynı.txt"), b"yeni").unwrap();
        std::fs::write(src.join("alt/yeni.txt"), b"sadece kaynakta").unwrap();
        std::fs::write(dst.join("kaynak/alt/aynı.txt"), b"eski").unwrap();
        std::fs::write(dst.join("kaynak/alt/sadece hedefte.txt"), "kalır".as_bytes()).unwrap();
        let asked = Arc::new(Mutex::new(Vec::new()));
        // skip: the old file stays, the new one arrives, the merge keeps the rest
        let mut t = tracker();
        let a2 = asked.clone();
        t.ask = Some(Box::new(move |c| {
            a2.lock().unwrap().extend(c.iter().map(|c| c.dst.clone()));
            Some(vec![Resolve::Skip; c.len()])
        }));
        transfer(Transfer::Copy, &[src.clone()], &dst, Arc::new(t)).unwrap();
        assert_eq!(*asked.lock().unwrap(), vec![dst.join("kaynak/alt/aynı.txt")], "only the clashing file is asked");
        assert_eq!(std::fs::read(dst.join("kaynak/alt/aynı.txt")).unwrap(), b"eski");
        assert_eq!(std::fs::read(dst.join("kaynak/alt/yeni.txt")).unwrap(), b"sadece kaynakta");
        assert!(dst.join("kaynak/alt/sadece hedefte.txt").exists());
        // keep both
        transfer(Transfer::Copy, &[src.clone()], &dst, Arc::new(answering(Resolve::KeepBoth))).unwrap();
        assert_eq!(std::fs::read(dst.join("kaynak/alt/aynı (2).txt")).unwrap(), b"yeni");
        assert_eq!(std::fs::read(dst.join("kaynak/alt/aynı.txt")).unwrap(), b"eski");
        // replace (a read-only target included), no temporary file left behind
        let target = dst.join("kaynak/alt/aynı.txt");
        let mut perm = std::fs::metadata(&target).unwrap().permissions();
        perm.set_readonly(true);
        std::fs::set_permissions(&target, perm).unwrap();
        transfer(Transfer::Copy, &[src.clone()], &dst, Arc::new(answering(Resolve::Replace))).unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"yeni");
        let leftovers = std::fs::read_dir(dst.join("kaynak/alt")).unwrap().flatten().filter(|e| e.file_name().to_string_lossy().ends_with(".rfpart")).count();
        assert_eq!(leftovers, 0);
        // cancel at the question: nothing written
        let mut t = tracker();
        t.ask = Some(Box::new(|_| None));
        std::fs::write(src.join("alt/başka.txt"), b"x").unwrap();
        assert!(!transfer(Transfer::Copy, &[src.clone()], &dst, Arc::new(t)).unwrap());
        assert!(!dst.join("kaynak/alt/başka.txt").exists());
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn keep_both_never_takes_a_planned_name() {
        let base = base("freename");
        let (src, dst) = (base.join("kaynak"), base.join("hedef"));
        std::fs::create_dir_all(&src).unwrap();
        std::fs::create_dir_all(&dst).unwrap();
        // "a (2).txt" is copied in this same run; the clashing "a.txt" must become "a (3).txt"
        std::fs::write(src.join("a.txt"), b"yeni a").unwrap();
        std::fs::write(src.join("a (2).txt"), b"kaynaktaki a (2)").unwrap();
        std::fs::write(dst.join("a.txt"), b"eski a").unwrap();
        transfer(Transfer::Copy, &[src.join("a.txt"), src.join("a (2).txt")], &dst, Arc::new(answering(Resolve::KeepBoth))).unwrap();
        assert_eq!(std::fs::read(dst.join("a.txt")).unwrap(), b"eski a");
        assert_eq!(std::fs::read(dst.join("a (2).txt")).unwrap(), b"kaynaktaki a (2)");
        assert_eq!(std::fs::read(dst.join("a (3).txt")).unwrap(), b"yeni a");
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn moves_on_one_volume_rename_and_merge() {
        let base = base("rename");
        let (src, dst) = (base.join("kaynak"), base.join("hedef"));
        tree(&src);
        std::fs::create_dir_all(dst.join("kaynak/a")).unwrap();
        std::fs::write(dst.join("kaynak/kök.txt"), b"eski").unwrap();
        std::fs::write(dst.join("kaynak/a/hedefte.txt"), "kalır".as_bytes()).unwrap();
        let t = Arc::new(answering(Resolve::Skip));
        assert!(transfer(Transfer::Move, &[src.clone()], &dst, t.clone()).unwrap());
        // merged: the clash skipped (stays in the source), the rest moved
        assert_eq!(std::fs::read(dst.join("kaynak/kök.txt")).unwrap(), b"eski");
        assert_eq!(std::fs::read(src.join("kök.txt")).unwrap(), b"merhaba");
        assert!(dst.join("kaynak/a/b/c/f299.txt").exists());
        assert!(dst.join("kaynak/a/hedefte.txt").exists());
        assert!(!src.join("a").exists(), "emptied folders go");
        assert!(src.exists(), "the source keeps the skipped file");
        // undo pairs name what was moved, never the merged folder itself
        let res = t.results.lock().unwrap().clone();
        assert!(!res.iter().any(|(_, d)| *d == dst.join("kaynak")), "{res:?}");
        // a plain rename of a whole folder
        let lone = base.join("tek");
        std::fs::create_dir_all(lone.join("iç")).unwrap();
        assert!(transfer(Transfer::Move, &[lone.clone()], &dst, Arc::new(tracker())).unwrap());
        assert!(!lone.exists() && dst.join("tek/iç").is_dir());
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn move_deletes_sources_only_after_everything_arrived() {
        let base = base("fastmove");
        let (src, keep, dst) = (base.join("kaynak"), base.join("kopya-referans"), base.join("hedef"));
        tree(&src);
        tree(&keep);
        std::fs::create_dir_all(&dst).unwrap();
        let lone = base.join("tek dosya.txt");
        std::fs::write(&lone, b"tek").unwrap();
        // the cross-volume path, exercised on one volume
        let t = Arc::new(tracker());
        let out = execute(&[src.clone(), lone.clone()], &dst, &t, 8, false);
        assert!(finish(Transfer::Move, &[src.clone(), lone.clone()], &dst, &t, out).unwrap());
        assert!(!src.exists() && !lone.exists(), "sources removed");
        same_bytes_tree(&keep, &dst.join("kaynak"));
        assert_eq!(std::fs::read(dst.join("tek dosya.txt")).unwrap(), b"tek");
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn verify_catches_a_changed_copy() {
        let base = base("verify");
        let (src, dst) = (base.join("kaynak"), base.join("hedef"));
        tree(&src);
        std::fs::create_dir_all(&dst).unwrap();
        let t = tracker();
        t.set_verify(Verify::Full);
        assert!(transfer(Transfer::Copy, &[src.clone()], &dst, Arc::new(t)).unwrap(), "a clean copy verifies");
        let pairs = vec![(src.join("a/b/büyük.bin"), dst.join("kaynak/a/b/büyük.bin"))];
        assert!(check(&pairs, Verify::Full, &tracker()).is_empty());
        // flip one byte in the middle, keep size and time
        let f = dst.join("kaynak/a/b/büyük.bin");
        let time = std::fs::metadata(&f).unwrap().modified().unwrap();
        let mut bytes = std::fs::read(&f).unwrap();
        bytes[1_500_000] ^= 0xFF;
        std::fs::write(&f, &bytes).unwrap();
        std::fs::File::options().write(true).open(&f).unwrap().set_modified(time).unwrap();
        assert!(check(&pairs, Verify::Quick, &tracker()).is_empty(), "size and time still match");
        assert_eq!(check(&pairs, Verify::Full, &tracker()), vec![src.join("a/b/büyük.bin")]);
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn pause_holds_and_speed_limit_slows() {
        let base = base("pause");
        let src = base.join("kaynak");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::create_dir_all(base.join("hedef")).unwrap();
        std::fs::write(src.join("2mb.bin"), vec![1u8; 2 << 20]).unwrap();
        // 4 MB/s limit: 2 MB take ~0.5 s
        let t = tracker();
        t.limit.store(4 << 20, Ordering::Relaxed);
        let start = Instant::now();
        copy(&[src.join("2mb.bin")], &base.join("hedef"), &t, 2);
        assert!(start.elapsed() >= Duration::from_millis(350), "{:?}", start.elapsed());
        // paused: nothing happens until resumed
        let t = Arc::new(tracker());
        t.paused.store(true, Ordering::Relaxed);
        let t2 = t.clone();
        let (s2, d2) = (src.clone(), base.join("hedef2"));
        std::fs::create_dir_all(&d2).unwrap();
        let h = std::thread::spawn(move || copy(&[s2.join("2mb.bin")], &d2, &t2, 2).undo.len());
        std::thread::sleep(Duration::from_millis(400));
        assert!(!base.join("hedef2/2mb.bin").exists(), "nothing copied while paused");
        t.paused.store(false, Ordering::Relaxed);
        assert_eq!(h.join().unwrap(), 1);
        assert!(base.join("hedef2/2mb.bin").exists());
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn order_spreads_small_files_over_folders() {
        let j = |d: &str, f: &str, size: u64| Job { src: PathBuf::from(format!("s/{d}/{f}")), dst: PathBuf::from(format!("d/{d}/{f}")), size, op: Op::Copy, replace: false };
        let jobs = vec![j("a", "1", 10), j("a", "2", 10), j("a", "3", 10), j("b", "1", 10), j("b", "2", 10), j("c", "big", 100 << 20)];
        let order: Vec<String> = order(jobs).iter().map(|j| j.dst.to_string_lossy().replace('\\', "/")).collect();
        assert_eq!(order, ["d/c/big", "d/a/1", "d/b/1", "d/a/2", "d/b/2", "d/a/3"]);
    }

    #[test]
    fn inside_is_component_wise() {
        assert!(inside(Path::new("/a/alt"), Path::new("/a")));
        assert!(inside(Path::new("/a"), Path::new("/a/")));
        assert!(!inside(Path::new("/ab"), Path::new("/a")));
        assert!(!inside(Path::new("/A/alt"), Path::new("/a")));
    }

    #[test]
    fn volumes_and_self_nesting() {
        let tmp = std::env::temp_dir();
        assert!(same_volume(&tmp, &tmp));
        // /proc is its own file system
        assert!(!same_volume(Path::new("/proc/self"), &tmp));
        // a folder into its own subfolder is left to the shell (which refuses it)
        let base = base("fastnest");
        std::fs::create_dir_all(base.join("a/alt")).unwrap();
        let t = tracker();
        let out = copy(&[base.join("a")], &base.join("a/alt"), &t, 2);
        assert!(out.undo.is_empty() && out.whole == vec![base.join("a")]);
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn cancel_stops_without_partial_files() {
        let base = base("fastcopy-c");
        let (src, dst) = (base.join("kaynak"), base.join("hedef"));
        tree(&src);
        std::fs::create_dir_all(&dst).unwrap();
        let t = tracker();
        t.cancel.store(true, Ordering::Relaxed);
        let _ = copy(&[src.clone()], &dst, &t, 4);
        // whatever exists is complete: no file differs from its source
        let mut stack = vec![dst.join("kaynak")];
        while let Some(d) = stack.pop() {
            for e in std::fs::read_dir(&d).into_iter().flatten().flatten() {
                let p = e.path();
                if p.is_dir() {
                    stack.push(p);
                } else {
                    let rel = p.strip_prefix(dst.join("kaynak")).unwrap();
                    assert_eq!(std::fs::read(&p).unwrap(), std::fs::read(src.join(rel)).unwrap());
                }
            }
        }
        let _ = std::fs::remove_dir_all(&base);
    }
}
