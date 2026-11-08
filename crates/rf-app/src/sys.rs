//! Small input and system queries that differ per platform. On Windows these are the same
//! Win32 calls the app always made; on Linux GPUI owns the input state, so they return
//! neutral answers until the call sites read GPUI's (see docs/LINUX_PORT.md, Phase 1).

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

/// Whether Shift is held right now.
pub fn shift_down() -> bool {
    #[cfg(windows)]
    unsafe {
        use windows::Win32::UI::Input::KeyboardAndMouse::{GetKeyState, VK_SHIFT};
        GetKeyState(VK_SHIFT.0 as i32) < 0
    }
    #[cfg(not(windows))]
    {
        false
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
        false
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
