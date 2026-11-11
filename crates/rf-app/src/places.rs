//! Shared sidebar/home data: drives and pinned folders, loaded off the UI thread.

use std::path::PathBuf;

use gpui_kit::assets::IconName as L;
use gpui_kit::*;
use rf_win::{Drive, DriveKind, KnownFolder};

/// Linux: the drive's name as Settings → Files and folders → "Show drives as" asks (there are
/// no drive letters).
pub fn drive_name(d: &Drive) -> String {
    linux_drive_name(d, crate::settings::drive_names())
}

fn linux_drive_name(d: &Drive, style: crate::settings::DriveNames) -> String {
    use crate::settings::DriveNames;
    let mount = d.root.to_string_lossy().into_owned();
    let folder = || d.root.file_name().map(|n| n.to_string_lossy().into_owned());
    let root_name = || crate::t!("Dosya Sistemi").to_string();
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
                format!("{} {}", rf_core::format::size(d.total), crate::t!("Volume"))
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

/// Windows' own "recent items" (%APPDATA%\Microsoft\Windows\Recent\*.lnk), newest first.
fn load_recent(limit: usize) -> Vec<Recent> {
    let Some(dir) = dirs::data_dir().map(|d| d.join("Microsoft").join("Windows").join("Recent")) else {
        return Vec::new();
    };
    let Ok(rd) = std::fs::read_dir(&dir) else { return Vec::new() };
    let mut links: Vec<(std::time::SystemTime, PathBuf)> = rd
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().is_some_and(|x| x.eq_ignore_ascii_case("lnk")))
        .filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path())))
        .collect();
    links.sort_by(|a, b| b.0.cmp(&a.0));
    let mut out: Vec<Recent> = Vec::new();
    for (t, link) in links.into_iter().take(limit * 3) {
        let Ok(info) = rf_win::shortcut::resolve(&link) else { continue };
        if info.target.as_os_str().is_empty() || out.iter().any(|r| r.target == info.target) {
            continue;
        }
        let Ok(meta) = std::fs::metadata(&info.target) else { continue };
        out.push(Recent {
            name: info.target.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
            target: info.target,
            used_ms: t.duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0),
            is_dir: meta.is_dir(),
        });
        if out.len() >= limit {
            break;
        }
    }
    out
}

/// Cloud folders (OneDrive, Dropbox, iCloud, Google Drive) and WSL distributions.
fn load_cloud_and_wsl() -> Vec<Pinned> {
    let mut out = Vec::new();
    let mut add = |title: String, path: PathBuf, icon: L| {
        if path.is_dir() && !out.iter().any(|p: &Pinned| p.path == path) {
            out.push(Pinned { title, path, icon });
        }
    };
    for (var, title) in [("OneDriveConsumer", "OneDrive"), ("OneDriveCommercial", "OneDrive (İş)"), ("OneDrive", "OneDrive")] {
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
    pub libraries: Vec<rf_win::libraries::Library>,
    /// Network locations ("Add a network location" shortcuts): (name, target).
    pub network: Vec<(String, std::path::PathBuf)>,
    /// Devices under This PC that are not drives (MTP phones, cameras): browsed as shell
    /// folders.
    pub devices: Vec<rf_win::shellfolder::ShellChild>,
    pub loaded: bool,
    _task: Option<Task<()>>,
}

impl Places {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let mut this =
            Self { drives: Vec::new(), cloud: Vec::new(), pinned: Vec::new(), recent: Vec::new(), libraries: Vec::new(), network: Vec::new(), devices: Vec::new(), loaded: false, _task: None };
        this.refresh(cx);
        // Notice drives appearing/disappearing (USB, network mappings) without any shell hooks.
        cx.spawn(async move |this, cx| {
            let mut mask = rf_win::drives::drive_mask();
            let mut recheck = false;
            loop {
                cx.background_executor().timer(std::time::Duration::from_secs(3)).await;
                let now = rf_win::drives::drive_mask();
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
        // Phones come and go without a drive letter: ask the shell now and then. Off the UI
        // thread, and the list is only replaced when it changed. Not while no window is in front
        // of the user (minimised, or another app active): enumerating This PC loads shell
        // extensions and can touch the drives; the look happens again when one comes back.
        cx.spawn(async move |this, cx| loop {
            if !rf_win::window::app_in_front() {
                cx.background_executor().timer(std::time::Duration::from_secs(5)).await;
                if this.upgrade().is_none() {
                    break;
                }
                continue;
            }
            let found = cx.background_spawn(async move { rf_win::shellfolder::devices() }).await;
            let alive = this
                .update(cx, |p, cx| {
                    if p.devices != found {
                        p.devices = found;
                        cx.notify();
                    }
                })
                .is_ok();
            if !alive {
                break;
            }
            cx.background_executor().timer(std::time::Duration::from_secs(5)).await;
        })
        .detach();
        this
    }

    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        let user_pins = cx.try_global::<crate::settings::Settings>().and_then(|s| s.pinned.clone());
        let work = cx.background_spawn(async move {
            let known = rf_win::known_folders();
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
            let mut drives = rf_win::list_drives();
            // Linux: connected but not mounted (opening one mounts it)
            drives.extend(rf_win::drives::unmounted_drives());
            let recent = load_recent(14);
            let cloud = load_cloud_and_wsl();
            let libraries = rf_win::libraries::list();
            let network = rf_win::network::network_shortcuts();
            (pinned, drives, recent, cloud, libraries, network)
        });
        self._task = Some(cx.spawn(async move |this, cx| {
            let (pinned, drives, recent, cloud, libraries, network) = work.await;
            let _ = this.update(cx, |p, cx| {
                p.pinned = pinned;
                p.drives = drives;
                p.recent = recent;
                p.cloud = cloud;
                p.libraries = libraries;
                p.network = network;
                p.loaded = true;
                cx.notify();
            });
        }));
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
    return rf_win::drives::is_unmounted(d);
}

pub fn drive_icon(d: &Drive) -> L {
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
    use rf_win::Drive;

    fn drive(root: &str, source: &str, label: &str) -> Drive {
        Drive {
            root: root.into(),
            letter: '\0',
            label: label.into(),
            source: source.into(),
            kind: rf_win::DriveKind::Fixed,
            total: 0,
            free: 0,
            fs: String::new(),
        }
    }

    #[test]
    fn linux_drive_names() {
        let root = drive("/", "/dev/nvme0n1p2", "");
        let usb = drive("/run/media/me/KINGSTON", "/dev/sdb1", "KINGSTON");
        let smb = drive("/mnt/media", "//10.0.0.9/media", "");
        let nfs = drive("/mnt/nas", "nas:/export/photos/", "");
        let ssh = drive("/home/me/remote", "me@box:", "");
        let root_name = crate::t!("Dosya Sistemi");
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
    }
}
