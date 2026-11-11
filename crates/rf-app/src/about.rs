//! This build's version, for Settings → About. Updates come from the
//! distribution's packages (or the Flatpak), so the app never checks for or downloads one.

/// "0.7.0 · b241ace · 11/11/2026 21:50": the version, the commit and when it was built.
pub fn version_full() -> String {
    // in the date format chosen in Settings
    let when = env!("RFILES_BUILT").parse::<i64>().map(|s| rf_core::format::datetime_full(s * 1000)).unwrap_or_default();
    format!("{} · {} · {when}", version(), env!("RFILES_COMMIT"))
}

pub fn version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}
