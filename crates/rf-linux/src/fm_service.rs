//! `org.freedesktop.FileManager1` on the session bus: what browsers and other apps call for
//! "Show in folder" (`ShowItems`), "Open folder" (`ShowFolders`) and "Properties"
//! (`ShowItemProperties`).
//!
//! D-Bus starts `rustyfiles --file-manager-service` for it while rustyfiles is the default
//! file manager (see `default_fm`). That process loads no UI: each call starts
//! `rustyfiles <folder>` or `rustyfiles /select,<file>`, which opens a window or hands the
//! request to the running one, exactly like a second launch. It exits when idle; D-Bus starts
//! it again on the next call.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::default_fm::SERVICE;

const OBJECT: &str = "/org/freedesktop/FileManager1";
/// The service quits after this long without a call.
const IDLE: Duration = Duration::from_secs(60);

/// A `file://` URI as a path (percent-decoded; the host part, if any, ignored). Other schemes
/// (trash:, smb:) aren't local paths: None. A plain absolute path is taken as it is.
pub fn uri_to_path(uri: &str) -> Option<PathBuf> {
    use std::os::unix::ffi::OsStringExt;
    if uri.starts_with('/') {
        return Some(PathBuf::from(uri));
    }
    let rest = uri.strip_prefix("file://")?;
    // file://host/path → /path
    let path = &rest[rest.find('/')?..];
    let b = path.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && let Some(v) = path.get(i + 1..i + 3).and_then(|h| u8::from_str_radix(h, 16).ok())
        {
            out.push(v);
            i += 3;
            continue;
        }
        out.push(b[i]);
        i += 1;
    }
    Some(PathBuf::from(std::ffi::OsString::from_vec(out)))
}

/// The rustyfiles launches for a call: one per folder. `select` shows each item selected in its
/// folder (`/select,<item>`); otherwise the folders themselves open.
pub fn launches(uris: &[String], select: bool) -> Vec<String> {
    let mut seen: Vec<PathBuf> = Vec::new();
    let mut out = Vec::new();
    for p in uris.iter().filter_map(|u| uri_to_path(u)) {
        let folder = if select { p.parent().map(Path::to_path_buf).unwrap_or_else(|| p.clone()) } else { p.clone() };
        if seen.contains(&folder) {
            continue;
        }
        seen.push(folder);
        let arg = if select && p.parent().is_some() { format!("/select,{}", p.display()) } else { p.display().to_string() };
        out.push(arg);
    }
    out
}

struct FileManager {
    exe: PathBuf,
    last_call: Arc<Mutex<Instant>>,
}

impl FileManager {
    fn open(&self, uris: &[String], select: bool, startup_id: &str) {
        *self.last_call.lock().unwrap_or_else(|e| e.into_inner()) = Instant::now();
        for arg in launches(uris, select) {
            let mut cmd = std::process::Command::new(&self.exe);
            cmd.arg(&arg);
            // lets the compositor give the new window focus (Wayland activation / X11 startup)
            if !startup_id.is_empty() {
                cmd.env("XDG_ACTIVATION_TOKEN", startup_id).env("DESKTOP_STARTUP_ID", startup_id);
            }
            if let Err(e) = crate::shell::spawn_detached(&mut cmd) {
                log::warn!("FileManager1: starting {}: {e}", self.exe.display());
            }
        }
    }
}

#[zbus::interface(name = "org.freedesktop.FileManager1")]
impl FileManager {
    fn show_folders(&self, uris: Vec<String>, startup_id: String) {
        self.open(&uris, false, &startup_id);
    }
    fn show_items(&self, uris: Vec<String>, startup_id: String) {
        self.open(&uris, true, &startup_id);
    }
    /// rustyfiles shows the item; its Properties are a keystroke away (Alt+Enter).
    fn show_item_properties(&self, uris: Vec<String>, startup_id: String) {
        self.open(&uris, true, &startup_id);
    }
}

/// Serves `org.freedesktop.FileManager1` until idle, starting `exe` for each call. Fails when
/// another file manager holds the name.
pub fn run(exe: &Path) -> anyhow::Result<()> {
    let last_call = Arc::new(Mutex::new(Instant::now()));
    let fm = FileManager { exe: exe.to_path_buf(), last_call: last_call.clone() };
    let _conn = zbus::blocking::connection::Builder::session()?.name(SERVICE)?.serve_at(OBJECT, fm)?.build()?;
    log::info!("serving {SERVICE}");
    loop {
        std::thread::sleep(Duration::from_secs(2));
        if last_call.lock().unwrap_or_else(|e| e.into_inner()).elapsed() > IDLE {
            return Ok(());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uris_become_paths() {
        assert_eq!(uri_to_path("file:///home/me/My%20File.txt"), Some(PathBuf::from("/home/me/My File.txt")));
        assert_eq!(uri_to_path("file://laptop/etc/hosts"), Some(PathBuf::from("/etc/hosts")));
        assert_eq!(uri_to_path("/tmp/x"), Some(PathBuf::from("/tmp/x")));
        assert_eq!(uri_to_path("file:///a%2"), Some(PathBuf::from("/a%2")));
        assert_eq!(uri_to_path("trash:///x"), None);
        // non-UTF-8 names survive
        use std::os::unix::ffi::OsStrExt;
        assert_eq!(uri_to_path("file:///caf%E9").unwrap().as_os_str().as_bytes(), b"/caf\xe9");
    }

    #[test]
    fn one_launch_per_folder() {
        let uris = ["file:///d/a.txt", "file:///d/b.txt", "file:///e/c.txt", "smb://nas/x"].map(String::from);
        assert_eq!(launches(&uris, true), ["/select,/d/a.txt", "/select,/e/c.txt"]);
        let dirs = ["file:///d", "file:///d", "file:///e"].map(String::from);
        assert_eq!(launches(&dirs, false), ["/d", "/e"]);
    }
}
