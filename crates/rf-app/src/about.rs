//! This build's version, and updates from GitHub Releases (Settings → About): a check soon
//! after start and once a day (unless turned off), and on request; the install of a newer
//! release in the same form this copy came in (see `rf_linux::update`), then a restart.

/// "0.7.0 · b241ace · 11/11/2026 21:50": the version, the commit and when it was built.
pub fn version_full() -> String {
    // in the date format chosen in Settings
    let when = env!("RFILES_BUILT").parse::<i64>().map(|s| rf_core::format::datetime_full(s * 1000)).unwrap_or_default();
    format!("{} · {} · {when}", version(), env!("RFILES_COMMIT"))
}

pub fn version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

// ---------------------------------------------------------------- updates

use std::sync::Mutex;
use std::time::Duration;

use rf_linux::update::{self, Install, Release};

#[derive(Clone, Debug, PartialEq)]
pub enum UpdateState {
    Idle,
    Checking,
    UpToDate,
    /// A newer release.
    Available(Release),
    /// Version and percent downloaded.
    Downloading(String, u8),
    Installing(String),
    /// Installed: a restart runs it.
    Installed(String),
    Failed(String),
}

static STATE: Mutex<UpdateState> = Mutex::new(UpdateState::Idle);

pub fn update_state() -> UpdateState {
    STATE.lock().unwrap().clone()
}

fn set(s: UpdateState) {
    *STATE.lock().unwrap() = s;
}

/// A check, download or install is running (Settings redraws while it does).
pub fn busy() -> bool {
    matches!(update_state(), UpdateState::Checking | UpdateState::Downloading(..) | UpdateState::Installing(_))
}

/// How this copy was installed, and whether an update can be installed from here (else only
/// the release page is offered).
pub fn installable() -> bool {
    update::install_kind() != Install::Other
}

/// Asks GitHub for the latest release. `manual`: errors are shown (a background check that
/// fails leaves things as they were and only logs). Blocking: off the UI thread.
pub fn check(manual: bool) {
    if busy() || matches!(update_state(), UpdateState::Installed(_)) {
        return;
    }
    let before = update_state();
    if manual {
        set(UpdateState::Checking);
    }
    match update::latest() {
        Ok(r) if update::newer(&r.version, &version()) => set(UpdateState::Available(r)),
        Ok(_) => set(UpdateState::UpToDate),
        Err(e) if manual => set(UpdateState::Failed(format!("couldn't reach GitHub ({e:#})"))),
        Err(e) => {
            log::info!("update check: {e:#}");
            set(before);
        }
    }
}

/// Downloads and installs the available release. Blocking: off the UI thread.
pub fn install_available() {
    let UpdateState::Available(r) = update_state() else { return };
    let kind = update::install_kind();
    let Some(asset) = update::pick_asset(&r, kind, std::env::consts::ARCH).cloned() else {
        set(UpdateState::Failed(format!("no {kind:?} package for this computer in {}", r.version)));
        return;
    };
    let v = r.version.clone();
    set(UpdateState::Downloading(v.clone(), 0));
    let progress = |done: u64, total: u64| set(UpdateState::Downloading(v.clone(), (done * 100 / total.max(1)).min(100) as u8));
    let file = match update::download(&asset, &update::download_dir(), &progress, &|| false) {
        Ok(f) => f,
        Err(e) => return set(UpdateState::Failed(format!("{e:#}"))),
    };
    set(UpdateState::Installing(v.clone()));
    match update::install(kind, &file) {
        Ok(()) => {
            let _ = std::fs::remove_dir_all(update::download_dir());
            set(UpdateState::Installed(v));
        }
        // a cancelled password prompt: the update is still there to install
        Err(e) if e.to_string().contains("cancelled") => set(UpdateState::Available(r)),
        Err(e) => set(UpdateState::Failed(format!("{e:#}"))),
    }
}

/// The exe as it was at start (once replaced by an update, /proc/self/exe says "(deleted)").
static EXE: std::sync::OnceLock<Option<std::path::PathBuf>> = std::sync::OnceLock::new();

/// Background checks: 30 s after start, then every 24 hours, while "Check for updates" is
/// on. Not for test copies (their own settings folder). Calls `changed` after each.
pub fn spawn_checks(cx: &mut gpui_kit::App) {
    EXE.get_or_init(|| std::env::current_exe().ok());
    if std::env::var_os("RFILES_CONFIG_DIR").is_some() {
        return;
    }
    cx.spawn(async move |cx| {
        let mut wait = Duration::from_secs(30);
        loop {
            cx.background_executor().timer(wait).await;
            wait = Duration::from_secs(24 * 3600);
            if !cx.update(|cx| cx.global::<crate::settings::Settings>().auto_update) {
                continue;
            }
            cx.background_executor().spawn(async { check(false) }).await;
            cx.update(|cx| cx.refresh_windows());
        }
    })
    .detach();
}

/// Runs `job` off the UI thread and redraws the windows while the update is busy.
pub fn run(cx: &mut gpui_kit::App, job: fn()) {
    cx.spawn(async move |cx| {
        let task = cx.background_executor().spawn(async move { job() });
        cx.background_executor().timer(Duration::from_millis(50)).await;
        while busy() {
            cx.update(|cx| cx.refresh_windows());
            cx.background_executor().timer(Duration::from_millis(200)).await;
        }
        task.await;
        cx.update(|cx| cx.refresh_windows());
    })
    .detach();
}

/// Quits and starts the installed version once this one has gone (DirectLook quits too, so
/// the next Space starts the new one).
pub fn restart(cx: &mut gpui_kit::App) {
    rf_linux::single_instance::send_to(&crate::look::ipc::app_name(), &crate::look::ipc::Msg::Quit.encode());
    let started = match rf_linux::sandbox::flatpak_id() {
        // on the host: wait for this sandbox to end, then run the app again
        Some(id) => rf_linux::sandbox::launch("sh")
            .args([
                "-c",
                "for i in $(seq 100); do flatpak ps --columns=application | grep -qx \"$1\" || break; sleep 0.2; done; exec flatpak run \"$1\"",
                "sh",
                id,
            ])
            .spawn(),
        None => {
            let exe = EXE.get().cloned().flatten().or_else(|| std::env::current_exe().ok()).unwrap_or_else(|| "rfiles".into());
            // the package may have moved the old file aside: run the path, not the inode
            let exe = exe.to_string_lossy().trim_end_matches(" (deleted)").to_string();
            std::process::Command::new("sh")
                .args(["-c", "for i in $(seq 100); do kill -0 \"$1\" 2>/dev/null || break; sleep 0.2; done; exec \"$2\"", "sh"])
                .arg(std::process::id().to_string())
                .arg(exe)
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
        }
    };
    if let Err(e) = started {
        log::warn!("restart after update: {e}");
    }
    crate::quit_when_idle(cx);
}
