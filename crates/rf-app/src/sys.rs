//! Small input and system queries that differ per platform. On Windows these are the same
//! Win32 calls the app always made; on Linux GPUI owns the input state: the modifier keys come
//! from the window's events, and the global cursor position doesn't exist (Wayland).

/// Screen rectangle for the preview host (`left/top/right/bottom`).
#[cfg(windows)]
pub type Rect = windows::Win32::Foundation::RECT;
#[cfg(not(windows))]
pub type Rect = rf_win::RECT;

/// Mouse position in screen pixels. Linux (Wayland) has no global cursor position: (0, 0).
pub fn cursor_pos() -> (i32, i32) {
    #[cfg(windows)]
    {
        let mut pt = windows::Win32::Foundation::POINT::default();
        unsafe {
            let _ = windows::Win32::UI::WindowsAndMessaging::GetCursorPos(&mut pt);
        }
        (pt.x, pt.y)
    }
    #[cfg(not(windows))]
    {
        (0, 0)
    }
}

/// Shift / Ctrl as last seen by a window (Linux has no global key state to ask; the
/// workspace reports every change and every mouse move). Bit 0 Shift, bit 1 Ctrl.
#[cfg(not(windows))]
static MODIFIERS: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);

/// Records the modifier keys a window event carried (no-op on Windows, which asks the OS).
pub fn note_modifiers(m: gpui_kit::Modifiers) {
    #[cfg(not(windows))]
    MODIFIERS.store(u8::from(m.shift) | u8::from(m.control) << 1, std::sync::atomic::Ordering::Relaxed);
    #[cfg(windows)]
    let _ = m;
}

/// Whether Shift is held right now.
pub fn shift_down() -> bool {
    #[cfg(windows)]
    unsafe {
        use windows::Win32::UI::Input::KeyboardAndMouse::{GetKeyState, VK_SHIFT};
        GetKeyState(VK_SHIFT.0 as i32) < 0
    }
    #[cfg(not(windows))]
    {
        MODIFIERS.load(std::sync::atomic::Ordering::Relaxed) & 1 != 0
    }
}

/// Whether Ctrl is held right now.
pub fn ctrl_down() -> bool {
    #[cfg(windows)]
    unsafe {
        use windows::Win32::UI::Input::KeyboardAndMouse::{GetKeyState, VK_CONTROL};
        GetKeyState(VK_CONTROL.0 as i32) < 0
    }
    #[cfg(not(windows))]
    {
        MODIFIERS.load(std::sync::atomic::Ordering::Relaxed) & 2 != 0
    }
}

/// The system double-click time in ms (GTK's default, 400, on Linux).
pub fn double_click_ms() -> u32 {
    #[cfg(windows)]
    unsafe {
        windows::Win32::UI::Input::KeyboardAndMouse::GetDoubleClickTime()
    }
    #[cfg(not(windows))]
    {
        400
    }
}

/// The UI thread's id, for the drag-out helper (0 on Linux, where it is unused).
pub fn current_thread_id() -> u32 {
    #[cfg(windows)]
    unsafe {
        windows::Win32::System::Threading::GetCurrentThreadId()
    }
    #[cfg(not(windows))]
    {
        0
    }
}

/// Lets go of the mouse capture before a native drag starts (nothing to do on Linux).
pub fn release_capture() {
    #[cfg(windows)]
    unsafe {
        let _ = windows::Win32::UI::Input::KeyboardAndMouse::ReleaseCapture();
    }
}

/// Windows file attribute bits (`rf_core::attrs`) for `path`. Linux derives them: read-only
/// (no write permission), hidden (dot name), directory, and a symlink counts as a reparse point.
pub fn file_attributes(path: &std::path::Path, m: &std::fs::Metadata) -> u32 {
    #[cfg(windows)]
    {
        let _ = path;
        use std::os::windows::fs::MetadataExt;
        m.file_attributes()
    }
    #[cfg(not(windows))]
    {
        use rf_core::attrs;
        let mut a = 0;
        if m.permissions().readonly() {
            a |= attrs::READONLY;
        }
        if path.file_name().is_some_and(|n| n.to_string_lossy().starts_with('.')) {
            a |= attrs::HIDDEN;
        }
        if m.is_dir() {
            a |= attrs::DIRECTORY;
        }
        if m.file_type().is_symlink() {
            a |= attrs::REPARSE_POINT;
        }
        a
    }
}

/// The OS UI language as a BCP 47 tag ("tr-TR"), if it can be told.
pub fn ui_language_tag() -> Option<String> {
    #[cfg(windows)]
    {
        use windows::Win32::Globalization::{GetUserDefaultUILanguage, LCIDToLocaleName};
        let mut buf = [0u16; 85];
        let n = unsafe { LCIDToLocaleName(GetUserDefaultUILanguage() as u32, Some(&mut buf), 0) };
        (n > 1).then(|| String::from_utf16_lossy(&buf[..n as usize - 1]))
    }
    #[cfg(not(windows))]
    {
        // LANGUAGE (a priority list) wins over the locale, as in gettext
        let raw = std::env::var("LANGUAGE")
            .ok()
            .and_then(|l| l.split(':').next().map(str::to_string))
            .or_else(|| ["LC_ALL", "LC_MESSAGES", "LANG"].iter().find_map(|v| std::env::var(v).ok()))
            .filter(|s| !s.is_empty() && s != "C" && s != "POSIX")?;
        // "tr_TR.UTF-8@euro" -> "tr-TR"
        let base = raw.split(['.', '@']).next()?;
        Some(base.replace('_', "-"))
    }
}

/// The monospace font for text previews: Cascadia Mono on Windows, the fontconfig
/// `monospace` family on Linux ("Noto Sans Mono", "DejaVu Sans Mono"…).
pub fn mono_font() -> &'static str {
    #[cfg(windows)]
    {
        "Cascadia Mono"
    }
    #[cfg(not(windows))]
    {
        static F: std::sync::OnceLock<String> = std::sync::OnceLock::new();
        F.get_or_init(|| fc_family("monospace").unwrap_or_else(|| "DejaVu Sans Mono".into()))
    }
}

/// The desktop's UI font on Linux (fontconfig `sans-serif`); `None` on Windows, where the
/// theme picks Segoe UI itself.
pub fn ui_font() -> Option<&'static str> {
    #[cfg(windows)]
    {
        None
    }
    #[cfg(not(windows))]
    {
        static F: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
        F.get_or_init(|| fc_family("sans-serif")).as_deref()
    }
}

/// First family fontconfig resolves `pattern` to (`fc-match`, ~5 ms, once per pattern).
#[cfg(not(windows))]
fn fc_family(pattern: &str) -> Option<String> {
    let out = std::process::Command::new("fc-match").args(["-f", "%{family}", pattern]).output().ok()?;
    let s = String::from_utf8(out.stdout).ok()?;
    let first = s.split(',').next()?.trim();
    (!first.is_empty()).then(|| first.to_string())
}

/// Whether menus, flyouts and dropdowns open as their own popup windows (Windows: Acrylic,
/// may extend past the app window) instead of being drawn inside the app window.
/// `RF_INWINDOW_MENU=1` asks for in-window on Windows too. Always in-window on Linux: Wayland
/// can't place a window at the cursor, and tiling window managers (Hyprland, Sway, i3…) would
/// tile a popup window next to the app.
pub fn separate_popups() -> bool {
    cfg!(windows) && std::env::var_os("RF_INWINDOW_MENU").is_none()
}

/// Whether the toolbar's View button opens the Layout flyout (tiles, size slider); otherwise a
/// plain menu with the same choices. Only Windows' `RF_INWINDOW_MENU` asks for the menu: the
/// flyout has an in-window mode of its own.
pub fn layout_flyout() -> bool {
    !cfg!(windows) || std::env::var_os("RF_INWINDOW_MENU").is_none()
}

/// Inserts a zero-width space after every path separator, so a long path can wrap there.
pub fn wrap_path(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 16);
    for c in s.chars() {
        out.push(c);
        if c == '\\' || c == '/' {
            out.push('\u{200B}');
        }
    }
    out
}

/// The path with the platform's separators: Windows' shell APIs can't parse `C:/a/b`; Linux
/// paths are left alone ("\\" is an ordinary character there).
pub fn native_separators(p: std::path::PathBuf) -> std::path::PathBuf {
    if cfg!(windows) { std::path::PathBuf::from(p.to_string_lossy().replace('/', "\\")) } else { p }
}

/// The app id window rules match on Linux (Wayland `app_id`, X11 `WM_CLASS`); Windows keeps
/// its own taskbar identity (`rf_win::window::set_app_id`).
pub fn app_id(name: &str) -> Option<String> {
    // Flatpak: the main window carries the app id, which names its desktop entry (icon, dock)
    #[cfg(unix)]
    if let Some(id) = rf_win::sandbox::flatpak_id().filter(|_| name == "rfiles") {
        return Some(id.to_string());
    }
    (!cfg!(windows)).then(|| name.to_string())
}

/// The work area (logical x, y, w, h) a new DirectLook window is sized and centered in:
/// Windows asks the monitor under the cursor; Linux takes the largest display's visible area
/// (a Wayland app can't know which output the pointer is on, the compositor places the window,
/// and GPUI's "primary" display is just the first output the compositor announced).
pub fn work_area_at_cursor(cx: &gpui_kit::App) -> [f32; 4] {
    #[cfg(windows)]
    {
        let _ = cx;
        rf_win::look::win::work_area_at_cursor()
    }
    #[cfg(not(windows))]
    {
        let area = |b: gpui_kit::Bounds<gpui_kit::Pixels>| f32::from(b.size.width) * f32::from(b.size.height);
        cx.displays()
            .into_iter()
            .map(|d| d.visible_bounds())
            .max_by(|a, b| area(*a).total_cmp(&area(*b)))
            .map_or([0., 0., 1280., 800.], bounds_array)
    }
}

/// The work area of the monitor `window` is on.
pub fn work_area_of(hwnd: isize, window: &gpui_kit::Window, cx: &gpui_kit::App) -> [f32; 4] {
    #[cfg(windows)]
    {
        let _ = (window, cx);
        rf_win::look::win::work_area_of(hwnd)
    }
    #[cfg(not(windows))]
    {
        let _ = hwnd;
        window.display(cx).map_or_else(|| work_area_at_cursor(cx), |d| bounds_array(d.visible_bounds()))
    }
}

/// Moves and resizes a window (logical px). Linux: only the size (Wayland apps can't place
/// themselves; a floating window keeps its place, a tiled one ignores both).
// Windows resizes DirectLook after the update instead (see look/window.rs)
#[cfg_attr(windows, allow(dead_code))]
pub fn set_window_rect(hwnd: isize, r: [f32; 4], window: &mut gpui_kit::Window) {
    #[cfg(windows)]
    rf_win::look::win::set_rect(hwnd, r, window.scale_factor());
    #[cfg(not(windows))]
    {
        let _ = hwnd;
        window.resize(gpui_kit::size(gpui_kit::px(r[2]), gpui_kit::px(r[3])));
    }
}

/// Whether DirectLook should reopen instead of resizing to `new` (logical px): only off
/// Windows (compositors keep a floating window's size), and only for a clear change (more than
/// 12 % and 48 px on a side), so small differences never flicker.
pub fn reopen_for_size(cur: (f32, f32), new: (f32, f32)) -> bool {
    let differs = |a: f32, b: f32| (a - b).abs() > 48.0 && (a - b).abs() > a.max(b) * 0.12;
    !cfg!(windows) && (differs(cur.0, new.0) || differs(cur.1, new.1))
}

#[cfg(not(windows))]
fn bounds_array(b: gpui_kit::Bounds<gpui_kit::Pixels>) -> [f32; 4] {
    [f32::from(b.origin.x), f32::from(b.origin.y), f32::from(b.size.width), f32::from(b.size.height)]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reopen_only_for_clear_size_changes() {
        let linux = !cfg!(windows);
        assert_eq!(reopen_for_size((1280., 760.), (1280., 360.)), linux);
        assert!(!reopen_for_size((1280., 760.), (1270., 740.)), "small change: resize in place");
        assert!(!reopen_for_size((200., 100.), (230., 100.)), "12 % but under 48 px");
    }

    #[test]
    fn paths_wrap_after_separators() {
        assert_eq!(wrap_path("/a/b"), "/\u{200B}a/\u{200B}b");
        assert_eq!(wrap_path(r"C:\a"), "C:\\\u{200B}a");
        let p = std::path::PathBuf::from("/a/b");
        if cfg!(windows) {
            assert_eq!(native_separators(p), std::path::PathBuf::from(r"\a\b"));
        } else {
            assert_eq!(native_separators(p.clone()), p);
        }
    }
}
