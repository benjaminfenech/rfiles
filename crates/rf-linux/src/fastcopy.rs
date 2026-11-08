//! Our own copy / move engine: many files at once with `CopyFile2` (attributes, times and
//! alternate data streams come along), instead of the shell engine's one-file-at-a-time
//! pipeline. Names that already exist at the destination are ours to handle too (like Files):
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
// Generated from rf-win's public API by a one-off stub generator, then edited by hand.
// Functions return safe defaults until they get a Linux implementation.
#![allow(unused)]
use std::cell::Cell;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicI32, AtomicU64, AtomicUsize, Ordering};
use std::time::{Duration, Instant, SystemTime};
use crate::fileops::{Tracker, Transfer};
use crate::wide::long_path;
/// Files above this size are copied unbuffered (no point filling the cache with them).
const UNBUFFERED: u64 = 256 << 20;
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
fn key(p: &Path) -> String {
    p.to_string_lossy().to_lowercase()
}
fn is_reparse(m: &std::fs::Metadata) -> bool {
    false
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
        if meta.is_dir() && inside(dest_dir, src) {
            self.whole.push(src.to_path_buf());
            return;
        }
        let mut sub = Plan {
            taken: std::mem::take(&mut self.taken),
            ..Default::default()
        };
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
    fn visit(
        &mut self,
        src: &Path,
        dst: &Path,
        meta: &std::fs::Metadata,
        rename: bool,
    ) -> Result<(), HasLink> {
        // Phase 1: the copy engine. Until then every source goes to the fallback.
        Err(HasLink)
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
                    self.jobs
                        .push(Job {
                            src: a,
                            dst: b,
                            size: m.len(),
                            op: Op::Copy,
                            replace: false,
                        });
                }
            }
        }
        Ok(())
    }
    /// Applies the user's answers to the conflicts.
    fn resolve(&mut self, answers: &[Resolve]) {
        let conflicts = std::mem::take(&mut self.conflicts);
        for ((c, mut job), r) in conflicts.into_iter().zip(answers.iter().copied()) {
            let r = if r == Resolve::Replace && c.mismatched() {
                Resolve::KeepBoth
            } else {
                r
            };
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
        let name = dst
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let (stem, ext) = match name.rfind('.') {
            Some(i) if i > 0 && !is_dir => (name[..i].to_string(), name[i..].to_string()),
            _ => (name.clone(), String::new()),
        };
        (2..)
            .map(|n| dir.join(format!("{stem} ({n}){ext}")))
            .find(|p| {
                !self.taken.contains(&key(p)) && std::fs::symlink_metadata(p).is_err()
            })
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
    let (mut big, small): (Vec<Job>, Vec<Job>) = jobs
        .into_iter()
        .partition(|j| j.size >= BIG);
    big.sort_by_key(|j| std::cmp::Reverse(j.size));
    let mut folders: Vec<std::collections::VecDeque<Job>> = Vec::new();
    let mut at: std::collections::HashMap<PathBuf, usize> = Default::default();
    for j in small {
        let parent = j.dst.parent().map(Path::to_path_buf).unwrap_or_default();
        let ix = *at
            .entry(parent)
            .or_insert_with(|| {
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
    /// Turned non-zero to stop CopyFile2 (it removes the partial file).
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
        (self
            .tracker
            .on_progress)(
            1000,
            permille,
            name.and_then(|n| n.file_name()).map(|n| n.to_string_lossy().into_owned()),
        );
    }
}
/// CopyFile2's per-file callback context.
struct FileCtx<'a> {
    run: &'a Run<'a>,
    name: &'a Path,
    sent: Cell<u64>,
}
fn copy_file(src: &Path, dst: &Path, size: u64, run: &Run) -> anyhow::Result<()> {
    Err(anyhow::anyhow!("rf-linux: `copy_file` is not implemented on Linux yet").into())
}
/// Moves `from` over `to` (replacing it when asked), clearing a read-only flag in the way.
fn move_into_place(from: &Path, to: &Path, replace: bool) -> anyhow::Result<()> {
    Err(
        anyhow::anyhow!("rf-linux: `move_into_place` is not implemented on Linux yet")
            .into(),
    )
}
/// A temporary name next to `dst` for a copy that replaces it.
fn part_name(dst: &Path) -> PathBuf {
    let name = dst
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    dst.with_file_name(format!(".{name}.{}.rfpart", std::process::id()))
}
/// One job; `Err` = leave it to the shell.
fn run_job(j: &Job, run: &Run) -> anyhow::Result<()> {
    Err(anyhow::anyhow!("rf-linux: `run_job` is not implemented on Linux yet").into())
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
fn execute(
    sources: &[PathBuf],
    dest_dir: &Path,
    tracker: &Tracker,
    threads: usize,
    rename: bool,
) -> Outcome {
    // Phase 1: the copy engine. Until then everything is left for the fallback.
    Outcome { whole: sources.to_vec(), ..Default::default() }
}
/// Copies `sources` into `dest_dir` in parallel (conflicts as `tracker.ask` says; without it
/// both are kept). Returns the undo pairs and the leftovers for the shell.
pub fn copy(
    sources: &[PathBuf],
    dest_dir: &Path,
    tracker: &Tracker,
    threads: usize,
) -> Outcome {
    execute(sources, dest_dir, tracker, threads, false)
}
/// Copy or move with our engine where it can, the shell engine for the rest. Returns like
/// `fileops::transfer_tracked`; undo pairs land in `tracker.results`. A verification failure
/// is an `Err` (the copies stay).
pub fn transfer(
    kind: Transfer,
    sources: &[PathBuf],
    dest_dir: &Path,
    tracker: Arc<Tracker>,
) -> anyhow::Result<bool> {
    let same_dir = sources.iter().all(|s| s.parent() == Some(dest_dir));
    if same_dir || !local_fs(dest_dir) || !sources.iter().all(|s| local_fs(s)) {
        return crate::fileops::transfer_tracked(kind, sources, dest_dir, tracker);
    }
    let (here, there): (Vec<PathBuf>, Vec<PathBuf>) = sources
        .iter()
        .cloned()
        .partition(|s| kind == Transfer::Move && same_volume(s, dest_dir));
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
fn finish(
    kind: Transfer,
    sources: &[PathBuf],
    dest_dir: &Path,
    tracker: &Arc<Tracker>,
    out: Outcome,
) -> anyhow::Result<bool> {
    tracker.results.lock().unwrap().extend(out.undo.iter().cloned());
    if out.cancelled || tracker.cancel.load(Ordering::Relaxed) {
        return Ok(false);
    }
    let mut ok = true;
    if !out.whole.is_empty() {
        ok
            &= crate::fileops::transfer_tracked(
                kind,
                &out.whole,
                dest_dir,
                tracker.clone(),
            )?;
    }
    let mut by_dir: Vec<(PathBuf, Vec<PathBuf>)> = Vec::new();
    for (f, d) in &out.failed {
        match by_dir.iter_mut().find(|(x, _)| x == d) {
            Some((_, v)) => v.push(f.clone()),
            None => by_dir.push((d.clone(), vec![f.clone()])),
        }
    }
    for (d, fs) in by_dir {
        let t = Arc::new(Tracker::new(Box::new(|_, _, _| {}), tracker.cancel.clone()));
        ok &= crate::fileops::transfer_tracked(kind, &fs, &d, t)?;
    }
    let mode = match (tracker.verify_mode(), kind) {
        (Verify::Off, Transfer::Move) => Verify::Quick,
        (m, _) => m,
    };
    let bad = check(&out.copied, mode, tracker);
    if tracker.cancel.load(Ordering::Relaxed) {
        return Ok(false);
    }
    if kind == Transfer::Move {
        let bad_set: HashSet<&PathBuf> = bad.iter().collect();
        let files: Vec<PathBuf> = out
            .copied
            .iter()
            .filter(|(s, _)| !bad_set.contains(s))
            .map(|(s, _)| s.clone())
            .collect();
        let mut dirs: Vec<PathBuf> = out.src_dirs.clone();
        for (s, _) in &out.undo {
            if s.is_dir() && sources.iter().any(|t| s.starts_with(t)) {
                walk_dirs(s, &mut dirs);
            }
        }
        dirs.sort_by_key(|d| d.components().count());
        dirs.dedup();
        if !crate::fastdelete::remove_verified(&files, &dirs) {
            log::info!(
                "fastcopy: some sources stay (skipped, failed or changed meanwhile)"
            );
        }
    }
    if !bad.is_empty() {
        let names: Vec<String> = bad
            .iter()
            .take(5)
            .filter_map(|p| p.file_name())
            .map(|n| n.to_string_lossy().into_owned())
            .collect();
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
    let threads = std::thread::available_parallelism()
        .map_or(4, |n| n.get())
        .clamp(2, 8);
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
                        log::warn!(
                            "fastcopy: {} does not match {}", b.display(), a.display()
                        );
                        bad.lock().unwrap().push(a.clone());
                    }
                    if mode == Verify::Full && i % 16 == 0 {
                        (tracker
                            .on_progress)(1000, (i * 1000 / pairs.len()) as u32, None);
                    }
                }
            });
        }
    });
    bad.into_inner().unwrap()
}
/// Same size and modification time (CopyFile2 keeps it; 2 s of slack for FAT/exFAT sticks).
fn same_meta(a: &Path, b: &Path) -> bool {
    let (Ok(ma), Ok(mb)) = (std::fs::metadata(a), std::fs::metadata(b)) else {
        return false
    };
    if ma.len() != mb.len() {
        return false;
    }
    match (ma.modified(), mb.modified()) {
        (Ok(x), Ok(y)) => {
            x.duration_since(y)
                .or_else(|_| y.duration_since(x))
                .is_ok_and(|d| d <= Duration::from_secs(2))
        }
        _ => false,
    }
}
/// Same bytes; the copy is read past the cache (what the disk holds, not what was just
/// written to memory).
fn same_bytes(a: &Path, b: &Path) -> std::io::Result<bool> {
    Err(
        std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "rf-linux: `same_bytes` is not implemented on Linux yet",
        ),
    )
}
/// `inner` is `outer` or inside it, comparing like Windows does (case-insensitive, either
/// separator).
fn inside(inner: &Path, outer: &Path) -> bool {
    let norm = |p: &Path| {
        p.to_string_lossy().replace('/', "\\").trim_end_matches('\\').to_lowercase()
    };
    let (i, o) = (norm(inner), norm(outer));
    i == o || i.strip_prefix(&o).is_some_and(|rest| rest.starts_with('\\'))
}
/// Same drive letter or same `\\server\share`: a move there is a rename.
fn same_volume(a: &Path, b: &Path) -> bool {
    use std::path::Component;
    let root = |p: &Path| match p.components().next() {
        Some(Component::Prefix(x)) => {
            Some(x.as_os_str().to_string_lossy().to_lowercase())
        }
        _ => None,
    };
    root(a).is_some() && root(a) == root(b)
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
/// Whether `p` is on a local or network file system path we can copy into directly (not a
/// shell namespace, library, phone, …).
fn local_fs(p: &Path) -> bool {
    // Phase 1: our engine on Linux. Until then nothing is "local" and the fallback reports it.
    if cfg!(unix) {
        return false;
    }
    let s = p.to_string_lossy();
    (s.len() >= 3 && s.as_bytes()[1] == b':')
        || s.starts_with(r"\\") && !s.starts_with(r"\\?\")
}
