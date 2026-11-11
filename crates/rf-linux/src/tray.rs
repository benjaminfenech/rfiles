//! The tray icon and notifications on Linux, for "keep running in the background": the icon
//! stays while no window is open; clicking it opens a window, its menu offers New window /
//! Quit. Long operations end with a desktop notification.
//!
//! The icon is a StatusNotifierItem (`ksni`): KDE, Waybar, Noctalia and most bars show it;
//! GNOME needs the AppIndicator extension. Notifications go to
//! `org.freedesktop.Notifications`. Both talk D-Bus on one worker thread; the public
//! functions only post to it.

use std::sync::OnceLock;
use std::sync::mpsc::{Sender, channel};

use ksni::blocking::{Handle, TrayMethods as _};
use ksni::menu::{MenuItem, StandardItem};
use parking_lot::Mutex;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayEvent {
    /// Click on the icon (or "Open" in its menu).
    Open,
    NewWindow,
    /// "Settings" in its menu.
    Settings,
    Quit,
}

/// Menu labels (the app's language).
#[derive(Clone)]
pub struct Labels {
    pub tooltip: String,
    pub open: String,
    pub new_window: String,
    /// Empty: no such item.
    pub settings: String,
    pub quit: String,
}

enum Cmd {
    Show(Labels),
    Hide,
    Notify { title: String, body: String },
}

static ON_EVENT: Mutex<Option<Box<dyn Fn(TrayEvent) + Send>>> = Mutex::new(None);

fn fire(e: TrayEvent) {
    if let Some(f) = ON_EVENT.lock().as_ref() {
        f(e);
    }
}

fn post(cmd: Cmd) {
    static TX: OnceLock<Mutex<Sender<Cmd>>> = OnceLock::new();
    let tx = TX.get_or_init(|| {
        let (tx, rx) = channel::<Cmd>();
        let _ = std::thread::Builder::new().name("rf-tray".into()).spawn(move || {
            let mut icon: Option<Handle<Item>> = None;
            for cmd in rx {
                run(cmd, &mut icon);
            }
        });
        Mutex::new(tx)
    });
    let _ = tx.lock().send(cmd);
}

fn run(cmd: Cmd, icon: &mut Option<Handle<Item>>) {
    match cmd {
        Cmd::Show(labels) => match icon {
            Some(h) => {
                h.update(|i| i.labels = labels);
            }
            // Flatpak can't own the per-process StatusNotifierItem-… name: register without it
            None => match (Item { labels }).disable_dbus_name(crate::sandbox::is_flatpak()).spawn() {
                Ok(h) => *icon = Some(h),
                // no tray host (GNOME without the extension): background mode still works
                Err(e) => log::info!("tray icon: {e}"),
            },
        },
        Cmd::Hide => {
            if let Some(h) = icon.take() {
                let _ = h.shutdown();
            }
        }
        Cmd::Notify { title, body } => {
            if let Err(e) = send_notification(&title, &body) {
                log::info!("notification: {e:#}");
            }
        }
    }
}

/// Where tray clicks / menu choices go (set once; later calls replace it).
pub fn set_handler(f: impl Fn(TrayEvent) + Send + 'static) {
    *ON_EVENT.lock() = Some(Box::new(f));
}

/// Shows (or updates) the icon.
pub fn show(labels: Labels) {
    post(Cmd::Show(labels));
}

/// Removes the icon.
pub fn hide() {
    post(Cmd::Hide);
}

/// A desktop notification from rfiles (the notification daemon decides about quiet hours).
pub fn notify(title: &str, body: &str) {
    post(Cmd::Notify { title: title.to_string(), body: body.to_string() });
}

struct Item {
    labels: Labels,
}

impl ksni::Tray for Item {
    fn id(&self) -> String {
        "rfiles".into()
    }
    fn title(&self) -> String {
        self.labels.tooltip.clone()
    }
    fn category(&self) -> ksni::Category {
        ksni::Category::ApplicationStatus
    }
    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        app_icon().to_vec()
    }
    fn tool_tip(&self) -> ksni::ToolTip {
        ksni::ToolTip { title: self.labels.tooltip.clone(), ..Default::default() }
    }
    fn activate(&mut self, _x: i32, _y: i32) {
        fire(TrayEvent::Open);
    }
    fn menu(&self) -> Vec<MenuItem<Self>> {
        let item = |label: &str, e: TrayEvent| {
            StandardItem { label: label.replace('_', "__"), activate: Box::new(move |_: &mut Self| fire(e)), ..Default::default() }.into()
        };
        let mut v = vec![item(&self.labels.open, TrayEvent::Open), item(&self.labels.new_window, TrayEvent::NewWindow)];
        if !self.labels.settings.is_empty() {
            v.push(item(&self.labels.settings, TrayEvent::Settings));
        }
        v.push(MenuItem::Separator);
        v.push(item(&self.labels.quit, TrayEvent::Quit));
        v
    }
    /// The bar restarted or isn't there yet: keep the icon, it registers again when it is.
    fn watcher_offline(&self, _reason: ksni::OfflineReason) -> bool {
        true
    }
}

/// The app icon (`rf-app/assets/rfiles.png`) as SNI pixmaps (ARGB32, big-endian), in a
/// few sizes for the bar to pick from.
fn app_icon() -> &'static [ksni::Icon] {
    static ICONS: OnceLock<Vec<ksni::Icon>> = OnceLock::new();
    ICONS.get_or_init(|| {
        let Ok(img) = image::load_from_memory(include_bytes!("../../rf-app/assets/rfiles.png")) else { return Vec::new() };
        [22u32, 32, 48, 64]
            .iter()
            .map(|&s| {
                let rgba = img.resize_exact(s, s, image::imageops::FilterType::Lanczos3).into_rgba8();
                let data = rgba.pixels().flat_map(|p| [p[3], p[0], p[1], p[2]]).collect();
                ksni::Icon { width: s as i32, height: s as i32, data }
            })
            .collect()
    })
}

fn send_notification(title: &str, body: &str) -> anyhow::Result<()> {
    use std::collections::HashMap;
    use zbus::zvariant::Value;
    static CONN: Mutex<Option<zbus::blocking::Connection>> = Mutex::new(None);
    let conn = {
        let mut c = CONN.lock();
        match &*c {
            Some(c) => c.clone(),
            None => {
                let new = zbus::blocking::Connection::session()?;
                *c = Some(new.clone());
                new
            }
        }
    };
    let mut hints: HashMap<&str, Value> = HashMap::new();
    hints.insert("desktop-entry", Value::from("rfiles"));
    let actions: Vec<&str> = Vec::new();
    conn.call_method(
        Some("org.freedesktop.Notifications"),
        "/org/freedesktop/Notifications",
        Some("org.freedesktop.Notifications"),
        "Notify",
        &("rfiles", 0u32, "system-file-manager", title, body, actions, hints, -1i32),
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn icon_pixmaps() {
        let icons = super::app_icon();
        assert_eq!(icons.len(), 4);
        for i in icons {
            assert_eq!(i.data.len(), (i.width * i.height * 4) as usize);
        }
        // ARGB: some pixels are opaque, the corners of a rounded icon aren't
        assert!(icons[3].data.chunks(4).any(|p| p[0] == 255));
    }
}
