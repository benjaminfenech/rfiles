//! Select box: a bordered field showing the current value with up/down chevrons; its options
//! open in a panel right under it (drawn on the workspace's top layer), at least as wide as
//! the field, the chosen one ticked on the right. More than [`SEARCH_FROM`] options get
//! a search box. Keys: ↑ ↓, PgUp/PgDn, Home/End, Enter, Esc. Clicking elsewhere closes it.

use std::cell::Cell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use gpui_kit::assets::IconName as L;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{Icon, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::t;
use crate::theme::{self, Palette};

/// More options than this: a search box on top.
pub const SEARCH_FROM: usize = 10;
const ROW_H: f32 = 32.;
const PAD: f32 = 4.;
const SEARCH_H: f32 = 40.;
const LIST_MAX: f32 = 320.;
const EDGE: f32 = 8.;
const PAGE: usize = 8;

pub struct Choice {
    pub label: SharedString,
    pub selected: bool,
    pub run: Rc<dyn Fn(&mut Window, &mut App)>,
}

impl Choice {
    pub fn new(label: impl Into<SharedString>, selected: bool, run: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        Self { label: label.into(), selected, run: Rc::new(run) }
    }
}

/// The open dropdown (one at a time), drawn inside its owner window.
struct Inline {
    owner: AnyWindowHandle,
    view: Entity<SelectView>,
    /// Where the list sits, in the owner's window coordinates.
    bounds: Bounds<Pixels>,
    /// Focus to give back on close.
    restore: Option<FocusHandle>,
}

/// The open dropdown, and which field closed one last: a click on the field that owns the open
/// dropdown first dismisses it; that same click must not reopen it.
#[derive(Default)]
struct Host {
    inline: Option<Inline>,
    closed: Option<(SharedString, Instant)>,
}
impl Global for Host {}

/// The field. `choices` is asked when it is clicked.
pub fn select_box(id: impl Into<SharedString>, p: &Palette, label: impl Into<SharedString>, choices: impl Fn(&App) -> Vec<Choice> + 'static) -> AnyElement {
    select_box_h(id, p, label, 32., choices)
}

/// [`select_box`] at a given height (to line up with other controls in a row).
pub fn select_box_h(id: impl Into<SharedString>, p: &Palette, label: impl Into<SharedString>, height: f32, choices: impl Fn(&App) -> Vec<Choice> + 'static) -> AnyElement {
    let key: SharedString = id.into();
    let bounds = Rc::new(Cell::new(Bounds::<Pixels>::default()));
    let seen = bounds.clone();
    h_flex()
        .id(ElementId::Name(key.clone()))
        .relative()
        .h(px(height))
        .min_w(px(180.))
        .max_w(px(320.))
        .px_3()
        .gap_2()
        .items_center()
        .rounded(theme::RADIUS_SM)
        .bg(p.hover)
        .border_1()
        .border_color(p.border)
        .text_sm()
        .cursor_pointer()
        .hover(|s| s.border_color(p.text_tertiary))
        .child(div().flex_1().min_w_0().overflow_hidden().text_ellipsis().whitespace_nowrap().child(label.into()))
        .child(Icon::new(L::ChevronsUpDown).size(px(14.)).text_color(p.text_secondary))
        .child(canvas(move |b, _, _| seen.set(b), |_, _, _, _| {}).absolute().top_0().left_0().size_full())
        .on_click(move |_, window, cx| {
            let key = key.clone();
            let just_closed = cx
                .try_global::<Host>()
                .and_then(|h| h.closed.clone())
                .is_some_and(|(k, at)| k == key && at.elapsed() < Duration::from_millis(250));
            if !just_closed {
                open(key, bounds.get(), choices(cx), window, cx);
            }
        })
        .into_any_element()
}

/// Turkish-insensitive search key: case, dotted/dotless i and common accents don't matter.
fn fold(s: &str) -> String {
    s.chars()
        .flat_map(|c| c.to_lowercase())
        .map(|c| match c {
            'ı' | 'í' | 'ì' | 'î' | 'ï' => 'i',
            'ç' | 'ć' | 'č' => 'c',
            'ğ' => 'g',
            'ö' | 'ó' | 'ò' | 'ô' | 'õ' | 'ø' => 'o',
            'ş' | 'ś' | 'š' => 's',
            'ü' | 'ú' | 'ù' | 'û' => 'u',
            'á' | 'à' | 'â' | 'ä' | 'ã' | 'å' => 'a',
            'é' | 'è' | 'ê' | 'ë' => 'e',
            'ñ' | 'ń' => 'n',
            'ž' | 'ź' | 'ż' => 'z',
            other => other,
        })
        .filter(|c| *c != '\u{307}') // the dot left over from lowercasing "İ"
        .collect()
}

/// Indices of `labels` matching `query`: prefix matches first, then the rest that contain it.
fn filter(labels: &[SharedString], query: &str) -> Vec<usize> {
    let q = fold(query.trim());
    if q.is_empty() {
        return (0..labels.len()).collect();
    }
    let (mut starts, mut contains) = (Vec::new(), Vec::new());
    for (i, l) in labels.iter().enumerate() {
        let f = fold(l);
        if f.starts_with(&q) {
            starts.push(i);
        } else if f.contains(&q) {
            contains.push(i);
        }
    }
    starts.extend(contains);
    starts
}

fn open(key: SharedString, field: Bounds<Pixels>, choices: Vec<Choice>, window: &mut Window, cx: &mut App) {
    if choices.is_empty() {
        return;
    }
    close_open(cx);
    let search = choices.len() > SEARCH_FROM;
    // wide enough for the longest label (estimated: the list is sized before it is drawn)
    let longest = choices.iter().map(|c| c.label.chars().count()).max().unwrap_or(0) as f32;
    let fw = f32::from(field.size.width);
    let w = (longest * 7.4 + 64.).clamp(fw.max(180.), fw.max(440.));
    let list_h = (choices.len() as f32 * ROW_H).min(LIST_MAX);
    let h = list_h + 2. * PAD + if search { SEARCH_H } else { 0. };

    let main = window.window_handle();
    // below the field when it fits, else above, kept in the window
    let vp = window.viewport_size();
    let (vw, vh) = (f32::from(vp.width), f32::from(vp.height));
    let fx = f32::from(field.origin.x);
    let ftop = f32::from(field.origin.y);
    let fbottom = ftop + f32::from(field.size.height);
    let x = fx.clamp(EDGE, (vw - EDGE - w).max(EDGE));
    let y = if fbottom + 4. + h <= vh - EDGE { fbottom + 4. } else { (ftop - 4. - h).max(EDGE) };
    let restore = window.focused(cx);
    let view = cx.new(|cx| SelectView::new(key, choices, search, main, window, cx));
    cx.default_global::<Host>().inline = Some(Inline { owner: main, view, bounds: Bounds::new(point(px(x), px(y)), size(px(w), px(h))), restore });
    window.refresh();
}

/// Closes the open dropdown (if any), after the current update.
fn close_open(cx: &mut App) {
    if let Some(i) = cx.default_global::<Host>().inline.take() {
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

/// The in-window dropdown, if one is open in `window`: a click-away layer with the list on it.
/// The workspace paints it as one of its top layers.
pub fn current(window: &mut Window, cx: &mut App) -> Option<AnyElement> {
    let (view, bounds) = {
        let i = cx.try_global::<Host>()?.inline.as_ref()?;
        if i.owner != window.window_handle() {
            return None;
        }
        (i.view.clone(), i.bounds)
    };
    let p = Palette::for_window(window);
    let dismiss = {
        let view = view.clone();
        move |cx: &mut App| view.update(cx, |v, cx| v.dismiss(cx))
    };
    let dismiss2 = dismiss.clone();
    Some(
        div()
            .id("select-layer")
            .absolute()
            .inset_0()
            .on_mouse_down(MouseButton::Left, move |_, _, cx| dismiss(cx))
            .on_mouse_down(MouseButton::Right, move |_, _, cx| dismiss2(cx))
            .on_scroll(|_, _, cx| cx.stop_propagation())
            .child(
                div()
                    .id("select-panel")
                    .absolute()
                    .left(bounds.origin.x)
                    .top(bounds.origin.y)
                    .w(bounds.size.width)
                    .h(bounds.size.height)
                    .rounded(theme::RADIUS)
                    .bg(p.card_bg)
                    .border_1()
                    .border_color(p.border)
                    .shadow_lg()
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_mouse_down(MouseButton::Right, |_, _, cx| cx.stop_propagation())
                    .child(view),
            )
            .into_any_element(),
    )
}

struct SelectView {
    key: SharedString,
    choices: Rc<Vec<Choice>>,
    labels: Vec<SharedString>,
    /// Indices into `choices`, in display order.
    shown: Vec<usize>,
    /// Highlighted position in `shown`.
    hl: usize,
    search: Option<Entity<InputState>>,
    focus: FocusHandle,
    scroll: ScrollHandle,
    main: AnyWindowHandle,
    done: bool,
    _subs: Vec<Subscription>,
}

impl SelectView {
    fn new(key: SharedString, choices: Vec<Choice>, search: bool, main: AnyWindowHandle, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let labels: Vec<SharedString> = choices.iter().map(|c| c.label.clone()).collect();
        let hl = choices.iter().position(|c| c.selected).unwrap_or(0);
        let focus = cx.focus_handle();
        let mut subs = vec![cx.observe_window_activation(window, |this: &mut Self, window, cx| {
            if !window.is_window_active() {
                this.dismiss(cx);
            }
        })];
        let search = search.then(|| {
            let input = cx.new(|cx| InputState::new(window, cx).placeholder(t!("Search")));
            subs.push(cx.subscribe_in(&input, window, |this: &mut Self, input, ev: &InputEvent, _, cx| {
                if let InputEvent::Change = ev {
                    let q = input.read(cx).value().to_string();
                    this.shown = filter(&this.labels, &q);
                    this.hl = 0;
                    this.scroll.scroll_to_item(0);
                    cx.notify();
                }
            }));
            input
        });
        match &search {
            Some(input) => input.update(cx, |s, cx| s.focus(window, cx)),
            None => window.focus(&focus, cx),
        }
        let scroll = ScrollHandle::new();
        scroll.scroll_to_item(hl);
        Self { key, shown: (0..choices.len()).collect(), choices: Rc::new(choices), labels, hl, search, focus, scroll, main, done: false, _subs: subs }
    }

    /// Closes without choosing (Esc, a click elsewhere).
    fn dismiss(&mut self, cx: &mut Context<Self>) {
        if self.done {
            return;
        }
        self.done = true;
        cx.default_global::<Host>().closed = Some((self.key.clone(), Instant::now()));
        close_open(cx);
    }

    fn pick(&mut self, pos: usize, cx: &mut Context<Self>) {
        let Some(&ix) = self.shown.get(pos) else { return };
        let run = self.choices[ix].run.clone();
        let main = self.main;
        self.dismiss(cx);
        cx.defer(move |cx| {
            let _ = main.update(cx, |_, window, cx| {
                window.activate_window();
                run(window, cx);
                window.refresh();
            });
        });
    }

    fn move_hl(&mut self, to: usize, cx: &mut Context<Self>) {
        if self.shown.is_empty() {
            return;
        }
        self.hl = to.min(self.shown.len() - 1);
        self.scroll.scroll_to_item(self.hl);
        cx.notify();
    }

    fn on_key(&mut self, ev: &KeyDownEvent, cx: &mut Context<Self>) {
        let n = self.shown.len();
        match ev.keystroke.key.as_str() {
            "escape" => self.dismiss(cx),
            "enter" => self.pick(self.hl, cx),
            "down" => self.move_hl(if self.hl + 1 < n { self.hl + 1 } else { 0 }, cx),
            "up" => self.move_hl(if self.hl > 0 { self.hl - 1 } else { n.saturating_sub(1) }, cx),
            "pagedown" => self.move_hl(self.hl + PAGE, cx),
            "pageup" => self.move_hl(self.hl.saturating_sub(PAGE), cx),
            // with a search box, Home/End move in the text
            "home" if self.search.is_none() => self.move_hl(0, cx),
            "end" if self.search.is_none() => self.move_hl(n.saturating_sub(1), cx),
            _ => return,
        }
        cx.stop_propagation();
    }
}

impl Render for SelectView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = Palette::for_window(window);
        let mut list = v_flex().id("select-list").flex_1().min_h_0().overflow_y_scroll().track_scroll(&self.scroll);
        if self.shown.is_empty() {
            list = list.child(div().h(px(ROW_H)).flex().items_center().justify_center().text_sm().text_color(p.text_secondary).child(t!("No matching items")));
        }
        for (pos, &ix) in self.shown.iter().enumerate() {
            let c = &self.choices[ix];
            list = list.child(
                h_flex()
                    .id(("opt", ix))
                    .flex_shrink_0()
                    .h(px(ROW_H))
                    .px_3()
                    .gap_2()
                    .items_center()
                    .rounded(px(4.))
                    .text_sm()
                    .when(pos == self.hl, |d| d.bg(p.hover))
                    .active(|s| s.bg(p.pressed))
                    .on_hover(cx.listener(move |this, on: &bool, _, cx| {
                        if *on && this.hl != pos {
                            this.hl = pos;
                            cx.notify();
                        }
                    }))
                    .on_click(cx.listener(move |this, _, _, cx| this.pick(pos, cx)))
                    .child(div().flex_1().min_w_0().overflow_hidden().text_ellipsis().whitespace_nowrap().child(c.label.clone()))
                    .child(div().w(px(16.)).flex_shrink_0().when(c.selected, |d| d.child(Icon::new(L::Check).size(px(16.)).text_color(p.accent)))),
            );
        }
        v_flex()
            .size_full()
            .p(px(PAD))
            .text_color(p.text)
            .track_focus(&self.focus)
            .capture_key_down(cx.listener(|this, ev: &KeyDownEvent, _, cx| this.on_key(ev, cx)))
            .children(self.search.as_ref().map(|input| {
                div()
                    .flex_shrink_0()
                    .h(px(SEARCH_H - PAD))
                    .mb(px(PAD))
                    .child(Input::new(input).prefix(Icon::new(L::Search).size(px(14.)).text_color(p.text_secondary)))
            }))
            .child(list)
    }
}

#[cfg(test)]
mod tests {
    use super::{filter, fold};
    use gpui_kit::SharedString;

    #[test]
    fn search_ignores_case_and_turkish_letters() {
        assert_eq!(fold("İngilizce"), "ingilizce");
        assert_eq!(fold("Türkçe"), "turkce");
        assert_eq!(fold("Français"), "francais");
        let labels: Vec<SharedString> = ["English", "Türkçe", "Deutsch", "Português (Brasil)"].into_iter().map(Into::into).collect();
        assert_eq!(filter(&labels, "tur"), vec![1]);
        assert_eq!(filter(&labels, "SC"), vec![2]); // contains
        assert_eq!(filter(&labels, "p"), vec![3]);
        assert_eq!(filter(&labels, ""), vec![0, 1, 2, 3]);
        assert_eq!(filter(&labels, "e"), vec![0, 1, 2, 3]); // "English" starts, the rest contain
    }
}
