//! File operations, clipboard and inline rename for a FolderView.
//! Everything that can block runs on worker threads or the shell STA thread.

use crate::t;
use std::path::PathBuf;

use futures::channel::oneshot;
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::*;
use rf_linux::fileops::{self, Transfer};

use crate::folder_view::FolderView;


/// "photos.tar.gz" → "photos" (pure string work; the unique folder is chosen at extract time).
pub(crate) fn archive_stem(p: &std::path::Path) -> String {
    let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let lower = name.to_lowercase();
    for suf in [".tar.gz", ".tar.zst", ".tar.xz", ".tar.bz2", ".tgz", ".tzst", ".txz", ".tbz2", ".tbz", ".zip", ".7z", ".rar", ".iso", ".tar", ".gz", ".xz", ".zst", ".bz2"] {
        if lower.ends_with(suf) && name.len() > suf.len() && name.is_char_boundary(name.len() - suf.len()) {
            return name[..name.len() - suf.len()].to_string();
        }
    }
    name
}

/// What "Flatten folder" did, for undo / redo: every file's (old, new) path and the folders it
/// removed.
#[derive(Debug, Clone, Default)]
pub struct Flattened {
    pub moves: Vec<(PathBuf, PathBuf)>,
    pub removed: Vec<PathBuf>,
}

/// Worker of "Flatten folder". `rec` gets every step that happened, also when it fails or is
/// cancelled half way, so even that can be undone.
fn flatten(dir: &std::path::Path, t: &fileops::Tracker, rec: &mut Flattened) -> anyhow::Result<bool> {
    use std::sync::atomic::Ordering;
    let mut files = Vec::new();
    let mut dirs = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d)?.flatten() {
            let p = e.path();
            let ft = e.file_type()?;
            if ft.is_dir() && !ft.is_symlink() {
                stack.push(p.clone());
                dirs.push(p);
            } else if d != dir {
                files.push(p);
            }
        }
    }
    let total = files.len() as u32;
    for (i, f) in files.iter().enumerate() {
        if t.cancel.load(Ordering::Relaxed) {
            return Ok(false);
        }
        let name = f.file_name().unwrap_or_default().to_string_lossy().into_owned();
        (t.on_progress)(total, i as u32, Some(name.clone()));
        let mut dest = dir.join(&name);
        let (stem, ext) = match name.rfind('.') {
            Some(i) if i > 0 => (name[..i].to_string(), name[i..].to_string()),
            _ => (name.clone(), String::new()),
        };
        let mut n = 2;
        while dest.exists() {
            dest = dir.join(format!("{stem} ({n}){ext}"));
            n += 1;
        }
        std::fs::rename(f, &dest)?;
        rec.moves.push((f.clone(), dest));
    }
    // deepest first: a folder is empty once its children are gone
    dirs.sort_by_key(|d| std::cmp::Reverse(d.components().count()));
    for d in dirs {
        if std::fs::remove_dir(&d).is_ok() {
            rec.removed.push(d);
        }
    }
    (t.on_progress)(total, total, None);
    Ok(true)
}

/// Undo of a flatten: the folders come back, every file goes back to where it was. A file
/// that is gone, or whose old place is taken by a new one, is left alone (and reported).
pub(crate) fn unflatten(rec: &Flattened) -> anyhow::Result<bool> {
    // shallowest first (create_dir_all would manage anyway; this keeps it obvious)
    let mut dirs = rec.removed.clone();
    dirs.sort_by_key(|d| d.components().count());
    for d in &dirs {
        std::fs::create_dir_all(d)?;
    }
    let mut failed = 0usize;
    for (old, new) in rec.moves.iter().rev() {
        if old.exists() || !new.exists() {
            failed += 1;
            continue;
        }
        if let Some(parent) = old.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if std::fs::rename(new, old).is_err() {
            failed += 1;
        }
    }
    anyhow::ensure!(failed == 0, "{failed} {}", t!("files could not be moved back"));
    Ok(true)
}

/// Redo of a flatten: the same moves again, then the same folders removed (if still empty).
pub(crate) fn reflatten(rec: &Flattened) -> anyhow::Result<bool> {
    let mut failed = 0usize;
    for (old, new) in &rec.moves {
        if new.exists() || !old.exists() || std::fs::rename(old, new).is_err() {
            failed += 1;
        }
    }
    let mut dirs = rec.removed.clone();
    dirs.sort_by_key(|d| std::cmp::Reverse(d.components().count()));
    for d in dirs {
        let _ = std::fs::remove_dir(&d);
    }
    anyhow::ensure!(failed == 0, "{failed} {}", t!("files could not be moved"));
    Ok(true)
}

fn bg(name: &'static str, f: impl FnOnce() -> anyhow::Result<bool> + Send + 'static) {
    std::thread::Builder::new()
        .name(format!("rf-op-{name}"))
        .spawn(move || {
            if let Err(e) = f() {
                log::warn!("{name}: {e:#}");
            }
        })
        .ok();
}

impl FolderView {
    pub fn current_dir(&self) -> Option<PathBuf> {
        self.location().path().map(|p| p.to_path_buf())
    }

    pub fn copy_to_clipboard(&mut self, kind: Transfer, _window: &mut Window, cx: &mut Context<Self>) {
        let paths = self.selection_paths();
        if paths.is_empty() {
            return;
        }
        let in_archive = self.current_dir().is_some_and(|d| crate::archive_view::is_browsable_path(&d));
        if in_archive {
            // items inside an archive: extract them (Status Center), then the copies go on
            // the clipboard; cut becomes copy
            crate::archive_view::copy_items(cx, paths);
            return;
        }
        // OpenClipboard can wait on other apps: never on the UI thread.
        rf_linux::sta::run(move |owner| {
            if let Err(e) = rf_linux::clipboard::set_files(owner, &paths, kind) {
                log::warn!("clipboard: {e:#}");
            }
        });
    }

    /// Ctrl+Shift+C: plain paths, one per line (Ctrl+Alt+C: with quotes, see `copy_path_with`).
    pub fn copy_path(&mut self, cx: &mut Context<Self>) {
        self.copy_path_with(false, cx)
    }

    pub fn copy_path_with(&mut self, quoted: bool, cx: &mut Context<Self>) {
        let paths = self.selection_paths();
        let q = |s: String| if quoted { format!("\"{s}\"") } else { s };
        let text = if paths.is_empty() {
            self.current_dir().map(|d| q(d.to_string_lossy().into_owned())).unwrap_or_default()
        } else {
            paths.iter().map(|p| q(p.display().to_string())).collect::<Vec<_>>().join("\r\n")
        };
        cx.write_to_clipboard(ClipboardItem::new_string(text));
    }

    /// Files' "Rotate left / right" on the selected images (JPEG: orientation tag only, others
    /// rotated losslessly through WIC). The list reloads; new thumbnails follow the new mtime.
    pub fn rotate_selection(&mut self, clockwise: bool, window: &mut Window, cx: &mut Context<Self>) {
        let paths: Vec<PathBuf> = self
            .selection_paths()
            .into_iter()
            .filter(|p| rf_linux::rotate::can_rotate(p) && crate::archive_view::split(p).is_none())
            .collect();
        if paths.is_empty() {
            return;
        }
        let task = cx.background_spawn(async move {
            let mut errors = Vec::new();
            for p in &paths {
                if let Err(e) = rf_linux::rotate::rotate(p, clockwise) {
                    errors.push(format!("{}: {e:#}", p.file_name().unwrap_or_default().to_string_lossy()));
                }
            }
            errors
        });
        cx.spawn_in(window, async move |this, cx| {
            let errors = task.await;
            let _ = this.update_in(cx, |v, window, cx| {
                v.soft_reload(cx);
                if !errors.is_empty() {
                    log::warn!("rotate: {}", errors.join("; "));
                    crate::prompt::show(
                        t!("Couldn't rotate"),
                        Some(errors.join("
")),
                        None,
                        vec![crate::prompt::Button::new(t!("OK"), crate::prompt::Kind::Primary, |_, _, _| Ok(()))],
                        window,
                        cx,
                    );
                }
            });
        })
        .detach();
    }

    pub fn paste(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(dir) = self.current_dir() else { return };
        self.paste_to(dir, window, cx);
    }

    /// Ctrl+Shift+V: paste into the selected folder (falls back to the current one).
    pub fn paste_into_selected(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let target = if self.selected_is_single_dir() { self.selection_paths().into_iter().next() } else { None };
        match target.or_else(|| self.current_dir()) {
            Some(dir) => self.paste_to(dir, window, cx),
            None => {}
        }
    }

    fn paste_to(&mut self, dir: PathBuf, _window: &mut Window, cx: &mut Context<Self>) {
        // GetClipboardData can block on a slow source app (delayed rendering).
        let (tx, rx) = oneshot::channel();
        rf_linux::sta::run(move |owner| {
            let _ = tx.send(rf_linux::clipboard::get_files(owner).map_err(|e| log::warn!("paste: {e:#}")).ok().flatten());
        });
        cx.spawn(async move |this, cx| {
            if let Ok(Some((paths, kind))) = rx.await {
                let _ = cx.update(|cx| {
                    let here = dir.clone();
                    crate::status_center::transfer_then(cx, kind, paths, dir, move |arrived, cx| {
                        // select what arrived, if this view still shows that folder
                        let names: Vec<String> = arrived.iter().filter_map(|p| p.file_name()).map(|n| n.to_string_lossy().into_owned()).collect();
                        let _ = this.update(cx, |v, cx| {
                            if v.current_dir().as_deref() == Some(here.as_path()) {
                                v.select_when_listed(names, cx);
                            }
                        });
                    });
                });
            }
        })
        .detach();
    }

    /// Deletes the selection, asking first per Settings → "Confirm before deleting".
    pub fn delete_selected(&mut self, permanent: bool, window: &mut Window, cx: &mut Context<Self>) {
        use crate::settings::DeletePref;
        let paths = self.selection_paths();
        if paths.is_empty() {
            return;
        }
        // inside an archive: remove the entries from it (always asks: there is no recycle bin).
        // An archive file itself splits as its own root (an empty inner path): that is a plain
        // file to delete (deleting a .zip asked to remove "" from inside it and failed)
        if let Some((archive, _)) = crate::archive_view::split_inside(&paths[0]) {
            let inners: Vec<String> = paths
                .iter()
                .filter_map(|p| crate::archive_view::split_inside(p).filter(|(a, _)| *a == archive))
                .map(|(_, i)| i.to_string_lossy().replace('\\', "/"))
                .collect();
            let what = if inners.len() == 1 { format!("\"{}\"", inners[0].rsplit('/').next().unwrap_or_default()) } else { crate::i18n::items(inners.len()) };
            crate::prompt::show(
                t!("Delete from the archive?"),
                Some(format!("{what} {}", t!("will be permanently deleted from the archive."))),
                None,
                vec![
                    crate::prompt::Button::new(t!("Delete"), crate::prompt::Kind::Danger, move |_, _, cx| {
                        crate::archive_view::delete_inside(cx, archive.clone(), inners.clone());
                        Ok(())
                    }),
                    crate::prompt::Button::cancel(),
                ],
                window,
                cx,
            );
            return;
        }
        let ask = match cx.global::<crate::settings::Settings>().confirm_delete {
            DeletePref::Always => true,
            DeletePref::PermanentOnly => permanent,
            DeletePref::Never => false,
        };
        if !ask {
            crate::status_center::delete(cx, paths, permanent);
            return;
        }
        let what = if paths.len() == 1 {
            format!("\"{}\"", rf_core::history::display_name(&paths[0]))
        } else {
            crate::i18n::items(paths.len())
        };
        let (title, msg) = if permanent {
            (t!("Delete permanently?"), format!("{what} {}", t!("will be deleted and can't be recovered.")))
        } else {
            (t!("Move to the Trash?"), format!("{what} {}", t!("will be moved to the Trash.")))
        };
        crate::prompt::show(
            title,
            Some(msg),
            None,
            vec![
                crate::prompt::Button::new(t!("Delete"), if permanent { crate::prompt::Kind::Danger } else { crate::prompt::Kind::Primary }, move |_, _, cx| {
                    crate::status_center::delete(cx, paths.clone(), permanent);
                    Ok(())
                }),
                crate::prompt::Button::cancel(),
            ],
            window,
            cx,
        );
    }

    pub fn create_new(&mut self, folder: bool, cx: &mut Context<Self>) {
        let Some(dir) = self.current_dir() else { return };
        // inside an archive: a folder entry, or an empty text file added to it
        if let Some((archive, inner)) = crate::archive_view::split(&dir) {
            let taken: Vec<String> = self.entries.iter().map(|e| e.name.to_string()).collect();
            let inner = inner.to_string_lossy().replace('\\', "/").trim_matches('/').to_string();
            if folder {
                let name = crate::archive_view::unique_name(t!("New folder"), "", &taken);
                crate::archive_view::mkdir_inside(cx, archive, inner, name.clone());
                self.set_reselect(name);
            } else {
                let name = crate::archive_view::unique_name(t!("New text document"), ".txt", &taken);
                let file = rf_linux::private_tmp::dir("new").map(|d| d.join(&name));
                if let Ok(file) = file && std::fs::write(&file, b"").is_ok() {
                    crate::archive_view::add_into(cx, archive, std::path::PathBuf::from(inner), vec![file]);
                    self.set_reselect(name);
                }
            }
            return;
        }
        let task = cx.background_spawn(async move {
            if folder { fileops::new_folder(&dir) } else { fileops::new_text_file(&dir) }
        });
        cx.spawn(async move |this, cx| {
            if let Ok(path) = task.await {
                crate::undo::push(crate::undo::UndoOp::Created { path: path.clone() });
                let name = path.file_name().map(|n| n.to_string_lossy().into_owned());
                let _ = this.update(cx, |v, cx| {
                    v.expect_new_item(name, cx);
                });
            }
        })
        .detach();
    }

    /// New → a template: a copy of it here, then rename it inline.
    pub fn create_from_template(&mut self, template: PathBuf, cx: &mut Context<Self>) {
        let Some(dir) = self.current_dir() else { return };
        if crate::archive_view::split(&dir).is_some() {
            return;
        }
        let task = cx.background_spawn(async move { rf_linux::templates::create_from(&template, &dir) });
        cx.spawn(async move |this, cx| match task.await {
            Ok(path) => {
                crate::undo::push(crate::undo::UndoOp::Created { path: path.clone() });
                let name = path.file_name().map(|n| n.to_string_lossy().into_owned());
                let _ = this.update(cx, |v, cx| v.expect_new_item(name, cx));
            }
            Err(e) => log::warn!("new from template: {e}"),
        })
        .detach();
    }

    /// "Create folder with selection": a new folder here, the selected items moved into it,
    /// then rename the folder inline.
    pub fn folder_with_selection(&mut self, items: Vec<PathBuf>, cx: &mut Context<Self>) {
        let Some(dir) = self.current_dir() else { return };
        if items.is_empty() {
            return;
        }
        let task = cx.background_spawn(async move { fileops::new_folder(&dir) });
        cx.spawn(async move |this, cx| {
            if let Ok(folder) = task.await {
                let name = folder.file_name().map(|n| n.to_string_lossy().into_owned());
                let _ = this.update(cx, |v, cx| {
                    crate::status_center::transfer(cx, Transfer::Move, items, folder.clone());
                    v.expect_new_item(name, cx);
                });
            }
        })
        .detach();
    }

    /// "Flatten folder": every file in the subfolders of `dir` moves up into `dir` (unique
    /// names on collisions), then the emptied subfolders are removed. Asks first.
    pub fn flatten_folder(&mut self, dir: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        let name = rf_core::history::display_name(&dir);
        crate::prompt::show(
            t!("Flatten folder?"),
            Some(format!("{} \"{name}\" {}", t!("All files in subfolders"), t!("will be moved into it, and the empty subfolders will be deleted."))),
            None,
            vec![
                crate::prompt::Button::new(t!("Flatten"), crate::prompt::Kind::Primary, move |_, _, cx| {
                    let dir = dir.clone();
                    crate::status_center::start(cx, format!("{} {name}", t!("Flattening:")), move |t| {
                        let mut rec = Flattened::default();
                        let r = flatten(&dir, &t, &mut rec);
                        if !rec.moves.is_empty() || !rec.removed.is_empty() {
                            crate::undo::push(crate::undo::UndoOp::Flattened { dir: dir.clone(), rec });
                        }
                        r
                    });
                    Ok(())
                }),
                crate::prompt::Button::cancel(),
            ],
            window,
            cx,
        );
    }

    pub fn begin_rename(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(ei) = self.cursor_entry() else { return };
        let e = self.entry(ei);
        // Extensions hidden: edit only the name and keep the extension (as Explorer does)
        let name = match self.hidden_extension(ei, cx) {
            Some(_) => e.name[..e.stem_len()].to_string(),
            None => e.name.to_string(),
        };
        let sel_end = if e.is_dir() { name.len() } else { e.stem_len().min(name.len()) };
        let input = cx.new(|cx| InputState::new(window, cx).default_value(name));
        let sub = cx.subscribe_in(&input, window, |this, _, ev: &InputEvent, window, cx| match ev {
            InputEvent::PressEnter { .. } | InputEvent::Blur => this.commit_rename(window, cx),
            _ => {}
        });
        input.update(cx, |s, cx| {
            s.focus(window, cx);
            s.set_selected_range(0..sel_end, cx);
        });
        self.set_renaming(Some((ei, input)), Some(sub));
        cx.notify();
    }

    pub fn cancel_rename(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.set_renaming(None, None);
        window.focus(&self.focus, cx);
        cx.notify();
    }

    /// The extension (with its dot) that the list hides for this entry, if any.
    fn hidden_extension(&self, ei: u32, cx: &App) -> Option<String> {
        let e = self.entry(ei);
        let hidden = !cx.global::<crate::settings::Settings>().show_extensions && !e.is_dir() && !e.ext.is_empty();
        hidden.then(|| e.name[e.stem_len()..].to_string())
    }

    pub fn commit_rename(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((ei, input)) = self.renaming.take() else { return };
        let typed = input.read(cx).value().trim().to_string();
        let new_name = match self.hidden_extension(ei, cx) {
            Some(ext) if !typed.is_empty() => format!("{typed}{ext}"),
            _ => typed,
        };
        let old = self.entry(ei).name.to_string();
        self.set_renaming(None, None);
        // the platform's rule, said up front instead of an error after the fact (Explorer's on
        // Windows; Linux only forbids "/")
        if invalid_file_name(&new_name) {
            let title = crate::t!("Rename").to_string();
            let rule = "A file name can't contain \"/\".".to_string();
            let msg = format!("{}\n\n{}", new_name, rule);
            alert_box(title, msg, window.window_handle(), cx);
            window.focus(&self.focus, cx);
            cx.notify();
            return;
        }
        // inside an archive: rewrite the archive with the new name
        if !new_name.is_empty() && new_name != old {
            let from_path = self.parent_of(ei).map(|d| d.join(&old));
            if let Some((archive, inner)) = from_path.as_deref().and_then(crate::archive_view::split) {
                let inner = inner.to_string_lossy().replace('\\', "/");
                crate::archive_view::rename_inside(cx, archive, inner, new_name.clone());
                self.set_reselect(new_name);
                window.focus(&self.focus, cx);
                cx.notify();
                return;
            }
        }
        if !new_name.is_empty() && new_name != old {
            if let Some(dir) = self.parent_of(ei).map(|d| d.to_path_buf()) {
                let from = dir.join(&old);
                let to = dir.join(&new_name);
                self.set_reselect(new_name.clone());
                // Explorer-style warning when a file's extension changes.
                let ext = |n: &str| std::path::Path::new(n).extension().map(|e| e.to_string_lossy().to_lowercase());
                let warn = cx.global::<crate::settings::Settings>().warn_extension_change;
                let confirm_rx = (warn && !self.entry(ei).is_dir() && ext(&old).is_some() && ext(&old) != ext(&new_name)).then(|| {
                    let title = crate::t!("File name extension").to_string();
                    let msg = crate::t!("If you change a file name extension, the file might become unusable. Are you sure you want to change it?").to_string();
                    confirm_box(title, msg, window.window_handle(), cx)
                });
                let win = window.window_handle();
                let (f2, n2) = (from.clone(), new_name.clone());
                cx.spawn(async move |this, cx| {
                    if let Some(r) = confirm_rx {
                        if !r.await.unwrap_or(false) {
                            let _ = this.update(cx, |v, _| v.clear_reselect());
                            return;
                        }
                    }
                    // Another item already has that name (a case-only change is fine): say so
                    // plainly instead of the shell's "parameter is incorrect".
                    let dst = f2.with_file_name(&n2);
                    let clash = cx
                        .background_spawn(async move {
                            // names are case-sensitive: "A.txt" is another file
                            std::fs::symlink_metadata(&dst).is_ok()
                        })
                        .await;
                    if clash {
                        let title = crate::t!("Rename").to_string();
                        let msg = format!("{}\n\n{}", n2, crate::t!("An item with this name already exists. Try another name."));
                        let _ = cx.update(|cx| alert_box(title, msg, win, cx));
                        let _ = this.update(cx, |v, _| v.clear_reselect());
                        return;
                    }
                    let (tx, rx) = oneshot::channel::<anyhow::Result<bool>>();
                    bg("rename", move || {
                        let _ = tx.send(fileops::rename(&f2, &n2));
                        Ok(true)
                    });
                    match rx.await {
                        Ok(Ok(true)) => {
                            crate::undo::push(crate::undo::UndoOp::Renamed { from: from.clone(), to: to.clone() });
                            let _ = cx.update(|cx| crate::tags::update(cx, |t| t.moved(&from, &to)));
                        }
                        other => {
                            if let Ok(Err(e)) = other {
                                log::warn!("rename: {e:#}");
                            }
                            // nothing to wait for
                            let _ = this.update(cx, |v, _| v.clear_reselect());
                        }
                    }
                })
                .detach();
            }
        }
        window.focus(&self.focus, cx);
        cx.notify();
    }

    /// "Create link": a link to each item beside it (in the item's own folder).
    pub fn create_links_beside(&mut self, items: Vec<PathBuf>, cx: &mut Context<Self>) {
        let mut by_dir: Vec<(PathBuf, Vec<PathBuf>)> = Vec::new();
        for p in items {
            let Some(dir) = p.parent().map(PathBuf::from) else { continue };
            match by_dir.iter_mut().find(|(d, _)| *d == dir) {
                Some((_, v)) => v.push(p),
                None => by_dir.push((dir, vec![p])),
            }
        }
        self.make_links(by_dir, cx);
    }

    /// "Paste as link": links in the current folder to the files on the clipboard.
    pub fn paste_as_link(&mut self, cx: &mut Context<Self>) {
        let Some(dir) = self.current_dir() else { return };
        if crate::archive_view::split(&dir).is_some() {
            return;
        }
        let (tx, rx) = oneshot::channel();
        rf_linux::sta::run(move |owner| {
            let _ = tx.send(rf_linux::clipboard::get_files(owner).ok().flatten());
        });
        cx.spawn(async move |this, cx| {
            if let Ok(Some((paths, _))) = rx.await {
                let _ = this.update(cx, |v, cx| v.make_links(vec![(dir, paths)], cx));
            }
        })
        .detach();
    }

    /// Makes links to each group's targets in its folder, through the Status Center (errors
    /// show there); what was made lands on the undo stack and is selected.
    fn make_links(&mut self, jobs: Vec<(PathBuf, Vec<PathBuf>)>, cx: &mut Context<Self>) {
        let jobs: Vec<_> = jobs.into_iter().filter(|(d, t)| !t.is_empty() && crate::archive_view::split(d).is_none()).collect();
        let n: usize = jobs.iter().map(|(_, t)| t.len()).sum();
        if n == 0 {
            return;
        }
        let made = std::sync::Arc::new(parking_lot::Mutex::new(Vec::<(PathBuf, PathBuf)>::new()));
        let m2 = made.clone();
        let suffix = t!("link");
        let me = cx.entity().downgrade();
        crate::status_center::start_with(
            cx,
            format!("{} {}", t!("Creating links:"), crate::i18n::items(n)),
            move |_| {
                for (dir, targets) in &jobs {
                    let (pairs, err) = fileops::create_links(targets, dir, suffix);
                    m2.lock().extend(pairs);
                    if let Some(e) = err {
                        return Err(e.into());
                    }
                }
                Ok(true)
            },
            Some(Box::new(move |_, cx| {
                let pairs = std::mem::take(&mut *made.lock());
                if pairs.is_empty() {
                    return;
                }
                let names: Vec<String> = pairs.iter().filter_map(|(_, l)| l.file_name()).map(|n| n.to_string_lossy().into_owned()).collect();
                let here: Vec<PathBuf> = pairs.iter().filter_map(|(_, l)| l.parent().map(PathBuf::from)).collect();
                crate::undo::push(crate::undo::UndoOp::Linked { pairs });
                let _ = me.update(cx, |v, cx| {
                    if v.current_dir().is_some_and(|d| here.contains(&d)) {
                        v.select_when_listed(names, cx);
                    }
                });
            })),
        );
    }

    /// Files dropped onto `dest` (a folder row or the current folder background).
    pub fn drop_into(&mut self, dest: PathBuf, paths: &ExternalPaths, cx: &mut Context<Self>) {
        let sources: Vec<PathBuf> = crate::file_drag::take(paths);
        if sources.is_empty() || rf_linux::dnd::is_invalid_drop(&sources, &dest) {
            return;
        }
        let (ctrl, shift) = (crate::sys::ctrl_down(), crate::sys::shift_down());
        // Ctrl+Shift: links (as Dolphin, Nautilus and Thunar do)
        if ctrl && shift {
            self.make_links(vec![(dest, sources)], cx);
            return;
        }
        let kind = rf_linux::dnd::default_transfer(&sources, &dest, ctrl, shift);
        crate::status_center::transfer(cx, kind, sources, dest);
    }

    pub fn pin(&mut self, dir: PathBuf, cx: &mut Context<Self>) {
        // Linux: a network folder by its URL (see Workspace::set_pinned)
        let dir = rf_linux::remote::remote_url(&dir).map_or(dir, PathBuf::from);
        let current: Vec<PathBuf> = self.places_pinned_paths(cx);
        crate::settings::update(cx, |s| {
            let mut v = s.pinned.clone().unwrap_or(current);
            if !v.contains(&dir) {
                v.push(dir);
            }
            s.pinned = Some(v);
        });
        self.refresh_places(cx);
    }

    /// Our Properties window for the selection (or the current folder when nothing is selected).
    pub fn properties(&self, cx: &mut App) {
        let mut paths = self.selection_paths();
        if paths.is_empty() {
            paths.extend(self.current_dir());
        }
        crate::properties_view::open(paths, self.images.clone(), cx);
    }

    pub fn open_with(&self) {
        if let Some(p) = self.selection_paths().into_iter().next() {
            rf_linux::sta::run(move |_| {
                let _ = rf_linux::shell::open_with(&p);
            });
        }
    }
}

#[cfg(test)]
mod tests {
    // not `super::*`: gpui's `test` macro would shadow the built-in one
    use super::{Flattened, fileops, flatten, reflatten, unflatten};
    use std::sync::Arc;
    use std::sync::atomic::AtomicBool;

    fn tree(root: &std::path::Path) {
        std::fs::create_dir_all(root.join("Gün1/derin")).unwrap();
        std::fs::create_dir_all(root.join("Gün2")).unwrap();
        std::fs::create_dir_all(root.join("bos")).unwrap();
        std::fs::write(root.join("kok.txt"), b"kok").unwrap();
        std::fs::write(root.join("Gün1/a.mp4"), b"a1").unwrap();
        std::fs::write(root.join("Gün1/derin/a.mp4"), b"a2").unwrap();
        std::fs::write(root.join("Gün2/b.mp4"), b"b").unwrap();
    }

    fn listing(root: &std::path::Path) -> Vec<(String, Option<Vec<u8>>)> {
        let mut out = Vec::new();
        let mut stack = vec![root.to_path_buf()];
        while let Some(d) = stack.pop() {
            for e in std::fs::read_dir(&d).unwrap().flatten() {
                let rel = e.path().strip_prefix(root).unwrap().to_string_lossy().into_owned();
                if e.path().is_dir() {
                    stack.push(e.path());
                    out.push((rel, None));
                } else {
                    out.push((rel, Some(std::fs::read(e.path()).unwrap())));
                }
            }
        }
        out.sort();
        out
    }

    #[test]
    fn flatten_undo_redo() {
        let root = std::env::temp_dir().join(format!("rf-flatten-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        tree(&root);
        let before = listing(&root);
        let t = fileops::Tracker::new(Box::new(|_, _, _| {}), Arc::new(AtomicBool::new(false)));

        let mut rec = Flattened::default();
        assert!(flatten(&root, &t, &mut rec).unwrap());
        let flat = listing(&root);
        // same-named files don't overwrite each other; every folder is gone
        assert_eq!(flat.iter().filter(|(_, c)| c.is_none()).count(), 0, "{flat:?}");
        assert_eq!(flat.len(), 4, "{flat:?}");
        assert!(root.join("a (2).mp4").exists());
        assert_eq!(rec.moves.len(), 3);
        assert_eq!(rec.removed.len(), 4); // Gün1, Gün1/derin, Gün2, bos

        assert!(unflatten(&rec).unwrap());
        assert_eq!(listing(&root), before); // folders (also the empty one) and files back

        assert!(reflatten(&rec).unwrap());
        assert_eq!(listing(&root), flat);

        // a file deleted after flattening: the rest still goes back, the miss is reported
        std::fs::remove_file(root.join("b.mp4")).unwrap();
        assert!(unflatten(&rec).is_err());
        assert!(root.join("Gün1/derin/a.mp4").exists() && root.join("Gün1/a.mp4").exists());
        let _ = std::fs::remove_dir_all(&root);
    }
}

/// Whether a new file name breaks the platform's rules: Explorer's characters and no trailing
/// dot on Windows; only "/" on Linux (and never "." or "..").
pub(crate) fn invalid_file_name(name: &str) -> bool {
    name.contains(['/', '\0']) || name == "." || name == ".."
}

/// A message box: the shell's on Windows, the app's own dialog in `win` elsewhere (Linux has
/// no shell message box).
pub(crate) fn alert_box(title: String, msg: String, win: AnyWindowHandle, cx: &mut App) {
    let _ = win.update(cx, |_, window, cx| {
        let ok = crate::prompt::Button::new(crate::t!("OK"), crate::prompt::Kind::Primary, |_, _, _| Ok(()));
        crate::prompt::show(title, Some(msg), None, vec![ok], window, cx);
    });
}

/// A yes / no question: the shell's on Windows, the app's own dialog (OK / Cancel) in `win`
/// elsewhere. Closing it without answering is a no.
fn confirm_box(title: String, msg: String, win: AnyWindowHandle, cx: &mut App) -> oneshot::Receiver<bool> {
    let (tx, rx) = oneshot::channel::<bool>();
    {
        let tx = std::rc::Rc::new(std::cell::RefCell::new(Some(tx)));
        let answer = move |yes: bool| {
            let tx = tx.clone();
            move |_: Option<String>, _: &mut Window, _: &mut App| {
                if let Some(tx) = tx.borrow_mut().take() {
                    let _ = tx.send(yes);
                }
                Ok(())
            }
        };
        let _ = win.update(cx, |_, window, cx| {
            let buttons = vec![
                crate::prompt::Button::new(crate::t!("OK"), crate::prompt::Kind::Primary, answer(true)),
                crate::prompt::Button::new(crate::t!("Cancel"), crate::prompt::Kind::Normal, answer(false)),
            ];
            crate::prompt::show(title, Some(msg), None, buttons, window, cx);
        });
    }
    rx
}

#[cfg(test)]
mod name_tests {
    use super::invalid_file_name;

    #[test]
    fn file_name_rules() {
        assert!(invalid_file_name("a/b"));
        assert!(!invalid_file_name("normal.txt"));
        // legal on Linux
        assert!(!invalid_file_name("notes: 2026?") && !invalid_file_name("dot.") && !invalid_file_name("a\\b"));
        assert!(invalid_file_name(".") && invalid_file_name(".."));
    }
}
