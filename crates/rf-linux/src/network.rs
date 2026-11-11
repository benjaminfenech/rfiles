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
}

/// Windows' own "Map network drive" dialog (drive letter, folder, reconnect at sign-in,
/// other credentials). Modal to `owner`; returns when it closes.
pub fn map_drive_dialog(_owner: isize) {}
/// Windows' "Disconnect network drive" dialog.
pub fn disconnect_drive_dialog(_owner: isize) {}
