//! Flyouts with arbitrary content (the "Layout" panel…): a panel under a toolbar button, drawn
//! on the workspace's top layer ([`current`]), closed by clicking elsewhere or Esc. Menus use
//! `fluent_menu`; this is for panels with controls.

use std::rc::Rc;
use std::time::{Duration, Instant};

use gpui_kit::component::v_flex;
use gpui_kit::*;

use crate::theme::Palette;

/// Builds the flyout content on every frame (so it always shows the current state).
pub type Body = Rc<dyn Fn(&Palette, &mut Window, &mut App) -> AnyElement>;

/// A flyout drawn inside its owner window.
struct Inline {
    owner: AnyWindowHandle,
    /// Top-left in the owner's window coordinates.
    pos: Point<Pixels>,
    width: f32,
    body: Body,
    focus: FocusHandle,
    /// Focus to give back on close.
    restore: Option<FocusHandle>,
}

struct Host {
    inline: Option<Inline>,
    key: &'static str,
    /// When a flyout closed itself (focus left): a click on its button right after is the
    /// click that closed it, not a request to reopen.
    closed: Option<(&'static str, Instant)>,
}
impl Global for Host {}

fn host(cx: &mut App) -> &mut Host {
    if !cx.has_global::<Host>() {
        cx.set_global(Host { inline: None, key: "", closed: None });
    }
    cx.global_mut::<Host>()
}

/// Whether a flyout is open (toolbar buttons hide their tooltips meanwhile).
pub fn is_open(cx: &App) -> bool {
    cx.try_global::<Host>().is_some_and(|h| h.inline.is_some())
}

/// Closes the open flyout, if any.
pub fn close(cx: &mut App) {
    if let Some(i) = host(cx).inline.take() {
        // after the current update: the owner may be the window being updated
        cx.defer(move |cx| {
            let _ = i.owner.update(cx, |_, window, cx| {
                if let Some(f) = &i.restore {
                    window.focus(f, cx);
                }
                window.refresh();
            });
        });
    }
}

/// Opens (or, when the same one is open, closes) the flyout `key` under `pos` (owner window
/// coordinates, the flyout's top-left), `width` logical pixels wide.
pub fn toggle(key: &'static str, pos: Point<Pixels>, width: f32, body: Body, owner: &mut Window, cx: &mut App) {
    let h = host(cx);
    if h.inline.is_some() && h.key == key {
        close(cx);
        return;
    }
    if h.closed.is_some_and(|(k, t)| k == key && t.elapsed() < Duration::from_millis(350)) {
        h.closed = None;
        return;
    }
    close(cx);
    let vw = f32::from(owner.viewport_size().width);
    let x = f32::from(pos.x).min(vw - width - 8.).max(8.);
    let focus = cx.focus_handle();
    let restore = owner.focused(cx);
    owner.focus(&focus, cx);
    let h = host(cx);
    h.inline = Some(Inline { owner: owner.window_handle(), pos: point(px(x), pos.y), width, body, focus, restore });
    h.key = key;
    owner.refresh();
}

/// The in-window flyout, if one is open in `window`: a click-away layer with the panel on it.
/// The workspace paints it as one of its top layers.
pub fn current(window: &mut Window, cx: &mut App) -> Option<AnyElement> {
    let (pos, width, body, focus, key) = {
        let i = cx.try_global::<Host>()?.inline.as_ref()?;
        if i.owner != window.window_handle() {
            return None;
        }
        (i.pos, i.width, i.body.clone(), i.focus.clone(), cx.global::<Host>().key)
    };
    let p = Palette::for_window(window);
    let content = body(&p, window, cx);
    let dismiss = move |cx: &mut App| {
        // the click that closes it may land on the flyout's own button: don't reopen there
        host(cx).closed = Some((key, Instant::now()));
        close(cx);
    };
    Some(
        div()
            .id("flyout-layer")
            .absolute()
            .inset_0()
            .on_mouse_down(MouseButton::Left, move |_, _, cx| dismiss(cx))
            .on_mouse_down(MouseButton::Right, move |_, _, cx| dismiss(cx))
            .child(
                v_flex()
                    .id("flyout-panel")
                    .absolute()
                    .left(pos.x)
                    .top(pos.y)
                    .w(px(width))
                    .max_h(window.viewport_size().height - pos.y - px(8.))
                    .overflow_y_scroll()
                    .rounded(crate::theme::RADIUS)
                    .bg(p.card_bg)
                    .border_1()
                    .border_color(p.border)
                    .shadow_lg()
                    .text_color(p.text)
                    .track_focus(&focus)
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_mouse_down(MouseButton::Right, |_, _, cx| cx.stop_propagation())
                    .on_scroll(|_, _, cx| cx.stop_propagation())
                    .on_key_down(|ev: &KeyDownEvent, _, cx| {
                        if ev.keystroke.key == "escape" {
                            close(cx);
                        }
                    })
                    .child(content),
            )
            .into_any_element(),
    )
}
