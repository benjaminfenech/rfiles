//! Mounting, unmounting and ejecting drives through UDisks2 (the system service desktops use
//! for this: no root needed for removable media; internal disks may ask for a password through
//! the desktop's polkit agent).
//!
//! Everything here blocks on the system bus (an unmount flushes caches and can take seconds):
//! call off the UI thread.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, anyhow};
use zbus::blocking::{Connection, Proxy};
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};

const DEST: &str = "org.freedesktop.UDisks2";
const BLOCK: &str = "org.freedesktop.UDisks2.Block";
const FS: &str = "org.freedesktop.UDisks2.Filesystem";
const DRIVE: &str = "org.freedesktop.UDisks2.Drive";
const ENCRYPTED: &str = "org.freedesktop.UDisks2.Encrypted";
const LOOP: &str = "org.freedesktop.UDisks2.Loop";

/// A file system UDisks knows about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Volume {
    /// `/dev/sdb1`
    pub device: PathBuf,
    pub label: String,
    /// `vfat`, `ntfs`, `ext4`…
    pub fs: String,
    pub size: u64,
    pub mount_points: Vec<PathBuf>,
    /// On a USB stick, SD card or disc (the drive says it can be removed).
    pub removable: bool,
    pub optical: bool,
    /// UDisks' advice: hide it (recovery and reserved partitions, swap…).
    pub ignore: bool,
    object: OwnedObjectPath,
    drive: Option<OwnedObjectPath>,
    /// The locked LUKS device this one is the unlocked view of.
    crypto_backing: Option<OwnedObjectPath>,
    /// A loop device (a disk image).
    pub loop_dev: bool,
    /// A LUKS volume nobody has unlocked yet: mounting it needs its passphrase
    /// ([`unlock_and_mount`]). `fs` is `crypto_LUKS`.
    pub locked: bool,
}

/// [`mount`] of a locked LUKS volume: ask for the passphrase, then [`unlock_and_mount`].
#[derive(Debug)]
pub struct NeedsPassphrase;

impl std::fmt::Display for NeedsPassphrase {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("This drive is encrypted. Enter its passphrase to open it.")
    }
}

impl std::error::Error for NeedsPassphrase {}

/// [`unlock_and_mount`] with the wrong passphrase.
#[derive(Debug)]
pub struct WrongPassphrase;

impl std::fmt::Display for WrongPassphrase {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Wrong passphrase.")
    }
}

impl std::error::Error for WrongPassphrase {}

fn conn() -> anyhow::Result<Connection> {
    static C: std::sync::Mutex<Option<Connection>> = std::sync::Mutex::new(None);
    let mut c = C.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(c) = &*c {
        return Ok(c.clone());
    }
    let new = Connection::system().context("system bus")?;
    *c = Some(new.clone());
    Ok(new)
}

type Props = HashMap<String, OwnedValue>;

fn prop<'a>(p: &'a Props, k: &str) -> Option<&'a Value<'a>> {
    p.get(k).map(|v| &**v)
}

fn s(p: &Props, k: &str) -> String {
    match prop(p, k) {
        Some(Value::Str(v)) => v.to_string(),
        _ => String::new(),
    }
}

fn b(p: &Props, k: &str) -> bool {
    matches!(prop(p, k), Some(Value::Bool(true)))
}

fn u(p: &Props, k: &str) -> u64 {
    match prop(p, k) {
        Some(Value::U64(v)) => *v,
        _ => 0,
    }
}

fn obj(p: &Props, k: &str) -> Option<OwnedObjectPath> {
    match prop(p, k) {
        Some(Value::ObjectPath(o)) if o.as_str() != "/" => Some(o.clone().into()),
        _ => None,
    }
}

/// A NUL-terminated byte string (`ay`), as UDisks sends paths.
fn bytes_path(v: &Value) -> Option<PathBuf> {
    use std::os::unix::ffi::OsStringExt;
    let Value::Array(a) = v else { return None };
    let mut out: Vec<u8> = a.iter().filter_map(|x| if let Value::U8(c) = x { Some(*c) } else { None }).collect();
    while out.last() == Some(&0) {
        out.pop();
    }
    (!out.is_empty()).then(|| PathBuf::from(std::ffi::OsString::from_vec(out)))
}

fn mount_points(p: &Props) -> Vec<PathBuf> {
    match prop(p, "MountPoints") {
        Some(Value::Array(a)) => a.iter().filter_map(bytes_path).collect(),
        _ => Vec::new(),
    }
}

/// Every file system on the machine (mounted or not), as UDisks sees them.
pub fn volumes() -> anyhow::Result<Vec<Volume>> {
    let c = conn()?;
    let om = zbus::blocking::fdo::ObjectManagerProxy::builder(&c)
        .destination(DEST)?
        .path("/org/freedesktop/UDisks2")?
        .build()?;
    let objects = om.get_managed_objects().context("UDisks2")?;
    let iface = |o: &OwnedObjectPath, i: &str| -> Option<&Props> {
        objects.get(o)?.iter().find(|(n, _)| n.as_str() == i).map(|(_, p)| p)
    };
    let mut out = Vec::new();
    for (path, ifaces) in &objects {
        let get = |i: &str| ifaces.iter().find(|(n, _)| n.as_str() == i).map(|(_, p)| p);
        // a locked LUKS volume: no file system to see until it's unlocked (an unlocked one
        // shows as its cleartext device, below)
        if let (Some(block), Some(enc), None) = (get(BLOCK), get(ENCRYPTED), get(FS)) {
            if obj(enc, "CleartextDevice").is_none_or(|c| c.as_str() == "/") {
                let drive = obj(block, "Drive");
                let drive_props = drive.as_ref().and_then(|d| iface(d, DRIVE));
                out.push(Volume {
                    device: prop(block, "PreferredDevice").and_then(bytes_path).or_else(|| prop(block, "Device").and_then(bytes_path)).unwrap_or_default(),
                    label: s(block, "IdLabel"),
                    fs: s(block, "IdType"),
                    size: u(block, "Size"),
                    mount_points: Vec::new(),
                    removable: drive_props.is_some_and(|d| b(d, "Removable") || b(d, "MediaRemovable") || b(d, "Ejectable")),
                    optical: false,
                    ignore: b(block, "HintIgnore"),
                    object: path.clone(),
                    drive,
                    crypto_backing: None,
                    loop_dev: get(LOOP).is_some(),
                    locked: true,
                });
            }
            continue;
        }
        let (Some(block), Some(fs)) = (get(BLOCK), get(FS)) else { continue };
        let backing = obj(block, "CryptoBackingDevice");
        // an unlocked LUKS volume's drive is its locked backing device's
        let drive = obj(block, "Drive").or_else(|| obj(iface(backing.as_ref()?, BLOCK)?, "Drive"));
        let drive_props = drive.as_ref().and_then(|d| iface(d, DRIVE));
        let removable = drive_props.is_some_and(|d| b(d, "Removable") || b(d, "MediaRemovable") || b(d, "Ejectable"));
        let optical = drive_props.is_some_and(|d| b(d, "Optical") || s(d, "Media").starts_with("optical"));
        let device = match prop(block, "PreferredDevice").and_then(bytes_path) {
            Some(p) if !p.starts_with("/dev/mapper") || backing.is_none() => p,
            _ => prop(block, "Device").and_then(bytes_path).unwrap_or_default(),
        };
        let mut label = s(block, "IdLabel");
        if label.is_empty() {
            label = s(block, "HintName");
        }
        // an unlocked LUKS disk image: the image is its backing device's loop
        let loop_dev = get(LOOP).is_some() || backing.as_ref().is_some_and(|bk| iface(bk, LOOP).is_some());
        out.push(Volume {
            device,
            label,
            fs: s(block, "IdType"),
            size: u(block, "Size"),
            mount_points: mount_points(fs),
            removable,
            optical,
            ignore: b(block, "HintIgnore") || s(block, "IdUsage") != "filesystem",
            object: path.clone(),
            drive,
            crypto_backing: backing,
            loop_dev,
            locked: false,
        });
    }
    out.sort_by(|a, b| a.device.cmp(&b.device));
    Ok(out)
}

/// File systems worth offering to mount: not mounted, not hidden by UDisks. Empty when UDisks
/// isn't running.
pub fn unmounted() -> Vec<Volume> {
    volumes().map(|v| v.into_iter().filter(|v| !v.ignore && v.mount_points.is_empty()).collect()).unwrap_or_default()
}

fn no_options() -> HashMap<&'static str, Value<'static>> {
    HashMap::new()
}

fn proxy<'a>(c: &'a Connection, path: &'a OwnedObjectPath, iface: &'a str) -> anyhow::Result<Proxy<'a>> {
    Ok(Proxy::new(c, DEST, path.as_str(), iface)?)
}

/// UDisks' error as a sentence for the user.
fn explain(e: zbus::Error, what: &Path) -> anyhow::Error {
    if let zbus::Error::MethodError(name, msg, _) = &e {
        let msg = msg.clone().unwrap_or_default();
        return match name.as_str() {
            "org.freedesktop.UDisks2.Error.DeviceBusy" => {
                let users = users_of(what);
                if users.is_empty() {
                    anyhow!("A program is still using {}.", what.display())
                } else {
                    anyhow!("{} is still in use by: {}.", what.display(), users.join(", "))
                }
            }
            "org.freedesktop.UDisks2.Error.NotAuthorizedCanObtain" | "org.freedesktop.UDisks2.Error.NotAuthorized" => {
                anyhow!("Your system asks for an administrator password for this drive, and no password prompt (polkit agent) is running.")
            }
            "org.freedesktop.UDisks2.Error.NotAuthorizedDismissed" => anyhow!("Cancelled."),
            _ if !msg.is_empty() => anyhow!("{msg}"),
            _ => anyhow!("{name}"),
        };
    }
    anyhow!(e)
}

/// Mounts `device` (`/dev/sdb1`) and returns where (usually `/run/media/<user>/<label>`).
/// Already mounted: its mount point.
pub fn mount(device: &Path) -> anyhow::Result<PathBuf> {
    let all = volumes()?;
    let canon = std::fs::canonicalize(device).unwrap_or_else(|_| device.to_path_buf());
    let v = all
        .iter()
        .find(|v| v.device == device || std::fs::canonicalize(&v.device).is_ok_and(|d| d == canon))
        .ok_or_else(|| anyhow!("{} is not a drive UDisks can mount.", device.display()))?;
    if let Some(p) = v.mount_points.first() {
        return Ok(p.clone());
    }
    if v.locked {
        return Err(NeedsPassphrase.into());
    }
    let c = conn()?;
    let r: String = proxy(&c, &v.object, FS)?.call("Mount", &(no_options(),)).map_err(|e| explain(e, device))?;
    Ok(PathBuf::from(r))
}

/// Unlocks the LUKS volume `device` with `passphrase`, then mounts the file system inside and
/// returns where. [`WrongPassphrase`] when it doesn't fit.
pub fn unlock_and_mount(device: &Path, passphrase: &str) -> anyhow::Result<PathBuf> {
    let all = volumes()?;
    let v = all
        .iter()
        .find(|v| v.locked && v.device == device)
        .ok_or_else(|| anyhow!("{} is not a locked encrypted drive.", device.display()))?;
    let c = conn()?;
    let clear: OwnedObjectPath = proxy(&c, &v.object, ENCRYPTED)?.call("Unlock", &(passphrase, no_options())).map_err(|e| match &e {
        zbus::Error::MethodError(_, Some(msg), _) if is_wrong_passphrase(msg) => anyhow::Error::new(WrongPassphrase),
        _ => explain(e, device),
    })?;
    // the cleartext device's file system shows up in UDisks a moment after the unlock
    for _ in 0..30 {
        if let Some(inner) = volumes()?.into_iter().find(|v| v.object == clear) {
            if let Some(p) = inner.mount_points.first() {
                return Ok(p.clone());
            }
            let r: String = proxy(&c, &inner.object, FS)?.call("Mount", &(no_options(),)).map_err(|e| explain(e, device))?;
            return Ok(PathBuf::from(r));
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    Err(anyhow!("{} is unlocked, but there's no file system inside that rfiles can open (LVM or another layer).", device.display()))
}

/// UDisks' wording for a passphrase that doesn't fit (cryptsetup's, passed on).
fn is_wrong_passphrase(msg: &str) -> bool {
    let m = msg.to_ascii_lowercase();
    m.contains("no key available") || (m.contains("passphrase") && (m.contains("incorrect") || m.contains("wrong")))
}

/// The volume mounted at `root`.
fn at(all: &[Volume], root: &Path) -> anyhow::Result<Volume> {
    all.iter()
        .find(|v| v.mount_points.iter().any(|m| m == root))
        .cloned()
        .ok_or_else(|| anyhow!("{} wasn't mounted through UDisks; unmount it the way it was mounted.", root.display()))
}

/// Unmounts every mount of `v`, then locks it if it was an unlocked LUKS volume.
fn unmount_volume(c: &Connection, v: &Volume) -> anyhow::Result<()> {
    if !v.mount_points.is_empty() {
        let what = v.mount_points[0].clone();
        let () = proxy(c, &v.object, FS)?.call("Unmount", &(no_options(),)).map_err(|e| explain(e, &what))?;
    }
    if let Some(bk) = &v.crypto_backing {
        let () = proxy(c, bk, ENCRYPTED)?.call("Lock", &(no_options(),)).map_err(|e| explain(e, &v.device))?;
    }
    Ok(())
}

/// Whether UDisks can unmount the drive mounted at `root` (not `/`, `/home` and other
/// mounts from `/etc/fstab` the system needs).
pub fn can_unmount(root: &Path) -> bool {
    volumes().is_ok_and(|all| all.iter().any(|v| v.mount_points.iter().any(|m| m == root)))
        && !["/", "/home", "/boot", "/efi", "/usr", "/var"].iter().any(|s| root == Path::new(s))
}

/// Whether the drive at `root` can be ejected (a USB stick, SD card, disc or disk image).
pub fn can_eject(root: &Path) -> bool {
    volumes().ok().and_then(|all| at(&all, root).ok()).is_some_and(|v| v.removable || v.optical || v.loop_dev)
}

/// Unmounts the file system at `root` (the drive stays connected; it can be mounted again).
pub fn unmount(root: &Path) -> anyhow::Result<()> {
    let all = volumes()?;
    let v = at(&all, root)?;
    unmount_volume(&conn()?, &v)
}

/// "Safely remove": unmounts every file system on the drive holding `root` (other
/// partitions of the stick too), then ejects a disc or powers the drive down so it can be
/// pulled out. A disk image (loop device) is detached instead.
pub fn eject(root: &Path) -> anyhow::Result<()> {
    let all = volumes()?;
    let v = at(&all, root)?;
    let c = conn()?;
    let Some(drive) = v.drive.clone() else {
        unmount_volume(&c, &v)?;
        if v.loop_dev {
            // UDisks deletes a loop device it set up once unmounted (autoclear); others: ask
            let image = v.crypto_backing.as_ref().unwrap_or(&v.object);
            let _ = proxy(&c, image, LOOP).and_then(|p| Ok(p.call::<_, _, ()>("Delete", &(no_options(),))?));
        }
        return Ok(());
    };
    for other in all.iter().filter(|o| o.drive.as_ref() == Some(&drive)) {
        unmount_volume(&c, other)?;
    }
    let d = proxy(&c, &drive, DRIVE)?;
    let ejectable: bool = d.get_property("Ejectable").unwrap_or(false);
    let can_power_off: bool = d.get_property("CanPowerOff").unwrap_or(false);
    if ejectable {
        let () = d.call("Eject", &(no_options(),)).map_err(|e| explain(e, root))?;
    }
    if can_power_off && !v.optical {
        // a stick that won't power down is still safe to pull once unmounted
        if let Err(e) = d.call::<_, _, ()>("PowerOff", &(no_options(),)) {
            log::info!("power off {}: {e}", v.device.display());
        }
    }
    Ok(())
}

/// Programs of ours (same user) with files open or their working folder under `root`, for
/// "still in use by" messages. Not this app.
pub fn users_of(root: &Path) -> Vec<String> {
    let me = std::process::id().to_string();
    let mut out: Vec<String> = Vec::new();
    let Ok(procs) = std::fs::read_dir("/proc") else { return out };
    for p in procs.flatten() {
        let pid = p.file_name();
        let pid = pid.to_string_lossy();
        if !pid.bytes().all(|c| c.is_ascii_digit()) || pid == me {
            continue;
        }
        let dir = p.path();
        let under = |l: std::io::Result<PathBuf>| l.is_ok_and(|t| t.starts_with(root));
        let busy = under(std::fs::read_link(dir.join("cwd")))
            || std::fs::read_dir(dir.join("fd")).is_ok_and(|fds| fds.flatten().any(|f| under(std::fs::read_link(f.path()))));
        if busy {
            let name = std::fs::read_to_string(dir.join("comm")).map(|s| s.trim().to_string()).unwrap_or_else(|_| pid.to_string());
            if !out.contains(&name) {
                out.push(name);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wrong_passphrase_messages() {
        assert!(super::is_wrong_passphrase("Error unlocking /dev/sdb1: Failed to activate device: Incorrect passphrase"));
        assert!(super::is_wrong_passphrase("Error unlocking /dev/sdb1: Failed to activate device: Operation not permitted (No key available with this passphrase)"));
        assert!(!super::is_wrong_passphrase("Error unlocking /dev/sdb1: Device or resource busy"));
    }

    #[test]
    fn byte_paths() {
        let v = Value::from(vec![b'/', b'm', b'n', b't', 0u8]);
        assert_eq!(bytes_path(&v), Some(PathBuf::from("/mnt")));
        assert_eq!(bytes_path(&Value::from(vec![0u8])), None);
    }

    #[test]
    fn finds_who_uses_a_folder() {
        let dir = std::env::temp_dir().join(format!("rf-busy-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut child = std::process::Command::new("sleep").arg("30").current_dir(&dir).spawn().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(100));
        let users = users_of(&dir);
        let _ = child.kill();
        let _ = child.wait();
        let _ = std::fs::remove_dir_all(&dir);
        assert!(users.iter().any(|u| u == "sleep"), "{users:?}");
    }
}
