
// Generated from rf-win's public API by a one-off stub generator, then edited by hand.
// Functions return safe defaults until they get a Linux implementation.
use std::path::{Path, PathBuf};
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DriveKind {
    Fixed,
    Removable,
    Network,
    Optical,
    RamDisk,
    Unknown,
}

#[derive(Debug, Clone)]
pub struct Drive {
    /// The mount point ("/", "/run/media/me/USB").
    pub root: PathBuf,
    /// Windows' drive letter; always '\0' on Linux.
    pub letter: char,
    /// The file system's label (`/dev/disk/by-label`), empty without one.
    pub label: String,
    /// Linux only: where it comes from: the device (`/dev/sdb1`) or the share
    /// (`//server/share`, `server:/export`, `user@host:/path`).
    pub source: String,
    pub kind: DriveKind,
    pub total: u64,
    pub free: u64,
    pub fs: String,
}

impl Drive {
    pub fn display_name(&self) -> String {
        if !self.label.is_empty() {
            return self.label.clone();
        }
        match self.kind {
            _ if self.root == Path::new("/") => "File System".to_string(),
            DriveKind::Removable => "USB Drive".to_string(),
            DriveKind::Network => "Network Drive".to_string(),
            DriveKind::Optical => "DVD Drive".to_string(),
            _ => self.root.file_name().map_or_else(|| "Local Disk".into(), |n| n.to_string_lossy().into_owned()),
        }
    }

    pub fn used_fraction(&self) -> f32 {
        if self.total == 0 { 0.0 } else { 1.0 - self.free as f32 / self.total as f32 }
    }
}

const NETWORK_FS: &[&str] = &["nfs", "nfs4", "cifs", "smb3", "smbfs", "sshfs", "fuse.sshfs", "9p", "afs", "ceph"];

/// One mount from `/proc/self/mountinfo`: (mount point, fs type, source).
pub(crate) fn mounts() -> Vec<(PathBuf, String, String)> {
    let Ok(text) = std::fs::read_to_string("/proc/self/mountinfo") else { return Vec::new() };
    text.lines()
        .filter_map(|l| {
            // id parent maj:min root mountpoint opts [optional...] - fstype source superopts
            let (left, right) = l.split_once(" - ")?;
            let point = left.split(' ').nth(4)?;
            let mut r = right.split(' ');
            let fs = r.next()?.to_string();
            let src = r.next()?.to_string();
            Some((PathBuf::from(unescape(point)), fs, unescape(&src)))
        })
        .collect()
}

/// mountinfo escapes space, tab, newline and backslash as octal (`\040`).
fn unescape(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'\\' && i + 3 < b.len() && b[i + 1..i + 4].iter().all(|c| (b'0'..=b'7').contains(c)) {
            out.push(u8::from_str_radix(&s[i + 1..i + 4], 8).unwrap_or(b'?'));
            i += 4;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Whether a block device is removable (`/sys/block/<dev>/removable`, USB sticks and SD cards).
fn removable_dev(src: &str) -> bool {
    let Some(name) = src.strip_prefix("/dev/") else { return false };
    let Ok(real) = std::fs::canonicalize(format!("/dev/{name}")) else { return false };
    let dev = real.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    // a partition (sdb1) lives under its disk in /sys/class/block/<part>/.. 
    let sys = Path::new("/sys/class/block").join(&dev);
    let disk = std::fs::canonicalize(&sys).ok().and_then(|p| {
        if p.join("partition").exists() { p.parent().map(Path::to_path_buf) } else { Some(p) }
    });
    disk.and_then(|d| std::fs::read_to_string(d.join("removable")).ok()).is_some_and(|s| s.trim() == "1")
}

fn kind_of(fs: &str, src: &str, point: &Path) -> Option<DriveKind> {
    if NETWORK_FS.contains(&fs) {
        return Some(DriveKind::Network);
    }
    if fs == "iso9660" || fs == "udf" {
        return Some(DriveKind::Optical);
    }
    // a disk image the user mounted (UDisks puts those under /run/media) counts as removable
    let user_mount = point.starts_with("/run/media") || point.starts_with("/media");
    if !src.starts_with("/dev/") || (src.starts_with("/dev/loop") && !user_mount) {
        return None; // proc, sysfs, tmpfs, overlay, snap loops…
    }
    if removable_dev(src) || user_mount {
        return Some(DriveKind::Removable);
    }
    Some(DriveKind::Fixed)
}

/// Whether reading `path` may take seconds: a network share, a FUSE mount (kio-fuse and gvfs
/// network places, sshfs, cloud drives: reading may download), or a removable or optical disk.
/// DirectLook opens such files off the UI thread. Reads `/proc/self/mountinfo`, no disk access.
pub fn slow_to_read(path: &Path) -> bool {
    // the innermost mount holding the path
    let Some((point, fs, src)) = mounts().into_iter().filter(|(m, _, _)| path.starts_with(m)).max_by_key(|(m, _, _)| m.components().count()) else {
        return false;
    };
    // fuseblk is a local disk through FUSE (ntfs-3g, exfat-fuse): as quick as any disk
    // autofs: an automount, which opening mounts (from the network, for fstab's shares)
    (fs.starts_with("fuse") && fs != "fuseblk") || fs == "autofs" || matches!(kind_of(&fs, &src, &point), Some(DriveKind::Network | DriveKind::Removable | DriveKind::Optical))
}

/// (total, free for the user) bytes of the filesystem holding `p`.
fn space(p: &Path) -> Option<(u64, u64)> {
    use std::os::unix::ffi::OsStrExt;
    let c = std::ffi::CString::new(p.as_os_str().as_bytes()).ok()?;
    let mut st: libc::statvfs = unsafe { std::mem::zeroed() };
    // SAFETY: valid C string and out pointer
    if unsafe { libc::statvfs(c.as_ptr(), &mut st) } != 0 {
        return None;
    }
    let f = st.f_frsize as u64;
    Some((st.f_blocks as u64 * f, st.f_bavail as u64 * f))
}

/// Mounted filesystems worth showing: real disks, removable media and network shares (the
/// root, /home, /boot… and anything under /run/media). Bind mounts and btrfs subvolumes of
/// the same device show once. `statvfs` can stall on a dead network mount: call off the UI
/// thread.
pub fn list_drives() -> Vec<Drive> {
    let labels = labels();
    let fstab = fstab_network(&std::fs::read_to_string("/etc/fstab").unwrap_or_default());
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for (point, fs, src) in mounts() {
        let (fs, src) = as_share(point.as_path(), fs, src, &fstab);
        let Some(kind) = kind_of(&fs, &src, &point) else { continue };
        // system plumbing mounts of the root disk
        if kind == DriveKind::Fixed && point != Path::new("/") && !point.starts_with("/home") && !point.starts_with("/mnt") {
            continue;
        }
        if !seen.insert(src.clone()) {
            continue;
        }
        let (total, free) = if kind == DriveKind::Network { (0, 0) } else { space(&point).unwrap_or((0, 0)) };
        let label = if src.starts_with("/dev/") { labels.get(&canonical(&src)).cloned().unwrap_or_default() } else { String::new() };
        out.push(Drive { root: point, letter: '\0', label, source: src, kind, total, free, fs });
    }
    // the shares in fstab that aren't mounted (yet), where they will be
    for (point, src, fs) in &fstab {
        if seen.insert(src.clone()) {
            out.push(Drive { root: point.clone(), letter: '\0', label: String::new(), source: src.clone(), kind: DriveKind::Network, total: 0, free: 0, fs: fs.clone() });
        }
    }
    // shares last, in fstab's order (automounts come up in whatever order they're opened)
    let rank = |d: &Drive| fstab.iter().position(|(_, s, _)| *s == d.source).unwrap_or(usize::MAX);
    out.sort_by_key(|d| if d.kind == DriveKind::Network { (1, rank(d)) } else { (0, 0) });
    out
}

/// The network shares in fstab (`//nas/media /mnt/media cifs …`): (mount point, source, fs
/// type), in the file's order.
fn fstab_network(text: &str) -> Vec<(PathBuf, String, String)> {
    text.lines()
        .filter_map(|l| {
            let l = l.trim();
            if l.starts_with('#') {
                return None;
            }
            let mut f = l.split_whitespace();
            let (src, point, fs) = (f.next()?, f.next()?, f.next()?);
            NETWORK_FS.contains(&fs).then(|| (PathBuf::from(unescape(point)), unescape(src), fs.to_string()))
        })
        .collect()
}

/// A systemd automount (`x-systemd.automount`) is an `autofs` mount holding the place of an
/// fstab share until something opens the folder: it stands for that share (fs type, source).
fn as_share(point: &Path, fs: String, src: String, fstab: &[(PathBuf, String, String)]) -> (String, String) {
    match fstab.iter().find(|(p, _, _)| p == point) {
        Some((_, s, f)) if fs == "autofs" => (f.clone(), s.clone()),
        _ => (fs, src),
    }
}

fn canonical(dev: &str) -> PathBuf {
    std::fs::canonicalize(dev).unwrap_or_else(|_| PathBuf::from(dev))
}

/// File system labels by device (`/dev/sdb1` → "KINGSTON"), from udev's `/dev/disk/by-label`.
fn labels() -> std::collections::HashMap<PathBuf, String> {
    let Ok(dir) = std::fs::read_dir("/dev/disk/by-label") else { return Default::default() };
    dir.flatten()
        .filter_map(|e| {
            let dev = std::fs::canonicalize(e.path()).ok()?;
            Some((dev, unescape_udev(&e.file_name().to_string_lossy())))
        })
        .collect()
}

/// udev escapes characters in link names as `\xNN` (a space is `\x20`).
fn unescape_udev(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'\\'
            && b.get(i + 1) == Some(&b'x')
            && let Some(v) = s.get(i + 2..i + 4).and_then(|h| u8::from_str_radix(h, 16).ok())
        {
            out.push(v);
            i += 4;
            continue;
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Whether `path` is on a removable drive (USB stick, SD card).
pub fn is_removable(path: &Path) -> bool {
    list_drives()
        .into_iter()
        .filter(|d| path.starts_with(&d.root))
        .max_by_key(|d| d.root.as_os_str().len())
        .is_some_and(|d| d.kind == DriveKind::Removable)
}

/// Drives that are connected but not mounted (a USB stick nothing mounted yet, the Windows
/// partition of a dual-boot disk), from UDisks. Their `root` is the device (`/dev/sdb1`):
/// opening it mounts it (`udisks::mount`). Talks to the system bus: off the UI thread.
pub fn unmounted_drives() -> Vec<Drive> {
    crate::udisks::unmounted()
        .into_iter()
        .map(|v| {
            let kind = if v.optical {
                DriveKind::Optical
            } else if v.removable || v.loop_dev {
                DriveKind::Removable
            } else {
                DriveKind::Fixed
            };
            let source = v.device.to_string_lossy().into_owned();
            Drive { root: v.device, letter: '\0', label: v.label, source, kind, total: v.size, free: 0, fs: v.fs }
        })
        .collect()
}

/// Whether `d` is a connected drive that isn't mounted (see [`unmounted_drives`]).
pub fn is_unmounted(d: &Drive) -> bool {
    d.root.starts_with("/dev/")
}

/// Changes when drives come and go (cheap; polled to notice USB plug/unplug). Linux has no
/// letter bitmask: a hash of the mount list stands in.
pub fn drive_mask() -> u32 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    // network shares and automounts too: a share mounts when something first opens it
    for (p, fs, src) in mounts() {
        if src.starts_with("/dev/") || fs == "autofs" || NETWORK_FS.contains(&fs.as_str()) {
            (p, fs).hash(&mut h);
        }
    }
    // block devices too: a plugged-in stick shows up before anything mounts it
    if let Ok(dir) = std::fs::read_dir("/sys/class/block") {
        let mut names: Vec<_> = dir.flatten().map(|e| e.file_name()).collect();
        names.sort();
        names.hash(&mut h);
    }
    h.finish() as u32
}

/// The mount holding `path` (the longest mount point it is under): (mount point, file
/// system type, source device or share).
pub(crate) fn mount_of(path: &Path) -> Option<(PathBuf, String, String)> {
    mounts().into_iter().filter(|(point, _, _)| path.starts_with(point)).max_by_key(|(point, _, _)| point.as_os_str().len())
}

/// The disk under a block device (`/dev/nvme0n1p2` → `/sys/class/block/nvme0n1`): partitions
/// sit inside their disk's sysfs folder.
fn disk_sysfs(source: &str) -> Option<PathBuf> {
    let dev = std::fs::canonicalize(source).ok()?;
    let name = dev.file_name()?.to_owned();
    let sys = std::fs::canonicalize(Path::new("/sys/class/block").join(name)).ok()?;
    if sys.join("partition").exists() { sys.parent().map(Path::to_path_buf) } else { Some(sys) }
}

/// Whether the disk holding `path` spins (`queue/rotational` of its disk in sysfs). `None` for
/// network shares, RAM disks and stacked devices (LVM, LUKS) that report nothing useful.
/// Goes through the mount table rather than `st_dev`: btrfs gives every subvolume an anonymous
/// device number.
pub fn seek_penalty(path: &Path) -> Option<bool> {
    let (_, _, source) = mount_of(path)?;
    if !source.starts_with("/dev/") {
        return None;
    }
    let rot = std::fs::read_to_string(disk_sysfs(&source)?.join("queue/rotational")).ok()?;
    match rot.trim() {
        "1" => Some(true),
        "0" => Some(false),
        _ => None,
    }
}

/// Whether something is mounted at `point` beyond a systemd automount waiting for its first
/// use (`mounts` from [`mounts`]). Looking inside an unmounted automount mounts it, which for
/// an offline share waits out the mount timeout.
pub(crate) fn really_mounted(point: &Path, mounts: &[(PathBuf, String, String)]) -> bool {
    mounts.iter().any(|(m, fs, _)| m == point && fs != "autofs")
}

/// Mount points holding only a systemd automount that nothing has opened yet (fstab shares
/// away from their network stay like this). Anything under one is unknown until it mounts:
/// a `stat` there waits for the mount.
pub fn sleeping_automounts() -> Vec<PathBuf> {
    let ms = mounts();
    let mut v: Vec<PathBuf> = ms.iter().filter(|(m, fs, _)| fs == "autofs" && !really_mounted(m, &ms)).map(|(m, _, _)| m.clone()).collect();
    v.dedup();
    v
}

/// The server of an fstab share that `path` is on, when that share isn't mounted yet and the
/// server doesn't answer: `Some("10.0.0.9")`. Opening such a folder makes systemd try to mount
/// it, and the call waits for the whole mount timeout (10 s or more) before failing. A short
/// TCP connect to the share's port finds out first. Only SMB (445, or the entry's `port=`) and
/// NFS (2049) shares are checked; anything else, or a mounted share, gives `None`. Blocks for
/// up to `wait`: call off the UI thread.
pub fn unreachable_share(path: &Path, wait: std::time::Duration) -> Option<String> {
    let ms = mounts();
    // the innermost mount point holding the path, and whether only the automount sits there
    let point = ms.iter().filter(|(m, _, _)| path.starts_with(m)).map(|(m, _, _)| m).max_by_key(|m| m.components().count())?;
    if really_mounted(point, &ms) {
        return None;
    }
    let (host, port) = share_server(&std::fs::read_to_string("/etc/fstab").unwrap_or_default(), point)?;
    let addrs: Vec<std::net::SocketAddr> = std::net::ToSocketAddrs::to_socket_addrs(&(host.as_str(), port)).ok()?.collect();
    let answers = addrs.iter().any(|a| match std::net::TcpStream::connect_timeout(a, wait) {
        Ok(_) => true,
        // refused: the host is there (the mount fails fast on its own)
        Err(e) => e.kind() == std::io::ErrorKind::ConnectionRefused,
    });
    (!answers && !addrs.is_empty()).then_some(host)
}

/// The shares among `roots` that [`unreachable_share`] finds offline, each with its server,
/// checked side by side (the whole call takes at most `wait`). Blocks: call off the UI thread.
pub fn offline_shares(roots: &[PathBuf], wait: std::time::Duration) -> Vec<(PathBuf, String)> {
    std::thread::scope(|s| {
        let probes: Vec<_> = roots.iter().map(|r| (r, s.spawn(move || unreachable_share(r, wait)))).collect();
        probes.into_iter().filter_map(|(r, h)| Some((r.clone(), h.join().ok()??))).collect()
    })
}

/// (host, port) of the fstab share mounted at `point`, for the share kinds whose port is known.
fn share_server(fstab: &str, point: &Path) -> Option<(String, u16)> {
    fstab.lines().map(str::trim).filter(|l| !l.starts_with('#')).find_map(|l| {
        let f: Vec<&str> = l.split_whitespace().collect();
        let (src, at, fs, opts) = (f.first()?, f.get(1)?, f.get(2)?, f.get(3).copied().unwrap_or(""));
        if Path::new(&unescape(at)) != point {
            return None;
        }
        let src = unescape(src);
        let opt_port = opts.split(',').find_map(|o| o.strip_prefix("port=")?.parse().ok());
        let (host, port) = match *fs {
            // //host/share
            "cifs" | "smb3" | "smbfs" => (src.trim_start_matches('/').split('/').next()?.to_string(), opt_port.unwrap_or(445)),
            // host:/export, [v6addr]:/export
            "nfs" | "nfs4" => (src.rsplit_once(":/")?.0.trim_matches(['[', ']']).to_string(), opt_port.unwrap_or(2049)),
            _ => return None,
        };
        (!host.is_empty()).then_some((host, port))
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn local_folders_are_never_offline() {
        let roots = vec![std::path::PathBuf::from("/"), std::path::PathBuf::from("/usr")];
        let t = std::time::Instant::now();
        assert!(super::offline_shares(&roots, std::time::Duration::from_millis(200)).is_empty());
        assert!(t.elapsed() < std::time::Duration::from_millis(200), "no probe for a local disk");
    }

    #[test]
    fn share_servers_from_fstab() {
        let fstab = "//10.0.0.9/media /mnt/media cifs x-systemd.automount,_netdev 0 0\n\
                     //nas/odd /mnt/odd cifs port=4445,noauto 0 0\n\
                     nas.lan:/export/My\\040Files /mnt/my\\040files nfs4 noauto 0 0\n\
                     [fd00::9]:/srv /mnt/v6 nfs defaults 0 0\n\
                     me@box:/home /mnt/box fuse.sshfs noauto 0 0\n";
        let at = |p: &str| super::share_server(fstab, std::path::Path::new(p));
        assert_eq!(at("/mnt/media"), Some(("10.0.0.9".into(), 445)));
        assert_eq!(at("/mnt/odd"), Some(("nas".into(), 4445)));
        assert_eq!(at("/mnt/my files"), Some(("nas.lan".into(), 2049)));
        assert_eq!(at("/mnt/v6"), Some(("fd00::9".into(), 2049)));
        assert_eq!(at("/mnt/box"), None);
        assert_eq!(at("/mnt/other"), None);
    }

    #[test]
    fn fstab_shares_and_their_automounts() {
        let fstab = super::fstab_network(
            "# TrueNAS\n//10.0.0.9/media /mnt/media cifs x-systemd.automount,_netdev 0 0\n\
             UUID=abcd / btrfs defaults 0 0\n\
             nas:/export/My\\040Files /mnt/my\\040files nfs4 noauto 0 0\n\
             #//10.0.0.9/old /mnt/old cifs defaults 0 0\n",
        );
        assert_eq!(
            fstab,
            vec![
                ("/mnt/media".into(), "//10.0.0.9/media".to_string(), "cifs".to_string()),
                ("/mnt/my files".into(), "nas:/export/My Files".to_string(), "nfs4".to_string()),
            ]
        );
        let p = std::path::Path::new("/mnt/media");
        // the automount stands for the share; the share itself, once mounted, is itself
        assert_eq!(super::as_share(p, "autofs".into(), "systemd-1".into(), &fstab), ("cifs".to_string(), "//10.0.0.9/media".to_string()));
        assert_eq!(super::as_share(p, "cifs".into(), "//10.0.0.9/media".into(), &fstab), ("cifs".to_string(), "//10.0.0.9/media".to_string()));
        assert_eq!(super::as_share(std::path::Path::new("/proc/sys/fs/binfmt_misc"), "autofs".into(), "systemd-1".into(), &fstab).0, "autofs");
    }


    /// The system disk and the home folder read quickly; a FUSE network mount, if one is
    /// mounted here, does not.
    #[test]
    fn local_disks_are_not_slow() {
        assert!(!slow_to_read(Path::new("/")));
        assert!(!slow_to_read(Path::new("/usr/bin")));
        for (point, fs, _) in mounts() {
            if fs == "fuse.kio-fuse" || fs == "fuse.gvfsd-fuse" || fs == "cifs" {
                assert!(slow_to_read(&point.join("x")), "{} ({fs})", point.display());
            }
        }
    }

    use super::*;

    #[test]
    fn root_is_listed() {
        let d = list_drives();
        let root = d.iter().find(|d| d.root == Path::new("/")).expect("root mount");
        assert!(root.total > 0 && root.free <= root.total, "{root:?}");
        assert!(root.source.starts_with("/dev/"), "{root:?}");
        // a labelled root (CI's is "cloudimg-rootfs") shows its label
        let want = if root.label.is_empty() { "File System" } else { root.label.as_str() };
        assert_eq!(root.display_name(), want);
    }

    #[test]
    fn udev_label_escapes() {
        assert_eq!(unescape_udev(r"My\x20Disk"), "My Disk");
        assert_eq!(unescape_udev(r"a\x2fb\xzz"), r"a/b\xzz");
        assert_eq!(unescape_udev("KINGSTON"), "KINGSTON");
    }

    #[test]
    fn root_has_a_mount_and_a_disk_answer() {
        let (point, _, _) = mount_of(Path::new("/usr")).expect("a mount for /usr");
        assert!(Path::new("/usr").starts_with(&point));
        // a real disk answers; a container or tmpfs root may not
        let _ = seek_penalty(Path::new("/"));
        assert_eq!(seek_penalty(Path::new("/proc/self")), None);
    }

    #[test]
    fn unescapes_mountinfo() {
        assert_eq!(unescape(r"/run/media/me/My\040Disk"), "/run/media/me/My Disk");
        assert_eq!(unescape(r"/a\\b"), r"/a\\b");
    }
}
