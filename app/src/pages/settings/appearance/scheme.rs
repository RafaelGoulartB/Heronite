//! Settings → Appearance → Theme (dark, light or the desktop's style) and
//! the solid panels' colour of each theme, with their console rows and
//! command palette entries.

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::appearance::{InterfaceMode, SolidTint, Theme};
use heronite_i18n::{tr, trc};

use super::look::set_look;
use super::{apply_interface, SettingsPage, State};
use crate::theme;
use crate::ui::controls::{row, tabs};

pub(super) fn theme_items() -> [(Theme, &'static str); 3] {
    [(Theme::Dark, trc!("theme", "Dark")), (Theme::Light, trc!("theme", "Light")), (Theme::System, trc!("theme", "System"))]
}

/// A small window in the colours of the light (`light`) or dark theme, with
/// the current accent and solid colour.
fn window_picture(light: bool) -> Div {
    theme::drawn_as(light, || {
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
        let pic = window_picture(light).absolute().top_0().w(relative(2.));
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
            Theme::Dark => window_picture(false),
            Theme::Light => window_picture(true),
            Theme::System => system_picture(),
        };
        let note = (t == Theme::System).then(|| if theme::system_is_light() { tr!("Light now") } else { tr!("Dark now") });
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
            row(tr!("Theme"), Some(tr!("Light or dark panels and text. System follows your desktop's light or dark style.")), div()).py_0(),
        )
        .child(cards)
        .into_any_element()
}

/// Rows of Solid's colour: the current theme's tints, or both rows with
/// System, so each theme keeps its own.
pub(super) fn tint_rows(a: &State, cx: &mut Context<SettingsPage>) -> Vec<AnyElement> {
    let ui = a.ui;
    let both = ui.theme == Theme::System;
    let mut rows = Vec::new();
    for light in [false, true] {
        if !both && light != theme::theme_light(ui.theme) {
            continue;
        }
        let (tints, active) = if light { (SolidTint::LIGHT, ui.solid_tint_light) } else { (SolidTint::DARK, ui.solid_tint) };
        let items: Vec<(SolidTint, &'static str)> = tints.iter().map(|t| (*t, t.label())).collect();
        let this = cx.entity();
        let picker = tabs(if light { "ui-tint-light" } else { "ui-tint" }, &items, active, move |t, _, cx| {
            this.update(cx, |d, cx| set_tint(d, t, cx));
        });
        let label = match (both, light) {
            (false, _) => tr!("Colour"),
            (true, false) => tr!("Colour when dark"),
            (true, true) => tr!("Colour when light"),
        };
        rows.push(row(label, None, picker).into_any_element());
    }
    rows
}

/// Solid panels in `t`, kept for the theme `t` belongs to.
pub(super) fn set_tint(d: &mut SettingsPage, t: SolidTint, cx: &mut Context<SettingsPage>) {
    let ui = &mut d.appearance.ui;
    ui.interface_mode = InterfaceMode::Solid;
    if t.is_light() {
        ui.solid_tint_light = t;
    } else {
        ui.solid_tint = t;
    }
    apply_interface(d, true, cx);
}

/// Console rows of the Solid colours (both themes: console mode is dark).
pub(super) fn console_tint_rows(d: &SettingsPage) -> [crate::console::list::Row; 2] {
    use crate::console::list::Row;
    let ui = d.appearance.ui;
    let row = |id: &'static str, label: &'static str, tints: [SolidTint; 4], active: SolidTint| {
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
    let (tints, active) = if light { (SolidTint::LIGHT, ui.solid_tint_light) } else { (SolidTint::DARK, ui.solid_tint) };
    for t in tints {
        let current = ui.interface_mode == InterfaceMode::Solid && active == t;
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
            set_tint(d, t, cx);
            true
        }
        None => false,
    }
}
