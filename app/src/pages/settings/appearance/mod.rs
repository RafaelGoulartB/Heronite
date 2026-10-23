//! Settings → Appearance: profiles, colour, reading, the two backgrounds,
//! covers, motion and layout.
//!
//! - Profiles (`profiles.rs`): built-in and saved looks, export / import.
//! - Colour, reading, the cover options, the background picture, motion and
//!   layout (`look.rs`): `appearance.json`, applied at once.
//! - Collection background (the selected game's art behind the Collection):
//!   on/off, blur, darkening, colour intensity, edge shading live in
//!   `local_library/settings.json` (`background`); its
//!   Opacity lives in this app's `appearance.json` (`collectionOpacity`).
//! - Interface background (menus, dialogs, pages, palette...): Transparent
//!   (glass) or Solid, opacity, blur, solid tint; all in `appearance.json`
//!   (`heronite_data::appearance`). Sliders apply live through
//!   `theme::set_appearance` and are saved on release.

use std::sync::Arc;
use std::time::Duration;

use gpui_kit::component::slider::{Slider, SliderEvent, SliderState, SliderValue};
use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::appearance::{
    Appearance, InterfaceMode, SolidTint, AUTO_VIEW_WIDTH_RANGE, AUTO_VIEW_WIDTH_STEP, INTERFACE_BLUR_RANGE, INTERFACE_BLUR_STEP,
    INTERFACE_OPACITY_RANGE,
};
use heronite_data::settings::{self, CoverAspect, EffectiveSettings, PartialBackground};
use heronite_i18n::{n_, tr};
use heronite_images::{ImageRef, Opts};

mod custom;
mod look;
mod profiles;
mod scheme;

use gpui_kit::component::input::InputState;
use heronite_data::appearance::Profile;

use super::SettingsPage;
use crate::collection::stage::{scrim_layers, Scrim};
use crate::pages::PageCtx;
use crate::theme;
use crate::ui::controls::{button, caption, card, row, tabs, toggle_row, with_sub_rows, ButtonKind};
use crate::ui::fields::{meta_text, select_field};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Field {
    Blur,
    Dimming,
    Saturation,
    Scrim,
    /// Collection art opacity (%).
    CollectionOpacity,
    /// Interface panel opacity (%).
    PanelOpacity,
    /// Interface blur (px).
    PanelBlur,
    /// Window width of the Focus / Grid switch (px).
    AutoViewWidth,
}

pub struct State {
    enabled: bool,
    ui: Appearance,
    blur: f32,
    dimming: f32,
    saturation: f32,
    scrim: f32,
    sliders: Vec<(Field, Entity<SliderState>)>,
    preview_chain: Vec<ImageRef>,
    preview: Option<Arc<RenderImage>>,
    preview_task: Option<Task<()>>,
    opts: Opts,
    aspect_open: bool,
    /// The "Under the cover" and background picture selects are open.
    label_open: bool,
    picture_open: bool,
    /// The Collection background's finer sliders (blur, colour intensity,
    /// edge shading) show.
    more_background: bool,
    /// The user's saved looks; `None` until read (`profiles::ensure_loaded`).
    profiles: Option<Vec<Profile>>,
    profiles_task: Option<Task<()>>,
    /// Name typed for "Save as profile".
    profile_name: Entity<InputState>,
    /// The custom theme's editor.
    custom_editor: custom::Editor,
    /// The picker of a solid colour of your own, and whether it shows.
    tint_picker: Entity<crate::ui::colour_picker::ColourPicker>,
    tint_picker_open: bool,
    /// The same for the accent.
    accent_picker: Entity<crate::ui::colour_picker::ColourPicker>,
    accent_picker_open: bool,
}

const ASPECTS: [(CoverAspect, &str); 6] = [
    (CoverAspect::Steam, "Steam (2:3)"),
    (CoverAspect::Igdb, "IGDB (3:4)"),
    (CoverAspect::Gog, "GOG (11:15)"),
    (CoverAspect::Square, n_!("Square (1:1)")),
    (CoverAspect::Dvd, n_!("DVD case (5:7)")),
    (CoverAspect::Banner, n_!("Banner (16:9)")),
];

impl State {
    pub fn new(
        ctx: &PageCtx,
        s: &EffectiveSettings,
        preview_chain: Vec<ImageRef>,
        window: &mut Window,
        cx: &mut Context<SettingsPage>,
    ) -> Self {
        let b = &s.background;
        // Already loaded by the Collection (no I/O here).
        let ui = theme::appearance();
        let mut sliders = Vec::new();
        for (field, min, max, step, value) in [
            (Field::CollectionOpacity, 0., 100., 1., ui.collection_opacity as f64),
            (Field::Blur, 0., 40., 1., b.blur.get()),
            (Field::Dimming, 0., 90., 1., b.dimming.get()),
            (Field::Saturation, 0., 150., 1., b.saturation.get()),
            (Field::Scrim, 0., 150., 1., b.scrim.get()),
            (Field::PanelOpacity, INTERFACE_OPACITY_RANGE.0, INTERFACE_OPACITY_RANGE.1, 1., ui.interface_opacity as f64),
            (Field::PanelBlur, INTERFACE_BLUR_RANGE.0, INTERFACE_BLUR_RANGE.1, INTERFACE_BLUR_STEP, ui.interface_blur as f64),
            (Field::AutoViewWidth, AUTO_VIEW_WIDTH_RANGE.0, AUTO_VIEW_WIDTH_RANGE.1, AUTO_VIEW_WIDTH_STEP, ui.auto_view.width as f64),
        ] {
            // `max` first: each setter clamps against the other bound, and the default `max` is 100.
            let state = cx.new(|_| SliderState::new().max(max).min(min).step(step).default_value(value as f32));
            cx.subscribe_in(&state, window, move |d: &mut SettingsPage, _, ev: &SliderEvent, _, cx| {
                let (value, release) = match ev {
                    SliderEvent::Change(v) => (v, false),
                    SliderEvent::Release(v) => (v, true),
                };
                let SliderValue::Single(v) = *value else { return };
                let a = &mut d.appearance;
                match field {
                    Field::Blur => a.blur = v,
                    Field::Dimming => a.dimming = v,
                    Field::Saturation => a.saturation = v,
                    Field::Scrim => a.scrim = v,
                    Field::CollectionOpacity => a.ui.collection_opacity = v,
                    Field::PanelOpacity => a.ui.interface_opacity = v,
                    Field::PanelBlur => a.ui.interface_blur = v,
                    Field::AutoViewWidth => a.ui.auto_view.width = v,
                }
                if matches!(field, Field::Blur | Field::Saturation) {
                    schedule_preview(d, cx);
                }
                if matches!(field, Field::CollectionOpacity | Field::PanelOpacity | Field::PanelBlur | Field::AutoViewWidth) {
                    apply_interface(d, release, cx);
                    // The Collection behind switches its view live as the width moves past the window.
                    if field == Field::AutoViewWidth {
                        cx.emit(crate::pages::PageEvent::AppearanceChanged);
                    }
                } else if release {
                    save_background(d, cx);
                }
                cx.notify();
            })
            .detach();
            sliders.push((field, state));
        }
        let mut this = Self {
            enabled: b.enabled,
            ui,
            blur: b.blur.get() as f32,
            dimming: b.dimming.get() as f32,
            saturation: b.saturation.get() as f32,
            scrim: b.scrim.get() as f32,
            sliders,
            preview_chain,
            preview: None,
            preview_task: None,
            opts: Opts::new(ctx.data_paths.images_cache_dir(), Some(ctx.app.thumbs_dir())),
            aspect_open: false,
            label_open: false,
            picture_open: false,
            more_background: false,
            profiles: None,
            profiles_task: None,
            profile_name: crate::ui::fields::new_input(window, cx, "", tr!("Profile name")),
            custom_editor: custom::Editor::new(&ui.custom, window, cx),
            tint_picker: scheme::new_tint_picker(window, cx),
            tint_picker_open: false,
            accent_picker: look::new_accent_picker(window, cx),
            accent_picker_open: false,
        };
        this.preview_task = None;
        this
    }
}

impl State {
    /// A different game is focused: the preview is rebuilt on the next render.
    pub fn set_chain(&mut self, chain: Vec<ImageRef>) {
        self.preview_chain = chain;
        self.preview = None;
        self.preview_task = None;
    }
}

/// Recomputes the blurred preview off the UI thread (debounced 60ms).
fn schedule_preview(d: &mut SettingsPage, cx: &mut Context<SettingsPage>) {
    let a = &d.appearance;
    if a.preview_chain.is_empty() {
        return;
    }
    let chain = a.preview_chain.clone();
    let opts = a.opts.clone();
    let (blur, sat) = (a.blur, a.saturation / 100.);
    d.appearance.preview_task = Some(cx.spawn(async move |this, cx| {
        cx.background_executor().timer(Duration::from_millis(60)).await;
        let decoded = cx.background_spawn(async move { heronite_images::load_backdrop(&chain, 776, 148, blur, sat, &opts) }).await;
        if let Ok(decoded) = decoded {
            let frame = image::Frame::new(decoded.into_image_buffer());
            let img = Arc::new(RenderImage::new(smallvec::smallvec![frame]));
            this.update(cx, |d, cx| {
                d.appearance.preview = Some(img);
                cx.notify();
            })
            .ok();
        }
    }));
}

/// Applies the Interface / Collection-opacity values to the running app (every
/// glass surface reads them) and, when `save`, writes them off the UI thread.
fn apply_interface(d: &mut SettingsPage, save: bool, cx: &mut Context<SettingsPage>) {
    let ui = d.appearance.ui.normalized();
    theme::set_appearance(ui);
    if save {
        let app = d.ctx.app.clone();
        cx.background_spawn(async move {
            if let Err(err) = ui.save(&app) {
                log::warn!("saving appearance.json: {err:#}");
            }
        })
        .detach();
    }
    cx.notify();
}

fn save_background(d: &mut SettingsPage, cx: &mut Context<SettingsPage>) {
    let a = &d.appearance;
    let patch = PartialBackground {
        enabled: Some(a.enabled),
        blur: Some(a.blur as f64),
        dimming: Some(a.dimming as f64),
        saturation: Some(a.saturation as f64),
        scrim: Some(a.scrim as f64),
    };
    let res = settings::save_ui(&d.ctx.data_paths, None, Some(&patch));
    d.saved(res, cx);
}

/// A slider row: label (with the live value) on the left, the slider on the right.
fn slider_row(label: String, state: &Entity<SliderState>) -> AnyElement {
    slider_row_with(label, None, state)
}

/// A [`slider_row`] with a description under the label.
fn slider_row_with(label: String, note: Option<&str>, state: &Entity<SliderState>) -> AnyElement {
    let slider = Slider::new(state).bg(theme::accent()).text_color(theme::accent());
    row(label, note, div().w(px(260.)).child(slider)).into_any_element()
}

fn set_sliders(a: &State, values: &[(Field, f32)], window: &mut Window, cx: &mut Context<SettingsPage>) {
    for (field, v) in values {
        if let Some((_, state)) = a.sliders.iter().find(|(f, _)| f == field) {
            state.update(cx, |s, cx| s.set_value(*v, window, cx));
        }
    }
}

fn slider_of(a: &State, field: Field) -> &Entity<SliderState> {
    &a.sliders.iter().find(|(f, _)| *f == field).expect("slider created in State::new").1
}

/// The panels' rows of the Colour card: Transparent or Solid, and under it
/// the glass's opacity and blur or the solid colour (the custom theme's own
/// panel colour needs no row).
pub(super) fn panel_rows(a: &State, cx: &mut Context<SettingsPage>) -> (AnyElement, Vec<AnyElement>) {
    let this = cx.entity();
    let mode =
        tabs("ui-mode", &[(InterfaceMode::Transparent, tr!("Transparent")), (InterfaceMode::Solid, tr!("Solid"))], a.ui.interface_mode, {
            let this = this.clone();
            move |m, _, cx| {
                this.update(cx, |d, cx| {
                    d.appearance.ui.interface_mode = m;
                    apply_interface(d, true, cx);
                });
            }
        });
    let panels = row(
        tr!("Panels"),
        Some(match a.ui.interface_mode {
            InterfaceMode::Transparent => {
                tr!("Menus, dialogs and pages like this one are glass: the blurred game artwork shows through.")
            }
            InterfaceMode::Solid => tr!("Menus, dialogs and pages like this one are a solid colour, no artwork."),
        }),
        mode,
    )
    .into_any_element();
    let rows = match a.ui.interface_mode {
        InterfaceMode::Transparent => vec![
            slider_row(tr!("Opacity: {value}%", value = a.ui.interface_opacity.round()), slider_of(a, Field::PanelOpacity)),
            slider_row(tr!("Blur: {value} px", value = a.ui.interface_blur.round()), slider_of(a, Field::PanelBlur)),
        ],
        InterfaceMode::Solid => scheme::tint_rows(a, cx),
    };
    (panels, rows)
}

pub fn render(d: &mut SettingsPage, window: &mut Window, cx: &mut Context<SettingsPage>) -> AnyElement {
    if d.appearance.preview.is_none() && d.appearance.preview_task.is_none() {
        schedule_preview(d, cx);
    }
    let a = &d.appearance;
    let scrim = Scrim { dim: a.dimming / 100., scrim: a.scrim / 100. };
    let preview = div()
        .relative()
        .h(px(148.))
        .w_full()
        .rounded(px(theme::R_TILE))
        .border_1()
        .border_color(theme::ink_a(0.08))
        .overflow_hidden()
        .bg(theme::page_bg())
        .when_some(a.preview.clone().filter(|_| a.enabled), |el, img| {
            el.child(img_el(img).opacity(a.ui.collection_opacity / 100.)).child(scrim_layers(scrim))
        })
        .child(
            div()
                .absolute()
                .left_0()
                .bottom_0()
                .flex()
                .flex_col()
                .gap(px(2.))
                .px(px(12.))
                .py(px(10.))
                .child(div().text_size(px(13.)).font_weight(theme::W_BOLD).text_color(theme::text_bright()).child(tr!("Live preview")))
                .child(
                    div().text_size(px(12.)).text_color(theme::ink_a(0.8)).child(tr!("Titles, details and tags sit on top of this layer.")),
                ),
        );

    let mut rows: Vec<AnyElement> = vec![toggle_row(
        "bg-enabled",
        tr!("Use the game artwork as page background"),
        Some(tr!("Only behind the Collection list and the game details.")),
        a.enabled,
    )
    .on_click(cx.listener(|d, _, _, cx| {
        d.appearance.enabled = !d.appearance.enabled;
        save_background(d, cx);
    }))
    .into_any_element()];
    // Off: nothing else applies, so nothing else shows.
    if a.enabled {
        rows.push(look::picture_row(d, cx));
        let a = &d.appearance;
        let more = a.more_background;
        let mut extra = Vec::new();
        for (field, state) in &a.sliders {
            let row = match field {
                Field::CollectionOpacity => slider_row(tr!("Opacity: {value}%", value = a.ui.collection_opacity.round()), state),
                // The light theme fades the art where the dark one darkens it.
                Field::Dimming if theme::theme_is_light() => slider_row(tr!("Fading: {value}%", value = a.dimming.round()), state),
                Field::Dimming => slider_row(tr!("Darkening: {value}%", value = a.dimming.round()), state),
                Field::Blur | Field::Saturation | Field::Scrim if !more => continue,
                Field::Blur => {
                    extra.push(slider_row(tr!("Blur: {value} px", value = a.blur.round()), state));
                    continue;
                }
                Field::Saturation => {
                    extra.push(slider_row(tr!("Colour intensity: {value}%", value = a.saturation.round()), state));
                    continue;
                }
                Field::Scrim => {
                    extra.push(slider_row_with(
                        tr!("Edge shading: {value}%", value = a.scrim.round()),
                        Some(if theme::theme_is_light() {
                            tr!("Fades the left column and the top and bottom edges, where the game list and the toolbars sit.")
                        } else {
                            tr!("Darkens the left column and the top and bottom edges, where the game list and the toolbars sit.")
                        }),
                        state,
                    ));
                    continue;
                }
                Field::PanelOpacity | Field::PanelBlur | Field::AutoViewWidth => continue,
            };
            rows.push(row);
        }
        // The finer sliders under their switch.
        let more_link = div()
            .id("bg-more")
            .flex()
            .items_center()
            .gap(px(6.))
            .py(px(11.))
            .cursor_pointer()
            .text_size(px(13.))
            .text_color(theme::accent())
            .hover(|s| s.text_color(theme::accent_overlay()))
            .on_click(cx.listener(|d, _, _, cx| {
                d.appearance.more_background = !d.appearance.more_background;
                cx.notify();
            }))
            .child(if more { tr!("Fewer adjustments") } else { tr!("More adjustments: blur, colour intensity, edge shading") })
            .child(
                crate::ui::icon(crate::icons::Icon::ExpandMore, px(16.), theme::accent())
                    .when(more, |i| i.with_transformation(Transformation::rotate(percentage(0.5)))),
            )
            .into_any_element();
        rows.push(with_sub_rows(more_link, extra));
    }
    let background = card(rows);

    let aspect = CoverAspect::parse(&d.settings.metadata.cover_aspect);
    let aspect_label = ASPECTS.iter().find(|(a, _)| *a == aspect).map(|(_, l)| tr(l)).unwrap_or("Steam (2:3)");
    let aspect_options: Vec<(SharedString, bool)> = ASPECTS.iter().map(|(a, l)| (tr(l).into(), *a == aspect)).collect();
    let aspect_select = select_field(
        "cover-aspect",
        None,
        aspect_label,
        false,
        a.aspect_open,
        aspect_options,
        cx,
        |d, _, cx| {
            d.appearance.aspect_open = !d.appearance.aspect_open;
            cx.notify();
        },
        |d, i, _, cx| {
            d.appearance.aspect_open = false;
            let mut patch = d.settings.metadata.clone();
            patch.cover_aspect = ASPECTS[i].0.as_str().to_string();
            let res = settings::save_metadata(&d.ctx.data_paths, &patch);
            d.saved(res, cx);
        },
    );

    let grey = d.settings.grey_uninstalled_games;
    let reset = button("bg-reset", ButtonKind::Glass, tr!("Reset background")).on_click(cx.listener(|d, _, window, cx| {
        let defaults = heronite_data::models::BackgroundSettings::default();
        let a = &mut d.appearance;
        a.enabled = defaults.enabled;
        a.blur = defaults.blur.get() as f32;
        a.dimming = defaults.dimming.get() as f32;
        a.saturation = defaults.saturation.get() as f32;
        a.scrim = defaults.scrim.get() as f32;
        a.ui.collection_opacity = Appearance::default().collection_opacity;
        set_sliders(
            a,
            &[
                (Field::CollectionOpacity, a.ui.collection_opacity),
                (Field::Blur, a.blur),
                (Field::Dimming, a.dimming),
                (Field::Saturation, a.saturation),
                (Field::Scrim, a.scrim),
            ],
            window,
            cx,
        );
        schedule_preview(d, cx);
        apply_interface(d, true, cx);
        save_background(d, cx);
    }));
    let reset_panels = button("ui-reset", ButtonKind::Glass, tr!("Reset panels")).on_click(cx.listener(|d, _, window, cx| {
        let defaults = Appearance::default();
        let a = &mut d.appearance;
        a.ui.interface_mode = defaults.interface_mode;
        a.ui.interface_opacity = defaults.interface_opacity;
        a.ui.interface_blur = defaults.interface_blur;
        a.ui.solid_tint = defaults.solid_tint;
        a.ui.solid_tint_light = defaults.solid_tint_light;
        a.ui.solid_own = None;
        a.ui.solid_own_light = None;
        set_sliders(a, &[(Field::PanelOpacity, defaults.interface_opacity), (Field::PanelBlur, defaults.interface_blur)], window, cx);
        apply_interface(d, true, cx);
    }));
    let cover_rows = look::cover_rows(d, cx);
    let covers = card(
        [
            row(
                tr!("Cover aspect"),
                Some(tr!("Shape of the covers in the grid, the details panel and related games.")),
                div().w(px(200.)).child(aspect_select),
            )
            .into_any_element(),
            toggle_row("grey-uninstalled", tr!("Show uninstalled games in grey"), None, grey)
                .on_click(cx.listener(move |d, _, _, cx| {
                    let res = settings::save_grey_uninstalled(&d.ctx.data_paths, !grey);
                    d.saved(res, cx);
                }))
                .into_any_element(),
        ]
        .into_iter()
        .chain(cover_rows),
    );
    let profiles = profiles::profiles_group(d, cx);
    let colour = look::colour_group(d, cx);
    let reading = look::reading_group(d, cx);
    let layout = look::layout_group(d, f32::from(window.viewport_size().width), cx);
    let enabled = d.appearance.enabled;
    // A caption, then a card, then (optionally) a note and a reset button.
    let footer = |note: SharedString, reset: Stateful<Div>| {
        div()
            .flex()
            .items_center()
            .justify_between()
            .gap(px(12.))
            .mt(px(10.))
            .child(div().flex_1().pl(px(6.)).child(meta_text(note)))
            .child(reset)
    };
    // From the colours of everything, to the picture behind the Collection,
    // to the covers, the text and the layout.
    div()
        .flex()
        .flex_col()
        .child(caption(tr!("Profiles")))
        .child(profiles)
        .child(caption(tr!("Colour")))
        .child(colour)
        .child(footer(
            tr!("Panels are the menus, the ☰ menu, context menus, dropdowns, dialogs, the drawer, pages like this one and the command palette.")
                .into(),
            reset_panels,
        ))
        .child(caption(tr!("Behind the Collection")))
        .when(enabled, |d| d.child(preview).child(div().h(px(10.))))
        .child(background)
        .child(footer(tr!("The game's art, a screenshot or a picture of your own, and how much of it shows.").into(), reset))
        .child(caption(tr!("Covers")))
        .child(covers)
        .child(caption(tr!("Text and motion")))
        .child(reading)
        .child(caption(tr!("Layout")))
        .child(layout)
        .into_any_element()
}

fn img_el(img: Arc<RenderImage>) -> Img {
    gpui_kit::img(ImageSource::Render(img)).absolute().inset_0().size_full().object_fit(ObjectFit::Cover)
}

/// Console mode rows (the sliders become ◀ ▶ numbers).
pub(super) fn console_rows(d: &SettingsPage) -> Vec<crate::console::list::Row> {
    use crate::console::list::Row;
    let a = &d.appearance;
    let n = |v: f32| v.round() as i64;
    let mut rows = vec![
        Row::header(tr!("Collection background")),
        Row::toggle("bg-enabled", tr!("Game artwork behind the library"), a.enabled).desc(tr!("Desktop mode's Collection background")),
        Row::number("bg-opacity", tr!("Opacity"), n(a.ui.collection_opacity), 0, 100, 5, "%"),
        Row::number("bg-blur", tr!("Blur"), n(a.blur), 0, 40, 1, " px"),
        Row::number("bg-dim", if theme::theme_is_light() { tr!("Fading") } else { tr!("Darkening") }, n(a.dimming), 0, 90, 5, "%"),
        Row::number("bg-sat", tr!("Colour intensity"), n(a.saturation), 0, 150, 5, "%"),
        Row::number("bg-scrim", tr!("Edge shading"), n(a.scrim), 0, 150, 5, "%"),
        Row::action("bg-reset", tr!("Reset the background")),
        Row::header(tr!("Interface background")),
        Row::choice(
            "ui-mode",
            tr!("Panels"),
            vec![tr!("Transparent").into(), tr!("Solid").into()],
            if a.ui.interface_mode == InterfaceMode::Solid { 1 } else { 0 },
        ),
    ];
    match a.ui.interface_mode {
        InterfaceMode::Transparent => {
            rows.push(Row::number(
                "ui-opacity",
                tr!("Opacity"),
                n(a.ui.interface_opacity),
                INTERFACE_OPACITY_RANGE.0 as i64,
                INTERFACE_OPACITY_RANGE.1 as i64,
                2,
                "%",
            ));
            rows.push(Row::number(
                "ui-blur",
                tr!("Blur"),
                n(a.ui.interface_blur),
                INTERFACE_BLUR_RANGE.0 as i64,
                INTERFACE_BLUR_RANGE.1 as i64,
                INTERFACE_BLUR_STEP as i64,
                " px",
            ));
        }
        InterfaceMode::Solid => rows.extend(scheme::console_tint_rows(d)),
    }
    rows.push(Row::action("ui-reset", tr!("Reset the interface")));
    rows.push(Row::header(tr!("Covers")));
    let aspect = CoverAspect::parse(&d.settings.metadata.cover_aspect);
    let ai = ASPECTS.iter().position(|(x, _)| *x == aspect).unwrap_or(0);
    rows.push(Row::choice("aspect", tr!("Cover aspect"), ASPECTS.iter().map(|(_, l)| SharedString::from(tr(l))).collect(), ai));
    rows.push(Row::toggle("grey", tr!("Show uninstalled games in grey"), d.settings.grey_uninstalled_games));
    rows.extend(console_look_rows(d));
    rows
}

/// Labels of `items` and the index of `active` among them.
fn console_choice<T: Copy + PartialEq>(items: &[(T, &'static str)], active: T) -> (Vec<SharedString>, usize) {
    (items.iter().map(|(_, l)| SharedString::from(*l)).collect(), items.iter().position(|(v, _)| *v == active).unwrap_or(0))
}

/// The pictures console mode offers: not "Your own picture" (it needs a file picker).
fn console_pictures() -> Vec<(heronite_data::appearance::BackgroundSource, &'static str)> {
    look::picture_items().into_iter().filter(|(p, _)| *p != heronite_data::appearance::BackgroundSource::Custom).collect()
}

/// Console rows of `look.rs` and the profiles (the sections' order stays a desktop setting).
fn console_look_rows(d: &SettingsPage) -> Vec<crate::console::list::Row> {
    use crate::console::list::Row;
    use heronite_data::appearance::{presets, Accent};
    let a = d.appearance.ui;
    let accents: Vec<(Accent, &'static str)> = Accent::ALL.iter().map(|x| (*x, look::accent_label(*x))).collect();
    let choice = |id: &'static str, label: &'static str, (items, i): (Vec<SharedString>, usize)| Row::choice(id, label, items, i);
    let mut rows = vec![
        choice("cover-corners", tr!("Corners"), console_choice(&look::corner_items(), a.cover_corners)),
        choice("cover-label", tr!("Under the cover"), console_choice(&look::label_items(), a.cover_label)),
        Row::toggle("cover-badges", tr!("Favourite star"), a.cover_badges),
        choice("cover-hover", tr!("Under the pointer"), console_choice(&look::hover_items(), a.cover_hover)),
        choice("bg-picture", tr!("Picture"), console_choice(&console_pictures(), a.background)),
        Row::toggle("bg-slideshow", tr!("Change the screenshot by itself"), a.background_slideshow),
        Row::header(tr!("Colour")),
        choice("theme", tr!("Theme"), console_choice(&scheme::theme_items(), a.theme)).desc(tr!("Console mode stays dark")),
        choice("accent", tr!("Accent colour"), console_choice(&accents, a.accent)),
        Row::toggle("high-contrast", tr!("High contrast"), a.high_contrast),
        Row::header(tr!("Reading")),
        choice("font", tr!("Font"), console_choice(&look::font_items(), a.font)),
        choice("density", tr!("Density"), console_choice(&look::density_items(), a.density)),
        Row::header(tr!("Motion")),
        choice("motion", tr!("Animations"), console_choice(&look::motion_items(), a.motion)),
        Row::header(tr!("Layout")),
        choice("start-view", tr!("Open the Collection in"), console_choice(&look::start_items(), a.start_view)),
        Row::action("reset-widths", tr!("Reset widths")),
        Row::header(tr!("Profiles")),
    ];
    rows.extend(presets().iter().map(|p| Row::action(SharedString::from(format!("preset:{}", p.id)), profiles::preset_name(p.id))));
    rows
}

/// The value at `i` of `items`.
fn pick<T: Copy>(items: &[(T, &'static str)], i: Option<usize>) -> Option<T> {
    i.and_then(|i| items.get(i).map(|(v, _)| *v))
}

/// Applies a console row of [`console_look_rows`]; `false` when `id` is not one.
fn console_apply_look(
    d: &mut SettingsPage,
    id: &str,
    index: Option<usize>,
    on: Option<bool>,
    window: &mut Window,
    cx: &mut Context<SettingsPage>,
) -> bool {
    use heronite_data::appearance::{presets, Accent};
    if let Some(p) = id.strip_prefix("preset:").and_then(|p| presets().into_iter().find(|x| x.id == p)) {
        profiles::apply_look(d, p.look, window, cx);
        return true;
    }
    match (id, index, on) {
        ("cover-corners", i, _) => {
            if let Some(v) = pick(&look::corner_items(), i) {
                look::set_look(d, cx, |a| a.cover_corners = v)
            }
        }
        ("cover-label", i, _) => {
            if let Some(v) = pick(&look::label_items(), i) {
                look::set_look(d, cx, |a| a.cover_label = v)
            }
        }
        ("cover-hover", i, _) => {
            if let Some(v) = pick(&look::hover_items(), i) {
                look::set_look(d, cx, |a| a.cover_hover = v)
            }
        }
        ("bg-picture", i, _) => {
            if let Some(v) = pick(&console_pictures(), i) {
                look::set_look(d, cx, |a| a.background = v)
            }
        }
        ("theme", i, _) => {
            if let Some(v) = pick(&scheme::theme_items(), i) {
                look::set_look(d, cx, |a| a.theme = v)
            }
        }
        ("accent", i, _) => {
            if let Some(v) = i.and_then(|i| Accent::ALL.get(i).copied()) {
                look::set_look(d, cx, |a| {
                    a.accent = v;
                    a.accent_own = None;
                })
            }
        }
        ("font", i, _) => {
            if let Some(v) = pick(&look::font_items(), i) {
                look::set_look(d, cx, |a| a.font = v)
            }
        }
        ("density", i, _) => {
            if let Some(v) = pick(&look::density_items(), i) {
                look::set_look(d, cx, |a| a.density = v)
            }
        }
        ("motion", i, _) => {
            if let Some(v) = pick(&look::motion_items(), i) {
                look::set_look(d, cx, |a| a.motion = v)
            }
        }
        ("start-view", i, _) => {
            if let Some(v) = pick(&look::start_items(), i) {
                look::set_look(d, cx, |a| a.start_view = v)
            }
        }
        ("cover-badges", _, Some(on)) => look::set_look(d, cx, |a| a.cover_badges = on),
        ("bg-slideshow", _, Some(on)) => look::set_look(d, cx, |a| a.background_slideshow = on),
        ("high-contrast", _, Some(on)) => look::set_look(d, cx, |a| a.high_contrast = on),
        ("reset-widths", _, _) => cx.emit(crate::pages::PageEvent::ResetPanelWidths),
        _ => return false,
    }
    true
}

pub(super) fn console_apply(
    d: &mut SettingsPage,
    outcome: crate::console::list::Outcome,
    window: &mut Window,
    cx: &mut Context<SettingsPage>,
) {
    use crate::console::list::{Outcome, Value};
    let handled = match &outcome {
        Outcome::Changed(id, Value::Choice { index, .. }) => console_apply_look(d, id, Some(*index), None, window, cx),
        Outcome::Changed(id, Value::Toggle(on)) => console_apply_look(d, id, None, Some(*on), window, cx),
        Outcome::Activate(id) => console_apply_look(d, id, None, None, window, cx),
        _ => false,
    };
    if handled {
        return;
    }
    match outcome {
        Outcome::Changed(id, Value::Number { value, .. }) => {
            let v = value as f32;
            let a = &mut d.appearance;
            let field = match id.as_ref() {
                "bg-opacity" => {
                    a.ui.collection_opacity = v;
                    Field::CollectionOpacity
                }
                "bg-blur" => {
                    a.blur = v;
                    Field::Blur
                }
                "bg-dim" => {
                    a.dimming = v;
                    Field::Dimming
                }
                "bg-sat" => {
                    a.saturation = v;
                    Field::Saturation
                }
                "bg-scrim" => {
                    a.scrim = v;
                    Field::Scrim
                }
                "ui-opacity" => {
                    a.ui.interface_opacity = v;
                    Field::PanelOpacity
                }
                "ui-blur" => {
                    a.ui.interface_blur = v;
                    Field::PanelBlur
                }
                _ => return,
            };
            set_sliders(a, &[(field, v)], window, cx);
            if matches!(field, Field::CollectionOpacity | Field::PanelOpacity | Field::PanelBlur) {
                apply_interface(d, true, cx);
            } else {
                save_background(d, cx);
            }
        }
        Outcome::Changed(id, Value::Toggle(on)) => match id.as_ref() {
            "bg-enabled" => {
                d.appearance.enabled = on;
                save_background(d, cx);
            }
            "grey" => {
                let res = settings::save_grey_uninstalled(&d.ctx.data_paths, on);
                d.saved(res, cx);
            }
            _ => {}
        },
        Outcome::Changed(id, Value::Choice { index, .. }) => match id.as_ref() {
            "ui-mode" => {
                d.appearance.ui.interface_mode = if index == 1 { InterfaceMode::Solid } else { InterfaceMode::Transparent };
                apply_interface(d, true, cx);
            }
            "ui-tint" | "ui-tint-light" => {
                let tints = if id.as_ref() == "ui-tint" { SolidTint::DARK } else { SolidTint::LIGHT };
                if let Some(t) = tints.get(index) {
                    scheme::set_tint(d, *t, false, cx);
                }
            }
            "aspect" => {
                let mut patch = d.settings.metadata.clone();
                patch.cover_aspect = ASPECTS[index].0.as_str().to_string();
                let res = settings::save_metadata(&d.ctx.data_paths, &patch);
                d.saved(res, cx);
            }
            _ => {}
        },
        Outcome::Activate(id) => match id.as_ref() {
            "bg-reset" => {
                let defaults = heronite_data::models::BackgroundSettings::default();
                let a = &mut d.appearance;
                a.enabled = defaults.enabled;
                a.blur = defaults.blur.get() as f32;
                a.dimming = defaults.dimming.get() as f32;
                a.saturation = defaults.saturation.get() as f32;
                a.scrim = defaults.scrim.get() as f32;
                a.ui.collection_opacity = Appearance::default().collection_opacity;
                let values = [
                    (Field::CollectionOpacity, a.ui.collection_opacity),
                    (Field::Blur, a.blur),
                    (Field::Dimming, a.dimming),
                    (Field::Saturation, a.saturation),
                    (Field::Scrim, a.scrim),
                ];
                set_sliders(a, &values, window, cx);
                apply_interface(d, true, cx);
                save_background(d, cx);
            }
            "ui-reset" => {
                let defaults = Appearance::default();
                let a = &mut d.appearance;
                a.ui.interface_mode = defaults.interface_mode;
                a.ui.interface_opacity = defaults.interface_opacity;
                a.ui.interface_blur = defaults.interface_blur;
                a.ui.solid_tint = defaults.solid_tint;
                a.ui.solid_tint_light = defaults.solid_tint_light;
                a.ui.solid_own = None;
                a.ui.solid_own_light = None;
                set_sliders(
                    a,
                    &[(Field::PanelOpacity, defaults.interface_opacity), (Field::PanelBlur, defaults.interface_blur)],
                    window,
                    cx,
                );
                apply_interface(d, true, cx);
            }
            _ => {}
        },
        _ => {}
    }
}

/// Command palette toggles of this section (see `settings/palette.rs`):
/// from the page when it exists, else from what the Collection loaded.
pub(super) fn palette_commands(d: Option<&SettingsPage>, loaded: Option<&EffectiveSettings>) -> Vec<crate::pages::commands::PageCommand> {
    use crate::icons::Icon;
    use crate::pages::commands::PageCommand;
    let settings = d.map(|d| &d.settings).or(loaded);
    let ui = d.map(|d| d.appearance.ui).unwrap_or_else(theme::appearance);
    let mut v = Vec::new();
    if let Some(bg) = d.map(|d| d.appearance.enabled).or(settings.map(|s| s.background.enabled)) {
        v.push(
            if bg {
                PageCommand::new(
                    "bg",
                    tr!("Turn off the background artwork"),
                    tr!("The selected game's art behind the Collection"),
                    Icon::VisibilityOff,
                )
            } else {
                PageCommand::new(
                    "bg",
                    tr!("Use the game artwork as background"),
                    tr!("The selected game's art behind the Collection"),
                    Icon::Visibility,
                )
            }
            .keywords("appearance stage wallpaper picture backdrop aparencia aparência plano de fundo papel de parede imagem arte"),
        );
    }
    v.push(
        match ui.interface_mode {
            InterfaceMode::Transparent => {
                PageCommand::new("mode", tr!("Switch the interface to Solid"), tr!("Opaque panels in a solid colour"), Icon::Tune)
            }
            InterfaceMode::Solid => {
                PageCommand::new("mode", tr!("Switch the interface to Transparent"), tr!("Glass panels over the game artwork"), Icon::Tune)
            }
        }
        .keywords("appearance glass opaque panels menus dialogs theme aparencia aparência vidro opaco transparente sólido solido painéis paineis tema"),
    );
    v.extend(scheme::palette_commands(ui));
    if let Some(grey) = settings.map(|s| s.grey_uninstalled_games) {
        v.push(
            PageCommand::new(
                "grey",
                if grey { tr!("Show uninstalled games in colour") } else { tr!("Show uninstalled games in grey") },
                tr!("Covers of the Collection"),
                Icon::GridView,
            )
            .keywords("appearance covers greyscale gray aparencia aparência capas cinza"),
        );
    }
    v
}

/// Runs a toggle of [`palette_commands`]; `false` when `id` is not one.
pub(super) fn run_palette(d: &mut SettingsPage, id: &str, cx: &mut Context<SettingsPage>) -> bool {
    match id {
        "bg" => {
            d.appearance.enabled = !d.appearance.enabled;
            save_background(d, cx);
        }
        "mode" => {
            d.appearance.ui.interface_mode = match d.appearance.ui.interface_mode {
                InterfaceMode::Transparent => InterfaceMode::Solid,
                InterfaceMode::Solid => InterfaceMode::Transparent,
            };
            apply_interface(d, true, cx);
        }
        "grey" => {
            let res = settings::save_grey_uninstalled(&d.ctx.data_paths, !d.settings.grey_uninstalled_games);
            d.saved(res, cx);
        }
        _ => return scheme::run_palette(d, id, cx),
    }
    true
}
