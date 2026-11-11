//! FTP browsing and transfers through WinINet (built into Windows: no extra dependency, uses
//! the system proxy settings). Locations are `ftp://[user[:pass]@]host[:port]/path` strings;
//! inside the app they travel as `PathBuf`s, so backslashes that `Path::join` adds are
//! normalized here. Blocking: call off the UI thread.
// Generated from rf-win's public API by a one-off stub generator, then edited by hand.
// Functions return safe defaults until they get a Linux implementation.
use std::path::Path;
use rf_core::Entry;
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FtpUrl {
    pub host: String,
    pub port: u16,
    pub user: Option<String>,
    pub pass: Option<String>,
    /// Always starts with '/', no trailing '/' except for the root.
    pub path: String,
}
pub fn is_ftp(p: &Path) -> bool {
    p.to_str()
        .is_some_and(|s| {
            s.len() > 6 && s.as_bytes()[..6].eq_ignore_ascii_case(b"ftp://")
        })
}
impl FtpUrl {
    pub fn parse(_s: &str) -> Option<FtpUrl> {
        None
    }
}
/// Lists a remote directory.
pub fn list(_url: &str) -> std::io::Result<Vec<Entry>> {
    Err(
        std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "rf-linux: `list` is not implemented on Linux yet",
        ),
    )
}
/// Downloads `url` (a file) to `dest`.
pub fn download(_url: &str, _dest: &Path) -> std::io::Result<()> {
    Err(
        std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "rf-linux: `download` is not implemented on Linux yet",
        ),
    )
}
/// Uploads a local file into the remote directory `dir_url`.
pub fn upload(_local: &Path, _dir_url: &str) -> std::io::Result<()> {
    Err(
        std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "rf-linux: `upload` is not implemented on Linux yet",
        ),
    )
}
