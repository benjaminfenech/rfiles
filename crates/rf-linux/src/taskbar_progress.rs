//! Progress on rfiles' taskbar or dock icon while files copy, through the Unity LauncherEntry
//! D-Bus API (`com.canonical.Unity.LauncherEntry`). KDE Plasma's task manager, Dash to Dock /
//! Ubuntu Dock, Plank and Latte show it; elsewhere nothing happens. The Status Center shows the
//! same progress in the window either way.
//!
//! The entry belongs to the app's desktop file, not a window: every rfiles window's icon shows
//! it. Signals go out from one background thread with its own session-bus connection, kept
//! open: Plasma drops an entry's progress when its sender leaves the bus.

use std::collections::HashMap;
use std::sync::OnceLock;
use std::sync::mpsc::{Sender, channel};

use zbus::zvariant::Value;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum State {
    /// No bar.
    None,
    /// Working, how far not known yet.
    Busy,
    /// A bar at `0.0..=1.0`.
    Normal(f32),
    /// Paused.
    Paused(f32),
    /// An operation failed.
    Error(f32),
}

const IFACE: &str = "com.canonical.Unity.LauncherEntry";
const PATH: &str = "/io/github/benjaminfenech/rfiles/LauncherEntry";

/// Shows `state` on rfiles' launcher icon. Cheap: the signal goes out on a background thread.
pub fn set(state: State) {
    static TX: OnceLock<Option<Sender<State>>> = OnceLock::new();
    let tx = TX.get_or_init(|| {
        let (tx, rx) = channel::<State>();
        std::thread::Builder::new()
            .name("rf-launcher-entry".into())
            .spawn(move || {
                let conn = match zbus::blocking::Connection::session() {
                    Ok(c) => c,
                    Err(e) => {
                        log::info!("launcher progress: no session bus: {e}");
                        return;
                    }
                };
                let uri = app_uri();
                while let Ok(mut s) = rx.recv() {
                    // only the latest state matters
                    while let Ok(next) = rx.try_recv() {
                        s = next;
                    }
                    if let Err(e) = conn.emit_signal(None::<()>, PATH, IFACE, "Update", &(uri.as_str(), properties(s))) {
                        log::debug!("launcher progress: {e}");
                    }
                }
            })
            .ok()
            .map(|_| tx)
    });
    if let Some(tx) = tx {
        let _ = tx.send(state);
    }
}

/// `application://<desktop file>`: the Flatpak's app id, or the packages' `rfiles.desktop`.
fn app_uri() -> String {
    let id = crate::sandbox::flatpak_id().map_or_else(|| crate::default_fm::DESKTOP_ID.to_string(), |id| format!("{id}.desktop"));
    format!("application://{id}")
}

/// The entry's properties for `s`. Busy shows no bar (the API has no "unknown" bar); a failure
/// marks the icon urgent, which docks show as attention (Plasma: the button turns red-ish).
fn properties(s: State) -> HashMap<&'static str, Value<'static>> {
    let (visible, progress, urgent) = match s {
        State::None | State::Busy => (false, 0.0, false),
        State::Normal(v) | State::Paused(v) => (true, v, false),
        State::Error(v) => (true, v, true),
    };
    HashMap::from([
        ("progress-visible", Value::from(visible)),
        ("progress", Value::from(f64::from(progress.clamp(0.0, 1.0)))),
        ("urgent", Value::from(urgent)),
    ])
}

#[cfg(test)]
mod tests {
    use super::{State, properties};
    use zbus::zvariant::Value;

    #[test]
    fn states_become_entry_properties() {
        let p = properties(State::Normal(0.25));
        assert_eq!(p["progress-visible"], Value::from(true));
        assert_eq!(p["progress"], Value::from(0.25f64));
        assert_eq!(p["urgent"], Value::from(false));
        assert_eq!(properties(State::None)["progress-visible"], Value::from(false));
        assert_eq!(properties(State::Busy)["progress-visible"], Value::from(false));
        assert_eq!(properties(State::Error(0.5))["urgent"], Value::from(true));
        assert_eq!(properties(State::Normal(1.7))["progress"], Value::from(1.0f64));
    }

    #[test]
    fn uri_names_the_desktop_file() {
        assert!(super::app_uri().starts_with("application://") && super::app_uri().ends_with(".desktop"));
    }
}
