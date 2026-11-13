//! Opening the terminal from Settings → Advanced → Terminal in a folder, optionally running a
//! command there ("Run command here", or `>command` in the address bar).

use std::path::PathBuf;
use std::sync::OnceLock;

use gpui_kit::{AnyWindowHandle, App, AppContext};

/// Opens the terminal in `dir`, running `command` when given. A failure is shown over `win`.
pub(crate) fn open(dir: PathBuf, command: Option<String>, win: Option<AnyWindowHandle>, cx: &mut App) {
    let choice = cx.global::<crate::settings::Settings>().terminal.clone();
    let task = cx.background_spawn(async move { rf_linux::terminal::open(&choice, &dir, command.as_deref()).map_err(|e| format!("{e:#}")) });
    cx.spawn(async move |cx| {
        if let Err(e) = task.await {
            log::warn!("terminal: {e}");
            if let Some(win) = win {
                cx.update(|cx| crate::ops::alert_box(crate::t!("Couldn't open a terminal").to_string(), e, win, cx));
            }
        }
    })
    .detach();
}

/// What Settings lists: Automatic's pick and the installed terminals. Looked up once, on a
/// worker thread (desktop files, gsettings); None until it's done.
pub(crate) fn choices() -> Option<&'static (Option<String>, Vec<rf_linux::terminal::Terminal>)> {
    static FOUND: OnceLock<(Option<String>, Vec<rf_linux::terminal::Terminal>)> = OnceLock::new();
    static STARTED: std::sync::Once = std::sync::Once::new();
    STARTED.call_once(|| {
        std::thread::spawn(|| {
            let _ = FOUND.set((rf_linux::terminal::automatic(), rf_linux::terminal::installed()));
        });
    });
    FOUND.get()
}

/// The text after `>` in the address bar: a command to run in the current folder.
pub(crate) fn address_command(text: &str) -> Option<&str> {
    text.trim_start().strip_prefix('>').map(str::trim)
}

#[cfg(test)]
mod tests {
    #[test]
    fn address_commands() {
        assert_eq!(super::address_command("> git status"), Some("git status"));
        assert_eq!(super::address_command(">"), Some(""));
        assert_eq!(super::address_command("/home/u"), None);
    }
}
