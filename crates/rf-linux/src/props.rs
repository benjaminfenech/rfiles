//! Media/document properties from the Windows property system (duration, resolution, frame
//! rate, bitrate, camera…), formatted by Windows itself (localized). Blocking: off the UI thread.
// Generated from rf-win's public API by a one-off stub generator, then edited by hand.
// Functions return safe defaults until they get a Linux implementation.
#![allow(unused)]
use std::path::Path;
use crate::shell::ensure_com;
use crate::wide::wide;
/// (label, key) pairs shown in the preview pane, in display order.
fn wanted() -> Vec<(&'static str, crate::Opaque)> {
    Default::default()
}
fn display(store: &crate::Opaque, key: &crate::Opaque) -> Option<String> {
    None
}
/// Returns non-empty properties as (label, value).
pub fn media_props(path: &Path) -> Vec<(&'static str, String)> {
    Default::default()
}
