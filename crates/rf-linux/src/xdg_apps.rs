//! Installed applications and which ones open what, per the freedesktop.org Desktop Entry and
//! MIME Applications Associations specifications:
//!
//! * apps are the `.desktop` files in `applications/` under every data dir (the user's own
//!   win over the system's, by desktop file ID);
//! * defaults and extra associations come from `mimeapps.list` files (the user's first, then
//!   desktop-specific ones, then system ones);
//! * a type's parents count too (`text/rust` is a `text/plain`, so text editors are offered).
//!
//! Read once per run: an app installed while rfiles runs shows up after a restart.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use crate::xdg_icons::{data_dirs, mime_for_ext};

#[derive(Debug, Clone)]
pub struct App {
    /// Desktop file ID (`org.gnome.TextEditor.desktop`).
    pub id: String,
    /// Display name, in the user's language when the file has it.
    pub name: String,
    pub exec: String,
    /// Icon name, or an absolute path.
    pub icon: Option<String>,
    pub mime_types: Vec<String>,
    pub terminal: bool,
    /// `Categories` (`System;TerminalEmulator;`).
    pub categories: Vec<String>,
    /// Kept out of menus unless it is a default.
    pub no_display: bool,
    pub path: PathBuf,
}

impl App {
    /// Whether its `Exec` line takes files or URLs.
    pub fn takes_files(&self) -> bool {
        ["%f", "%F", "%u", "%U"].iter().any(|c| self.exec.contains(c))
    }
}

// ---------------------------------------------------------------------------------------------
// .desktop files
// ---------------------------------------------------------------------------------------------

/// The `[Desktop Entry]` group of a desktop file as key → value (string escapes resolved).
fn desktop_entry(text: &str) -> HashMap<String, String> {
    let mut out = HashMap::new();
    let mut in_group = false;
    for line in text.lines().map(str::trim) {
        if line.starts_with('[') {
            in_group = line == "[Desktop Entry]";
        } else if in_group && !line.starts_with('#') {
            if let Some((k, v)) = line.split_once('=') {
                out.insert(k.trim().to_string(), unescape_value(v.trim()));
            }
        }
    }
    out
}

/// Desktop Entry string escapes: `\s \n \t \r \\`.
pub(crate) fn unescape_value(v: &str) -> String {
    let mut out = String::with_capacity(v.len());
    let mut chars = v.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('s') => out.push(' '),
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('r') => out.push('\r'),
            Some('\\') => out.push('\\'),
            // other escapes (`\"`, `\$`…) belong to the Exec quoting rules: keep them
            Some(o) => {
                out.push('\\');
                out.push(o);
            }
            None => out.push('\\'),
        }
    }
    out
}

/// `Name[en_GB]`, `Name[en]`, then `Name`, for the user's `LC_MESSAGES` / `LANG`.
pub(crate) fn localized(e: &HashMap<String, String>, key: &str) -> Option<String> {
    let lang = ["LC_ALL", "LC_MESSAGES", "LANG"].iter().find_map(|v| std::env::var(v).ok().filter(|s| !s.is_empty())).unwrap_or_default();
    let base = lang.split(['.', '@']).next().unwrap_or("");
    let short = base.split('_').next().unwrap_or("");
    [format!("{key}[{base}]"), format!("{key}[{short}]"), key.to_string()].iter().find_map(|k| e.get(k).filter(|v| !v.is_empty()).cloned())
}

fn in_path(prog: &str) -> bool {
    crate::sandbox::in_path(prog)
}

fn parse_app(id: String, path: &Path) -> Option<App> {
    let e = desktop_entry(&std::fs::read_to_string(path).ok()?);
    if e.get("Type").map(String::as_str) != Some("Application") || e.get("Hidden").is_some_and(|v| v == "true") {
        return None;
    }
    if let Some(t) = e.get("TryExec") {
        if !in_path(t) {
            return None;
        }
    }
    Some(App {
        id,
        name: localized(&e, "Name")?,
        exec: e.get("Exec")?.clone(),
        icon: e.get("Icon").filter(|s| !s.is_empty()).cloned(),
        mime_types: e.get("MimeType").map(|m| m.split(';').filter(|s| !s.is_empty()).map(String::from).collect()).unwrap_or_default(),
        terminal: e.get("Terminal").is_some_and(|v| v == "true"),
        categories: e.get("Categories").map(|m| m.split(';').filter(|s| !s.is_empty()).map(String::from).collect()).unwrap_or_default(),
        no_display: e.get("NoDisplay").is_some_and(|v| v == "true"),
        path: path.to_path_buf(),
    })
}

/// Every installed app by desktop file ID; earlier data dirs win.
fn apps() -> &'static HashMap<String, App> {
    static APPS: OnceLock<HashMap<String, App>> = OnceLock::new();
    APPS.get_or_init(|| {
        let mut out = HashMap::new();
        for dir in data_dirs() {
            let root = dir.join("applications");
            let mut stack = vec![root.clone()];
            while let Some(d) = stack.pop() {
                for e in std::fs::read_dir(&d).into_iter().flatten().flatten() {
                    let p = e.path();
                    if p.is_dir() {
                        stack.push(p);
                    } else if p.extension().is_some_and(|x| x == "desktop") {
                        // the ID: path below applications/, '/' → '-'
                        let Ok(rel) = p.strip_prefix(&root) else { continue };
                        let id = rel.to_string_lossy().replace('/', "-");
                        if out.contains_key(&id) {
                            continue;
                        }
                        // a user file that hides a system app still claims its ID
                        match parse_app(id.clone(), &p) {
                            Some(app) => {
                                out.insert(id, app);
                            }
                            None => {
                                out.insert(id, App { id: String::new(), name: String::new(), exec: String::new(), icon: None, mime_types: Vec::new(), terminal: false, categories: Vec::new(), no_display: true, path: p });
                            }
                        }
                    }
                }
            }
        }
        out.retain(|_, a| !a.id.is_empty());
        out
    })
}

// ---------------------------------------------------------------------------------------------
// mimeapps.list
// ---------------------------------------------------------------------------------------------

#[derive(Default)]
struct Assoc {
    /// type → defaults, in priority order (all files)
    defaults: HashMap<String, Vec<String>>,
    added: HashMap<String, Vec<String>>,
    removed: HashMap<String, HashSet<String>>,
}

/// The `mimeapps.list` files in the order the spec gives (highest priority first).
fn mimeapps_files() -> Vec<PathBuf> {
    let desktops: Vec<String> = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default().split(':').filter(|s| !s.is_empty()).map(str::to_lowercase).collect();
    let config_home = dirs::config_dir();
    let config_dirs: Vec<PathBuf> = std::env::var("XDG_CONFIG_DIRS").ok().filter(|s| !s.is_empty()).map(|s| std::env::split_paths(&s).collect()).unwrap_or_else(|| vec![PathBuf::from("/etc/xdg")]);
    let mut dirs_in_order: Vec<PathBuf> = config_home.into_iter().chain(config_dirs).collect();
    dirs_in_order.extend(data_dirs().into_iter().map(|d| d.join("applications")));
    let mut out = Vec::new();
    for d in dirs_in_order {
        for de in &desktops {
            out.push(d.join(format!("{de}-mimeapps.list")));
        }
        out.push(d.join("mimeapps.list"));
    }
    out
}

fn assoc() -> &'static Assoc {
    static A: OnceLock<Assoc> = OnceLock::new();
    A.get_or_init(|| {
        let mut a = Assoc::default();
        for f in mimeapps_files() {
            let Ok(text) = std::fs::read_to_string(&f) else { continue };
            let mut group = String::new();
            for line in text.lines().map(str::trim) {
                if let Some(g) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
                    group = g.to_string();
                    continue;
                }
                let Some((ty, list)) = line.split_once('=') else { continue };
                let ids = list.split(';').map(str::trim).filter(|s| !s.is_empty()).map(String::from);
                match group.as_str() {
                    "Default Applications" => a.defaults.entry(ty.trim().into()).or_default().extend(ids),
                    "Added Associations" => a.added.entry(ty.trim().into()).or_default().extend(ids),
                    "Removed Associations" => a.removed.entry(ty.trim().into()).or_default().extend(ids),
                    _ => {}
                }
            }
        }
        a
    })
}

// ---------------------------------------------------------------------------------------------
// Types and lookups
// ---------------------------------------------------------------------------------------------

/// A type and its ancestors, nearest first (`text/rust` → `text/plain` →
/// `application/octet-stream`); `text/*` falls back to `text/plain`.
pub fn with_parents(mime: &str) -> Vec<String> {
    static PARENTS: OnceLock<HashMap<String, Vec<String>>> = OnceLock::new();
    let parents = PARENTS.get_or_init(|| {
        let mut m: HashMap<String, Vec<String>> = HashMap::new();
        for d in data_dirs().iter().rev() {
            if let Ok(text) = std::fs::read_to_string(d.join("mime/subclasses")) {
                for line in text.lines() {
                    if let Some((child, parent)) = line.split_once(' ') {
                        m.entry(child.into()).or_default().push(parent.into());
                    }
                }
            }
        }
        m
    });
    let mut out = vec![mime.to_string()];
    let mut i = 0;
    while i < out.len() && out.len() < 16 {
        for p in parents.get(&out[i]).into_iter().flatten() {
            if !out.contains(p) {
                out.push(p.clone());
            }
        }
        i += 1;
    }
    if mime.starts_with("text/") && !out.iter().any(|t| t == "text/plain") {
        out.push("text/plain".into());
    }
    out
}

/// The type of a file or folder (`inode/directory` for folders), by extension.
pub fn mime_for_path(path: &Path) -> String {
    if std::fs::metadata(path).is_ok_and(|m| m.is_dir()) {
        return "inode/directory".into();
    }
    let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    mime_for_ext(&ext).unwrap_or("application/octet-stream").to_string()
}

/// The user's default app for `mime` (or its nearest ancestor that has one).
pub fn default_app(mime: &str) -> Option<&'static App> {
    let (a, all) = (assoc(), apps());
    for ty in with_parents(mime) {
        if let Some(app) = a.defaults.get(&ty).into_iter().flatten().find_map(|id| all.get(id)) {
            return Some(app);
        }
        // no default set: the first app that says it handles it (and isn't hidden)
        if let Some(app) = handlers_of(&ty).into_iter().next() {
            return Some(app);
        }
    }
    None
}

/// Apps associated with exactly `mime`: added associations, then declared handlers, minus
/// removed ones.
fn handlers_of(mime: &str) -> Vec<&'static App> {
    let (a, all) = (assoc(), apps());
    let removed = a.removed.get(mime);
    let mut out: Vec<&App> = a.added.get(mime).into_iter().flatten().filter_map(|id| all.get(id)).collect();
    let mut declared: Vec<&App> = all.values().filter(|app| !app.no_display && app.mime_types.iter().any(|t| t == mime)).collect();
    declared.sort_by_key(|app| app.name.to_lowercase());
    out.extend(declared);
    out.retain(|app| removed.is_none_or(|r| !r.contains(&app.id)));
    out
}

/// Apps that can open a `mime` file: the default first, then the type's own handlers, then
/// those of its parent types. Each once. `application/octet-stream` alone matches nothing
/// useful, so it isn't followed.
pub fn apps_for(mime: &str) -> Vec<&'static App> {
    let mut out: Vec<&App> = default_app(mime).into_iter().collect();
    for ty in with_parents(mime) {
        if ty == "application/octet-stream" && ty != mime {
            continue;
        }
        for app in handlers_of(&ty) {
            if !out.iter().any(|a| a.id == app.id) {
                out.push(app);
            }
        }
    }
    out
}

/// Every visible app, whether or not it opens files (the quick search's "Apps"), by name.
pub(crate) fn launchable() -> Vec<&'static App> {
    let mut v: Vec<&App> = apps().values().filter(|a| !a.no_display).collect();
    v.sort_by_key(|a| a.name.to_lowercase());
    v
}

/// The app with desktop file ID `id`.
pub(crate) fn by_id(id: &str) -> Option<&'static App> {
    apps().get(id)
}

/// Every visible app that opens files, by name (for "Choose another app").
/// Installed terminal emulators (`Categories=TerminalEmulator`), by name.
pub fn terminals() -> Vec<&'static App> {
    let mut v: Vec<&App> = apps().values().filter(|a| !a.no_display && a.categories.iter().any(|c| c == "TerminalEmulator")).collect();
    v.sort_by_key(|a| a.name.to_lowercase());
    v
}

/// The program an app's `Exec` line runs (`kitty`, `/usr/bin/foot`), without `env VAR=x`.
pub fn program(app: &App) -> Option<String> {
    let words = exec_words(&app.exec);
    let mut it = words.into_iter().peekable();
    if it.peek().is_some_and(|w| w == "env") {
        it.next();
        while it.peek().is_some_and(|w| w.contains('=')) {
            it.next();
        }
    }
    it.next()
}

pub fn all_apps() -> Vec<&'static App> {
    let mut v: Vec<&App> = apps().values().filter(|a| !a.no_display && a.takes_files()).collect();
    v.sort_by_key(|a| a.name.to_lowercase());
    v
}

// ---------------------------------------------------------------------------------------------
// Launching
// ---------------------------------------------------------------------------------------------

/// Splits an `Exec` value into words per the spec's quoting rules (double quotes; `\"`, `\``,
/// `\$`, `\\` inside them).
fn exec_words(exec: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut cur = String::new();
    let (mut quoted, mut started) = (false, false);
    let mut chars = exec.chars();
    while let Some(c) = chars.next() {
        match c {
            '"' => {
                quoted = !quoted;
                started = true;
            }
            '\\' if quoted => {
                if let Some(n) = chars.next() {
                    cur.push(n);
                }
            }
            c if c.is_whitespace() && !quoted => {
                if started {
                    words.push(std::mem::take(&mut cur));
                    started = false;
                }
            }
            c => {
                cur.push(c);
                started = true;
            }
        }
    }
    if started {
        words.push(cur);
    }
    words
}

/// [`exec_words`], for other modules here.
pub(crate) fn exec_words_pub(exec: &str) -> Vec<String> {
    exec_words(exec)
}

/// The command lines to run `app` on `files`: one per file when it takes a single file (`%f`,
/// `%u`), else one for all. Files are appended when the line has no file code.
pub fn command_lines(app: &App, files: &[PathBuf]) -> Vec<Vec<String>> {
    // URIs need absolute paths; apps get the same
    let files: Vec<PathBuf> = files.iter().map(|f| std::path::absolute(f).unwrap_or_else(|_| f.clone())).collect();
    let files = &files[..];
    let words = exec_words(&app.exec);
    let single = words.iter().any(|w| w.contains("%f") || w.contains("%u"));
    let multi = words.iter().any(|w| w.contains("%F") || w.contains("%U"));
    let uri = |p: &PathBuf| crate::xdg_thumbs::file_uri(p);
    let expand = |batch: &[PathBuf]| -> Vec<String> {
        let mut out = Vec::new();
        for w in &words {
            match w.as_str() {
                "%F" => out.extend(batch.iter().map(|p| p.to_string_lossy().into_owned())),
                "%U" => out.extend(batch.iter().map(uri)),
                "%i" => {
                    if let Some(i) = &app.icon {
                        out.push("--icon".into());
                        out.push(i.clone());
                    }
                }
                _ => {
                    let mut s = String::new();
                    let mut chars = w.chars();
                    while let Some(c) = chars.next() {
                        if c != '%' {
                            s.push(c);
                            continue;
                        }
                        match chars.next() {
                            Some('f') => s.push_str(&batch.first().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default()),
                            Some('u') => s.push_str(&batch.first().map(uri).unwrap_or_default()),
                            Some('c') => s.push_str(&app.name),
                            Some('k') => s.push_str(&app.path.to_string_lossy()),
                            Some('%') => s.push('%'),
                            // deprecated / unknown codes are dropped
                            _ => {}
                        }
                    }
                    // a word that was only a dropped code disappears
                    if !s.is_empty() || !w.starts_with('%') {
                        out.push(s);
                    }
                }
            }
        }
        if !single && !multi {
            out.extend(batch.iter().map(|p| p.to_string_lossy().into_owned()));
        }
        out
    };
    let lines: Vec<Vec<String>> = if single && !multi && files.len() > 1 { files.iter().map(|f| expand(std::slice::from_ref(f))).collect() } else { vec![expand(files)] };
    if !app.terminal {
        return lines;
    }
    // Terminal=true: run inside the user's terminal
    let term = std::env::var("TERMINAL").ok().filter(|t| !t.is_empty()).or_else(|| {
        ["x-terminal-emulator", "kgx", "gnome-terminal", "konsole", "ptyxis", "kitty", "alacritty", "foot", "wezterm", "xterm"].iter().find(|t| in_path(t)).map(|t| t.to_string())
    });
    match term {
        Some(t) => lines
            .into_iter()
            .map(|l| {
                let mut v = vec![t.clone()];
                // kitty and foot take the command as is; gnome-terminal wants "--"; the rest "-e"
                match t.rsplit('/').next().unwrap_or(&t) {
                    "kitty" | "foot" => {}
                    "gnome-terminal" | "kgx" | "ptyxis" => v.push("--".into()),
                    _ => v.push("-e".into()),
                }
                v.extend(l);
                v
            })
            .collect(),
        None => lines,
    }
}

/// Starts `app` on `files` (detached).
pub fn launch(app: &App, files: &[PathBuf]) -> anyhow::Result<()> {
    for line in command_lines(app, files) {
        let Some((prog, args)) = line.split_first() else { continue };
        crate::shell::spawn_detached(crate::sandbox::launch(prog).args(args)).map_err(|e| anyhow::anyhow!("{}: {e}", app.name))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app(exec: &str, terminal: bool) -> App {
        App { id: "x.desktop".into(), name: "X".into(), exec: exec.into(), icon: Some("x-icon".into()), mime_types: vec![], terminal, categories: vec![], no_display: false, path: "/usr/share/applications/x.desktop".into() }
    }

    #[test]
    fn exec_quoting_and_codes() {
        assert_eq!(exec_words(r#"sh -c "echo \"hi\" \$HOME" %f"#), ["sh", "-c", "echo \"hi\" $HOME", "%f"]);
        let files = vec![PathBuf::from("/a b/1.txt"), PathBuf::from("/c/2.txt")];
        // %f: once per file
        assert_eq!(command_lines(&app("ed %f", false), &files), vec![vec!["ed", "/a b/1.txt"], vec!["ed", "/c/2.txt"]]);
        // %U: all at once, as URIs
        assert_eq!(command_lines(&app("viewer %U", false), &files), vec![vec!["viewer", "file:///a%20b/1.txt", "file:///c/2.txt"]]);
        // %i, %c, %%, deprecated %d dropped, no file code: files appended
        assert_eq!(command_lines(&app("tool %i --title=%c 100%% %d", false), &files[..1]), vec![vec!["tool", "--icon", "x-icon", "--title=X", "100%", "/a b/1.txt"]]);
        assert_eq!(unescape_value(r"a\sb\\c"), r"a b\c");
    }

    #[test]
    fn terminal_apps_run_in_a_terminal() {
        let lines = command_lines(&app("vim %F", true), &[PathBuf::from("/x")]);
        let l = &lines[0];
        // whatever terminal is found (if any), the command comes last
        assert_eq!(&l[l.len() - 2..], ["vim", "/x"]);
    }

    #[test]
    fn localized_names() {
        let e = desktop_entry("[Desktop Entry]\nName=Files\nName[tr]=Dosyalar\nName[en_GB]=Files (UK)\n[Other]\nName=No\n");
        assert_eq!(e.get("Name").map(String::as_str), Some("Files"));
        assert!(!e.contains_key("Other"));
        assert_eq!(e.get("Name[tr]").map(String::as_str), Some("Dosyalar"));
    }

    #[test]
    fn types_have_parents() {
        if !Path::new("/usr/share/mime/subclasses").exists() {
            return;
        }
        let p = with_parents("text/rust");
        assert_eq!(p[0], "text/rust");
        assert!(p.contains(&"text/plain".to_string()), "{p:?}");
    }
}
