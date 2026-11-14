//! Sharing a folder on the network through Samba's user shares (`net usershare`), as Dolphin
//! and Nautilus do: no root needed once the system allows user shares (`usershare path` in
//! smb.conf, and the user in its group). Blocking: off the UI thread.

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::bail;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Share {
    pub name: String,
    pub path: PathBuf,
    pub comment: String,
    /// Others may change files (`Everyone:F`), not just read them (`Everyone:R`).
    pub writable: bool,
    /// Guests may open it without a user name and password.
    pub guest: bool,
}

/// Why folders can't be shared here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Unavailable {
    /// Samba's server (`smbd`) or its `net` tool isn't installed.
    NoSamba,
    /// Samba is there but user shares are off; `net`'s own words.
    Disabled(String),
}

impl std::fmt::Display for Unavailable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Unavailable::NoSamba => f.write_str("Samba is not installed"),
            Unavailable::Disabled(m) => f.write_str(m),
        }
    }
}
impl std::error::Error for Unavailable {}

fn net(args: &[&str]) -> anyhow::Result<String> {
    // `net` comes with the Samba client; without the server (smbd) a share is made but
    // nobody serves it
    if !crate::sandbox::in_path("smbd") && !Path::new("/usr/sbin/smbd").exists() {
        return Err(Unavailable::NoSamba.into());
    }
    let mut c = crate::sandbox::tool("net", &[]);
    c.arg("usershare").args(args);
    let out = match crate::tool::run(c, Duration::from_secs(20)) {
        Ok(o) => o,
        Err(e) if e.is::<crate::tool::NotInstalled>() => return Err(Unavailable::NoSamba.into()),
        Err(e) => return Err(e),
    };
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    if !out.ok {
        let err = String::from_utf8_lossy(&out.stderr);
        let msg = [err.as_ref(), text.as_str()].iter().flat_map(|s| s.lines()).map(str::trim).filter(|l| !l.is_empty()).last().unwrap_or("").to_string();
        let msg = msg.trim_start_matches("net usershare: ").to_string();
        // the system doesn't allow user shares (no usershare path, or no smb.conf at all)
        if msg.contains("disabled") || msg.contains("smb.conf") || msg.contains("usershare path") {
            return Err(Unavailable::Disabled(msg).into());
        }
        bail!("{msg}");
    }
    Ok(text)
}

/// The user shares on this computer. Fails with [`Unavailable`] when folders can't be shared.
pub fn list() -> anyhow::Result<Vec<Share>> {
    Ok(parse_info(&net(&["info"])?))
}

/// The share of folder `path`, if it has one.
pub fn share_of(path: &Path) -> anyhow::Result<Option<Share>> {
    Ok(list()?.into_iter().find(|s| s.path == path))
}

/// Shares `share.path` as `share.name` (replacing a share of that name).
pub fn add(share: &Share) -> anyhow::Result<()> {
    check_name(&share.name)?;
    let acl = if share.writable { "Everyone:F" } else { "Everyone:R" };
    let guest = if share.guest { "guest_ok=y" } else { "guest_ok=n" };
    let path = share.path.to_string_lossy();
    net(&["add", &share.name, &path, &share.comment, acl, guest]).map(|_| ())
}

/// Stops sharing `name`.
pub fn remove(name: &str) -> anyhow::Result<()> {
    net(&["delete", name]).map(|_| ())
}

/// Characters Samba refuses in a share name.
const BAD: &str = "%<>*?|/\\+=;:\",";

/// Whether Samba takes `name` as a share name.
pub fn name_ok(name: &str) -> bool {
    let n = name.trim();
    !n.is_empty() && n.chars().count() <= 80 && !n.chars().any(|c| BAD.contains(c) || c.is_control())
}

fn check_name(name: &str) -> anyhow::Result<()> {
    if !name_ok(name) {
        bail!("Samba doesn't allow that share name");
    }
    Ok(())
}

/// A share name for `path`: its folder name, without what Samba refuses.
pub fn default_name(path: &Path) -> String {
    let n = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let n: String = n.chars().map(|c| if BAD.contains(c) || c.is_control() { '_' } else { c }).take(80).collect();
    if n.trim().is_empty() { "share".to_string() } else { n }
}

/// `net usershare info`: an ini-style block per share.
fn parse_info(text: &str) -> Vec<Share> {
    let mut out: Vec<Share> = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            out.push(Share { name: name.to_string(), path: PathBuf::new(), comment: String::new(), writable: false, guest: false });
            continue;
        }
        let (Some(s), Some((k, v))) = (out.last_mut(), line.split_once('=')) else { continue };
        match k.trim() {
            "path" => s.path = PathBuf::from(v.trim()),
            "comment" => s.comment = v.trim().to_string(),
            "usershare_acl" => s.writable = v.split(',').any(|e| e.trim().to_ascii_lowercase().starts_with("everyone:f")),
            "guest_ok" => s.guest = v.trim().eq_ignore_ascii_case("y"),
            _ => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn info() {
        let text = "[films]\npath=/home/me/Films\ncomment=\nusershare_acl=Everyone:F,\nguest_ok=y\n\n[docs]\npath=/home/me/My Docs\ncomment=work\nusershare_acl=S-1-1-0:R,\nguest_ok=n\n";
        let s = parse_info(text);
        assert_eq!(s.len(), 2);
        assert_eq!(s[0], Share { name: "films".into(), path: "/home/me/Films".into(), comment: String::new(), writable: true, guest: true });
        assert_eq!((s[1].path.to_str(), s[1].writable, s[1].guest, s[1].comment.as_str()), (Some("/home/me/My Docs"), false, false, "work"));
        assert!(parse_info("").is_empty());
    }

    #[test]
    fn names() {
        assert!(name_ok("Films & TV"));
        assert!(!name_ok("a/b") && !name_ok("") && !name_ok("  ") && !name_ok("x:y"));
        assert_eq!(default_name(Path::new("/home/me/a+b=c")), "a_b_c");
        assert_eq!(default_name(Path::new("/")), "share");
    }
}
