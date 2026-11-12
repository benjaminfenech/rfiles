//! Settings → Quick search → "Global shortcut": opens the quick search from any app through
//! the desktop portal (see `rf_linux::global_shortcuts`). Off by default: turning it on makes
//! KDE and GNOME ask the user to confirm the key.

use std::sync::Mutex;

use futures::StreamExt;
use gpui_kit::App;
use rf_linux::global_shortcuts::{QUICK_SEARCH, Shortcuts};

use crate::settings::Settings;

/// The running session, while the setting is on (read by Settings to show the key).
static KEYS: Mutex<Option<Shortcuts>> = Mutex::new(None);

pub fn init(cx: &mut App) {
    // the portal ties our app id to the first portal call: tell it now, off the UI thread
    std::thread::Builder::new().name("rf-portal-id".into()).spawn(rf_linux::global_shortcuts::register_host_app).ok();
    apply(cx);
    cx.observe_global::<Settings>(apply).detach();
}

/// Starts or ends the session to match the settings.
fn apply(cx: &mut App) {
    let s = cx.global::<Settings>();
    let want = s.quick_search && s.global_shortcut;
    let mut keys = KEYS.lock().unwrap();
    if want == keys.is_some() {
        return;
    }
    if !want {
        *keys = None;
        return;
    }
    let (tx, mut rx) = futures::channel::mpsc::unbounded::<String>();
    let desc = crate::t!("Open rfiles quick search").to_string();
    *keys = Some(Shortcuts::start(vec![(QUICK_SEARCH, desc, "ALT+space")], move |id| {
        let _ = tx.unbounded_send(id.to_string());
    }));
    drop(keys);
    cx.spawn(async move |cx| {
        while let Some(id) = rx.next().await {
            log::info!("global shortcut: {id}");
            if id == QUICK_SEARCH {
                cx.update(crate::quick_search::toggle);
            }
        }
    })
    .detach();
}

/// The line under the setting: the bound key, why it can't work, or how to bind it.
pub fn status() -> Option<String> {
    let keys = KEYS.lock().unwrap();
    let k = keys.as_ref()?;
    Some(if let Some(e) = k.error() {
        e
    } else if let Some(t) = k.trigger(QUICK_SEARCH) {
        format!("{}: {t}", crate::t!("Shortcut"))
    } else {
        // Hyprland binds it in its config; the name is `<app id>:quick-search`
        crate::t!("No key assigned yet. On Hyprland, bind the \"quick-search\" shortcut that `hyprctl globalshortcuts` lists, e.g. bind = ALT, SPACE, global, <name>").to_string()
    })
}

/// Whether the desktop can show its own settings for the shortcut.
pub fn can_configure() -> bool {
    KEYS.lock().unwrap().as_ref().is_some_and(|k| k.can_configure() && k.error().is_none())
}

pub fn configure() {
    if let Some(k) = KEYS.lock().unwrap().as_ref() {
        k.configure();
    }
}
