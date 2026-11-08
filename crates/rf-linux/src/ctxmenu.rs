//! Native Explorer context menu (IContextMenu), shown from the shell STA thread.
// Generated from rf-win's public API by a one-off stub generator, then edited by hand.
// Functions return safe defaults until they get a Linux implementation.
#![allow(unused)]
use std::path::{Path, PathBuf};
const CMIC_MASK_UNICODE: u32 = 0x4000;
use crate::wide::wide;
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MenuOutcome {
    Dismissed,
    /// User chose "Rename": the app should start inline rename.
    Rename,
    /// A shell verb ran (e.g. "delete", "copy", "properties", or a handler without a verb).
    Invoked(String),
    /// One of the app's own items (see `ExtraItem`) was chosen.
    Custom(u32),
}
/// App-provided entries shown above the shell's items. `id` is returned in `MenuOutcome::Custom`.
#[derive(Debug, Clone)]
pub struct ExtraItem {
    pub id: u32,
    pub label: String,
    pub enabled: bool,
}
const EXTRA_BASE: u32 = 0x9000;
struct Pidl(*mut crate::Opaque);
impl Drop for Pidl {
    fn drop(&mut self) {}
}
fn parse(path: &Path) -> anyhow::Result<Pidl> {
    Err(anyhow::anyhow!("rf-linux: `parse` is not implemented on Linux yet").into())
}
fn folder(path: &Path) -> anyhow::Result<crate::Opaque> {
    Err(anyhow::anyhow!("rf-linux: `folder` is not implemented on Linux yet").into())
}
fn menu_for_items(
    owner: crate::Opaque,
    items: &[PathBuf],
) -> anyhow::Result<crate::Opaque> {
    Err(
        anyhow::anyhow!("rf-linux: `menu_for_items` is not implemented on Linux yet")
            .into(),
    )
}
/// Bound through the item's real parent in the shell namespace, so drive roots (`C:\`,
/// parent = "This PC") work too.
fn menu_for_one(owner: crate::Opaque, item: &Path) -> anyhow::Result<crate::Opaque> {
    Err(
        anyhow::anyhow!("rf-linux: `menu_for_one` is not implemented on Linux yet")
            .into(),
    )
}
fn menu_for_background(
    owner: crate::Opaque,
    dir: &Path,
) -> anyhow::Result<crate::Opaque> {
    Err(
        anyhow::anyhow!(
            "rf-linux: `menu_for_background` is not implemented on Linux yet"
        )
            .into(),
    )
}
/// Must be called on the shell STA thread (see `sta::run`) with its owner window.
pub fn show_for_items(
    owner: crate::Opaque,
    items: &[PathBuf],
    screen: (i32, i32),
    extended: bool,
    extra: &[ExtraItem],
) -> anyhow::Result<MenuOutcome> {
    Err(
        anyhow::anyhow!("rf-linux: `show_for_items` is not implemented on Linux yet")
            .into(),
    )
}
/// Menu for one item (used by the sidebar).
pub fn show_for_one(
    owner: crate::Opaque,
    item: &Path,
    screen: (i32, i32),
    extended: bool,
    extra: &[ExtraItem],
) -> anyhow::Result<MenuOutcome> {
    Err(
        anyhow::anyhow!("rf-linux: `show_for_one` is not implemented on Linux yet")
            .into(),
    )
}
/// Background (empty space) menu of a folder.
pub fn show_for_background(
    owner: crate::Opaque,
    dir: &Path,
    screen: (i32, i32),
    extended: bool,
    extra: &[ExtraItem],
) -> anyhow::Result<MenuOutcome> {
    Err(
        anyhow::anyhow!(
            "rf-linux: `show_for_background` is not implemented on Linux yet"
        )
            .into(),
    )
}
/// What a shell menu is built for.
#[derive(Debug, Clone)]
pub enum ShellTarget {
    /// Items of one folder (the selection).
    Items(Vec<PathBuf>),
    /// A single item through its real parent (sidebar entries, drive roots).
    One(PathBuf),
    /// The empty area of a folder.
    Background(PathBuf),
}
/// A menu bitmap as top-down BGRA rows (premultiplied, as the shell hands them out).
#[derive(Debug, Clone)]
pub struct ShellIcon {
    pub width: u32,
    pub height: u32,
    pub bgra: Vec<u8>,
}
#[derive(Debug, Clone)]
pub struct ShellItem {
    /// Command id for [`invoke`]; 0 for separators and submenus.
    pub id: u32,
    pub label: String,
    /// Text after a tab in the label (shortcut hint), if any.
    pub hint: Option<String>,
    /// Canonical verb ("open", "properties", "Windows.ModernShare"…), empty if the handler has none.
    pub verb: String,
    pub icon: Option<ShellIcon>,
    pub enabled: bool,
    pub separator: bool,
    pub children: Vec<ShellItem>,
}
struct Session {
    id: u64,
    cm: crate::Opaque,
    hmenu: crate::Opaque,
}
impl Drop for Session {
    fn drop(&mut self) {}
}
thread_local! {
    #[doc =
    " The one live shell menu (kept until invoked, released, or replaced by the next one)."]
    static SESSION : std::cell::RefCell < Option < Session >> = const {
    std::cell::RefCell::new(None) };
}
static NEXT_SESSION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
fn context_menu_for(
    owner: crate::Opaque,
    target: &ShellTarget,
) -> anyhow::Result<(crate::Opaque, bool)> {
    Err(
        anyhow::anyhow!("rf-linux: `context_menu_for` is not implemented on Linux yet")
            .into(),
    )
}
/// Builds the shell menu for `target` and returns it as data plus a session id for
/// [`invoke`]. Submenus that fill themselves on demand (Send to, Open with…) are initialised
/// here. Call on the shell STA thread.
pub fn build(
    owner: crate::Opaque,
    target: &ShellTarget,
    extended: bool,
) -> anyhow::Result<(u64, Vec<ShellItem>)> {
    Err(anyhow::anyhow!("rf-linux: `build` is not implemented on Linux yet").into())
}
/// Runs command `id` of session `session` (from [`build`]). `screen` is where the menu was
/// opened (some handlers place their UI there). Call on the shell STA thread.
pub fn invoke(
    owner: crate::Opaque,
    session: u64,
    id: u32,
    screen: (i32, i32),
) -> anyhow::Result<MenuOutcome> {
    Err(anyhow::anyhow!("rf-linux: `invoke` is not implemented on Linux yet").into())
}
/// Drops the live session if it is `session` (menu closed without choosing a shell command).
pub fn release(session: u64) {
    SESSION
        .with(|s| {
            let mut s = s.borrow_mut();
            if s.as_ref().is_some_and(|x| x.id == session) {
                *s = None;
            }
        });
}
/// Invokes a canonical verb ("properties", "Windows.ModernShare", "openas"…) on `target`
/// without showing a menu. Call on the shell STA thread.
pub fn invoke_verb(
    owner: crate::Opaque,
    target: &ShellTarget,
    verb: &str,
    screen: (i32, i32),
) -> anyhow::Result<()> {
    Err(
        anyhow::anyhow!("rf-linux: `invoke_verb` is not implemented on Linux yet").into(),
    )
}
fn verb_of(cm: &crate::Opaque, offset: usize) -> String {
    Default::default()
}
fn invoke_offset(
    owner: crate::Opaque,
    cm: &crate::Opaque,
    offset: u32,
    _: (i32, i32),
) -> anyhow::Result<MenuOutcome> {
    Err(
        anyhow::anyhow!("rf-linux: `invoke_offset` is not implemented on Linux yet")
            .into(),
    )
}
/// "&Open\tCtrl+O" → ("Open", Some("Ctrl+O")); "&&" stays a literal ampersand.
pub fn clean_label(raw: &str) -> (String, Option<String>) {
    let (text, hint) = match raw.split_once('\t') {
        Some((t, h)) => (t, Some(h.trim().to_string()).filter(|h| !h.is_empty())),
        None => (raw, None),
    };
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '&' {
            if chars.peek() == Some(&'&') {
                chars.next();
                out.push('&');
            }
            continue;
        }
        out.push(c);
    }
    (out.trim().to_string(), hint)
}
unsafe fn read_menu(
    cm: &crate::Opaque,
    cm3: Option<&crate::Opaque>,
    cm2: Option<&crate::Opaque>,
    hmenu: crate::Opaque,
    depth: u32,
) -> Vec<ShellItem> {
    Default::default()
}
/// Reads a menu item bitmap as BGRA. Special `HBMMENU_*` values (owner-drawn callbacks,
/// system glyphs) yield `None`.
unsafe fn bitmap_bgra(hbm: crate::Opaque) -> Option<ShellIcon> {
    None
}
fn run_menu(
    owner: crate::Opaque,
    cm: &crate::Opaque,
    _: (i32, i32),
    extended: bool,
    can_rename: bool,
    extra: &[ExtraItem],
) -> anyhow::Result<MenuOutcome> {
    Err(anyhow::anyhow!("rf-linux: `run_menu` is not implemented on Linux yet").into())
}
/// A plain popup menu of app items at `screen` (no shell items). Returns the chosen id.
/// Call on the shell STA thread with its owner window.
pub fn show_simple(
    owner: crate::Opaque,
    screen: (i32, i32),
    items: &[ExtraItem],
) -> Option<u32> {
    None
}
