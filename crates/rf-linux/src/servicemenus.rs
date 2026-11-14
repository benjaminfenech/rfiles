//! The user's own context-menu actions, in the formats other file managers already use, so
//! actions installed for them show in rfiles too:
//!
//! * Dolphin's service menus: `.desktop` files in `kio/servicemenus/` (KDE 6) and
//!   `kservices5/ServiceMenus/` (KDE 5) under every data dir, the user's own winning by file
//!   name. Each has `[Desktop Action …]` groups with a `Name`, `Icon` and `Exec`, and says which
//!   types it suits (`MimeType`, or the types in `X-KDE-ServiceTypes`), how many items
//!   (`X-KDE-MinNumberOfUrls`, `X-KDE-MaxNumberOfUrls`, `X-KDE-RequiredNumberOfUrls`), and
//!   optionally a submenu (`X-KDE-Submenu`).
//! * Nautilus scripts: executable files in `~/.local/share/nautilus/scripts`, run with the
//!   selected files as arguments (and in `NAUTILUS_SCRIPT_SELECTED_FILE_PATHS`).
//!
//! Read on every menu (they're small, and an action added while rfiles runs shows at once);
//! call off the UI thread.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::xdg_apps::{localized, unescape_value};

/// One action: a menu row that runs a command on the selection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Action {
    pub name: String,
    /// Icon name, or an absolute path.
    pub icon: Option<String>,
    /// The `Exec` line, with the Desktop Entry field codes (`%f %F %u %U`).
    pub exec: String,
    /// The submenu it goes in (`X-KDE-Submenu`; "Scripts" for Nautilus scripts).
    pub submenu: Option<String>,
    /// A Nautilus script: also gets `NAUTILUS_SCRIPT_SELECTED_FILE_PATHS`.
    pub script: bool,
}

/// A service menu file: when its actions apply, and the actions.
#[derive(Debug, Clone, Default)]
struct ServiceMenu {
    types: Vec<String>,
    exclude: Vec<String>,
    min: Option<usize>,
    max: Option<usize>,
    /// Exactly these counts (`X-KDE-RequiredNumberOfUrls=1,2`).
    exactly: Vec<usize>,
    actions: Vec<Action>,
}

/// Every `[group]` of a desktop file as name → (key → value).
fn groups(text: &str) -> HashMap<String, HashMap<String, String>> {
    let mut out: HashMap<String, HashMap<String, String>> = HashMap::new();
    let mut cur: Option<String> = None;
    for line in text.lines().map(str::trim) {
        if let Some(g) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            cur = Some(g.to_string());
            out.entry(g.to_string()).or_default();
        } else if let (Some(g), false) = (&cur, line.starts_with('#')) {
            if let Some((k, v)) = line.split_once('=') {
                out.entry(g.clone()).or_default().insert(k.trim().to_string(), unescape_value(v.trim()));
            }
        }
    }
    out
}

fn list(v: Option<&String>, sep: &[char]) -> Vec<String> {
    v.map(|s| s.split(sep).map(str::trim).filter(|s| !s.is_empty()).map(String::from).collect()).unwrap_or_default()
}

fn parse(text: &str) -> Option<ServiceMenu> {
    let g = groups(text);
    let e = g.get("Desktop Entry")?;
    if e.get("Hidden").is_some_and(|v| v == "true") {
        return None;
    }
    // only menus for local files (no X-KDE-Protocols means "all", local ones included)
    let protocols = list(e.get("X-KDE-Protocols"), &[',', ';']);
    if !protocols.is_empty() && !protocols.iter().any(|p| p == "file") {
        return None;
    }
    if let Some(t) = e.get("TryExec") {
        if !crate::sandbox::in_path(t) {
            return None;
        }
    }
    // the types: MimeType, or the X-KDE-ServiceTypes entries that aren't the plugin kind
    let mut types = list(e.get("MimeType"), &[';', ',']);
    types.extend(list(e.get("X-KDE-ServiceTypes"), &[',', ';']).into_iter().filter(|t| !t.starts_with("KonqPopupMenu") && !t.starts_with("KFileItemAction")));
    if types.is_empty() {
        types.push("all/all".into());
    }
    let num = |k: &str| e.get(k).and_then(|v| v.trim().parse::<usize>().ok());
    let submenu = localized(e, "X-KDE-Submenu").filter(|s| !s.is_empty());
    let mut actions = Vec::new();
    for id in list(e.get("Actions"), &[';', ',']) {
        // KDE's "_SEPARATOR_" between actions has no meaning in a list of rows
        let Some(a) = g.get(&format!("Desktop Action {id}")) else { continue };
        let (Some(name), Some(exec)) = (localized(a, "Name"), a.get("Exec").filter(|x| !x.trim().is_empty())) else { continue };
        // a program that isn't installed (Dolphin hides those too)
        if !exec_installed(exec) {
            continue;
        }
        actions.push(Action { name, icon: a.get("Icon").filter(|s| !s.is_empty()).cloned(), exec: exec.clone(), submenu: submenu.clone(), script: false });
    }
    if actions.is_empty() {
        return None;
    }
    Some(ServiceMenu {
        types,
        exclude: list(e.get("X-KDE-ExcludeMimeTypes"), &[',', ';']),
        min: num("X-KDE-MinNumberOfUrls"),
        max: num("X-KDE-MaxNumberOfUrls"),
        exactly: list(e.get("X-KDE-RequiredNumberOfUrls"), &[',', ';']).iter().filter_map(|n| n.parse().ok()).collect(),
        actions,
    })
}

/// Whether the program an `Exec` line starts with is there (a word before `=` is an
/// environment setting, as in `env FOO=1 prog`).
fn exec_installed(exec: &str) -> bool {
    let words = crate::xdg_apps::exec_words_pub(exec);
    let prog = words.iter().map(String::as_str).find(|w| *w != "env" && !w.contains('='));
    prog.is_some_and(crate::sandbox::in_path)
}

/// Whether type `pattern` covers an item of type `mime` (with its parent types `chain`).
fn covers(pattern: &str, mime: &str, chain: &[String]) -> bool {
    let dir = mime == "inode/directory";
    match pattern {
        "all/all" | "*" | "*/*" => true,
        "all/allfiles" | "application/octet-stream" => !dir,
        p => match p.strip_suffix("/*") {
            Some(top) => chain.iter().any(|m| m.split('/').next() == Some(top)),
            None => chain.iter().any(|m| m == p),
        },
    }
}

impl ServiceMenu {
    fn applies(&self, mimes: &[(String, Vec<String>)]) -> bool {
        let n = mimes.len();
        if n == 0 || self.min.is_some_and(|m| n < m) || self.max.is_some_and(|m| n > m) || (!self.exactly.is_empty() && !self.exactly.contains(&n)) {
            return false;
        }
        mimes.iter().all(|(mime, chain)| self.types.iter().any(|t| covers(t, mime, chain)) && !self.exclude.iter().any(|t| covers(t, mime, chain)))
    }
}

/// The folders service menus live in, the user's first.
fn menu_dirs() -> Vec<PathBuf> {
    crate::xdg_icons::data_dirs().into_iter().flat_map(|d| [d.join("kio/servicemenus"), d.join("kservices5/ServiceMenus")]).collect()
}

/// Every service menu, by file name (the first folder that has a name wins).
fn menus(dirs: &[PathBuf]) -> Vec<ServiceMenu> {
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for d in dirs {
        let Ok(rd) = std::fs::read_dir(d) else { continue };
        let mut files: Vec<PathBuf> = rd.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "desktop")).collect();
        files.sort();
        for f in files {
            let Some(name) = f.file_name().map(|n| n.to_os_string()) else { continue };
            if !seen.insert(name) {
                continue;
            }
            if let Some(m) = std::fs::read_to_string(&f).ok().and_then(|t| parse(&t)) {
                out.push(m);
            }
        }
    }
    out
}

/// Nautilus scripts: the executable files in `dir` (one level), by name.
fn scripts(dir: &Path) -> Vec<Action> {
    use std::os::unix::fs::PermissionsExt;
    let Ok(rd) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut out: Vec<Action> = rd
        .flatten()
        .filter(|e| !e.file_name().to_string_lossy().starts_with('.'))
        .filter(|e| std::fs::metadata(e.path()).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0))
        .map(|e| {
            let path = e.path();
            // a path with spaces, quoted for the Exec rules
            let quoted = format!("\"{}\"", path.to_string_lossy().replace('\\', "\\\\").replace('"', "\\\"").replace('`', "\\`").replace('$', "\\$"));
            Action { name: e.file_name().to_string_lossy().into_owned(), icon: None, exec: format!("{quoted} %F"), submenu: Some("Scripts".into()), script: true }
        })
        .collect();
    out.sort_by_key(|a| a.name.to_lowercase());
    out
}

fn types_of(paths: &[PathBuf]) -> Vec<(String, Vec<String>)> {
    paths
        .iter()
        .map(|p| {
            let m = crate::xdg_apps::mime_for_path(p);
            let chain = crate::xdg_apps::with_parents(&m);
            (m, chain)
        })
        .collect()
}

/// The actions for `paths` (the selection, or the folder for its background menu): service
/// menus that suit every item, then Nautilus scripts.
pub fn actions_for(paths: &[PathBuf]) -> Vec<Action> {
    let mimes = types_of(paths);
    let mut out: Vec<Action> = menus(&menu_dirs()).into_iter().filter(|m| m.applies(&mimes)).flat_map(|m| m.actions).collect();
    // a script already offered through a service menu (some installers add both) shows once
    let scripts = dirs::data_dir().map(|d| scripts(&d.join("nautilus/scripts"))).unwrap_or_default();
    let scripts: Vec<Action> = scripts.into_iter().filter(|s| !out.iter().any(|a| a.exec == s.exec)).collect();
    out.extend(scripts);
    out
}

/// Runs `action` on `files` (detached), the way its `Exec` line says.
pub fn run(action: &Action, files: &[PathBuf]) -> anyhow::Result<()> {
    let app = crate::xdg_apps::App {
        id: String::new(),
        name: action.name.clone(),
        exec: action.exec.clone(),
        icon: action.icon.clone(),
        mime_types: Vec::new(),
        terminal: false,
        categories: Vec::new(),
        no_display: true,
        path: PathBuf::new(),
    };
    for line in crate::xdg_apps::command_lines(&app, files) {
        let Some((prog, args)) = line.split_first() else { continue };
        let mut cmd = crate::sandbox::launch(prog);
        cmd.args(args);
        if action.script {
            let list: String = files.iter().map(|f| format!("{}\n", f.to_string_lossy())).collect();
            cmd.env("NAUTILUS_SCRIPT_SELECTED_FILE_PATHS", list);
            if let Some(dir) = files.first().and_then(|f| f.parent()) {
                cmd.current_dir(dir);
            }
        }
        crate::shell::spawn_detached(&mut cmd).map_err(|e| anyhow::anyhow!("{}: {e}", action.name))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const KRENAME: &str = "# comment\n[Desktop Entry]\nType=Service\nMimeType=inode/directory\nActions=rename;\nX-KDE-Submenu=Rename\nX-KDE-Submenu[de]=Umbenennen\n\n[Desktop Action rename]\nExec=sh -r %U\nIcon=krename\nName=Rename with KRename\n";

    fn t(m: &str) -> (String, Vec<String>) {
        let mut chain = vec![m.to_string()];
        if m == "image/png" {
            chain.push("application/octet-stream".into());
        }
        (m.to_string(), chain)
    }

    #[test]
    fn parses_and_matches() {
        let m = parse(KRENAME).unwrap();
        assert_eq!(m.actions[0], Action { name: "Rename with KRename".into(), icon: Some("krename".into()), exec: "sh -r %U".into(), submenu: Some("Rename".into()), script: false });
        assert!(m.applies(&[t("inode/directory")]));
        assert!(!m.applies(&[t("inode/directory"), t("image/png")]), "every item must suit");

        let fonts = parse("[Desktop Entry]\nX-KDE-ServiceTypes=KonqPopupMenu/Plugin,font/ttf,image/*\nActions=a\nX-KDE-MaxNumberOfUrls=1\nX-KDE-ExcludeMimeTypes=image/gif\n[Desktop Action a]\nName=A\nExec=sh %f\n").unwrap();
        assert_eq!(fonts.types, ["font/ttf", "image/*"]);
        assert!(fonts.applies(&[t("image/png")]));
        assert!(!fonts.applies(&[t("image/gif")]), "excluded");
        assert!(!fonts.applies(&[t("image/png"), t("image/png")]), "at most one");

        let all = parse("[Desktop Entry]\nMimeType=all/allfiles;\nActions=a\nX-KDE-RequiredNumberOfUrls=2\n[Desktop Action a]\nName=Compare\nExec=sh %F\n").unwrap();
        assert!(all.applies(&[t("image/png"), t("text/plain")]));
        assert!(!all.applies(&[t("text/plain")]), "exactly two");
        assert!(!all.applies(&[t("inode/directory"), t("text/plain")]), "files only");

        // not for local files, hidden, or no usable action
        assert!(parse("[Desktop Entry]\nX-KDE-Protocols=smb\nActions=a\n[Desktop Action a]\nName=A\nExec=sh\n").is_none());
        assert!(parse("[Desktop Entry]\nHidden=true\nActions=a\n[Desktop Action a]\nName=A\nExec=sh\n").is_none());
        assert!(parse("[Desktop Entry]\nActions=a;b\n[Desktop Action a]\nName=A\n").is_none());
        assert!(parse("[Desktop Entry]\nActions=a\n[Desktop Action a]\nName=A\nExec=rfiles-no-such-program %f\n").is_none(), "not installed");
        assert!(exec_installed("env LANG=C sh -c true") && !exec_installed("env X=1"));
    }

    #[test]
    fn user_menus_win_and_scripts_run() {
        use std::os::unix::fs::PermissionsExt;
        let d = std::env::temp_dir().join(format!("rf-servicemenus-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        let (user, sys) = (d.join("user"), d.join("sys"));
        std::fs::create_dir_all(&user).unwrap();
        std::fs::create_dir_all(&sys).unwrap();
        std::fs::write(sys.join("x.desktop"), KRENAME).unwrap();
        std::fs::write(user.join("x.desktop"), KRENAME.replace("Rename with KRename", "Mine")).unwrap();
        std::fs::write(sys.join("other.desktop"), KRENAME.replace("Rename with KRename", "Other")).unwrap();
        let names: Vec<String> = menus(&[user.clone(), sys.clone()]).into_iter().flat_map(|m| m.actions).map(|a| a.name).collect();
        assert_eq!(names, ["Mine", "Other"]);

        // scripts: executable files only; they get the selection as arguments and in the env
        let sd = d.join("scripts");
        std::fs::create_dir_all(&sd).unwrap();
        let out = d.join("out.txt");
        let script = sd.join("Write list");
        std::fs::write(&script, format!("#!/bin/sh\nprintf '%s|%s' \"$1\" \"$NAUTILUS_SCRIPT_SELECTED_FILE_PATHS\" > '{}'\n", out.display())).unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::fs::write(sd.join("not executable"), b"").unwrap();
        let found = scripts(&sd);
        assert_eq!(found.len(), 1);
        let target = d.join("a b.txt");
        std::fs::write(&target, b"").unwrap();
        run(&found[0], std::slice::from_ref(&target)).unwrap();
        for _ in 0..50 {
            if out.exists() && std::fs::read_to_string(&out).is_ok_and(|s| !s.is_empty()) {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert_eq!(std::fs::read_to_string(&out).unwrap(), format!("{0}|{0}\n", target.display()));
        let _ = std::fs::remove_dir_all(&d);
    }
}
