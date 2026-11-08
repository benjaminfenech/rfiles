//! Network locations: shares of a server (`\\server` is not listable with FindFirstFile) and the
//! user's network shortcuts. Blocking and potentially slow (SMB timeouts): off the UI thread.
// Generated from rf-win's public API by a one-off stub generator, then edited by hand.
// Functions return safe defaults until they get a Linux implementation.
#![allow(unused)]
use std::path::{Path, PathBuf};
use crate::wide::wide;
const MAX_PREFERRED_LENGTH: u32 = u32::MAX;
const ERROR_MORE_DATA: u32 = 234;
/// `\\server` or `\\server\` (no share part) → `Some("server")`.
pub fn server_root(path: &Path) -> Option<String> {
    let s = path.to_str()?;
    let rest = s.strip_prefix(r"\\")?;
    if rest.starts_with('?') || rest.starts_with('.') {
        return None;
    }
    let rest = rest.trim_end_matches('\\');
    (!rest.is_empty() && !rest.contains('\\')).then(|| rest.to_string())
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Share {
    pub name: String,
    pub remark: String,
}
/// Disk shares of `server` (hidden `$` and admin shares excluded).
pub fn shares(server: &str) -> std::io::Result<Vec<Share>> {
    Err(
        std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "rf-linux: `shares` is not implemented on Linux yet",
        ),
    )
}
/// Folders under `%APPDATA%\Microsoft\Windows\Network Shortcuts` ("Add a network location").
/// Each is a folder holding `target.lnk`; returns (display name, target).
pub fn network_shortcuts() -> Vec<(String, PathBuf)> {
    let Some(dir) = std::env::var_os("APPDATA")
        .map(|a| PathBuf::from(a).join(r"Microsoft\Windows\Network Shortcuts")) else {
        return Vec::new();
    };
    let Ok(rd) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut out: Vec<(String, PathBuf)> = rd
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let p = e.path();
            let lnk = p.join("target.lnk");
            let target = crate::shortcut::resolve(&lnk).ok()?.target;
            if target.as_os_str().is_empty() {
                return None;
            }
            Some((e.file_name().to_string_lossy().into_owned(), target))
        })
        .collect();
    out.sort_by(|a, b| rf_core::sort::natural_cmp(&a.0, &b.0));
    out
}
/// Windows' own "Map network drive" dialog (drive letter, folder, reconnect at sign-in,
/// other credentials). Modal to `owner`; returns when it closes.
pub fn map_drive_dialog(owner: isize) {}
/// Windows' "Disconnect network drive" dialog.
pub fn disconnect_drive_dialog(owner: isize) {}
