//! Settings → Appearance: colour, reading, the cover options, the picture
//! behind the Collection, motion and layout. All of it lives in
//! `appearance.json` and applies at once.

use gpui_kit::component::slider::Slider;
use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::appearance::{
    Accent, Appearance, BackgroundSource, Corners, CoverHover, CoverLabel, Density, FontChoice, Motion, Section, StartView,
};
use heronite_i18n::tr;

use super::{apply_interface, slider_of, Field, SettingsPage};
use crate::icons::Icon;
use crate::pages::PageEvent;
use crate::theme;
use crate::ui::controls::{button, card, row, tabs, toggle, toggle_row, ButtonKind};
use crate::ui::fields::select_field;
use crate::ui::icon;

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
        Accent::Mint => tr!("Mint"),
        Accent::Amber => tr!("Amber"),
        Accent::Rose => tr!("Rose"),
        Accent::Lavender => tr!("Lavender"),
        Accent::Game => tr!("From the game"),
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
        Section::Timeline => tr!("Timeline"),
        Section::Related => tr!("Related games"),
    }
}

/// Accent colour swatches and the high-contrast switch.
pub(super) fn colour_group(d: &SettingsPage, cx: &mut Context<SettingsPage>) -> Div {
    let a = d.appearance.ui;
    let mut swatches = div().flex().items_center().gap(px(10.));
    for accent in Accent::ALL.iter().copied() {
        let on = a.accent == accent;
        let base = div()
            .id(SharedString::from(format!("accent-{}", accent.as_str())))
            .flex_none()
            .h(px(26.))
            .rounded_full()
            .cursor_pointer()
            .border_2()
            .border_color(if on { white() } else { theme::white_a(0.12) })
            .on_click(cx.listener(move |d, _, _, cx| set_look(d, cx, |a| a.accent = accent)));
        swatches = swatches.child(match accent.rgb() {
            Some((r, g, b)) => base.w(px(26.)).bg(theme::rgb_a(r, g, b, 1.)),
            None => base
                .flex()
                .items_center()
                .gap(px(6.))
                .px(px(10.))
                .bg(theme::white_a(0.06))
                .text_size(px(12.5))
                .text_color(theme::text_primary())
                .child(icon(Icon::Palette, px(14.), theme::accent()))
                .child(accent_label(accent)),
        });
    }
    let contrast = a.high_contrast;
    card([
        row(
            tr!("Accent colour"),
            Some(tr!(
                "The selection ring, links, active buttons and sliders. \"From the game\" takes the colour of the selected game's cover."
            )),
            swatches,
        )
        .into_any_element(),
        toggle_row(
            "high-contrast",
            tr!("High contrast"),
            Some(tr!("Stronger secondary text and borders, for reading over bright artwork.")),
            contrast,
        )
        .on_click(cx.listener(move |d, _, _, cx| set_look(d, cx, |a| a.high_contrast = !contrast)))
        .into_any_element(),
    ])
}

/// Font and density.
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

/// Rows of the Collection background card: which picture, the slideshow,
/// and the button that picks your own.
pub(super) fn picture_rows(d: &SettingsPage, cx: &mut Context<SettingsPage>) -> Vec<AnyElement> {
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
    let mut rows =
        vec![row(tr!("Picture"), Some(tr!("What shows behind the Collection.")), div().w(px(240.)).child(select)).into_any_element()];
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
    rows
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

pub(super) fn motion_group(d: &SettingsPage, cx: &mut Context<SettingsPage>) -> Div {
    let a = d.appearance.ui;
    card([row(
        tr!("Animations"),
        Some(tr!("Reduced keeps the background's cross-fade and drops the rest.")),
        choice("motion", &motion_items(), a.motion, cx, |a, v| a.motion = v),
    )
    .into_any_element()])
}

/// Start view, the switch by the window's width, panel widths and the
/// sections of a game's details. `window_w`: this window's width (px).
pub(super) fn layout_group(d: &SettingsPage, window_w: f32, cx: &mut Context<SettingsPage>) -> Div {
    let a = d.appearance.ui;
    let auto = a.auto_view;
    let mut rows = vec![
        row(tr!("Open the Collection in"), None, choice("start-view", &start_items(), a.start_view, cx, |a, v| a.start_view = v))
            .into_any_element(),
        toggle_row(
            "auto-view",
            tr!("Switch with the window's width"),
            Some(tr!(
                "Grid while the window is narrower than the width below, Focus once it is wider. The Focus / Grid switch in the header still works until the window crosses it again."
            )),
            auto.on,
        )
        .on_click(cx.listener(move |d, _, _, cx| set_look(d, cx, |a| a.auto_view.on = !auto.on)))
        .into_any_element(),
    ];
    if auto.on {
        let slider = Slider::new(slider_of(&d.appearance, Field::AutoViewWidth)).bg(theme::accent()).text_color(theme::accent());
        rows.push(
            row(
                tr!("Width: {value} px", value = auto.width.round()),
                Some(&tr!("This window is {width} px wide now.", width = window_w.round())),
                div().w(px(260.)).child(slider),
            )
            .into_any_element(),
        );
    }
    rows.extend([
        row(
            tr!("Panel widths"),
            Some(tr!("The Focus list and the grid's game panel grow when you drag their edge.")),
            button("reset-widths", ButtonKind::Glass, tr!("Reset widths"))
                .on_click(cx.listener(|_, _, _, cx| cx.emit(PageEvent::ResetPanelWidths))),
        )
        .into_any_element(),
        row(tr!("Game sections"), Some(tr!("Order and visibility of the blocks of a game's page and of the grid's panel.")), div())
            .into_any_element(),
    ]);
    let sections = a.sections;
    let order = sections.order();
    for (i, section) in order.into_iter().enumerate() {
        let shown = sections.shown(section);
        let id = section.as_str();
        let arrow = |dir: &'static str, ic: Icon, delta: i32, enabled: bool| {
            div()
                .id(SharedString::from(format!("sec-{id}-{dir}")))
                .size(px(28.))
                .flex()
                .items_center()
                .justify_center()
                .rounded_full()
                .when(enabled, |d| {
                    d.cursor_pointer()
                        .hover(|s| s.bg(theme::white_a(0.08)))
                        .on_click(cx.listener(move |d, _, _, cx| set_look(d, cx, |a| a.sections = a.sections.moved(section, delta))))
                })
                .child(icon(ic, px(18.), if enabled { theme::text_muted().into() } else { theme::white_a(0.15) }))
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
                        .text_color(if shown { theme::text_primary().into() } else { theme::white_a(0.4) })
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
    card(rows)
}
