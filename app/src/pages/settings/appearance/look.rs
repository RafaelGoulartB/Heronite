//! Settings → Appearance: colour, reading, the cover options, the picture
//! behind the Collection, motion and layout. All of it lives in
//! `appearance.json` and applies at once.

use gpui_kit::component::slider::Slider;
use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::appearance::{
    Accent, Appearance, BackgroundSource, Corners, CoverHover, CoverLabel, Density, FontChoice, Motion, Section, StartView, Theme,
};
use heronite_i18n::tr;

use super::scheme::colour_dot as dot;
use super::{apply_interface, slider_of, Field, SettingsPage};
use crate::icons::Icon;
use crate::pages::PageEvent;
use crate::theme;
use crate::ui::colour_picker::{ColourPicker, PickerEvent};
use crate::ui::controls::{button, card, icon_button, row, tabs, toggle, toggle_row, with_sub_rows, ButtonKind};
use crate::ui::fields::select_field;

/// Changes the look, applies it and saves it.
pub(super) fn set_look(d: &mut SettingsPage, cx: &mut Context<SettingsPage>, edit: impl FnOnce(&mut Appearance)) {
    edit(&mut d.appearance.ui);
    apply_interface(d, true, cx);
    cx.emit(PageEvent::AppearanceChanged);
}

/// Segmented choice of one appearance value.
fn choice<T: Copy + PartialEq + 'static>(
    id: &str,
    items: &[(T, &'static str)],
    active: T,
    cx: &mut Context<SettingsPage>,
    apply: fn(&mut Appearance, T),
) -> Div {
    let this = cx.entity();
    tabs(id, items, active, move |v, _, cx| this.update(cx, |d, cx| set_look(d, cx, |a| apply(a, v))))
}

pub(super) fn accent_label(a: Accent) -> &'static str {
    match a {
        Accent::Ice => tr!("Ice"),
        Accent::Sky => tr!("Sky"),
        Accent::Mint => tr!("Mint"),
        Accent::Amber => tr!("Amber"),
        Accent::Coral => tr!("Coral"),
        Accent::Rose => tr!("Rose"),
        Accent::Lavender => tr!("Lavender"),
        Accent::Game => tr!("From the game"),
        Accent::Desktop => tr!("From the desktop"),
    }
}

pub(super) fn font_items() -> [(FontChoice, &'static str); 3] {
    [(FontChoice::Default, "Cabin + Rubik"), (FontChoice::Rubik, "Rubik"), (FontChoice::System, tr!("System"))]
}

pub(super) fn density_items() -> [(Density, &'static str); 2] {
    [(Density::Comfortable, tr!("Comfortable")), (Density::Compact, tr!("Compact"))]
}

pub(super) fn corner_items() -> [(Corners, &'static str); 3] {
    [(Corners::Square, tr!("Square")), (Corners::Soft, tr!("Soft")), (Corners::Round, tr!("Round"))]
}

pub(super) fn hover_items() -> [(CoverHover, &'static str); 3] {
    [(CoverHover::Nothing, tr!("Nothing")), (CoverHover::Lift, tr!("Lift")), (CoverHover::Glow, tr!("Glow"))]
}

pub(super) fn motion_items() -> [(Motion, &'static str); 3] {
    [(Motion::Full, tr!("Full")), (Motion::Reduced, tr!("Reduced")), (Motion::Off, tr!("Off"))]
}

pub(super) fn start_items() -> [(StartView, &'static str); 3] {
    [(StartView::Last, tr!("Last used")), (StartView::Focus, tr!("Focus")), (StartView::Grid, tr!("Grid"))]
}

pub(super) fn label_items() -> [(CoverLabel, &'static str); 6] {
    [
        (CoverLabel::Auto, tr!("Playtime or the sort value")),
        (CoverLabel::Playtime, tr!("Playtime")),
        (CoverLabel::LastPlayed, tr!("Last played")),
        (CoverLabel::Status, tr!("Status")),
        (CoverLabel::Title, tr!("Title")),
        (CoverLabel::Nothing, tr!("Nothing")),
    ]
}

pub(super) fn picture_items() -> [(BackgroundSource, &'static str); 5] {
    [
        (BackgroundSource::GameArt, tr!("The game's artwork")),
        (BackgroundSource::Screenshot, tr!("One of your screenshots")),
        (BackgroundSource::Cover, tr!("The cover, blurred")),
        (BackgroundSource::Solid, tr!("Solid colour")),
        (BackgroundSource::Custom, tr!("Your own picture")),
    ]
}

pub(super) fn section_label(s: Section) -> &'static str {
    match s {
        Section::Screenshots => tr!("Screenshots"),
        Section::Overview => tr!("Overview"),
        Section::Details => tr!("Details"),
        Section::Achievements => tr!("Achievements"),
        Section::Timeline => tr!("Timeline"),
        Section::Related => tr!("Related games"),
    }
}

/// The Colour card, each choice followed by what depends on it: the theme
/// (and the custom theme's colours), the panels (their glass or their
/// colour), the accent (and its own colour) and high contrast.
pub(super) fn colour_group(d: &mut SettingsPage, cx: &mut Context<SettingsPage>) -> Div {
    let a = d.appearance.ui;
    let contrast = a.high_contrast;
    let theme_row = super::scheme::theme_row(d, cx);
    let custom_rows = super::custom::rows(d, cx);
    let (panels, panel_rows) = super::panel_rows(&d.appearance, cx);
    // The custom theme has an accent of its own.
    let accent = (a.theme != Theme::Custom).then(|| with_sub_rows(accent_row(d, cx), accent_picker_rows(d)));
    let high_contrast = toggle_row(
        "high-contrast",
        tr!("High contrast"),
        Some(tr!("Stronger secondary text and borders, for reading over bright artwork.")),
        contrast,
    )
    .on_click(cx.listener(move |d, _, _, cx| set_look(d, cx, |a| a.high_contrast = !contrast)))
    .into_any_element();
    card([with_sub_rows(theme_row, custom_rows), with_sub_rows(panels, panel_rows)].into_iter().chain(accent).chain([high_contrast]))
}

/// A preset accent, or "From the game": the own colour goes.
fn set_accent(d: &mut SettingsPage, accent: Accent, cx: &mut Context<SettingsPage>) {
    d.appearance.accent_picker_open = false;
    set_look(d, cx, |a| {
        a.accent = accent;
        a.accent_own = None;
    });
}

/// The accent's dots: the presets, then "From the game", the desktop's
/// (while it has one) and your own colour.
fn accent_row(d: &SettingsPage, cx: &mut Context<SettingsPage>) -> AnyElement {
    let a = d.appearance.ui;
    let open = d.appearance.accent_picker_open;
    let mut dots = div().flex().flex_wrap().items_center().justify_end().gap(px(4.));
    for accent in Accent::ALL.iter().copied().filter(|x| x.is_preset()) {
        dots = dots.child(
            dot(
                SharedString::from(format!("accent-{}", accent.as_str())),
                theme::accent_shade(accent),
                None,
                a.accent_own.is_none() && a.accent == accent,
            )
            .tooltip(crate::ui::tooltip(accent_label(accent)))
            .on_click(cx.listener(move |d, _, _, cx| set_accent(d, accent, cx))),
        );
    }
    let game_on = a.accent_own.is_none() && a.accent == Accent::Game;
    dots = dots.child(super::scheme::dot_divider()).child(
        dot("accent-game".into(), None, Some(Icon::SportsEsports), game_on)
            .tooltip(crate::ui::tooltip(accent_label(Accent::Game)))
            .on_click(cx.listener(|d, _, _, cx| set_accent(d, Accent::Game, cx))),
    );
    let desktop = theme::accent_shade(Accent::Desktop);
    let desktop_on = a.accent_own.is_none() && a.accent == Accent::Desktop;
    if desktop.is_some() || desktop_on {
        dots = dots.child(
            dot("accent-desktop".into(), desktop, Some(Icon::Tv), desktop_on)
                .tooltip(crate::ui::tooltip(accent_label(Accent::Desktop)))
                .on_click(cx.listener(|d, _, _, cx| set_accent(d, Accent::Desktop, cx))),
        );
    }
    let own = a.accent_own.map(theme::own_accent_shade);
    dots = dots.child(
        dot("accent-own".into(), own, own.is_none().then_some(Icon::Palette), a.accent_own.is_some() || open)
            .tooltip(crate::ui::tooltip(tr!("Your own colour")))
            .on_click(cx.listener(|d, _, window, cx| {
                let a = &mut d.appearance;
                a.accent_picker_open = !a.accent_picker_open;
                a.tint_picker_open = false;
                if a.accent_picker_open {
                    let c = a.ui.accent_own.unwrap_or_else(theme::accent_rgb);
                    a.accent_picker.update(cx, |p, cx| p.set(c, window, cx));
                }
                cx.notify();
            })),
    );
    row(
        tr!("Accent colour"),
        Some(tr!("The selection ring, links, active buttons and sliders. The controller takes the colour of the selected game's cover.")),
        dots,
    )
    .into_any_element()
}

/// The accent's colour picker, while it is open.
fn accent_picker_rows(d: &SettingsPage) -> Vec<AnyElement> {
    let a = &d.appearance;
    if !a.accent_picker_open {
        return Vec::new();
    }
    vec![super::scheme::picker_row(
        tr!("Drag on the square and the strip, or type a hex colour. Each theme shows it a little lighter or darker when it would be hard to read."),
        a.accent_picker.clone(),
    )]
}

/// The picker of [`accent_row`]: a drag moves the accent on screen, its end
/// saves it.
pub(super) fn new_accent_picker(window: &mut Window, cx: &mut Context<SettingsPage>) -> Entity<ColourPicker> {
    let picker = cx.new(|cx| ColourPicker::new(theme::accent_rgb(), window, cx));
    cx.subscribe(&picker, |d: &mut SettingsPage, _, ev: &PickerEvent, cx| {
        let (PickerEvent::Change(c) | PickerEvent::Commit(c)) = *ev;
        d.appearance.ui.accent_own = Some(c);
        apply_interface(d, matches!(ev, PickerEvent::Commit(_)), cx);
        cx.emit(PageEvent::AppearanceChanged);
    })
    .detach();
    picker
}

/// Font, density and animations.
pub(super) fn reading_group(d: &SettingsPage, cx: &mut Context<SettingsPage>) -> Div {
    let a = d.appearance.ui;
    card([
        row(
            tr!("Font"),
            Some(tr!("Cabin for text and Rubik for titles, Rubik everywhere, or your desktop's font.")),
            choice("font", &font_items(), a.font, cx, |a, v| a.font = v),
        )
        .into_any_element(),
        row(
            tr!("Density"),
            Some(tr!("Compact brings the covers closer together, so more games fit on screen.")),
            choice("density", &density_items(), a.density, cx, |a, v| a.density = v),
        )
        .into_any_element(),
        row(
            tr!("Animations"),
            Some(tr!("Reduced keeps the background's cross-fade and drops the rest.")),
            choice("motion", &motion_items(), a.motion, cx, |a, v| a.motion = v),
        )
        .into_any_element(),
    ])
}

/// Rows of the Covers card after the aspect and the grey switch.
pub(super) fn cover_rows(d: &SettingsPage, cx: &mut Context<SettingsPage>) -> Vec<AnyElement> {
    let a = d.appearance.ui;
    let labels = label_items();
    let current = labels.iter().find(|(l, _)| *l == a.cover_label).map(|(_, t)| *t).unwrap_or_default();
    let label_select = select_field(
        "cover-label",
        None,
        current,
        false,
        d.appearance.label_open,
        labels.iter().map(|(l, t)| (SharedString::from(*t), *l == a.cover_label)).collect(),
        cx,
        |d, _, cx| {
            d.appearance.label_open = !d.appearance.label_open;
            cx.notify();
        },
        |d, i, _, cx| {
            d.appearance.label_open = false;
            let label = label_items()[i].0;
            set_look(d, cx, |a| a.cover_label = label);
        },
    );
    let badges = a.cover_badges;
    vec![
        row(tr!("Corners"), None, choice("corners", &corner_items(), a.cover_corners, cx, |a, v| a.cover_corners = v)).into_any_element(),
        row(
            tr!("Under the cover"),
            Some(tr!("The line under each cover in the list and the grid.")),
            div().w(px(240.)).child(label_select),
        )
        .into_any_element(),
        toggle_row("cover-badges", tr!("Favourite star"), Some(tr!("A star on the covers of your favourites.")), badges)
            .on_click(cx.listener(move |d, _, _, cx| set_look(d, cx, |a| a.cover_badges = !badges)))
            .into_any_element(),
        row(
            tr!("Under the pointer"),
            Some(tr!("What a cover does when the pointer is over it.")),
            choice("hover", &hover_items(), a.cover_hover, cx, |a, v| a.cover_hover = v),
        )
        .into_any_element(),
    ]
}

/// The Collection background card's picture row, with the slideshow or the
/// button that picks your own under it.
pub(super) fn picture_row(d: &SettingsPage, cx: &mut Context<SettingsPage>) -> AnyElement {
    let a = d.appearance.ui;
    let items = picture_items();
    let current = items.iter().find(|(p, _)| *p == a.background).map(|(_, t)| *t).unwrap_or_default();
    let select = select_field(
        "bg-picture",
        None,
        current,
        !d.appearance.enabled,
        d.appearance.picture_open,
        items.iter().map(|(p, t)| (SharedString::from(*t), *p == a.background)).collect(),
        cx,
        |d, _, cx| {
            d.appearance.picture_open = !d.appearance.picture_open;
            cx.notify();
        },
        |d, i, _, cx| {
            d.appearance.picture_open = false;
            let source = picture_items()[i].0;
            if source == BackgroundSource::Custom && !d.ctx.app.custom_background().exists() {
                pick_picture(cx);
                return;
            }
            set_look(d, cx, |a| a.background = source);
        },
    );
    let picture = row(tr!("Picture"), Some(tr!("What shows behind the Collection.")), div().w(px(240.)).child(select)).into_any_element();
    let mut rows = Vec::new();
    match a.background {
        BackgroundSource::Screenshot => {
            let on = a.background_slideshow;
            rows.push(
                toggle_row(
                    "bg-slideshow",
                    tr!("Change the screenshot by itself"),
                    Some(tr!("Goes through your screenshots of the game, one every 20 seconds.")),
                    on,
                )
                .on_click(cx.listener(move |d, _, _, cx| set_look(d, cx, |a| a.background_slideshow = !on)))
                .into_any_element(),
            );
        }
        BackgroundSource::Custom => rows.push(
            row(
                tr!("Your picture"),
                Some(tr!("Copied into Heronite's settings, so moving the file later does not matter.")),
                button("bg-custom-pick", ButtonKind::Glass, tr!("Choose a picture…")).on_click(cx.listener(|_, _, _, cx| pick_picture(cx))),
            )
            .into_any_element(),
        ),
        _ => {}
    }
    with_sub_rows(picture, rows)
}

/// Asks for a picture, copies it to `AppPaths::custom_background` and shows it.
fn pick_picture(cx: &mut Context<SettingsPage>) {
    let rx =
        cx.prompt_for_paths(PathPromptOptions { files: true, directories: false, multiple: false, prompt: Some(tr!("Select").into()) });
    cx.spawn(async move |this, cx| {
        let Ok(Ok(Some(paths))) = rx.await else { return };
        let Some(src) = paths.into_iter().next() else { return };
        let Ok(dest) = this.read_with(cx, |d, _| d.ctx.app.custom_background()) else { return };
        let copied = cx
            .background_spawn(async move {
                if let Some(dir) = dest.parent() {
                    std::fs::create_dir_all(dir)?;
                }
                std::fs::copy(&src, &dest).map(|_| ())
            })
            .await;
        this.update(cx, |d, cx| match copied {
            Ok(()) => {
                theme::bump_custom_background();
                set_look(d, cx, |a| a.background = BackgroundSource::Custom);
            }
            Err(err) => {
                d.error = Some(tr!("Could not copy the picture: {err}", err = err).into());
                cx.notify();
            }
        })
        .ok();
    })
    .detach();
}

/// Start view, the switch by the window's width, panel widths and the
/// sections of a game's details. `window_w`: this window's width (px).
pub(super) fn layout_group(d: &SettingsPage, window_w: f32, cx: &mut Context<SettingsPage>) -> Div {
    let a = d.appearance.ui;
    let auto = a.auto_view;
    let start = row(tr!("Open the Collection in"), None, choice("start-view", &start_items(), a.start_view, cx, |a, v| a.start_view = v))
        .into_any_element();
    let switch = toggle_row(
            "auto-view",
            tr!("Switch with the window's width"),
            Some(tr!(
                "Grid while the window is narrower than the width below, Focus once it is wider. The Focus / Grid switch in the header still works until the window crosses it again."
            )),
            auto.on,
        )
        .on_click(cx.listener(move |d, _, _, cx| set_look(d, cx, |a| a.auto_view.on = !auto.on)))
        .into_any_element();
    let mut width = Vec::new();
    if auto.on {
        let slider = Slider::new(slider_of(&d.appearance, Field::AutoViewWidth)).bg(theme::accent()).text_color(theme::accent());
        width.push(
            row(
                tr!("Width: {value} px", value = auto.width.round()),
                Some(&tr!("This window is {width} px wide now.", width = window_w.round())),
                div().w(px(260.)).child(slider),
            )
            .into_any_element(),
        );
    }
    let widths = row(
        tr!("Panel widths"),
        Some(tr!("The Focus list and the grid's game panel grow when you drag their edge.")),
        button("reset-widths", ButtonKind::Glass, tr!("Reset widths"))
            .on_click(cx.listener(|_, _, _, cx| cx.emit(PageEvent::ResetPanelWidths))),
    )
    .into_any_element();
    let sections_row =
        row(tr!("Game sections"), Some(tr!("Order and visibility of the blocks of a game's page and of the grid's panel.")), div())
            .into_any_element();
    let mut rows = Vec::new();
    let sections = a.sections;
    let order = sections.order();
    for (i, section) in order.into_iter().enumerate() {
        let shown = sections.shown(section);
        let id = section.as_str();
        let arrow = |dir: &'static str, ic: Icon, delta: i32, enabled: bool| {
            icon_button(SharedString::from(format!("sec-{id}-{dir}")), ic, 18., enabled).when(enabled, |b| {
                b.on_click(cx.listener(move |d, _, _, cx| set_look(d, cx, |a| a.sections = a.sections.moved(section, delta))))
            })
        };
        rows.push(
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .py(px(6.))
                .pl(px(4.))
                .child(
                    div()
                        .flex_1()
                        .text_size(px(14.))
                        .text_color(if shown { theme::text_primary().into() } else { theme::ink_a(0.4) })
                        .child(section_label(section)),
                )
                .child(arrow("up", Icon::ArrowUpward, -1, i > 0))
                .child(arrow("down", Icon::ArrowDownward, 1, i + 1 < order.len()))
                .child(
                    toggle(SharedString::from(format!("sec-{id}-shown")), shown, false)
                        .on_click(cx.listener(move |d, _, _, cx| set_look(d, cx, |a| a.sections = a.sections.toggled(section)))),
                )
                .into_any_element(),
        );
    }
    card([start, with_sub_rows(switch, width), widths, with_sub_rows(sections_row, rows)])
}
