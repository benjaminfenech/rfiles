//! Folder aliases (Files' "@name"): a user-defined short name for a folder, typed in the
//! address box as `@name` or `@name\sub\path`. Kept in settings (`folder_aliases`); a copy
//! lives here so path parsing anywhere can resolve without the app context.

use std::path::{Path, PathBuf};
use std::sync::RwLock;

pub const PREFIX: char = '@';

static ALIASES: RwLock<Vec<(String, PathBuf)>> = RwLock::new(Vec::new());

pub fn set(list: impl IntoIterator<Item = (String, PathBuf)>) {
    let mut v: Vec<(String, PathBuf)> = list.into_iter().collect();
    v.sort_by_key(|(n, _)| n.to_lowercase());
    if let Ok(mut a) = ALIASES.write() {
        *a = v;
    }
}

pub fn all() -> Vec<(String, PathBuf)> {
    ALIASES.read().map(|a| a.clone()).unwrap_or_default()
}

pub fn is_valid_name(name: &str) -> bool {
    let n = name.trim();
    !n.is_empty() && !n.contains(['\\', '/']) && !n.starts_with(PREFIX)
}

fn split(text: &str) -> Option<(&str, &str)> {
    let rest = text.trim().strip_prefix(PREFIX)?;
    Some(match rest.find(['\\', '/']) {
        Some(i) => (rest[..i].trim(), &rest[i..]),
        None => (rest.trim(), ""),
    })
}

/// `@name` / `@name\sub` → the folder path (sub path and trailing separator kept).
pub fn resolve(text: &str) -> Option<String> {
    resolve_in(text, &all())
}

fn resolve_in(text: &str, list: &[(String, PathBuf)]) -> Option<String> {
    let (name, sub) = split(text)?;
    let (_, path) = list.iter().find(|(n, _)| n.eq_ignore_ascii_case(name) || n.to_lowercase() == name.to_lowercase())?;
    let base = path.to_string_lossy();
    let base = base.trim_end_matches(['\\', '/']);
    let sep = std::path::MAIN_SEPARATOR;
    // "/" typed in the sub path becomes the platform's separator (backslash on Windows)
    Some(if sub.is_empty() { format!("{base}{sep}") } else { format!("{base}{}", sub.replace(['/', '\\'], &sep.to_string())) })
}

/// Aliases starting with the typed name, while only the name is typed (`@pro`).
pub fn matches(text: &str) -> Vec<(String, PathBuf)> {
    match split(text) {
        Some((q, "")) => {
            let q = q.to_lowercase();
            all().into_iter().filter(|(n, _)| n.to_lowercase().starts_with(&q)).collect()
        }
        _ => Vec::new(),
    }
}

/// The alias name pointing at `dir`, if any.
pub fn name_of(dir: &Path) -> Option<String> {
    all().into_iter().find(|(_, p)| p == dir).map(|(n, _)| n)
}

/// "Takma ad ver…": asks for a name for `dir` (pre-filled with its current alias, if any).
pub fn edit_dialog(dir: PathBuf, window: &mut gpui_kit::Window, cx: &mut gpui_kit::App) {
    use crate::prompt::{Button, Kind};
    use crate::t;
    let current = name_of(&dir);
    let folder = dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| dir.to_string_lossy().into_owned());
    let initial = current.clone().unwrap_or_else(|| folder.to_lowercase().replace(' ', "-"));
    let message = format!("{} @{}", t!("Type this in the address bar to go to the folder:"), initial);
    let mut buttons = vec![Button::new(t!("Save"), Kind::Primary, {
        let dir = dir.clone();
        let current = current.clone();
        move |text, _, cx| {
            let name = text.unwrap_or_default().trim().trim_start_matches(PREFIX).trim().to_string();
            if !is_valid_name(&name) {
                return Err(t!("The name can't be empty or contain \\ or /").to_string());
            }
            let taken = all().into_iter().any(|(n, p)| n.to_lowercase() == name.to_lowercase() && p != dir);
            if taken {
                return Err(t!("This name is used by another folder").to_string());
            }
            let (dir, current) = (dir.clone(), current.clone());
            crate::settings::update(cx, move |s| {
                if let Some(old) = &current {
                    s.folder_aliases.remove(old);
                }
                s.folder_aliases.insert(name, dir);
            });
            Ok(())
        }
    })];
    if let Some(old) = current {
        buttons.push(Button::new(t!("Remove"), Kind::Danger, move |_, _, cx| {
            let old = old.clone();
            crate::settings::update(cx, move |s| {
                s.folder_aliases.remove(&old);
            });
            Ok(())
        }));
    }
    buttons.push(Button::cancel());
    crate::prompt::show(
        format!("{} — {folder}", t!("Folder alias")),
        Some(message),
        Some((initial, t!("name").to_string())),
        buttons,
        window,
        cx,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_on_unix() {
        let list = vec![("proje".to_string(), PathBuf::from("/home/me/İşler/Proje")), ("Ölçüm".to_string(), PathBuf::from("/data/"))];
        assert_eq!(resolve_in("@proje", &list).as_deref(), Some("/home/me/İşler/Proje/"));
        assert_eq!(resolve_in("@PROJE/src/lib", &list).as_deref(), Some("/home/me/İşler/Proje/src/lib"));
        assert_eq!(resolve_in(" @ölçüm/", &list).as_deref(), Some("/data/"));
        assert_eq!(resolve_in("/proje", &list), None);
    }
}
