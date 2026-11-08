//! Windows' common file dialogs (IFileOpenDialog / IFileSaveDialog) and the "start with
//! Windows" Run entry. Dialogs are modal to `owner`: call them on the shell STA thread.
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
/// Shows the dialog; `None` when cancelled.
pub fn pick(owner: crate::Opaque, title: &str, kind: Pick<'_>) -> Option<PathBuf> {
    None
}
const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const RUN_VALUE: &str = "rustyfiles";
/// Whether the app starts with Windows (HKCU Run entry, no admin rights needed).
pub fn starts_with_windows() -> bool {
    starts_with_windows_as(RUN_VALUE)
}
/// [`starts_with_windows`] for another program of the bundle (Run value `name`).
pub fn starts_with_windows_as(name: &str) -> bool {
    false
}
pub fn set_start_with_windows(exe: &Path, on: bool) -> anyhow::Result<()> {
    Err(
        anyhow::anyhow!(
            "rf-linux: `set_start_with_windows` is not implemented on Linux yet"
        )
            .into(),
    )
}
/// Run value `name` → `"exe" args…` (HKCU, no admin rights).
pub fn set_start_with_windows_as(
    name: &str,
    exe: &Path,
    args: &[&str],
    on: bool,
) -> anyhow::Result<()> {
    Err(
        anyhow::anyhow!(
            "rf-linux: `set_start_with_windows_as` is not implemented on Linux yet"
        )
            .into(),
    )
}
