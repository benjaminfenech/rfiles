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
//! * Crash-safe: the planned job list goes to a journal (synced to disk) before the first byte
//!   is copied, and finished files are recorded as they complete. If the app dies mid-copy,
//!   [`interrupted`] finds the job on the next start and [`resume`] carries it on with the same
//!   decisions (conflicts aren't asked again); every file the journal or the destination says
//!   is done is re-checked against its source before it's skipped. Linux: big files carry on
//!   mid-file from synced checkpoints (rf-win: `COPY_FILE_RESTARTABLE`).
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
use std::sync::{Mutex, OnceLock};
use std::sync::atomic::{AtomicI32, AtomicU64, AtomicUsize, Ordering};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::os::fd::AsRawFd;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::time::{Duration, Instant, SystemTime};

use crate::fileops::{Tracker, Transfer};

/// Bytes per `copy_file_range` call: progress, pause and cancel are checked between them.
const CHUNK: usize = if cfg!(test) { 256 << 10 } else { 8 << 20 };
/// Files above this size are started first (they set the pace; small ones fill the gaps).
const BIG: u64 = 8 << 20;
/// With a journal, files from this size on record checkpoints while they copy, so a copy cut
/// off by a crash carries on mid-file instead of starting over (smaller ones copy again in a
/// moment). rf-win uses 2 GB: Windows' restartable mode costs about 30 % throughout, while a
/// checkpoint here is one `fdatasync` every `CHECKPOINT_EVERY`.
const RESTARTABLE_MIN: u64 = if cfg!(test) { 1 << 20 } else { 64 << 20 };
/// How often a big copy is synced and its progress recorded.
const CHECKPOINT_EVERY: Duration = if cfg!(test) { Duration::ZERO } else { Duration::from_secs(2) };
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
    /// A big copy cut off earlier: carry it on from this many bytes (its last checkpoint).
    resume: u64,
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
                self.jobs.push(Job { src: src.into(), dst: dst.into(), size: RENAME_WEIGHT, op: Op::Rename, replace: false, resume: 0 });
                self.undo.push((src.into(), dst.into()));
            }
            None if meta.is_dir() => {
                self.new_tree(src, dst)?;
                self.undo.push((src.into(), dst.into()));
            }
            None => {
                self.taken.insert(key(dst));
                self.jobs.push(Job { src: src.into(), dst: dst.into(), size: meta.len(), op: Op::Copy, replace: false, resume: 0 });
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
                self.conflicts.push((c, Job { src: src.into(), dst: dst.into(), size, op: if rename { Op::Rename } else { Op::Copy }, replace: false, resume: 0 }));
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
                    self.jobs.push(Job { src: a, dst: b, size: m.len(), op: Op::Copy, replace: false, resume: 0 });
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
    /// In temporary names (".name.<tag>.rfpart"): the journal's id (the same when the job is
    /// resumed, so a big copy finds its partial file again), else the process id.
    tag: String,
    /// Journaled: big files record checkpoints, so a crash doesn't cost them their progress.
    journal: Option<&'a Jr>,
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

/// A big journaled copy's progress record: every `CHECKPOINT_EVERY` the copy is synced and
/// its length recorded, so a resumed job carries on from there (rf-win: `COPY_FILE_RESTARTABLE`).
struct Checkpoint<'a> {
    jr: &'a Jr,
    /// The job's index in the journal.
    index: usize,
    /// The source's modification time: a source changed since doesn't resume.
    stamp: String,
    last: Instant,
}

/// Copies the bytes of `from` into `to` from their current positions, `CHUNK` at a time with
/// `copy_file_range` (falling back to read/write where the kernel or file system can't),
/// counting progress and honoring pause, speed limit and cancel between chunks. `at`: where
/// the copy starts (a resumed one). Returns the bytes copied.
fn copy_bytes(from: &File, to: &mut File, name: &Path, run: &Run, at: u64, mut ck: Option<Checkpoint>) -> std::io::Result<u64> {
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
        if let Some(c) = ck.as_mut()
            && c.last.elapsed() >= CHECKPOINT_EVERY
        {
            // the bytes first, then the record that vouches for them
            to.sync_data()?;
            c.jr.checkpoint(c.index, at + sent, &c.stamp);
            c.last = Instant::now();
        }
    }
}

/// A file's modification time, exactly (for checkpoints).
fn stamp(m: &std::fs::Metadata) -> String {
    format!("{}.{:09}", m.mtime(), m.mtime_nsec())
}

/// Copies one file: a new file (never over an existing one), its bytes, then the source's
/// permissions and times. A failed or cancelled copy leaves nothing behind. `at` > 0 carries
/// on a partial copy at `dst` from there (whatever lies beyond is cut off first). `index`:
/// the job's index in the journal (big files record checkpoints there).
fn copy_file(src: &Path, dst: &Path, size: u64, run: &Run, at: u64, index: Option<usize>) -> std::io::Result<()> {
    let from = File::open(src)?;
    let meta = from.metadata()?;
    let mut to = if at > 0 {
        let f = std::fs::OpenOptions::new().write(true).open(dst)?;
        f.set_len(at)?;
        (&from).seek(SeekFrom::Start(at))?;
        (&f).seek(SeekFrom::Start(at))?;
        // counted without the speed limit: those bytes were copied before
        run.done.fetch_add(at, Ordering::Relaxed);
        f
    } else {
        // writable by us while it is written; the source's mode comes at the end
        std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(dst)?
    };
    let ck = match (run.journal, index) {
        (Some(jr), Some(index)) if size >= RESTARTABLE_MIN => Some(Checkpoint { jr, index, stamp: stamp(&meta), last: Instant::now() }),
        _ => None,
    };
    let r = copy_bytes(&from, &mut to, src, run, at, ck).and_then(|sent| {
        // whatever wasn't counted (an empty file copies no chunk)
        run.add(size.max(1).saturating_sub(at + sent), None);
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
fn part_name(dst: &Path, tag: &str) -> PathBuf {
    let name = dst.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    dst.with_file_name(format!(".{name}.{tag}.rfpart"))
}

/// A copy into `to`; a resumed one carries on from its checkpoint, and starts over if it can't
/// (the partial file is removed first).
fn copy_or_resume(j: &Job, to: &Path, run: &Run, index: Option<usize>) -> std::io::Result<()> {
    if j.resume > 0 {
        match copy_file(&j.src, to, j.size, run, j.resume, index) {
            Ok(()) => {
                log::info!("fastcopy: carried on {} from {} MB", j.dst.display(), j.resume >> 20);
                return Ok(());
            }
            Err(e) if run.cancelled() => return Err(e),
            Err(e) => {
                log::info!("fastcopy: resume of {} failed ({e}), copying it again", j.dst.display());
                let _ = std::fs::remove_file(to);
            }
        }
    }
    copy_file(&j.src, to, j.size, run, 0, index)
}

/// One job (`index`: its index in the journal); `Err` = leave it to the fallback engine.
fn run_job(j: &Job, run: &Run, index: Option<usize>) -> std::io::Result<()> {
    match j.op {
        Op::Rename => {
            let r = move_into_place(&j.src, &j.dst, j.replace);
            run.add(j.size, Some(&j.src));
            r
        }
        Op::Copy if j.replace => {
            let tmp = part_name(&j.dst, &run.tag);
            if j.resume == 0 {
                let _ = std::fs::remove_file(&tmp);
            }
            copy_or_resume(j, &tmp, run, index)?;
            move_into_place(&tmp, &j.dst, true).inspect_err(|_| {
                let _ = std::fs::remove_file(&tmp);
            })
        }
        Op::Copy => copy_or_resume(j, &j.dst, run, index),
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

/// Plans, asks about conflicts, journals the plan and runs a transfer with our engine (`rename`
/// = a move on one volume). Everything it can't do is in the returned leftovers. The journal is
/// returned open: the caller closes it once `finish` is done (a crash before that leaves it).
fn execute(kind: Transfer, sources: &[PathBuf], dest_dir: &Path, tracker: &Tracker, threads: usize, rename: bool) -> (Outcome, Option<Jr>) {
    let mut p = plan(sources, dest_dir, rename);
    if !p.conflicts.is_empty() {
        let list: Vec<Conflict> = p.conflicts.iter().map(|c| c.0.clone()).collect();
        let answers = match &tracker.ask {
            Some(ask) => ask(&list),
            None => Some(vec![Resolve::KeepBoth; list.len()]),
        };
        match answers {
            Some(a) => p.resolve(&a),
            None => return (Outcome { cancelled: true, ..Default::default() }, None),
        }
    }
    p.jobs = order(std::mem::take(&mut p.jobs));
    let jr = journal_start(kind, rename, tracker.verify_mode(), sources, dest_dir, &p);
    let n = p.jobs.len();
    let out = run_jobs(p, tracker, threads, jr.as_ref().map(|j| (j, (0..n).collect::<Vec<_>>())), Vec::new());
    (out, jr)
}

/// Creates the plan's folders and runs its jobs (already ordered) on `threads` threads.
/// `journal`: where finished jobs are recorded, with each job's index in the journal.
/// `already`: (source, copy) pairs done before (a resumed job), checked and finished like the
/// rest.
fn run_jobs(mut p: Plan, tracker: &Tracker, threads: usize, journal: Option<(&Jr, Vec<usize>)>, already: Vec<(PathBuf, PathBuf)>) -> Outcome {
    for d in &p.dirs {
        if let Err(e) = std::fs::create_dir(d) {
            if e.kind() != std::io::ErrorKind::AlreadyExists {
                log::warn!("fastcopy: mkdir {}: {e}", d.display());
            }
        }
    }
    let jobs = std::mem::take(&mut p.jobs);
    let tag = journal.as_ref().and_then(|(jr, _)| jr.path.file_stem()).map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| std::process::id().to_string());
    let run = Run {
        tracker,
        tag,
        journal: journal.as_ref().map(|(jr, _)| *jr),
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
                    let index = journal.as_ref().map(|(_, ix)| ix[i]);
                    match run_job(j, &run, index) {
                        Ok(()) => {
                            done.lock().unwrap().push(i);
                            if let (Some((jr, _)), Some(index)) = (&journal, index) {
                                jr.done(index);
                            }
                        }
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
    if let Some((jr, _)) = &journal {
        jr.sync();
    }
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
    let mut copied = already;
    copied.extend(done.into_inner().unwrap().into_iter().filter(|&i| jobs[i].op == Op::Copy).map(|i| (jobs[i].src.clone(), jobs[i].dst.clone())));
    Outcome { undo: p.undo, failed, whole: p.whole, copied, src_dirs: p.src_dirs, cancelled }
}

/// Copies `sources` into `dest_dir` in parallel (conflicts as `tracker.ask` says; without it
/// both are kept). Returns the undo pairs and the leftovers for the shell.
pub fn copy(sources: &[PathBuf], dest_dir: &Path, tracker: &Tracker, threads: usize) -> Outcome {
    let (out, jr) = execute(Transfer::Copy, sources, dest_dir, tracker, threads, false);
    if let Some(j) = jr {
        j.close();
    }
    out
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
        let (out, jr) = execute(Transfer::Move, &here, dest_dir, &tracker, 4, true);
        let r = finish(Transfer::Move, &here, dest_dir, &tracker, out);
        if let Some(j) = jr {
            j.close();
        }
        ok &= r?;
    }
    if !there.is_empty() && !tracker.cancel.load(Ordering::Relaxed) {
        let threads = threads_for(&there, dest_dir);
        let (out, jr) = execute(kind, &there, dest_dir, &tracker, threads, false);
        let r = finish(kind, &there, dest_dir, &tracker, out);
        if let Some(j) = jr {
            j.close();
        }
        ok &= r?;
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

// ---------------------------------------------------------------- crash-safe job journal
//
// One file per running plan in the journal folder, records (see `journal`; rf-win's format,
// with paths escaped: `\\`, `\t`, `\n`, and `\xNN` for bytes that aren't UTF-8):
//   B kind(c|m) rename(0|1) verify(0|1|2) dest sources…   the job
//   D dir · N src dst · S src_dir · W whole                 the plan's folders and shell items
//   J op(c|r) replace(0|1) size src dst                    one per job, in run order
//   P                                                       the plan is complete (synced)
//   F index                                                 job `index` finished (batched)
//   C index bytes mtime                                     Linux: a big copy's first `bytes`
//                                                           are on the disk (source as of mtime)
// A journal without P never started copying. Closing a journal deletes it, so what's left in the
// folder is exactly the jobs that were cut off.

static JOURNAL_DIR: OnceLock<PathBuf> = OnceLock::new();
/// Finished-file records are flushed to disk at most this often (a lost one only means that
/// file is checked again on resume).
const JOURNAL_SYNC: Duration = Duration::from_millis(250);

/// Where journals live; set once at startup. Without it nothing is journaled.
pub fn set_journal_dir(dir: PathBuf) {
    let _ = std::fs::create_dir_all(&dir);
    let _ = JOURNAL_DIR.set(dir);
}

struct Jr {
    w: Mutex<(crate::journal::Writer, Instant)>,
    path: PathBuf,
}

impl Jr {
    fn done(&self, index: usize) {
        let mut g = self.w.lock().unwrap();
        let _ = g.0.record(&["F", &index.to_string()]);
        if g.1.elapsed() >= JOURNAL_SYNC {
            let _ = g.0.sync();
            g.1 = Instant::now();
        }
    }
    /// Job `index`'s copy has its first `bytes` on the disk (synced by the caller): recorded
    /// and synced at once (these come seconds apart).
    fn checkpoint(&self, index: usize, bytes: u64, stamp: &str) {
        let mut g = self.w.lock().unwrap();
        let _ = g.0.record(&["C", &index.to_string(), &bytes.to_string(), stamp]);
        let _ = g.0.sync();
        g.1 = Instant::now();
    }
    fn sync(&self) {
        let _ = self.w.lock().unwrap().0.sync();
    }
    /// The job ended (done, failed or cancelled by the user): nothing to resume.
    fn close(self) {
        let Jr { w, path } = self;
        drop(w);
        let _ = std::fs::remove_file(&path);
    }
}

/// A path as a journal field (any bytes, no tabs or line breaks).
fn ps(p: &Path) -> String {
    use std::fmt::Write as _;
    use std::os::unix::ffi::OsStrExt;
    let mut s = String::new();
    for chunk in p.as_os_str().as_bytes().utf8_chunks() {
        for c in chunk.valid().chars() {
            match c {
                '\\' => s.push_str("\\\\"),
                '\t' => s.push_str("\\t"),
                '\n' => s.push_str("\\n"),
                c => s.push(c),
            }
        }
        for b in chunk.invalid() {
            let _ = write!(s, "\\x{b:02x}");
        }
    }
    s
}

/// The path a journal field holds ([`ps`] undone).
fn pp(s: &str) -> PathBuf {
    use std::os::unix::ffi::OsStringExt;
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'\\' && i + 1 < b.len() {
            match b[i + 1] {
                b'\\' => out.push(b'\\'),
                b't' => out.push(b'\t'),
                b'n' => out.push(b'\n'),
                b'x' => {
                    if let Some(v) = s.get(i + 2..i + 4).and_then(|h| u8::from_str_radix(h, 16).ok()) {
                        out.push(v);
                        i += 4;
                        continue;
                    }
                    out.extend(b"\\x");
                }
                x => out.extend([b'\\', x]),
            }
            i += 2;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    PathBuf::from(std::ffi::OsString::from_vec(out))
}

fn journal_start(kind: Transfer, rename: bool, verify: Verify, sources: &[PathBuf], dest: &Path, p: &Plan) -> Option<Jr> {
    let dir = JOURNAL_DIR.get()?;
    static N: AtomicUsize = AtomicUsize::new(0);
    let stamp = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map_or(0, |d| d.as_millis());
    let path = dir.join(format!("{}-{stamp}-{}.jrn", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
    let mut w = crate::journal::Writer::create(&path).map_err(|e| log::warn!("journal: {}: {e}", path.display())).ok()?;
    let b = |x: bool| if x { "1" } else { "0" };
    let written = (|| -> std::io::Result<()> {
        let mut head = vec!["B".to_string(), (if kind == Transfer::Move { "m" } else { "c" }).into(), b(rename).into(), (verify as u8).to_string(), ps(dest)];
        head.extend(sources.iter().map(|x| ps(x)));
        w.record(&head.iter().map(String::as_str).collect::<Vec<_>>())?;
        for d in &p.dirs {
            w.record(&["D", &ps(d)])?;
        }
        for (a, c) in &p.new_dirs {
            w.record(&["N", &ps(a), &ps(c)])?;
        }
        for d in &p.src_dirs {
            w.record(&["S", &ps(d)])?;
        }
        for x in &p.whole {
            w.record(&["W", &ps(x)])?;
        }
        for j in &p.jobs {
            w.record(&["J", if j.op == Op::Copy { "c" } else { "r" }, b(j.replace), &j.size.to_string(), &ps(&j.src), &ps(&j.dst)])?;
        }
        w.record(&["P"])?;
        w.sync()
    })();
    if let Err(e) = written {
        log::warn!("journal: {}: {e}", path.display());
        drop(w);
        let _ = std::fs::remove_file(&path);
        return None;
    }
    Some(Jr { w: Mutex::new((w, Instant::now())), path })
}

/// A job that was cut off (the app closed or crashed while it ran).
#[derive(Clone, Debug)]
pub struct Interrupted {
    pub journal: PathBuf,
    pub kind: Transfer,
    pub verify: Verify,
    pub sources: Vec<PathBuf>,
    pub dest: PathBuf,
    /// Jobs in the plan, and how many were recorded as finished.
    pub files: usize,
    pub done: usize,
}

struct Loaded {
    info: Interrupted,
    rename: bool,
    records: Vec<Vec<String>>,
}

fn load(path: &Path) -> Option<Loaded> {
    let records = crate::journal::read(path).ok()?;
    let head = records.first().filter(|r| r.len() >= 5 && r[0] == "B")?;
    if !records.iter().any(|r| r[0] == "P") {
        return None;
    }
    let kind = if head[1] == "m" { Transfer::Move } else { Transfer::Copy };
    let verify = match head[3].as_str() {
        "1" => Verify::Quick,
        "2" => Verify::Full,
        _ => Verify::Off,
    };
    let files = records.iter().filter(|r| r[0] == "J").count();
    let done = records.iter().filter(|r| r[0] == "F").filter_map(|r| r.get(1)?.parse::<usize>().ok()).collect::<HashSet<_>>().len();
    Some(Loaded {
        rename: head[2] == "1",
        info: Interrupted { journal: path.to_path_buf(), kind, verify, sources: head[5..].iter().map(|s| pp(s)).collect(), dest: pp(&head[4]), files, done },
        records,
    })
}

/// Jobs left unfinished by an earlier run. Journals that never got past planning (nothing was
/// copied) are removed; journals still open (a running job, maybe in another window's process)
/// are left alone.
pub fn interrupted() -> Vec<Interrupted> {
    let Some(dir) = JOURNAL_DIR.get() else { return Vec::new() };
    let Ok(rd) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut out = Vec::new();
    for e in rd.flatten() {
        let path = e.path();
        let ext = path.extension().and_then(|x| x.to_str());
        if !matches!(ext, Some("jrn" | "jrn-new")) || crate::journal::in_use(&path) {
            continue;
        }
        // a resume cut off while it swapped in a clean journal: the old one still holds the job
        if ext == Some("jrn-new") {
            let _ = std::fs::remove_file(&path);
            continue;
        }
        match load(&path) {
            Some(l) => out.push(l.info),
            None => {
                let _ = std::fs::remove_file(&path);
            }
        }
    }
    out.sort_by(|a, b| a.journal.cmp(&b.journal));
    out
}

/// Forget an interrupted job (what it copied stays).
pub fn discard(journal: &Path) {
    let _ = std::fs::remove_file(journal);
}

/// Carries an interrupted job on with its original plan: no conflict is asked again. A file is
/// skipped only when the journal says it finished, or its copy matches the source (size and
/// time; byte for byte when the job verifies fully: that happens in `finish` for every file,
/// skipped or not). A big copy with a checkpoint carries on from there, if its source hasn't
/// changed. Anything else at a destination is a partial copy from the cut, and is replaced
/// (through a temporary name, as always). Moves delete sources only after the check.
pub fn resume(journal: &Path, tracker: Arc<Tracker>) -> anyhow::Result<bool> {
    // one resume at a time: another window may have offered the same job
    let guard = match crate::journal::Writer::append(journal) {
        Ok(w) => w,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(true),
        Err(e) => return Err(e.into()),
    };
    let Some(l) = load(journal) else {
        drop(guard);
        discard(journal);
        return Ok(true);
    };
    let tag = journal.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let mut p = Plan::default();
    let mut finished: HashSet<usize> = HashSet::new();
    // the last checkpoint of each big copy: (bytes, source mtime)
    let mut checkpoints: std::collections::HashMap<usize, (u64, &str)> = std::collections::HashMap::new();
    let mut jobs: Vec<Job> = Vec::new();
    for r in &l.records {
        match (r[0].as_str(), r.len()) {
            ("D", 2) => p.dirs.push(pp(&r[1])),
            ("N", 3) => p.new_dirs.push((pp(&r[1]), pp(&r[2]))),
            ("S", 2) => p.src_dirs.push(pp(&r[1])),
            ("W", 2) => p.whole.push(pp(&r[1])),
            ("J", 6) => jobs.push(Job {
                op: if r[1] == "r" { Op::Rename } else { Op::Copy },
                replace: r[2] == "1",
                size: r[3].parse().unwrap_or(0),
                src: pp(&r[4]),
                dst: pp(&r[5]),
                resume: 0,
            }),
            ("F", 2) => {
                if let Ok(i) = r[1].parse() {
                    finished.insert(i);
                }
            }
            ("C", 4) => {
                if let (Ok(i), Ok(n)) = (r[1].parse(), r[2].parse()) {
                    checkpoints.insert(i, (n, r[3].as_str()));
                }
            }
            _ => {}
        }
    }
    let mut already = Vec::new();
    let mut index = Vec::new();
    for (i, mut j) in jobs.into_iter().enumerate() {
        let src = std::fs::symlink_metadata(&j.src).ok();
        let dst_there = std::fs::symlink_metadata(&j.dst).is_ok();
        match j.op {
            Op::Rename => {
                if src.is_none() {
                    continue; // renamed before the cut
                }
            }
            Op::Copy => {
                let tmp = part_name(&j.dst, &tag);
                let Some(src) = src else {
                    let _ = std::fs::remove_file(&tmp);
                    continue; // a move that already deleted it: its copy checked out then
                };
                if dst_there && (finished.contains(&i) || same_meta(&j.src, &j.dst)) {
                    let _ = std::fs::remove_file(&tmp);
                    already.push((j.src.clone(), j.dst.clone()));
                    continue;
                }
                // a big copy cut off mid-way carries on from its last checkpoint, if the source
                // is as it was and the partial copy got that far
                let partial = if j.replace { &tmp } else { &j.dst };
                let from = checkpoints
                    .get(&i)
                    .filter(|(n, st)| src.len() == j.size && stamp(&src) == *st && std::fs::symlink_metadata(partial).is_ok_and(|m| m.is_file() && m.len() >= *n))
                    .map_or(0, |c| c.0);
                if from > 0 {
                    j.resume = from;
                } else {
                    let _ = std::fs::remove_file(&tmp);
                    // a partial copy from the cut (or a file the plan replaces): again, through
                    // a temporary name
                    j.replace = j.replace || dst_there;
                }
            }
        }
        index.push(i);
        p.jobs.push(j);
    }
    // a clean journal (no torn tail) to go on recording into
    let w = crate::journal::rewrite(journal, &l.records)?;
    drop(guard);
    let jr = Jr { w: Mutex::new((w, Instant::now())), path: journal.to_path_buf() };
    tracker.set_verify(l.info.verify);
    let threads = if l.rename { 4 } else { threads_for(&l.info.sources, &l.info.dest) };
    let out = run_jobs(p, &tracker, threads, Some((&jr, index)), already);
    let kind = if l.rename { Transfer::Move } else { l.info.kind };
    let r = finish(kind, &l.info.sources, &l.info.dest, &tracker, out);
    // done, failed or cancelled by the user: either way nothing is left to resume
    jr.close();
    for s in &l.info.sources {
        if let Some(p) = s.parent() {
            crate::shell::notify_updated(p);
        }
    }
    crate::shell::notify_updated(&l.info.dest);
    r
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicBool;

    fn tracker() -> Tracker {
        Tracker::new(Box::new(|_, _, _| {}), Arc::new(AtomicBool::new(false)))
    }

    fn journals() -> PathBuf {
        let d = std::env::temp_dir().join(format!("rf-journals-test-{}", std::process::id()));
        set_journal_dir(d.clone());
        JOURNAL_DIR.get().unwrap().clone()
    }

    fn test_run<'a>(t: &'a Tracker, jr: &'a Jr) -> Run<'a> {
        let tag = jr.path.file_stem().unwrap().to_string_lossy().into_owned();
        Run { tracker: t, tag, journal: Some(jr), total: 1, done: AtomicU64::new(0), cancel: AtomicI32::new(0), clock: Mutex::new((Instant::now(), 0)), last_report: Mutex::new(Instant::now()) }
    }

    fn rfparts(dir: &Path) -> usize {
        std::fs::read_dir(dir).unwrap().flatten().filter(|e| e.file_name().to_string_lossy().ends_with(".rfpart")).count()
    }

    /// A copy cut off half-way (some files finished and recorded, one partial at its real name,
    /// a replace whose temporary copy was left behind), then resumed: the result is the same as
    /// an uncut copy, nothing is asked again, the user's "replace" answer still holds, and the
    /// journal is gone.
    #[test]
    fn resume_after_a_crash() {
        let b = base("resume");
        let src = b.join("kaynak");
        tree(&src);
        let dst = b.join("hedef");
        std::fs::create_dir_all(dst.join("kaynak")).unwrap();
        // an older file the user chose to replace
        std::fs::write(dst.join("kaynak/kök.txt"), b"eski").unwrap();
        journals();
        let t = answering(Resolve::Replace);
        let mut p = plan(std::slice::from_ref(&src), &dst, false);
        let list: Vec<Conflict> = p.conflicts.iter().map(|c| c.0.clone()).collect();
        p.resolve(&vec![Resolve::Replace; list.len()]);
        p.jobs = order(std::mem::take(&mut p.jobs));
        let jr = journal_start(Transfer::Copy, false, Verify::Full, std::slice::from_ref(&src), &dst, &p).expect("journal");
        assert!(crate::journal::in_use(&jr.path), "a running job's journal is locked");
        for d in &p.dirs {
            let _ = std::fs::create_dir(d);
        }
        // the "crash": a third of the files done (most recorded), one partial, one torn temp
        let run = test_run(&t, &jr);
        let n = p.jobs.len();
        for (i, j) in p.jobs.iter().enumerate().take(n / 3) {
            run_job(j, &run, Some(i)).unwrap();
            if i % 4 != 0 {
                jr.done(i); // every 4th finished file's record is lost
            }
        }
        let partial = p.jobs[n / 3].clone();
        std::fs::write(&partial.dst, b"yarim").unwrap();
        let rep = p.jobs.iter().find(|j| j.replace).expect("the replace job").dst.clone();
        std::fs::write(part_name(&rep, &run.tag), b"yarim").unwrap();
        drop(run);
        jr.sync();
        let journal = jr.path.clone();
        drop(jr); // the process dies: the journal stays
        let found = interrupted().into_iter().find(|i| i.journal == journal).expect("the cut job is found");
        assert_eq!(found.dest, dst);
        assert_eq!(found.sources, vec![src.clone()]);
        assert!(found.done > 0 && found.done < found.files);
        // resume: nothing is asked (an `ask` that panics proves it)
        let mut t2 = tracker();
        t2.ask = Some(Box::new(|_| panic!("resume must not ask")));
        assert!(resume(&journal, Arc::new(t2)).unwrap());
        same(&src, &dst.join("kaynak"));
        assert!(!journal.exists(), "the journal is removed once the job is done");
        assert_eq!(rfparts(&dst.join("kaynak")), 0);
        let _ = std::fs::remove_dir_all(&b);
    }

    /// A big file cut off mid-way carries on from its last checkpoint: in place (same inode)
    /// for a new file, in its temporary copy for a replace; bytes past the checkpoint (written
    /// but maybe never synced) are cut off and copied again. A source changed since starts over.
    #[test]
    fn big_files_resume_mid_file() {
        use std::os::unix::fs::MetadataExt;
        let b = base("midfile");
        let src = b.join("kaynak");
        std::fs::create_dir_all(&src).unwrap();
        let data: Vec<u8> = (0..5_000_000u32).map(|i| (i * 7 % 253) as u8).collect();
        for n in ["yeni.bin", "eski.bin", "değişti.bin"] {
            std::fs::write(src.join(n), &data).unwrap();
        }
        let dst = b.join("hedef");
        std::fs::create_dir_all(dst.join("kaynak")).unwrap();
        std::fs::write(dst.join("kaynak/eski.bin"), b"eski hali").unwrap();
        journals();
        let _t = answering(Resolve::Replace);
        let mut p = plan(std::slice::from_ref(&src), &dst, false);
        p.resolve(&vec![Resolve::Replace; p.conflicts.len()]);
        p.jobs = order(std::mem::take(&mut p.jobs));
        let jr = journal_start(Transfer::Copy, false, Verify::Off, std::slice::from_ref(&src), &dst, &p).expect("journal");
        let tag = jr.path.file_stem().unwrap().to_string_lossy().into_owned();
        // the "crash": each copy has 3 MB on the disk, the first 2 MB vouched for by a checkpoint
        let mut inodes = Vec::new();
        for (i, j) in p.jobs.iter().enumerate() {
            let at = if j.replace { part_name(&j.dst, &tag) } else { j.dst.clone() };
            let mut torn = data[..2 << 20].to_vec();
            torn.extend(std::iter::repeat_n(0xEEu8, 1 << 20));
            std::fs::write(&at, &torn).unwrap();
            jr.checkpoint(i, 2 << 20, &stamp(&std::fs::metadata(&j.src).unwrap()));
            inodes.push((j.dst.clone(), std::fs::metadata(&at).unwrap().ino()));
        }
        let journal = jr.path.clone();
        drop(jr);
        // one source changes after the cut: its checkpoint no longer holds
        let changed = src.join("değişti.bin");
        std::fs::File::options().write(true).open(&changed).unwrap().set_modified(SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000)).unwrap();
        assert!(resume(&journal, Arc::new(tracker())).unwrap());
        same(&src, &dst.join("kaynak"));
        for (d, ino) in inodes {
            let now = std::fs::metadata(&d).unwrap().ino();
            if d.ends_with("değişti.bin") {
                assert_ne!(now, ino, "a changed source is copied again, through a temporary name");
            } else {
                assert_eq!(now, ino, "{} carried on in its partial copy", d.display());
            }
        }
        assert_eq!(rfparts(&dst.join("kaynak")), 0);
        assert!(!journal.exists());
        let _ = std::fs::remove_dir_all(&b);
    }

    /// Checkpoints are written while a big journaled file copies.
    #[test]
    fn big_copies_record_checkpoints() {
        let b = base("checkpoints");
        let src = b.join("büyük.bin");
        std::fs::write(&src, vec![7u8; 3 << 20]).unwrap();
        let dst = b.join("hedef");
        std::fs::create_dir_all(&dst).unwrap();
        journals();
        let t = tracker();
        let p = plan(std::slice::from_ref(&src), &dst, false);
        let jr = journal_start(Transfer::Copy, false, Verify::Off, std::slice::from_ref(&src), &dst, &p).expect("journal");
        let run = test_run(&t, &jr);
        run_job(&p.jobs[0], &run, Some(0)).unwrap();
        drop(run);
        jr.sync();
        let records = crate::journal::read(&jr.path).unwrap();
        let ck: Vec<u64> = records.iter().filter(|r| r[0] == "C").map(|r| r[2].parse().unwrap()).collect();
        assert!(ck.len() >= 2 && ck.windows(2).all(|w| w[0] < w[1]) && *ck.last().unwrap() == 3 << 20, "{ck:?}");
        jr.close();
        let _ = std::fs::remove_dir_all(&b);
    }

    /// Linux names can hold tabs, line breaks, backslashes and bytes that aren't UTF-8.
    #[test]
    fn journal_paths_round_trip() {
        use std::os::unix::ffi::OsStringExt;
        for raw in [&b"/a b/\xc3\xa7.txt"[..], b"/tab\there", b"/line\nbreak", b"/back\\slash\\x41", b"/bad\xff\xfe utf8", b"/trailing\\"] {
            let p = PathBuf::from(std::ffi::OsString::from_vec(raw.to_vec()));
            let s = ps(&p);
            assert!(!s.contains(['\t', '\n']), "{s:?}");
            assert_eq!(pp(&s), p, "{s:?}");
        }
    }

    /// A job being resumed (its journal locked) is neither listed nor resumed a second time.
    #[test]
    fn a_running_resume_is_left_alone() {
        let b = base("twice");
        let src = b.join("dosya.txt");
        std::fs::write(&src, b"bir").unwrap();
        let dst = b.join("hedef");
        std::fs::create_dir_all(&dst).unwrap();
        journals();
        let p = plan(std::slice::from_ref(&src), &dst, false);
        let jr = journal_start(Transfer::Copy, false, Verify::Off, std::slice::from_ref(&src), &dst, &p).expect("journal");
        let journal = jr.path.clone();
        drop(jr);
        let held = crate::journal::Writer::append(&journal).unwrap();
        assert!(!interrupted().iter().any(|i| i.journal == journal));
        assert!(resume(&journal, Arc::new(tracker())).is_err());
        drop(held);
        assert!(interrupted().iter().any(|i| i.journal == journal));
        assert!(resume(&journal, Arc::new(tracker())).unwrap());
        assert_eq!(std::fs::read(dst.join("dosya.txt")).unwrap(), b"bir");
        let _ = std::fs::remove_dir_all(&b);
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
        let (out, jr) = execute(Transfer::Move, &[src.clone(), lone.clone()], &dst, &t, 8, false);
        assert!(finish(Transfer::Move, &[src.clone(), lone.clone()], &dst, &t, out).unwrap());
        if let Some(j) = jr {
            j.close();
        }
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
        let j = |d: &str, f: &str, size: u64| Job { src: PathBuf::from(format!("s/{d}/{f}")), dst: PathBuf::from(format!("d/{d}/{f}")), size, op: Op::Copy, replace: false, resume: 0 };
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
