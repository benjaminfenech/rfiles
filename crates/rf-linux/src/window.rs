//! Small top-level window helpers that GPUI does not expose.
// Generated from rf-win's public API by a one-off stub generator, then edited by hand.
// Functions return safe defaults until they get a Linux implementation.
#![allow(unused)]
/// Keeps the window above all non-topmost windows ("compact overlay" / always on top).
pub fn set_topmost(hwnd: isize, on: bool) -> anyhow::Result<()> {
    Err(
        anyhow::anyhow!("rf-linux: `set_topmost` is not implemented on Linux yet").into(),
    )
}
pub fn is_topmost(hwnd: isize) -> bool {
    false
}
/// Whether the foreground window belongs to this process.
pub fn app_in_front() -> bool {
    false
}
/// Window rectangle in physical pixels `[left, top, right, bottom]`, and whether it is maximized.
pub fn rect(hwnd: isize) -> Option<([i32; 4], bool)> {
    None
}
/// Restores (un-maximizes) and moves/resizes the window, physical pixels.
pub fn place(hwnd: isize, r: [i32; 4]) {}
pub fn maximize(hwnd: isize) {}
/// Overrides the minimum window size (logical pixels) at runtime, `None` = back to the
/// size given at creation. Read by our GPUI fork on WM_GETMINMAXINFO.
pub fn set_min_size(hwnd: isize, size: Option<(u16, u16)>) {}
/// Dark or light window frame / system backdrop (Mica follows it), independent of the system
/// setting — so the app's own Light / Dark choice also tints Mica.
/// The process's AppUserModelID: windows group under, and pins match, the shortcuts carrying
/// the same id (not the exe path, which changes folder on every update).
pub fn set_app_id(id: &str) {}
pub fn set_dark_mode(hwnd: isize, dark: bool) {}
/// Makes `owner` (a window of another process, possibly) the owner of `hwnd`: it stays above
/// it and minimizes with it, like the owner's own dialog.
pub fn set_owner(hwnd: isize, owner: isize) {}
/// Gives the foreground back to `owner` (another app's window) while we still hold it: when our
/// window closes, Windows would otherwise activate whatever window comes next.
pub fn hand_foreground(owner: isize) {}
/// Brings `hwnd` to the front and gives it the keyboard (the caller was allowed to by the app
/// in front, see AllowSetForegroundWindow).
///
/// When that permission didn't come (Chrome asks from a background helper process, which
/// can't grant it), the window would open behind the app: then our thread shares the input
/// state of the window in front for a moment, which lets the switch through.
pub fn bring_to_front(hwnd: isize) {}
/// Whether `hwnd` is the window in front (has the keyboard).
pub fn is_foreground(hwnd: isize) -> bool {
    false
}
