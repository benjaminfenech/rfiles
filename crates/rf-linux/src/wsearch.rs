//! Windows Search (the system index) — what Files and Explorer use for fast name *and content*
//! search with AQS ("kind:picture", "size:>10MB", "datemodified:this week").
//!
//! The query helper of the index turns the user's AQS into SQL (Windows handles the syntax,
//! localized keywords included) and the SQL runs through the index's OLE DB provider. Only used
//! for folders inside the crawl scope: for anything else Windows would fall back to a slow
//! crawl, and our own walker is faster. Blocking: call off the UI thread.
// Generated from rf-win's public API by a one-off stub generator, then edited by hand.
// Functions return safe defaults until they get a Linux implementation.
#![allow(unused)]
use std::path::{Path, PathBuf};
use rf_core::{Entry, EntryKind};
use crate::search::SearchHit;
use crate::shell::ensure_com;
use crate::wide::wide;
const CLSID_COLLATOR_DSO: crate::Opaque = 0;
const DBGUID_DEFAULT: crate::Opaque = 0;
fn manager() -> anyhow::Result<crate::Opaque> {
    Err(anyhow::anyhow!("rf-linux: `manager` is not implemented on Linux yet").into())
}
/// Whether `root` is inside the Windows Search crawl scope (and the service answers).
pub fn is_indexed(root: &Path) -> bool {
    false
}
/// The SQL for `query` restricted to `root` (and below).
fn sql_for(root: &Path, query: &str, max: usize) -> anyhow::Result<String> {
    Err(anyhow::anyhow!("rf-linux: `sql_for` is not implemented on Linux yet").into())
}
const PATH_CHARS: usize = 1024;
const OFF_PATH: usize = 0;
const OFF_PATH_LEN: usize = PATH_CHARS * 2;
const OFF_PATH_ST: usize = OFF_PATH_LEN + 8;
const OFF_SIZE: usize = OFF_PATH_ST + 8;
const OFF_SIZE_ST: usize = OFF_SIZE + 8;
const OFF_MOD: usize = OFF_SIZE_ST + 8;
const OFF_MOD_ST: usize = OFF_MOD + 8;
const OFF_CRE: usize = OFF_MOD_ST + 8;
const OFF_CRE_ST: usize = OFF_CRE + 8;
const OFF_ATTR: usize = OFF_CRE_ST + 8;
const OFF_ATTR_ST: usize = OFF_ATTR + 4;
const ROW: usize = OFF_ATTR_ST + 4;
fn rd<T: Copy>(buf: &[u8], off: usize) -> T {
    unsafe { std::ptr::read_unaligned(buf.as_ptr().add(off) as *const T) }
}
fn parse_row(buf: &[u8]) -> Option<SearchHit> {
    None
}
/// Runs `query` (AQS or plain words; names and contents) over `root` and its subfolders
/// through the index. Returns the number of hits.
pub fn search(
    root: &Path,
    query: &str,
    max_results: usize,
    cancelled: &dyn Fn() -> bool,
    on_batch: &mut dyn FnMut(Vec<SearchHit>),
) -> anyhow::Result<usize> {
    Err(anyhow::anyhow!("rf-linux: `search` is not implemented on Linux yet").into())
}
