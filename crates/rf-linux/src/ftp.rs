//! FTP browsing and transfers through WinINet (built into Windows: no extra dependency, uses
//! the system proxy settings). Locations are `ftp://[user[:pass]@]host[:port]/path` strings;
//! inside the app they travel as `PathBuf`s, so backslashes that `Path::join` adds are
//! normalized here. Blocking: call off the UI thread.
// Generated from rf-win's public API by a one-off stub generator, then edited by hand.
// Functions return safe defaults until they get a Linux implementation.
#![allow(unused)]
use std::path::Path;
use rf_core::Entry;
use crate::wide::wide;
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
    pub fn parse(s: &str) -> Option<FtpUrl> {
        None
    }
}
struct Handle(*mut core::ffi::c_void);
impl Drop for Handle {
    fn drop(&mut self) {}
}
/// WinINet codes live in wininet.dll's message table, which `io::Error` cannot format.
fn wininet_message(code: i32) -> Option<&'static str> {
    Some(
        match code {
            12002 => "zaman aşımı",
            12007 => "sunucu adı çözülemedi",
            12013 => "kullanıcı adı yanlış",
            12014 => "parola yanlış",
            12015 => "oturum açılamadı",
            12029 => "sunucuya bağlanılamadı",
            12030 => "bağlantı kesildi",
            12031 => "bağlantı sunucu tarafından sıfırlandı",
            12003 => "sunucu isteği reddetti",
            12111 => "FTP oturumu iptal edildi",
            _ => return None,
        },
    )
}
fn describe(e: std::io::Error) -> String {
    match e.raw_os_error().and_then(wininet_message) {
        Some(m) => m.to_string(),
        None => e.to_string(),
    }
}
fn last_err(what: &str) -> std::io::Error {
    let e = std::io::Error::last_os_error();
    std::io::Error::new(e.kind(), format!("{what}: {}", describe(e)))
}
/// (session, connection) — the session must outlive the connection; keep both.
fn connect(u: &FtpUrl) -> std::io::Result<(Handle, Handle)> {
    Err(
        std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "rf-linux: `connect` is not implemented on Linux yet",
        ),
    )
}
/// Lists a remote directory.
pub fn list(url: &str) -> std::io::Result<Vec<Entry>> {
    Err(
        std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "rf-linux: `list` is not implemented on Linux yet",
        ),
    )
}
/// Downloads `url` (a file) to `dest`.
pub fn download(url: &str, dest: &Path) -> std::io::Result<()> {
    Err(
        std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "rf-linux: `download` is not implemented on Linux yet",
        ),
    )
}
/// Uploads a local file into the remote directory `dir_url`.
pub fn upload(local: &Path, dir_url: &str) -> std::io::Result<()> {
    Err(
        std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "rf-linux: `upload` is not implemented on Linux yet",
        ),
    )
}
