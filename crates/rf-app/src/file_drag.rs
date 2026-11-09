//! Dragging files on Linux, with GPUI's own drag (Windows uses a shell OLE drag instead, see
//! `ops::start_drag_out`).
//!
//! Every drop target takes `ExternalPaths` (what other apps drop), so our drags carry that
//! type too: then dropping on a folder, a tab, the sidebar or another window works with the
//! same handlers. A row's drag value is only its own path (built every frame, so it must be
//! cheap); when the drag starts, the whole selection it belongs to is recorded here, and
//! drops expand that one path back to it ([`take`]). Leaving the window hands the files to
//! the desktop as a native drag (GPUI does that on Wayland; X11 isn't supported by GPUI).

use std::path::PathBuf;

use gpui_kit::*;
use parking_lot::Mutex;

/// The drag in progress: (the row's path, everything dragged).
static DRAG: Mutex<Option<(PathBuf, Vec<PathBuf>)>> = Mutex::new(None);

/// The drag of `anchor` started; `items` are all the files it carries.
pub fn begin(anchor: PathBuf, items: Vec<PathBuf>) {
    *DRAG.lock() = Some((anchor, items));
}

/// The files of a drop (ours expanded to the whole selection) — once: the record is cleared.
pub fn take(p: &ExternalPaths) -> Vec<PathBuf> {
    let mut d = DRAG.lock();
    match d.take() {
        Some((anchor, items)) if p.paths() == std::slice::from_ref(&anchor) => items,
        _ => p.paths().to_vec(),
    }
}

/// Like [`take`], for hovering (the record stays).
pub fn peek(p: &ExternalPaths) -> Vec<PathBuf> {
    match &*DRAG.lock() {
        Some((anchor, items)) if p.paths() == std::slice::from_ref(anchor) => items.clone(),
        _ => p.paths().to_vec(),
    }
}

/// What the desktop gets when the drag leaves the window.
pub fn payload(p: &ExternalPaths) -> Option<ExternalDragPayload> {
    let items = peek(p);
    (!items.is_empty()).then(|| ExternalDragPayload::Files(FileDragPaths::new(items.into_iter().map(|p| {
        let dir = p.is_dir();
        (p, dir)
    }))))
}

/// The little label under the pointer: the name, or "N items".
pub struct Ghost(pub SharedString);

impl Render for Ghost {
    fn render(&mut self, window: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let p = crate::theme::Palette::for_window(window);
        div()
            .px_3()
            .py_1p5()
            .rounded(crate::theme::RADIUS)
            .bg(p.card_bg)
            .border_1()
            .border_color(p.accent)
            .text_sm()
            .text_color(p.text)
            .shadow_md()
            .child(self.0.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::{begin, peek, take};
    use gpui_kit::ExternalPaths;
    use std::path::PathBuf;

    #[test]
    fn a_row_stands_for_its_selection() {
        let (a, b) = (PathBuf::from("/d/a"), PathBuf::from("/d/b"));
        begin(a.clone(), vec![a.clone(), b.clone()]);
        let row = ExternalPaths(vec![a.clone()].into());
        assert_eq!(peek(&row), [a.clone(), b.clone()]);
        // another app's drop of other files is left alone
        assert_eq!(peek(&ExternalPaths(vec![b.clone()].into())), [b.clone()]);
        assert_eq!(take(&row), [a.clone(), b]);
        // used up
        assert_eq!(take(&row), [a]);
    }
}
