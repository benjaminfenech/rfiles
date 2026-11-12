//! The shell part of the context menu. On Windows it is Explorer's (IContextMenu); on Linux
//! there is no shell menu, so [`build`] offers what Linux file managers do: "Open with" (the
//! apps for the selection's type, from [`crate::xdg_apps`]), and [`invoke`] launches the
//! chosen app. The app's own items cover the rest.
// The rest of this file mirrors rf-win's API; functions without a Linux meaning return
// "not available" results.
use std::path::{Path, PathBuf};
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
/// Must be called on the shell STA thread (see `sta::run`) with its owner window.
pub fn show_for_items(
    _owner: crate::Opaque,
    _items: &[PathBuf],
    _screen: (i32, i32),
    _extended: bool,
    _extra: &[ExtraItem],
) -> anyhow::Result<MenuOutcome> {
    Err(
        anyhow::anyhow!("rf-linux: `show_for_items` is not implemented on Linux yet")
            .into(),
    )
}
/// Menu for one item (used by the sidebar).
pub fn show_for_one(
    _owner: crate::Opaque,
    _item: &Path,
    _screen: (i32, i32),
    _extended: bool,
    _extra: &[ExtraItem],
) -> anyhow::Result<MenuOutcome> {
    Err(
        anyhow::anyhow!("rf-linux: `show_for_one` is not implemented on Linux yet")
            .into(),
    )
}
/// Background (empty space) menu of a folder.
pub fn show_for_background(
    _owner: crate::Opaque,
    _dir: &Path,
    _screen: (i32, i32),
    _extended: bool,
    _extra: &[ExtraItem],
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
/// The live menu's commands: id - 1 → (app ID, files).
static LIVE: std::sync::Mutex<Option<(u64, Vec<(String, Vec<PathBuf>)>)>> = std::sync::Mutex::new(None);
static NEXT_SESSION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

/// An app's icon for a menu row (cached per app).
fn app_icon(app: &crate::xdg_apps::App) -> Option<ShellIcon> {
    static CACHE: std::sync::OnceLock<std::sync::Mutex<std::collections::HashMap<String, Option<ShellIcon>>>> = std::sync::OnceLock::new();
    let cache = CACHE.get_or_init(Default::default);
    if let Some(hit) = cache.lock().unwrap().get(&app.id) {
        return hit.clone();
    }
    // drawn at 32 px: sharp at 2x scaling
    let icon = app.icon.as_ref().and_then(|i| {
        if i.starts_with('/') {
            crate::xdg_icons::render_file(Path::new(i), 32)
        } else {
            crate::xdg_icons::render_first(&[i.clone(), "application-x-executable".into()], 32)
        }
    });
    let icon = icon.map(|b| ShellIcon { width: b.width, height: b.height, bgra: b.data });
    cache.lock().unwrap().insert(app.id.clone(), icon.clone());
    icon
}

/// Builds the shell part of the menu for `target`: on Linux an "Open with" submenu (the
/// default app first, then the others for the type and its parent types, then "Choose
/// another app" with every installed app). The background of a folder has none.
pub fn build(_owner: crate::Opaque, target: &ShellTarget, _extended: bool) -> anyhow::Result<(u64, Vec<ShellItem>)> {
    let session = NEXT_SESSION.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let files: Vec<PathBuf> = match target {
        ShellTarget::Items(v) => v.clone(),
        ShellTarget::One(p) => vec![p.clone()],
        ShellTarget::Background(_) => Vec::new(),
    };
    if files.is_empty() {
        *LIVE.lock().unwrap() = Some((session, Vec::new()));
        return Ok((session, Vec::new()));
    }
    // apps that suit every selected item, in the first item's order
    let mut types: Vec<String> = files.iter().map(|f| crate::xdg_apps::mime_for_path(f)).collect();
    types.dedup();
    let mut suits = crate::xdg_apps::apps_for(&types[0]);
    for t in &types[1..] {
        let other = crate::xdg_apps::apps_for(t);
        suits.retain(|a| other.iter().any(|o| o.id == a.id));
    }
    let mut commands: Vec<(String, Vec<PathBuf>)> = Vec::new();
    let row = |app: &crate::xdg_apps::App, commands: &mut Vec<(String, Vec<PathBuf>)>| {
        commands.push((app.id.clone(), files.clone()));
        ShellItem { id: commands.len() as u32, label: app.name.clone(), hint: None, verb: String::new(), icon: app_icon(app), enabled: true, separator: false, children: Vec::new() }
    };
    let mut children: Vec<ShellItem> = suits.iter().map(|a| row(a, &mut commands)).collect();
    let others: Vec<ShellItem> = crate::xdg_apps::all_apps().into_iter().filter(|a| !suits.iter().any(|s| s.id == a.id)).map(|a| row(a, &mut commands)).collect();
    let sep = ShellItem { id: 0, label: String::new(), hint: None, verb: String::new(), icon: None, enabled: true, separator: true, children: Vec::new() };
    if !children.is_empty() && !others.is_empty() {
        children.push(sep);
    }
    if !others.is_empty() {
        // the app translates this label; "openas" marks the submenu as "Open with"
        children.push(ShellItem { id: 0, label: "Choose another app".into(), hint: None, verb: "openas".into(), icon: None, enabled: true, separator: false, children: others });
    }
    *LIVE.lock().unwrap() = Some((session, commands));
    let open_with = ShellItem { id: 0, label: "Open with".into(), hint: None, verb: String::new(), icon: None, enabled: true, separator: false, children };
    Ok((session, vec![open_with]))
}

/// Runs command `id` of session `session` (from [`build`]): launches that app on the files.
pub fn invoke(_owner: crate::Opaque, session: u64, id: u32, _screen: (i32, i32)) -> anyhow::Result<MenuOutcome> {
    let cmd = {
        let live = LIVE.lock().unwrap();
        let Some((_s, cmds)) = live.as_ref().filter(|(s, _)| *s == session) else { anyhow::bail!("the menu is gone") };
        cmds.get((id as usize).wrapping_sub(1)).cloned()
    };
    let Some((app_id, files)) = cmd else { anyhow::bail!("no such command") };
    let app = crate::xdg_apps::all_apps()
        .into_iter()
        .chain(crate::xdg_apps::apps_for(&crate::xdg_apps::mime_for_path(files.first().map(PathBuf::as_path).unwrap_or(Path::new("")))))
        .find(|a| a.id == app_id)
        .ok_or_else(|| anyhow::anyhow!("{app_id} is not installed any more"))?;
    crate::xdg_apps::launch(app, &files)?;
    Ok(MenuOutcome::Invoked("open".into()))
}

/// Drops the live session if it is `session` (menu closed without choosing a shell command).
pub fn release(session: u64) {
    let mut live = LIVE.lock().unwrap();
    if live.as_ref().is_some_and(|(s, _)| *s == session) {
        *live = None;
    }
}

/// Invokes a canonical verb on `target` without showing a menu. Linux knows "openas" (the
/// desktop's app chooser); Explorer verbs ("properties", "Windows.ModernShare"…) have no
/// equivalent.
pub fn invoke_verb(_owner: crate::Opaque, target: &ShellTarget, verb: &str, _screen: (i32, i32)) -> anyhow::Result<()> {
    let path = match target {
        ShellTarget::Items(v) => v.first().cloned(),
        ShellTarget::One(p) | ShellTarget::Background(p) => Some(p.clone()),
    };
    match (verb, path) {
        ("openas", Some(p)) => crate::shell::open_with(&p),
        _ => anyhow::bail!("rf-linux: the `{verb}` verb is not available on Linux"),
    }
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
/// A plain popup menu of app items at `screen` (no shell items). Returns the chosen id.
/// Call on the shell STA thread with its owner window.
pub fn show_simple(
    _owner: crate::Opaque,
    _screen: (i32, i32),
    _items: &[ExtraItem],
) -> Option<u32> {
    None
}
