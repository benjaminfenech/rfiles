//! Media/document properties from the Windows property system (duration, resolution, frame
//! rate, bitrate, camera…), formatted by Windows itself (localized). Blocking: off the UI thread.
// Generated from rf-win's public API by a one-off stub generator, then edited by hand.
// Functions return safe defaults until they get a Linux implementation.
use std::path::Path;
/// Returns non-empty properties as (label, value).
pub fn media_props(_path: &Path) -> Vec<(&'static str, String)> {
    Default::default()
}
