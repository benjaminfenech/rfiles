//! Localized folder names, the way Explorer shows them: a read-only or system folder whose
//! `desktop.ini` has `LocalizedResourceName` (C:\Users → "Kullanıcılar", Downloads →
//! "İndirilenler" on a Turkish system). Only the shown name changes; paths stay real.
//!
//! Results are cached per folder and per resource string (a handful of distinct ones), so
//! breadcrumbs and tab titles can ask on every frame.
// Generated from rf-win's public API by a one-off stub generator, then edited by hand.
// Functions return safe defaults until they get a Linux implementation.
use std::path::Path;
use std::sync::Arc;
/// The localized name of folder `dir`, if it has one. `attrs` are the folder's attributes
/// when already known (from a listing), else they are read.
pub fn folder_label(_dir: &Path, _attrs: Option<u32>) -> Option<Arc<str>> {
    None
}
/// A UNC path or a network drive letter (reading desktop.ini there costs a round trip).
pub fn is_network(_p: &Path) -> bool {
    false
}
/// `%Name%` → the environment variable's value (unknown names stay as written).
pub fn expand_env(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 32);
    let mut rest = s;
    while let Some(i) = rest.find('%') {
        out.push_str(&rest[..i]);
        let after = &rest[i + 1..];
        match after.find('%') {
            Some(j) => {
                let name = &after[..j];
                match std::env::var(name) {
                    Ok(v) if !name.is_empty() => out.push_str(&v),
                    _ => {
                        out.push('%');
                        out.push_str(name);
                        out.push('%');
                    }
                }
                rest = &after[j + 1..];
            }
            None => {
                out.push_str(&rest[i..]);
                rest = "";
            }
        }
    }
    out.push_str(rest);
    out
}
