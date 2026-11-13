//! Shell actions (open, properties). Each call initializes COM on the calling thread,
//! so run them on a worker thread — shell extensions can block for seconds.
// Generated from rf-win's public API by a one-off stub generator, then edited by hand.
// Functions return safe defaults until they get a Linux implementation.
use std::path::Path;
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
/// dialog are Windows shell verbs: Phase 1 (`.desktop` files) and rfiles' own window.
fn exec(path: &Path, verb: Option<&str>, _args: Option<&str>) -> anyhow::Result<()> {
    match verb {
        None => {
            spawn_detached(crate::sandbox::launch("xdg-open").arg(path))
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
/// Yes/No warning box owned by `owner` (call on the STA shell thread). True on "Yes".
pub fn confirm(_owner: crate::Opaque, _title: &str, _text: &str) -> bool {
    false
}
/// Information box owned by `owner` (call on the STA shell thread).
pub fn alert(_owner: crate::Opaque, _title: &str, _text: &str) {}
/// `shell:Downloads`, `shell:startup`, `shell:sendto`… → file-system path (None for virtual
/// folders like `shell:RecycleBinFolder`). Blocking (shell namespace): off the UI thread.
pub fn resolve_shell_path(_s: &str) -> Option<std::path::PathBuf> {
    None
}
/// No "There is no disk in the drive" / "Insert a disk" system boxes when touching an empty
/// card reader or an ejected disc: such calls just fail (and the app shows "drive not ready").
/// Process-wide; call once at startup.
pub fn suppress_critical_error_dialogs() {}
/// Tells the shell (Explorer windows, desktop, file dialogs) that `dir`'s contents changed,
/// the way the shell's own copy engine does when it finishes.
pub fn notify_updated(_dir: &std::path::Path) {}
