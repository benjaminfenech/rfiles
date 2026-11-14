//! Volume details and safe removal. Blocking (disks can stall): call off the UI thread.
// Generated from rf-win's public API by a one-off stub generator, then edited by hand.
// Functions return safe defaults until they get a Linux implementation.
use std::path::{Path, PathBuf};
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VolumeInfo {
    pub root: PathBuf,
    pub label: String,
    pub filesystem: String,
    pub serial: u32,
    pub total: u64,
    pub free: u64,
    /// Turkish, e.g. "Yerel Disk".
    pub drive_type: String,
}
impl VolumeInfo {
    /// "A1B2-C3D4", as shown by `vol`.
    pub fn serial_string(&self) -> String {
        format!("{:04X}-{:04X}", self.serial >> 16, self.serial & 0xFFFF)
    }
}
/// Turkish name for a `GetDriveTypeW` result.
pub fn drive_type_name(t: u32) -> &'static str {
    match t {
        2 => "Removable Disk",
        3 => "Local Disk",
        4 => "Network Drive",
        5 => "Optical Drive (CD/DVD)",
        6 => "RAM Disk",
        1 => "Not Connected",
        _ => "Unknown",
    }
}
/// Details of the volume at `root` (e.g. `C:\`). Size fields stay 0 when the media is missing.
pub fn volume_info(_root: &Path) -> anyhow::Result<VolumeInfo> {
    Err(
        anyhow::anyhow!("rf-linux: `volume_info` is not implemented on Linux yet").into(),
    )
}
