//! Progress on a window's taskbar button, like Explorer while it copies (upstream:
//! `ITaskbarList3`). Linux: not shown yet (docks that show progress take it over D-Bus,
//! `com.canonical.Unity.LauncherEntry`); the Status Center shows it as before.

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

pub fn set(_hwnd: isize, _state: State) {}
