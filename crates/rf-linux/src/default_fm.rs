//! "Set as default file manager" on Linux, for this user only: folders opened from other apps
//! (`xdg-open`, desktop shortcuts) and "Show in folder" in browsers and other apps land in
//! rustyfiles. Three files:
//!
//! * `rustyfiles.desktop` in `$XDG_DATA_HOME/applications` (also lists us in "Open with");
//! * `inode/directory=rustyfiles.desktop` in `$XDG_CONFIG_HOME/mimeapps.list` (and in a
//!   desktop-specific `<desktop>-mimeapps.list` that already sets folders, since that one wins).
//!   The default it replaces is kept under `[X-rustyfiles]` and put back by [`disable`];
//! * a D-Bus activation file for `org.freedesktop.FileManager1`, so "Show in folder" starts
//!   `rustyfiles --file-manager-service` (see `fm_service`) when nothing serves it.
//!
//! `classes_root` is Windows' registry root ([`CLASSES`]: the user's real folders); tests pass a
//! scratch folder instead (with `config/` and `data/` under it).
#![allow(unused)]
use std::path::{Path, PathBuf};

pub const CLASSES: &str = r"Software\Classes";
/// Archive types rustyfiles browses (Windows offers them in Default apps; unused on Linux).
pub const ARCHIVE_EXTS: [&str; 9] = [".zip", ".7z", ".rar", ".tar", ".tgz", ".txz", ".tzst", ".tbz2", ".tbz"];
/// Name under Windows' RegisteredApplications.
pub const APP_NAME: &str = "rustyfiles";
/// Our desktop entry's id.
pub const DESKTOP_ID: &str = "rustyfiles.desktop";
/// The D-Bus service rustyfiles answers as default file manager.
pub const SERVICE: &str = "org.freedesktop.FileManager1";
/// The argument D-Bus activation starts us with (no window: see `fm_service`).
pub const SERVICE_ARG: &str = "--file-manager-service";
const FOLDER: &str = "inode/directory";
const OURS: &str = "X-rustyfiles";

struct Places {
    config: PathBuf,
    data: PathBuf,
}

fn places(classes_root: &str) -> Option<Places> {
    if classes_root == CLASSES {
        return Some(Places { config: dirs::config_dir()?, data: dirs::data_dir()? });
    }
    let base = PathBuf::from(classes_root);
    Some(Places { config: base.join("config"), data: base.join("data") })
}

impl Places {
    fn desktop_file(&self) -> PathBuf {
        self.data.join("applications").join(DESKTOP_ID)
    }
    fn service_file(&self) -> PathBuf {
        self.data.join("dbus-1").join("services").join(format!("{SERVICE}.service"))
    }
    /// The user's mimeapps.list files, highest priority first: desktop-specific, then generic.
    fn mimeapps(&self) -> Vec<PathBuf> {
        let desktops = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default();
        desktops
            .split(':')
            .filter(|d| !d.is_empty())
            .map(|d| self.config.join(format!("{}-mimeapps.list", d.to_lowercase())))
            .chain([self.config.join("mimeapps.list")])
            .collect()
    }
}

// ---- mimeapps.list editing (a key file: groups of key=value lines) ----

/// The lines of `text` split into groups; the first has no header.
fn groups(text: &str) -> Vec<(Option<String>, Vec<String>)> {
    let mut out: Vec<(Option<String>, Vec<String>)> = vec![(None, Vec::new())];
    for line in text.lines() {
        let t = line.trim();
        if let Some(g) = t.strip_prefix('[').and_then(|t| t.strip_suffix(']')) {
            out.push((Some(g.to_string()), Vec::new()));
        } else {
            out.last_mut().unwrap().1.push(line.to_string());
        }
    }
    out
}

fn write_groups(gs: &[(Option<String>, Vec<String>)]) -> String {
    let mut out = String::new();
    for (name, lines) in gs {
        if let Some(n) = name {
            if !out.is_empty() && !out.ends_with("\n\n") {
                out.push('\n');
            }
            out.push_str(&format!("[{n}]\n"));
        }
        // blank lines at a group's end are rewritten as one separator above
        let end = lines.iter().rposition(|l| !l.trim().is_empty()).map_or(0, |i| i + 1);
        for l in &lines[..end] {
            out.push_str(l);
            out.push('\n');
        }
    }
    out
}

fn get(gs: &[(Option<String>, Vec<String>)], group: &str, key: &str) -> Option<String> {
    let (_, lines) = gs.iter().find(|(n, _)| n.as_deref() == Some(group))?;
    lines.iter().find_map(|l| l.split_once('=').filter(|(k, _)| k.trim() == key).map(|(_, v)| v.trim().to_string()))
}

/// Sets (`Some`) or removes (`None`) `key` in `group`, creating the group when needed.
fn set(gs: &mut Vec<(Option<String>, Vec<String>)>, group: &str, key: &str, value: Option<&str>) {
    let i = match gs.iter().position(|(n, _)| n.as_deref() == Some(group)) {
        Some(i) => i,
        None if value.is_none() => return,
        None => {
            gs.push((Some(group.to_string()), Vec::new()));
            gs.len() - 1
        }
    };
    let lines = &mut gs[i].1;
    let at = lines.iter().position(|l| l.split_once('=').is_some_and(|(k, _)| k.trim() == key));
    match (at, value) {
        (Some(at), Some(v)) => lines[at] = format!("{key}={v}"),
        (None, Some(v)) => {
            let end = lines.iter().rposition(|l| !l.trim().is_empty()).map_or(0, |i| i + 1);
            lines.insert(end, format!("{key}={v}"));
        }
        (Some(at), None) => {
            lines.remove(at);
        }
        (None, None) => {}
    }
    if lines.iter().all(|l| l.trim().is_empty()) && group == OURS {
        gs.remove(i);
    }
}

/// The first app listed as `mime`'s default in mimeapps.list `text`.
fn default_in(text: &str, mime: &str) -> Option<String> {
    let v = get(&groups(text), "Default Applications", mime)?;
    v.split(';').map(str::trim).find(|s| !s.is_empty()).map(str::to_string)
}

/// `text` with `id` as `mime`'s default; the default it replaces is remembered (once).
fn with_default(text: &str, mime: &str, id: &str) -> String {
    let mut gs = groups(text);
    let old = get(&gs, "Default Applications", mime);
    let remembered = format!("previous-{mime}");
    if let Some(old) = old.filter(|o| !o.split(';').any(|s| s.trim() == id)) {
        set(&mut gs, OURS, &remembered, Some(&old));
    }
    set(&mut gs, "Default Applications", mime, Some(&format!("{id};")));
    write_groups(&gs)
}

/// `text` with `mime`'s remembered default back (or no default of ours) when `id` is it.
fn without_default(text: &str, mime: &str, id: &str) -> String {
    let mut gs = groups(text);
    let remembered = format!("previous-{mime}");
    let previous = get(&gs, OURS, &remembered);
    if default_in(text, mime).as_deref() == Some(id) {
        set(&mut gs, "Default Applications", mime, previous.as_deref());
    }
    set(&mut gs, OURS, &remembered, None);
    write_groups(&gs)
}

// ---- the registration ----

fn desktop_entry(exe: &str) -> String {
    let exe = crate::dialogs::exec_quote(exe);
    format!(
        "[Desktop Entry]\nType=Application\nName=rustyfiles\nGenericName=File Manager\nComment=Browse files and folders\n\
         Exec={exe} %F\nIcon=system-file-manager\nTerminal=false\nCategories=System;FileTools;FileManager;\n\
         MimeType=inode/directory;\nStartupWMClass=rustyfiles\n"
    )
}

fn service_entry(exe: &str) -> String {
    format!("[D-BUS Service]\nName={SERVICE}\nExec={} {SERVICE_ARG}\n", crate::dialogs::exec_quote(exe))
}

/// Asks the session bus to read its service files again: it read them when it started, maybe
/// before our activation file (or its folder) existed, and would start another file manager.
fn reload_bus() {
    let reload = || -> zbus::Result<()> {
        let conn = zbus::blocking::Connection::session()?;
        zbus::blocking::fdo::DBusProxy::new(&conn)?.reload_config()?;
        Ok(())
    };
    if let Err(e) = reload() {
        log::warn!("D-Bus ReloadConfig: {e}");
    }
}

fn write(path: &Path, text: &str) -> anyhow::Result<()> {
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d)?;
    }
    std::fs::write(path, text)?;
    Ok(())
}

/// Makes `exe` the user's default file manager.
pub fn enable(exe: &str, classes_root: &str) -> anyhow::Result<()> {
    let pl = places(classes_root).ok_or_else(|| anyhow::anyhow!("no config folder"))?;
    write(&pl.desktop_file(), &desktop_entry(exe))?;
    write(&pl.service_file(), &service_entry(exe))?;
    if classes_root == CLASSES {
        reload_bus();
    }
    let files = pl.mimeapps();
    let generic = files.last().cloned();
    for f in files {
        let text = std::fs::read_to_string(&f).unwrap_or_default();
        // a desktop-specific file only when it already decides folders (it would win)
        if Some(&f) == generic.as_ref() || default_in(&text, FOLDER).is_some() {
            write(&f, &with_default(&text, FOLDER, DESKTOP_ID))?;
        }
    }
    Ok(())
}

/// Puts the previous default file manager back and stops answering "Show in folder".
pub fn disable(classes_root: &str) -> anyhow::Result<()> {
    let pl = places(classes_root).ok_or_else(|| anyhow::anyhow!("no config folder"))?;
    for f in pl.mimeapps() {
        let Ok(text) = std::fs::read_to_string(&f) else { continue };
        let new = without_default(&text, FOLDER, DESKTOP_ID);
        if new != text {
            write(&f, &new)?;
        }
    }
    match std::fs::remove_file(pl.service_file()) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e.into()),
        _ => {}
    }
    if classes_root == CLASSES {
        reload_bus();
    }
    // the desktop entry stays: it lists rustyfiles in "Open with" for folders
    Ok(())
}

/// Whether folders open in rustyfiles: the first of the user's mimeapps.list files that names a
/// folder app names ours.
pub fn is_enabled(classes_root: &str) -> bool {
    let Some(pl) = places(classes_root) else { return false };
    pl.mimeapps().iter().find_map(|f| std::fs::read_to_string(f).ok().and_then(|t| default_in(&t, FOLDER))).as_deref() == Some(DESKTOP_ID)
}

/// The exe our desktop entry starts (None when not enabled).
pub fn registered_exe(classes_root: &str) -> Option<PathBuf> {
    if !is_enabled(classes_root) {
        return None;
    }
    let text = std::fs::read_to_string(places(classes_root)?.desktop_file()).ok()?;
    let exec = text.lines().find_map(|l| l.strip_prefix("Exec="))?;
    let first = match exec.strip_prefix('"') {
        Some(rest) => {
            // inside quotes: \x escapes x
            let mut out = String::new();
            let mut chars = rest.chars();
            while let Some(c) = chars.next() {
                match c {
                    '\\' => out.extend(chars.next()),
                    '"' => break,
                    c => out.push(c),
                }
            }
            out
        }
        None => exec.split_whitespace().next()?.to_string(),
    };
    Some(PathBuf::from(first.replace("%%", "%")))
}

/// While we are the default, a desktop entry pointing at an exe that no longer exists (moved,
/// rebuilt elsewhere) would leave folders opening nowhere: point it at `current`. True when it
/// re-registered.
pub fn heal(current: &str, classes_root: &str) -> bool {
    match registered_exe(classes_root) {
        Some(old) if !old.exists() && Path::new(current).exists() => enable(current, classes_root).is_ok(),
        _ => false,
    }
}

/// Windows brings its archive registration up to date here; Linux has none.
pub fn add_archives(_exe: &str, _classes_root: &str) -> bool {
    false
}

/// True when the argument should be handed back to Explorer (shell namespaces, non-folders).
pub fn should_forward_to_explorer(arg: &str) -> bool {
    let a = arg.trim().trim_matches('"');
    !a.is_empty()
        && (a.starts_with("::") || a.starts_with("shell:")
            || !std::path::Path::new(a).is_dir())
}
/// Explorer opens folders through the same `Folder\shell\open` verb we take over, so an
/// argument we hand to it can come straight back (a server root like `\\nas`, a folder that
/// looked like a file). Forwards are remembered for a few seconds to break such a loop.
fn forward_log() -> std::path::PathBuf {
    std::env::temp_dir().join("rustyfiles-forward.txt")
}
fn now_ms() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis())
}
/// True when `arg` was handed to Explorer within the last few seconds.
pub fn forwarded_recently(arg: &str) -> bool {
    forwarded_within(&forward_log(), arg, now_ms(), 5_000)
}
/// Remembers that `arg` is being handed to Explorer.
pub fn note_forward(arg: &str) {
    note_in(&forward_log(), arg, now_ms());
}
fn forwarded_within(
    log: &std::path::Path,
    arg: &str,
    now: u128,
    window_ms: u128,
) -> bool {
    let Ok(text) = std::fs::read_to_string(log) else { return false };
    text.lines()
        .filter_map(|l| l.split_once('\t'))
        .any(|(t, a)| {
            a == arg
                && t.parse::<u128>().is_ok_and(|t| now.saturating_sub(t) <= window_ms)
        })
}
fn note_in(log: &std::path::Path, arg: &str, now: u128) {
    let old = std::fs::read_to_string(log).unwrap_or_default();
    let mut lines: Vec<&str> = old.lines().filter(|l| !l.is_empty()).collect();
    let keep = lines.len().saturating_sub(7);
    lines.drain(..keep);
    let mut out: String = lines.iter().map(|l| format!("{l}\n")).collect();
    out.push_str(&format!("{now}\t{arg}\n"));
    let _ = std::fs::write(log, out);
}

#[cfg(test)]
mod tests {
    use super::*;

    const DOLPHIN: &str = "[Added Associations]\nvideo/mp4=mpv.desktop;\n\n[Default Applications]\ninode/directory=org.kde.dolphin.desktop;\ntext/plain=org.gnome.TextEditor.desktop;\n";

    #[test]
    fn mimeapps_default_round_trip() {
        let on = with_default(DOLPHIN, FOLDER, DESKTOP_ID);
        assert_eq!(default_in(&on, FOLDER).as_deref(), Some(DESKTOP_ID));
        assert_eq!(default_in(&on, "text/plain").as_deref(), Some("org.gnome.TextEditor.desktop"), "other types untouched");
        assert!(on.contains("[X-rustyfiles]\nprevious-inode/directory=org.kde.dolphin.desktop;\n"), "{on}");
        // enabling twice keeps Dolphin as the one to go back to
        let twice = with_default(&on, FOLDER, DESKTOP_ID);
        assert_eq!(twice, on);
        assert_eq!(without_default(&on, FOLDER, DESKTOP_ID), DOLPHIN);
        // no previous default: ours is just removed
        let fresh = with_default("", FOLDER, DESKTOP_ID);
        assert_eq!(fresh, "[Default Applications]\ninode/directory=rustyfiles.desktop;\n");
        assert_eq!(default_in(&without_default(&fresh, FOLDER, DESKTOP_ID), FOLDER), None);
        // someone else became the default since: disabling leaves them be
        let changed = on.replace("inode/directory=rustyfiles.desktop;", "inode/directory=nautilus.desktop;");
        assert_eq!(default_in(&without_default(&changed, FOLDER, DESKTOP_ID), FOLDER).as_deref(), Some("nautilus.desktop"));
    }

    #[test]
    fn enable_and_disable_in_a_scratch_home() {
        let base = std::env::temp_dir().join(format!("rf-dfm-{}", std::process::id()));
        let root = base.to_string_lossy().into_owned();
        std::fs::create_dir_all(base.join("config")).unwrap();
        std::fs::write(base.join("config/mimeapps.list"), DOLPHIN).unwrap();
        assert!(!is_enabled(&root));
        enable("/opt/rusty files/rustyfiles", &root).unwrap();
        assert!(is_enabled(&root));
        assert_eq!(registered_exe(&root), Some(PathBuf::from("/opt/rusty files/rustyfiles")));
        let desktop = std::fs::read_to_string(base.join("data/applications/rustyfiles.desktop")).unwrap();
        assert!(desktop.contains("Exec=\"/opt/rusty files/rustyfiles\" %F\n"), "{desktop}");
        let service = std::fs::read_to_string(base.join("data/dbus-1/services/org.freedesktop.FileManager1.service")).unwrap();
        assert!(service.contains("Exec=\"/opt/rusty files/rustyfiles\" --file-manager-service\n"), "{service}");
        // the exe moved: heal points the entry at the new one
        let me = std::env::current_exe().unwrap().to_string_lossy().into_owned();
        assert!(heal(&me, &root));
        assert_eq!(registered_exe(&root), Some(PathBuf::from(&me)));
        disable(&root).unwrap();
        assert!(!is_enabled(&root));
        assert_eq!(std::fs::read_to_string(base.join("config/mimeapps.list")).unwrap(), DOLPHIN, "Dolphin is back");
        assert!(!base.join("data/dbus-1/services/org.freedesktop.FileManager1.service").exists());
        let _ = std::fs::remove_dir_all(&base);
    }
}
