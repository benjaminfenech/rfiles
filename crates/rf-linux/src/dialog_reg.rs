//! "rustyfiles in Open / Save dialogs": registers rustyfiles_dialog.dll (crates/rf-dialog) as the
//! COM server of Windows' FileOpenDialog and FileSaveDialog classes, per user (HKCU, no admin).
//! Apps running as administrator don't read per-user COM registrations and keep Windows' own
//! dialog; so do 32-bit apps (nothing is registered under WOW6432Node).
// Generated from rf-win's public API by a one-off stub generator, then edited by hand.
// Functions return safe defaults until they get a Linux implementation.
#![allow(unused)]
pub const CLASSES: &str = r"Software\Classes";
pub const DLL_NAME: &str = "rustyfiles_dialog.dll";
const CLSIDS: [&str; 2] = [
    "{DC1C5A9C-E88A-4dde-A5A1-60F82A20AEF7}",
    "{C0B4E2F3-BA21-4773-8DBA-335EC946EB8B}",
];
fn server_key(classes_root: &str, clsid: &str) -> String {
    format!(r"{classes_root}\CLSID\{clsid}\InprocServer32")
}
fn registered(classes_root: &str, clsid: &str) -> Option<String> {
    None
}
fn ours(path: &str) -> bool {
    path.to_lowercase().ends_with(DLL_NAME)
}
/// Points both classes at `dll`.
pub fn enable(dll: &str, classes_root: &str) -> anyhow::Result<()> {
    Err(anyhow::anyhow!("rf-linux: `enable` is not implemented on Linux yet").into())
}
/// Removes our registrations (Windows' own dialog comes back); someone else's stays.
pub fn disable(classes_root: &str) -> anyhow::Result<()> {
    Err(anyhow::anyhow!("rf-linux: `disable` is not implemented on Linux yet").into())
}
pub fn is_enabled(classes_root: &str) -> bool {
    CLSIDS.iter().all(|c| registered(classes_root, c).is_some_and(|p| ours(&p)))
}
/// While enabled: registers `dll` if the registration points anywhere else (a newer build, a
/// new install). True when it changed something.
pub fn point_to(dll: &str, classes_root: &str) -> bool {
    let differs = CLSIDS
        .iter()
        .any(|c| {
            registered(classes_root, c).is_none_or(|p| !p.eq_ignore_ascii_case(dll))
        });
    is_enabled(classes_root) && differs && enable(dll, classes_root).is_ok()
}
/// While enabled, a DLL that moved (reinstalled elsewhere, rebuilt) would leave every app's
/// file dialog broken: point the registration at `dll`. True when it changed something.
pub fn heal(dll: &str, classes_root: &str) -> bool {
    let stale = CLSIDS
        .iter()
        .any(|c| {
            registered(classes_root, c)
                .is_some_and(|p| {
                    ours(&p) && !p.eq_ignore_ascii_case(dll)
                        && !std::path::Path::new(&p).exists()
                })
        });
    stale && std::path::Path::new(dll).exists() && enable(dll, classes_root).is_ok()
}
