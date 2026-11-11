//! "Open the folder and select this file" from other apps while rfiles is the default file
//! manager (upstream: Windows names the file afterwards, through the shell's window list). Linux:
//! other apps call `org.freedesktop.FileManager1.ShowItems`, which already passes the file
//! (`fm_service`): nothing ever arrives here.

use std::path::{Path, PathBuf};
use std::time::Duration;

/// The file a "show in folder" names for `folder`, if one comes within `wait`. Linux: none.
pub fn wait_for_select(_folder: &Path, _wait: Duration) -> Option<PathBuf> {
    None
}
