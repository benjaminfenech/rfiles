//! A folder's own icon (Windows' desktop.ini), for the Properties window's Customize tab,
//! which Linux doesn't show: no folder has one.

use std::path::Path;

pub fn folder_icon(_dir: &Path) -> Option<(String, i32)> {
    None
}

pub fn set_folder_icon(_dir: &Path, _icon: Option<(&str, i32)>) -> anyhow::Result<()> {
    anyhow::bail!("folder icons (desktop.ini) are a Windows feature")
}
