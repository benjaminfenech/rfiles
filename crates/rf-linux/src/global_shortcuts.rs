//! Shortcuts that work in any app, through the desktop portal's GlobalShortcuts interface
//! (Wayland gives apps no global key grabs). KDE Plasma and GNOME 48+ ask the user once to
//! confirm the key (and remember it); Hyprland binds it in its config:
//! `bind = ALT, SPACE, global, io.github.benjaminfenech.rfiles:quick-search`.
//!
//! The session lives on its own thread for the app's life; dropping the [`Shortcuts`] handle
//! closes it (the desktop forgets the binding until the next session, not the key).

use std::sync::{Arc, Mutex};

use ashpd::desktop::global_shortcuts::{GlobalShortcuts, NewShortcut, Shortcut};
use futures::StreamExt;
use futures::channel::mpsc;

/// The quick search window's shortcut.
pub const QUICK_SEARCH: &str = "quick-search";

/// The app id the portal knows a non-Flatpak rfiles by (a Flatpak's comes from the sandbox).
/// Hyprland names its shortcuts `<app id>:<shortcut id>`.
pub const HOST_APP_ID: &str = "io.github.benjaminfenech.rfiles";

/// Tells the portal who we are, when rfiles isn't a Flatpak. Must come before any other
/// portal call of the process (the portal ties the id to the connection's first use).
/// Desktops without the host registry (xdg-desktop-portal before 1.19) keep working without it.
pub fn register_host_app() {
    if crate::sandbox::is_flatpak() {
        return;
    }
    let r = futures::executor::block_on(async {
        let id: ashpd::AppID = HOST_APP_ID.parse()?;
        ashpd::register_host_app(id).await
    });
    if let Err(e) = r {
        log::info!("portal host app registration: {e}");
    }
}

enum Cmd {
    Configure,
}

/// A running global shortcuts session.
pub struct Shortcuts {
    tx: mpsc::UnboundedSender<Cmd>,
    /// What the desktop says each shortcut is bound to ("Alt+Space"), by id; empty until it
    /// says (Hyprland: once the config binds it).
    triggers: Arc<Mutex<Vec<(String, String)>>>,
    /// Why the session couldn't start (no portal, or no GlobalShortcuts in it).
    error: Arc<Mutex<Option<String>>>,
    /// Whether the portal can show its own settings for the shortcuts (version 2).
    can_configure: Arc<Mutex<bool>>,
}

impl Shortcuts {
    /// Asks the portal for `shortcuts` ((id, description, preferred trigger)), then calls
    /// `on_activate(id)` from the session's thread whenever one is pressed.
    pub fn start(shortcuts: Vec<(&'static str, String, &'static str)>, on_activate: impl Fn(&str) + Send + 'static) -> Shortcuts {
        let (tx, mut rx) = mpsc::unbounded::<Cmd>();
        let triggers = Arc::new(Mutex::new(Vec::new()));
        let error = Arc::new(Mutex::new(None));
        let can_configure = Arc::new(Mutex::new(false));
        let (t2, e2, c2) = (triggers.clone(), error.clone(), can_configure.clone());
        let spawned = std::thread::Builder::new().name("rf-global-shortcuts".into()).spawn(move || {
            let run = async {
                let gs = GlobalShortcuts::new().await?;
                *c2.lock().unwrap() = gs.version() >= 2;
                let session = gs.create_session(Default::default()).await?;
                let wanted: Vec<NewShortcut> =
                    shortcuts.iter().map(|(id, desc, key)| NewShortcut::new(*id, desc.as_str()).preferred_trigger(*key)).collect();
                // KDE and GNOME show their confirmation dialog here, the first time
                let bound = gs.bind_shortcuts(&session, &wanted, None, Default::default()).await?.response()?;
                *t2.lock().unwrap() = triggers_of(bound.shortcuts());
                let mut activated = gs.receive_activated().await?.fuse();
                let mut changed = gs.receive_shortcuts_changed().await?.fuse();
                loop {
                    futures::select! {
                        a = activated.next() => match a {
                            Some(a) => on_activate(a.shortcut_id()),
                            None => break,
                        },
                        c = changed.next() => {
                            if let Some(c) = c {
                                *t2.lock().unwrap() = triggers_of(c.shortcuts());
                            }
                        }
                        cmd = rx.next() => match cmd {
                            Some(Cmd::Configure) => {
                                if let Err(e) = gs.configure_shortcuts(&session, None, Default::default()).await {
                                    log::info!("global shortcuts: configure: {e}");
                                }
                            }
                            // the handle was dropped: end the session
                            None => break,
                        },
                    }
                }
                let _ = session.close().await;
                Ok::<_, ashpd::Error>(())
            };
            if let Err(e) = futures::executor::block_on(run) {
                log::info!("global shortcuts: {e}");
                *e2.lock().unwrap() = Some(describe(&e));
            }
        });
        if let Err(e) = spawned {
            *error.lock().unwrap() = Some(e.to_string());
        }
        Shortcuts { tx, triggers, error, can_configure }
    }

    /// The key the desktop bound `id` to ("Alt+Space"), when it said.
    pub fn trigger(&self, id: &str) -> Option<String> {
        self.triggers.lock().unwrap().iter().find(|(i, t)| i == id && !t.is_empty()).map(|(_, t)| t.clone())
    }

    /// Why the shortcuts don't work, when they can't.
    pub fn error(&self) -> Option<String> {
        self.error.lock().unwrap().clone()
    }

    /// Whether [`configure`](Self::configure) can show the desktop's shortcut settings.
    pub fn can_configure(&self) -> bool {
        *self.can_configure.lock().unwrap()
    }

    /// Opens the desktop's own settings for these shortcuts (where the portal has them).
    pub fn configure(&self) {
        let _ = self.tx.unbounded_send(Cmd::Configure);
    }
}

fn triggers_of(list: &[Shortcut]) -> Vec<(String, String)> {
    list.iter().map(|s| (s.id().to_string(), s.trigger_description().to_string())).collect()
}

/// The portal's error as a sentence for Settings.
fn describe(e: &ashpd::Error) -> String {
    match e {
        ashpd::Error::PortalNotFound(_) => "Your desktop's portal doesn't offer global shortcuts.".into(),
        ashpd::Error::Response(ashpd::desktop::ResponseError::Cancelled) => "The shortcut wasn't confirmed.".into(),
        e => format!("{e}"),
    }
}
