//! "Set as default file manager" (Files' Advanced setting), per-user (HKCU only, no admin):
//! folders and Win+E open in rustyfiles. Same keys as Files' SetFilesAsDefault.reg. Archives
//! are offered as an app registration (see `register_app`): the user picks rustyfiles for them
//! in Windows' Default apps.
//!
//! The app must hand non-folder arguments (e.g. `::{CLSID}` shell namespaces like Control Panel)
//! back to explorer.exe — see `rf-app` main.
// Generated from rf-win's public API by a one-off stub generator, then edited by hand.
// Functions return safe defaults until they get a Linux implementation.
#![allow(unused)]
pub const CLASSES: &str = r"Software\Classes";
const WIN_E_CLSID: &str = r"CLSID\{52205fd8-5dfb-447d-801a-d0b52f2e83e1}";
/// zip (CompressedFolder) and 7z, rar, tar, gz… (ArchiveFolder): their `open` verbs. Earlier
/// versions pointed these at rustyfiles; no more: "Open with" then merged Windows' own handler
/// ("File Explorer") with ours into one entry with Explorer's name. Ours are removed.
const ARCHIVE_VERBS: [&str; 2] = [
    r"CompressedFolder\shell\open",
    r"ArchiveFolder\shell\open",
];
fn set_command(base: &crate::Opaque, sub: &str, command: &str) -> anyhow::Result<()> {
    Err(
        anyhow::anyhow!("rf-linux: `set_command` is not implemented on Linux yet").into(),
    )
}
/// Registers `exe` under `classes_root` (normally `CLASSES`; tests pass a scratch key).
pub fn enable(exe: &str, classes_root: &str) -> anyhow::Result<()> {
    Err(anyhow::anyhow!("rf-linux: `enable` is not implemented on Linux yet").into())
}
/// Archive types rustyfiles browses, offered in Windows' Default apps and "Open with". Which
/// app opens them is the user's choice (Windows keeps it signed; no program may set it).
pub const ARCHIVE_EXTS: [&str; 9] = [
    ".zip",
    ".7z",
    ".rar",
    ".tar",
    ".tgz",
    ".txz",
    ".tzst",
    ".tbz2",
    ".tbz",
];
const PROGID: &str = "rustyfiles.Archive";
/// Name under RegisteredApplications (`ms-settings:defaultapps?registeredAppUser=rustyfiles`).
pub const APP_NAME: &str = "rustyfiles";
/// `Software` for `Software\Classes` (tests use a scratch root).
fn software_root(classes_root: &str) -> &str {
    classes_root.strip_suffix(r"\Classes").unwrap_or(classes_root)
}
/// Registers rustyfiles as an app that opens archives (Default Programs capabilities): it
/// can then be picked in Settings → Default apps or "Open with → Always". Changes no default.
pub fn register_app(exe: &str, classes_root: &str) -> anyhow::Result<()> {
    Err(
        anyhow::anyhow!("rf-linux: `register_app` is not implemented on Linux yet")
            .into(),
    )
}
fn unregister_app(classes_root: &str) {}
/// True when the archive app registration points at `exe`.
fn app_registered(exe: &str, classes_root: &str) -> bool {
    false
}
fn verb_command(classes_root: &str, verb: &str) -> Option<String> {
    None
}
fn ours(classes_root: &str, verb: &str) -> bool {
    verb_command(classes_root, verb)
        .is_some_and(|v| v.to_lowercase().contains("rustyfiles"))
}
/// Our old archive verb overrides go (another app's own ones stay).
fn remove_archive_verbs(classes_root: &str) -> bool {
    false
}
/// Brings an older registration up to date: the archive app registration added, the old archive
/// verb overrides removed. True when it changed something.
pub fn add_archives(exe: &str, classes_root: &str) -> bool {
    if !is_enabled(classes_root) {
        return false;
    }
    let removed = remove_archive_verbs(classes_root);
    let added = !app_registered(exe, classes_root)
        && register_app(exe, classes_root).is_ok();
    removed || added
}
/// Removes our overrides (the system defaults under HKLM take over again).
pub fn disable(classes_root: &str) -> anyhow::Result<()> {
    Err(anyhow::anyhow!("rf-linux: `disable` is not implemented on Linux yet").into())
}
pub fn is_enabled(classes_root: &str) -> bool {
    false
}
/// The exe our `Folder\shell\open` command points at (None when not enabled).
pub fn registered_exe(classes_root: &str) -> Option<std::path::PathBuf> {
    None
}
/// While we are the default, a registered exe that no longer exists (moved, rebuilt elsewhere,
/// its folder cleaned) would leave Windows unable to open any folder: point it at `current`.
/// Returns true when it re-registered.
pub fn heal(current: &str, classes_root: &str) -> bool {
    match registered_exe(classes_root) {
        Some(old) if !old.exists() && std::path::Path::new(current).exists() => {
            enable(current, classes_root).is_ok()
        }
        _ => false,
    }
}
/// True when the argument should be handed back to Explorer (shell namespaces, non-folders).
pub fn should_forward_to_explorer(arg: &str) -> bool {
    let a = arg.trim().trim_matches('"');
    !a.is_empty()
        && (a.starts_with("::") || a.starts_with("shell:")
            || !std::path::Path::new(a).is_dir())
}
/// Explorer opens folders through the same `Folder\shell\open` verb we take over, so an
/// argument we hand to it can come straight back (a server root like `\\nas`, a folder that
/// looked like a file). Forwards are remembered for a few seconds to break such a loop.
fn forward_log() -> std::path::PathBuf {
    std::env::temp_dir().join("rustyfiles-forward.txt")
}
fn now_ms() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis())
}
/// True when `arg` was handed to Explorer within the last few seconds.
pub fn forwarded_recently(arg: &str) -> bool {
    forwarded_within(&forward_log(), arg, now_ms(), 5_000)
}
/// Remembers that `arg` is being handed to Explorer.
pub fn note_forward(arg: &str) {
    note_in(&forward_log(), arg, now_ms());
}
fn forwarded_within(
    log: &std::path::Path,
    arg: &str,
    now: u128,
    window_ms: u128,
) -> bool {
    let Ok(text) = std::fs::read_to_string(log) else { return false };
    text.lines()
        .filter_map(|l| l.split_once('\t'))
        .any(|(t, a)| {
            a == arg
                && t.parse::<u128>().is_ok_and(|t| now.saturating_sub(t) <= window_ms)
        })
}
fn note_in(log: &std::path::Path, arg: &str, now: u128) {
    let old = std::fs::read_to_string(log).unwrap_or_default();
    let mut lines: Vec<&str> = old.lines().filter(|l| !l.is_empty()).collect();
    let keep = lines.len().saturating_sub(7);
    lines.drain(..keep);
    let mut out: String = lines.iter().map(|l| format!("{l}\n")).collect();
    out.push_str(&format!("{now}\t{arg}\n"));
    let _ = std::fs::write(log, out);
}
