//! Ön plandaki pencerenin türü (Explorer, masaüstü, iletişim kutusu…) ve içindeki seçili öğe.
// Generated from rf-win's public API by a one-off stub generator, then edited by hand.
// Functions return safe defaults until they get a Linux implementation.
#![allow(unused)]
use std::path::PathBuf;
use crate::wide::wide;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TargetKind {
    Explorer,
    Desktop,
    Dialog,
    Everything,
    DirectoryOpus,
    None,
}
/// Pencere sınıfı ve içeriğinden hedef türü (saf).
pub fn classify(
    class: &str,
    _title: &str,
    has_shell_view: bool,
    has_dui_view: bool,
) -> TargetKind {
    match class {
        "CabinetWClass" | "ExploreWClass" => TargetKind::Explorer,
        "Progman" | "WorkerW" if has_shell_view => TargetKind::Desktop,
        "#32770" if has_dui_view => TargetKind::Dialog,
        "dopus.lister" => TargetKind::DirectoryOpus,
        c if c.starts_with("EVERYTHING") => TargetKind::Everything,
        _ => TargetKind::None,
    }
}
pub(crate) fn class_of(h: crate::Opaque) -> String {
    Default::default()
}
fn child(parent: crate::Opaque, class: &str) -> Option<crate::Opaque> {
    None
}
/// Ön plandaki pencere ve türü.
pub fn foreground_target() -> (TargetKind, isize) {
    (TargetKind::None, 0)
}
/// Pencerenin iş parçacığında yazı yazılıyor mu (imleç var, menü açık ya da Explorer arama kutusu odakta).
pub fn is_typing(hwnd: isize) -> bool {
    false
}
fn browser_of(disp: &crate::Opaque) -> Option<crate::Opaque> {
    None
}
fn explorer_browser(
    windows: &crate::Opaque,
    hwnd: crate::Opaque,
) -> Option<crate::Opaque> {
    None
}
fn desktop_browser(windows: &crate::Opaque) -> Option<crate::Opaque> {
    None
}
/// Explorer penceresinde ya da masaüstünde seçili ilk öğenin yolu. Çağıran iş parçacığı COM başlatmış olmalı.
pub fn selected_path(kind: TargetKind, hwnd: isize) -> Option<PathBuf> {
    None
}
