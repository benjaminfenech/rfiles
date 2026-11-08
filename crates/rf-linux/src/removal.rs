//! "Safely remove" support, the way Explorer does it.
//!
//! Windows refuses to eject a USB stick (or dismount any volume) while a process holds a handle
//! on it — our folder watcher, an open preview handler, a running size walk. Explorer keeps a
//! handle on every volume root registered with `RegisterDeviceNotification`, so the system
//! asks it first (`DBT_DEVICEQUERYREMOVE`); it then closes its windows on that drive and lets go.
//!
//! This module does the same on its own thread (never the UI or shell STA thread: the app is
//! given a moment to release its handles while the system waits for our answer):
//! - one notification handle per non-system volume, re-synced when drives come and go;
//! - on a removal query: close our handle, call the app, wait (bounded) for its ack, allow;
//! - removal failed (someone else still uses it): register again.
// Generated from rf-win's public API by a one-off stub generator, then edited by hand.
// Functions return safe defaults until they get a Linux implementation.
#![allow(unused)]
use std::collections::HashMap;
use std::sync::OnceLock;
use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};
use parking_lot::Mutex;
use crate::wide::wide;
/// Called on the removal thread with the drive letter and an ack sender; send (or drop) the
/// ack once the app has let go of everything on that drive.
type OnQuery = Box<dyn Fn(char, Sender<()>) + Send>;
struct Reg {
    handle: isize,
    notify: isize,
}
#[derive(Default)]
struct State {
    hwnd: isize,
    regs: HashMap<char, Reg>,
    /// Letters released for a pending removal: not re-registered until it completes, fails,
    /// or the grace period passes (re-opening a handle now would block the eject again).
    released: HashMap<char, Instant>,
}
static STATE: OnceLock<Mutex<State>> = OnceLock::new();
static CALLBACK: OnceLock<Mutex<Option<OnQuery>>> = OnceLock::new();
/// How long the system's removal request waits for the app; the eject proceeds regardless.
const APP_WAIT: Duration = Duration::from_secs(3);
/// Handle closes that happen just after the ack (watcher threads) get this much extra time.
const SETTLE: Duration = Duration::from_millis(300);
const RELEASE_GRACE: Duration = Duration::from_secs(20);
fn state() -> &'static Mutex<State> {
    STATE.get_or_init(|| Mutex::new(State::default()))
}
/// Starts watching (idempotent; the first callback wins).
pub fn start(on_query_remove: impl Fn(char, Sender<()>) + Send + 'static) {}
fn system_drive() -> char {
    std::env::var("SystemDrive")
        .ok()
        .and_then(|s| s.chars().next())
        .unwrap_or('C')
        .to_ascii_uppercase()
}
/// Registers new volumes, forgets vanished ones.
fn sync(hwnd: crate::Opaque) {}
fn register(hwnd: crate::Opaque, letter: char) -> Option<Reg> {
    None
}
fn unregister(r: Reg) {}
fn letter_for(handle: isize, notify: isize) -> Option<char> {
    state()
        .lock()
        .regs
        .iter()
        .find(|(_, r)| r.handle == handle || r.notify == notify)
        .map(|(l, _)| *l)
}
/// Drive letters currently registered (for tests and diagnostics).
pub fn registered() -> Vec<char> {
    let mut v: Vec<char> = state().lock().regs.keys().copied().collect();
    v.sort();
    v
}
