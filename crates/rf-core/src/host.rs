//! Inside Flatpak: whether the host system has a program, so tools run there when they can
//! (the user's own ffmpeg, git, poppler… with their codecs and settings). Shared by rf-linux's
//! `sandbox` and rf-player.

use std::collections::HashMap;
use std::ffi::{OsStr, OsString};
use std::process::{Command, Stdio};
use std::sync::Mutex;

/// Whether the host has `prog` on its `PATH` (asked through `flatpak-spawn` once per program).
/// Only meaningful inside Flatpak.
pub fn has(prog: impl AsRef<OsStr>) -> bool {
    static SEEN: Mutex<Option<HashMap<OsString, bool>>> = Mutex::new(None);
    let prog = prog.as_ref();
    if let Some(&known) = SEEN.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_with(HashMap::new).get(prog) {
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
    SEEN.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_with(HashMap::new).insert(prog.to_os_string(), found);
    found
}
