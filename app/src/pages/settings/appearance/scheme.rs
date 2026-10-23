//! Settings → Appearance → Theme (dark, light or the desktop's style) and
//! the solid panels' colour of each theme, with their console rows and
//! command palette entries.

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::appearance::{is_light_colour, own_panel, Appearance, InterfaceMode, Rgb, SolidTint, Theme};
use heronite_i18n::{tr, trc};

use super::look::set_look;
use super::{apply_interface, SettingsPage, State};
use crate::icons::Icon;
use crate::theme::{self, Scheme};
use crate::ui::colour_picker::{ColourPicker, PickerEvent};
use crate::ui::controls::row;
use crate::ui::fields::meta_text;

pub(super) fn theme_items() -> [(Theme, &'static str); 4] {
    [
        (Theme::Dark, trc!("theme", "Dark")),
        (Theme::Light, trc!("theme", "Light")),
        (Theme::System, trc!("theme", "System")),
        (Theme::Custom, trc!("theme", "Custom")),
    ]
}

/// A small window in the colours of `scheme`, with the current accent and
/// solid colour.
fn window_picture(scheme: Scheme) -> Div {
    theme::drawn_as(scheme, || {
        let bar = |top: f32, w: f32, a: f32| {
            div().absolute().left(relative(0.34)).top(px(top)).w(relative(w)).h(px(6.)).rounded(px(3.)).bg(theme::ink_a(a))
        };
        div()
            .relative()
            .size_full()
            .bg(theme::background())
            .child(div().absolute().left_0().top_0().bottom_0().w(relative(0.26)).bg(theme::recess_a(0.3)))
            .child(div().absolute().left(relative(0.06)).top(px(12.)).w(relative(0.14)).h(px(6.)).rounded(px(3.)).bg(theme::accent()))
            .child(bar(12., 0.44, 0.32))
            .child(bar(28., 0.56, 0.16))
            .child(bar(40., 0.38, 0.16))
    })
}

/// The dark window's left half next to the light window's right half.
fn system_picture() -> Div {
    let half = |light: bool| {
        let pic = window_picture(if light { Scheme::Light } else { Scheme::Dark }).absolute().top_0().w(relative(2.));
        div().relative().w(relative(0.5)).h_full().overflow_hidden().child(if light { pic.left(relative(-1.)) } else { pic.left_0() })
    };
    div().flex().size_full().child(half(false)).child(half(true))
}

/// The Theme row: a card per choice, System saying what the desktop uses now.
pub(super) fn theme_row(d: &SettingsPage, cx: &mut Context<SettingsPage>) -> AnyElement {
    let current = d.appearance.ui.theme;
    let mut cards = div().flex().flex_wrap().gap(px(10.));
    for (t, label) in theme_items() {
        let picture = match t {
            Theme::Dark => window_picture(Scheme::Dark),
            Theme::Light => window_picture(Scheme::Light),
            Theme::System => system_picture(),
            Theme::Custom => window_picture(Scheme::Custom),
        };
        let note = match t {
            Theme::System => Some(if theme::system_is_light() { tr!("Light now") } else { tr!("Dark now") }),
            Theme::Custom => Some(tr!("Your colours")),
            _ => None,
        };
        let on = t == current;
        cards = cards.child(
            div()
                .id(SharedString::from(format!("theme-{}", t.as_str())))
                .flex()
                .flex_col()
                .w(px(132.))
                .p(px(6.))
                .rounded(px(theme::R_ITEM + 2.))
                .border_2()
                .border_color(if on { theme::accent().into() } else { theme::card_border() })
                .bg(theme::ink_a(0.03))
                .cursor_pointer()
                .hover(|s| s.bg(theme::ink_a(0.06)))
                .on_click(cx.listener(move |d, _, _, cx| set_look(d, cx, |a| a.theme = t)))
                .child(div().h(px(62.)).rounded(px(7.)).overflow_hidden().border_1().border_color(theme::ink_a(0.08)).child(picture))
                .child(div().mt(px(7.)).px(px(2.)).text_size(px(13.)).text_color(theme::text_primary()).child(label))
                .child(div().px(px(2.)).h(px(15.)).text_size(px(11.)).text_color(theme::text_dim()).children(note)),
        );
    }
    div()
        .flex()
        .flex_col()
        .gap(px(10.))
        .py(px(11.))
        .child(
            row(
                tr!("Theme"),
                Some(tr!(
                    "Light or dark panels and text. System follows your desktop's light or dark style; Custom uses colours you pick."
                )),
                div(),
            )
            .py_0(),
        )
        .child(cards)
        .into_any_element()
}

/// Solid's colour: a dot per tint, the dark ones above the light ones,
/// then the desktop's window colour (while it has one) and a dot that opens
/// a colour picker for one of your own. A light colour turns the theme
/// light and a dark one dark; with System each side keeps its own.
pub(super) fn tint_rows(a: &State, cx: &mut Context<SettingsPage>) -> Vec<AnyElement> {
    let ui = a.ui;
    // The custom theme's panels are its own Panels colour.
    if ui.theme == Theme::Custom {
        return Vec::new();
    }
    let both = ui.theme == Theme::System;
    let light_now = theme::theme_light(ui.theme);
    let shown = |light: bool| both || light == light_now;
    let own = |light: bool| if light { ui.solid_own_light } else { ui.solid_own };
    let tint = |light: bool| if light { ui.solid_tint_light } else { ui.solid_tint };
    let desktop = ui.solid_desktop;

    let line = |tints: [SolidTint; 6], cx: &mut Context<SettingsPage>| {
        let mut l = div().flex().items_center().gap(px(4.));
        for t in tints {
            let side = t.is_light();
            let on = !desktop && shown(side) && own(side).is_none() && tint(side) == t;
            l = l.child(
                colour_dot(SharedString::from(format!("ui-tint-{}", t.as_str())), Some(t.rgb()), None, on)
                    .tooltip(crate::ui::tooltip(t.label()))
                    .on_click(cx.listener(move |d, _, _, cx| set_tint(d, t, true, cx))),
            );
        }
        l
    };
    let presets = div().flex().flex_col().gap(px(2.)).child(line(SolidTint::DARK, cx)).child(line(SolidTint::LIGHT, cx));
    let mut dots = div().flex().items_center().gap(px(4.)).child(presets).child(dot_divider().h(px(56.)));
    let d = theme::desktop_colours();
    let desk = if light_now { d.window_light.or(d.window_dark) } else { d.window_dark.or(d.window_light) };
    if desk.is_some() || desktop {
        dots = dots.child(
            colour_dot("ui-tint-desktop".into(), desk, Some(Icon::Tv), desktop)
                .tooltip(crate::ui::tooltip(tr!("From the desktop")))
                .on_click(cx.listener(|d, _, _, cx| set_desktop_panels(d, cx))),
        );
    }
    let mine = [false, true].into_iter().filter(|s| shown(*s)).find_map(own);
    let own_on = !desktop && [false, true].into_iter().any(|s| shown(s) && own(s).is_some());
    dots = dots.child(
        colour_dot("ui-tint-own".into(), mine, mine.is_none().then_some(Icon::Palette), own_on || a.tint_picker_open)
            .tooltip(crate::ui::tooltip(tr!("Your own colour")))
            .on_click(cx.listener(|d, _, window, cx| {
                let a = &mut d.appearance;
                a.tint_picker_open = !a.tint_picker_open;
                a.accent_picker_open = false;
                if a.tint_picker_open {
                    let c = theme::panel_rgb();
                    a.tint_picker.update(cx, |p, cx| p.set(c, window, cx));
                }
                cx.notify();
            })),
    );
    let note = if desktop {
        tr!("Your desktop's window colour, light or dark with it.")
    } else if both {
        tr!("A dark and a light colour: your desktop's style picks between them.")
    } else {
        tr!("A light colour turns the theme light, a dark one dark.")
    };
    let mut rows = vec![row(tr!("Panel colour"), Some(note), dots).into_any_element()];
    if a.tint_picker_open {
        rows.push(picker_row(
            tr!("Drag on the square and the strip, or type a hex colour. It is adjusted when the text would be hard to read on it."),
            a.tint_picker.clone(),
        ));
    }
    rows
}

/// A round swatch of `c` (a faint fill without one) with `ic` on it,
/// ringed when `on`.
pub(super) fn colour_dot(id: SharedString, c: Option<Rgb>, ic: Option<Icon>, on: bool) -> Stateful<Div> {
    let inner = div().size(px(24.)).rounded_full().border_1().border_color(theme::ink_a(0.22)).flex().items_center().justify_center();
    let (inner, ink) = match c {
        Some((r, g, b)) => {
            (inner.bg(theme::rgb_a(r, g, b, 1.)), if is_light_colour((r, g, b)) { theme::black_a(0.7) } else { theme::white_a(0.9) })
        }
        None => (inner.bg(theme::ink_a(0.06)), theme::text_muted().into()),
    };
    div()
        .id(id)
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(px(32.))
        .rounded_full()
        .border_2()
        .border_color(if on { theme::accent().into() } else { gpui_kit::transparent_black() })
        .cursor_pointer()
        .hover(|s| s.bg(theme::ink_a(0.06)))
        .child(inner.children(ic.map(|ic| crate::ui::icon(ic, px(14.), ink))))
}

/// The thin line between the presets and the dots after them.
pub(super) fn dot_divider() -> Div {
    div().w(px(1.)).h(px(22.)).mx(px(6.)).bg(theme::ink_a(0.12))
}

/// "Your own colour" under a row of dots: a note on the left, `picker` on the right.
pub(super) fn picker_row(note: &str, picker: Entity<ColourPicker>) -> AnyElement {
    div()
        .flex()
        .items_start()
        .justify_between()
        .gap(px(16.))
        .py(px(12.))
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .child(div().text_size(px(14.)).text_color(theme::text_primary()).child(tr!("Your own colour")))
                .child(meta_text(note.to_string())),
        )
        .child(div().w(px(300.)).flex_none().child(picker))
        .into_any_element()
}

/// The picker of [`tint_rows`]: a drag moves the panels' colour on screen,
/// its end saves it.
pub(super) fn new_tint_picker(window: &mut Window, cx: &mut Context<SettingsPage>) -> Entity<ColourPicker> {
    let picker = cx.new(|cx| ColourPicker::new(theme::panel_rgb(), window, cx));
    cx.subscribe(&picker, |d: &mut SettingsPage, _, ev: &PickerEvent, cx| match *ev {
        PickerEvent::Change(c) => set_own(d, c, false, cx),
        PickerEvent::Commit(c) => set_own(d, c, true, cx),
    })
    .detach();
    picker
}

/// `light` panels on the light theme: Dark becomes Light and the other way
/// round; System and Custom keep theirs.
fn follow(ui: &mut Appearance, light: bool) {
    ui.theme = match ui.theme {
        Theme::Dark if light => Theme::Light,
        Theme::Light if !light => Theme::Dark,
        t => t,
    };
}

/// Solid panels in `t`, kept for the theme `t` belongs to; `follow_it`
/// switches to that theme.
pub(super) fn set_tint(d: &mut SettingsPage, t: SolidTint, follow_it: bool, cx: &mut Context<SettingsPage>) {
    let ui = &mut d.appearance.ui;
    ui.interface_mode = InterfaceMode::Solid;
    ui.solid_desktop = false;
    if t.is_light() {
        ui.solid_tint_light = t;
        ui.solid_own_light = None;
    } else {
        ui.solid_tint = t;
        ui.solid_own = None;
    }
    if follow_it {
        follow(ui, t.is_light());
    }
    apply_interface(d, true, cx);
}

/// Solid panels in a colour picked by hand (see [`own_panel`]).
pub(super) fn set_own(d: &mut SettingsPage, c: Rgb, save: bool, cx: &mut Context<SettingsPage>) {
    let (light, c) = own_panel(c);
    let ui = &mut d.appearance.ui;
    ui.interface_mode = InterfaceMode::Solid;
    ui.solid_desktop = false;
    if light {
        ui.solid_own_light = Some(c);
    } else {
        ui.solid_own = Some(c);
    }
    follow(ui, light);
    apply_interface(d, save, cx);
}

/// Solid panels in the desktop's window colour, the theme following the
/// desktop's light or dark style so the two stay together.
fn set_desktop_panels(d: &mut SettingsPage, cx: &mut Context<SettingsPage>) {
    d.appearance.tint_picker_open = false;
    let ui = &mut d.appearance.ui;
    ui.interface_mode = InterfaceMode::Solid;
    ui.solid_desktop = true;
    ui.theme = Theme::System;
    apply_interface(d, true, cx);
}

/// Console rows of the Solid colours (both themes: console mode is dark).
pub(super) fn console_tint_rows(d: &SettingsPage) -> [crate::console::list::Row; 2] {
    use crate::console::list::Row;
    let ui = d.appearance.ui;
    let row = |id: &'static str, label: &'static str, tints: [SolidTint; 6], active: SolidTint| {
        let names = tints.iter().map(|t| SharedString::from(t.label())).collect();
        Row::choice(id, label, names, tints.iter().position(|t| *t == active).unwrap_or(0))
    };
    [
        row("ui-tint", tr!("Colour when dark"), SolidTint::DARK, ui.solid_tint),
        row("ui-tint-light", tr!("Colour when light"), SolidTint::LIGHT, ui.solid_tint_light),
    ]
}

/// The theme's entries of the command palette: switch to each theme, and
/// solid panels in each tint of the theme in use.
pub(super) fn palette_commands(ui: heronite_data::appearance::Appearance) -> Vec<crate::pages::commands::PageCommand> {
    use crate::icons::Icon;
    use crate::pages::commands::PageCommand;
    let themes = [
        (Theme::Light, Icon::Sun, tr!("Use the light theme"), tr!("Light panels and dark text")),
        (Theme::Dark, Icon::Moon, tr!("Use the dark theme"), tr!("Dark panels and light text")),
        (Theme::System, Icon::Sunset, tr!("Follow the desktop's light or dark style"), tr!("Light or dark like the rest of the desktop")),
        (Theme::Custom, Icon::Palette, tr!("Use your custom theme"), tr!("The colours you picked in Settings → Appearance")),
    ];
    let mut v: Vec<PageCommand> = themes
        .into_iter()
        .map(|(t, ic, title, desc)| {
            PageCommand::new(format!("theme:{}", t.as_str()), title, desc, ic)
                .keywords("appearance theme light dark mode night day system aparencia aparência tema claro escuro modo noite dia sistema")
                .disabled((ui.theme == t).then_some(tr!("In use")))
        })
        .collect();
    let light = theme::theme_light(ui.theme);
    let (tints, active, own) =
        if light { (SolidTint::LIGHT, ui.solid_tint_light, ui.solid_own_light) } else { (SolidTint::DARK, ui.solid_tint, ui.solid_own) };
    for t in tints {
        let current = ui.interface_mode == InterfaceMode::Solid && !ui.solid_desktop && own.is_none() && active == t;
        v.push(
            PageCommand::new(
                format!("tint:{}", t.as_str()),
                tr!("Solid interface: {tint}", tint = t.label()),
                tr!("Panels in this colour"),
                Icon::Tune,
            )
            .keywords("appearance colour color tint theme aparencia aparência cor tema")
            .disabled(current.then_some(tr!("In use"))),
        );
    }
    v
}

/// Runs a palette entry of [`palette_commands`]; `false` when `id` is not one.
pub(super) fn run_palette(d: &mut SettingsPage, id: &str, cx: &mut Context<SettingsPage>) -> bool {
    if let Some(t) = id.strip_prefix("theme:").and_then(Theme::parse) {
        set_look(d, cx, |a| a.theme = t);
        return true;
    }
    match id.strip_prefix("tint:").and_then(SolidTint::parse) {
        Some(t) => {
            set_tint(d, t, true, cx);
            true
        }
        None => false,
    }
}
