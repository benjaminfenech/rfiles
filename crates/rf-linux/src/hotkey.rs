//! One global keyboard shortcut for the quick search (upstream: `RegisterHotKey`). Wayland gives
//! apps no global shortcuts: on Linux the desktop's own shortcut settings start
//! `rfiles --quick-search` (Hyprland: `bind = ALT, SPACE, exec, rfiles --quick-search`;
//! GNOME and KDE: a custom shortcut). Nothing is registered here.

/// Why a shortcut can't be the hotkey.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HotkeyError {
    /// Not a shortcut we can register (no modifier, unknown key).
    Invalid,
    /// Another app (or the system) has it.
    Taken,
}

/// Where hotkey presses would go. Linux: they never come (the desktop starts the app instead).
pub fn set_handler(_f: impl Fn() + Send + 'static) {}

/// Holds `keys` as the hotkey (None: none). Linux: accepted and left to the desktop.
pub fn set(_keys: Option<&str>) -> Result<(), HotkeyError> {
    Ok(())
}
