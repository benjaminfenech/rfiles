//! Preview / details pane: large thumbnail, text/code preview and file properties for the
//! focused item. Content is read on background threads; nothing here blocks the UI.

use crate::t;
use std::path::{Path, PathBuf};

use gpui_kit::assets::IconName as L;
use gpui_kit::component::{Icon, h_flex, v_flex};
use gpui_kit::*;
use gpui_kit::prelude::FluentBuilder as _;
use rf_core::{Entry, format};

use crate::icons::{self, Category};
use crate::image_cache::ImageCache;
use crate::theme::{self, Palette};

const TEXT_LIMIT: usize = 48 * 1024;
const PREVIEW_PX: u32 = 384;

/// Shared folder-size cache (Size column, preview pane, Properties window).
pub fn size_cache() -> &'static rf_linux::foldersize::FolderSizeCache {
    rf_linux::foldersize::shared_cache()
}

pub struct Preview {
    target: Option<(PathBuf, Entry)>,
    text: Option<(PathBuf, SharedString)>,
    folder_size: Option<(PathBuf, Option<rf_linux::foldersize::FolderSize>)>,
    hash: Option<Entity<crate::hash_panel::HashPanel>>,
    /// Media/document properties (duration, resolution, …) for the current target.
    media: Option<(PathBuf, Vec<(&'static str, String)>)>,
    _media_task: Option<Task<()>>,
    size_cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
    images: Entity<ImageCache>,
    _task: Option<Task<()>>,
    /// Quick Look (Space): one big visual and a one-line summary instead of the pane layout.
    large: bool,
    /// Quick Look: size of the visual box (measured while painting).
    ql_box: std::cell::Cell<(f32, f32)>,
    /// Archive top level / shortcut target for the current target.
    extra: Option<(PathBuf, Extra)>,
    _extra_task: Option<Task<()>>,
    /// The last thumbnail shown, and when the target last changed: while the next file's
    /// thumbnail loads, the previous one stays up briefly instead of flashing the type icon.
    last_thumb: Option<std::sync::Arc<RenderImage>>,
    /// An item inside an archive and its extracted copy (thumbnails come from the copy).
    archive_copy: Option<(PathBuf, PathBuf)>,
    _archive_task: Option<Task<()>>,
    changed_at: std::time::Instant,
    _hold_task: Option<Task<()>>,
}

/// How long the previous thumbnail may stand in for one that is still loading.
const HOLD: std::time::Duration = std::time::Duration::from_millis(300);

enum Extra {
    /// (name, is folder, size) of the top-level entries, and how many there are in total.
    Archive(Vec<(String, bool, u64)>, usize),
}

fn is_markdown(e: &Entry) -> bool {
    matches!(&*e.ext, "md" | "markdown" | "mdx")
}

fn is_text_like(e: &Entry) -> bool {
    matches!(icons::category(e), Category::Code | Category::Text)
        || matches!(&*e.ext, "csv" | "gitignore" | "env" | "editorconfig" | "gitattributes" | "srt")
}

impl Preview {
    pub fn new(images: Entity<ImageCache>, cx: &mut Context<Self>) -> Self {
        cx.observe(&images, |_, _, cx| cx.notify()).detach();
        Self {
            target: None,
            text: None,
            folder_size: None,
            hash: None,
            media: None,
            _media_task: None,
            size_cancel: Default::default(),
            images,
            _task: None,
            large: false,
            ql_box: Default::default(),
            extra: None,
            _extra_task: None,
            last_thumb: None,
            archive_copy: None,
            _archive_task: None,
            changed_at: std::time::Instant::now(),
            _hold_task: None,
        }
    }

    /// The thumbnail to draw for `e`: its own once loaded; meanwhile, for a short while, the
    /// previous one (arrow-keying through photos shouldn't flash an icon between them).
    fn thumb_or_hold(&mut self, dir: &Path, e: &Entry, size: u32, cx: &mut Context<Self>) -> Option<std::sync::Arc<RenderImage>> {
        // an item inside an archive: its extracted copy, once there
        let copy_dir = self
            .archive_copy
            .as_ref()
            .filter(|(p, _)| p.parent() == Some(dir) && p.file_name().is_some_and(|n| n.to_string_lossy() == *e.name))
            .and_then(|(_, real)| real.parent().map(Path::to_path_buf));
        let dir = copy_dir.as_deref().unwrap_or(dir);
        let wants = icons::has_thumbnail(e);
        let thumb = if wants { self.images.update(cx, |c, cx| c.thumbnail(dir, e, size, cx)) } else { None };
        match thumb {
            Some(im) => {
                self.last_thumb = Some(im.clone());
                Some(im)
            }
            None if wants && self.changed_at.elapsed() < HOLD => self.last_thumb.clone(),
            None => None,
        }
    }

    fn render_large(&mut self, path: PathBuf, e: Entry, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let p = Palette::for_window(window);
        let cat = icons::category(&e);
        let dir = path.parent().map(Path::to_path_buf).unwrap_or_default();
        let text = self.text.as_ref().filter(|(tp, _)| *tp == path).map(|(_, t)| t.clone());
        let visual: AnyElement = if let Some(text) = text {
            div()
                .id("ql-text")
                .size_full()
                .overflow_y_scroll()
                .p_4()
                .rounded(theme::RADIUS_SM)
                .bg(p.content_bg)
                .text_sm()
                .when(!is_markdown(&e), |d| d.font_family(crate::sys::mono_font()).child(text.clone()))
                .when(is_markdown(&e), |d| d.child(gpui_kit::component::text::TextView::markdown("ql-md", text.clone())))
                .into_any_element()
        } else {
            let thumb = self.thumb_or_hold(&dir, &e, 1280, cx);
            match thumb {
                Some(im) => {
                    // fit by hand: GPUI sizes images by their aspect ratio from the width, which
                    // overflows a wide box with a tall image
                    let s = im.size(0);
                    let (iw, ih) = ((s.width.0 as f32).max(1.), (s.height.0 as f32).max(1.));
                    let (bw, bh) = self.ql_box.get();
                    let k = (bw / iw).min(bh / ih);
                    let me = cx.entity().downgrade();
                    div()
                        .size_full()
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(
                            canvas(
                                move |bounds, _, cx| {
                                    let b = (f32::from(bounds.size.width), f32::from(bounds.size.height));
                                    let _ = me.update(cx, |pv, cx| {
                                        if pv.ql_box.get() != b {
                                            pv.ql_box.set(b);
                                            cx.notify();
                                        }
                                    });
                                },
                                |_, _, _, _| {},
                            )
                            .absolute()
                            .size_full(),
                        )
                        .children((bw > 0.).then(|| img(im).w(px(iw * k)).h(px(ih * k))))
                        .into_any_element()
                }
                None => div()
                    .size_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(Icon::new(icons::icon_for(cat)).size(px(160.)).text_color(theme::kind_color(cat, p.dark)))
                    .into_any_element(),
            }
        };
        // one line: type · size · the first media facts (resolution, duration…)
        let mut facts = vec![icons::type_label(&e).to_string()];
        if !e.is_dir() {
            facts.push(format::size(e.size));
        }
        if let Some((mp, list)) = &self.media {
            if *mp == path {
                facts.extend(list.iter().take(2).map(|(_, v)| v.clone()));
            }
        }
        facts.push(format::datetime(e.modified_ms));
        v_flex()
            .size_full()
            .gap_3()
            // absolute child = a definite box, so `contain` fits instead of overflowing
            .child(div().flex_1().min_h_0().w_full().relative().overflow_hidden().child(div().absolute().inset_0().child(visual)))
            .child(
                h_flex()
                    .w_full()
                    .gap_3()
                    .items_baseline()
                    .child(div().text_base().font_weight(FontWeight::SEMIBOLD).child(SharedString::from(e.name.to_string())))
                    .child(div().text_sm().text_color(p.text_secondary).child(facts.join(" · "))),
            )
            .into_any_element()
    }

    /// The drive is about to be ejected: drop a target on it (a hosted preview handler keeps
    /// the file open, which would block the removal).
    pub fn release_where(&mut self, on_it: impl Fn(&std::path::Path) -> bool, cx: &mut Context<Self>) {
        if self.target.as_ref().is_some_and(|(p, _)| on_it(p)) {
            self.set_target(None, cx);
            cx.notify();
        }
    }

    /// Called by the workspace whenever the focused item may have changed.
    pub fn set_target(&mut self, target: Option<(PathBuf, Entry)>, cx: &mut Context<Self>) {
        let same = match (&self.target, &target) {
            (Some((a, ea)), Some((b, eb))) => a == b && ea.modified_ms == eb.modified_ms && ea.size == eb.size,
            (None, None) => true,
            _ => false,
        };
        if same {
            return;
        }
        self.target = target;
        self.changed_at = std::time::Instant::now();
        // redraw once the hold is over: the icon then shows if the thumbnail still isn't there
        self._hold_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(HOLD).await;
            let _ = this.update(cx, |_, cx| cx.notify());
        }));
        self._task = None;
        self.media = None;
        self._media_task = None;
        self.extra = None;
        self._extra_task = None;
        if let Some((path, e)) = &self.target {
            let p = path.clone();
            let work: Option<Task<Option<Extra>>> = if e.is_dir() || e.is_cloud_placeholder() {
                None
            } else if crate::archive_view::is_browsable(path) {
                Some(cx.background_spawn(async move {
                    let all = crate::archive_view::list_inside(&p, Path::new("")).ok()?;
                    let total = all.len();
                    let mut v: Vec<(String, bool, u64)> = all.into_iter().map(|e| (e.name.to_string(), e.is_dir(), e.size)).collect();
                    v.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.to_lowercase().cmp(&b.0.to_lowercase())));
                    v.truncate(200);
                    Some(Extra::Archive(v, total))
                }))
            } else {
                None
            };
            if let Some(work) = work {
                let p = path.clone();
                self._extra_task = Some(cx.spawn(async move |this, cx| {
                    let r = work.await;
                    let _ = this.update(cx, |v, cx| {
                        if let Some(x) = r {
                            v.extra = Some((p, x));
                            cx.notify();
                        }
                    });
                }));
            }
        }
        if let Some((path, e)) = &self.target {
            let rich = matches!(
                icons::category(e),
                Category::Image | Category::Video | Category::Audio | Category::Pdf | Category::Document | Category::Spreadsheet
            );
            if rich && !e.is_dir() && !e.is_cloud_placeholder() {
                let p = path.clone();
                let p2 = path.clone();
                let work = cx.background_spawn(async move { rf_linux::props::media_props(&p2) });
                self._media_task = Some(cx.spawn(async move |this, cx| {
                    let props = work.await;
                    let _ = this.update(cx, |v, cx| {
                        if v.target.as_ref().is_some_and(|(tp, _)| *tp == p) {
                            v.media = Some((p, props));
                            cx.notify();
                        }
                    });
                }));
            }
        }
        // Hash panel follows the selection while open (only for files).
        match (&self.target, self.hash.clone()) {
            (Some((p, e)), Some(h)) if !e.is_dir() => h.update(cx, |hp, cx| hp.set_path(p.clone(), cx)),
            (_, Some(_)) => self.hash = None,
            _ => {}
        }
        self.size_cancel.store(true, std::sync::atomic::Ordering::Relaxed);
        self.folder_size = None;
        if let Some((path, e)) = &self.target {
            // a folder inside an archive has no size on disk to walk
            if e.is_dir() && !crate::archive_view::is_browsable_path(path) {
                let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
                self.size_cancel = cancel.clone();
                let p = path.clone();
                self.folder_size = Some((p.clone(), None));
                let work = cx.background_spawn(async move {
                    let cache = size_cache();
                    cache.get_or_compute(&p, &|| cancel.load(std::sync::atomic::Ordering::Relaxed), &mut |_| {}).ok()
                });
                let p2 = path.clone();
                cx.spawn(async move |this, cx| {
                    let r = work.await;
                    let _ = this.update(cx, |v, cx| {
                        if let (Some((cur, slot)), Some(r)) = (v.folder_size.as_mut(), r) {
                            if *cur == p2 {
                                *slot = Some(r);
                                cx.notify();
                            }
                        }
                    });
                })
                .detach();
            }
        }
        if let Some((path, e)) = &self.target {
            // by the path alone (no disk access on the UI thread): a parent named like an archive
            let in_archive = path.ancestors().skip(1).any(|a| a.as_os_str().len() > 3 && crate::archive_view::is_browsable(a));
            if in_archive && e.size <= crate::archive_view::PREVIEW_EXTRACT_MAX && !e.is_dir() && icons::has_thumbnail(e) {
                // images (and other thumbnailed files) inside an archive: extract, then the
                // thumbnail is made from the copy
                let p2 = path.clone();
                let work = cx.background_spawn(async move { crate::archive_view::real_copy(&p2) });
                let path2 = path.clone();
                self._archive_task = Some(cx.spawn(async move |this, cx| {
                    if let Some(real) = work.await {
                        let _ = this.update(cx, |v, cx| {
                            v.archive_copy = Some((path2, real));
                            cx.notify();
                        });
                    }
                }));
            }
            if !e.is_dir() && is_text_like(e) && e.size > 0 && !e.is_cloud_placeholder() && (!in_archive || e.size <= crate::archive_view::PREVIEW_EXTRACT_MAX) {
                let path = path.clone();
                let p2 = path.clone();
                let read = cx.background_spawn(async move {
                    // inside an archive: the extracted copy (small members only)
                    let real = if p2.exists() { Some(p2) } else { crate::archive_view::real_copy(&p2) };
                    real.and_then(|r| read_text_head(&r))
                });
                self._task = Some(cx.spawn(async move |this, cx| {
                    let text = read.await;
                    let _ = this.update(cx, |v, cx| {
                        v.text = text.map(|t| (path, t.into()));
                        cx.notify();
                    });
                }));
            }
        }
        cx.notify();
    }
}

fn read_text_head(path: &Path) -> Option<String> {
    use std::io::Read;
    let mut f = std::fs::File::open(path).ok()?;
    let mut buf = vec![0u8; TEXT_LIMIT];
    let n = f.read(&mut buf).ok()?;
    buf.truncate(n);
    if buf.iter().take(4096).filter(|&&b| b == 0).count() > 2 {
        return None; // binary
    }
    // UTF-16 LE BOM
    if buf.starts_with(&[0xFF, 0xFE]) {
        let u: Vec<u16> = buf[2..].chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
        return Some(String::from_utf16_lossy(&u));
    }
    // UTF-8 (with or without BOM), else the locale's legacy ANSI page (Turkish: Windows-1254)
    let s = rf_linux::encoding::decode_text(buf.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(&buf));
    // keep it light: at most 400 lines
    Some(s.lines().take(400).collect::<Vec<_>>().join("\n"))
}

/// Archive contents / shortcut target block of the pane.
fn render_extra(p: &Palette, x: &Extra) -> AnyElement {
    match x {
        Extra::Archive(list, total) => {
            let rows = list.iter().map(|(name, dir, size)| {
                h_flex()
                    .gap_2()
                    .items_center()
                    .text_xs()
                    .child(Icon::new(if *dir { L::Folder } else { L::File }).size(px(12.)).text_color(p.text_secondary))
                    .child(div().flex_1().min_w_0().overflow_hidden().whitespace_nowrap().text_ellipsis().child(name.clone()))
                    .children((!*dir).then(|| div().text_color(p.text_tertiary).child(format::size(*size))))
            });
            v_flex()
                .id("pv-archive")
                .w_full()
                .max_h(px(300.))
                .overflow_y_scroll()
                .p_2()
                .gap_1()
                .rounded(theme::RADIUS_SM)
                .bg(p.tile_bg)
                .border_1()
                .border_color(p.border)
                .child(div().text_xs().text_color(p.text_secondary).child(format!("{} — {}", t!("Archive contents"), crate::i18n::items(*total))))
                .children(rows)
                .into_any_element()
        }
    }
}

fn prop_row(p: &Palette, k: &'static str, v: String) -> impl IntoElement {
    h_flex()
        .w_full()
        .gap_2()
        .items_start()
        .text_xs()
        .child(div().w(px(92.)).flex_shrink_0().text_color(p.text_secondary).child(k))
        .child(div().flex_1().min_w_0().overflow_hidden().child(v))
}

impl Render for Preview {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = Palette::for_window(window);
        let mut col = v_flex().id("preview").size_full().p_3().gap_3().overflow_y_scroll();
        let Some((path, e)) = self.target.clone() else {
            return col
                .items_center()
                .justify_center()
                .child(Icon::new(L::PanelRight).size(px(32.)).text_color(p.text_tertiary))
                .child(div().text_sm().text_color(p.text_secondary).child(t!("Select an item to preview")))
                .into_any_element();
        };
        if self.large {
            return self.render_large(path, e, window, cx);
        }
        let cat = icons::category(&e);
        let dir = path.parent().map(Path::to_path_buf).unwrap_or_default();
        let visual_box: AnyElement = {
            let thumb = self.thumb_or_hold(&dir, &e, PREVIEW_PX, cx);
            let visual: AnyElement = match thumb {
                Some(im) => img(im).w_full().h(px(220.)).object_fit(ObjectFit::Contain).into_any_element(),
                None => Icon::new(icons::icon_for(cat))
                    .size(px(96.))
                    .text_color(theme::kind_color(cat, p.dark))
                    .into_any_element(),
            };
            div()
                .w_full()
                .h(px(232.))
                .flex()
                .items_center()
                .justify_center()
                .rounded(theme::RADIUS)
                .bg(p.hover)
                .child(visual)
                .into_any_element()
        };
        col = col
            .child(visual_box)
            .child(
                div()
                    .text_sm()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(SharedString::from(e.name.to_string())),
            );

        if let Some((tp, text)) = &self.text {
            if *tp == path {
                let md = is_markdown(&e);
                col = col.child(
                    div()
                        .id("preview-text")
                        .w_full()
                        .max_h(px(360.))
                        .overflow_y_scroll()
                        .p_2()
                        .rounded(theme::RADIUS_SM)
                        .bg(p.tile_bg)
                        .border_1()
                        .border_color(p.border)
                        .text_xs()
                        .when(!md, |d| d.font_family(crate::sys::mono_font()).text_color(p.text_secondary).child(text.clone()))
                        .when(md, |d| d.child(gpui_kit::component::text::TextView::markdown("pv-md", text.clone()))),
                );
            }
        }
        if let Some((xp, x)) = &self.extra {
            if *xp == path {
                col = col.child(render_extra(&p, x));
            }
        }

        let mut props = v_flex().w_full().gap_1p5();
        props = props.child(prop_row(&p, t!("Type"), icons::type_label(&e)));
        if e.is_dir() && !crate::archive_view::is_browsable_path(&path) {
            let on_disk = cx.global::<crate::settings::Settings>().folder_size_on_disk;
            let txt = match &self.folder_size {
                Some((fp, Some(fs))) if *fp == path => {
                    let shown = if on_disk { fs.allocated.unwrap_or(fs.bytes) } else { fs.bytes };
                    format!("{} · {} {}, {} {}", format::size(shown), group(fs.files), t!("files"), group(fs.dirs), t!("folders"))
                }
                _ => t!("Calculating…").to_string(),
            };
            props = props.child(prop_row(&p, t!("Size"), txt));
        }
        if !e.is_dir() {
            props = props.child(prop_row(&p, t!("Size"), format!("{} ({} {})", format::size(e.size), group(e.size), t!("bytes"))));
        }
        if let Some((mp, list)) = &self.media {
            if *mp == path {
                for (label, value) in list {
                    props = props.child(prop_row(&p, crate::i18n::t(label), value.clone()));
                }
            }
        }
        props = props
            .child(prop_row(&p, t!("Modified"), format::datetime(e.modified_ms)))
            .child(prop_row(&p, t!("Created"), format::datetime(e.created_ms)))
            // zero-width space after each separator: long paths wrap at folder boundaries
            .child(prop_row(&p, t!("Location"), crate::sys::wrap_path(&dir.to_string_lossy())));
        let mut flags = Vec::new();
        if e.is_hidden() {
            flags.push(t!("Hidden"));
        }
        if e.is_system() {
            flags.push(t!("System"));
        }
        if e.attrs & rf_core::attrs::READONLY != 0 {
            flags.push(t!("Read-only"));
        }
        if e.is_cloud_placeholder() {
            flags.push(t!("Online-only"));
        }
        if e.is_symlink() {
            flags.push(t!("Link"));
        }
        if !flags.is_empty() {
            props = props.child(prop_row(&p, t!("Attributes"), flags.join(", ")));
        }
        let hash_section: Option<AnyElement> = if e.is_dir() {
            None
        } else if let Some(h) = self.hash.clone() {
            Some(div().w_full().child(h).into_any_element())
        } else {
            let path2 = path.clone();
            Some(
                div()
                    .id("hash-btn")
                    .px_3()
                    .py_1p5()
                    .rounded(theme::RADIUS_SM)
                    .bg(p.hover)
                    .hover(|s| s.bg(p.pressed))
                    .cursor_pointer()
                    .text_sm()
                    .child(t!("Calculate hash (MD5, SHA…)"))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        let pth = path2.clone();
                        this.hash = Some(cx.new(|cx| crate::hash_panel::HashPanel::new(pth, cx)));
                        cx.notify();
                    }))
                    .into_any_element(),
            )
        };
        col.child(div().w_full().h(px(1.)).bg(p.divider)).child(props).children(hash_section).into_any_element()
    }
}

/// 1234567 → "1.234.567"
fn group(n: u64) -> String {
    let sep = if crate::i18n::is_tr() { '.' } else { ',' };
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push(sep);
        }
        out.push(c);
    }
    out
}
