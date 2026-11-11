//! rfiles was called rustyfiles until 2026-11 (it began as the Linux port of
//! [rustyfiles](https://github.com/gelenmedya/rustyfiles)). What an install under the old
//! name left behind is taken over once, at start-up.

/// Old `RUSTYFILES_*` variables count as their `RFILES_*` names, and the settings, data and
/// cache folders move from `…/rustyfiles` to `…/rfiles` when the new ones don't exist yet.
/// Call first thing in `main`, before any thread starts (it sets environment variables).
pub fn adopt_old_name() {
    for (k, v) in std::env::vars_os() {
        let Some(rest) = k.to_str().and_then(|k| k.strip_prefix("RUSTYFILES_")) else { continue };
        let new = format!("RFILES_{rest}");
        if std::env::var_os(&new).is_none() {
            // SAFETY: called at the start of main, before other threads exist
            unsafe { std::env::set_var(new, v) };
        }
    }
    // a test copy keeps its data in its own folder
    if std::env::var_os("RFILES_CONFIG_DIR").is_some() {
        return;
    }
    for base in [dirs::config_dir(), dirs::data_local_dir(), dirs::cache_dir()].into_iter().flatten() {
        let (old, new) = (base.join("rustyfiles"), base.join("rfiles"));
        if old.is_dir() && !new.exists() {
            if let Err(e) = std::fs::rename(&old, &new) {
                eprintln!("rfiles: couldn't move {} to {}: {e}", old.display(), new.display());
            }
        }
    }
}

/// The desktop entries the old name wrote (the default file manager, starting at login, for
/// rfiles and DirectLook) point at this install, once the old program is gone.
pub fn adopt_old_integration() {
    if std::env::var_os("RFILES_CONFIG_DIR").is_some() {
        return;
    }
    let Ok(exe) = std::env::current_exe() else { return };
    if rf_win::default_fm::adopt_renamed("rustyfiles.desktop", &exe.to_string_lossy(), rf_win::default_fm::CLASSES) {
        log::info!("default file manager: now rfiles.desktop (was rustyfiles.desktop)");
    }
    if rf_win::dialogs::adopt_renamed_autostart("rustyfiles", &exe) {
        log::info!("start at login: now rfiles.desktop (was rustyfiles.desktop)");
    }
    if rf_win::dialogs::repoint_autostart("DirectLook", &exe.with_file_name("directlook")) {
        log::info!("DirectLook's start at login: now this install's directlook");
    }
}
