//! Read-only Windows system parameters the UI follows.
// Generated from rf-win's public API by a one-off stub generator, then edited by hand.
// Functions return safe defaults until they get a Linux implementation.
/// Windows' "Animation effects" (Settings → Accessibility → Visual effects). True when it can't
/// be read (the Windows default).
///
/// Linux: GNOME's `enable-animations` when gsettings has it (many non-GNOME desktops keep it
/// in step too), else on. Read once: it starts a process.
pub fn client_area_animation() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| {
        crate::sandbox::tool("gsettings", &[])
            .args(["get", "org.gnome.desktop.interface", "enable-animations"])
            .stderr(std::process::Stdio::null())
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).trim() != "false")
            .unwrap_or(true)
    })
}
