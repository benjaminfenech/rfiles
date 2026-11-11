//! Sistem geneli klavye kancası: kendi iş parçacığında çalışır, Hızlı Bakış tuşlarını olaya çevirir.
// Generated from rf-win's public API by a one-off stub generator, then edited by hand.
// Functions return safe defaults until they get a Linux implementation.
#![allow(unused)]
use std::cell::RefCell;
use std::sync::RwLock;
use std::sync::mpsc::{Sender, channel};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};
use super::keys::{KeyGate, LookAction, LookKey};
use super::target::{TargetKind, foreground_target, is_typing};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LookEvent {
    pub action: LookAction,
    pub target: TargetKind,
    pub hwnd: isize,
}
/// Önizleme penceremizin hwnd'leri: bunlar öndeyken de tuşlar geçerlidir.
static OWN_WINDOWS: RwLock<Vec<isize>> = RwLock::new(Vec::new());
pub fn set_own_windows(hwnds: Vec<isize>) {
    *OWN_WINDOWS.write().unwrap_or_else(|e| e.into_inner()) = hwnds;
}
fn is_own(h: isize) -> bool {
    h != 0 && OWN_WINDOWS.read().unwrap_or_else(|e| e.into_inner()).contains(&h)
}
struct HookState {
    gate: KeyGate,
    tx: Sender<LookEvent>,
    trace: Option<(u64, Duration)>,
}
thread_local! {
    static STATE : RefCell < Option < HookState >> = const { RefCell::new(None) };
}
fn modifiers_down(info: &crate::Opaque) -> bool {
    false
}
fn handle_key(st: &mut HookState, msg: u32, info: &crate::Opaque) {}
pub struct HookHandle {
    thread_id: u32,
    hook: Option<JoinHandle<()>>,
    dispatch: Option<JoinHandle<()>>,
}
impl HookHandle {
    pub fn stop(mut self) {
        self.shutdown();
    }
    fn shutdown(&mut self) {}
}
impl Drop for HookHandle {
    fn drop(&mut self) {
        self.shutdown();
    }
}
/// Kancayı kendi iş parçacığında kurar. `on_event` ayrı bir iş parçacığında çağrılır (kanca hiç beklemez).
///
/// Linux: Wayland has no global keyboard hooks, so DirectLook only answers keys inside
/// rfiles. The handle is inert and `on_event` is never called.
pub fn start(on_event: impl Fn(LookEvent) + Send + 'static) -> HookHandle {
    HookHandle { thread_id: 0, hook: None, dispatch: None }
}
