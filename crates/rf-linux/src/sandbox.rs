//! Running inside Flatpak. The sandbox has its own `/usr`, `/tmp` and XDG folders, but
//! rustyfiles is a file manager: it works on the user's own files, trash, settings and apps.
//! So with `--filesystem=host` it uses the host's XDG folders ([`adopt_host_dirs`]) and runs
//! host programs (apps, terminals, git, ffmpeg, poppler, gio, thumbnailers) through
//! `flatpak-spawn --host` ([`tool`], [`launch`]). Outside Flatpak all of this is a no-op.

use std::collections::HashMap;
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::OnceLock;

use parking_lot::Mutex;

/// The Flatpak app id when running inside Flatpak.
pub fn flatpak_id() -> Option<&'static str> {
    static ID: OnceLock<Option<String>> = OnceLock::new();
    ID.get_or_init(|| {
        if !Path::new("/.flatpak-info").exists() {
            return None;
        }
        std::env::var("FLATPAK_ID").ok().filter(|s| !s.is_empty())
    })
    .as_deref()
}

pub fn is_flatpak() -> bool {
    flatpak_id().is_some()
}

/// Inside Flatpak: points the XDG folders back at the host's (Flatpak gives the app its own
/// under `~/.var/app`), so the trash, `mimeapps.list`, bookmarks, user folders, thumbnails and
/// rustyfiles' own settings are the same as a native install's; adds the host's and its
/// Flatpak apps' data folders (desktop entries, icons) ahead of the sandbox's; and moves
/// temporary files to a folder the host can see too (host tools write their output there).
///
/// Call first thing in `main`, before any thread starts (it sets environment variables).
pub fn adopt_host_dirs() {
    if !is_flatpak() {
        return;
    }
    let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else { return };
    let host = |var: &str, default: &str| -> OsString {
        std::env::var_os(format!("HOST_{var}")).filter(|v| !v.is_empty()).unwrap_or_else(|| home.join(default).into_os_string())
    };
    let mut set: Vec<(&str, OsString)> = vec![
        ("XDG_CONFIG_HOME", host("XDG_CONFIG_HOME", ".config")),
        ("XDG_DATA_HOME", host("XDG_DATA_HOME", ".local/share")),
        ("XDG_CACHE_HOME", host("XDG_CACHE_HOME", ".cache")),
        ("XDG_STATE_HOME", host("XDG_STATE_HOME", ".local/state")),
    ];
    let data_home = PathBuf::from(&set[1].1);
    let mut data_dirs = vec![
        data_home.join("flatpak/exports/share"),
        PathBuf::from("/var/lib/flatpak/exports/share"),
        PathBuf::from("/run/host/usr/local/share"),
        PathBuf::from("/run/host/usr/share"),
    ];
    if let Some(own) = std::env::var_os("XDG_DATA_DIRS") {
        data_dirs.extend(std::env::split_paths(&own));
    }
    if let Ok(v) = std::env::join_paths(&data_dirs) {
        set.push(("XDG_DATA_DIRS", v));
    }
    let mut config_dirs = vec![PathBuf::from("/run/host/etc/xdg")];
    if let Some(own) = std::env::var_os("XDG_CONFIG_DIRS") {
        config_dirs.extend(std::env::split_paths(&own));
    }
    if let Ok(v) = std::env::join_paths(&config_dirs) {
        set.push(("XDG_CONFIG_DIRS", v));
    }
    // $XDG_RUNTIME_DIR/app/<id> is the same folder inside and outside the sandbox
    if let (Some(run), Some(id)) = (std::env::var_os("XDG_RUNTIME_DIR"), flatpak_id()) {
        let tmp = PathBuf::from(run).join("app").join(id);
        if std::fs::create_dir_all(&tmp).is_ok() {
            set.push(("TMPDIR", tmp.into_os_string()));
        }
    }
    for (k, v) in set {
        // SAFETY: called at the start of main, before other threads exist
        unsafe { std::env::set_var(k, v) };
    }
}

/// Whether the sandbox itself has `prog` (on its `PATH`, or an absolute path inside it).
fn in_sandbox(prog: &OsStr) -> bool {
    let p = Path::new(prog);
    if p.is_absolute() {
        return p.is_file();
    }
    std::env::var_os("PATH").is_some_and(|path| std::env::split_paths(&path).any(|d| d.join(p).is_file()))
}

/// Whether the host has `prog` (asked once per program).
fn on_host(prog: &OsStr) -> bool {
    static SEEN: Mutex<Option<HashMap<OsString, bool>>> = Mutex::new(None);
    if let Some(&known) = SEEN.lock().get_or_insert_with(HashMap::new).get(prog) {
        return known;
    }
    let found = Command::new("flatpak-spawn")
        .args(["--host", "sh", "-c", "command -v -- \"$1\" >/dev/null", "sh"])
        .arg(prog)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success());
    SEEN.lock().get_or_insert_with(HashMap::new).insert(prog.to_os_string(), found);
    found
}

/// Whether `prog` can be run: on `PATH` (or an existing absolute path); inside Flatpak, in
/// the sandbox or on the host.
pub fn in_path(prog: &str) -> bool {
    let p = OsStr::new(prog);
    in_sandbox(p) || (is_flatpak() && on_host(p))
}

fn host_command(prog: &OsStr, envs: &[(&str, &str)], watch: bool) -> Command {
    let mut c = Command::new("flatpak-spawn");
    c.arg("--host");
    if watch {
        // ends the host process when flatpak-spawn goes (a timeout's kill, or rustyfiles quits)
        c.arg("--watch-bus");
    }
    for (k, v) in envs {
        c.arg(format!("--env={k}={v}"));
    }
    c.arg(prog);
    c
}

/// A command-line tool (git, ffmpeg, poppler, gio…) that rustyfiles waits for: inside
/// Flatpak the host's when it has one (the runtime's own ffmpeg, gio and gsettings see the
/// sandbox, not the user's desktop, codecs and mounts), else the sandbox's. `envs` are set
/// for it (Flatpak doesn't pass the command's own environment to the host). A program
/// neither has gives `Command::new(prog)`, so spawning fails with `NotFound` as it does
/// outside Flatpak.
pub fn tool(prog: impl AsRef<OsStr>, envs: &[(&str, &str)]) -> Command {
    let prog = prog.as_ref();
    if is_flatpak() && on_host(prog) {
        return host_command(prog, envs, true);
    }
    let mut c = Command::new(prog);
    c.envs(envs.iter().copied());
    c
}

/// An app or terminal that outlives rustyfiles: always the host's inside Flatpak.
pub fn launch(prog: impl AsRef<OsStr>) -> Command {
    let prog = prog.as_ref();
    if is_flatpak() && on_host(prog) {
        return host_command(prog, &[], false);
    }
    Command::new(prog)
}

/// The start of a desktop entry's `Exec=` that runs `exe` (rustyfiles or directlook) from the
/// host: inside Flatpak `flatpak run --command=… <id>`, else the exe's own (quoted) path.
pub fn exec_for(exe: &str) -> String {
    match flatpak_id() {
        Some(id) => {
            let name = Path::new(exe).file_name().map_or("rustyfiles".into(), |n| n.to_string_lossy().into_owned());
            format!("flatpak run --command={} {id}", crate::dialogs::exec_quote(&name))
        }
        None => crate::dialogs::exec_quote(exe),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outside_flatpak_nothing_changes() {
        assert!(!is_flatpak());
        assert_eq!(tool("git", &[("A", "1")]).get_program(), "git");
        assert!(tool("git", &[("A", "1")]).get_envs().any(|(k, v)| k == "A" && v == Some(OsStr::new("1"))));
        assert_eq!(launch("xdg-open").get_program(), "xdg-open");
        assert_eq!(exec_for("/opt/rusty files/rustyfiles"), "\"/opt/rusty files/rustyfiles\"");
        assert!(in_path("sh"));
        assert!(!in_path("rustyfiles-no-such-program"));
    }
}
