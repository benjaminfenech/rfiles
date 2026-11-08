//! Localized folder names, the way Explorer shows them: a read-only or system folder whose
//! `desktop.ini` has `LocalizedResourceName` (C:\Users → "Kullanıcılar", Downloads →
//! "İndirilenler" on a Turkish system). Only the shown name changes; paths stay real.
//!
//! Results are cached per folder and per resource string (a handful of distinct ones), so
//! breadcrumbs and tab titles can ask on every frame.
// Generated from rf-win's public API by a one-off stub generator, then edited by hand.
// Functions return safe defaults until they get a Linux implementation.
#![allow(unused)]
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use crate::wide::{from_wide, long_path, wide};
const READONLY: u32 = 0x1;
const SYSTEM: u32 = 0x4;
const DIRECTORY: u32 = 0x10;
type Cache<K> = Mutex<HashMap<K, Option<Arc<str>>>>;
fn by_path() -> &'static Cache<PathBuf> {
    static C: OnceLock<Cache<PathBuf>> = OnceLock::new();
    C.get_or_init(Default::default)
}
fn by_resource() -> &'static Cache<String> {
    static C: OnceLock<Cache<String>> = OnceLock::new();
    C.get_or_init(Default::default)
}
/// The localized name of folder `dir`, if it has one. `attrs` are the folder's attributes
/// when already known (from a listing), else they are read.
pub fn folder_label(dir: &Path, attrs: Option<u32>) -> Option<Arc<str>> {
    None
}
/// A UNC path or a network drive letter (reading desktop.ini there costs a round trip).
pub fn is_network(p: &Path) -> bool {
    false
}
/// desktop.ini is UTF-16LE with a BOM, or the ANSI code page (ASCII in practice).
fn decode(raw: &[u8]) -> String {
    if raw.starts_with(&[0xFF, 0xFE]) {
        let units: Vec<u16> = raw[2..]
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        String::from_utf16_lossy(&units)
    } else {
        String::from_utf8_lossy(raw.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(raw))
            .into_owned()
    }
}
/// `LocalizedResourceName=` in the `[.ShellClassInfo]` section.
fn resource_name(ini: &str) -> Option<String> {
    let mut in_section = false;
    for line in ini.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_section = line.eq_ignore_ascii_case("[.ShellClassInfo]");
            continue;
        }
        if !in_section {
            continue;
        }
        if let Some((k, v)) = line.split_once('=') {
            if k.trim().eq_ignore_ascii_case("LocalizedResourceName") {
                let v = v.trim();
                return (!v.is_empty()).then(|| v.to_string());
            }
        }
    }
    None
}
/// "@%SystemRoot%\system32\shell32.dll,-21813" → the string in the user's UI language; a
/// plain value is used as is.
fn resolve(res: &str) -> Option<Arc<str>> {
    None
}
/// `%Name%` → the environment variable's value (unknown names stay as written).
fn expand_env(s: &str) -> String {
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
