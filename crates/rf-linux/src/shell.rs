//! Shell actions (open, properties). Each call initializes COM on the calling thread,
//! so run them on a worker thread — shell extensions can block for seconds.
// Generated from rf-win's public API by a one-off stub generator, then edited by hand.
// Functions return safe defaults until they get a Linux implementation.
#![allow(unused)]
use std::path::Path;
use crate::wide::wide;
/// Initializes an STA on the current thread (idempotent; errors ignored when already initialized).
pub fn ensure_com() {}
/// Runs a launcher detached; a waiter thread reaps it so no zombie is left behind.
pub(crate) fn spawn_detached(cmd: &mut std::process::Command) -> std::io::Result<()> {
    use std::process::Stdio;
    let mut child = cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn()?;
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

/// `verb` None = open with the default app (`xdg-open`). "Open with" and the properties
/// dialog are Windows shell verbs: Phase 1 (`.desktop` files) and rustyfiles' own window.
fn exec(path: &Path, verb: Option<&str>, args: Option<&str>) -> anyhow::Result<()> {
    match verb {
        None => {
            spawn_detached(std::process::Command::new("xdg-open").arg(path))
                .map_err(|e| anyhow::anyhow!("xdg-open: {e}"))
        }
        Some(v) => anyhow::bail!("rf-linux: the `{v}` verb is not implemented on Linux yet"),
    }
}
/// Opens a file with its default handler (same as double-click in Explorer).
pub fn open(path: &Path) -> anyhow::Result<()> {
    exec(path, None, None)
}
/// Shows the shell "Open with" dialog.
///
/// Linux: the desktop's own app chooser through the xdg-desktop-portal (`OpenURI` with
/// `ask`), which GNOME, KDE and the GTK portal used on wlroots / Hyprland provide. Blocks
/// until the portal has taken the request (not until the user has chosen).
pub fn open_with(path: &Path) -> anyhow::Result<()> {
    use ashpd::desktop::open_uri::OpenFileRequest;
    let file = std::fs::File::open(path)?;
    futures::executor::block_on(async { OpenFileRequest::default().ask(true).send_file(&file).await })
        .map(|_| ())
        .map_err(|e| anyhow::anyhow!("the desktop's app chooser isn't available ({e})"))
}
/// Shows the Windows properties dialog for the item.
pub fn properties(path: &Path) -> anyhow::Result<()> {
    exec(path, Some("properties"), None)
}
/// Opens a terminal in `dir`: `$TERMINAL`, else the first common one found on `PATH`.
pub fn open_terminal(dir: &Path) -> anyhow::Result<()> {
    let mut candidates: Vec<String> = std::env::var("TERMINAL").ok().filter(|t| !t.is_empty()).into_iter().collect();
    candidates.extend(
        ["x-terminal-emulator", "kgx", "gnome-terminal", "konsole", "ptyxis", "kitty", "alacritty", "foot", "wezterm", "xterm"]
            .map(String::from),
    );
    for t in &candidates {
        if spawn_detached(std::process::Command::new(t).current_dir(dir)).is_ok() {
            return Ok(());
        }
    }
    anyhow::bail!("no terminal found (set $TERMINAL)")
}
/// Yes/No warning box owned by `owner` (call on the STA shell thread). True on "Yes".
pub fn confirm(owner: crate::Opaque, title: &str, text: &str) -> bool {
    false
}
/// Information box owned by `owner` (call on the STA shell thread).
pub fn alert(owner: crate::Opaque, title: &str, text: &str) {}
/// `shell:Downloads`, `shell:startup`, `shell:sendto`… → file-system path (None for virtual
/// folders like `shell:RecycleBinFolder`). Blocking (shell namespace): off the UI thread.
pub fn resolve_shell_path(s: &str) -> Option<std::path::PathBuf> {
    None
}
/// No "There is no disk in the drive" / "Insert a disk" system boxes when touching an empty
/// card reader or an ejected disc: such calls just fail (and the app shows "drive not ready").
/// Process-wide; call once at startup.
pub fn suppress_critical_error_dialogs() {}
/// Tells the shell (Explorer windows, desktop, file dialogs) that `dir`'s contents changed,
/// the way the shell's own copy engine does when it finishes.
pub fn notify_updated(dir: &std::path::Path) {}
