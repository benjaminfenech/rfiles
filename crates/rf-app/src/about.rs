//! This build's version, for Settings → About. Updates come from the
//! distribution's packages (or the Flatpak), so the app never checks for or downloads one.

/// "0.1.0 · b241ace · 08.11.2026 02:40": the version, the commit and when it was built.
pub fn version_full() -> String {
    let built = env!("RFILES_BUILT").parse::<i64>().ok().and_then(|s| chrono::DateTime::from_timestamp(s, 0));
    let when = built.map(|t| t.with_timezone(&chrono::Local).format("%d.%m.%Y %H:%M").to_string()).unwrap_or_default();
    format!("{} · {} · {when}", version(), env!("RFILES_COMMIT"))
}

pub fn version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}
