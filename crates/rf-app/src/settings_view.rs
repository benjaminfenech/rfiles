//! Settings page (Location::Settings), laid out like Files: a navigation column on the left,
//! one page of cards on the right. Every card is one setting: icon, title, optional
//! description, and its control (a switch, a drop-down that opens our Acrylic menu, buttons) —
//! or an expander with child settings.

use std::collections::HashSet;

use gpui_kit::assets::IconName as L;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{Icon, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::fluent_menu::{self, Item, Menu, MenuIcon};
use crate::select_box::{Choice, select_box};
use crate::i18n::Lang;
use crate::settings::{
    self, AnimPref, BackdropPref, ClickPref, DefaultView, DeletePref, IdePref, Settings, StartupPref, ThemePref,
};
use crate::t;
use crate::look::config::LookSettings;
use crate::theme::{self, Palette};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Page {
    General,
    Appearance,
    Layout,
    Files,
    Actions,
    Tags,
    Look,
    Dev,
    Advanced,
    About,
}

const PAGES: [(Page, L, &str); 10] = [
    (Page::General, L::Settings, "General"),
    (Page::Appearance, L::Palette, "Appearance"),
    (Page::Layout, L::LayoutGrid, "Layout"),
    (Page::Files, L::Folder, "Files and folders"),
    (Page::Actions, L::Keyboard, "Actions"),
    (Page::Tags, L::Tag, "Tags"),
    (Page::Look, L::Eye, "DirectLook"),
    (Page::Dev, L::SquareTerminal, "Developer tools"),
    (Page::Advanced, L::Wrench, "Advanced"),
    (Page::About, L::Info, "About"),
];

/// Which page is shown and which expanders are open (kept while the app runs).
struct Ui {
    page: Page,
    open: HashSet<&'static str>,
    /// The date pattern field (made on first show; it needs a window), and the pattern it last
    /// saved, to tell a change made elsewhere (an import) from the user's typing.
    date_input: Option<(Entity<InputState>, String)>,
}
impl Global for Ui {}

fn ui(cx: &mut App) -> &mut Ui {
    if !cx.has_global::<Ui>() {
        cx.set_global(Ui { page: Page::General, open: HashSet::new(), date_input: None });
    }
    cx.global_mut::<Ui>()
}

pub fn open_page(page: Page, cx: &mut App) {
    ui(cx).page = page;
}

// ---------------------------------------------------------------- controls

/// A Windows 11 switch with its state word ("Açık"/"Kapalı") on the left.
pub(crate) fn switch(id: impl Into<ElementId>, p: &Palette, on: bool, toggle: impl Fn(&mut Window, &mut App) + 'static) -> AnyElement {
    let knob = if on { p.accent_text } else { p.text_secondary };
    h_flex()
        .gap_3()
        .items_center()
        .child(div().text_sm().text_color(p.text_secondary).child(if on { t!("On") } else { t!("Off") }))
        .child(
            h_flex()
                .id(id)
                .w(px(40.))
                .h(px(20.))
                .px(px(3.))
                .items_center()
                .rounded_full()
                .border_1()
                .when(on, |d| d.bg(p.accent).border_color(p.accent).justify_end())
                .when(!on, |d| d.border_color(p.text_secondary).hover(|s| s.bg(p.hover)))
                .child(div().size(px(12.)).rounded_full().bg(knob))
                .on_click(move |_, window, cx| toggle(window, cx)),
        )
        .into_any_element()
}

/// Switch bound to a boolean setting.
fn sw(id: &'static str, p: &Palette, on: bool, set: fn(&mut Settings, bool)) -> AnyElement {
    switch(id, p, on, move |_, cx| settings::update(cx, |s| set(s, !on)))
}

/// A button showing the current value; clicking opens the choices in our menu.
fn dropdown<T: Copy + PartialEq + 'static>(id: &'static str, p: &Palette, current: T, options: Vec<(T, String)>, set: fn(&mut Settings, T)) -> AnyElement {
    let label = options.iter().find(|(v, _)| *v == current).map(|(_, l)| l.clone()).unwrap_or_default();
    select_box(id, p, label, move |_| {
        options.iter().map(|(v, l)| {
            let v = *v;
            Choice::new(l.clone(), v == current, move |_, cx| settings::update(cx, |s| set(s, v)))
        }).collect()
    })
}

fn button(id: impl Into<ElementId>, p: &Palette, label: impl Into<SharedString>, run: impl Fn(&mut Window, &mut App) + 'static) -> AnyElement {
    div()
        .id(id)
        .h(px(32.))
        .px_4()
        .flex()
        .items_center()
        .rounded(theme::RADIUS_SM)
        .bg(p.hover)
        .border_1()
        .border_color(p.border)
        .text_sm()
        .hover(|s| s.bg(p.pressed))
        .active(|s| s.opacity(0.8))
        .child(label.into())
        .on_click(move |_, window, cx| run(window, cx))
        .into_any_element()
}

// ---------------------------------------------------------------- layout pieces

fn card_body(p: &Palette, icon: L, title: SharedString, desc: Option<SharedString>, control: Option<AnyElement>) -> Div {
    h_flex()
        .min_h(px(64.))
        .px_4()
        .py_3()
        .gap_4()
        .items_center()
        .child(Icon::new(icon).size(px(18.)).flex_shrink_0().text_color(p.text))
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .gap_0p5()
                .child(div().text_sm().child(title))
                .children(desc.map(|d| div().text_xs().text_color(p.text_secondary).child(d))),
        )
        .children(control)
}

fn shell(p: &Palette) -> Div {
    v_flex().w_full().rounded(theme::RADIUS).bg(p.hover).border_1().border_color(p.border)
}

/// One setting row.
fn card(p: &Palette, icon: L, title: &str, desc: Option<&str>, control: AnyElement) -> AnyElement {
    shell(p)
        .child(card_body(p, icon, title.to_string().into(), desc.map(|d| d.to_string().into()), Some(control)))
        .into_any_element()
}

/// A card that opens to show `children` (state kept per `key`).
fn expander(key: &'static str, p: &Palette, icon: L, title: &str, desc: Option<&str>, control: Option<AnyElement>, children: impl FnOnce() -> Vec<AnyElement>, cx: &mut App) -> AnyElement {
    let open = ui(cx).open.contains(key);
    let chevron = div()
        .id(SharedString::from(format!("exp-chev-{key}")))
        .size(px(28.))
        .flex()
        .items_center()
        .justify_center()
        .rounded(theme::RADIUS_SM)
        .hover(|s| s.bg(p.pressed))
        .child(Icon::new(if open { L::ChevronUp } else { L::ChevronDown }).size(px(14.)))
        .on_click(move |_, window, cx| {
            let u = ui(cx);
            if !u.open.remove(key) {
                u.open.insert(key);
            }
            window.refresh();
        });
    let header_control = h_flex().gap_2().items_center().children(control).child(chevron).into_any_element();
    shell(p)
        .child(card_body(p, icon, title.to_string().into(), desc.map(|d| d.to_string().into()), Some(header_control)))
        .when(open, |d| {
            d.child(div().h(px(1.)).bg(p.divider)).child(v_flex().py_1().children(children()))
        })
        .into_any_element()
}

/// A child row inside an expander (no icon, indented like Files).
fn sub(p: &Palette, title: &str, desc: Option<&str>, control: AnyElement) -> AnyElement {
    h_flex()
        .min_h(px(48.))
        .pl(px(56.))
        .pr_4()
        .py_2()
        .gap_4()
        .items_center()
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .child(div().text_sm().child(title.to_string()))
                .children(desc.map(|d| div().text_xs().text_color(p.text_secondary).child(d.to_string()))),
        )
        .child(control)
        .into_any_element()
}

fn section(p: &Palette, title: &str) -> AnyElement {
    div().pt_5().pb_1().text_sm().font_weight(FontWeight::SEMIBOLD).text_color(p.text).child(title.to_string()).into_any_element()
}

fn link_row(p: &Palette, id: &'static str, icon: L, title: &str, desc: Option<&str>, run: impl Fn(&mut Window, &mut App) + 'static) -> AnyElement {
    shell(p)
        .id(id)
        .hover(|s| s.bg(p.pressed))
        .on_click(move |_, window, cx| run(window, cx))
        .child(card_body(
            p,
            icon,
            title.to_string().into(),
            desc.map(|d| d.to_string().into()),
            Some(Icon::new(L::SquareArrowOutUpRight).size(px(14.)).text_color(p.text_secondary).into_any_element()),
        ))
        .into_any_element()
}

// ---------------------------------------------------------------- page

pub fn render(p: &Palette, window: &mut Window, cx: &mut App) -> AnyElement {
    let page = ui(cx).page;
    let s = cx.global::<Settings>().clone();
    let mut nav = v_flex().w(px(232.)).flex_shrink_0().h_full().px_3().pt_5().gap_0p5().child(
        div().px_2().pb_3().text_xl().font_weight(FontWeight::SEMIBOLD).child(t!("Settings")),
    );
    for (i, (pg, icon, label)) in PAGES.iter().copied().enumerate() {
        let on = pg == page;
        nav = nav.child(
            h_flex()
                .id(("set-nav", i))
                .h(px(36.))
                .px_3()
                .gap_3()
                .items_center()
                .relative()
                .rounded(theme::RADIUS_SM)
                .text_sm()
                .when(on, |d| d.bg(p.selection).child(theme::selection_pill(p)))
                .when(!on, |d| d.hover(|s| s.bg(p.hover)))
                .child(Icon::new(icon).size(px(16.)).text_color(if on { p.accent } else { p.text_secondary }))
                .child(crate::i18n::t(label))
                .on_click(move |_, window, cx| {
                    open_page(pg, cx);
                    window.refresh();
                }),
        );
    }
    let title = PAGES.iter().find(|x| x.0 == page).map(|x| crate::i18n::t(x.2)).unwrap_or_default();
    let body: Vec<AnyElement> = match page {
        Page::General => general(p, &s, window, cx),
        Page::Appearance => appearance(p, &s, cx),
        Page::Layout => layout(p, &s, cx),
        Page::Files => files(p, &s, cx),
        Page::Actions => actions(p, cx),
        Page::Tags => tags(p, cx),
        Page::Look => look(p, &s, cx),
        Page::Dev => dev(p, &s),
        Page::Advanced => advanced(p, &s),
        Page::About => about(p, cx),
    };
    h_flex()
        .size_full()
        .child(nav)
        .child(
            div().id("settings-page").flex_1().min_w_0().h_full().overflow_y_scroll().child(
                v_flex()
                    .w_full()
                    .max_w(px(980.))
                    .px_6()
                    .pt_5()
                    .pb_8()
                    .gap_2()
                    .child(div().pb_2().text_xl().font_weight(FontWeight::SEMIBOLD).child(title))
                    .children(body),
            ),
        )
        .into_any_element()
}

/// What a desktop shortcut runs to open the quick search (Linux has no global hotkey).
/// The command a desktop shortcut runs for quick search (Linux).
fn qs_command() -> &'static str {
    static CMD: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    CMD.get_or_init(|| {
        if let Some(id) = rf_linux::sandbox::flatpak_id() {
            return format!("flatpak run {id} --quick-search");
        }
        "rfiles --quick-search".to_string()
    })
}

fn qs_command_desc() -> &'static str {
    static DESC: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    DESC.get_or_init(|| {
        let c = qs_command();
        format!("Add a shortcut in your desktop's keyboard settings that runs `{c}` (Hyprland: bind = ALT, SPACE, exec, {c})")
    })
}

/// Quick search: on/off, the command a desktop shortcut runs, web engine, bookmarks.
fn quick_search_cards(p: &Palette, s: &Settings) -> Vec<AnyElement> {
    vec![
        section(p, t!("Quick search")),
        card(
            p,
            L::Search,
            t!("Quick search"),
            Some(if s.run_in_background {
                t!("A shortcut from anywhere: apps, files, folders, bookmarks and the web")
            } else {
                t!("A shortcut from anywhere: apps, files, folders, bookmarks and the web. To use it with no window open, turn on running in the background under Advanced")
            }),
            sw("quick-search", p, s.quick_search, |s, v| s.quick_search = v),
        ),
        // through the desktop portal (KDE, GNOME 48+, Hyprland): asks once to confirm the key
        card(
            p,
            L::Keyboard,
            t!("Global shortcut"),
            Some(&crate::global_keys::status().filter(|_| s.global_shortcut).unwrap_or_else(|| {
                t!("Open the quick search from any app with a shortcut your desktop assigns (KDE and GNOME ask you to confirm it)").to_string()
            })),
            h_flex()
                .gap_2()
                .items_center()
                .when(s.global_shortcut && crate::global_keys::can_configure(), |d| {
                    d.child(button("qs-configure", p, t!("Change"), |_, _| crate::global_keys::configure()))
                })
                .child(sw("global-shortcut", p, s.global_shortcut, |s, v| s.global_shortcut = v))
                .into_any_element(),
        ),
        // without the portal: a shortcut of the desktop's own runs the command
        card(
            p,
            L::Keyboard,
            t!("Shortcut"),
            Some(qs_command_desc()),
            button("qs-copy-command", p, "Copy command", |_, cx| cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string(qs_command().to_string()))),
        ),
        card(
            p,
            L::Globe,
            t!("Web search"),
            None,
            dropdown(
                "web-search",
                p,
                s.web_search,
                vec![
                    (crate::settings::WebSearch::Google, "Google".into()),
                    (crate::settings::WebSearch::Bing, "Bing".into()),
                    (crate::settings::WebSearch::DuckDuckGo, "DuckDuckGo".into()),
                    (crate::settings::WebSearch::Yandex, "Yandex".into()),
                ],
                |s, v| s.web_search = v,
            ),
        ),
        card(
            p,
            L::Bookmark,
            t!("Search Chrome and Edge bookmarks too"),
            Some(t!("Bookmarks are read on this PC and sent nowhere")),
            sw("qs-bookmarks", p, s.quick_search_bookmarks, |s, v| s.quick_search_bookmarks = v),
        ),
    ]
}

/// The date pattern field, kept between frames (it is made once, with the window).
fn date_input(current: &str, window: &mut Window, cx: &mut App) -> Entity<InputState> {
    if let Some((inp, written)) = ui(cx).date_input.clone() {
        // changed elsewhere (settings imported): show the new pattern
        if written != current && inp.read(cx).value() != current {
            let v = current.to_string();
            inp.update(cx, |s, cx| s.set_value(v.clone(), window, cx));
            ui(cx).date_input = Some((inp.clone(), v));
        }
        return inp;
    }
    let inp = cx.new(|cx| {
        let mut s = InputState::new(window, cx).placeholder(rf_core::format::DEFAULT_DATE_PATTERN);
        s.set_value(current.to_string(), window, cx);
        s
    });
    // a valid pattern is saved as it is typed; an invalid one only shows why (below the field)
    cx.subscribe(&inp, |inp, ev: &InputEvent, cx| {
        if !matches!(ev, InputEvent::Change) {
            return;
        }
        let v = inp.read(cx).value().trim().to_string();
        if rf_core::format::parse_date_pattern(&v).is_ok() {
            if let Some((_, written)) = &mut ui(cx).date_input {
                *written = v.clone();
            }
            settings::update(cx, |s| s.date_pattern = v);
        }
        cx.refresh_windows();
    })
    .detach();
    ui(cx).date_input = Some((inp.clone(), current.to_string()));
    inp
}

/// Date format: a pattern field, with a preview (or what is wrong) under the title.
fn date_pattern_card(p: &Palette, s: &Settings, window: &mut Window, cx: &mut App) -> AnyElement {
    let inp = date_input(&s.date_pattern, window, cx);
    let typed = inp.read(cx).value().trim().to_string();
    let now = chrono::Local::now().timestamp_millis();
    let desc = match rf_core::format::preview_date_pattern(&typed, now) {
        Some(sample) => format!("Now: {sample}"),
        None => match rf_core::format::parse_date_pattern(&typed) {
            Err(bad) if !bad.is_empty() => format!("\"{bad}\" isn't a date field; still using {}", s.date_pattern),
            _ => format!("Type a format, e.g. dd/mm/yyyy hh:mm; still using {}", s.date_pattern),
        },
    };
    card(p, L::CalendarClock, t!("Date format"), Some(&desc), div().w(px(220.)).child(Input::new(&inp)).into_any_element())
}

/// What the date pattern accepts, and a few to pick from.
fn date_help(p: &Palette) -> AnyElement {
    const PRESETS: [&str; 4] = ["dd/mm/yyyy hh:mm", "dd-mm-yyyy hh:mm", "yyyy-mm-dd hh:mm", "dd.mm.yyyy hh:mm"];
    let chips = PRESETS.iter().enumerate().map(|(i, pat)| {
        let pat = *pat;
        div()
            .id(("date-preset", i))
            .px_2()
            .py_0p5()
            .rounded(theme::RADIUS_SM)
            .border_1()
            .border_color(p.border)
            .text_xs()
            .font_family(crate::sys::mono_font())
            .cursor_pointer()
            .hover(|d| d.bg(p.hover))
            .child(pat)
            .on_click(move |_, window, cx| {
                if let Some((inp, _)) = ui(cx).date_input.clone() {
                    inp.update(cx, |s, cx| s.set_value(pat.to_string(), window, cx));
                    ui(cx).date_input = Some((inp, pat.to_string()));
                }
                settings::update(cx, |s| s.date_pattern = pat.to_string());
                window.refresh();
            })
    });
    v_flex()
        .pl(px(56.))
        .pr_4()
        .pb_2()
        .gap_1p5()
        .text_xs()
        .text_color(p.text_secondary)
        .child(
            "dd day · mm month (mmm Nov, mmmm November) · yy or yyyy year · hh hour · mm right after hh: minutes · ss seconds. \
             Upper or lower case; anything else (: . / - , spaces) is shown as typed.",
        )
        .child(h_flex().gap_1p5().flex_wrap().children(chips))
        .into_any_element()
}

fn general(p: &Palette, s: &Settings, window: &mut Window, cx: &mut App) -> Vec<AnyElement> {
    let mut v = vec![
        card(
            p,
            L::Languages,
            t!("Language"),
            None,
            dropdown(
                "lang",
                p,
                s.language,
                std::iter::once((Lang::System, t!("Use system setting").to_string()))
                    .chain(Lang::all().into_iter().map(|l| (l, l.native_name().to_string())))
                    .collect(),
                |s, v| s.language = v,
            ),
        ),
        date_pattern_card(p, s, window, cx),
        date_help(p),
        card(
            p,
            L::CalendarClock,
            "Recent times as \"5 minutes ago\"",
            Some("In the file list, for the last week; older dates use the format above"),
            sw("relative-dates", p, s.relative_dates, |s, v| s.relative_dates = v),
        ),
    ];
    let startup_dd = dropdown(
        "startup",
        p,
        s.startup,
        vec![
            (StartupPref::Continue, t!("Continue where you left off").into()),
            (StartupPref::NewTab, t!("Open a new tab").into()),
            (StartupPref::Pages, t!("Open specific pages").into()),
        ],
        |s, v| s.startup = v,
    );
    let pages = s.startup_pages.clone();
    let pp = *p;
    v.push(expander(
        "startup",
        p,
        L::Power,
        t!("Startup settings"),
        None,
        Some(startup_dd),
        move || {
            let mut rows: Vec<AnyElement> = pages
                .iter()
                .enumerate()
                .map(|(i, page)| {
                    let shown = if page.is_empty() { t!("Home").to_string() } else { page.clone() };
                    sub(
                        &pp,
                        &shown,
                        None,
                        button(("sp-del", i), &pp, t!("Remove"), move |_, cx| settings::update(cx, |s| {
                            if i < s.startup_pages.len() {
                                s.startup_pages.remove(i);
                            }
                        })),
                    )
                })
                .collect();
            rows.push(sub(
                &pp,
                t!("Specific pages"),
                Some(t!("Opens when \"Open specific pages\" is selected.")),
                button("sp-use", &pp, t!("Use currently open tabs"), |_, cx| {
                    settings::update(cx, |s| {
                        s.startup_pages = s.session_tabs.clone();
                        s.startup = StartupPref::Pages;
                    })
                }),
            ));
            rows
        },
        cx,
    ));
    v.push(card(p, L::SquarePlus, t!("Always switch to the newly opened tab"), None, sw("focus-tab", p, s.focus_new_tab, |s, v| s.focus_new_tab = v)));

    v.push(section(p, t!("Dual pane")));
    v.push(card(p, L::Columns2, t!("Open new tabs in dual pane"), None, sw("dual-new", p, s.dual_pane_new_tabs, |s, v| s.dual_pane_new_tabs = v)));
    v.push(card(
        p,
        L::SquareSplitHorizontal,
        t!("Dual pane orientation"),
        None,
        dropdown(
            "dual-dir",
            p,
            s.dual_pane_vertical,
            vec![(true, t!("Side by side").into()), (false, t!("Stacked").into())],
            |s, v| s.dual_pane_vertical = v,
        ),
    ));

    v.push(section(p, t!("Context menu")));
    let m = s.menu.clone();
    let pp = *p;
    v.push(expander(
        "ctx",
        p,
        L::SquareMenu,
        t!("Context menu options"),
        Some(t!("Which commands appear in the menu")),
        None,
        move || {
            type Set = fn(&mut Settings, bool);
            let rows: [(&str, bool, Set); 8] = [
                (t!("Open in new tab"), m.open_in_new_tab, |s, v| s.menu.open_in_new_tab = v),
                (t!("Open in new window"), m.open_in_new_window, |s, v| s.menu.open_in_new_window = v),
                (t!("Copy item path"), m.copy_path, |s, v| s.menu.copy_path = v),
                (t!("Pin to sidebar"), m.pin, |s, v| s.menu.pin = v),
                (t!("Compress / Extract"), m.compress, |s, v| s.menu.compress = v),
                (t!("Tags"), m.tags, |s, v| s.menu.tags = v),
                (t!("Add to shelf"), m.shelf, |s, v| s.menu.shelf = v),
                (t!("Open in terminal"), m.terminal, |s, v| s.menu.terminal = v),
            ];
            rows.into_iter()
                .enumerate()
                .map(|(i, (label, on, set))| sub(&pp, label, None, switch(("ctx-sw", i), &pp, on, move |_, cx| settings::update(cx, |s| set(s, !on)))))
                .collect()
        },
        cx,
    ));

    v.push(section(p, t!("Scrolling")));
    v.push(card(p, L::ChevronsUpDown, t!("Smooth scrolling"), None, sw("smooth", p, s.smooth_scroll, |s, v| s.smooth_scroll = v)));
    v.extend(quick_search_cards(p, s));
    v
}

/// Background tint presets (Files' "Background color").
const TINTS: [(&str, u32); 15] = [
    ("Goldenrod", 0xC19C00),
    ("Orange", 0xCA5010),
    ("Brick red", 0xA4262C),
    ("Red", 0xD13438),
    ("Rose", 0xE3008C),
    ("Purple", 0x881798),
    ("Iris", 0x8764B8),
    ("Blue", 0x0078D4),
    ("Sea", 0x038387),
    ("Mint", 0x00B294),
    ("Green", 0x107C10),
    ("Camouflage", 0x486860),
    ("Storm", 0x4C4A48),
    ("Blue gray", 0x515C6B),
    ("Gray", 0x7A7574),
];

fn appearance(p: &Palette, s: &Settings, cx: &mut App) -> Vec<AnyElement> {
    let mut v = vec![
        card(
            p,
            L::SunMoon,
            t!("Theme"),
            None,
            dropdown(
                "theme",
                p,
                s.theme,
                vec![(ThemePref::System, t!("Use system setting").into()), (ThemePref::Light, t!("Light").into()), (ThemePref::Dark, t!("Dark").into())],
                |s, v| s.theme = v,
            ),
        ),
        card(
            p,
            L::Layers,
            t!("Background material"),
            Some(t!("Translucent windows let the desktop show through; with blur, compositors that support it (KDE, Hyprland) blur it.")),
            dropdown(
                "backdrop",
                p,
                s.backdrop,
                // Mica Alt looks like Mica here: both are a see-through window
                vec![
                    (BackdropPref::Mica, t!("Translucent").into()),
                    (BackdropPref::Acrylic, t!("Translucent with blur").into()),
                    (BackdropPref::Solid, t!("Solid").into()),
                ],
                |s, v| s.backdrop = v,
            ),
        ),
        card(
            p,
            L::Activity,
            t!("Animations"),
            Some(t!("Short transitions when tabs open and close. Never delays anything.")),
            dropdown(
                "animations",
                p,
                s.animations,
                vec![
                    (AnimPref::System, t!("Use system setting").into()),
                    (AnimPref::On, t!("On").into()),
                    (AnimPref::Off, t!("Off").into()),
                ],
                |s, v| s.animations = v,
            ),
        ),
        card(
            p,
            L::PanelRight,
            t!("Show scrollbar"),
            Some(t!("On the right of the Details and Grid lists. Drag it, or click a spot to jump there.")),
            sw("scrollbar", p, s.show_scrollbar, |s, v| s.show_scrollbar = v),
        ),
    ];
    // tint swatches
    let tint = s.tint;
    let pp = *p;
    let swatch_now: AnyElement = match tint {
        Some(c) => div().size(px(20.)).rounded(px(4.)).bg(rgb(c)).border_1().border_color(p.border).into_any_element(),
        None => div().text_sm().text_color(p.text_secondary).child(t!("Default")).into_any_element(),
    };
    v.push(expander(
        "tint",
        p,
        L::Paintbrush,
        t!("Background color"),
        None,
        Some(swatch_now),
        move || {
            let tile = |id: usize, label: String, color: Option<u32>, selected: bool| {
                v_flex()
                    .id(("tint", id))
                    .w(px(96.))
                    .gap_1()
                    .items_center()
                    .p_1()
                    .rounded(theme::RADIUS_SM)
                    .border_1()
                    .border_color(if selected { pp.accent } else { transparent_black() })
                    .hover(|s| s.bg(pp.pressed))
                    .child(
                        div()
                            .w_full()
                            .h(px(44.))
                            .rounded(theme::RADIUS_SM)
                            .border_1()
                            .border_color(pp.border)
                            .when_some(color, |d, c| d.bg(hsla_alpha(c, 0.55)))
                            .when(color.is_none(), |d| d.bg(pp.content_bg)),
                    )
                    .child(div().text_xs().text_color(pp.text_secondary).child(label))
                    .on_click(move |_, _, cx| settings::update(cx, |s| s.tint = color))
            };
            let mut grid = div().px_4().py_3().flex().flex_wrap().gap_2();
            grid = grid.child(tile(0, t!("Default").to_string(), None, tint.is_none()));
            for (i, (name, c)) in TINTS.iter().enumerate() {
                grid = grid.child(tile(i + 1, crate::i18n::t(name).to_string(), Some(*c), tint == Some(*c)));
            }
            vec![grid.into_any_element()]
        },
        cx,
    ));
    // background image
    let img = s.background_image.clone();
    let opacity = s.background_opacity;
    let pp = *p;
    let img_desc = img.as_ref().map(|i| i.display().to_string());
    v.push(expander(
        "bgimg",
        p,
        L::Image,
        t!("Background image"),
        Some(img_desc.as_deref().unwrap_or(t!("None"))),
        Some(button("bg-browse", p, t!("Browse"), |window, cx| pick_into(window, cx, t!("Background image"), PickTarget::BackgroundImage))),
        move || {
            vec![
                sub(
                    &pp,
                    t!("Visibility"),
                    None,
                    dropdown(
                        "bg-opacity",
                        &pp,
                        (opacity * 100.).round() as u32,
                        vec![(10, "%10".into()), (20, "%20".into()), (35, "%35".into()), (50, "%50".into())],
                        |s, v| s.background_opacity = v as f32 / 100.,
                    ),
                ),
                sub(&pp, t!("Remove image"), None, button("bg-clear", &pp, t!("Remove"), |_, cx| settings::update(cx, |s| s.background_image = None))),
            ]
        },
        cx,
    ));
    // font
    let fonts = font_choices(cx);
    let current_font = s.font.clone().unwrap_or_default();
    let mut font_opts: Vec<(usize, String)> = vec![(0, t!("Default").to_string())];
    font_opts.extend(fonts.iter().enumerate().map(|(i, f)| (i + 1, f.clone())));
    let cur_ix = fonts.iter().position(|f| *f == current_font).map(|i| i + 1).unwrap_or(0);
    let fonts2 = fonts.clone();
    v.push(card(
        p,
        L::Type,
        t!("Font"),
        None,
        select_box("font", p, font_opts.iter().find(|(i, _)| *i == cur_ix).map(|x| x.1.clone()).unwrap_or_default(), move |_| {
            font_opts
                .iter()
                .map(|(i, l)| {
                    let name = if *i == 0 { None } else { fonts2.get(i - 1).cloned() };
                    Choice::new(l.clone(), *i == cur_ix, move |_, cx| {
                        let n = name.clone();
                        settings::update(cx, |s| s.font = n)
                    })
                })
                .collect()
        }),
    ));
    v.push(card(p, L::PanelTop, t!("Show toolbar"), Some("Ctrl+Shift+B"), sw("toolbar", p, s.show_toolbar, |s, v| s.show_toolbar = v)));
    v.push(card(p, L::PanelBottom, t!("Show status bar"), None, sw("statusbar", p, s.show_statusbar, |s, v| s.show_statusbar = v)));
    v.push(card(p, L::PanelRight, t!("Preview pane"), Some("Alt+P"), sw("preview", p, s.preview_open, |s, v| s.preview_open = v)));
    let hidden = s.sidebar_hidden.clone();
    let pp = *p;
    v.push(expander(
        "sidebar",
        p,
        L::PanelLeft,
        t!("Sidebar sections"),
        None,
        None,
        move || {
            let shown = |k: &str| !hidden.iter().any(|h| h == k);
            vec![
                sub(&pp, t!("Drives"), None, switch("sb-drives", &pp, shown("drives"), {
                    let on = shown("drives");
                    move |_, cx| settings::update(cx, |s| show_section(s, "drives", !on))
                })),
                sub(&pp, t!("Cloud folders"), None, switch("sb-cloud", &pp, shown("cloud"), {
                    let on = shown("cloud");
                    move |_, cx| settings::update(cx, |s| show_section(s, "cloud", !on))
                })),
                sub(&pp, t!("Network"), None, switch("sb-net", &pp, shown("network"), {
                    let on = shown("network");
                    move |_, cx| settings::update(cx, |s| show_section(s, "network", !on))
                })),
                sub(&pp, t!("Tags"), None, switch("sb-tags", &pp, shown("tags"), {
                    let on = shown("tags");
                    move |_, cx| settings::update(cx, |s| show_section(s, "tags", !on))
                })),
            ]
        },
        cx,
    ));
    v
}

fn hsla_alpha(c: u32, a: f32) -> Hsla {
    let mut h: Hsla = rgb(c).into();
    h.a = a;
    h
}

/// Installed UI-friendly fonts (curated, only those present).
fn font_choices(cx: &App) -> Vec<String> {
    let all: HashSet<String> = cx.text_system().all_font_names().into_iter().collect();
    [
        "Segoe UI Variable Text",
        "Segoe UI",
        "Inter",
        "Aptos",
        "Calibri",
        "Arial",
        "Verdana",
        "Tahoma",
        "Roboto",
        "Open Sans",
        "Noto Sans",
        "Source Sans 3",
        // common on Linux desktops
        "Cantarell",
        "Ubuntu",
        "Fira Sans",
        "DejaVu Sans",
        "Cascadia Code",
        "Consolas",
    ]
    .iter()
    .filter(|f| all.contains(**f))
    .map(|f| f.to_string())
    .collect()
}

fn layout(p: &Palette, s: &Settings, cx: &mut App) -> Vec<AnyElement> {
    use rf_core::SortKey;
    let mut v = vec![
        card(
            p,
            L::FolderSync,
            t!("Use the same layout and sorting in all folders"),
            Some(t!("When off, each folder remembers its own view and sorting.")),
            sw("sync-layout", p, s.sync_layout, |s, v| s.sync_layout = v),
        ),
        card(
            p,
            L::LayoutGrid,
            t!("Default layout"),
            Some(t!("Adaptive: folders that mostly contain images or videos open in grid view.")),
            dropdown(
                "view",
                p,
                s.default_view,
                vec![
                    (DefaultView::Details, t!("Details").into()),
                    (DefaultView::Grid, t!("Grid").into()),
                    (DefaultView::Columns, t!("Columns").into()),
                    (DefaultView::Adaptive, t!("Adaptive").into()),
                ],
                |s, v| s.default_view = v,
            ),
        ),
        section(p, t!("Sort and group")),
        card(
            p,
            L::ArrowUpDown,
            t!("Sort"),
            None,
            dropdown(
                "sort",
                p,
                s.default_sort,
                vec![
                    (SortKey::Name, t!("Name").into()),
                    (SortKey::Modified, t!("Date modified").into()),
                    (SortKey::Created, t!("Date created").into()),
                    (SortKey::Type, t!("Type").into()),
                    (SortKey::Size, t!("Size").into()),
                ],
                |s, v| s.default_sort = v,
            ),
        ),
        card(p, L::ArrowDownWideNarrow, t!("Descending order"), None, sw("sort-desc", p, s.default_sort_desc, |s, v| s.default_sort_desc = v)),
        card(p, L::FolderUp, t!("Folders first"), None, sw("dirs-first", p, s.folders_first, |s, v| s.folders_first = v)),
        card(
            p,
            L::ListTree,
            t!("Group by"),
            None,
            dropdown(
                "group",
                p,
                group_ix(&s.default_group),
                vec![
                    (0, t!("None").into()),
                    (1, t!("Name").into()),
                    (2, t!("Type").into()),
                    (3, t!("Date modified").into()),
                    (4, t!("Size").into()),
                    (5, t!("Date created").into()),
                    (6, t!("Year").into()),
                    (7, t!("Month").into()),
                    (8, t!("Day").into()),
                    (9, t!("Tag").into()),
                ],
                |s, v| s.default_group = crate::folder_view::GroupBy::ALL[v.min(9)].1.into(),
            ),
        ),
        section(p, t!("Details view")),
    ];
    let c = s.columns.clone();
    let pp = *p;
    v.push(expander(
        "columns",
        p,
        L::Columns3,
        t!("Columns"),
        Some(t!("Name column is always visible")),
        None,
        move || {
            type Set = fn(&mut Settings, bool);
            let rows: [(&str, bool, Set); 11] = [
                (t!("Date modified"), c.modified, |s, v| s.columns.modified = v),
                (t!("Date created"), c.created, |s, v| s.columns.created = v),
                (t!("Type"), c.kind, |s, v| s.columns.kind = v),
                (t!("Size"), c.size, |s, v| s.columns.size = v),
                (t!("File tag"), c.tags, |s, v| s.columns.tags = v),
                (t!("Git status"), c.git, |s, v| s.columns.git = v),
                (t!("Sync status"), c.sync, |s, v| s.columns.sync = v),
                (t!("Last commit message"), c.git_message, |s, v| s.columns.git_message = v),
                (t!("Commit author"), c.git_author, |s, v| s.columns.git_author = v),
                (t!("Commit date"), c.git_date, |s, v| s.columns.git_date = v),
                (t!("Commit hash"), c.git_hash, |s, v| s.columns.git_hash = v),
            ];
            rows.into_iter()
                .enumerate()
                // the sync status is OneDrive's (cloud placeholders): Windows only
                .filter(|(_, (label, _, _))| *label != t!("Sync status"))
                .map(|(i, (label, on, set))| sub(&pp, label, None, switch(("col-sw", i), &pp, on, move |_, cx| settings::update(cx, |s| set(s, !on)))))
                .collect()
        },
        cx,
    ));
    v
}

pub fn group_ix(g: &str) -> usize {
    crate::folder_view::GroupBy::ALL.iter().position(|(_, name)| *name == g).unwrap_or(0)
}

fn files(p: &Palette, s: &Settings, cx: &mut App) -> Vec<AnyElement> {
    let mut v = vec![section(p, t!("Appearance"))];
    let (h, dot) = (s.show_hidden, s.show_dot_files);
    let pp = *p;
    v.push(expander(
        "hidden",
        p,
        L::EyeOff,
        t!("Hidden items"),
        None,
        None,
        move || {
            vec![
                sub(&pp, t!("Show hidden items"), Some("Ctrl+H"), switch("h1", &pp, h, move |_, cx| settings::update(cx, |s| s.show_hidden = !h))),
                sub(
                    &pp,
                    t!("Show files that start with a dot"),
                    Some(t!("When off, items such as .git and .config are treated as hidden")),
                    switch("h3", &pp, dot, move |_, cx| settings::update(cx, |s| s.show_dot_files = !dot)),
                ),
            ]
        },
        cx,
    ));
    v.push(card(p, L::FileType, t!("Show file extensions"), None, sw("ext", p, s.show_extensions, |s, v| s.show_extensions = v)));
    v.push(card(p, L::Image, t!("Show thumbnails"), None, sw("thumbs", p, s.show_thumbnails, |s, v| s.show_thumbnails = v)));
    v.push(card(p, L::SquareCheck, t!("Show checkboxes when selecting"), None, sw("checkboxes", p, s.show_checkboxes, |s, v| s.show_checkboxes = v)));
    // Linux has no drive letters: people name drives differently
    v.push(card(
        p,
        L::HardDrive,
        t!("Show drives as"),
        Some(t!("Names in the sidebar and on the home page")),
        dropdown(
            "drivenames",
            p,
            s.drive_names,
            vec![
                (crate::settings::DriveNames::Auto, t!("Automatic: label, share or folder").into()),
                (crate::settings::DriveNames::MountPoint, t!("Mount point: /mnt/media").into()),
                (crate::settings::DriveNames::Folder, t!("Last folder: media").into()),
                (crate::settings::DriveNames::Device, t!("Device or share: sda1, //server/share").into()),
            ],
            |s, v| s.drive_names = v,
        ),
    ));
    v.push(card(
        p,
        L::Network,
        t!("Show network folders by their address"),
        Some(t!("smb://server/share in the address bar, instead of where it is mounted (/run/user/…)")),
        sw("addr-url", p, s.address_shows_url, |s, v| s.address_shows_url = v),
    ));

    v.push(section(p, t!("Behaviors")));
    v.push(card(
        p,
        L::MousePointerClick,
        t!("Open items with a single click"),
        None,
        dropdown(
            "single-click",
            p,
            s.single_click,
            vec![(ClickPref::Never, t!("Never").into()), (ClickPref::Folders, t!("Folders only").into()), (ClickPref::Always, t!("Always").into())],
            |s, v| s.single_click = v,
        ),
    ));
    v.push(card(p, L::SquarePlus, t!("Open folders in a new tab"), None, sw("dir-newtab", p, s.open_folders_in_new_tab, |s, v| s.open_folders_in_new_tab = v)));
    v.push(card(
        p,
        L::Trash,
        t!("Confirm before deleting"),
        None,
        dropdown(
            "confirm-del",
            p,
            s.confirm_delete,
            vec![
                (DeletePref::Always, t!("Always").into()),
                (DeletePref::PermanentOnly, t!("Only for permanent deletion").into()),
                (DeletePref::Never, t!("Never").into()),
            ],
            |s, v| s.confirm_delete = v,
        ),
    ));
    v.push(card(p, L::TriangleAlert, t!("Warn when the extension changes"), None, sw("ext-warn", p, s.warn_extension_change, |s, v| s.warn_extension_change = v)));
    v.push(card(p, L::MousePointer, t!("Select on hover"), None, sw("hover-sel", p, s.select_on_hover, |s, v| s.select_on_hover = v)));
    v.push(card(p, L::ArrowUp, t!("Double-click empty space to go up a folder"), None, sw("dbl-up", p, s.double_click_up, |s, v| s.double_click_up = v)));
    v.push(card(p, L::FolderUp, t!("Select the previous folder when going up"), None, sw("prev-sel", p, s.select_previous_on_up, |s, v| s.select_previous_on_up = v)));
    v.push(card(
        p,
        L::FileArchive,
        t!("Open archives like folders"),
        Some(t!("Double-click a ZIP, 7z or tar file to browse it; files you open inside are extracted to a temporary folder")),
        sw("browse-arc", p, s.browse_archives, |s, v| s.browse_archives = v),
    ));
    v.push(card(
        p,
        L::FolderOutput,
        t!("Open the folder after extracting"),
        Some(t!("When an archive is extracted into a new folder, that folder opens in a new tab")),
        sw("open-extracted", p, s.open_after_extract, |s, v| s.open_after_extract = v),
    ));
    // the name index covers NTFS/ReFS drives (Linux: the home folder and local data disks)
    v.push(card(
        p,
        L::Zap,
        t!("Fast search index"),
        Some(&name_index_status()),
        sw("name-index", p, s.name_index, |s, v| s.name_index = v),
    ));
    if s.name_index {
        v.push(index_exclusions(p, s));
    }
    v.push(card(
        p,
        L::HardDrive,
        t!("Size format"),
        None,
        dropdown(
            "size-fmt",
            p,
            s.decimal_sizes,
            vec![(false, t!("Binary (1 KB = 1024 bytes)").into()), (true, t!("Decimal (1 KB = 1000 bytes)").into())],
            |s, v| s.decimal_sizes = v,
        ),
    ));
    let pp = *p;
    v.push(expander(
        "folder-sizes",
        p,
        L::FolderSearch,
        t!("Calculate folder sizes"),
        Some(t!("The Size column shows the total size of folders")),
        Some(sw("calc-sizes", p, s.calculate_folder_sizes, |s, v| s.calculate_folder_sizes = v)),
        move || {
            vec![
                h_flex()
                    .mx_4()
                    .my_2()
                    .px_3()
                    .py_2()
                    .gap_2()
                    .items_center()
                    .rounded(theme::RADIUS_SM)
                    .bg(hsla_alpha(0xFCE100, 0.12))
                    .child(Icon::new(L::CircleAlert).size(px(14.)).text_color(rgb(0xFCE100)))
                    .child(div().text_xs().text_color(pp.text).child(t!("Large folders can increase disk and CPU usage; calculation runs in the background and the interface stays responsive.")))
                    .into_any_element(),
            ]
        },
        cx,
    ));

    v.push(card(
        p,
        L::HardDrive,
        t!("Show size on disk for folders"),
        Some(t!("The Size column shows folders by the space they take on disk (rounded up to whole clusters) instead of the sum of their file sizes; sorting follows it")),
        sw("folder-on-disk", p, s.folder_size_on_disk, |s, v| s.folder_size_on_disk = v),
    ));

    // Files' folder aliases: "@name" in the address box
    v.push(section(p, t!("Folder aliases")));
    let aliases: Vec<(String, std::path::PathBuf)> = s.folder_aliases.iter().map(|(n, p)| (n.clone(), p.clone())).collect();
    let count = aliases.len();
    v.push(expander(
        "aliases",
        p,
        L::AtSign,
        t!("Aliases"),
        Some(t!("Type @name in the address bar to go to a folder. To add one, right-click a folder and choose \"Add alias…\".")),
        Some(div().text_sm().text_color(p.text_secondary).child(count.to_string()).into_any_element()),
        move || {
            if aliases.is_empty() {
                return vec![sub(&pp, t!("No aliases yet"), None, div().into_any_element())];
            }
            aliases
                .into_iter()
                .enumerate()
                .map(|(i, (name, path))| {
                    let key = name.clone();
                    sub(
                        &pp,
                        &format!("@{name}"),
                        Some(&path.to_string_lossy()),
                        button(("alias-rm", i), &pp, t!("Remove"), move |_, cx| {
                            let key = key.clone();
                            settings::update(cx, move |s| {
                                s.folder_aliases.remove(&key);
                            });
                        }),
                    )
                })
                .collect()
        },
        cx,
    ));
    v
}

fn actions(p: &Palette, cx: &mut App) -> Vec<AnyElement> {
    let mut grid = div().px_4().py_3().flex().flex_wrap().gap_x_6().gap_y_0p5();
    let capturing = crate::keymap_editor::capturing(cx);
    for (i, (label, keys, action)) in crate::command_palette::shortcut_entries().into_iter().enumerate() {
        let custom = crate::keymap_editor::custom_keys(&action, cx);
        let waiting = capturing.as_deref() == Some(action.as_str());
        let shown: SharedString = if waiting {
            t!("Press keys…").into()
        } else if let Some(k) = &custom {
            crate::keymap_editor::pretty(k).into()
        } else if keys.is_empty() {
            "—".into()
        } else {
            keys.into()
        };
        let act = action.clone();
        let reset_act = action.clone();
        grid = grid.child(
            h_flex()
                .id(("kb", i))
                .w(px(380.))
                .h(px(32.))
                .px_2()
                .rounded(theme::RADIUS_SM)
                .justify_between()
                .items_center()
                .text_sm()
                .cursor_pointer()
                .hover(|s| s.bg(p.pressed))
                .when(waiting, |d| d.bg(p.selection))
                .on_click(move |_, window, cx| {
                    crate::keymap_editor::start(act.clone(), cx);
                    window.refresh();
                })
                .child(crate::i18n::t(label))
                .child(
                    h_flex()
                        .gap_1()
                        .items_center()
                        .child(
                            div()
                                .px_2()
                                .py_0p5()
                                .rounded(px(4.))
                                .border_1()
                                .border_color(p.border)
                                .text_xs()
                                .text_color(if waiting || custom.is_some() { p.accent } else { p.text_secondary })
                                .child(shown),
                        )
                        .when(custom.is_some() && !waiting, |d| {
                            d.child(
                                div()
                                    .id(("kb-reset", i))
                                    .px_1()
                                    .rounded(theme::RADIUS_SM)
                                    .text_xs()
                                    .text_color(p.text_secondary)
                                    .hover(|s| s.bg(p.pressed))
                                    .child("↺")
                                    .on_click(move |_, window, cx| {
                                        cx.stop_propagation();
                                        crate::keymap_editor::reset(&reset_act, cx);
                                        window.refresh();
                                    }),
                            )
                        }),
                ),
        );
    }
    vec![
        shell(p)
            .child(card_body(
                p,
                L::Keyboard,
                t!("Keyboard shortcuts").into(),
                Some(t!("To change a shortcut, click a command and press the new keys (Esc: cancel). ↺ restores the default.").into()),
                None,
            ))
            .child(div().h(px(1.)).bg(p.divider))
            .child(grid)
            .into_any_element(),
    ]
}

fn tags(p: &Palette, cx: &mut App) -> Vec<AnyElement> {
    let defs = cx.global::<crate::tags::Tags>().0.defs.clone();
    let mut list = v_flex().py_1();
    for d in defs {
        let (id, name, color) = (d.id, d.name.clone(), d.color);
        let rename_name = name.clone();
        list = list.child(
            h_flex()
                .min_h(px(48.))
                .pl(px(56.))
                .pr_4()
                .gap_3()
                .items_center()
                .child(
                    div()
                        .id(("tag-color", id as usize))
                        .size(px(18.))
                        .rounded_full()
                        .bg(rgb(color))
                        .cursor_pointer()
                        .on_click(move |ev: &ClickEvent, window, cx| {
                            let items = TAG_COLORS
                                .iter()
                                .map(|(n, c)| {
                                    let c = *c;
                                    Item::action(crate::i18n::t(n), MenuIcon::Dot(rgb(c).into()), move |_, cx| crate::tags::update(cx, |t| t.recolor(id, c)))
                                        .checked(c == color)
                                })
                                .collect();
                            fluent_menu::show(Menu { quick: Vec::new(), items }, ev.position(), window, cx);
                        }),
                )
                .child(div().flex_1().text_sm().child(crate::i18n::td(&name)))
                .child(button(("tag-edit", id as usize), p, t!("Rename"), move |window, cx| {
                    tag_name_dialog(Some((id, rename_name.clone())), window, cx)
                }))
                .child(button(("tag-del", id as usize), p, t!("Delete"), move |window, cx| {
                    crate::prompt::show(
                        t!("Delete tag?"),
                        Some(t!("The tag is removed from all items. Files are not deleted.").to_string()),
                        None,
                        vec![
                            crate::prompt::Button::new(t!("Delete"), crate::prompt::Kind::Danger, move |_, _, cx| {
                                crate::tags::update(cx, |t| t.remove(id));
                                Ok(())
                            }),
                            crate::prompt::Button::cancel(),
                        ],
                        window,
                        cx,
                    )
                })),
        );
    }
    vec![
        shell(p)
            .child(card_body(
                p,
                L::Tag,
                t!("Tags").into(),
                Some(t!("Group files independently of folders using colored tags. Click a color to change it.").into()),
                Some(button("tag-new", p, t!("New tag"), |window, cx| tag_name_dialog(None, window, cx))),
            ))
            .child(div().h(px(1.)).bg(p.divider))
            .child(list)
            .into_any_element(),
    ]
}

const TAG_COLORS: [(&str, u32); 8] = [
    ("Red", 0xFF453A),
    ("Orange", 0xFF9F0A),
    ("Yellow", 0xFFD60A),
    ("Green", 0x30D158),
    ("Turquoise", 0x40C8E0),
    ("Blue", 0x0A84FF),
    ("Purple", 0xBF5AF2),
    ("Gray", 0x8E8E93),
];

fn tag_name_dialog(existing: Option<(u32, String)>, window: &mut Window, cx: &mut App) {
    let title = if existing.is_some() { t!("Rename tag") } else { t!("New tag") };
    let initial = existing.as_ref().map(|e| e.1.clone()).unwrap_or_default();
    crate::prompt::show(
        title,
        None,
        Some((initial, t!("Tag name").to_string())),
        vec![
            crate::prompt::Button::new(t!("Save"), crate::prompt::Kind::Primary, move |value, _, cx| {
                let name = value.unwrap_or_default().trim().to_string();
                if name.is_empty() {
                    return Err(t!("Enter a name").to_string());
                }
                match existing {
                    Some((id, _)) => crate::tags::update(cx, |t| t.rename(id, &name)),
                    None => crate::tags::update(cx, |t| {
                        t.add(&name, TAG_COLORS[t.defs.len() % TAG_COLORS.len()].1);
                    }),
                }
                Ok(())
            }),
            crate::prompt::Button::cancel(),
        ],
        window,
        cx,
    );
}

fn dev(p: &Palette, s: &Settings) -> Vec<AnyElement> {
    let ide_label = if s.ide_path.is_empty() { t!("VS Code (automatic)").to_string() } else { s.ide_path.clone() };
    vec![
        card(
            p,
            L::Code,
            t!("\"Open in editor\" button in the status bar"),
            None,
            dropdown(
                "ide-btn",
                p,
                s.ide_button,
                vec![
                    (IdePref::GitRepos, t!("Show in Git repositories").into()),
                    (IdePref::Always, t!("Always show").into()),
                    (IdePref::Never, t!("Don't show").into()),
                ],
                |s, v| s.ide_button = v,
            ),
        ),
        card(
            p,
            L::SquareTerminal,
            t!("Editor"),
            Some(&ide_label),
            h_flex()
                .gap_2()
                .child(button("ide-browse", p, t!("Browse"), |window, cx| pick_into(window, cx, t!("Choose editor"), PickTarget::Ide)))
                .child(button("ide-reset", p, t!("Automatic"), |_, cx| settings::update(cx, |s| s.ide_path.clear())))
                .into_any_element(),
        ),
    ]
}

/// DirectLook ayarı anahtarı (ayrı dosya `directlook.toml`; değişince DirectLook'a haber verilir).
fn lsw(id: &'static str, p: &Palette, on: bool, set: fn(&mut LookSettings, bool)) -> AnyElement {
    switch(id, p, on, move |_, cx| look_update(cx, |s| set(s, !on)))
}

fn look_update(cx: &mut App, f: impl FnOnce(&mut LookSettings)) {
    crate::look::config::update(cx, f);
    crate::look::ipc::send(crate::look::ipc::Msg::Reload, false);
}

fn ldropdown<T: Copy + PartialEq + 'static>(id: &'static str, p: &Palette, current: T, options: Vec<(T, String)>, set: fn(&mut LookSettings, T)) -> AnyElement {
    let label = options.iter().find(|(v, _)| *v == current).map(|(_, l)| l.clone()).unwrap_or_default();
    select_box(id, p, label, move |_| {
        options.iter().map(|(v, l)| {
            let v = *v;
            Choice::new(l.clone(), v == current, move |_, cx| look_update(cx, |s| set(s, v)))
        }).collect()
    })
}

/// DirectLook (Hızlı Bakış, ayrı program) ayarları.
fn look(p: &Palette, s: &Settings, cx: &mut App) -> Vec<AnyElement> {
    use crate::settings::{LookBackdrop, LookImageTheme};
    // DirectLook da yazabilir: sayfa her açılışta dosyadan okunur
    let l = LookSettings::load();
    let enabled = s.directlook_enabled;
    let mut v = vec![card(
        p,
        L::Eye,
        t!("Use DirectLook"),
        Some(t!("Preview with Space. Runs as a separate program: it doesn't slow rfiles down, and if one closes the other is unaffected.")),
        switch("dl-enabled", p, enabled, move |_, cx| {
            settings::update(cx, |s| s.directlook_enabled = !enabled);
            if enabled {
                crate::look::ipc::send(crate::look::ipc::Msg::Quit, false);
            } else {
                crate::look::ipc::send(crate::look::ipc::Msg::Wake, true);
            }
        }),
    )];
    if !enabled {
        return v;
    }
    let _ = cx;
    v.extend([
        section(p, t!("Window")),
        card(
            p,
            L::Layers,
            t!("Background"),
            None,
            ldropdown(
                "look-backdrop",
                p,
                l.look_backdrop,
                vec![
                    (LookBackdrop::Mica, t!("Translucent").into()),
                    (LookBackdrop::Acrylic, t!("Translucent with blur").into()),
                    (LookBackdrop::None, t!("Solid color").into()),
                ],
                |s, v| s.look_backdrop = v,
            ),
        ),
        card(p, L::PictureInPicture, t!("Always on top"), None, lsw("look-top", p, l.look_topmost, |s, v| s.look_topmost = v)),
        card(
            p,
            L::MousePointerClick,
            t!("Focus the window when it opens"),
            Some(t!("When off, the keyboard stays in rfiles: arrow keys move between files")),
            lsw("look-focus", p, l.look_focus_on_open, |s, v| s.look_focus_on_open = v),
        ),
        card(p, L::CircleX, t!("Close when another window is activated"), Some(t!("Pinned windows stay open")), lsw("look-lostfocus", p, l.look_close_on_lost_focus, |s, v| s.look_close_on_lost_focus = v)),
        card(p, L::Zap, t!("Transition animation"), None, lsw("look-transition", p, l.look_transition, |s, v| s.look_transition = v)),
        card(p, L::RefreshCw, t!("Reload button"), None, lsw("look-reload", p, l.look_show_reload, |s, v| s.look_show_reload = v)),
        card(p, L::FolderSync, t!("Reload when the file changes"), None, lsw("look-autoreload", p, l.look_auto_reload, |s, v| s.look_auto_reload = v)),
        section(p, t!("Contains")),
        card(
            p,
            L::Image,
            t!("Image background"),
            None,
            ldropdown(
                "look-imgtheme",
                p,
                l.look_image_theme,
                vec![(LookImageTheme::Dark, t!("Dark").into()), (LookImageTheme::Light, t!("On").into())],
                |s, v| s.look_image_theme = v,
            ),
        ),
        card(p, L::RotateCw, t!("Loop videos"), None, lsw("look-loop", p, l.look_loop, |s, v| s.look_loop = v)),
        card(p, L::List, t!("Line numbers in text"), Some(t!("Alt+L in the preview")), lsw("look-lines", p, l.look_text_line_numbers, |s, v| s.look_text_line_numbers = v)),
        card(p, L::TextAlignJustify, t!("Word wrap in text"), Some(t!("Alt+Z in the preview")), lsw("look-wrap", p, l.look_text_wrap, |s, v| s.look_text_wrap = v)),
    ]);
    // text font and size
    let font_label = if l.look_text_font.is_empty() { t!("Default").to_string() } else { l.look_text_font.clone() };
    let cur_font = l.look_text_font.clone();
    let fonts = font_choices(cx);
    v.push(card(
        p,
        L::Type,
        t!("Text font"),
        None,
        select_box("look-font", p, font_label, move |_| {
            std::iter::once(String::new())
                .chain(fonts.iter().cloned())
                .map(|f| {
                    let label = if f.is_empty() { t!("Default").to_string() } else { f.clone() };
                    let sel = f == cur_font;
                    Choice::new(label, sel, move |_, cx| {
                        let f = f.clone();
                        look_update(cx, |s| s.look_text_font = f)
                    })
                })
                .collect()
        }),
    ));
    let cur_size = l.look_text_font_size.round() as u32;
    v.push(card(
        p,
        L::ALargeSmall,
        t!("Text size"),
        None,
        select_box("look-font-size", p, format!("{cur_size} px"), move |_| {
            [11u32, 12, 13, 14, 15, 16, 18, 20, 24]
                .into_iter()
                .map(|n| Choice::new(format!("{n} px"), n == cur_size, move |_, cx| look_update(cx, |s| s.look_text_font_size = n as f32)))
                .collect()
        }),
    ));
    // which file types DirectLook previews
    v.push(card(
        p,
        L::ListFilter,
        t!("Files to preview"),
        None,
        ldropdown(
            "look-ext-mode",
            p,
            l.look_ext_allow_mode,
            vec![(false, t!("All except those listed").to_string()), (true, t!("Only those listed").to_string())],
            |s, v| s.look_ext_allow_mode = v,
        ),
    ));
    let allow = l.look_ext_allow_mode;
    let list = if allow { l.look_ext_allowlist.clone() } else { l.look_ext_blocklist.clone() };
    let shown = if list.trim().is_empty() { "—".to_string() } else { list.clone() };
    v.push(card(
        p,
        L::CircleSlash,
        if allow { t!("Extensions to preview") } else { t!("Extensions not to preview") },
        Some(&shown),
        button("look-ext-edit", p, t!("Edit…"), move |window, cx| {
            let ok = crate::prompt::Button::new(t!("Save"), crate::prompt::Kind::Primary, move |text, _, cx| {
                let text = normalize_exts(&text.unwrap_or_default());
                look_update(cx, |s| if allow { s.look_ext_allowlist = text } else { s.look_ext_blocklist = text });
                Ok(())
            });
            crate::prompt::show(
                if allow { t!("Extensions to preview") } else { t!("Extensions not to preview") },
                Some(t!("Separate with semicolons, for example: .iso; .insv; .psd").to_string()),
                Some((list.clone(), ".iso; .insv".to_string())),
                vec![ok, crate::prompt::Button::cancel()],
                window,
                cx,
            );
        }),
    ));
    v
}

/// "iso, .PSD ;; insv" → ".iso;.psd;.insv": dots added, lowercase, no duplicates.
fn normalize_exts(text: &str) -> String {
    let mut out: Vec<String> = Vec::new();
    for part in text.split([';', ',', ' ']) {
        let p = part.trim().trim_start_matches('*').to_lowercase();
        if p.is_empty() || p == "." {
            continue;
        }
        let e = if p.starts_with('.') { p } else { format!(".{p}") };
        if !out.contains(&e) {
            out.push(e);
        }
    }
    out.join(";")
}

#[cfg(test)]
mod ext_tests {
    #[test]
    fn extension_lists_are_normalized() {
        assert_eq!(super::normalize_exts("iso, .PSD ;; insv *.Mkv .iso"), ".iso;.psd;.insv;.mkv");
        assert_eq!(super::normalize_exts("  "), "");
    }
}

fn advanced(p: &Palette, s: &Settings) -> Vec<AnyElement> {
    let start_on = rf_linux::dialogs::starts_with_windows();
    let dfm_on = crate::DEFAULT_FM.load(std::sync::atomic::Ordering::Relaxed);
    let mut out = vec![
        link_row(p, "export", L::Upload, t!("Export settings"), None, |window, cx| pick_into(window, cx, t!("Export settings"), PickTarget::Export)),
        link_row(p, "import", L::Download, t!("Import settings"), None, |window, cx| pick_into(window, cx, t!("Import settings"), PickTarget::Import)),
        link_row(p, "edit-file", L::FilePen, t!("Edit settings file"), Some(t!("settings.toml opens in a text editor")), |_, _| {
            if let Some(path) = settings::path() {
                std::thread::spawn(move || {
                    let _ = rf_linux::shell::open(&path);
                });
            }
        }),
        card(
            p,
            L::Power,
            t!("Open rfiles when you log in"),
            None,
            switch("start-win", p, start_on, move |window, _| {
                if let Ok(exe) = std::env::current_exe() {
                    if let Err(e) = rf_linux::dialogs::set_start_with_windows(&exe, !start_on) {
                        log::warn!("start with Windows: {e}");
                    }
                }
                window.refresh();
            }),
        ),
        card(
            p,
            L::PanelBottom,
            t!("Keep running in the background when the window is closed"),
            Some(t!("An icon stays in the system tray; new windows open instantly")),
            sw("background", p, s.run_in_background, |s, v| s.run_in_background = v),
        ),
        card(
            p,
            L::Bell,
            t!("Notify when long operations finish"),
            Some(t!("When copying, moving or compressing took over 10 seconds and rfiles isn't in front")),
            sw("notify-ops", p, s.notify_long_ops, |s, v| s.notify_long_ops = v),
        ),
        card(
            p,
            L::SquareTerminal,
            t!("Terminal"),
            Some(t!("For \"Open in terminal\" and \"Run command here\". In the address bar, > and a command runs it in this folder")),
            terminal_choice(p, &s.terminal),
        ),
        section(p, t!("Performance")),
        card(
            p,
            L::Zap,
            t!("Fast file operations"),
            Some(t!("Copying, moving to another drive and permanent deletion use our parallel engine. When off, files are copied one at a time.")),
            sw("fast-ops", p, s.fast_file_ops, |s, v| s.fast_file_ops = v),
        ),
        card(
            p,
            L::ShieldCheck,
            t!("Verify copies"),
            Some(t!("Each file is compared with its source after copying. Full verification reads the copy back from the disk and takes longer.")),
            dropdown(
                "copy-verify",
                p,
                s.copy_verify,
                vec![
                    (crate::settings::CopyVerify::Off, t!("Off").into()),
                    (crate::settings::CopyVerify::Quick, t!("Quick: size and date").into()),
                    (crate::settings::CopyVerify::Full, t!("Full: content").into()),
                ],
                |s, v| s.copy_verify = v,
            ),
        ),
        card(
            p,
            L::Gauge,
            t!("Copy speed limit"),
            Some(t!("So copies to a network share or a USB stick don't choke the connection.")),
            dropdown(
                "copy-limit",
                p,
                s.copy_speed_limit,
                std::iter::once((0, t!("Unlimited").to_string())).chain([5u32, 10, 25, 50, 100, 250].into_iter().map(|m| (m, format!("{m} MB/s")))).collect(),
                |s, v| s.copy_speed_limit = v,
            ),
        ),
        card(
            p,
            L::Activity,
            t!("Performance info in the status bar"),
            Some("How long the current folder took to list"),
            sw("perf-info", p, s.show_perf_info, |s, v| s.show_perf_info = v),
        ),
        section(p, t!("Experimental")),
        card(
            p,
            L::AppWindow,
            t!("Set as default file manager"),
            Some(t!("Folders opened from other apps, and \"Show in folder\" in browsers, open in rfiles. Applies to this user only; turning it off brings back the previous file manager.")),
            switch("dfm", p, dfm_on, move |window, _| {
                let exe = std::env::current_exe().map(|e| e.display().to_string()).unwrap_or_default();
                std::thread::spawn(move || {
                    let classes = rf_linux::default_fm::CLASSES;
                    let r = if dfm_on { rf_linux::default_fm::disable(classes) } else { rf_linux::default_fm::enable(&exe, classes) };
                    if let Err(e) = r {
                        log::warn!("default file manager: {e:#}");
                    }
                    crate::DEFAULT_FM.store(rf_linux::default_fm::is_enabled(classes), std::sync::atomic::Ordering::Relaxed);
                });
                window.refresh();
            }),
        ),
        file_chooser_card(p),
    ];
    out.extend([
        card(
            p,
            L::FolderTree,
            t!("Show the flatten option"),
            Some(t!("Moves the files from all subfolders into a single level. Ctrl+Z undoes it.")),
            sw("flatten", p, s.show_flatten, |s, v| s.show_flatten = v),
        ),
    ]);
    out
}

/// The terminal picker: Automatic (naming the one it picks) and the installed terminals.
fn terminal_choice(p: &Palette, current: &str) -> AnyElement {
    let found = crate::terminal::choices();
    let auto = match found.and_then(|f| f.0.as_deref()) {
        Some(t) => format!("{} ({t})", t!("Automatic")),
        None => t!("Automatic").to_string(),
    };
    let mut options: Vec<(String, String)> = vec![(String::new(), auto)];
    options.extend(found.map(|f| f.1.clone()).unwrap_or_default().into_iter().map(|t| (t.program, t.name)));
    if !current.is_empty() && !options.iter().any(|(v, _)| v == current) {
        // set by hand in settings.toml
        options.push((current.to_string(), current.to_string()));
    }
    let label = options.iter().find(|(v, _)| v == current).map(|(_, l)| l.clone()).unwrap_or_default();
    let current = current.to_string();
    select_box("terminal", p, label, move |_| {
        options
            .iter()
            .map(|(v, l)| {
                let v = v.clone();
                Choice::new(l.clone(), v == current, move |_, cx| settings::update(cx, |s| s.terminal = v.clone()))
            })
            .collect()
    })
}

/// "Use rfiles for Open and Save dialogs": the portal's FileChooser (see rf-linux file_chooser).
fn file_chooser_card(p: &Palette) -> AnyElement {
    static STATE: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);
    // 0 = not read yet, 1 = off, 2 = on, 3 = not installed (read on a worker thread)
    let state = STATE.load(std::sync::atomic::Ordering::Relaxed);
    let refresh = || {
        std::thread::spawn(|| {
            let s = if !rf_linux::file_chooser::installed() { 3 } else if rf_linux::file_chooser::enabled() { 2 } else { 1 };
            STATE.store(s, std::sync::atomic::Ordering::Relaxed);
        });
    };
    if state == 0 {
        refresh();
    }
    let on = state == 2;
    let note = if state == 3 {
        t!("Needs the deb, rpm or Arch package (it installs the desktop portal's rfiles.portal); the Flatpak can't add a portal backend.")
    } else {
        t!("Apps that ask the desktop portal for a file dialog (Firefox, Chromium, GTK, Qt and Electron apps) get rfiles. Turning it off gives the dialog back.")
    };
    card(
        p,
        L::FolderOpen,
        t!("Use rfiles for Open and Save dialogs"),
        Some(note),
        if state == 3 {
            div().into_any_element()
        } else {
            switch("file-chooser", p, on, move |window, _| {
                STATE.store(if on { 1 } else { 2 }, std::sync::atomic::Ordering::Relaxed);
                std::thread::spawn(move || {
                    if let Err(e) = rf_linux::file_chooser::set_enabled(!on) {
                        log::warn!("file chooser: {e:#}");
                    }
                    let s = if rf_linux::file_chooser::enabled() { 2 } else { 1 };
                    STATE.store(s, std::sync::atomic::Ordering::Relaxed);
                });
                window.refresh();
            })
        },
    )
}

fn open_url(url: &str) {
    let url = url.to_string();
    std::thread::spawn(move || {
        let _ = rf_linux::shell::open(std::path::Path::new(&url));
    });
}

const REPO_URL: &str = "https://github.com/benjaminfenech/rfiles";
const ISSUES_URL: &str = "https://github.com/benjaminfenech/rfiles/issues/new";
const RELEASES_URL: &str = "https://github.com/benjaminfenech/rfiles/releases";
/// The project rfiles grew out of.
const UPSTREAM_URL: &str = "https://github.com/gelenmedya/rustyfiles";

/// Updates from GitHub Releases: where things stand and the button that fits.
fn update_card(p: &Palette) -> AnyElement {
    use crate::about::UpdateState as S;
    let check = |p: &Palette| {
        button("upd-check", p, "Check now", |window, cx| {
            crate::about::run(cx, || crate::about::check(true));
            window.refresh();
        })
    };
    let (icon, text, control): (L, String, AnyElement) = match crate::about::update_state() {
        S::Idle => (L::RefreshCw, "Updates come from rfiles' releases on GitHub.".into(), check(p)),
        S::Checking => (L::RefreshCw, "Checking…".into(), div().into_any_element()),
        S::UpToDate => (L::CircleCheck, format!("rfiles {} is the latest version.", crate::about::version()), check(p)),
        S::Available(r) => {
            let url = r.url.clone();
            let notes = button("upd-notes", p, "Release notes", move |_, _| open_url(&url));
            let action = if crate::about::installable() {
                button("upd-install", p, "Install", |window, cx| {
                    crate::about::run(cx, crate::about::install_available);
                    window.refresh();
                })
            } else {
                // built from source: the release page has the packages
                let url = r.url.clone();
                button("upd-page", p, "Download", move |_, _| open_url(&url))
            };
            let how = if crate::about::installable() { "Your desktop may ask for your password to install it." } else { "This copy wasn't installed from a package: download it from the release page." };
            (L::Download, format!("rfiles {} is available. {how}", r.version), h_flex().gap_2().child(notes).child(action).into_any_element())
        }
        S::Downloading(v, pct) => (L::Download, format!("Downloading rfiles {v}… {pct}%"), div().into_any_element()),
        S::Installing(v) => (L::Download, format!("Installing rfiles {v}…"), div().into_any_element()),
        S::Installed(v) => (
            L::CircleCheck,
            format!("rfiles {v} is installed. Restart to use it."),
            button("upd-restart", p, "Restart rfiles", |_, cx| crate::about::restart(cx)),
        ),
        S::Failed(e) => (L::TriangleAlert, format!("Update failed: {e}"), check(p)),
    };
    card(p, icon, "Updates", Some(&text), control)
}

fn about(p: &Palette, cx: &mut App) -> Vec<AnyElement> {
    let ver = crate::about::version_full();
    let version = format!("{} {ver}", t!("Version:"));
    let pp = *p;
    vec![
        card(
            p,
            L::FolderOpen,
            "rfiles",
            Some(&version),
            button("copy-ver", p, t!("Copy"), move |_, cx| cx.write_to_clipboard(ClipboardItem::new_string(format!("rfiles {ver}")))),
        ),
        update_card(p),
        card(
            p,
            L::RefreshCw,
            "Check for updates automatically",
            Some("Soon after rfiles starts, then once a day. Nothing is installed without asking."),
            sw("auto-upd", p, cx.global::<Settings>().auto_update, |s, v| s.auto_update = v),
        ),
        section(p, "GitHub"),
        link_row(p, "gh-star", L::Star, t!("Star it on GitHub"), Some(t!("If rfiles is useful to you, it helps others find it.")), |_, _| open_url(REPO_URL)),
        link_row(p, "gh-issue", L::MessageSquare, t!("Report a bug or suggest something"), None, |_, _| open_url(ISSUES_URL)),
        link_row(p, "gh-notes", L::ScrollText, t!("Release notes"), None, |_, _| open_url(RELEASES_URL)),
        // rfiles began as the Linux port of rustyfiles
        link_row(p, "upstream", L::GitFork, "Based on rustyfiles", Some("by Özer Güzel: github.com/gelenmedya/rustyfiles"), |_, _| open_url(UPSTREAM_URL)), // i18n-ok: a name
        div()
            .px_1()
            .text_xs()
            .text_color(p.text_secondary)
            .child(t!("A lightweight file manager written in Rust + GPUI that never freezes."))
            .into_any_element(),
        section(p, t!("Help and support")),
        link_row(p, "open-log", L::FolderOpen, t!("Open log and settings folder"), settings::app_dir().map(|d| d.display().to_string()).as_deref(), |_, _| {
            if let Some(d) = settings::app_dir() {
                std::thread::spawn(move || {
                    let _ = rf_linux::shell::open(&d);
                });
            }
        }),
        section(p, t!("Open source")),
        expander(
            "libs",
            p,
            L::Library,
            t!("Third-party libraries"),
            None,
            None,
            move || {
                [
                    ("GPUI", "Apache-2.0"),
                    ("gpui-kit", "Apache-2.0"),
                    ("Lucide icons", "ISC"),
                    ("zip, sevenz-rust2, tar, flate2, zstd", "MIT / Apache-2.0"),
                    ("notify", "CC0 / Artistic-2.0"),
                    ("chrono, serde, toml", "MIT / Apache-2.0"),
                ]
                .iter()
                .map(|(n, l)| sub(&pp, n, Some(l), div().into_any_element()))
                .collect()
            },
            cx,
        ),
    ]
}

/// Shows or hides a sidebar section (`Settings::sidebar_hidden`).
fn show_section(s: &mut Settings, key: &str, show: bool) {
    s.sidebar_hidden.retain(|h| h != key);
    if !show {
        s.sidebar_hidden.push(key.to_string());
    }
}

enum PickTarget {
    BackgroundImage,
    Ide,
    Export,
    Import,
}

/// Runs the desktop's file dialog (through the portal) on the shell thread and applies the
/// result.
fn pick_into(_window: &mut Window, cx: &mut App, title: &str, target: PickTarget) {
    let title = title.to_string();
    let (tx, rx) = futures::channel::oneshot::channel::<Option<std::path::PathBuf>>();
    let images = [(t!("Pictures"), "*.png;*.jpg;*.jpeg;*.webp;*.bmp;*.gif")];
    let toml = [(t!("Settings file"), "*.toml")];
    let (images, toml) = (
        images.map(|(a, b)| (a.to_string(), b.to_string())),
        toml.map(|(a, b)| (a.to_string(), b.to_string())),
    );
    let kind = match target {
        PickTarget::BackgroundImage => 0,
        PickTarget::Ide => 1,
        PickTarget::Export => 2,
        PickTarget::Import => 3,
    };
    rf_linux::sta::run(move |owner| {
        use rf_linux::dialogs::{Pick, pick};
        fn f(v: &[(String, String)]) -> Vec<(&str, &str)> {
            v.iter().map(|(a, b)| (a.as_str(), b.as_str())).collect()
        }
        let r = match kind {
            0 => pick(owner, &title, Pick::File { filters: &f(&images) }),
            // Linux programs have no extension to filter on
            1 => pick(owner, &title, Pick::File { filters: &Vec::new() }),
            2 => pick(owner, &title, Pick::Save { filters: &f(&toml), default_name: "rfiles-ayarlar.toml" }),
            _ => pick(owner, &title, Pick::File { filters: &f(&toml) }),
        };
        let _ = tx.send(r);
    });
    cx.spawn(async move |cx| {
        let Ok(Some(path)) = rx.await else { return };
        let _ = cx.update(|cx| match kind {
            0 => settings::update(cx, |s| s.background_image = Some(path)),
            1 => settings::update(cx, |s| s.ide_path = path.display().to_string()),
            2 => {
                let s = cx.global::<Settings>().clone();
                std::thread::spawn(move || {
                    if let Ok(text) = toml::to_string_pretty(&s) {
                        let _ = std::fs::write(&path, text);
                    }
                });
            }
            _ => {
                if let Ok(text) = std::fs::read_to_string(&path) {
                    if let Ok(new) = Settings::parse_lenient_pub(text.trim_start_matches('\u{feff}')) {
                        // keep this machine's window placement and session
                        settings::update(cx, |s| {
                            let (window, tabs, active) = (s.window, s.session_tabs.clone(), s.session_active);
                            *s = new;
                            s.window = window;
                            s.session_tabs = tabs;
                            s.session_active = active;
                        });
                    }
                }
            }
        });
    })
    .detach();
}

/// "Local drives' file names are kept in an index … · C:\ 2.1 million entries, 84 MB · E:\ indexing…"
fn name_index_status() -> String {
    let mut s = t!("File names on the local drives are kept in an index, so searching subfolders takes milliseconds. The index is built in the background and kept up to date.").to_string();
    let parts: Vec<String> = rf_linux::indexer::status()
        .into_iter()
        .map(|(root, state, live, mem)| {
            let r = root.to_string_lossy().trim_end_matches('\\').to_string();
            match state {
                rf_linux::indexer::State::Building => format!("{r} {}", t!("indexing…")),
                rf_linux::indexer::State::Failed => format!("{r} {}", t!("couldn't be indexed")),
                rf_linux::indexer::State::Ready => {
                    t!("{root} {n} items, {mb} MB", root = r, n = crate::disk_usage::count(live as u64), mb = mem >> 20)
                }
            }
        })
        .collect();
    if !parts.is_empty() {
        s.push_str(" · ");
        s.push_str(&parts.join(" · "));
    }
    s
}

/// The folders left out of the search index: the user's (removable) and the built-in ones.
fn index_exclusions(p: &Palette, s: &Settings) -> AnyElement {
    let mut list = v_flex().py_1();
    for (i, dir) in s.index_excluded.iter().enumerate() {
        let d = dir.clone();
        list = list.child(
            h_flex()
                .min_h(px(40.))
                .pl(px(56.))
                .pr_4()
                .gap_3()
                .items_center()
                .child(Icon::new(L::FolderX).size(px(14.)).text_color(p.text_secondary))
                .child(div().flex_1().min_w_0().text_sm().text_ellipsis().overflow_hidden().whitespace_nowrap().child(dir.clone()))
                .child(
                    div()
                        .id(("idx-excl-rm", i))
                        .p_1()
                        .rounded(theme::RADIUS_SM)
                        .hover(|st| st.bg(p.pressed))
                        .child(Icon::new(L::X).size(px(14.)).text_color(p.text_secondary))
                        .on_click(move |_, _, cx| {
                            let d = d.clone();
                            settings::update(cx, |s| s.index_excluded.retain(|e| *e != d));
                        }),
                ),
        );
    }
    let defaults = rf_linux::nameindex::default_excluded_names().join(", ");
    shell(p)
        .child(
            h_flex()
                .min_h(px(48.))
                .pl(px(56.))
                .pr_4()
                .py_2()
                .gap_4()
                .items_center()
                .child(
                    v_flex()
                        .flex_1()
                        .min_w_0()
                        .child(div().text_sm().child(t!("Folders left out of the index")))
                        .child(div().text_xs().text_color(p.text_secondary).child(format!(
                            "{} {defaults}{}. {}",
                            t!("Always:"),
                            "",
                            t!("Searching inside these folders walks the disk. You can also right-click a folder and choose \"Leave out of the search index\".")
                        ))),
                )
                .child(button("idx-excl-add", p, t!("Add folder…"), |_window, cx| {
                    // Linux: the desktop's folder dialog through the portal, on the shell thread
                    {
                        let (tx, rx) = futures::channel::oneshot::channel::<Option<std::path::PathBuf>>();
                        let title = t!("Folder to leave out of the index").to_string();
                        rf_linux::sta::run(move |owner| {
                            let _ = tx.send(rf_linux::dialogs::pick(owner, &title, rf_linux::dialogs::Pick::Folder));
                        });
                        cx.spawn(async move |cx| {
                            let Ok(Some(dir)) = rx.await else { return };
                            let d = dir.to_string_lossy().trim_end_matches('/').to_string();
                            let _ = cx.update(|cx| {
                                settings::update(cx, |s| {
                                    if !d.is_empty() && !s.index_excluded.contains(&d) {
                                        s.index_excluded.push(d);
                                    }
                                })
                            });
                        })
                        .detach();
                    }
                })),
        )
        .when(!s.index_excluded.is_empty(), |d| d.child(div().h(px(1.)).bg(p.divider)).child(list))
        .into_any_element()
}
