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

// ---- Window z-order and lookup (upstream, after a1d7ca3): the Open/Save dialog picker keeps
// itself above the app that asked for it. Linux has no rf-dialog (no injected picker) and
// Wayland doesn't let apps inspect other windows: answers that keep those paths idle.

/// Whether `a` lies above `b` in the z-order. Linux: unknown.
pub fn is_above(_a: isize, _b: isize) -> Option<bool> {
    None
}

/// Whether `hwnd` is still a window. Linux: no foreign windows are tracked.
pub fn exists(_hwnd: isize) -> bool {
    false
}

/// The window in front (0 when none). Linux: not known to apps.
pub fn foreground() -> isize {
    0
}

/// A window for a log line.
pub fn describe(hwnd: isize) -> String {
    format!("window {hwnd:#x}")
}

/// The window that owns `hwnd` (0 when none).
pub fn owner_of(_hwnd: isize) -> isize {
    0
}

/// Raises `hwnd` above its owner. Linux: the compositor stacks windows.
pub fn raise_above_owner(_hwnd: isize) {}

/// The colour on screen down a column of `hwnd`'s client area. Linux: apps can't read the
/// screen (Wayland), so the app's own colours are used.
pub fn screen_color(_hwnd: isize, _x: i32, _y0: i32, _y1: i32) -> Option<[u8; 3]> {
    None
}
