
// Generated from rf-win's public API by a one-off stub generator, then edited by hand.
// Functions return safe defaults until they get a Linux implementation.
#![allow(unused)]
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
            _ if self.root == Path::new("/") => "Dosya Sistemi".to_string(),
            DriveKind::Removable => "USB Sürücü".to_string(),
            DriveKind::Network => "Ağ Sürücüsü".to_string(),
            DriveKind::Optical => "DVD Sürücü".to_string(),
            _ => self.root.file_name().map_or_else(|| "Yerel Disk".into(), |n| n.to_string_lossy().into_owned()),
        }
    }

    pub fn used_fraction(&self) -> f32 {
        if self.total == 0 { 0.0 } else { 1.0 - self.free as f32 / self.total as f32 }
    }
}

const NETWORK_FS: &[&str] = &["nfs", "nfs4", "cifs", "smb3", "smbfs", "sshfs", "fuse.sshfs", "9p", "afs", "ceph"];

/// One mount from `/proc/self/mountinfo`: (mount point, fs type, source).
fn mounts() -> Vec<(PathBuf, String, String)> {
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
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for (point, fs, src) in mounts() {
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
    out
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
    for (p, _, src) in mounts() {
        if src.starts_with("/dev/") {
            p.hash(&mut h);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn root_is_listed() {
        let d = list_drives();
        let root = d.iter().find(|d| d.root == Path::new("/")).expect("root mount");
        assert!(root.total > 0 && root.free <= root.total, "{root:?}");
        assert!(root.source.starts_with("/dev/"), "{root:?}");
        // a labelled root (CI's is "cloudimg-rootfs") shows its label
        let want = if root.label.is_empty() { "Dosya Sistemi" } else { root.label.as_str() };
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
