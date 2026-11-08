//! Recursive filename search ("in this folder and subfolders").
//!
//! Directories are walked in parallel on a dedicated rayon pool whose threads run in background
//! priority mode, so the search never competes with the user's foreground work. Results are
//! streamed in batches to the calling thread (which owns `on_batch` and `cancelled`).
//! Blocking: call off the UI thread.
// Generated from rf-win's public API by a one-off stub generator, then edited by hand.
// Functions return safe defaults until they get a Linux implementation.
#![allow(unused)]
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;
use crossbeam_channel::{RecvTimeoutError, Sender, unbounded};
use rf_core::{Entry, attrs};
use crate::listing::to_entry;
use crate::wide::long_path;
const BATCH: usize = 200;
#[derive(Debug, Clone)]
pub struct SearchQuery {
    pub root: PathBuf,
    /// Case-insensitive substring, or a glob when it contains `*` / `?` (matched against the whole name).
    pub pattern: String,
    pub include_hidden: bool,
    pub max_results: usize,
}
#[derive(Debug, Clone)]
pub struct SearchHit {
    pub path: PathBuf,
    pub entry: Entry,
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SearchStats {
    pub dirs_scanned: usize,
    pub entries_scanned: usize,
    pub hits: usize,
    /// Directories that could not be read (access denied etc.).
    pub skipped_dirs: usize,
    /// Stopped because `max_results` was reached.
    pub truncated: bool,
    pub cancelled: bool,
}
/// Lowercases and drops the combining dot above (U+0307) that `"İ".to_lowercase()` yields, so
/// "İ" and "i" compare equal.
fn fold(s: &str) -> Vec<char> {
    s.to_lowercase().chars().filter(|&c| c != '\u{307}').collect()
}
enum Matcher {
    All,
    Substr(Vec<char>),
    Glob(Vec<char>),
}
impl Matcher {
    fn new(pattern: &str) -> Self {
        let p = fold(pattern.trim());
        if p.is_empty() {
            Matcher::All
        } else if p.iter().any(|&c| c == '*' || c == '?') {
            Matcher::Glob(p)
        } else {
            Matcher::Substr(p)
        }
    }
    fn is_match(&self, name: &str) -> bool {
        false
    }
}
/// `*` = any run (including empty), `?` = exactly one char. Iterative with backtracking.
fn glob_match(p: &[char], n: &[char]) -> bool {
    let (mut pi, mut ni) = (0, 0);
    let (mut star, mut mark) = (usize::MAX, 0);
    while ni < n.len() {
        if pi < p.len() && p[pi] == '*' {
            star = pi;
            mark = ni;
            pi += 1;
        } else if pi < p.len() && (p[pi] == '?' || p[pi] == n[ni]) {
            pi += 1;
            ni += 1;
        } else if star != usize::MAX {
            pi = star + 1;
            mark += 1;
            ni = mark;
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }
    pi == p.len()
}
/// Enumerates `dir`, calling `f` per raw find record (including "." and ".."). Err = Win32 error code.
pub(crate) fn for_each_raw(
    dir: &Path,
    f: impl FnMut(&crate::Opaque),
) -> Result<(), u32> {
    // ENOSYS: callers log it and carry on
    Err(38)
}
struct Shared<'a> {
    matcher: &'a Matcher,
    include_hidden: bool,
    max: usize,
    stop: &'a AtomicBool,
    reserved: &'a AtomicUsize,
    dirs: &'a AtomicUsize,
    entries: &'a AtomicUsize,
    skipped: &'a AtomicUsize,
    tx: &'a Sender<Vec<SearchHit>>,
}
fn walk<'s>(sc: &rayon::Scope<'s>, sh: &'s Shared<'s>, dir: PathBuf) {}
/// Searches `q.root` recursively. `on_batch` receives ~200 hits at a time on the calling thread;
/// `cancelled` is polled there too (every few milliseconds). Hidden entries are skipped (and
/// hidden directories not entered) unless `include_hidden`; reparse points (junctions, symlinks)
/// are listed but never followed.
pub fn search(
    q: &SearchQuery,
    cancelled: &dyn Fn() -> bool,
    on_batch: &mut dyn FnMut(Vec<SearchHit>),
) -> anyhow::Result<SearchStats> {
    Err(anyhow::anyhow!("rf-linux: `search` is not implemented on Linux yet").into())
}
