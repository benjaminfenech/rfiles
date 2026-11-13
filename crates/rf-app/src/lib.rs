//! rfiles ve DirectLook'un ortak kodu; iki program da bu kütüphaneyi kullanır.

mod actions;
mod aliases;
mod background;
mod renamed;
mod archive_view;
mod bulk_rename;
mod command_palette;
mod conflict_dialog;
mod ctx_menu;
mod file_drag;
mod disk_usage;
mod fluent_menu;
mod flyout;
mod layout_panel;
mod git_ui;
mod prompt;
mod hash_panel;
mod i18n;
mod folder_view;
mod icons;
mod keymap_editor;
mod layouts;
pub mod look;
mod image_cache;
mod about;
mod ops;
mod places;
mod settings;
mod select_box;
mod settings_view;
mod shelf_view;
mod status_center;
mod tags;
mod undo;
mod preview;
mod properties_view;
mod global_keys;
mod terminal;
mod picker;
mod quick_search;
mod recycle_view;
mod theme;
mod workspace;
mod sys;

use std::path::PathBuf;

use gpui_kit::component::TitleBar;
use gpui_kit::*;
use rf_core::Location;

use crate::workspace::Workspace;

/// Window placement from the last session (ignored if it is implausibly small).
pub(crate) fn restored_bounds(cx: &App) -> Option<WindowBounds> {
    let [x, y, w, h, max] = cx.global::<settings::Settings>().window?;
    if w < 500. || h < 350. {
        return None;
    }
    let b = Bounds { origin: point(px(x), px(y)), size: size(px(w), px(h)) };
    Some(if max > 0.5 { WindowBounds::Maximized(b) } else { WindowBounds::Windowed(b) })
}

/// Panics are also written to crash.log in the config folder (launchers hide stderr).
pub(crate) fn install_panic_log() {
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        if let Some(dir) = crate::settings::app_dir() {
            let _ = std::fs::create_dir_all(&dir);
            let bt = std::backtrace::Backtrace::force_capture();
            let line = format!(
                "[{}] {info}\n{bt}\n\n",
                chrono::Local::now().format("%Y-%m-%d %H:%M:%S")
            );
            use std::io::Write;
            if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(dir.join("crash.log")) {
                let _ = f.write_all(line.as_bytes());
            }
        }
        prev(info);
    }));
}

/// Whether rfiles is the default file manager (read off the UI thread at start).
pub static DEFAULT_FM: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
/// This process is the Open/Save dialog service (`--file-chooser-service`).
pub(crate) static PICKER_SERVICE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
/// Opens an rfiles window. `start = None` restores the session (first window) or opens Home.
pub fn open_main_window(
    start: Option<Location>,
    ipc: Option<futures::channel::mpsc::UnboundedReceiver<String>>,
    bounds: Option<WindowBounds>,
    cx: &mut App,
) {
    let options = WindowOptions {
        window_bounds: Some(bounds.unwrap_or_else(|| WindowBounds::centered(size(px(1280.), px(800.)), cx))),
        window_min_size: Some(size(px(720.), px(420.))),
        window_background: WindowBackgroundAppearance::MicaBackdrop,
        app_id: crate::sys::app_id("rfiles"),
        ..TitleBar::window_options()
    };
    if let Err(e) = gpui_kit::open_window(options, cx, move |window, cx| cx.new(|cx| Workspace::new(start, ipc, window, cx))) {
        log::error!("open window: {e:#}");
    }
}

/// Quits now, or once running copy/move/delete operations finish (closing the window must not
/// cut a copy in half). A window opened meanwhile (second launch) cancels the quit.
pub(crate) fn quit_when_idle(cx: &mut App) {
    // (an app or file the quick search is still starting counts as running work)
    let running = |cx: &App| cx.try_global::<status_center::OpsGlobal>().map_or(0, |g| g.0.read(cx).running()) + quick_search::launching();
    if running(cx) == 0 {
        save_and_quit(cx);
        return;
    }
    cx.spawn(async move |cx| loop {
        cx.background_executor().timer(std::time::Duration::from_millis(500)).await;
        let done = cx.update(|cx| {
            if workspace::any_main_window(cx) {
                return Some(false);
            }
            (running(cx) == 0).then_some(true)
        });
        match done {
            Some(true) => {
                cx.update(save_and_quit);
                break;
            }
            Some(false) => break,
            None => {}
        }
    })
    .detach();
}

/// The session/placement save on window close runs on a helper thread; write once more here,
/// synchronously, so quitting cannot cut it off (a small file, only at exit).
fn save_and_quit(cx: &mut App) {
    cx.global::<settings::Settings>().clone().save();
    // the search index keeps its place in the change journal (or its changes): the next start
    // then catches up instead of walking the drives; bounded, quitting must not hang on it
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        rf_linux::indexer::stop();
        let _ = tx.send(());
    });
    let _ = rx.recv_timeout(std::time::Duration::from_secs(2));
    cx.quit();
}

/// `Path::is_dir` with a time limit (it can block for the SMB timeout on an offline share).
fn is_dir_bounded(p: PathBuf, limit: std::time::Duration) -> Option<bool> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(p.is_dir());
    });
    rx.recv_timeout(limit).ok()
}

/// rfiles'ın girişi.
/// Start-up timing (`RFILES_TRACE_START=1`, logged at info): milliseconds since the process
/// was created, at the steps before the first window.
pub(crate) fn start_mark(step: &str) {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    if *ON.get_or_init(|| std::env::var_os("RFILES_TRACE_START").is_some()) {
        eprintln!("start: {step} at {} ms", rf_linux::process_age_ms());
    }
}

pub fn main() {
    // Flatpak: the host's XDG folders (trash, settings, apps), before any thread starts
    rf_linux::sandbox::adopt_host_dirs();
    // rfiles was rustyfiles before: old variables and folders
    renamed::adopt_old_name();
    start_mark("main");
    install_panic_log();
    rf_linux::shell::suppress_critical_error_dialogs();
    // before any shell icon/thumbnail work: see keep_gdiplus_alive (loader-lock deadlock)
    rf_linux::thumbs::keep_gdiplus_alive();
    // Logs go to stderr; override with RUST_LOG.
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn"))
        .target(env_logger::Target::Stderr)
        .format_timestamp_millis()
        .init();
    start_mark("logging");

    // Linux: D-Bus started us to answer "Show in folder" while we are the default file
    // manager. No window here: each call starts `rfiles <folder>` (see rf-linux fm_service).
    if std::env::args().nth(1).as_deref() == Some(rf_linux::default_fm::SERVICE_ARG) {
        match std::env::current_exe() {
            Ok(exe) => {
                if let Err(e) = rf_linux::fm_service::run(&exe) {
                    log::warn!("{}: {e:#}", rf_linux::default_fm::SERVICE);
                }
            }
            Err(e) => log::warn!("{}: {e}", rf_linux::default_fm::SERVICE),
        }
        return;
    }

    // D-Bus started us as other apps' Open/Save dialog (see rf-linux file_chooser): no main
    // window, a picker per call, and its own process (never handed to a running rfiles)
    let chooser = std::env::args().nth(1).as_deref() == Some(rf_linux::file_chooser::SERVICE_ARG);
    if chooser {
        PICKER_SERVICE.store(true, std::sync::atomic::Ordering::Relaxed);
    }

    // `--settings=directlook`: DirectLook'un simge menüsünden "DirectLook ayarları"
    let settings_arg = std::env::args().nth(1).and_then(|a| a.strip_prefix("--settings=").map(str::to_string));
    let open_look_settings = settings_arg.is_some();
    // `--quick-search`: the quick search window, as its hotkey opens it (a shortcut of the
    // user's own can run this)
    let quick_search = std::env::args().nth(1).as_deref() == Some(quick_search::LAUNCH_ARG);
    let arg = std::env::args().nth(1).map(|a| a.trim().trim_matches('"').to_string()).filter(|a| !a.is_empty());
    // Linux: launchers pass file:// URIs (the desktop entry's %U, which lets smb://… in too)
    let arg = arg.map(|a| match a.starts_with("file://").then(|| rf_linux::fm_service::uri_to_path(&a)).flatten() {
        Some(p) => p.to_string_lossy().into_owned(),
        None => a,
    });
    let arg = arg.filter(|_| !open_look_settings && !quick_search && !chooser);
    // the launcher's "New window" action
    let new_window = arg.as_deref() == Some(workspace::NEW_WINDOW_ARG);
    let arg = arg.filter(|_| !new_window);
    let mut start: Option<Location> = None;
    // `/select,<file>` (from the "Show in folder" service): its folder, with the file selected
    // (callers also write `/select, <file>`: the path then comes as the next argument)
    let all_args = std::env::args().skip(1).collect::<Vec<_>>().join(" ");
    let mut select = {
        let lower = all_args.to_ascii_lowercase();
        (lower.starts_with("/select,") || lower.starts_with("-select,"))
            .then(|| all_args[8..].trim().trim_matches('"').to_string())
            // bounded like the folder check below: an offline share must not hold the window
            .filter(|file| {
                PathBuf::from(file)
                    .parent()
                    .is_some_and(|d| is_dir_bounded(d.to_path_buf(), std::time::Duration::from_secs(2)) != Some(false))
            })
    };
    let arg = if select.is_some() { None } else { arg };
    if let Some(file) = &select {
        let p = PathBuf::from(file);
        start = p.parent().map(|d| Location::Path(d.to_path_buf()));
        *workspace::START_SELECT.lock().unwrap() = p.file_name().map(|n| n.to_string_lossy().into_owned());
    }
    // `shell:RecycleBinFolder` and search links (also what a second launch forwards)
    if let Some(loc) = arg.as_deref().and_then(Location::from_shell_name) {
        start = Some(loc);
    } else if let Some(arg) = arg {
        // network locations (smb://, sftp://…) are mounted by the window
        let remote = rf_linux::remote::is_remote(std::path::Path::new(&arg));
        // bounded: an offline network path must not keep the window from appearing for the
        // whole SMB timeout. Undecided after 2 s = treat as a (slow) folder.
        let is_dir = if remote { Some(true) } else { is_dir_bounded(PathBuf::from(&arg), std::time::Duration::from_secs(2)) };
        // a browsable archive (DirectLook's "rfiles'ta aç") opens like a folder, here
        let archive = archive_view::is_browsable(std::path::Path::new(&arg));
        if is_dir == Some(false) && !archive {
            // a file goes to its own default app; should that be us, it comes straight back:
            // never ping-pong, it is then shown here (its folder, with the file selected)
            if !rf_linux::default_fm::forwarded_recently(&arg) {
                rf_linux::default_fm::note_forward(&arg);
                let _ = rf_linux::shell::open(std::path::Path::new(&arg));
                return;
            }
            let p = PathBuf::from(&arg);
            if let (Some(dir), Some(name)) = (p.parent().filter(|_| p.is_file()), p.file_name()) {
                start = Some(Location::Path(dir.to_path_buf()));
                *workspace::START_SELECT.lock().unwrap() = Some(name.to_string_lossy().into_owned());
                select = Some(arg.clone());
            }
        }
        if start.is_none() {
            start = Some(Location::Path(PathBuf::from(arg)));
        }
    }
    // The default file manager state for the settings page (read once, off the hot path).
    std::thread::spawn(|| {
        // copies opened/copied out of archives days ago
        crate::archive_view::clean_temp();
        // rustyfiles' (the old name's) default file manager and autostart entries become ours
        renamed::adopt_old_integration();
        // default file manager whose registered exe vanished (moved / rebuilt elsewhere):
        // re-point it here, or other apps can't open any folder. Not from test instances.
        if std::env::var_os("RFILES_CONFIG_DIR").is_none() {
            if let Ok(me) = std::env::current_exe() {
                if rf_linux::default_fm::heal(&me.display().to_string(), rf_linux::default_fm::CLASSES) {
                    log::warn!("default file manager re-registered to {}", me.display());
                }
                if rf_linux::default_fm::add_archives(&me.display().to_string(), rf_linux::default_fm::CLASSES) {
                    log::warn!("default file manager: archive registration updated");
                }
            }
        }
        DEFAULT_FM.store(
            rf_linux::default_fm::is_enabled(rf_linux::default_fm::CLASSES),
            std::sync::atomic::Ordering::Relaxed,
        )
    });

    // Single instance: a second launch opens its folder as a tab in the running window.
    let message = match &start {
        _ if open_look_settings => look::ipc::ToApp::OpenSettings.encode(),
        _ if new_window => workspace::NEW_WINDOW_ARG.to_string(),
        _ if quick_search => quick_search::LAUNCH_ARG.to_string(),
        _ if select.is_some() => format!("{}{}", workspace::SELECT_PREFIX, select.as_deref().unwrap_or_default()),
        Some(Location::Path(p)) => p.to_string_lossy().into_owned(),
        Some(Location::RecycleBin) => "shell:RecycleBinFolder".to_string(),
        // a search link goes on to the running window as a search link
        Some(Location::Search { root, query }) => {
            let enc = |s: &str| s.replace('%', "%25").replace('&', "%26");
            format!("search-ms:query={}&crumb=location:{}", enc(query), enc(&root.to_string_lossy()))
        }
        _ => String::new(),
    };
    // RFILES_MULTI_INSTANCE=1 skips forwarding (automated tests run several copies).
    let multi = std::env::var_os("RFILES_MULTI_INSTANCE").is_some() || chooser;
    let _instance = if multi {
        None
    } else {
        match rf_linux::single_instance::acquire(&message) {
            rf_linux::single_instance::Role::Forwarded => return,
            rf_linux::single_instance::Role::Primary(guard) => Some(guard),
        }
    };
    let (ipc_tx, ipc_rx) = futures::channel::mpsc::unbounded::<String>();
    if !multi {
        rf_linux::single_instance::serve(move |m| {
            let _ = ipc_tx.unbounded_send(m);
        });
    }

    start_mark("before the app");
    gpui_kit::application().with_assets(gpui_kit::assets::AllAssets).run(move |cx| {
        start_mark("app running");
        gpui_kit::init(cx);
        cx.set_global(settings::Settings::load());
        start_mark("settings loaded");
        rf_core::sort::set_turkish_collation(rf_linux::encoding::user_locale_is_turkish());
        // eski kod sayfalı metinlerin tahmini: Türkçe olmayan sistemde Türkçe ipucu verilmez
        if !rf_linux::encoding::user_locale_is_turkish() {
            rf_core::textload::set_locale_hint(None);
        }
        settings::apply_component_theme(cx);
        cx.set_global(tags::load());
        let ops = cx.new(status_center::Ops::new);
        // the Status Center flyout is its own Acrylic window, opened/closed with `open`
        cx.observe(&ops, |ops, cx| {
            status_center::sync_popup(&ops, cx);
            status_center::sync_taskbar(&ops, cx);
        })
        .detach();
        cx.set_global(status_center::OpsGlobal(ops));
        actions::bind(cx);
        actions::bind_user(cx);
        keymap_editor::init(cx);
        if let Some(page) = &settings_arg {
            settings_view::open_page(
                match page.as_str() {
                    "general" => settings_view::Page::General,
                    _ => settings_view::Page::Look,
                },
                cx,
            );
        }
        // copies and moves keep a crash-safe journal there (see rf_linux::fastcopy)
        if let Some(d) = settings::app_dir() {
            rf_linux::fastcopy::set_journal_dir(d.join("jobs"));
        }
        // folder sizes survive a restart (shown at once, measured again when asked for)
        if let Some(d) = settings::local_data_dir() {
            rf_linux::foldersize::shared_cache().persist(d.join("foldersizes.bin"));
        }
        let bounds = restored_bounds(cx);
        if chooser {
            picker::serve(cx);
            cx.set_quit_mode(gpui_kit::QuitMode::Explicit);
            cx.activate(true);
            return;
        }
        if quick_search {
            // started for the quick search only: no main window (it quits when that closes,
            // unless running in the background)
            quick_search::toggle(cx);
        } else {
            start_mark("opening the window");
            open_main_window(if open_look_settings { Some(Location::Settings) } else { start }, None, bounds, cx);
            start_mark("window opened");
            // a copy or move cut off last time: ask to carry it on
            status_center::offer_resume(cx);
        }
        // DirectLook ayrı süreç: önceden başlatılır ki ilk Boşluk beklemesin (testlerde değil)
        if cx.global::<settings::Settings>().directlook_enabled && std::env::var_os("RFILES_MULTI_INSTANCE").is_none() {
            look::ipc::send(look::ipc::Msg::Wake, true);
        }
        // Second launches arrive here and go to whichever window is still open.
        let mut ipc_rx = ipc_rx;
        cx.spawn(async move |cx| {
            use futures::StreamExt;
            while let Some(msg) = ipc_rx.next().await {
                cx.update(|cx| workspace::route_ipc(msg, cx));
            }
        })
        .detach();
        // The last window closing ends the app (GPUI does not on its own: the process would
        // linger invisibly and swallow every later launch through the single-instance pipe).
        // With "run in the background" on, the app stays in the notification area instead.
        background::init(cx);
        quick_search::init(cx);
        global_keys::init(cx);
        // Settings lists the terminals: find them before it opens
        let _ = terminal::choices();
        // updates from GitHub Releases (Settings → About)
        about::spawn_checks(cx);
        // GPUI would quit by itself when the last window closes (QuitMode::Default on Windows):
        // that cut running copies in half and makes background mode impossible. We decide.
        cx.set_quit_mode(gpui_kit::QuitMode::Explicit);
        cx.on_window_closed(|cx, _| {
            if !workspace::any_main_window(cx) {
                // menus / flyouts left behind go too
                for w in cx.windows() {
                    let _ = w.update(cx, |_, window, _| window.remove_window());
                }
                if cx.global::<settings::Settings>().run_in_background && !background::quitting() {
                    let s = cx.global::<settings::Settings>().clone();
                    std::thread::spawn(move || s.save());
                } else {
                    quit_when_idle(cx);
                }
            }
        })
        .detach();
        cx.activate(true);
    });
}
