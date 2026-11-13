//! The Open/Save dialog other apps get when rfiles is their portal's FileChooser
//! (`rfiles --file-chooser-service`, see rf-linux `file_chooser`). One window per request:
//! places on the left, the folder list (a [`FolderView`] in picker mode) and, at the bottom,
//! the file name (saving), the file type and the buttons.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use futures::channel::oneshot;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{Icon, TitleBar, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use rf_core::history::Location;
use rf_linux::file_chooser::{Kind, Outcome, Request};

use crate::folder_view::{FolderEvent, FolderView};
use crate::image_cache::ImageCache;
use crate::places::Places;
use crate::select_box::{Choice, select_box};
use crate::t;
use crate::theme::{self, Palette};
use gpui_kit::assets::IconName as L;

/// Pickers open now (the service quits once none has been open for a while).
static OPEN: AtomicUsize = AtomicUsize::new(0);

/// The service quits after this long with no picker open.
const IDLE: Duration = Duration::from_secs(120);

/// Starts answering the portal's calls in this (windowless) app: each opens a picker. Quits
/// when idle, and at once if another rfiles already serves the portal.
pub(crate) fn serve(cx: &mut App) {
    let (tx, mut rx) = futures::channel::mpsc::unbounded::<(Request, oneshot::Sender<Outcome>, oneshot::Receiver<()>)>();
    let handler: rf_linux::file_chooser::Handler = std::sync::Arc::new(move |req, reply, close| {
        let _ = tx.unbounded_send((req, reply, close));
    });
    if let Err(e) = rf_linux::file_chooser::serve(handler) {
        log::warn!("{}: {e:#}", rf_linux::file_chooser::BUS_NAME);
        cx.quit();
        return;
    }
    let images = cx.new(ImageCache::new);
    let places = cx.new(Places::new);
    cx.spawn(async move |cx| {
        use futures::StreamExt;
        while let Some((req, reply, close)) = rx.next().await {
            let (images, places) = (images.clone(), places.clone());
            cx.update(|cx| open(req, reply, close, places, images, cx));
        }
    })
    .detach();
    // idle: no picker for a while
    cx.spawn(async move |cx| {
        let mut idle_since = Instant::now();
        loop {
            cx.background_executor().timer(Duration::from_secs(5)).await;
            if OPEN.load(Ordering::Relaxed) > 0 {
                idle_since = Instant::now();
            } else if idle_since.elapsed() > IDLE {
                cx.update(|cx| cx.quit());
                break;
            }
        }
    })
    .detach();
}

fn open(req: Request, reply: oneshot::Sender<Outcome>, close: oneshot::Receiver<()>, places: Entity<Places>, images: Entity<ImageCache>, cx: &mut App) {
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::centered(size(px(960.), px(620.)), cx)),
        window_min_size: Some(size(px(560.), px(380.))),
        window_background: WindowBackgroundAppearance::MicaBackdrop,
        // its own app id, so tiling compositors can float it by rule (they can't tell it's a
        // dialog: Wayland gives it no parent in another app)
        app_id: Some("rfiles-dialog".into()),
        // a dialog: tiling compositors float it (Wayland's xdg-dialog)
        kind: WindowKind::Dialog,
        ..TitleBar::window_options()
    };
    let opened = gpui_kit::open_window(options, cx, move |window, cx| cx.new(|cx| PickerView::new(req, reply, places, images, window, cx)));
    match opened {
        Ok(handle) => {
            // the portal took the request back (the app gave up): close the window
            cx.spawn(async move |cx| {
                if close.await.is_ok() {
                    let _ = cx.update(|cx| handle.0.update(cx, |_, window, _| window.remove_window()));
                }
            })
            .detach();
        }
        Err(e) => log::error!("file chooser window: {e:#}"),
    }
}

pub(crate) struct PickerView {
    req: Request,
    reply: Option<oneshot::Sender<Outcome>>,
    view: Entity<FolderView>,
    places: Entity<Places>,
    name: Entity<InputState>,
    filter: Option<usize>,
    /// Saving over this existing file: asked first.
    confirm_replace: Option<PathBuf>,
    error: Option<String>,
    /// A name to put in the box at the next render (a file picked in the list).
    pending_name: Option<String>,
    _subs: Vec<Subscription>,
}

impl PickerView {
    fn new(req: Request, reply: oneshot::Sender<Outcome>, places: Entity<Places>, images: Entity<ImageCache>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        OPEN.fetch_add(1, Ordering::Relaxed);
        let start = req
            .current_file
            .as_deref()
            .and_then(Path::parent)
            .map(Path::to_path_buf)
            .or_else(|| req.current_folder.clone())
            .filter(|d| d.is_dir())
            .or_else(dirs::home_dir)
            .unwrap_or_else(|| PathBuf::from("/"));
        let view = cx.new(|cx| FolderView::new(Location::Path(start), places.clone(), images, cx));
        let suggested = req.current_name.clone().or_else(|| req.current_file.as_deref().and_then(Path::file_name).map(|n| n.to_string_lossy().into_owned())).unwrap_or_default();
        let name = cx.new(|cx| {
            let mut s = InputState::new(window, cx).placeholder(t!("File name"));
            s.set_value(suggested.clone(), window, cx);
            s
        });
        let mut subs = vec![cx.subscribe_in(&view, window, |this, _, ev: &FolderEvent, window, cx| match ev {
            FolderEvent::Picked(paths) => this.accept_paths(paths.clone(), window, cx),
            FolderEvent::OpenInNewTab(Location::Path(p)) | FolderEvent::OpenInNewWindow(Location::Path(p)) => {
                let p = p.clone();
                this.view.update(cx, |v, cx| v.navigate(Location::Path(p), cx));
            }
            _ => {}
        })];
        subs.push(cx.subscribe_in(&name, window, |this, _, ev: &InputEvent, window, cx| {
            if let InputEvent::PressEnter { .. } = ev {
                this.accept(window, cx);
            }
            if let InputEvent::Change = ev {
                this.confirm_replace = None;
                this.error = None;
                cx.notify();
            }
        }));
        // a file picked in the list while saving: its name, to save over it
        subs.push(cx.observe(&view, |this, view, cx| {
            if this.req.kind == Kind::Save
                && let [p] = view.read(cx).selection_paths().as_slice()
                && !p.is_dir()
                && let Some(n) = p.file_name()
            {
                let n = n.to_string_lossy().into_owned();
                this.pending_name = Some(n);
            }
            cx.notify();
        }));
        let mut this = Self { req, reply: Some(reply), view, places, name, filter: None, confirm_replace: None, error: None, _subs: subs, pending_name: None };
        this.filter = this.req.current_filter.or((!this.req.filters.is_empty()).then_some(0));
        this.apply_filter(cx);
        if this.req.kind == Kind::Save {
            // the name, without its extension, ready to type over
            this.name.update(cx, |s, cx| s.focus(window, cx));
        } else {
            let focus = this.view.read(cx).focus.clone();
            window.focus(&focus, cx);
        }
        this
    }

    fn apply_filter(&mut self, cx: &mut Context<Self>) {
        let spec = self.filter.and_then(|i| self.req.filters.get(i)).map(|f| f.glob_spec()).unwrap_or_default();
        let folders_only = self.req.directory;
        self.view.update(cx, |v, cx| v.set_picker(spec, folders_only, cx));
    }

    fn dir(&self, cx: &App) -> Option<PathBuf> {
        self.view.read(cx).location().path().map(Path::to_path_buf)
    }

    fn finish(&mut self, outcome: Outcome, window: &mut Window) {
        if let Some(r) = self.reply.take() {
            let _ = r.send(outcome);
        }
        window.remove_window();
    }

    fn cancel(&mut self, window: &mut Window) {
        self.finish(Outcome::Cancelled, window);
    }

    /// Paths opened in the list (double-click, Enter on files).
    fn accept_paths(&mut self, paths: Vec<PathBuf>, window: &mut Window, cx: &mut Context<Self>) {
        match self.req.kind {
            Kind::Open if !self.req.directory => {
                let paths = if self.req.multiple { paths } else { paths.into_iter().take(1).collect() };
                self.finish(Outcome::Picked { paths, filter: self.filter }, window);
            }
            // saving: a file opened in the list is the one to save over
            Kind::Save => {
                if let Some(n) = paths.first().and_then(|p| p.file_name()) {
                    let n = n.to_string_lossy().into_owned();
                    self.name.update(cx, |s, cx| s.set_value(n, window, cx));
                }
                self.accept(window, cx);
            }
            _ => {}
        }
    }

    /// The accept button (Open / Save / Select).
    fn accept(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(dir) = self.dir(cx) else { return };
        let selected = self.view.read(cx).selection_paths();
        match self.req.kind {
            Kind::Open if self.req.directory => {
                let dirs: Vec<PathBuf> = selected.into_iter().filter(|p| p.is_dir()).collect();
                let paths = if dirs.is_empty() { vec![dir] } else if self.req.multiple { dirs } else { dirs.into_iter().take(1).collect() };
                self.finish(Outcome::Picked { paths, filter: self.filter }, window);
            }
            Kind::Open => {
                // one folder selected: go into it, like a double-click
                if let [only] = selected.as_slice()
                    && only.is_dir()
                {
                    let p = only.clone();
                    self.view.update(cx, |v, cx| v.navigate(Location::Path(p), cx));
                    return;
                }
                let files: Vec<PathBuf> = selected.into_iter().filter(|p| !p.is_dir()).collect();
                if !files.is_empty() {
                    self.accept_paths(files, window, cx);
                }
            }
            Kind::SaveFiles => {
                let target = match selected.as_slice() {
                    [only] if only.is_dir() => only.clone(),
                    _ => dir,
                };
                self.finish(Outcome::Picked { paths: vec![target], filter: self.filter }, window);
            }
            Kind::Save => {
                let name = self.name.read(cx).value().trim().to_string();
                if name.is_empty() {
                    return;
                }
                if name.contains('/') && !name.starts_with('/') && !Path::new(&dir).join(&name).parent().is_some_and(Path::is_dir) {
                    self.error = Some(t!("That folder doesn't exist").to_string());
                    cx.notify();
                    return;
                }
                let target = if name.starts_with('/') { PathBuf::from(&name) } else { dir.join(&name) };
                if target.is_dir() {
                    // a folder's name typed: go there
                    self.name.update(cx, |s, cx| s.set_value("", window, cx));
                    self.view.update(cx, |v, cx| v.navigate(Location::Path(target), cx));
                    return;
                }
                if target.exists() && self.confirm_replace.as_deref() != Some(&target) {
                    self.confirm_replace = Some(target);
                    cx.notify();
                    return;
                }
                self.finish(Outcome::Picked { paths: vec![target], filter: self.filter }, window);
            }
        }
    }

    fn accept_label(&self) -> String {
        if let Some(l) = &self.req.accept_label {
            // GTK mnemonics: "_Open"
            return l.replace('_', "");
        }
        match self.req.kind {
            Kind::Open if self.req.directory => t!("Select").to_string(),
            Kind::Open => t!("Open").to_string(),
            Kind::Save | Kind::SaveFiles => t!("Save").to_string(),
        }
    }

    fn title(&self) -> String {
        if !self.req.title.is_empty() {
            return self.req.title.clone();
        }
        match self.req.kind {
            Kind::Open if self.req.directory => t!("Select a folder").to_string(),
            Kind::Open => t!("Open").to_string(),
            Kind::Save | Kind::SaveFiles => t!("Save").to_string(),
        }
    }

    fn sidebar(&self, p: &Palette, cx: &mut Context<Self>) -> impl IntoElement {
        let mut rows: Vec<(String, PathBuf, L)> = Vec::new();
        if let Some(h) = dirs::home_dir() {
            rows.push((t!("Home").to_string(), h, L::House));
        }
        let places = self.places.read(cx);
        for k in rf_linux::known::known_folders() {
            rows.push((crate::i18n::t(k.0.title()).to_string(), k.1, crate::places::known_icon(k.0)));
        }
        for pin in &places.pinned {
            if !rows.iter().any(|r| r.1 == pin.path) {
                rows.push((pin.title.clone(), pin.path.clone(), L::Folder));
            }
        }
        for d in places.drives.iter().filter(|d| !crate::places::unmounted(d)) {
            rows.push((crate::places::drive_name(d), d.root.clone(), crate::places::drive_icon(d)));
        }
        let here = self.dir(cx);
        v_flex().w(px(200.)).flex_none().h_full().py_2().px_2().gap(px(2.)).children(rows.into_iter().enumerate().map(|(i, (title, path, icon))| {
            let on = here.as_deref() == Some(path.as_path());
            h_flex()
                .id(("pick-place", i))
                .h(px(32.))
                .px_2()
                .gap_2()
                .items_center()
                .rounded(theme::RADIUS_SM)
                .text_sm()
                .when(on, |d| d.bg(p.selection))
                .when(!on, |d| d.hover(|s| s.bg(p.hover)))
                .cursor_pointer()
                .child(Icon::new(icon).size(px(16.)).text_color(p.text_secondary))
                .child(div().truncate().child(title))
                .on_click(cx.listener(move |this, _, _, cx| {
                    let path = path.clone();
                    this.view.update(cx, |v, cx| v.navigate(Location::Path(path), cx));
                }))
        }))
    }
}

impl Drop for PickerView {
    fn drop(&mut self) {
        OPEN.fetch_sub(1, Ordering::Relaxed);
        // closed with the window's own close button
        if let Some(r) = self.reply.take() {
            let _ = r.send(Outcome::Cancelled);
        }
    }
}

fn button(id: &'static str, label: String, primary: bool, p: &Palette) -> Stateful<Div> {
    h_flex()
        .id(id)
        .h(px(32.))
        .px_4()
        .items_center()
        .justify_center()
        .min_w(px(96.))
        .rounded(theme::RADIUS_SM)
        .text_sm()
        .cursor_pointer()
        .when(primary, |d| d.bg(p.accent).text_color(p.accent_text).hover(|s| s.opacity(0.9)))
        .when(!primary, |d| d.bg(p.hover).border_1().border_color(p.border).hover(|s| s.bg(p.pressed)))
        .child(label)
}

impl Render for PickerView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = Palette::for_window(window);
        if let Some(n) = self.pending_name.take() {
            self.name.update(cx, |s, cx| s.set_value(n, window, cx));
        }
        let title = self.title();
        window.set_window_title(&title);
        let here = self.dir(cx);
        let (can_back, can_up) = {
            let v = self.view.read(cx);
            (v.history.can_back(), here.as_deref().and_then(Path::parent).is_some())
        };
        let nav = |id: &'static str, icon: L, enabled: bool| {
            h_flex()
                .id(id)
                .size(px(30.))
                .items_center()
                .justify_center()
                .rounded(theme::RADIUS_SM)
                .when(enabled, |d| d.hover(|s| s.bg(p.hover)).cursor_pointer())
                .when(!enabled, |d| d.opacity(0.4))
                .child(Icon::new(icon).size(px(16.)))
        };
        let path_text = here.as_ref().map(|d| d.to_string_lossy().into_owned()).unwrap_or_default();
        let header = h_flex()
            .h(px(44.))
            .px_3()
            .gap_1()
            .items_center()
            .border_b_1()
            .border_color(p.divider)
            .child(nav("pick-back", L::ArrowLeft, can_back).on_click(cx.listener(|this, _, _, cx| this.view.update(cx, |v, cx| v.back(cx)))))
            .child(nav("pick-up", L::ArrowUp, can_up).on_click(cx.listener(|this, _, _, cx| this.view.update(cx, |v, cx| v.up(cx)))))
            .child(div().flex_1().min_w_0().px_2().text_sm().truncate().child(path_text));

        let filters: Vec<(usize, String)> = self.req.filters.iter().enumerate().map(|(i, f)| (i, f.name.clone())).collect();
        let filter_box = (!filters.is_empty()).then(|| {
            let current = self.filter;
            let label = current.and_then(|i| filters.get(i)).map(|f| f.1.clone()).unwrap_or_default();
            let me = cx.entity().downgrade();
            select_box("pick-filter", &p, label, move |_| {
                filters
                    .iter()
                    .map(|(i, name)| {
                        let (i, me) = (*i, me.clone());
                        Choice::new(name.clone(), Some(i) == current, move |_, cx| {
                            let _ = me.update(cx, |this, cx| {
                                this.filter = Some(i);
                                this.apply_filter(cx);
                            });
                        })
                    })
                    .collect()
            })
        });
        let saving = self.req.kind == Kind::Save;
        let replace = self.confirm_replace.as_ref().and_then(|t| t.file_name()).map(|n| n.to_string_lossy().into_owned());
        let bottom = v_flex()
            .px_4()
            .py_3()
            .gap_2()
            .border_t_1()
            .border_color(p.divider)
            .children(replace.map(|n| {
                div().text_sm().text_color(p.danger).child(format!("{} {}", n, t!("already exists. Save again to replace it.")))
            }))
            .children(self.error.clone().map(|e| div().text_sm().text_color(p.danger).child(e)))
            .children((self.req.kind == Kind::SaveFiles && !self.req.files.is_empty()).then(|| {
                div().text_sm().text_color(p.text_secondary).child(format!("{}: {}", t!("Files to save"), self.req.files.join(", ")))
            }))
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .when(saving, |d| {
                        d.child(div().text_sm().child(t!("Name")))
                            .child(div().flex_1().min_w_0().child(Input::new(&self.name)))
                    })
                    .when(!saving, |d| d.child(div().flex_1()))
                    .children(filter_box)
                    .child(button("pick-cancel", t!("Cancel").to_string(), false, &p).on_click(cx.listener(|this, _, window, _| this.cancel(window))))
                    .child(button("pick-accept", self.accept_label(), true, &p).on_click(cx.listener(|this, _, window, cx| this.accept(window, cx)))),
            );

        v_flex()
            .size_full()
            .bg(p.window_bg)
            .text_color(p.text)
            .on_key_down(cx.listener(|this, ev: &KeyDownEvent, window, cx| {
                if ev.keystroke.key == "escape" && !crate::fluent_menu::is_open(cx) {
                    this.cancel(window);
                }
            }))
            .child(
                TitleBar::new()
                    .h(theme::TITLEBAR_H)
                    .bg(p.titlebar_bg)
                    .border_color(gpui_kit::transparent_black())
                    .child(h_flex().gap_2().items_center().text_xs().child(Icon::new(L::FolderOpen).size(px(14.))).child(title)),
            )
            .child(header)
            .child(
                h_flex()
                    .flex_1()
                    .min_h_0()
                    .child(self.sidebar(&p, cx))
                    .child(div().flex_1().min_w_0().h_full().mr_2().rounded(theme::RADIUS).overflow_hidden().bg(p.content_bg).child(self.view.clone())),
            )
            .child(bottom)
            // last children = painted on top
            .children(crate::fluent_menu::current(window, cx))
            .children(crate::prompt::current(window, cx))
            .children(crate::conflict_dialog::current(window, cx))
            .children(crate::flyout::current(window, cx))
            .children(crate::select_box::current(window, cx))
    }
}
