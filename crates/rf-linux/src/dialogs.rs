//! Windows' common file dialogs (IFileOpenDialog / IFileSaveDialog) and the "start with
//! Windows" Run entry (on Linux: the XDG autostart entry). Dialogs are modal to `owner`: call them on the shell STA thread.
// Generated from rf-win's public API by a one-off stub generator, then edited by hand.
// Functions return safe defaults until they get a Linux implementation.
#![allow(unused)]
use std::path::{Path, PathBuf};
use crate::wide::wide;
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
pub fn pick(owner: crate::Opaque, title: &str, kind: Pick<'_>) -> Option<PathBuf> {
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
const RUN_VALUE: &str = "rustyfiles";

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
        assert!(!starts_in(&dir, "rustyfiles"));
        set_in(&dir, "rustyfiles", Path::new("/opt/my apps/rustyfiles"), &["--background"], true).unwrap();
        assert!(starts_in(&dir, "rustyfiles"));
        let text = std::fs::read_to_string(dir.join("rustyfiles.desktop")).unwrap();
        assert!(text.contains("Exec=\"/opt/my apps/rustyfiles\" --background\n"), "{text}");
        // switched off in the desktop's settings
        std::fs::write(dir.join("rustyfiles.desktop"), text.replace("enabled=true", "enabled=false")).unwrap();
        assert!(!starts_in(&dir, "rustyfiles"));
        set_in(&dir, "rustyfiles", Path::new("/x"), &[], false).unwrap();
        assert!(!dir.join("rustyfiles.desktop").exists());
        set_in(&dir, "rustyfiles", Path::new("/x"), &[], false).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn exec_quoting() {
        assert_eq!(exec_quote("/usr/bin/rustyfiles"), "/usr/bin/rustyfiles");
        assert_eq!(exec_quote("a b"), "\"a b\"");
        assert_eq!(exec_quote("50%"), "50%%");
        assert_eq!(exec_quote("$HOME\\x\"y"), "\"\\$HOME\\\\x\\\"y\"");
    }
}
