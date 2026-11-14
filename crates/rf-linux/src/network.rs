//! Network locations: shares of a server (`\\server` is not listable with FindFirstFile) and the
//! user's network shortcuts. Blocking and potentially slow (SMB timeouts): off the UI thread.
// Generated from rf-win's public API by a one-off stub generator, then edited by hand.
// Functions return safe defaults until they get a Linux implementation.
use std::path::{Path, PathBuf};
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
pub fn shares(_server: &str) -> std::io::Result<Vec<Share>> {
    Err(
        std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "rf-linux: `shares` is not implemented on Linux yet",
        ),
    )
}
/// Linux: the network bookmarks of KDE (`~/.local/share/user-places.xbel`, Dolphin's places)
/// and GTK (`~/.config/gtk-3.0/bookmarks`): (name, URL). Opening one mounts it
/// (`remote::mount`).
pub fn network_shortcuts() -> Vec<(String, PathBuf)> {
    let mut out: Vec<(String, PathBuf)> = Vec::new();
    let mut add = |name: String, href: &str| {
        let Some(url) = crate::remote::to_url(href) else { return };
        if out.iter().any(|(_, u)| u.as_os_str() == url.as_str()) {
            return;
        }
        let name = if name.trim().is_empty() { url.split_once("://").map_or(url.as_str(), |(_, r)| r).trim_end_matches('/').to_string() } else { name };
        out.push((name, PathBuf::from(url)));
    };
    if let Some(data) = dirs::data_dir()
        && let Ok(xml) = std::fs::read_to_string(data.join("user-places.xbel"))
    {
        for (href, title) in xbel_bookmarks(&xml) {
            add(title, &href);
        }
    }
    if let Some(cfg) = dirs::config_dir()
        && let Ok(text) = std::fs::read_to_string(cfg.join("gtk-3.0/bookmarks"))
    {
        for line in text.lines() {
            let (href, name) = line.split_once(' ').unwrap_or((line, ""));
            add(name.to_string(), href);
        }
    }
    out.sort_by(|a, b| rf_core::sort::natural_cmp(&a.0, &b.0));
    out
}

/// A file server on the local network that announces itself (mDNS / DNS-SD, as NAS boxes,
/// Macs and Linux machines running Samba with Avahi do).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Server {
    /// The name it announces ("Media NAS").
    pub name: String,
    /// What opens it: `smb://host/` lists its shares.
    pub url: String,
}

/// The service types worth opening as folders, and their URL schemes.
const SERVICES: &[(&str, &str)] = &[
    ("_smb._tcp", "smb"),
    ("_sftp-ssh._tcp", "sftp"),
    ("_webdav._tcp", "dav"),
    ("_webdavs._tcp", "davs"),
    ("_ftp._tcp", "ftp"),
    ("_nfs._tcp", "nfs"),
];

/// The file servers announcing themselves on the local network, through Avahi's
/// `avahi-browse` (it answers from the daemon's cache, in a second or two). Empty when Avahi
/// isn't there. Blocking: off the UI thread.
pub fn discover() -> Vec<Server> {
    let mut c = crate::sandbox::tool("avahi-browse", &[]);
    // -t: stop once the cache is listed, -r: resolve to address and port, -p: parsable
    c.args(["-trp", "-a"]);
    match crate::tool::run(c, std::time::Duration::from_secs(8)) {
        Ok(out) => servers_from_browse(&String::from_utf8_lossy(&out.stdout)),
        Err(e) => {
            log::debug!("avahi-browse: {e:#}");
            Vec::new()
        }
    }
}

/// The servers in `avahi-browse -rp` output: its resolved (`=`) lines, one per service and
/// address. An IPv4 address is preferred (an IPv6 one needs brackets and often a scope).
fn servers_from_browse(text: &str) -> Vec<Server> {
    let mut found: Vec<(Server, String, bool)> = Vec::new();
    for line in text.lines() {
        // =;iface;proto;name;type;domain;host;address;port;txt
        let f: Vec<&str> = line.splitn(10, ';').collect();
        if f.len() < 9 || f[0] != "=" {
            continue;
        }
        let Some((_, scheme)) = SERVICES.iter().find(|(t, _)| *t == f[4]) else { continue };
        let v4 = f[2] == "IPv4";
        let host = if v4 { f[7].to_string() } else { f[6].to_string() };
        let port: u16 = f[8].parse().unwrap_or(0);
        let default_port = match *scheme {
            "smb" => 445,
            "sftp" => 22,
            "dav" => 80,
            "davs" => 443,
            "ftp" => 21,
            _ => 0,
        };
        let port = if port == 0 || port == default_port || *scheme == "smb" || *scheme == "nfs" { String::new() } else { format!(":{port}") };
        let txt = f.get(9).copied().unwrap_or("");
        let path = txt_value(txt, "path").map(|p| p.trim_matches('/').to_string()).filter(|p| !p.is_empty()).map(|p| format!("{p}/")).unwrap_or_default();
        let name = unescape(f[3]);
        let url = format!("{scheme}://{host}{port}/{path}");
        let key = format!("{}|{name}", f[4]);
        match found.iter_mut().find(|(_, k, _)| *k == key) {
            // the IPv4 answer wins over an IPv6 one for the same service
            Some(slot) if !slot.2 && v4 => *slot = (Server { name, url }, key, v4),
            Some(_) => {}
            None => found.push((Server { name, url }, key, v4)),
        }
    }
    let mut out: Vec<Server> = found.into_iter().map(|(s, _, _)| s).collect();
    out.sort_by(|a, b| rf_core::sort::natural_cmp(&a.name, &b.name).then(a.url.cmp(&b.url)));
    out
}

/// avahi-browse escapes names: `\032` is byte 32 (decimal), `\.` a dot.
fn unescape(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'\\' && i + 3 < b.len() && b[i + 1..i + 4].iter().all(u8::is_ascii_digit) {
            out.push(s[i + 1..i + 4].parse::<u16>().ok().and_then(|n| u8::try_from(n).ok()).unwrap_or(b'?'));
            i += 4;
        } else if b[i] == b'\\' && i + 1 < b.len() {
            out.push(b[i + 1]);
            i += 2;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// A TXT record's value (`"path=/share" "u=me"`).
fn txt_value(txt: &str, key: &str) -> Option<String> {
    txt.split('"').skip(1).step_by(2).find_map(|kv| kv.strip_prefix(key)?.strip_prefix('=').map(str::to_string))
}

/// (href, title) of each `<bookmark>` in an XBEL file (enough of XML for KDE's places).
fn xbel_bookmarks(xml: &str) -> Vec<(String, String)> {
    let unescape = |s: &str| s.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&apos;", "'").replace("&amp;", "&");
    xml.split("<bookmark ")
        .skip(1)
        .filter_map(|item| {
            let item = item.split("</bookmark>").next()?;
            let (_, r) = item.split_once("href=\"")?;
            let href = unescape(r.split_once('"')?.0);
            let title = item.split_once("<title>").and_then(|(_, r)| r.split_once("</title>")).map(|(t, _)| unescape(t)).unwrap_or_default();
            Some((href, title))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    #[test]
    fn kde_places() {
        let xml = r#"<xbel><bookmark href="file:///home/me"><title>Home</title></bookmark>
            <bookmark href="smb://nas/media"><title>Films &amp; TV</title><info/></bookmark>
            <bookmark href="remote:/"><title>Network</title></bookmark></xbel>"#;
        let b = super::xbel_bookmarks(xml);
        assert_eq!(b.len(), 3);
        assert_eq!(b[1], ("smb://nas/media".to_string(), "Films & TV".to_string()));
        // only network ones become shortcuts
        assert!(crate::remote::to_url(&b[0].0).is_none() && crate::remote::to_url(&b[2].0).is_none());
    }

    #[test]
    fn browsed_servers() {
        let text = "+;enp5s0;IPv4;Media\\032NAS;_smb._tcp;local
=;enp5s0;IPv6;Media\\032NAS;_smb._tcp;local;nas.local;fe80::1;445;
=;enp5s0;IPv4;Media\\032NAS;_smb._tcp;local;nas.local;10.0.0.9;445;
=;enp5s0;IPv4;box;_sftp-ssh._tcp;local;box.local;10.0.0.5;2222;
=;enp5s0;IPv6;cloud;_webdavs._tcp;local;cloud.local;fd00::5;443;\"path=/remote.php/webdav/\" \"u=x\"
=;enp5s0;IPv4;Printer;_ipp._tcp;local;p.local;10.0.0.7;631;
";
        let s = super::servers_from_browse(text);
        let got: Vec<(&str, &str)> = s.iter().map(|s| (s.name.as_str(), s.url.as_str())).collect();
        assert_eq!(
            got,
            [("box", "sftp://10.0.0.5:2222/"), ("cloud", "davs://cloud.local/remote.php/webdav/"), ("Media NAS", "smb://10.0.0.9/")]
        );
        assert_eq!(super::unescape("a\\.b\\092c"), "a.b\\c");
    }
}

/// Windows' own "Map network drive" dialog (drive letter, folder, reconnect at sign-in,
/// other credentials). Modal to `owner`; returns when it closes.
pub fn map_drive_dialog(_owner: isize) {}
/// Windows' "Disconnect network drive" dialog.
pub fn disconnect_drive_dialog(_owner: isize) {}
