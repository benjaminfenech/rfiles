//! Shared sidebar/home data: drives and pinned folders, loaded off the UI thread.

use std::path::{Path, PathBuf};

use gpui_kit::assets::IconName as L;
use gpui_kit::*;
use rf_linux::{Drive, DriveKind, KnownFolder};

/// Linux: the drive's name as Settings → Files and folders → "Show drives as" asks (there are
/// no drive letters).
pub fn drive_name(d: &Drive) -> String {
    linux_drive_name(d, crate::settings::drive_names())
}

fn linux_drive_name(d: &Drive, style: crate::settings::DriveNames) -> String {
    use crate::settings::DriveNames;
    let mount = d.root.to_string_lossy().into_owned();
    let folder = || d.root.file_name().map(|n| n.to_string_lossy().into_owned());
    let root_name = || crate::t!("File System").to_string();
    match style {
        DriveNames::MountPoint => mount,
        DriveNames::Folder => folder().unwrap_or_else(root_name),
        DriveNames::Device => match d.source.strip_prefix("/dev/") {
            Some(dev) => dev.to_string(),
            None if !d.source.is_empty() => d.source.clone(),
            None => mount,
        },
        DriveNames::Auto => {
            if !d.label.is_empty() {
                d.label.clone()
            } else if unmounted(d) {
                // what other file managers call a partition without a label: "500 GB Volume"
                let what = if locked(d) { crate::t!("Encrypted volume") } else { crate::t!("Volume") };
                format!("{} {}", rf_core::format::size(d.total), what)
            } else if let Some(share) = share_name(&d.source) {
                share
            } else {
                folder().unwrap_or_else(root_name)
            }
        }
    }
}

/// A network share as "share (server)": `//10.0.0.9/media` (SMB), `nas:/export/media` (NFS),
/// `me@host:/srv/files` (sshfs). Local devices: None.
fn share_name(source: &str) -> Option<String> {
    let last = |p: &str| p.trim_end_matches('/').rsplit('/').next().filter(|s| !s.is_empty()).map(str::to_string);
    if let Some(rest) = source.strip_prefix("//") {
        let (host, path) = rest.split_once('/')?;
        return Some(format!("{} ({host})", last(path)?));
    }
    if source.starts_with('/') {
        return None;
    }
    let (host, path) = source.split_once(':')?;
    let host = host.rsplit('@').next().unwrap_or(host);
    if host.is_empty() {
        return None;
    }
    Some(match last(path) {
        Some(name) => format!("{name} ({host})"),
        None => host.to_string(),
    })
}

#[derive(Clone)]
pub struct Pinned {
    pub title: String,
    pub path: PathBuf,
    pub icon: L,
}

#[derive(Clone)]
pub struct Recent {
    pub name: String,
    pub target: PathBuf,
    pub used_ms: i64,
    pub is_dir: bool,
}

/// Recently used files for Home's Recent section: the desktop's list (freedesktop's
/// `$XDG_DATA_HOME/recently-used.xbel`, written by GTK, KDE and most apps), newest first.
/// Inside Flatpak `adopt_host_dirs` has pointed `XDG_DATA_HOME` at the host's.
fn load_recent(limit: usize) -> Vec<Recent> {
    let Some(xml) = dirs::data_dir().and_then(|d| std::fs::read_to_string(d.join("recently-used.xbel")).ok()) else {
        return Vec::new();
    };
    // a file on an fstab share that isn't mounted (away from home) is kept unchecked: checking
    // would mount the share, or wait out the mount timeout when its server is away
    let asleep = rf_linux::drives::sleeping_automounts();
    recent_from_xbel(&xml, limit, |p| {
        if asleep.iter().any(|a| p.starts_with(a)) {
            return Some(false);
        }
        std::fs::metadata(p).ok().map(|m| m.is_dir())
    })
}

/// The local files in a `recently-used.xbel`, newest first, without duplicates, at most
/// `limit`. `is_dir` says whether a path exists (None: gone, skipped) and is a folder.
fn recent_from_xbel(xml: &str, limit: usize, is_dir: impl Fn(&Path) -> Option<bool>) -> Vec<Recent> {
    let mut items: Vec<(PathBuf, i64)> = xml
        .split("<bookmark")
        .skip(1)
        .filter(|item| item.starts_with(|c: char| c.is_ascii_whitespace()))
        .filter_map(|item| {
            let tag = item.split_once('>')?.0;
            let href = xml_unescape(&xml_attr(tag, "href")?);
            if !href.starts_with("file://") {
                return None;
            }
            let path = rf_linux::fm_service::uri_to_path(&href)?;
            let used_ms = ["modified", "visited", "added"]
                .iter()
                .filter_map(|a| xml_attr(tag, a))
                .find_map(|t| chrono::DateTime::parse_from_rfc3339(&t).ok())
                .map_or(0, |t| t.timestamp_millis());
            Some((path, used_ms))
        })
        .collect();
    // newest first; for a path listed twice the newest entry stays
    items.sort_by(|a, b| b.1.cmp(&a.1));
    let mut out: Vec<Recent> = Vec::new();
    for (target, used_ms) in items {
        if out.len() >= limit {
            break;
        }
        if out.iter().any(|r| r.target == target) {
            continue;
        }
        let Some(is_dir) = is_dir(&target) else { continue };
        out.push(Recent {
            name: target.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
            target,
            used_ms,
            is_dir,
        });
    }
    out
}

/// The value of attribute `name` in the inside of an XML start tag (`href="…" modified='…'`).
fn xml_attr(tag: &str, name: &str) -> Option<String> {
    let mut rest = tag;
    loop {
        rest = rest.trim_start();
        let (key, after) = rest.split_once('=')?;
        let after = after.trim_start();
        let quote = after.chars().next().filter(|c| *c == '"' || *c == '\'')?;
        let (value, tail) = after[1..].split_once(quote)?;
        if key.trim() == name {
            return Some(value.to_string());
        }
        rest = tail;
    }
}

fn xml_unescape(s: &str) -> String {
    s.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&apos;", "'").replace("&amp;", "&")
}

/// Cloud folders (OneDrive, Dropbox, iCloud, Google Drive) and WSL distributions.
fn load_cloud_and_wsl() -> Vec<Pinned> {
    let mut out = Vec::new();
    let mut add = |title: String, path: PathBuf, icon: L| {
        if path.is_dir() && !out.iter().any(|p: &Pinned| p.path == path) {
            out.push(Pinned { title, path, icon });
        }
    };
    for (var, title) in [("OneDriveConsumer", "OneDrive"), ("OneDriveCommercial", "OneDrive (Work)"), ("OneDrive", "OneDrive")] {
        if let Some(v) = std::env::var_os(var) {
            add(title.into(), PathBuf::from(v), L::Cloud);
        }
    }
    if let Some(home) = dirs::home_dir() {
        add("iCloud Drive".into(), home.join("iCloudDrive"), L::Cloud);
        add("Dropbox".into(), home.join("Dropbox"), L::Cloud);
    }
    if let Some(local) = dirs::data_local_dir() {
        // Dropbox records its real location in info.json
        if let Ok(t) = std::fs::read_to_string(local.join("Dropbox").join("info.json")) {
            if let Some(i) = t.find("\"path\": \"") {
                let rest = &t[i + 9..];
                if let Some(j) = rest.find('"') {
                    add("Dropbox".into(), PathBuf::from(rest[..j].replace("\\\\", "\\")), L::Cloud);
                }
            }
        }
    }
    for letter in ['G', 'H'] {
        add("Google Drive".into(), PathBuf::from(format!("{letter}:\\My Drive")), L::Cloud);
    }
    out
}

pub struct Places {
    pub drives: Vec<Drive>,
    pub cloud: Vec<Pinned>,
    pub pinned: Vec<Pinned>,
    pub recent: Vec<Recent>,
    /// Network locations ("Add a network location" shortcuts): (name, target).
    pub network: Vec<(String, std::path::PathBuf)>,
    /// File servers announcing themselves on the local network (Avahi), looked for after
    /// each refresh and every minute, apart from the rest (it takes a second or two).
    pub servers: Vec<rf_linux::network::Server>,
    /// Phones, tablets and cameras plugged in over USB (opened through gvfs or a FUSE tool).
    pub devices: Vec<rf_linux::devices::Device>,
    /// fstab shares whose server doesn't answer (away from home), with that server: shown as
    /// offline. Checked after each refresh and every 30 s, apart from the rest (it can take
    /// a moment and must not hold the sidebar up).
    pub offline: std::collections::HashMap<PathBuf, String>,
    pub loaded: bool,
    _task: Option<Task<()>>,
    _probe: Option<Task<()>>,
    _discover: Option<Task<()>>,
}

impl Places {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let mut this =
            Self {
                drives: Vec::new(),
                cloud: Vec::new(),
                pinned: Vec::new(),
                recent: Vec::new(),
                network: Vec::new(),
                servers: Vec::new(),
                devices: Vec::new(),
                offline: Default::default(),
                loaded: false,
                _task: None,
                _probe: None,
                _discover: None,
            };
        this.refresh(cx);
        // Notice drives appearing/disappearing (USB, network mappings) without any shell hooks.
        cx.spawn(async move |this, cx| {
            // a phone or camera has no block device: USB devices coming and going count too
            let stamp = || (rf_linux::drives::drive_mask(), rf_linux::devices::usb_stamp());
            let mut mask = stamp();
            let mut recheck = false;
            let mut tick = 0u32;
            loop {
                cx.background_executor().timer(std::time::Duration::from_secs(3)).await;
                // back on the share's network (or off it): its offline mark follows
                tick += 1;
                if tick.is_multiple_of(10) && this.update(cx, |p, cx| p.probe_offline(cx)).is_err() {
                    break;
                }
                if tick.is_multiple_of(20) && this.update(cx, |p, cx| p.discover(cx)).is_err() {
                    break;
                }
                let now = stamp();
                // after a change, look once more on the next tick: a new stick's file system
                // shows up in UDisks a moment after its device does
                if now != mask || recheck {
                    recheck = now != mask;
                    mask = now;
                    if this.update(cx, |p, cx| p.refresh(cx)).is_err() {
                        break;
                    }
                }
            }
        })
        .detach();
        this
    }

    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        let user_pins = cx.try_global::<crate::settings::Settings>().and_then(|s| s.pinned.clone());
        let work = cx.background_spawn(async move {
            let known = rf_linux::known_folders();
            let pinned = match user_pins {
                None => known
                    .into_iter()
                    .map(|(k, path)| Pinned { title: k.title().to_string(), path, icon: known_icon(k) })
                    .collect::<Vec<_>>(),
                Some(paths) => paths
                    .into_iter()
                    .map(|path| match known.iter().find(|(_, p)| *p == path) {
                        Some((k, _)) => Pinned { title: k.title().to_string(), path, icon: known_icon(*k) },
                        None => Pinned {
                            title: rf_core::history::display_name(&path),
                            path,
                            icon: L::Folder,
                        },
                    })
                    .collect(),
            };
            #[allow(unused_mut)]
            let mut drives = rf_linux::list_drives();
            // Linux: connected but not mounted (opening one mounts it)
            drives.extend(rf_linux::drives::unmounted_drives());
            let recent = load_recent(14);
            let cloud = load_cloud_and_wsl();
            let network = rf_linux::network::network_shortcuts();
            let devices = rf_linux::devices::list();
            (pinned, drives, recent, cloud, network, devices)
        });
        self._task = Some(cx.spawn(async move |this, cx| {
            let (pinned, drives, recent, cloud, network, devices) = work.await;
            let _ = this.update(cx, |p, cx| {
                p.pinned = pinned;
                p.drives = drives;
                p.recent = recent;
                p.cloud = cloud;
                p.network = network;
                p.devices = devices;
                p.loaded = true;
                p.probe_offline(cx);
                p.discover(cx);
                cx.notify();
            });
        }));
    }

    /// Looks for file servers on the local network (see `servers`).
    fn discover(&mut self, cx: &mut Context<Self>) {
        if self._discover.is_some() {
            return;
        }
        let work = cx.background_spawn(async move { rf_linux::network::discover() });
        self._discover = Some(cx.spawn(async move |this, cx| {
            let found = work.await;
            let _ = this.update(cx, |p, cx| {
                p._discover = None;
                if p.servers != found {
                    p.servers = found;
                    cx.notify();
                }
            });
        }));
    }

    /// Checks whether the network shares' servers answer (see `offline`).
    fn probe_offline(&mut self, cx: &mut Context<Self>) {
        let roots: Vec<PathBuf> = self.drives.iter().filter(|d| d.kind == DriveKind::Network).map(|d| d.root.clone()).collect();
        if roots.is_empty() {
            if !self.offline.is_empty() {
                self.offline.clear();
                cx.notify();
            }
            return;
        }
        let work = cx.background_spawn(async move { rf_linux::drives::offline_shares(&roots, std::time::Duration::from_millis(1500)) });
        self._probe = Some(cx.spawn(async move |this, cx| {
            let found: std::collections::HashMap<PathBuf, String> = work.await.into_iter().collect();
            let _ = this.update(cx, |p, cx| {
                if p.offline != found {
                    p.offline = found;
                    cx.notify();
                }
            });
        }));
    }
}

pub fn device_icon(d: &rf_linux::devices::Device) -> L {
    match d.kind {
        rf_linux::devices::DeviceKind::Phone => L::Smartphone,
        rf_linux::devices::DeviceKind::Camera => L::Camera,
        rf_linux::devices::DeviceKind::Apple => L::TabletSmartphone,
    }
}

pub fn known_icon(k: KnownFolder) -> L {
    match k {
        KnownFolder::Desktop => L::Monitor,
        KnownFolder::Downloads => L::Download,
        KnownFolder::Documents => L::FileText,
        KnownFolder::Pictures => L::Image,
        KnownFolder::Music => L::Music,
        KnownFolder::Videos => L::Video,
        KnownFolder::Profile => L::House,
    }
}

/// Linux: a connected drive that isn't mounted yet (its `root` is the device; opening it
/// mounts it). Never on Windows.
pub fn unmounted(d: &Drive) -> bool {
    rf_linux::drives::is_unmounted(d)
}

/// A LUKS drive nobody has unlocked yet (opening it asks for the passphrase).
pub fn locked(d: &Drive) -> bool {
    unmounted(d) && d.fs == "crypto_LUKS"
}

pub fn drive_icon(d: &Drive) -> L {
    if locked(d) {
        return L::Lock;
    }
    match d.kind {
        DriveKind::Removable => L::Usb,
        DriveKind::Network => L::Network,
        DriveKind::Optical => L::Disc3,
        _ => L::HardDrive,
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::linux_drive_name;
    use crate::settings::DriveNames::*;
    use rf_linux::Drive;

    fn drive(root: &str, source: &str, label: &str) -> Drive {
        Drive {
            root: root.into(),
            letter: '\0',
            label: label.into(),
            source: source.into(),
            kind: rf_linux::DriveKind::Fixed,
            total: 0,
            free: 0,
            fs: String::new(),
        }
    }

    #[test]
    fn linux_drive_names() {
        // the root's and unlabelled volumes' names are translated: keep the language still
        let _lock = crate::i18n::test_lock();
        let root = drive("/", "/dev/nvme0n1p2", "");
        let usb = drive("/run/media/me/KINGSTON", "/dev/sdb1", "KINGSTON");
        let smb = drive("/mnt/media", "//10.0.0.9/media", "");
        let nfs = drive("/mnt/nas", "nas:/export/photos/", "");
        let ssh = drive("/home/me/remote", "me@box:", "");
        let root_name = crate::t!("File System");
        let names = |s| [&root, &usb, &smb, &nfs, &ssh].map(|d| linux_drive_name(d, s));
        assert_eq!(names(Auto), [root_name, "KINGSTON", "media (10.0.0.9)", "photos (nas)", "box"]);
        assert_eq!(names(MountPoint), ["/", "/run/media/me/KINGSTON", "/mnt/media", "/mnt/nas", "/home/me/remote"]);
        assert_eq!(names(Folder), [root_name, "KINGSTON", "media", "nas", "remote"]);
        assert_eq!(names(Device), ["nvme0n1p2", "sdb1", "//10.0.0.9/media", "nas:/export/photos/", "me@box:"]);
        // connected, not mounted: named by its size until it has a label
        let mut part = drive("/dev/sda2", "/dev/sda2", "");
        part.total = 500_000_000_000;
        assert!(super::unmounted(&part));
        assert!(linux_drive_name(&part, Auto).ends_with(crate::t!("Volume")), "{}", linux_drive_name(&part, Auto));
        assert_eq!(linux_drive_name(&part, Device), "sda2");
        part.label = "Windows".into();
        assert_eq!(linux_drive_name(&part, Auto), "Windows");
        // a locked LUKS partition says so
        let mut luks = drive("/dev/sdc1", "/dev/sdc1", "");
        luks.fs = "crypto_LUKS".into();
        assert!(super::locked(&luks));
        assert!(linux_drive_name(&luks, Auto).ends_with(crate::t!("Encrypted volume")));
        assert!(!super::locked(&usb));
    }

    const XBEL: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<xbel version="1.0" xmlns:bookmark="http://www.freedesktop.org/standards/desktop-bookmarks"
      xmlns:mime="http://www.freedesktop.org/standards/shared-mime-info">
  <bookmark href="file:///home/me/Documents/report.pdf" added="2026-11-01T09:00:00Z" modified="2026-11-10T12:00:00.123456Z" visited="2026-11-10T12:00:00Z">
    <info><metadata owner="http://freedesktop.org"><mime:mime-type type="application/pdf"/>
      <bookmark:applications><bookmark:application name="Evince" exec="&apos;evince %u&apos;" modified="2026-11-10T12:00:00Z" count="3"/></bookmark:applications>
    </metadata></info>
  </bookmark>
  <bookmark href="https://example.com/page" added="2026-11-11T08:00:00Z" modified="2026-11-11T08:00:00Z"/>
  <bookmark modified='2026-11-11T07:00:00Z' href='file:///home/me/My%20Files/caf%C3%A9%20%26%20bar.txt'/>
  <bookmark href="file:///home/me/Pictures" modified="2026-11-09T10:00:00Z"/>
  <bookmark href="file:///home/me/gone.txt" modified="2026-11-11T09:00:00Z"/>
  <bookmark href="file:///home/me/Documents/report%2Epdf" modified="2026-11-02T10:00:00Z"/>
  <bookmark href="file:///home/me/old.txt" modified="2025-01-01T00:00:00Z"/>
</xbel>"#;

    fn exists(p: &std::path::Path) -> Option<bool> {
        let p = p.to_str().unwrap();
        (!p.ends_with("gone.txt")).then(|| p.ends_with("Pictures"))
    }

    #[test]
    fn recent_from_xbel() {
        let r = super::recent_from_xbel(XBEL, 10, exists);
        let targets: Vec<_> = r.iter().map(|r| r.target.to_str().unwrap()).collect();
        // newest first; web pages, missing files and the repeated report are left out
        assert_eq!(
            targets,
            ["/home/me/My Files/café & bar.txt", "/home/me/Documents/report.pdf", "/home/me/Pictures", "/home/me/old.txt"]
        );
        assert_eq!(r[0].name, "café & bar.txt");
        assert_eq!(r[1].used_ms, 1_794_312_000_123);
        assert!(r[2].is_dir && !r[1].is_dir);
        // the limit counts only the files shown
        let two = super::recent_from_xbel(XBEL, 2, exists);
        assert_eq!(two.len(), 2);
        assert_eq!(two[1].target, r[1].target);
        assert!(super::recent_from_xbel(XBEL, 0, exists).is_empty());
    }

    #[test]
    fn recent_from_bad_xbel() {
        for xml in ["", "not xml", "<xbel><bookmark", "<bookmark href=\"file:///a", "<bookmark href=>", "<bookmark:application href=\"file:///a\"/>"] {
            assert!(super::recent_from_xbel(xml, 10, |_| Some(false)).is_empty(), "{xml}");
        }
        // no usable time: still listed, oldest
        let r = super::recent_from_xbel(r#"<bookmark href="file:///a"/><bookmark href="file:///b" modified="2026-01-01T00:00:00Z"/>"#, 10, |_| Some(false));
        assert_eq!(r.iter().map(|r| r.used_ms == 0).collect::<Vec<_>>(), [false, true]);
    }
}
