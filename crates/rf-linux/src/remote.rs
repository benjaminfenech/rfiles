//! Network locations (`smb://server/share`, `sftp://host/path`, `ftp://…`, WebDAV…) opened as
//! ordinary folders through the desktop's network layer:
//! - KDE: kio-fuse (`org.kde.KIOFuse` on the session bus) shows any KIO URL under
//!   `/run/user/<uid>/kio-fuse-*/`; KIO asks for passwords and keeps them in KWallet.
//! - GNOME and others: gvfs (`gio mount`), under `/run/user/<uid>/gvfs/`.
//!
//! Blocking (a mount waits for the server, and maybe a password prompt): off the UI thread.

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context as _, anyhow, bail};

/// URL schemes opened through kio-fuse / gvfs.
const SCHEMES: &[&str] = &["smb", "sftp", "ftp", "ftps", "fish", "webdav", "webdavs", "dav", "davs", "nfs", "afp"];

/// How long a mount may take (a password prompt counts).
const MOUNT_TIMEOUT: Duration = Duration::from_secs(120);

/// `\\server\share\dir` and `//server/share` (how Windows and Samba write them) become
/// `smb://server/share/dir`; URLs of a known scheme are returned as they are (the scheme in
/// lower case). Anything else: None.
pub fn to_url(text: &str) -> Option<String> {
    let t = text.trim();
    if let Some((scheme, rest)) = t.split_once("://") {
        let s = scheme.to_ascii_lowercase();
        return (SCHEMES.contains(&s.as_str()) && !rest.trim_matches('/').is_empty()).then(|| format!("{s}://{rest}"));
    }
    let rest = t.strip_prefix(r"\\").or_else(|| t.strip_prefix("//"))?;
    if rest.is_empty() || rest.starts_with(['?', '.', '/', '\\']) {
        return None;
    }
    Some(format!("smb://{}", rest.replace('\\', "/")))
}

/// Whether `p` is a network location this module opens.
pub fn is_remote(p: &Path) -> bool {
    p.to_str().is_some_and(|s| to_url(s).is_some())
}

/// Opens the network location `text` (see [`to_url`]) and returns its local folder.
pub fn mount(text: &str) -> anyhow::Result<PathBuf> {
    let url = to_url(text).ok_or_else(|| anyhow!("{text} is not a network location"))?;
    let kio = match kio_fuse(&url) {
        Ok(p) => return Ok(p),
        Err(e) if e.is::<Missing>() => e,
        // kio-fuse is there and answered: its answer stands
        Err(e) => return Err(e),
    };
    match gvfs(&url) {
        Ok(p) => Ok(p),
        Err(g) => {
            log::info!("kio-fuse: {kio:#}; gvfs: {g:#}");
            match (kio.downcast_ref::<Missing>(), g.downcast_ref::<Missing>()) {
                (Some(_), Some(_)) => {
                    bail!("Opening network locations needs kio-fuse (KDE) or gvfs with its backends (GNOME); neither is installed.")
                }
                // the one that is installed has the real reason
                (Some(_), None) => Err(g),
                _ => Err(kio),
            }
        }
    }
}

/// The network location a folder inside a kio-fuse or gvfs mount stands for (without the
/// password), e.g. to pin it: the local path changes between sessions, the URL doesn't.
/// Other paths: None (without asking anyone).
pub fn remote_url(local: &Path) -> Option<String> {
    let run_dir = format!("/run/user/{}/", unsafe { libc::getuid() });
    let s = local.to_str()?;
    let rest = s.strip_prefix(&run_dir)?;
    if rest.starts_with("kio-fuse-") {
        let conn = zbus::blocking::Connection::session().ok()?;
        let reply = conn.call_method(Some("org.kde.KIOFuse"), "/org/kde/KIOFuse", Some("org.kde.KIOFuse.VFS"), "remoteUrl", &(s,)).ok()?;
        return reply.body().deserialize::<String>().ok();
    }
    if rest.starts_with("gvfs/") {
        let mut c = std::process::Command::new("gio");
        c.args(["info", "-a", "standard::name", s]);
        let out = crate::tool::run(c, Duration::from_secs(10)).ok()?;
        return String::from_utf8_lossy(&out.stdout).lines().find_map(|l| l.strip_prefix("uri: ")).map(|u| u.trim().to_string());
    }
    None
}

/// The server wants a (different) user name and password: ask, then [`with_login`].
#[derive(Debug)]
pub struct NeedsLogin;
impl std::fmt::Display for NeedsLogin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("The server refused access: wrong or missing user name and password.")
    }
}
impl std::error::Error for NeedsLogin {}

/// The user name in the location, if it has one (to pre-fill the login).
pub fn login_user(text: &str) -> Option<String> {
    let url = to_url(text)?;
    let (_, rest) = url.split_once("://")?;
    let host = rest.split('/').next()?;
    let (info, _) = host.rsplit_once('@')?;
    let user = info.split(':').next()?;
    (!user.is_empty()).then(|| decode(user))
}

/// The location with `user` and `password` in it (`smb://user:pass@server/share`), escaped.
/// kio-fuse and gvfs keep the password out of the folder path.
pub fn with_login(text: &str, user: &str, password: &str) -> Option<String> {
    let url = to_url(text)?;
    let (scheme, rest) = url.split_once("://")?;
    let (host, path) = rest.split_once('/').map_or((rest, ""), |(h, p)| (h, p));
    let host = host.rsplit_once('@').map_or(host, |(_, h)| h);
    let login = if password.is_empty() { encode(user) } else { format!("{}:{}", encode(user), encode(password)) };
    let slash = if rest.contains('/') { "/" } else { "" };
    Some(format!("{scheme}://{login}@{host}{slash}{path}"))
}

/// Percent-escapes what can't stand in a URL's user or password part.
fn encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-._~!$&'()*+,;=".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

fn decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && let Some(v) = s.get(i + 1..i + 3).and_then(|h| u8::from_str_radix(h, 16).ok())
        {
            out.push(v);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The layer isn't installed (as opposed to: it failed).
#[derive(Debug)]
struct Missing;
impl std::fmt::Display for Missing {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("not installed")
    }
}
impl std::error::Error for Missing {}

fn kio_fuse(url: &str) -> anyhow::Result<PathBuf> {
    // a long method timeout: the default 25 s isn't enough to type a password
    let conn = zbus::blocking::connection::Builder::session()?.method_timeout(MOUNT_TIMEOUT).build().context("session bus")?;
    let dbus = zbus::blocking::fdo::DBusProxy::new(&conn)?;
    let name = zbus::names::BusName::try_from("org.kde.KIOFuse")?;
    let running = dbus.name_has_owner(name.clone()).unwrap_or(false);
    if !running && !dbus.list_activatable_names()?.iter().any(|n| n.as_str() == "org.kde.KIOFuse") {
        return Err(Missing.into());
    }
    let reply = conn.call_method(Some("org.kde.KIOFuse"), "/org/kde/KIOFuse", Some("org.kde.KIOFuse.VFS"), "mountUrl", &(url,));
    let path: String = match reply {
        Ok(m) => m.body().deserialize()?,
        // KIO's password prompt has nowhere to show from kio-fuse: it reports "cancelled"
        Err(zbus::Error::MethodError(_, Some(msg), _)) if msg.contains("Operation cancelled") => return Err(NeedsLogin.into()),
        Err(zbus::Error::MethodError(_, Some(msg), _)) => bail!("{msg}"),
        Err(e) => return Err(e.into()),
    };
    let path = PathBuf::from(path);
    // kio-fuse answers before the server has; a refused login shows up on first access
    check_access(&path)?;
    Ok(path)
}

fn gvfs(url: &str) -> anyhow::Result<PathBuf> {
    use crate::tool::{NotInstalled, run};
    use std::process::Command;
    let gio = |args: &[&str]| {
        let mut c = Command::new("gio");
        c.args(args);
        c
    };
    let missing = |e: anyhow::Error| if e.is::<NotInstalled>() { Missing.into() } else { e };
    // already mounted: "mount" fails with "already mounted", which is fine
    let out = run(gio(&["mount", url]), MOUNT_TIMEOUT).map_err(missing)?;
    let err = String::from_utf8_lossy(&out.stderr);
    if !out.ok && !err.contains("lready mounted") {
        let msg = err.lines().last().unwrap_or("").trim().trim_start_matches("gio: ").to_string();
        // gio without gvfs (or without its smb/sftp backend) doesn't know the scheme at all
        if msg.contains("not supported") || msg.contains("implement mount") {
            return Err(Missing.into());
        }
        // gio asks on the terminal, which it doesn't have here
        if err.contains("Password") || err.contains("denied") || err.contains("ancelled") {
            return Err(NeedsLogin.into());
        }
        bail!("{msg}");
    }
    let info = run(gio(&["info", "-a", "standard::name", url]), Duration::from_secs(30))?;
    let text = String::from_utf8_lossy(&info.stdout);
    let path = text
        .lines()
        .find_map(|l| l.strip_prefix("local path: "))
        .ok_or_else(|| anyhow!("gvfs mounted {url} but has no local folder for it (is gvfs-fuse running?)"))?;
    let path = PathBuf::from(path.trim());
    check_access(&path)?;
    Ok(path)
}

/// Fails with the server's answer when the folder can't be read (wrong password, no access).
fn check_access(path: &Path) -> anyhow::Result<()> {
    match std::fs::read_dir(path) {
        Ok(_) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied || e.raw_os_error() == Some(libc::EPERM) => Err(NeedsLogin.into()),
        Err(e) => bail!("{e}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls() {
        assert_eq!(to_url(r"\\nas\media\Films").as_deref(), Some("smb://nas/media/Films"));
        assert_eq!(to_url("//10.0.0.9/media").as_deref(), Some("smb://10.0.0.9/media"));
        assert_eq!(to_url("SMB://nas/").as_deref(), Some("smb://nas/"));
        assert_eq!(to_url("sftp://me@host:2222/srv").as_deref(), Some("sftp://me@host:2222/srv"));
        assert_eq!(to_url("ftp://ftp.example.org/pub").as_deref(), Some("ftp://ftp.example.org/pub"));
        assert_eq!(to_url("smb://"), None);
        assert_eq!(to_url("https://example.org"), None);
        assert_eq!(to_url("/home/me"), None);
        assert_eq!(to_url(r"\\?\C:\x"), None);
        assert!(is_remote(Path::new("smb://nas/x")));
        assert!(!is_remote(Path::new("/mnt/x")));
    }

    #[test]
    fn logins() {
        assert_eq!(with_login("smb://nas/media/x", "me", "p@ss:/1").as_deref(), Some("smb://me:p%40ss%3A%2F1@nas/media/x"));
        assert_eq!(with_login("smb://old:x@nas/media", "new", "").as_deref(), Some("smb://new@nas/media"));
        assert_eq!(with_login(r"\\nas", "me", "pw").as_deref(), Some("smb://me:pw@nas"));
        assert_eq!(login_user("sftp://jo%20e@host/x").as_deref(), Some("jo e"));
        assert_eq!(login_user("smb://nas/x"), None);
    }
}
