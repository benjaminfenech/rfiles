//! Linux stand-in for the speed test page (it races File Explorer, Files and File Pilot:
//! Windows only). Settings hides the page; `--settings=speed` shows it empty.

use gpui_kit::*;

use crate::theme::Palette;

pub fn page(_p: &Palette, _cx: &mut App) -> Vec<AnyElement> {
    Vec::new()
}

/// 2330750 → "2.330.750" (Turkish) / "2,330,750" (as the Windows page's).
pub(crate) fn count(n: u64) -> String {
    let sep = if crate::i18n::t("1.000 dosya").starts_with("1.000") { '.' } else { ',' };
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push(sep);
        }
        out.push(c);
    }
    out
}
