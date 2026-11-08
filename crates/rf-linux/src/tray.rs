//! Notification-area (tray) icon and notifications, like Files' "keep running in the
//! background": the icon stays while no window is open; clicking it opens a window, its menu
//! offers New window / Quit. Notifications use the icon's balloon, which Windows 10/11 show as
//! a regular toast — no packaging / AppUserModelID registration needed.
//!
//! Everything runs on its own thread with a hidden window; the public functions only post to it.
// Generated from rf-win's public API by a one-off stub generator, then edited by hand.
// Functions return safe defaults until they get a Linux implementation.
#![allow(unused)]
use std::collections::VecDeque;
use std::sync::OnceLock;
use parking_lot::Mutex;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayEvent {
    /// Click on the icon (or "Open" in its menu).
    Open,
    NewWindow,
    Quit,
}
/// Menu labels (the app's language).
#[derive(Clone)]
pub struct Labels {
    pub tooltip: String,
    pub open: String,
    pub new_window: String,
    pub quit: String,
}
enum Cmd {
    Show(Labels),
    Hide,
    Notify { title: String, body: String },
}
struct Tray {
    hwnd: isize,
    queue: Mutex<VecDeque<Cmd>>,
    on_event: Mutex<Option<Box<dyn Fn(TrayEvent) + Send>>>,
}
static TRAY: OnceLock<Tray> = OnceLock::new();
const WM_CMD: u32 = 0;
const WM_TRAY: u32 = 0;
const ICON_ID: u32 = 1;
thread_local! {
    #[doc =
    " Thread-side state: whether the icon is shown on request (vs. only for a notification)."]
    static STATE : std::cell::RefCell < (Option < Labels >, bool) > = const {
    std::cell::RefCell::new((None, false)) };
}
/// Linux: the StatusNotifierItem tray is Phase 3. The handler is kept; nothing is shown.
fn tray() -> &'static Tray {
    TRAY.get_or_init(|| Tray { hwnd: 0, queue: Mutex::new(VecDeque::new()), on_event: Mutex::new(None) })
}
fn post(cmd: Cmd) {}
/// Where tray clicks / menu choices go (set once; later calls replace it).
pub fn set_handler(f: impl Fn(TrayEvent) + Send + 'static) {
    *tray().on_event.lock() = Some(Box::new(f));
}
/// Shows (or updates) the icon.
pub fn show(labels: Labels) {
    post(Cmd::Show(labels));
}
/// Removes the icon (a notification still adds it for a moment).
pub fn hide() {
    post(Cmd::Hide);
}
/// A notification (toast) from rustyfiles. Respects Focus Assist / quiet hours.
pub fn notify(title: &str, body: &str) {
    post(Cmd::Notify {
        title: title.to_string(),
        body: body.to_string(),
    });
}
fn copy_into(dst: &mut [u16], s: &str) {
    let v: Vec<u16> = s.encode_utf16().take(dst.len() - 1).collect();
    dst[..v.len()].copy_from_slice(&v);
    dst[v.len()] = 0;
}
/// Adds the icon if it is not there yet. Returns whether it was added now.
fn ensure_icon(hwnd: crate::Opaque, tooltip: &str) -> bool {
    false
}
fn remove_icon(hwnd: crate::Opaque) {}
fn run_commands(hwnd: crate::Opaque) {}
fn fire(e: TrayEvent) {
    if let Some(t) = TRAY.get() {
        if let Some(f) = t.on_event.lock().as_ref() {
            f(e);
        }
    }
}
fn menu(hwnd: crate::Opaque) {}
