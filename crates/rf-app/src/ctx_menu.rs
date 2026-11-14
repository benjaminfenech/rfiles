//! The file-list context menu (Files style): quick action buttons, our own commands, and the
//! shell's items loaded in the background into "Open with", "New" and "Show more options".
//! Shift+right-click still opens the classic Explorer menu.

use std::path::PathBuf;
use std::rc::Rc;

use futures::channel::oneshot;
use gpui_kit::assets::IconName as L;
use gpui_kit::*;
use rf_core::{Location, SortDir, SortKey};
use rf_linux::ctxmenu::{MenuOutcome, ShellItem, ShellTarget};
use rf_linux::fileops::Transfer;

use crate::fluent_menu::{self, Item, Menu, MenuIcon, Pending, Quick, Sub};
use crate::folder_view::{FolderEvent, FolderView, GroupBy, ViewMode};
use crate::t;

/// Shell verbs we show as our own commands (or that make no sense here), hidden from
/// "Show more options" — the same idea as Files' filter list.
const HIDDEN_VERBS: &[&str] = &[
    "open",
    "opennewwindow",
    "opennewtab",
    "opennewprocess",
    "explore",
    "cut",
    "copy",
    "paste",
    "pastelink",
    "delete",
    "rename",
    "properties",
    "link",
    "copyaspath",
    "pintohome",
    "pintohomefile",
    "windows.modernshare",
    "windows.share",
    "openas",
    "newfolder",
    // Windows Terminal's "Open in Terminal" (we have our own)
    "{9f156763-7844-4dc4-b2b1-901f640f5155}",
];

type Me = WeakEntity<FolderView>;

/// A menu action that runs on the folder view.
fn act(me: &Me, f: impl Fn(&mut FolderView, &mut Window, &mut Context<FolderView>) + 'static) -> impl Fn(&mut Window, &mut App) + 'static {
    let me = me.clone();
    move |w, cx| {
        let _ = me.update(cx, |v, cx| f(v, w, cx));
    }
}

fn icon(l: L) -> MenuIcon {
    MenuIcon::Lucide(l)
}

fn cursor() -> (i32, i32) {
    crate::sys::cursor_pos()
}

impl FolderView {
    /// Opens our context menu for the selection (or the folder background) at the mouse.
    pub fn show_context_menu(&mut self, background: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(dir) = self.current_dir() else { return };
        let items = if background { Vec::new() } else { self.selection_paths() };
        let background = background || items.is_empty();
        let me = cx.entity().downgrade();
        let menu = if background { self.background_menu(&me, &dir, cx) } else { self.items_menu(&me, &dir, &items, cx) };
        let menu_view = fluent_menu::show(menu, window.mouse_position(), window, cx);

        // Shell items in the background: QueryContextMenu can take a second with slow extensions.
        let target = if background { ShellTarget::Background(dir.clone()) } else { ShellTarget::Items(items.clone()) };
        let (tx, rx) = oneshot::channel();
        rf_linux::sta::run(move |owner| {
            let _ = tx.send(rf_linux::ctxmenu::build(owner, &target, false).map_err(|e| format!("{e:#}")));
        });
        let menu_weak = menu_view.downgrade();
        cx.spawn(async move |_, cx| {
            let result = rx.await.unwrap_or_else(|_| Err("cancelled".into()));
            let _ = cx.update(|cx| {
                let (more, open_with, new) = match result {
                    Ok((session, shell)) => split_shell_items(&me, session, shell, background, cx),
                    Err(e) => {
                        log::warn!("shell menu: {e}");
                        (Pending::Failed, Pending::Failed, Pending::Failed)
                    }
                };
                let inline = !cx.global::<crate::settings::Settings>().menu.shell_submenu;
                if let Some(m) = menu_weak.upgrade() {
                    m.update(cx, |m, cx| {
                        m.set_pending("more", more, cx);
                        m.set_pending("openwith", open_with, cx);
                        m.set_pending("new", new, cx);
                        // Settings: shell extensions listed in the menu itself, not in a submenu
                        if inline {
                            m.inline_pending("more", cx);
                        }
                    });
                }
            });
        })
        .detach();
    }

    fn quick_clipboard(&self, me: &Me, with_items: bool) -> Vec<Quick> {
        let paste_ok = rf_linux::clipboard::has_files();
        if !with_items {
            return vec![
                Quick::new(L::ClipboardPaste, t!("Paste  (Ctrl+V)"), paste_ok, act(me, |v, w, cx| v.paste(w, cx))),
                Quick::new(L::Wrench, t!("Properties  (Alt+Enter)"), true, act(me, |v, _, cx| v.folder_properties(cx))),
            ];
        }
        vec![
            Quick::new(L::Scissors, t!("Cut  (Ctrl+X)"), true, act(me, |v, w, cx| v.copy_to_clipboard(Transfer::Move, w, cx))),
            Quick::new(L::Copy, t!("Copy  (Ctrl+C)"), true, act(me, |v, w, cx| v.copy_to_clipboard(Transfer::Copy, w, cx))),
            Quick::new(
                L::ClipboardPaste,
                t!("Paste  (Ctrl+V)"),
                paste_ok,
                act(me, |v, w, cx| if v.selected_is_single_dir() { v.paste_into_selected(w, cx) } else { v.paste(w, cx) }),
            ),
            Quick::new(L::TextCursorInput, t!("Rename  (F2)"), true, act(me, |v, w, cx| v.rename_selection(w, cx))),
            Quick::new(L::Trash, t!("Delete  (Del)"), true, act(me, |v, w, cx| v.delete_selected(false, w, cx))),
            Quick::new(L::Wrench, t!("Properties  (Alt+Enter)"), true, act(me, |v, _, cx| v.properties(cx))),
        ]
    }

    fn items_menu(&self, me: &Me, dir: &std::path::Path, items: &[PathBuf], cx: &mut Context<Self>) -> Menu {
        let single_dir = items.len() == 1 && self.selected_is_single_dir();
        let has_files = !self.selected_is_single_dir() || items.len() > 1;
        let target_dir = if single_dir { items[0].clone() } else { dir.to_path_buf() };
        let prefs = cx.global::<crate::settings::Settings>();
        let (mp, show_flatten) = (prefs.menu.clone(), prefs.show_flatten);
        let mut v: Vec<Item> = Vec::new();

        v.push(Item::action(t!("Open"), icon(L::SquareArrowOutUpRight), act(me, |v, _, cx| v.open_selected(cx))).hint("Enter"));
        if single_dir {
            let (a, b) = (target_dir.clone(), target_dir.clone());
            if mp.open_in_new_tab {
                v.push(Item::action(t!("Open in new tab"), icon(L::SquarePlus), act(me, move |_, _, cx| {
                    cx.emit(FolderEvent::OpenInNewTab(Location::Path(a.clone())))
                })));
            }
            if mp.open_in_new_window {
                v.push(Item::action(t!("Open in new window"), icon(L::AppWindow), act(me, move |_, _, cx| {
                    cx.emit(FolderEvent::OpenInNewWindow(Location::Path(b.clone())))
                })));
            }
            if self.dual {
                let c = target_dir.clone();
                v.push(Item::action(t!("Open in other pane"), icon(L::Columns2), act(me, move |_, _, cx| {
                    cx.emit(FolderEvent::OpenInOtherPane(Location::Path(c.clone())))
                })));
            }
        }
        if has_files {
            v.push(Item::Sub { label: t!("Open with").into(), icon: icon(L::LayoutGrid), items: Sub::Pending { key: "openwith", fallback: None } });
        }
        if self.is_search() && items.len() == 1 {
            let p = items[0].clone();
            v.push(Item::action(t!("Open file location"), icon(L::FolderOpen), act(me, move |v, _, cx| {
                if let (Some(parent), Some(name)) = (p.parent(), p.file_name()) {
                    v.set_reselect(name.to_string_lossy().into_owned());
                    v.navigate(Location::Path(parent.to_path_buf()), cx);
                }
            })));
        }
        v.push(Item::Separator);
        if mp.copy_path {
            v.push(Item::action(t!("Copy item path"), icon(L::ClipboardCopy), act(me, |v, _, cx| v.copy_path(cx))).hint("Ctrl+Shift+C"));
        }
        if !self.is_search() {
            let sel = items.to_vec();
            v.push(Item::action(t!("New folder with selection"), icon(L::FolderInput), act(me, move |v, _, cx| v.folder_with_selection(sel.clone(), cx))));
        }
        if single_dir && crate::archive_view::split(&target_dir).is_none() {
            let d = target_dir.clone();
            v.push(Item::action(t!("Show disk usage"), icon(L::ChartPie), act(me, move |_, _, cx| {
                cx.emit(FolderEvent::OpenInNewTab(Location::DiskUsage(d.clone())))
            })));
        }
        if single_dir && mp.pin {
            let d = target_dir.clone();
            v.push(Item::action(t!("Pin to sidebar"), icon(L::Pin), act(me, move |v, _, cx| v.pin(d.clone(), cx))));
        }
        if single_dir && crate::archive_view::split(&target_dir).is_none() {
            let d = target_dir.clone();
            let label = match crate::aliases::name_of(&d) {
                Some(n) => format!("{} (@{n})…", t!("Edit alias")),
                None => format!("{}…", t!("Add alias")),
            };
            v.push(Item::action(label, icon(L::AtSign), move |w, cx| crate::aliases::edit_dialog(d.clone(), w, cx)));
        }
        // leave a folder (a build output, a huge cache) out of the search index (Linux: paths
        // are case-sensitive)
        if single_dir && cx.global::<crate::settings::Settings>().name_index && crate::archive_view::split(&target_dir).is_none() {
            let key = target_dir.to_string_lossy().trim_end_matches(std::path::MAIN_SEPARATOR).to_string();
            let key = if key.is_empty() { std::path::MAIN_SEPARATOR.to_string() } else { key };
            let same = |e: &String, k: &str| e == k;
            let out = cx.global::<crate::settings::Settings>().index_excluded.iter().any(|e| same(e, &key));
            v.push(
                Item::action(t!("Leave out of the search index"), icon(L::SearchSlash), move |_, cx| {
                    let key = key.clone();
                    crate::settings::update(cx, |s| {
                        if out {
                            s.index_excluded.retain(|e| !same(e, &key));
                        } else {
                            s.index_excluded.push(key);
                        }
                    });
                })
                .checked(out),
            );
        }
        if single_dir && show_flatten {
            let d = target_dir.clone();
            v.push(Item::action(t!("Flatten folder"), icon(L::FolderTree), act(me, move |v, w, cx| v.flatten_folder(d.clone(), w, cx))));
        }
        // type-specific actions (Files: rotate, set as background, install font)
        if !items.is_empty() && items.iter().all(|p| rf_linux::rotate::can_rotate(p) && crate::archive_view::split(p).is_none()) {
            v.push(Item::action(t!("Rotate right"), icon(L::RotateCw), act(me, |v, w, cx| v.rotate_selection(true, w, cx))));
            v.push(Item::action(t!("Rotate left"), icon(L::RotateCcw), act(me, |v, w, cx| v.rotate_selection(false, w, cx))));
        }
        // only where the desktop's portal can set wallpapers (KDE Plasma, GNOME)
        if items.len() == 1 && rf_linux::fileactions::is_image(&items[0]) && crate::archive_view::split(&items[0]).is_none() && rf_linux::fileactions::can_set_wallpaper() {
            let p = items[0].clone();
            v.push(Item::action(t!("Set as desktop background"), icon(L::Monitor), move |_, cx| {
                let p = p.clone();
                crate::status_center::start_with(cx, t!("Set as desktop background").to_string(), move |_| {
                    rf_linux::fileactions::set_wallpaper(&p)?;
                    Ok(true)
                }, None);
            }));
        }
        if !items.is_empty() && items.iter().all(|p| rf_linux::fileactions::is_font(p) && crate::archive_view::split(p).is_none()) {
            let fonts = items.to_vec();
            v.push(Item::action(t!("Install font"), icon(L::Type), move |w, cx| {
                let fonts = fonts.clone();
                let title = format!("{} {}", t!("Installing font:"), crate::i18n::items(fonts.len()));
                crate::status_center::start_with(cx, title, move |_| {
                    for f in &fonts {
                        rf_linux::fileactions::install_font_for_user(f)?;
                    }
                    Ok(true)
                }, None);
                let _ = w;
            }));
        }
        v.push(Item::Separator);

        // archives
        let zip_items = items.to_vec();
        let z7_items = items.to_vec();
        let dlg_items = items.to_vec();
        if mp.compress {
            v.push(Item::sub(
                t!("Compress"),
                icon(L::Archive),
                vec![
                    Item::action(t!("Create archive…"), icon(L::Archive), move |w, cx| create_archive_dialog(dlg_items.clone(), w, cx)),
                    Item::Separator,
                    Item::action(t!("Compress to ZIP"), icon(L::FileArchive), move |_, cx| compress(cx, zip_items.clone(), rf_archive::Format::Zip)),
                    Item::action(t!("Compress to 7z"), icon(L::FileArchive), move |_, cx| compress(cx, z7_items.clone(), rf_archive::Format::SevenZ)),
                ]
                .into_iter()
                .chain(crate::archive_view::winrar().is_some().then(|| {
                    let rar_items = items.to_vec();
                    Item::action(t!("Compress to RAR"), icon(L::FileArchive), move |_, cx| {
                        if let Some((dir, stem)) = archive_stem(&rar_items) {
                            let dest = rf_linux::fileops::unique_name(&dir, &stem, ".rar");
                            crate::status_center::compress_rar(cx, rar_items.clone(), dest, 5, None);
                        }
                    })
                }))
                .collect(),
            ));
        }
        if mp.compress && items.len() == 1 && rf_archive::is_supported(&items[0]) {
            let (a, b) = (items[0].clone(), items[0].clone());
            v.push(Item::sub(
                t!("Extract"),
                icon(L::FolderOutput),
                vec![
                    Item::action(t!("Extract here"), icon(L::FolderDown), move |_, cx| {
                        let dest = a.parent().map(|p| p.to_path_buf()).unwrap_or_default();
                        crate::status_center::extract(cx, a.clone(), Some(dest));
                    }),
                    Item::action(format!("{} {}\\", t!("Extract to folder:"), crate::ops::archive_stem(&b)), icon(L::FolderInput), move |_, cx| {
                        crate::status_center::extract(cx, b.clone(), None)
                    }),
                ],
            ));
        }
        if mp.tags {
            v.push(Item::sub(t!("Tags"), icon(L::Tag), tag_items(me, items, cx)));
        }
        if mp.shelf {
            let shelf_items = items.to_vec();
            v.push(Item::action(t!("Add to shelf"), icon(L::Inbox), act(me, move |_, _, cx| cx.emit(FolderEvent::AddToShelf(shelf_items.clone())))).hint("Ctrl+Shift+S"));
        }
        if items.len() > 1 && !self.is_search() {
            let b = items.to_vec();
            v.push(Item::action(t!("Bulk rename…"), icon(L::PencilLine), act(me, move |_, _, cx| cx.emit(FolderEvent::BulkRename(b.clone())))).hint("F2"));
        }
        if self.dual {
            let (c, m) = (items.to_vec(), items.to_vec());
            v.push(Item::action(t!("Copy to other pane"), icon(L::CopyPlus), act(me, move |_, _, cx| cx.emit(FolderEvent::ToOtherPane(Transfer::Copy, c.clone())))));
            v.push(Item::action(t!("Move to other pane"), icon(L::MoveRight), act(me, move |_, _, cx| cx.emit(FolderEvent::ToOtherPane(Transfer::Move, m.clone())))));
        }
        if single_dir && mp.terminal {
            v.push(Item::Separator);
            let d = target_dir.clone();
            v.push(Item::action(t!("Open in terminal"), icon(L::SquareTerminal), move |w, cx| open_terminal(d.clone(), w, cx)).hint("Ctrl+Shift+`"));
        }
        Menu { quick: self.quick_clipboard(me, true), items: v }
    }

    fn background_menu(&self, me: &Me, dir: &std::path::Path, _cx: &mut Context<Self>) -> Menu {
        let mode = self.view_mode;
        let sort = self.sort;
        let group = self.group;
        let mut v: Vec<Item> = Vec::new();
        v.push(Item::sub(t!("Layout"), icon(L::LayoutGrid), layout_items(me, mode, self.grid_style)));
        v.push(Item::sub(t!("Sort"), icon(L::ArrowUpDown), sort_items(me, sort)));
        v.push(Item::sub(t!("Group by"), icon(L::ListTree), group_items(me, group)));
        v.push(Item::action(t!("Refresh"), icon(L::RotateCw), act(me, |v, _, cx| v.refresh(cx))).hint("Ctrl+R"));
        v.push(Item::Separator);
        let fallback = Rc::new(own_new_items(me));
        v.push(Item::Sub { label: t!("New").into(), icon: icon(L::CirclePlus), items: Sub::Pending { key: "new", fallback: Some(fallback) } });
        let d = dir.to_path_buf();
        v.push(Item::action(t!("Pin to sidebar"), icon(L::Pin), act(me, move |v, _, cx| v.pin(d.clone(), cx))));
        v.push(Item::action(t!("Copy path"), icon(L::ClipboardCopy), act(me, |v, _, cx| v.copy_path(cx))));
        if !self.is_search() && !crate::archive_view::is_browsable_path(dir) {
            let d = dir.to_path_buf();
            v.push(Item::action(t!("Show disk usage"), icon(L::ChartPie), act(me, move |_, _, cx| {
                cx.emit(FolderEvent::OpenInNewTab(Location::DiskUsage(d.clone())))
            })));
        }
        let show_hidden = self.filter.show_hidden;
        v.push(Item::action(t!("Show hidden items"), icon(L::Eye), act(me, |v, _, cx| v.toggle_hidden(cx))).checked(show_hidden).hint("Ctrl+H"));
        v.push(Item::Separator);
        let d = dir.to_path_buf();
        v.push(Item::action(t!("Open in terminal"), icon(L::SquareTerminal), move |w, cx| open_terminal(d.clone(), w, cx)).hint("Ctrl+Shift+`"));
        v.push(Item::action(t!("Run command here…"), icon(L::Terminal), |w, cx| w.dispatch_action(Box::new(crate::actions::RunCommand), cx)));
        // Git (Files: initialize / clone from the folder background)
        if !self.is_search() && !crate::archive_view::is_browsable_path(dir) {
            let mut git_items = Vec::new();
            if self.git.is_none() {
                let (m, d) = (me.clone(), dir.to_path_buf());
                git_items.push(Item::action(t!("Initialize Git repository"), icon(L::GitBranchPlus), move |_, cx| crate::git_ui::init_repo(m.clone(), d.clone(), cx)));
            }
            let (m, d) = (me.clone(), dir.to_path_buf());
            git_items.push(Item::action(t!("Clone repository…"), icon(L::GitFork), move |w, cx| crate::git_ui::clone_dialog(m.clone(), d.clone(), w, cx)));
            v.push(Item::sub("Git", icon(L::GitBranch), git_items));
        }
        Menu { quick: self.quick_clipboard(me, false), items: v }
    }

    /// F2 from the menu: bulk rename for several items, inline for one.
    pub(crate) fn rename_selection(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.selection_paths().len() > 1 {
            cx.emit(FolderEvent::BulkRename(self.selection_paths()));
        } else {
            self.begin_rename(window, cx);
        }
    }

    /// Properties of the current folder (background menu).
    pub(crate) fn folder_properties(&self, cx: &mut App) {
        let Some(dir) = self.current_dir() else { return };
        crate::properties_view::open(vec![dir], self.images.clone(), cx);
    }
}

fn layout_items(me: &Me, mode: ViewMode, style: crate::folder_view::GridStyle) -> Vec<Item> {
    use crate::folder_view::GridStyle;
    let layout = |label: &str, l: L, m: ViewMode, st: GridStyle, hint: &str| {
        let on = mode == m && (m != ViewMode::Grid || style == st);
        Item::action(label.to_string(), icon(l), act(me, move |v, _, cx| v.set_layout(m, st, cx))).checked(on).hint(hint.to_string())
    };
    vec![
        layout(t!("Details"), L::List, ViewMode::Details, GridStyle::Tiles, "Ctrl+Shift+1"),
        layout(t!("List"), L::TextAlignJustify, ViewMode::Grid, GridStyle::List, "Ctrl+Shift+5"),
        layout(t!("Cards"), L::LayoutList, ViewMode::Grid, GridStyle::Cards, "Ctrl+Shift+4"),
        layout(t!("Grid"), L::LayoutGrid, ViewMode::Grid, GridStyle::Tiles, "Ctrl+Shift+2"),
        layout(t!("Columns"), L::Columns3, ViewMode::Columns, GridStyle::Tiles, "Ctrl+Shift+3"),
    ]
}

fn sort_items(me: &Me, sort: rf_core::SortSpec) -> Vec<Item> {
    let by = |label: &str, k: SortKey| {
        Item::action(label.to_string(), MenuIcon::None, act(me, move |v, _, cx| {
            let mut s = v.sort;
            s.key = k;
            v.set_sort_spec(s, cx)
        }))
        .checked(sort.key == k)
    };
    let dir_item = |label: &str, d: SortDir| {
        Item::action(label.to_string(), MenuIcon::None, act(me, move |v, _, cx| {
            let mut s = v.sort;
            s.dir = d;
            v.set_sort_spec(s, cx)
        }))
        .checked(sort.dir == d)
    };
    vec![
        by(t!("Name"), SortKey::Name),
        by(t!("Date modified"), SortKey::Modified),
        by(t!("Date created"), SortKey::Created),
        by(t!("Type"), SortKey::Type),
        by(t!("Size"), SortKey::Size),
        Item::Separator,
        dir_item(t!("Ascending"), SortDir::Asc),
        dir_item(t!("Descending"), SortDir::Desc),
    ]
}

/// Folders first / files first / mixed (Files' three radio items under Sort and Group).
fn order_items(me: &Me, sort: rf_core::SortSpec) -> Vec<Item> {
    let order = |label: &str, dirs: bool, files: bool| {
        Item::action(label.to_string(), MenuIcon::None, act(me, move |v, _, cx| {
            let mut s = v.sort;
            s.dirs_first = dirs;
            s.files_first = files;
            v.set_sort_spec(s, cx)
        }))
        .checked(sort.dirs_first == dirs && (dirs || sort.files_first == files))
    };
    vec![
        order(t!("Sort folders first"), true, false),
        order(t!("Sort files first"), false, true),
        order(t!("Sort files and folders together"), false, false),
    ]
}

fn group_items(me: &Me, group: GroupBy) -> Vec<Item> {
    let grp = |label: &str, g: GroupBy| Item::action(label.to_string(), MenuIcon::None, act(me, move |v, _, cx| v.set_group(g, cx))).checked(group == g);
    vec![
        grp(t!("None"), GroupBy::None),
        grp(t!("Name"), GroupBy::Name),
        grp(t!("Type"), GroupBy::Type),
        grp(t!("Date modified"), GroupBy::Modified),
        grp(t!("Size"), GroupBy::Size),
    ]
}

/// Drop-down menus of the command bar.
pub enum BarMenu {
    New,
    Sort,
    /// Layout + view toggles; the flags are workspace state shown as check marks.
    View { dual: bool, shelf: bool },
}

impl FolderView {
    pub fn show_bar_menu(&mut self, which: BarMenu, pos: Point<Pixels>, window: &mut Window, cx: &mut Context<Self>) {
        let me = cx.entity().downgrade();
        let items = match which {
            BarMenu::New => {
                let Some(dir) = self.current_dir() else { return };
                let view = fluent_menu::show(Menu { quick: Vec::new(), items: own_new_items(&me) }, pos, window, cx);
                // the shell's templates (Word, Excel…) join as soon as the shell answers
                let (tx, rx) = oneshot::channel();
                rf_linux::sta::run(move |owner| {
                    let _ = tx.send(rf_linux::ctxmenu::build(owner, &ShellTarget::Background(dir), false).ok());
                });
                let weak = view.downgrade();
                cx.spawn(async move |_, cx| {
                    let Ok(Some((session, shell))) = rx.await else { return };
                    let _ = cx.update(|cx| {
                        if let (_, _, Pending::Ready(items)) = split_shell_items(&me, session, shell, true, cx) {
                            if let Some(m) = weak.upgrade() {
                                m.update(cx, |m, cx| m.set_root((*items).clone(), cx));
                            }
                        }
                    });
                })
                .detach();
                return;
            }
            BarMenu::Sort => {
                // Files' layout: Sort ▸ / Group ▸, then the folder/file order; these apply to
                // this folder only (each folder remembers its own) unless "sync" is on
                let mut v = vec![
                    Item::sub(t!("Sort"), icon(L::ArrowUpDown), sort_items(&me, self.sort)),
                    Item::sub(t!("Group by"), icon(L::ListTree), group_items(&me, self.group)),
                    Item::Separator,
                ];
                v.extend(order_items(&me, self.sort));
                if !cx.global::<crate::settings::Settings>().sync_layout && matches!(self.location(), Location::Path(_)) {
                    v.push(Item::Separator);
                    v.push(Item::action(t!("Make default for all folders"), icon(L::CopyCheck), act(&me, |v, _, cx| v.make_layout_default(cx))));
                }
                v
            }
            BarMenu::View { dual, shelf } => {
                // Files' layout flyout (tiles, size slider, switches)
                // opened after this update: its first frame reads this view
                window.defer(cx, move |window, cx| crate::layout_panel::open(me, dual, shelf, pos, window, cx));
                return;
            }
        };
        fluent_menu::show(Menu { quick: Vec::new(), items }, pos, window, cx);
    }
}

fn own_new_items(me: &Me) -> Vec<Item> {
    vec![
        Item::action(t!("Folder"), MenuIcon::Tinted(L::Folder, crate::theme::kind_color(crate::icons::Category::Folder, true)), act(me, |v, _, cx| v.create_new(true, cx)))
            .hint("Ctrl+Shift+N"),
        Item::action(t!("Text document"), icon(L::FileText), act(me, |v, _, cx| v.create_new(false, cx))),
    ]
}

fn tag_items(me: &Me, items: &[PathBuf], cx: &App) -> Vec<Item> {
    let store = &cx.global::<crate::tags::Tags>().0;
    store
        .defs
        .iter()
        .map(|d| {
            let all = items.iter().all(|p| store.has(p, d.id));
            let (paths, id) = (items.to_vec(), d.id);
            let color = crate::tags::color(d.color);
            Item::action(crate::i18n::td(&d.name), MenuIcon::Dot(color), act(me, move |_, _, cx| {
                crate::tags::update(cx, |t| t.toggle(&paths, id));
                cx.notify();
            }))
            .checked(all)
        })
        .collect()
}

/// Default archive name for `items`: the item's name (one) or the folder's name (several).
fn archive_stem(items: &[PathBuf]) -> Option<(PathBuf, String)> {
    let first = items.first()?;
    let dir = first.parent().map(|p| p.to_path_buf()).unwrap_or_default();
    let stem = if items.len() == 1 {
        if first.is_dir() {
            first.file_name().map(|s| s.to_string_lossy().into_owned())
        } else {
            first.file_stem().map(|s| s.to_string_lossy().into_owned())
        }
        .unwrap_or_else(|| "Archive".into())
    } else {
        rf_core::history::display_name(&dir)
    };
    Some((dir, stem))
}

fn compress(cx: &mut App, items: Vec<PathBuf>, format: rf_archive::Format) {
    let Some((dir, stem)) = archive_stem(&items) else { return };
    let dest = dir.join(format!("{stem}.{}", format.extension()));
    crate::status_center::compress(cx, items, dest, format);
}

/// Files' "Create archive" dialog: name, format and compression level.
fn create_archive_dialog(items: Vec<PathBuf>, window: &mut Window, cx: &mut App) {
    use crate::prompt::{Button, Choice, Kind};
    let Some((dir, stem)) = archive_stem(&items) else { return };
    let count = items.len();
    let format = std::rc::Rc::new(std::cell::Cell::new(0usize));
    let level = std::rc::Rc::new(std::cell::Cell::new(2usize));
    let protect = std::rc::Rc::new(std::cell::Cell::new(0usize));
    let split = std::rc::Rc::new(std::cell::Cell::new(0usize));
    let dict = std::rc::Rc::new(std::cell::Cell::new(0usize));
    // (None = RAR, WinRAR's format: offered only when its rar is installed)
    let mut formats = vec![
        ("ZIP", Some(rf_archive::Format::Zip)),
        ("7z", Some(rf_archive::Format::SevenZ)),
        ("tar.zst", Some(rf_archive::Format::TarZst)),
        ("tar.gz", Some(rf_archive::Format::TarGz)),
        ("tar", Some(rf_archive::Format::Tar)),
    ];
    if crate::archive_view::winrar().is_some() {
        formats.push(("RAR", None));
    }
    let choices = vec![
        Choice { label: t!("Format").into(), options: formats.iter().map(|(n, _)| SharedString::from(*n)).collect(), selected: format.clone() },
        Choice {
            label: t!("Compression level").into(),
            options: vec![t!("None").into(), t!("Fast").into(), t!("Normal").into(), t!("Maximum").into()],
            selected: level.clone(),
        },
        // ZIP: AES-256; 7z: AES-256 with encrypted names
        Choice { label: t!("Password").into(), options: vec![t!("None").into(), t!("Set password…").into()], selected: protect.clone() },
        // (parts as 7-Zip names them: name.zip.001, .002, ...)
        Choice {
            label: t!("Split into parts").into(),
            options: std::iter::once(SharedString::from(t!("None"))).chain(SPLIT_SIZES.iter().map(|(n, _)| SharedString::from(*n))).collect(),
            selected: split.clone(),
        },
        Choice {
            label: t!("Dictionary size (7z)").into(),
            options: std::iter::once(SharedString::from(t!("Automatic"))).chain(DICT_SIZES.iter().map(|(n, _)| SharedString::from(*n))).collect(),
            selected: dict.clone(),
        },
    ];
    let create = Button::new(t!("Create"), Kind::Primary, move |name, window, cx| {
        let name = name.unwrap_or_default();
        let name = name.trim();
        if name.is_empty() {
            return Err(t!("Enter a name").into());
        }
        if crate::ops::invalid_file_name(name) {
            return Err("A file name can't contain \"/\".".into());
        }
        let lvl = [0, 1, 5, 9][level.get().min(3)];
        let fmt = formats.get(format.get()).and_then(|f| f.1);
        let volume_size = split.get().checked_sub(1).and_then(|i| SPLIT_SIZES.get(i)).map(|s| s.1);
        let dict_size = dict.get().checked_sub(1).and_then(|i| DICT_SIZES.get(i)).map(|s| s.1);
        let Some(fmt) = fmt else {
            if volume_size.is_some() {
                return Err(t!("RAR can't be split here; choose ZIP or 7z").into());
            }
            let dest = dir.join(format!("{name}.rar"));
            let items = items.clone();
            if protect.get() == 1 {
                let finish: Finish = Rc::new(move |cx, pw| crate::status_center::compress_rar(cx, items.clone(), dest.clone(), lvl, Some(pw)));
                let d = dir.join(format!("{name}.rar"));
                window.defer(cx, move |window, cx| archive_password(d, finish, None, window, cx));
            } else {
                crate::status_center::compress_rar(cx, items, dest, lvl, None);
            }
            return Ok(());
        };
        let dest = dir.join(format!("{name}.{}", fmt.extension()));
        let opts = rf_archive::CompressOptions { password: None, dict_size, volume_size };
        if protect.get() == 1 {
            if !fmt.encrypts() {
                return Err(t!("tar formats don't support passwords; choose ZIP or 7z").into());
            }
            // asked after this dialog closes (one dialog at a time)
            let items = items.clone();
            let d = dest.clone();
            let finish: Finish = Rc::new(move |cx, pw| {
                let opts = rf_archive::CompressOptions { password: Some(pw), ..opts.clone() };
                crate::status_center::compress_opts(cx, items.clone(), dest.clone(), fmt, lvl, opts)
            });
            window.defer(cx, move |window, cx| archive_password(d, finish, None, window, cx));
            return Ok(());
        }
        crate::status_center::compress_opts(cx, items.clone(), dest, fmt, lvl, opts);
        Ok(())
    });
    crate::prompt::show_with(
        t!("Create archive"),
        Some(format!("{} — {}", crate::i18n::items(count),t!("the extension follows the format"))),
        Some((stem, t!("Archive name").to_string())),
        choices,
        vec![create, Button::cancel()],
        window,
        cx,
    );
}

/// "Split into parts" sizes: e-mail attachments, a CD, FAT32's file limit (4 GB less 1 MB).
const SPLIT_SIZES: &[(&str, u64)] = &[("25 MB", 25 << 20), ("100 MB", 100 << 20), ("700 MB", 700 << 20), ("1 GB", 1 << 30), ("4 GB", (4 << 30) - (1 << 20))];
const DICT_SIZES: &[(&str, u64)] = &[("1 MB", 1 << 20), ("16 MB", 16 << 20), ("64 MB", 64 << 20), ("256 MB", 256 << 20)];

type Finish = Rc<dyn Fn(&mut App, String)>;

/// "Create archive" with a password: typed twice (`first` = the first entry), then `finish`.
fn archive_password(dest: PathBuf, finish: Finish, first: Option<String>, window: &mut Window, cx: &mut App) {
    use crate::prompt::{Button, Kind};
    let again = first.is_some();
    let title = if again { t!("Enter the password again") } else { t!("Archive password") };
    let msg = format!("{} — {}", rf_core::history::display_name(&dest), t!("If you forget the password, the archive can't be opened."));
    let label = if again { t!("Create") } else { t!("Next") };
    crate::prompt::show_secret(
        title,
        Some(msg),
        t!("Password").to_string(),
        vec![
            Button::new(label, Kind::Primary, move |text, window, cx| {
                let pw = text.unwrap_or_default();
                if pw.is_empty() {
                    return Err(t!("The password can't be empty").into());
                }
                match &first {
                    None => {
                        let (d, f) = (dest.clone(), finish.clone());
                        window.defer(cx, move |window, cx| archive_password(d, f, Some(pw), window, cx));
                    }
                    Some(p) if *p == pw => finish(cx, pw),
                    Some(_) => return Err(t!("The passwords don't match").into()),
                }
                Ok(())
            }),
            Button::cancel(),
        ],
        window,
        cx,
    );
}

fn open_terminal(d: PathBuf, w: &mut Window, cx: &mut App) {
    crate::terminal::open(d, None, Some(w.window_handle()), cx);
}

/// Turns the shell menu into our three lazy lists: "Show more options" (what is left after
/// hiding our own commands), "Open with" and "New".
fn split_shell_items(me: &Me, session: u64, shell: Vec<ShellItem>, background: bool, cx: &mut App) -> (Pending, Pending, Pending) {
    let is_hidden = |it: &ShellItem| !it.verb.is_empty() && HIDDEN_VERBS.contains(&it.verb.to_ascii_lowercase().as_str());
    let has_child_verb = |it: &ShellItem, verb: &str| it.children.iter().any(|c| c.verb.eq_ignore_ascii_case(verb));

    let mut more_src = Vec::new();
    let mut open_with: Option<Vec<ShellItem>> = None;
    let mut new: Option<Vec<ShellItem>> = None;
    for it in shell {
        if !it.children.is_empty() && has_child_verb(&it, "openas") {
            open_with = Some(it.children.clone());
            continue;
        }
        if !it.children.is_empty() && has_child_verb(&it, "NewFolder") {
            new = Some(it.children.clone());
            continue;
        }
        if it.verb.eq_ignore_ascii_case("openas") && open_with.is_none() {
            // a single "Open with…" entry: say what it does
            open_with = Some(vec![ShellItem { label: t!("Choose another app").into(), ..it.clone() }]);
        }
        if is_hidden(&it) {
            continue;
        }
        more_src.push(it);
    }
    let more = to_items(me, session, &tidy(more_src), cx);

    let open_with = match open_with {
        Some(v) => Pending::Ready(Rc::new(to_items(me, session, &tidy(v), cx))),
        None => Pending::Ready(Rc::new(vec![Item::action(t!("Choose another app"), icon(L::LayoutGrid), act(me, |v, _, _| v.open_with()))])),
    };
    let new = match new.filter(|_| background) {
        Some(v) => {
            // ours for Folder and Text (inline rename), the shell for every other template
            let rest: Vec<ShellItem> = v
                .into_iter()
                .filter(|c| !c.verb.eq_ignore_ascii_case("NewFolder") && !c.verb.eq_ignore_ascii_case(".txt"))
                .collect();
            let mut items = own_new_items(me);
            let rest = to_items(me, session, &tidy(rest), cx);
            if !rest.is_empty() {
                items.push(Item::Separator);
                items.extend(rest);
            }
            Pending::Ready(Rc::new(items))
        }
        None => Pending::Failed,
    };
    (Pending::Ready(Rc::new(more)), open_with, new)
}

/// No leading/trailing/double separators.
fn tidy(items: Vec<ShellItem>) -> Vec<ShellItem> {
    let mut out: Vec<ShellItem> = Vec::with_capacity(items.len());
    for it in items {
        if it.separator && out.last().is_none_or(|l| l.separator) {
            continue;
        }
        out.push(it);
    }
    while out.last().is_some_and(|l| l.separator) {
        out.pop();
    }
    out
}

fn to_items(me: &Me, session: u64, items: &[ShellItem], cx: &mut App) -> Vec<Item> {
    items
        .iter()
        .map(|it| {
            if it.separator {
                return Item::Separator;
            }
            let icon = it.icon.as_ref().and_then(|i| fluent_menu::shell_icon(i, cx)).map(MenuIcon::Image).unwrap_or(MenuIcon::None);
            if !it.children.is_empty() {
                return Item::Sub { label: crate::i18n::td(&it.label).into(), icon, items: Sub::Items(Rc::new(to_items(me, session, &tidy(it.children.clone()), cx))) };
            }
            let (id, me) = (it.id, me.clone());
            // Windows hands labels over translated; Linux uses the app's source texts
            let mut item = Item::action(crate::i18n::td(&it.label), icon, move |_, cx| invoke_shell(&me, session, id, cx)).enabled(it.enabled);
            if let Some(h) = &it.hint {
                item = item.hint(h.clone());
            }
            item
        })
        .collect()
}

fn invoke_shell(me: &Me, session: u64, id: u32, cx: &mut App) {
    let (tx, rx) = oneshot::channel();
    rf_linux::sta::run(move |owner| {
        let r = rf_linux::ctxmenu::invoke(owner, session, id, cursor());
        if let Err(e) = &r {
            log::warn!("shell command: {e:#}");
        }
        let _ = tx.send(r.ok());
    });
    let me = me.clone();
    cx.spawn(async move |cx| {
        let outcome = rx.await;
        let _ = me.update(cx, |v, cx| {
            match outcome {
                Ok(Some(MenuOutcome::Rename)) => {
                    v.request_rename();
                    cx.notify();
                }
                _ => {}
            }
        });
    })
    .detach();
}

impl FolderView {
}
