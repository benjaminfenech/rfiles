//! Windows' common file dialogs (IFileOpenDialog / IFileSaveDialog) and the "start with
//! Windows" Run entry (on Linux: the XDG autostart entry). Dialogs are modal to `owner`: call them on the shell STA thread.
// Generated from rf-win's public API by a one-off stub generator, then edited by hand.
// Functions return safe defaults until they get a Linux implementation.
use std::path::{Path, PathBuf};
pub enum Pick<'a> {
    /// Open an existing file; filters are (label, pattern) pairs like ("Ayar dosyası", "*.toml").
    File { filters: &'a [(&'a str, &'a str)] },
    /// Choose a folder.
    Folder,
    /// Save as `default_name`.
    Save { filters: &'a [(&'a str, &'a str)], default_name: &'a str },
}
/// Shows the dialog; `None` when cancelled. Linux: the desktop's file chooser through
/// xdg-desktop-portal (KDE's, GNOME's or GTK's dialog, as the desktop sets it up). Blocks.
pub fn pick(_owner: crate::Opaque, title: &str, kind: Pick<'_>) -> Option<PathBuf> {
    use ashpd::desktop::file_chooser::SelectedFiles;
    let filters = |list: &[(&str, &str)]| -> Vec<_> { list.iter().map(|(label, pats)| portal_filter(label, pats)).collect() };
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"));
    let r = futures::executor::block_on(async {
        let request = match kind {
            Pick::File { filters: f } => SelectedFiles::open_file().title(title).modal(true).filters(filters(f)).send().await,
            Pick::Folder => SelectedFiles::open_file().title(title).modal(true).directory(true).send().await,
            Pick::Save { filters: f, default_name } => {
                let mut b = SelectedFiles::save_file().title(title).modal(true).current_name(default_name).current_folder(&home)?;
                for x in filters(f) {
                    b = b.filter(x);
                }
                b.send().await
            }
        };
        request?.response()
    });
    match r {
        Ok(files) => files.uris().first().and_then(|u| uri_path(u.as_str())),
        Err(ashpd::Error::Response(_)) => None, // cancelled
        Err(e) => {
            log::warn!("file chooser portal: {e}");
            None
        }
    }
}

/// A Windows-style filter ("Pictures", "*.png;*.jpg") as a portal one; patterns match both
/// cases (the portal's globs are case-sensitive, Windows' aren't).
fn portal_filter(label: &str, patterns: &str) -> ashpd::desktop::file_chooser::FileFilter {
    let mut f = ashpd::desktop::file_chooser::FileFilter::new(label);
    for p in patterns.split(';').map(str::trim).filter(|p| !p.is_empty()) {
        f = f.glob(&any_case(p));
    }
    f
}

/// `*.png` → `*.[pP][nN][gG]`
fn any_case(glob: &str) -> String {
    glob.chars()
        .map(|c| if c.is_ascii_alphabetic() { format!("[{}{}]", c.to_ascii_lowercase(), c.to_ascii_uppercase()) } else { c.to_string() })
        .collect()
}

/// `file:///a%20b` → `/a b`.
fn uri_path(uri: &str) -> Option<PathBuf> {
    crate::fm_service::uri_to_path(uri)
}
const RUN_VALUE: &str = "rfiles";

/// Linux: "start with Windows" is the XDG autostart entry
/// (`$XDG_CONFIG_HOME/autostart/<name>.desktop`) that desktops run at login.
fn autostart_dir() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("autostart"))
}

/// Whether the app starts at login.
pub fn starts_with_windows() -> bool {
    starts_with_windows_as(RUN_VALUE)
}
/// [`starts_with_windows`] for another program of the bundle (entry `name`).
pub fn starts_with_windows_as(name: &str) -> bool {
    autostart_dir().is_some_and(|d| starts_in(&d, name))
}
pub fn set_start_with_windows(exe: &Path, on: bool) -> anyhow::Result<()> {
    set_start_with_windows_as(RUN_VALUE, exe, &[], on)
}
/// Autostart entry `name` → `exe args…`, or removes it.
pub fn set_start_with_windows_as(name: &str, exe: &Path, args: &[&str], on: bool) -> anyhow::Result<()> {
    let dir = autostart_dir().ok_or_else(|| anyhow::anyhow!("no config folder"))?;
    set_in(&dir, name, exe, args, on)
}

/// An entry the desktop's own settings switched off (`Hidden=true`, GNOME's
/// `X-GNOME-Autostart-enabled=false`) doesn't count.
fn starts_in(dir: &Path, name: &str) -> bool {
    let Ok(text) = std::fs::read_to_string(dir.join(format!("{name}.desktop"))) else { return false };
    !text.lines().map(str::trim).any(|l| l == "Hidden=true" || l == "X-GNOME-Autostart-enabled=false")
}

fn set_in(dir: &Path, name: &str, exe: &Path, args: &[&str], on: bool) -> anyhow::Result<()> {
    let file = dir.join(format!("{name}.desktop"));
    if !on {
        return match std::fs::remove_file(&file) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.into()),
            _ => Ok(()),
        };
    }
    std::fs::create_dir_all(dir)?;
    let exec = std::iter::once(crate::sandbox::exec_for(&exe.to_string_lossy())).chain(args.iter().map(|a| exec_quote(a))).collect::<Vec<_>>().join(" ");
    let text = format!("[Desktop Entry]\nType=Application\nName={name}\nExec={exec}\nX-GNOME-Autostart-enabled=true\n");
    std::fs::write(&file, text)?;
    Ok(())
}

/// The arguments of a desktop entry's `Exec=` value, unquoted (the reverse of [`exec_quote`]).
pub(crate) fn exec_args(exec: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur: Option<String> = None;
    let mut chars = exec.chars();
    while let Some(c) = chars.next() {
        match c {
            '"' => {
                let s = cur.get_or_insert_with(String::new);
                while let Some(c) = chars.next() {
                    match c {
                        '\\' => s.extend(chars.next()),
                        '"' => break,
                        c => s.push(c),
                    }
                }
            }
            c if c.is_whitespace() => out.extend(cur.take()),
            c => cur.get_or_insert_with(String::new).push(c),
        }
    }
    out.extend(cur);
    out.into_iter().map(|a| a.replace("%%", "%")).collect()
}

/// The program and arguments an autostart entry `name` runs, when there is one.
fn autostart_exec(dir: &Path, name: &str) -> Option<Vec<String>> {
    let text = std::fs::read_to_string(dir.join(format!("{name}.desktop"))).ok()?;
    let exec = text.lines().find_map(|l| l.strip_prefix("Exec="))?;
    Some(exec_args(exec)).filter(|a| !a.is_empty())
}

/// Whether `prog` (an entry's program) is an absolute path that no longer exists: the app it
/// started was moved, renamed or uninstalled. `flatpak` and other bare names never count.
fn gone(prog: &str) -> bool {
    Path::new(prog).is_absolute() && !Path::new(prog).exists()
}

/// rfiles was called rustyfiles before: its autostart entry (`old`) whose program is gone
/// becomes ours, for `exe`, with the same arguments. True when it did.
pub fn adopt_renamed_autostart(old: &str, exe: &Path) -> bool {
    autostart_dir().is_some_and(|d| adopt_in(&d, old, RUN_VALUE, exe))
}

/// An autostart entry `name` whose program is gone (it moved with a rename) starts `exe`
/// instead, with the same arguments. True when it did.
pub fn repoint_autostart(name: &str, exe: &Path) -> bool {
    autostart_dir().is_some_and(|d| adopt_in(&d, name, name, exe))
}

fn adopt_in(dir: &Path, old: &str, new: &str, exe: &Path) -> bool {
    let Some(args) = autostart_exec(dir, old) else { return false };
    if !gone(&args[0]) {
        return false;
    }
    let on = starts_in(dir, old);
    let rest: Vec<&str> = args[1..].iter().map(String::as_str).collect();
    if old != new {
        let _ = std::fs::remove_file(dir.join(format!("{old}.desktop")));
    }
    // an entry the desktop's settings switched off stays off
    set_in(dir, new, exe, &rest, on).is_ok()
}

/// One argument of a desktop entry's `Exec=` (Desktop Entry spec: quoted when it has reserved
/// characters; `"`, `` ` ``, `$` and `\` escaped inside quotes; `%` doubled).
pub(crate) fn exec_quote(arg: &str) -> String {
    let arg = arg.replace('%', "%%");
    if !arg.is_empty() && !arg.chars().any(|c| " \t\n\"'\\><~|&;$*?#()`".contains(c)) {
        return arg;
    }
    let mut out = String::from("\"");
    for c in arg.chars() {
        if matches!(c, '"' | '`' | '$' | '\\') {
            out.push('\\');
        }
        out.push(c);
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn globs_match_any_case() {
        assert_eq!(any_case("*.png"), "*.[pP][nN][gG]");
        assert_eq!(any_case("*.7z"), "*.7[zZ]");
    }

    #[test]
    fn autostart_entry() {
        let dir = std::env::temp_dir().join(format!("rf-autostart-{}", std::process::id()));
        assert!(!starts_in(&dir, "rfiles"));
        set_in(&dir, "rfiles", Path::new("/opt/my apps/rfiles"), &["--background"], true).unwrap();
        assert!(starts_in(&dir, "rfiles"));
        let text = std::fs::read_to_string(dir.join("rfiles.desktop")).unwrap();
        assert!(text.contains("Exec=\"/opt/my apps/rfiles\" --background\n"), "{text}");
        // switched off in the desktop's settings
        std::fs::write(dir.join("rfiles.desktop"), text.replace("enabled=true", "enabled=false")).unwrap();
        assert!(!starts_in(&dir, "rfiles"));
        set_in(&dir, "rfiles", Path::new("/x"), &[], false).unwrap();
        assert!(!dir.join("rfiles.desktop").exists());
        set_in(&dir, "rfiles", Path::new("/x"), &[], false).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn renamed_autostart_entries_move_over() {
        let dir = std::env::temp_dir().join(format!("rf-autostart-rename-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        set_in(&dir, "rustyfiles", Path::new("/nonexistent/rustyfiles/rustyfiles"), &["--background"], true).unwrap();
        set_in(&dir, "DirectLook", Path::new("/nonexistent/rustyfiles/directlook"), &["--background"], true).unwrap();
        assert!(adopt_in(&dir, "rustyfiles", "rfiles", Path::new("/usr/lib/rfiles/rfiles")));
        assert!(!dir.join("rustyfiles.desktop").exists());
        assert_eq!(autostart_exec(&dir, "rfiles").unwrap(), ["/usr/lib/rfiles/rfiles", "--background"]);
        assert!(adopt_in(&dir, "DirectLook", "DirectLook", Path::new("/usr/lib/rfiles/directlook")));
        assert_eq!(autostart_exec(&dir, "DirectLook").unwrap(), ["/usr/lib/rfiles/directlook", "--background"]);
        // an entry whose program still exists (a source build next to an installed copy) stays
        set_in(&dir, "other", Path::new("/bin/sh"), &[], true).unwrap();
        assert!(!adopt_in(&dir, "other", "rfiles", Path::new("/x")));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn exec_args_undo_exec_quote() {
        for a in ["/opt/my apps/rfiles", "50%", "a\"b", "plain"] {
            assert_eq!(exec_args(&format!("{} --x", exec_quote(a))), [a, "--x"]);
        }
    }

    #[test]
    fn exec_quoting() {
        assert_eq!(exec_quote("/usr/bin/rfiles"), "/usr/bin/rfiles");
        assert_eq!(exec_quote("a b"), "\"a b\"");
        assert_eq!(exec_quote("50%"), "50%%");
        assert_eq!(exec_quote("$HOME\\x\"y"), "\"\\$HOME\\\\x\\\"y\"");
    }
}
