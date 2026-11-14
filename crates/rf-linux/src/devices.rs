//! Phones, tablets and cameras plugged in over USB. They have no file system to mount: they
//! speak MTP (Android), PTP (cameras, phones in "photo" mode) or Apple's AFC (iPhone, iPad),
//! and the desktop's layer turns them into folders, as it does for network places
//! ([`crate::remote`]):
//! - KDE: kio-extras' `mtp:/`, `camera:/` and `afc:/` workers, through kio-fuse.
//! - GNOME and others: gvfs (`mtp://<serial>/`, `gphoto2://[usb:bus,dev]/`, `afc://<udid>/`).
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

/// The phones and cameras plugged in now. `kio`: open them through KDE's workers (kio-fuse
/// is installed), else through gvfs.
pub fn list(kio: bool) -> Vec<Device> {
    let mut out: Vec<Device> = usb_devices().iter().filter_map(|d| classify(d, kio)).collect();
    // KDE's mtp:/ and camera:/ list every device of their kind: one row each is enough
    if kio {
        let mut seen = std::collections::HashSet::new();
        out.retain(|d| seen.insert(d.url.clone()));
    }
    out
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

fn classify(d: &UsbInfo, kio: bool) -> Option<Device> {
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
    let url = match (kind, kio) {
        (DeviceKind::Phone, true) => "mtp:///".to_string(),
        (DeviceKind::Camera, true) => "camera:///".to_string(),
        (DeviceKind::Apple, true) => "afc:///".to_string(),
        (DeviceKind::Phone, false) => format!("mtp://{}/", gvfs_host(serial)),
        (DeviceKind::Camera, false) => format!("gphoto2://[usb:{:03},{:03}]/", d.bus, d.dev),
        (DeviceKind::Apple, false) => format!("afc://{}/", d.get("ID_SERIAL_SHORT").unwrap_or(serial)),
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
        let d = classify(&phone, false).unwrap();
        assert_eq!((d.kind, d.name.as_str(), d.url.as_str()), (DeviceKind::Phone, "Google Pixel 7", "mtp://Google_Pixel_7_2A1B/"));
        assert_eq!(classify(&phone, true).unwrap().url, "mtp:///");

        let cam = info("E:ID_USB_INTERFACES=:060101:\nE:ID_SERIAL=Canon_EOS\n", "Canon Digital Camera", "Canon Inc.");
        let d = classify(&cam, false).unwrap();
        assert_eq!((d.kind, d.url.as_str()), (DeviceKind::Camera, "gphoto2://[usb:001,007]/"));
        assert_eq!(d.name, "Canon Inc. Canon Digital Camera");

        let ipad = info("E:ID_VENDOR_ID=05ac\nE:ID_USB_INTERFACES=:060101:fffe02:\nE:ID_SERIAL_SHORT=00008030abc\n", "iPad", "Apple Inc.");
        let d = classify(&ipad, false).unwrap();
        assert_eq!((d.kind, d.url.as_str()), (DeviceKind::Apple, "afc://00008030abc/"));

        let mouse = info("E:ID_USB_INTERFACES=:030102:\n", "Mouse", "Logi");
        assert!(classify(&mouse, false).is_none());
        let unnamed = info("E:ID_MTP_DEVICE=1\nE:ID_MODEL=Galaxy_A52\n", "", "");
        assert_eq!(classify(&unnamed, true).unwrap().name, "Galaxy A52");
    }

    #[test]
    fn listing_runs() {
        // whatever is plugged in here: it must not panic
        let _ = list(true);
        let _ = list(false);
        assert_eq!(usb_stamp(), usb_stamp());
    }
}
