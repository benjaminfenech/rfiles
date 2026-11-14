//! Our own Properties window, with the tabs of Windows' property sheet:
//! - General: name, type, the app it opens with (and "Change…"), location, size / size on disk /
//!   contents, times, and the Permissions section (owner, group, read / write / execute). A
//!   drive shows its file system and how full it is instead.
//! - Details: the version resource of programs, media / document properties, the file facts.
//! - Hashes (files).
//!
//! Every disk access happens on background threads; the window never waits on a slow drive.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use gpui_kit::assets::IconName as L;
use gpui_kit::component::input::{Input, InputState};
use rf_linux::usershare;
use gpui_kit::component::{Icon, TitleBar, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use rf_core::{Entry, EntryKind, format};
use rf_linux::fileinfo::Info;
use rf_linux::perms::{self, Perms};

use crate::image_cache::ImageCache;
use crate::t;
use crate::theme::{self, Palette};

/// Opens a Properties window for `paths` (one window per call), on the display of the window
/// in front.
pub fn open(paths: Vec<PathBuf>, images: Entity<ImageCache>, cx: &mut App) {
    if paths.is_empty() {
        return;
    }
    let display = cx.active_window().and_then(|w| w.update(cx, |_, w, cx| w.display(cx).map(|d| d.id())).ok().flatten());
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds::centered(display, size(px(500.), px(680.)), cx))),
        display_id: display,
        window_min_size: Some(size(px(420.), px(460.))),
        window_background: WindowBackgroundAppearance::MicaBackdrop,
        app_id: crate::sys::app_id("rfiles"),
        ..TitleBar::window_options()
    };
    if let Err(e) = gpui_kit::open_window(options, cx, move |window, cx| cx.new(|cx| PropsView::new(paths, images, window, cx))) {
        log::error!("properties window: {e:#}");
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Page {
    General,
    Details,
    Hashes,
    Sharing,
}

/// The Sharing page (a folder shared through Samba's user shares), read when the page opens.
#[derive(Default)]
enum ShareLoad {
    #[default]
    Idle,
    Loading,
    Ready(Option<usershare::Share>),
    Unavailable(usershare::Unavailable),
    Failed(String),
}

#[derive(Default)]
struct ShareEdit {
    load: ShareLoad,
    on: bool,
    writable: bool,
    guest: bool,
    name: Option<Entity<InputState>>,
}

impl ShareEdit {
    fn name(&self, cx: &App) -> String {
        self.name.as_ref().map(|n| n.read(cx).value().trim().to_string()).unwrap_or_default()
    }

    /// The share as edited, when that differs from the one there is (None: no change). The
    /// inner None stops sharing.
    fn change(&self, cx: &App) -> Option<Option<(String, bool, bool)>> {
        let ShareLoad::Ready(cur) = &self.load else { return None };
        let want = self.on.then(|| (self.name(cx), self.writable, self.guest));
        let have = cur.as_ref().map(|s| (s.name.clone(), s.writable, s.guest));
        (want != have).then_some(want)
    }
}

/// Totals for a multi-selection.
#[derive(Default, Clone)]
struct Totals {
    files: u64,
    dirs: u64,
    bytes: u64,
    on_disk: u64,
    /// Selected folders (the selection itself, not what is inside).
    sel_dirs: u64,
    /// None before the first file; Some(Some(ext)) while every selected file has `ext`
    /// ("" = none); Some(None) once they differ.
    same_ext: Option<Option<String>>,
    done: bool,
}

/// A checkbox over one attribute bit: None = mixed across the selection; `was` = on load.
#[derive(Clone, Copy, Default, PartialEq)]
struct Flag {
    now: Option<bool>,
    was: Option<bool>,
}

impl Flag {
    fn load(v: Option<bool>) -> Self {
        Flag { now: v, was: v }
    }
    fn changed(&self) -> bool {
        self.now != self.was
    }
    fn toggle(&mut self) {
        self.now = Some(!self.now.unwrap_or(false));
    }
    fn commit(&mut self) {
        self.was = self.now;
    }
}

/// The permission bits of the grid: owner, group, others × read, write, execute.
const BITS: [u32; 9] = [0o400, 0o200, 0o100, 0o040, 0o020, 0o010, 0o004, 0o002, 0o001];

/// The selection's permissions folded together: a value the items share, or None where they
/// differ.
#[derive(Clone, Default)]
struct Fold {
    bits: [Option<bool>; 9],
    owner: Option<String>,
    group: Option<(u32, String)>,
    any_dir: bool,
    any_file: bool,
    /// Whether this user may change every item (owns it, or is root).
    mine: bool,
    n: usize,
}

impl Fold {
    fn add(&mut self, p: &Perms) {
        let first = self.n == 0;
        for (slot, bit) in self.bits.iter_mut().zip(BITS) {
            let on = p.mode & bit != 0;
            *slot = if first || *slot == Some(on) { Some(on) } else { None };
        }
        self.owner = if first || self.owner.as_deref() == Some(p.owner.as_str()) { Some(p.owner.clone()) } else { None };
        self.group = if first || self.group.as_ref().map(|g| g.0) == Some(p.gid) { Some((p.gid, p.group.clone())) } else { None };
        self.any_dir |= p.is_dir;
        self.any_file |= !p.is_dir;
        self.mine = (first || self.mine) && p.changeable();
        self.n += 1;
    }
}

/// The Permissions section's checkboxes and group, as loaded and as edited.
#[derive(Clone, Default)]
struct PermEdit {
    fold: Fold,
    bits: [Flag; 9],
    /// A group picked from the list.
    new_gid: Option<u32>,
}

impl PermEdit {
    fn from(fold: Fold) -> Self {
        let bits = fold.bits.map(Flag::load);
        PermEdit { fold, bits, new_gid: None }
    }
    fn loaded(&self) -> bool {
        self.fold.n > 0
    }
    fn change(&self) -> perms::Change {
        let mut ch = perms::Change::default();
        for (f, bit) in self.bits.iter().zip(BITS) {
            if let (true, Some(on)) = (f.changed(), f.now) {
                if on {
                    ch.set |= bit;
                } else {
                    ch.clear |= bit;
                }
            }
        }
        ch.gid = self.new_gid.filter(|g| self.fold.group.as_ref().map(|x| x.0) != Some(*g));
        ch
    }
    fn changed(&self) -> bool {
        !self.change().is_empty()
    }
    /// "Allow running as a program": on gives execute to whoever may read (the owner always),
    /// off takes it from everyone.
    fn set_executable(&mut self, on: bool) {
        for class in 0..3 {
            let (r, x) = (class * 3, class * 3 + 2);
            if !on {
                self.bits[x].now = Some(false);
            } else if class == 0 || self.bits[r].now == Some(true) {
                self.bits[x].now = Some(true);
            }
        }
    }
    fn commit(&mut self, groups: &[(u32, String)]) {
        for f in &mut self.bits {
            f.commit();
        }
        for (slot, f) in self.fold.bits.iter_mut().zip(&self.bits) {
            *slot = f.now;
        }
        if let Some(g) = self.new_gid.take() {
            let name = groups.iter().find(|x| x.0 == g).map_or_else(|| g.to_string(), |x| x.1.clone());
            self.fold.group = Some((g, name));
        }
    }
}

/// What a single item's background load brings.
#[derive(Default)]
struct Loaded {
    info: Option<Result<Info, String>>,
    perms: Option<Perms>,
    drive: Option<rf_linux::drives::Drive>,
    pe: Option<rf_linux::pe::PeInfo>,
}

pub struct PropsView {
    paths: Vec<PathBuf>,
    images: Entity<ImageCache>,
    page: Page,
    info: Option<Result<Info, String>>,
    folder: Option<Option<rf_linux::foldersize::FolderSize>>,
    totals: Option<Totals>,
    media: Vec<(&'static str, String)>,
    name: Option<Entity<InputState>>,
    perm: PermEdit,
    /// The groups this user may give items to.
    groups: Vec<(u32, String)>,
    /// A folder's permission change goes down its whole tree too.
    recurse: bool,
    drive: Option<rf_linux::drives::Drive>,
    pe: Option<rf_linux::pe::PeInfo>,
    hash: Option<Entity<crate::hash_panel::HashPanel>>,
    share: ShareEdit,
    error: Option<SharedString>,
    busy: bool,
    frame_dark: Option<bool>,
    cancel: Arc<AtomicBool>,
    focus: FocusHandle,
}

impl Drop for PropsView {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

fn file_name(p: &Path) -> String {
    p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| p.to_string_lossy().into_owned())
}

fn ext_of(p: &Path) -> String {
    p.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default()
}

/// Programs and other files that carry a version resource.
fn is_pe(p: &Path) -> bool {
    matches!(ext_of(p).as_str(), "exe" | "dll" | "sys" | "ocx" | "cpl" | "scr" | "drv" | "efi" | "mui" | "ax" | "com")
}

/// The root, "/" (other drives are mount points, found in the drive list instead).
fn is_root(p: &Path) -> bool {
    p.parent().is_none()
}

/// Text with a breaking point after each backslash (long paths wrap at folders).
fn breakable(s: &str) -> String {
    // Linux: after `/` too
    crate::sys::wrap_path(s)
}

impl PropsView {
    fn new(paths: Vec<PathBuf>, images: Entity<ImageCache>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        cx.observe(&images, |_, _, cx| cx.notify()).detach();
        let single = paths.len() == 1 && !is_root(&paths[0]);
        let name = single.then(|| {
            let n = file_name(&paths[0]);
            cx.new(|cx| {
                let mut s = InputState::new(window, cx);
                s.set_value(n, window, cx);
                s
            })
        });
        let mut v = Self {
            paths,
            images,
            page: Page::General,
            info: None,
            folder: None,
            totals: None,
            media: Vec::new(),
            name,
            perm: PermEdit::default(),
            groups: Vec::new(),
            recurse: false,
            drive: None,
            pe: None,
            hash: None,
            share: ShareEdit::default(),
            error: None,
            busy: false,
            frame_dark: None,
            cancel: Default::default(),
            focus: cx.focus_handle(),
        };
        v.load(window, cx);
        v.load_groups(cx);
        window.focus(&v.focus, cx);
        window.activate_window();
        v
    }

    fn single(&self) -> Option<&Path> {
        (self.paths.len() == 1).then(|| self.paths[0].as_path())
    }

    fn load(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let cancel = self.cancel.clone();
        if let Some(path) = self.single().map(Path::to_path_buf) {
            let p = path.clone();
            let task = cx.background_spawn(async move { load_one(&p) });
            cx.spawn_in(window, async move |this, cx| {
                let l = task.await;
                let _ = this.update_in(cx, |v, _, cx| {
                    v.drive = l.drive;
                    v.pe = l.pe;
                    if let Some(p) = &l.perms {
                        let mut fold = Fold::default();
                        fold.add(p);
                        v.perm = PermEdit::from(fold);
                    }
                    if let (Some(Ok(i)), None) = (&l.info, &v.drive) {
                        v.start_details(i.is_dir, cx);
                    }
                    v.info = l.info;
                    cx.notify();
                });
            })
            .detach();
        } else {
            self.totals = Some(Totals::default());
            let paths = self.paths.clone();
            let (tx, mut rx) = futures::channel::mpsc::unbounded::<(Totals, Fold)>();
            std::thread::Builder::new()
                .name("rf-props".into())
                .spawn(move || {
                    let cache = crate::preview::size_cache();
                    let mut t = Totals::default();
                    let mut acc = Fold::default();
                    let mut first_file = true;
                    for p in &paths {
                        if cancel.load(Ordering::Relaxed) {
                            return;
                        }
                        let Ok(info) = rf_linux::fileinfo::info(p) else { continue };
                        if let Ok(pm) = perms::read(p) {
                            acc.add(&pm);
                        }
                        if info.is_dir {
                            t.sel_dirs += 1;
                            t.dirs += 1;
                            if let Ok(fs) = cache.get_or_compute(p, &|| cancel.load(Ordering::Relaxed), &mut |_| {}) {
                                t.bytes += fs.bytes;
                                t.on_disk += fs.allocated.unwrap_or(fs.bytes);
                                t.files += fs.files;
                                t.dirs += fs.dirs;
                            }
                        } else {
                            t.files += 1;
                            t.bytes += info.size;
                            t.on_disk += info.size_on_disk.unwrap_or(info.size);
                            let e = ext_of(p);
                            t.same_ext = match &t.same_ext {
                                _ if first_file => Some(Some(e)),
                                Some(Some(x)) if *x == e => Some(Some(e)),
                                _ => Some(None),
                            };
                            first_file = false;
                        }
                        let _ = tx.unbounded_send((t.clone(), acc.clone()));
                    }
                    t.done = true;
                    let _ = tx.unbounded_send((t, acc));
                })
                .ok();
            cx.spawn(async move |this, cx| {
                use futures::StreamExt;
                let mut first = true;
                while let Some((t, acc)) = rx.next().await {
                    let done = t.done;
                    let ok = this
                        .update(cx, |v, cx| {
                            v.totals = Some(t);
                            if first || done {
                                v.perm = PermEdit::from(acc);
                            }
                            cx.notify();
                        })
                        .is_ok();
                    first = false;
                    if !ok {
                        break;
                    }
                }
            })
            .detach();
        }
    }

    /// The groups this user is in, for the group list.
    fn load_groups(&mut self, cx: &mut Context<Self>) {
        let task = cx.background_spawn(async move { perms::my_groups() });
        cx.spawn(async move |this, cx| {
            let g = task.await;
            let _ = this.update(cx, |v, cx| {
                v.groups = g;
                cx.notify();
            });
        })
        .detach();
    }

    /// Folder contents (folders) or media properties (files), after the basic info arrived.
    fn start_details(&mut self, is_dir: bool, cx: &mut Context<Self>) {
        let Some(path) = self.single().map(Path::to_path_buf) else { return };
        let cancel = self.cancel.clone();
        if is_dir {
            self.folder = Some(None);
            let task = cx.background_spawn(async move {
                crate::preview::size_cache().get_or_compute(&path, &|| cancel.load(Ordering::Relaxed), &mut |_| {}).ok()
            });
            cx.spawn(async move |this, cx| {
                let r = task.await;
                let _ = this.update(cx, |v, cx| {
                    v.folder = Some(r);
                    cx.notify();
                });
            })
            .detach();
        } else {
            let task = cx.background_spawn(async move { rf_linux::props::media_props(&path) });
            cx.spawn(async move |this, cx| {
                let r = task.await;
                let _ = this.update(cx, |v, cx| {
                    v.media = r;
                    cx.notify();
                });
            })
            .detach();
        }
    }

    fn renamed_to(&self, cx: &App) -> Option<String> {
        match (&self.name, self.single()) {
            (Some(n), Some(p)) => {
                let v = n.read(cx).value().trim().to_string();
                (v != file_name(p)).then_some(v)
            }
            _ => None,
        }
    }

    fn dirty(&self, cx: &App) -> bool {
        self.renamed_to(cx).is_some() || self.perm.changed() || self.share.change(cx).is_some()
    }

    /// Reads whether the folder is shared (the Sharing page opened).
    fn load_share(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(path) = self.single().map(Path::to_path_buf) else { return };
        self.share.load = ShareLoad::Loading;
        let task = cx.background_spawn(async move { usershare::share_of(&path).map_err(|e| e.downcast::<usershare::Unavailable>().map_err(|e| format!("{e:#}"))) });
        cx.spawn_in(window, async move |this, cx| {
            let r = task.await;
            let _ = this.update_in(cx, |v, window, cx| {
                v.share_loaded(r, window, cx);
                cx.notify();
            });
        })
        .detach();
    }

    fn share_loaded(&mut self, r: Result<Option<usershare::Share>, Result<usershare::Unavailable, String>>, window: &mut Window, cx: &mut Context<Self>) {
        self.share.load = match r {
            Ok(cur) => {
                let name = cur.as_ref().map_or_else(|| usershare::default_name(&self.paths[0]), |s| s.name.clone());
                self.share.on = cur.is_some();
                self.share.writable = cur.as_ref().is_some_and(|s| s.writable);
                self.share.guest = cur.as_ref().is_some_and(|s| s.guest);
                self.share.name = Some(cx.new(|cx| {
                    let mut s = InputState::new(window, cx);
                    s.set_value(name, window, cx);
                    s
                }));
                ShareLoad::Ready(cur)
            }
            Err(Ok(u)) => ShareLoad::Unavailable(u),
            Err(Err(m)) => ShareLoad::Failed(m),
        };
    }

    /// Applies the edits on a helper thread; `close` closes the window when it worked.
    fn apply(&mut self, close: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        if !self.dirty(cx) {
            if close {
                window.remove_window();
            }
            return;
        }
        let new_name = self.renamed_to(cx);
        if let Some(n) = &new_name {
            if let Err(e) = rf_core::rename::validate(n) {
                self.error = Some(crate::i18n::td(&e.to_string()).into());
                cx.notify();
                return;
            }
        }
        let share_edit = self.share.change(cx);
        if let Some(Some((n, _, _))) = &share_edit
            && !usershare::name_ok(n)
        {
            self.error = Some(t!("Samba doesn't allow that share name").into());
            cx.notify();
            return;
        }
        let share_now = match &self.share.load {
            ShareLoad::Ready(cur) => cur.clone(),
            _ => None,
        };
        // what to do to the share: (the share there is now, the one wanted). A shared folder
        // that is renamed gets its share moved along.
        let share_act = match share_edit {
            Some(want) => Some((share_now, want)),
            None if new_name.is_some() && share_now.is_some() => {
                let s = share_now.clone().unwrap();
                Some((share_now, Some((s.name, s.writable, s.guest))))
            }
            None => None,
        };
        let change = self.perm.change();
        let recurse = self.recurse;
        let paths = self.paths.clone();
        let cancel = self.cancel.clone();
        self.busy = true;
        self.error = None;
        let task = cx.background_spawn(async move {
            let mut paths = paths;
            let named = |p: &Path, e: &dyn std::fmt::Display| format!("{}: {e}", file_name(p));
            for p in &paths {
                if !change.is_empty() {
                    if recurse && p.is_dir() {
                        perms::apply_tree(p, &change, &|| cancel.load(Ordering::Relaxed)).map_err(|e| named(p, &e))?;
                    } else {
                        perms::apply(p, &change).map_err(|e| named(p, &e))?;
                    }
                }
            }
            let mut renamed = None;
            if let Some(n) = new_name {
                let from = paths[0].clone();
                let to = from.with_file_name(&n);
                if to.exists() && !n.eq_ignore_ascii_case(&file_name(&from)) {
                    return Err(t!("An item with this name already exists").to_string());
                }
                std::fs::rename(&from, &to).map_err(|e| e.to_string())?;
                renamed = Some((from, to.clone()));
                paths[0] = to;
            }
            let mut shared = None;
            if let Some((now, want)) = share_act {
                let comment = now.as_ref().map(|s| s.comment.clone()).unwrap_or_default();
                // a new name (or no share) drops the old one
                if let Some(old) = now.as_ref().filter(|o| want.as_ref().is_none_or(|w| w.0 != o.name)) {
                    usershare::remove(&old.name).map_err(|e| format!("{e:#}"))?;
                }
                let new = want.map(|(name, writable, guest)| usershare::Share { name, path: paths[0].clone(), comment, writable, guest });
                if let Some(n) = &new {
                    usershare::add(n).map_err(|e| format!("{e:#}"))?;
                }
                shared = Some(new);
            }
            Ok::<_, String>((paths, renamed, shared))
        });
        cx.spawn_in(window, async move |this, cx| {
            let r = task.await;
            let _ = this.update_in(cx, |v, window, cx| {
                v.busy = false;
                match r {
                    Ok((paths, renamed, shared)) => {
                        if let Some(s) = shared {
                            v.share.load = ShareLoad::Ready(s);
                        }
                        if let Some((from, to)) = renamed {
                            crate::undo::push(crate::undo::UndoOp::Renamed { from, to });
                        }
                        v.paths = paths;
                        let groups = v.groups.clone();
                        v.perm.commit(&groups);
                        v.recurse = false;
                        if close {
                            window.remove_window();
                        }
                    }
                    Err(e) => v.error = Some(e.into()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// The HWND of this window, for dialogs it owns.
    fn hwnd(window: &Window) -> isize {
        use raw_window_handle::{HasWindowHandle, RawWindowHandle};
        match HasWindowHandle::window_handle(window).map(|h| h.as_raw()) {
            Ok(RawWindowHandle::Win32(h)) => h.hwnd.get(),
            _ => 0,
        }
    }

    /// Windows' "Open with" chooser for the type; the new default shows when it closes.
    fn change_app(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(p) = self.single().map(Path::to_path_buf) else { return };
        let owner = Self::hwnd(window);
        let (tx, rx) = futures::channel::oneshot::channel();
        rf_linux::sta::run(move |_| {
            // Linux: the desktop's app chooser (it returns before the choice is made)
            let _ = (owner, rf_linux::shell::open_with(&p));
            let _ = tx.send(rf_linux::fileinfo::opens_with(&p));
        });
        cx.spawn(async move |this, cx| {
            if let Ok(app) = rx.await {
                let _ = this.update(cx, |v, cx| {
                    if let Some(Ok(i)) = &mut v.info {
                        i.opens_with = app;
                    }
                    cx.notify();
                });
            }
        })
        .detach();
    }

    fn entry(&self) -> Option<Entry> {
        let p = self.single()?;
        let Some(Ok(i)) = &self.info else { return None };
        let kind = if i.is_dir { EntryKind::Dir } else { EntryKind::File };
        Some(Entry::new(&file_name(p), kind, i.size, i.modified_ms, i.created_ms, i.attrs))
    }

    fn pages(&self) -> Vec<(Page, &'static str)> {
        let mut pages = vec![(Page::General, t!("General"))];
        if self.single().is_none() {
            return pages;
        }
        let Some(Ok(info)) = &self.info else { return pages };
        if !info.is_dir {
            pages.push((Page::Details, t!("Details")));
        }
        if !info.is_dir {
            pages.push((Page::Hashes, t!("Hashes")));
        }
        // a folder on this computer (a network folder is someone else's to share)
        let path = &self.paths[0];
        if info.is_dir && !is_root(path) && rf_linux::remote::mount_url(path).is_none() {
            pages.push((Page::Sharing, t!("Sharing")));
        }
        pages
    }

    // ---------- rendering ----------

    fn render_general(&self, p: &Palette, cx: &mut Context<Self>) -> Vec<AnyElement> {
        if let Some(d) = &self.drive {
            return self.render_drive(d, p);
        }
        let mut out = Vec::new();
        let mut card1 = Vec::new();
        if let Some(path) = self.single() {
            match &self.info {
                None => card1.push(row(p, t!("Status"), t!("Loading…").into())),
                Some(Err(e)) => card1.push(row(p, t!("Error"), e.clone())),
                Some(Ok(i)) => {
                    if let Some(e) = self.entry() {
                        let ext = ext_of(path);
                        let label = crate::icons::type_label(&e).to_string();
                        let label = if i.is_dir || ext.is_empty() { label } else { format!("{label} (.{ext})") };
                        card1.push(row(p, t!("Type"), label));
                    }
                    // a program or shortcut runs itself: Windows shows its description there
                    let runs = matches!(ext_of(path).as_str(), "exe" | "com" | "scr" | "lnk" | "bat" | "cmd" | "msi" | "url");
                    if let Some(d) = self.pe.as_ref().and_then(|pe| pe.description.clone()).filter(|_| runs) {
                        card1.push(row(p, t!("Description"), d));
                    }
                    if !i.is_dir && !runs {
                        let app = i.opens_with.clone().unwrap_or_else(|| t!("Not chosen").to_string());
                        card1.push(row_with(
                            p,
                            t!("Opens with"),
                            app,
                            link_button(p, "change-app", L::AppWindow, t!("Change…")).on_click(cx.listener(|v, _, window, cx| v.change_app(window, cx))).into_any_element(),
                        ));
                    }
                    let loc = path.parent().map(|d| breakable(&d.to_string_lossy())).unwrap_or_default();
                    card1.push(row(p, t!("Location"), loc));
                    if i.is_dir {
                        let (size, contents) = match &self.folder {
                            Some(Some(fs)) => (
                                format!("{} ({} {})", format::size(fs.bytes), group(fs.bytes), t!("bytes")),
                                format!("{} {}, {} {}", group(fs.files), t!("files"), group(fs.dirs), t!("folders")),
                            ),
                            Some(None) if self.folder.is_some() => (t!("Calculating…").into(), t!("Calculating…").into()),
                            _ => ("—".into(), "—".into()),
                        };
                        card1.push(row(p, t!("Size"), size));
                        if let Some(d) = self.folder.as_ref().and_then(|f| f.as_ref()).and_then(|fs| fs.allocated) {
                            card1.push(row(p, t!("Size on disk"), format!("{} ({} {})", format::size(d), group(d), t!("bytes"))));
                        }
                        card1.push(row(p, t!("Contains"), contents));
                    } else {
                        card1.push(row(p, t!("Size"), format!("{} ({} {})", format::size(i.size), group(i.size), t!("bytes"))));
                        if let Some(d) = i.size_on_disk {
                            card1.push(row(p, t!("Size on disk"), format!("{} ({} {})", format::size(d), group(d), t!("bytes"))));
                        }
                    }
                }
            }
        } else if let Some(t) = &self.totals {
            let dir = self.paths[0].parent().map(|d| breakable(&d.to_string_lossy())).unwrap_or_default();
            let sel_files = (self.paths.len() as u64).saturating_sub(t.sel_dirs);
            card1.push(row(p, t!("Selection"), format!("{} {}, {} {}", group(sel_files), t!("files"), group(t.sel_dirs), t!("folders"))));
            let kind = match (&t.same_ext, t.sel_dirs) {
                (Some(Some(e)), 0) if !e.is_empty() => format!("{} .{e}", t!("All of type")),
                (_, d) if d as usize == self.paths.len() => t!("All folders").to_string(),
                _ => t!("Multiple types").to_string(),
            };
            card1.push(row(p, t!("Type"), kind));
            card1.push(row(p, t!("Location"), format!("{} {dir}", t!("All in"))));
            let suffix = if t.done { String::new() } else { format!(" — {}", t!("calculating…")) };
            card1.push(row(p, t!("Size"), format!("{} ({} {}){suffix}", format::size(t.bytes), group(t.bytes), t!("bytes"))));
            card1.push(row(p, t!("Size on disk"), format!("{} ({} {}){suffix}", format::size(t.on_disk), group(t.on_disk), t!("bytes"))));
            card1.push(row(p, t!("Contains"), format!("{} {}, {} {}", group(t.files), t!("files"), group(t.dirs), t!("folders"))));
        }
        out.push(card(p, card1));

        if let Some(Ok(i)) = &self.info {
            out.push(card(
                p,
                vec![
                    row(p, t!("Created"), format::datetime_full(i.created_ms)),
                    row(p, t!("Modified"), format::datetime_full(i.modified_ms)),
                    row(p, t!("Accessed"), format::datetime_full(i.accessed_ms)),
                ],
            ));
        }

        if self.perm.loaded() {
            out.push(self.render_perms(p, cx));
        }

        out
    }

    fn render_sharing(&self, p: &Palette, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let note = |text: String| div().py_2().text_sm().child(text).into_any_element();
        let small = |text: String| div().pb_2().text_xs().text_color(p.text_secondary).child(text).into_any_element();
        let rows = match &self.share.load {
            ShareLoad::Idle | ShareLoad::Loading => vec![note(t!("Loading…").to_string())],
            ShareLoad::Unavailable(usershare::Unavailable::NoSamba) => {
                vec![note(t!("Sharing folders needs Samba. Install it, then turn on user shares.").to_string())]
            }
            ShareLoad::Unavailable(usershare::Unavailable::Disabled(m)) => vec![
                note(t!("Samba's user shares are turned off. Turn them on with a usershare path in /etc/samba/smb.conf.").to_string()),
                small(m.clone()),
            ],
            ShareLoad::Failed(m) => vec![note(m.clone())],
            ShareLoad::Ready(cur) => {
                let e = &self.share;
                let mut rows = vec![section(p, t!("Sharing"))];
                let flip = |f: fn(&mut ShareEdit)| {
                    cx.listener(move |v: &mut Self, _: &ClickEvent, _, cx| {
                        f(&mut v.share);
                        cx.notify();
                    })
                };
                rows.push(check_row(p, "share-on", t!("Share this folder on the network"), Some(e.on), flip(|s| s.on = !s.on)));
                if e.on {
                    if let Some(n) = &e.name {
                        rows.push(
                            h_flex()
                                .gap_3()
                                .py(px(4.))
                                .items_center()
                                .text_sm()
                                .child(div().w(px(150.)).flex_shrink_0().text_color(p.text_secondary).child(t!("Share name")))
                                .child(text_field(p, n))
                                .into_any_element(),
                        );
                    }
                    rows.push(check_row(p, "share-write", t!("Others can add, change and delete files"), Some(e.writable), flip(|s| s.writable = !s.writable)));
                    rows.push(check_row(p, "share-guest", t!("Allow guests (no password)"), Some(e.guest), flip(|s| s.guest = !s.guest)));
                }
                // where others find it, once shared
                if let Some(s) = cur.as_ref().filter(|_| e.on) {
                    let host = std::fs::read_to_string("/proc/sys/kernel/hostname").map(|h| h.trim().to_string()).unwrap_or_default();
                    rows.push(small(format!("smb://{host}/{}", s.name)));
                }
                rows
            }
        };
        vec![card(p, rows)]
    }

    fn render_perms(&self, p: &Palette, cx: &mut Context<Self>) -> AnyElement {
        let e = &self.perm;
        let f = &e.fold;
        let can = f.mine;
        let mut rows = vec![section(p, t!("Permissions"))];
        rows.push(row(p, t!("Owner"), f.owner.clone().unwrap_or_else(|| t!("Various").to_string())));
        let gid = e.new_gid.or(f.group.as_ref().map(|g| g.0));
        let group_label = match (e.new_gid, &f.group) {
            (Some(g), _) => self.groups.iter().find(|x| x.0 == g).map_or_else(|| g.to_string(), |x| x.1.clone()),
            (None, Some((_, n))) => n.clone(),
            (None, None) => t!("Various").to_string(),
        };
        if can && self.groups.len() > 1 {
            let groups = self.groups.clone();
            let this = cx.entity().downgrade();
            let pick = crate::select_box::select_box("perm-group", p, group_label, move |_| {
                groups
                    .iter()
                    .map(|(g, name)| {
                        let (g, this) = (*g, this.clone());
                        crate::select_box::Choice::new(name.clone(), Some(g) == gid, move |_, cx| {
                            let _ = this.update(cx, |v, cx| {
                                v.perm.new_gid = Some(g);
                                cx.notify();
                            });
                        })
                    })
                    .collect()
            });
            rows.push(
                h_flex()
                    .gap_3()
                    .py(px(4.))
                    .items_center()
                    .text_sm()
                    .child(div().w(px(150.)).flex_shrink_0().text_color(p.text_secondary).child(t!("Group")))
                    .child(div().w(px(200.)).child(pick))
                    .into_any_element(),
            );
        } else {
            rows.push(row(p, t!("Group"), group_label));
        }

        // the grid: who × what
        let head = |label: &'static str| div().w(px(64.)).flex().justify_center().text_xs().text_color(p.text_secondary).child(label);
        rows.push(
            h_flex()
                .gap_3()
                .pt_2()
                .child(div().w(px(150.)).flex_shrink_0())
                .child(head(t!("Read")))
                .child(head(t!("Write")))
                .child(head(if f.any_file { t!("Execute") } else { t!("Open") }))
                .into_any_element(),
        );
        for (class, label) in [t!("Owner"), t!("Group"), t!("Others")].into_iter().enumerate() {
            let mut line = h_flex().gap_3().py(px(4.)).items_center().text_sm().child(div().w(px(150.)).flex_shrink_0().text_color(p.text_secondary).child(label));
            for col in 0..3 {
                let i = class * 3 + col;
                line = line.child(
                    div()
                        .id(("perm-bit", i))
                        .w(px(64.))
                        .flex()
                        .justify_center()
                        .when(can, |d| d.cursor_pointer())
                        .when(!can, |d| d.opacity(0.5))
                        .child(check_mark(p, e.bits[i].now))
                        .on_click(cx.listener(move |v, _, _, cx| {
                            if v.perm.fold.mine {
                                v.perm.bits[i].toggle();
                                cx.notify();
                            }
                        })),
                );
            }
            rows.push(line.into_any_element());
        }

        // scripts and AppImages: one checkbox for the usual case
        if f.any_file && !f.any_dir {
            let on = e.bits[2].now;
            rows.push(check_row(p, "perm-exec", t!("Allow running as a program"), on, cx.listener(move |v, _, _, cx| {
                if v.perm.fold.mine {
                    v.perm.set_executable(on != Some(true));
                    cx.notify();
                }
            })));
        }
        if f.any_dir && can {
            rows.push(check_row(p, "perm-recurse", t!("Apply to everything inside"), Some(self.recurse), cx.listener(|v, _, _, cx| {
                v.recurse = !v.recurse;
                cx.notify();
            })));
            if self.recurse {
                rows.push(div().text_xs().text_color(p.text_secondary).pb_1().child(t!("Files inside keep their own execute permission.")).into_any_element());
            }
        }
        if !can {
            rows.push(div().text_xs().text_color(p.text_secondary).py_1().child(t!("Only the owner can change these.")).into_any_element());
        }
        card(p, rows)
    }

    fn render_drive(&self, d: &rf_linux::drives::Drive, p: &Palette) -> Vec<AnyElement> {
        use rf_linux::drives::DriveKind;
        let kind = match d.kind {
            DriveKind::Removable => t!("Removable disk"),
            DriveKind::Network => t!("Network drive"),
            DriveKind::Optical => t!("CD/DVD drive"),
            _ => t!("Local Disk"),
        };
        let used = d.total.saturating_sub(d.free);
        let frac = d.used_fraction().clamp(0.0, 1.0);
        let bytes = |n: u64| format!("{} ({} {})", format::size(n), group(n), t!("bytes"));
        let swatch = |c: Hsla| div().size(px(12.)).rounded(px(3.)).bg(c).flex_shrink_0();
        let full = frac > 0.9;
        let used_color = if full { p.danger } else { p.accent };
        let mut out = vec![card(
            p,
            vec![
                row(p, t!("Type"), kind.to_string()),
                row(p, t!("File system"), if d.fs.is_empty() { "—".into() } else { d.fs.clone() }),
                row(p, t!("Tag"), if d.label.is_empty() { "—".into() } else { d.label.clone() }),
            ],
        )];
        out.push(card(
            p,
            vec![
                h_flex().gap_3().py(px(7.)).text_sm().items_center().child(swatch(used_color)).child(div().w(px(120.)).text_color(p.text_secondary).child(t!("Used space"))).child(bytes(used)).into_any_element(),
                h_flex().gap_3().py(px(7.)).text_sm().items_center().child(swatch(p.hover)).child(div().w(px(120.)).text_color(p.text_secondary).child(t!("Free space"))).child(bytes(d.free)).into_any_element(),
                div().h(px(1.)).my_1().bg(p.divider).into_any_element(),
                h_flex().gap_3().py(px(7.)).text_sm().items_center().child(div().size(px(12.))).child(div().w(px(120.)).text_color(p.text_secondary).child(t!("Capacity"))).child(bytes(d.total)).into_any_element(),
                // how full: a bar like the sidebar's, larger
                div()
                    .mt_2()
                    .mb_1()
                    .h(px(14.))
                    .w_full()
                    .rounded(px(7.))
                    .bg(p.hover)
                    .border_1()
                    .border_color(p.border)
                    .child(div().h_full().rounded(px(7.)).bg(used_color).w(relative(frac)))
                    .into_any_element(),
                div().text_xs().text_color(p.text_secondary).child(format!("{} %{:.0}", t!("Full:"), frac * 100.0)).into_any_element(),
            ],
        ));
        out
    }

    fn render_details(&self, p: &Palette) -> Vec<AnyElement> {
        let mut out = Vec::new();
        if let Some(pe) = &self.pe {
            let mut rows = vec![section(p, t!("Program"))];
            let mut add = |k: &'static str, v: &Option<String>| {
                if let Some(v) = v.as_ref().filter(|v| !v.trim().is_empty()) {
                    rows.push(row(p, k, v.clone()));
                }
            };
            add(t!("File description"), &pe.description);
            add(t!("File version"), &pe.version);
            add(t!("Product name"), &pe.product);
            add(t!("Product version"), &pe.product_version);
            add(t!("Company"), &pe.company);
            add(t!("Copyright"), &pe.copyright);
            add(t!("Trademarks"), &pe.trademarks);
            add(t!("Language"), &pe.language);
            add(t!("Original filename"), &pe.original_filename);
            add(t!("Internal name"), &pe.internal_name);
            rows.push(row(p, t!("Architecture"), pe.arch.to_string()));
            out.push(card(p, rows));
        }
        if !self.media.is_empty() {
            let mut rows = vec![section(p, t!("Contains"))];
            rows.extend(self.media.iter().map(|(k, v)| row(p, crate::i18n::t(k), v.clone())));
            out.push(card(p, rows));
        }
        if let (Some(path), Some(Ok(i))) = (self.single(), &self.info) {
            let mut rows = vec![
                section(p, t!("File")),
                row(p, t!("Name"), file_name(path)),
                row(p, t!("Folder path"), path.parent().map(|d| breakable(&d.to_string_lossy())).unwrap_or_default()),
                row(p, t!("Created"), format::datetime_full(i.created_ms)),
                row(p, t!("Modified"), format::datetime_full(i.modified_ms)),
                row(p, t!("Size"), format::size(i.size)),
            ];
            let f = &self.perm.fold;
            if let (Some(o), Some((_, g))) = (&f.owner, &f.group) {
                let mode: u32 = f.bits.iter().zip(BITS).filter(|(b, _)| **b == Some(true)).map(|(_, bit)| bit).sum();
                rows.push(row(p, t!("Permissions"), perms::mode_string(mode)));
                rows.push(row(p, t!("Owner"), o.clone()));
                rows.push(row(p, t!("Group"), g.clone()));
            }
            if let Ok(pc) = std::env::var("COMPUTERNAME") {
                rows.push(row(p, t!("Computer"), format!("{pc} ({})", t!("this PC"))));
            }
            out.push(card(p, rows));
        }
        out
    }

}

/// Everything the window shows about one item, read on a background thread.
fn load_one(p: &Path) -> Loaded {
    let mut l = Loaded::default();
    let info = rf_linux::fileinfo::info(p).map_err(|e| e.to_string());
    let is_dir = info.as_ref().is_ok_and(|i| i.is_dir);
    let is_file = info.as_ref().is_ok_and(|i| !i.is_dir);
    // a drive is its mount point
    if is_dir {
        l.drive = rf_linux::drives::list_drives().into_iter().find(|d| d.root == p);
    }
    if is_dir {
    }
    if is_file {
        if is_pe(p) {
            l.pe = rf_linux::pe::info(p);
        }
    }
    l.perms = perms::read(p).ok();
    l.info = Some(info);
    l
}

fn group(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push('.');
        }
        out.push(c);
    }
    out
}

fn row(p: &Palette, k: &'static str, v: String) -> AnyElement {
    h_flex()
        .w_full()
        .gap_3()
        .items_start()
        .py(px(7.))
        .text_sm()
        .child(div().w(px(150.)).flex_shrink_0().text_color(p.text_secondary).child(k))
        .child(div().flex_1().min_w_0().child(v))
        .into_any_element()
}

/// A row with a small action on its right ("Change…").
fn row_with(p: &Palette, k: &'static str, v: String, action: AnyElement) -> AnyElement {
    h_flex()
        .w_full()
        .gap_3()
        .items_center()
        .py(px(5.))
        .text_sm()
        .child(div().w(px(150.)).flex_shrink_0().text_color(p.text_secondary).child(k))
        .child(div().flex_1().min_w_0().child(v))
        .child(action)
        .into_any_element()
}

fn section(p: &Palette, title: &'static str) -> AnyElement {
    div().pt_1().pb(px(2.)).text_xs().font_weight(FontWeight::SEMIBOLD).text_color(p.text_secondary).child(title).into_any_element()
}

fn link_button(p: &Palette, id: &'static str, icon: L, label: &'static str) -> Stateful<Div> {
    h_flex()
        .id(id)
        .gap_1p5()
        .items_center()
        .flex_shrink_0()
        .text_sm()
        .text_color(p.accent)
        .cursor_pointer()
        .hover(|s| s.opacity(0.8))
        .child(Icon::new(icon).size(px(14.)))
        .child(label)
}

fn text_field(p: &Palette, input: &Entity<InputState>) -> AnyElement {
    div()
        .flex_1()
        .min_w_0()
        .h(px(30.))
        .px_2()
        .flex()
        .items_center()
        .rounded(theme::RADIUS_SM)
        .bg(p.content_bg)
        .border_1()
        .border_color(p.border)
        .child(Input::new(input).appearance(false))
        .into_any_element()
}

fn card(p: &Palette, rows: Vec<AnyElement>) -> AnyElement {
    v_flex()
        .w_full()
        .px_4()
        .py_2()
        .rounded(theme::RADIUS)
        .bg(p.card_bg)
        .border_1()
        .border_color(p.border)
        .children(rows)
        .into_any_element()
}

fn check_row(
    p: &Palette,
    id: &'static str,
    label: &'static str,
    on: Option<bool>,
    click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> AnyElement {
    h_flex()
        .id(id)
        .gap_3()
        .py(px(7.))
        .items_center()
        .text_sm()
        .cursor_pointer()
        .child(check_mark(p, on))
        .child(div().flex_1().min_w_0().child(label))
        .on_click(click)
        .into_any_element()
}

/// A checkbox: ticked, empty, or a dash when the selection is mixed (None).
fn check_mark(p: &Palette, on: Option<bool>) -> Div {
    let (fill, mark) = match on {
        Some(true) => (Some(p.accent), Some(L::Check)),
        None => (Some(p.accent), Some(L::Minus)),
        Some(false) => (None, None),
    };
    div()
        .size(px(18.))
        .flex_shrink_0()
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(4.))
        .when_some(fill, |d, c| d.bg(c))
        .when(fill.is_none(), |d| d.border_1().border_color(p.text_secondary))
        .children(mark.map(|m| Icon::new(m).size(px(12.)).text_color(p.accent_text)))
}

impl Render for PropsView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = Palette::for_window(window);
        if self.frame_dark != Some(p.dark) {
            self.frame_dark = Some(p.dark);
            let h = Self::hwnd(window);
            if h != 0 {
                rf_linux::window::set_dark_mode(h, p.dark);
            }
        }
        let title = match (self.single(), &self.drive) {
            (_, Some(d)) => format!("{} — {}", d.display_name(), t!("Properties")),
            (Some(path), None) => format!("{} — {}", file_name(path), t!("Properties")),
            (None, None) => format!("{} — {}", crate::i18n::items(self.paths.len()), t!("Properties")),
        };
        window.set_window_title(&title);

        // header: big icon / thumbnail + name
        let icon: AnyElement = match (self.entry(), &self.drive) {
            (_, Some(_)) => Icon::new(L::HardDrive).size(px(40.)).text_color(p.text_secondary).into_any_element(),
            (Some(e), None) => {
                let dir = self.paths[0].parent().map(Path::to_path_buf).unwrap_or_default();
                match self.images.update(cx, |c, cx| c.thumbnail(&dir, &e, 96, cx)) {
                    Some(im) => img(im).size(px(48.)).object_fit(ObjectFit::Contain).into_any_element(),
                    None => {
                        let cat = crate::icons::category(&e);
                        Icon::new(crate::icons::icon_for(cat)).size(px(40.)).text_color(theme::kind_color(cat, p.dark)).into_any_element()
                    }
                }
            }
            (None, None) => Icon::new(if self.paths.len() > 1 { L::Files } else { L::File }).size(px(40.)).text_color(p.text_secondary).into_any_element(),
        };
        let name: AnyElement = match (&self.name, &self.drive) {
            (_, Some(d)) => div().flex_1().text_base().font_weight(FontWeight::SEMIBOLD).child(d.display_name()).into_any_element(),
            (Some(n), None) => text_field(&p, n),
            (None, None) => div().flex_1().text_base().font_weight(FontWeight::SEMIBOLD).child(crate::i18n::items(self.paths.len())).into_any_element(),
        };

        // tabs (they wrap onto a second row when many, as Windows' do)
        let pages = self.pages();
        if !pages.iter().any(|(pg, _)| *pg == self.page) {
            self.page = Page::General;
        }
        let mut tabs = h_flex().flex_wrap().p(px(2.)).gap(px(2.)).rounded(theme::RADIUS_SM).bg(p.content_bg).border_1().border_color(p.border);
        let show_tabs = pages.len() > 1;
        for (pg, label) in pages {
            let on = pg == self.page;
            tabs = tabs.child(
                div()
                    .id(label)
                    .flex_grow(1.)
                    .h(px(28.))
                    .px_3()
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(4.))
                    .text_sm()
                    .when(on, |d| d.bg(p.card_bg).shadow_sm().font_weight(FontWeight::MEDIUM))
                    .when(!on, |d| d.text_color(p.text_secondary).hover(|s| s.bg(p.hover)))
                    .child(label)
                    .on_click(cx.listener(move |v, _, window, cx| {
                        v.page = pg;
                        match pg {
                            Page::Sharing if matches!(v.share.load, ShareLoad::Idle) => v.load_share(window, cx),
                            Page::Hashes if v.hash.is_none() => {
                                let path = v.paths[0].clone();
                                v.hash = Some(cx.new(|cx| crate::hash_panel::HashPanel::new(path, cx)));
                            }
                            _ => {}
                        }
                        cx.notify();
                    })),
            );
        }

        let body: Vec<AnyElement> = match self.page {
            Page::General => self.render_general(&p, cx),
            Page::Details => self.render_details(&p),
            Page::Hashes => self.hash.clone().map(|h| vec![card(&p, vec![div().py_2().child(h).into_any_element()])]).unwrap_or_default(),
            Page::Sharing => self.render_sharing(&p, cx),
        };

        let button = |id: &'static str, label: &'static str, primary: bool| {
            div()
                .id(id)
                .min_w(px(88.))
                .h(px(32.))
                .px_4()
                .flex()
                .items_center()
                .justify_center()
                .rounded(theme::RADIUS_SM)
                .text_sm()
                .when(primary, |d| d.bg(p.accent).text_color(p.accent_text))
                .when(!primary, |d| d.bg(p.hover).border_1().border_color(p.border))
                .hover(|s| s.opacity(0.9))
                .active(|s| s.opacity(0.8))
                .child(label)
        };
        let dirty = self.dirty(cx);

        v_flex()
            .relative()
            .size_full()
            .bg(p.window_bg)
            .text_color(p.text)
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|v, ev: &KeyDownEvent, window, cx| match ev.keystroke.key.as_str() {
                "escape" => window.remove_window(),
                "enter" => v.apply(true, window, cx),
                _ => {}
            }))
            .child(
                TitleBar::new()
                    .h(theme::TITLEBAR_H)
                    .bg(p.titlebar_bg)
                    .border_color(gpui_kit::transparent_black())
                    .child(h_flex().gap_2().items_center().text_xs().child(Icon::new(L::Info).size(px(14.))).child(title.clone())),
            )
            .child(
                v_flex()
                    .id("props-body")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .px_5()
                    .pt_2()
                    .pb_4()
                    .gap_4()
                    .child(h_flex().gap_3().items_center().child(div().size(px(48.)).flex().items_center().justify_center().child(icon)).child(name))
                    .when(show_tabs, |d| d.child(tabs))
                    .children(body),
            )
            .child(
                h_flex()
                    .px_5()
                    .py_3()
                    .gap_2()
                    .items_center()
                    .border_t_1()
                    .border_color(p.divider)
                    .child(div().flex_1().min_w_0().text_xs().text_color(p.danger).children(self.error.clone()))
                    .child(button("ok", t!("OK"), true).on_click(cx.listener(|v, _, window, cx| v.apply(true, window, cx))))
                    .child(button("cancel", t!("Cancel"), false).on_click(|_, window, _| window.remove_window()))
                    .child(
                        button("apply", t!("Apply"), false)
                            .when(!dirty, |d| d.opacity(0.5))
                            .on_click(cx.listener(|v, _, window, cx| v.apply(false, window, cx))),
                    ),
            )
            // the group list, painted on top
            .children(crate::select_box::current(window, cx))
    }
}

#[cfg(test)]
mod tests {
    use super::{BITS, Fold, Path, PermEdit, is_pe, is_root};
    use rf_linux::perms::Perms;

    fn item(mode: u32, gid: u32, is_dir: bool) -> Perms {
        let uid = rf_linux::perms::my_uid();
        Perms { mode, uid, gid, owner: "me".into(), group: format!("g{gid}"), is_dir }
    }

    #[test]
    fn permissions_fold_and_change() {
        let mut f = Fold::default();
        f.add(&item(0o644, 1, false));
        f.add(&item(0o664, 2, false));
        assert_eq!(f.bits[4], None, "group write differs");
        assert_eq!(f.bits[0], Some(true));
        assert!(f.group.is_none() && f.owner.as_deref() == Some("me") && f.mine);
        let mut e = PermEdit::from(f);
        assert!(!e.changed());
        e.bits[4].toggle();
        assert_eq!(e.change().set, 0o020);
        e.new_gid = Some(1);
        assert_eq!(e.change().gid, Some(1), "a group the items don't all share is a change");
        e.commit(&[(1, "g1".into())]);
        assert!(!e.changed());
        assert_eq!(e.fold.group, Some((1, "g1".into())));
    }

    #[test]
    fn allow_running_follows_read() {
        let mut f = Fold::default();
        f.add(&item(0o640, 1, false));
        let mut e = PermEdit::from(f);
        e.set_executable(true);
        assert_eq!(e.change().set, 0o110, "owner and group (who may read), not others");
        e.set_executable(false);
        assert!(!e.changed());
        let _ = BITS;
    }

    #[test]
    fn kinds_of_files() {
        assert!(is_pe(Path::new(r"C:\a\b.EXE")) && is_pe(Path::new("x.dll")) && !is_pe(Path::new("x.txt")));
        assert!(is_root(Path::new("/")) && !is_root(Path::new("/home")));
    }
}
