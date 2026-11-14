//! Phones, tablets and cameras plugged in over USB. They have no file system to mount: they
//! speak MTP (Android), PTP (cameras, phones in "photo" mode) or Apple's AFC (iPhone, iPad),
//! and something has to turn them into folders:
//! - gvfs (`gvfs-mtp`, `gvfs-gphoto2`, `gvfs-afc`): `mtp://<serial>/`, `gphoto2://[usb:bus,dev]/`,
//!   `afc://<udid>/`, through [`crate::remote::mount`].
//! - Without gvfs, a FUSE tool if one is installed ([`fuse_mount`]): `jmtpfs` for phones,
//!   `ifuse` for iPhones and iPads, `gphotofs` for cameras.
//!
//! KDE's kio-extras can show phones in Dolphin, but kio-fuse refuses to mount them (KIO counts
//! `mtp:` as a local protocol), so it can't help here.
//!
//! Found through udev's database (`/run/udev/data`), which libmtp's and libgphoto2's rules
//! fill in, so no daemon has to be running to list them.

use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceKind {
    Phone,
    Camera,
    Apple,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Device {
    pub name: String,
    pub kind: DeviceKind,
    /// What opens it: a URL [`crate::remote::mount`] understands.
    pub url: String,
}

/// What udev knows about one USB device (its `E:` lines), plus sysfs's names for it.
#[derive(Debug, Default, Clone)]
struct UsbInfo {
    props: Vec<(String, String)>,
    bus: u32,
    dev: u32,
    product: String,
    manufacturer: String,
}

impl UsbInfo {
    fn get(&self, key: &str) -> Option<&str> {
        self.props.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
    }
}

/// The phones and cameras plugged in now.
pub fn list() -> Vec<Device> {
    usb_devices().iter().filter_map(classify).collect()
}

/// Mounts a phone, camera or iPhone (its URL from [`list`]) with a FUSE tool, for when gvfs
/// isn't there: under `$XDG_RUNTIME_DIR/rfiles-devices/`. Fails with what to install when
/// no tool is.
pub fn fuse_mount(url: &str) -> anyhow::Result<std::path::PathBuf> {
    use anyhow::{anyhow, bail};
    let (scheme, rest) = url.split_once("://").ok_or_else(|| anyhow!("{url} is not a device"))?;
    let host = rest.split('/').next().unwrap_or("");
    let dev = usb_devices().into_iter().find(|d| classify(d).is_some_and(|c| c.url == url)).ok_or_else(|| anyhow!("The device isn't plugged in any more."))?;
    let (tool, package, args): (&str, &str, Vec<String>) = match scheme {
        "mtp" => ("jmtpfs", "gvfs-mtp", vec![format!("-device={},{}", dev.bus, dev.dev)]),
        "afc" => ("ifuse", "gvfs-afc", vec!["-u".into(), host.to_string()]),
        "gphoto2" => ("gphotofs", "gvfs-gphoto2", vec![format!("--port=usb:{:03},{:03}", dev.bus, dev.dev)]),
        _ => bail!("{url} is not a device"),
    };
    let base = dirs::runtime_dir().unwrap_or_else(std::env::temp_dir).join("rfiles-devices");
    let dir = base.join(host.trim_matches(|c| c == '[' || c == ']').replace([':', ','], "_"));
    // mounted earlier (and still there)
    if crate::drives::mounts().iter().any(|(p, fs, _)| *p == dir && fs.starts_with("fuse")) {
        return Ok(dir);
    }
    std::fs::create_dir_all(&dir)?;
    let mut c = crate::sandbox::tool(tool, &[]);
    c.args(&args).arg(&dir);
    let out = match crate::tool::run(c, std::time::Duration::from_secs(60)) {
        Ok(o) => o,
        Err(e) if e.is::<crate::tool::NotInstalled>() => {
            let _ = std::fs::remove_dir(&dir);
            bail!("Opening this device needs {package} (or {tool}). Install it, then try again.");
        }
        Err(e) => return Err(e),
    };
    if !out.ok {
        let _ = std::fs::remove_dir(&dir);
        let err = String::from_utf8_lossy(&out.stderr);
        bail!("{}", err.lines().map(str::trim).filter(|l| !l.is_empty()).last().unwrap_or("The device could not be opened."));
    }
    Ok(dir)
}

/// Changes when USB devices come and go (cheap; polled with `drives::drive_mask`).
pub fn usb_stamp() -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    if let Ok(dir) = std::fs::read_dir("/sys/bus/usb/devices") {
        let mut names: Vec<_> = dir.flatten().map(|e| e.file_name()).collect();
        names.sort();
        names.hash(&mut h);
    }
    h.finish()
}

fn usb_devices() -> Vec<UsbInfo> {
    let Ok(dir) = std::fs::read_dir("/sys/bus/usb/devices") else { return Vec::new() };
    dir.flatten()
        .filter_map(|e| {
            let sys = e.path();
            let read = |f: &str| std::fs::read_to_string(sys.join(f)).ok().map(|s| s.trim().to_string());
            // interfaces (1-2:1.0) have no device numbers: only devices do
            let bus: u32 = read("busnum")?.parse().ok()?;
            let dev: u32 = read("devnum")?.parse().ok()?;
            let (major, minor) = read("dev")?.split_once(':').and_then(|(a, b)| Some((a.parse::<u32>().ok()?, b.parse::<u32>().ok()?)))?;
            let db = std::fs::read_to_string(Path::new("/run/udev/data").join(format!("c{major}:{minor}"))).unwrap_or_default();
            Some(UsbInfo {
                props: udev_props(&db),
                bus,
                dev,
                product: read("product").unwrap_or_default(),
                manufacturer: read("manufacturer").unwrap_or_default(),
            })
        })
        .collect()
}

/// The `E:KEY=value` lines of a udev database file.
fn udev_props(db: &str) -> Vec<(String, String)> {
    db.lines().filter_map(|l| l.strip_prefix("E:")?.split_once('=').map(|(k, v)| (k.to_string(), v.to_string()))).collect()
}

fn classify(d: &UsbInfo) -> Option<Device> {
    let interfaces = d.get("ID_USB_INTERFACES").unwrap_or("");
    let vendor = d.get("ID_VENDOR_ID").unwrap_or("");
    let serial = d.get("ID_SERIAL").unwrap_or("");
    // Apple's usbmux interface (class ff, subclass fe, protocol 02): an iPhone or iPad
    let kind = if vendor == "05ac" && interfaces.contains(":fffe02:") {
        DeviceKind::Apple
    } else if d.get("ID_MTP_DEVICE") == Some("1") {
        DeviceKind::Phone
    } else if d.get("ID_GPHOTO2") == Some("1") || interfaces.contains(":060101:") {
        DeviceKind::Camera
    } else {
        return None;
    };
    let name = if !d.product.is_empty() && !d.manufacturer.is_empty() && !d.product.starts_with(&d.manufacturer) {
        format!("{} {}", d.manufacturer, d.product)
    } else if !d.product.is_empty() {
        d.product.clone()
    } else {
        d.get("ID_MODEL").unwrap_or("USB device").replace('_', " ")
    };
    let url = match kind {
        DeviceKind::Phone => format!("mtp://{}/", gvfs_host(serial)),
        DeviceKind::Camera => format!("gphoto2://[usb:{:03},{:03}]/", d.bus, d.dev),
        DeviceKind::Apple => format!("afc://{}/", d.get("ID_SERIAL_SHORT").unwrap_or(serial)),
    };
    Some(Device { name, kind, url })
}

/// gvfs names an MTP device by its udev serial, with what can't stand in a URL host replaced.
fn gvfs_host(serial: &str) -> String {
    serial.chars().map(|c| if c.is_ascii_alphanumeric() || "-_.".contains(c) { c } else { '_' }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(db: &str, product: &str, manufacturer: &str) -> UsbInfo {
        UsbInfo { props: udev_props(db), bus: 1, dev: 7, product: product.into(), manufacturer: manufacturer.into() }
    }

    #[test]
    fn kinds_and_urls() {
        let phone = info("I:1\nE:ID_MTP_DEVICE=1\nE:ID_SERIAL=Google_Pixel_7_2A1B\nE:ID_USB_INTERFACES=:ff4201:\n", "Pixel 7", "Google");
        let d = classify(&phone).unwrap();
        assert_eq!((d.kind, d.name.as_str(), d.url.as_str()), (DeviceKind::Phone, "Google Pixel 7", "mtp://Google_Pixel_7_2A1B/"));

        let cam = info("E:ID_USB_INTERFACES=:060101:\nE:ID_SERIAL=Canon_EOS\n", "Canon Digital Camera", "Canon Inc.");
        let d = classify(&cam).unwrap();
        assert_eq!((d.kind, d.url.as_str()), (DeviceKind::Camera, "gphoto2://[usb:001,007]/"));
        assert_eq!(d.name, "Canon Inc. Canon Digital Camera");

        let ipad = info("E:ID_VENDOR_ID=05ac\nE:ID_USB_INTERFACES=:060101:fffe02:\nE:ID_SERIAL_SHORT=00008030abc\n", "iPad", "Apple Inc.");
        let d = classify(&ipad).unwrap();
        assert_eq!((d.kind, d.url.as_str()), (DeviceKind::Apple, "afc://00008030abc/"));

        let mouse = info("E:ID_USB_INTERFACES=:030102:\n", "Mouse", "Logi");
        assert!(classify(&mouse).is_none());
        let unnamed = info("E:ID_MTP_DEVICE=1\nE:ID_MODEL=Galaxy_A52\n", "", "");
        assert_eq!(classify(&unnamed).unwrap().name, "Galaxy A52");
    }

    #[test]
    fn listing_runs() {
        // whatever is plugged in here: it must not panic
        let _ = list();
        assert_eq!(usb_stamp(), usb_stamp());
    }
}
