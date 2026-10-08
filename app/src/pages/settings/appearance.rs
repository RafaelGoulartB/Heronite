//! Settings → Appearance: two independent groups plus the covers.
//!
//! - Collection background (the selected game's art behind the Collection):
//!   on/off, blur, darkening, colour intensity, edge shading live in Heronite's
//!   `local_library/settings.json` (`background`, shared with Heronite); its
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
use heronite_data::appearance::{Appearance, InterfaceMode, SolidTint, INTERFACE_BLUR_RANGE, INTERFACE_BLUR_STEP, INTERFACE_OPACITY_RANGE};
use heronite_data::settings::{self, CoverAspect, EffectiveSettings, PartialBackground};
use heronite_images::{ImageRef, Opts};

use super::SettingsPage;
use crate::collection::stage::{scrim_layers, Scrim};
use crate::pages::PageCtx;
use crate::theme;
use crate::ui::controls::{button, caption, card, row, tabs, toggle_row, ButtonKind};
use crate::ui::glass::glass_menu_surface;
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
}

const ASPECTS: [(CoverAspect, &str); 6] = [
    (CoverAspect::Steam, "Steam (2:3)"),
    (CoverAspect::Igdb, "IGDB (3:4)"),
    (CoverAspect::Gog, "GOG (11:15)"),
    (CoverAspect::Square, "Square (1:1)"),
    (CoverAspect::Dvd, "DVD case (5:7)"),
    (CoverAspect::Banner, "Banner (16:9)"),
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
        ] {
            let state = cx.new(|_| SliderState::new().min(min).max(max).step(step).default_value(value as f32));
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
                }
                if matches!(field, Field::Blur | Field::Saturation) {
                    schedule_preview(d, cx);
                }
                if matches!(field, Field::CollectionOpacity | Field::PanelOpacity | Field::PanelBlur) {
                    apply_interface(d, release, cx);
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
            opts: Opts::new(ctx.heroic.images_cache_dir(), Some(ctx.app.thumbs_dir())),
            aspect_open: false,
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
        let decoded = cx
            .background_spawn(async move { heronite_images::load_stage(&chain, 776, 148, blur, sat, &opts) })
            .await;
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
    let res = settings::save_ui(&d.ctx.heroic, None, Some(&patch));
    d.saved(res, cx);
}

/// A slider row: label (with the live value) on the left, the slider on the right.
fn slider_row(label: String, state: &Entity<SliderState>) -> AnyElement {
    let slider = Slider::new(state).bg(theme::accent()).text_color(theme::accent());
    row(label, None, div().w(px(260.)).child(slider)).into_any_element()
}

fn set_sliders(a: &State, values: &[(Field, f32)], window: &mut Window, cx: &mut Context<SettingsPage>) {
    for (field, v) in values {
        if let Some((_, state)) = a.sliders.iter().find(|(f, _)| f == field) {
            state.update(cx, |s, cx| s.set_value(*v, window, cx));
        }
    }
}

fn slider_of<'a>(a: &'a State, field: Field) -> &'a Entity<SliderState> {
    &a.sliders.iter().find(|(f, _)| *f == field).expect("slider created in State::new").1
}

/// Interface mode (Transparent / Solid), its controls and a sample menu.
fn interface_group(a: &State, cx: &mut Context<SettingsPage>) -> Div {
    let this = cx.entity();
    let mode = tabs("ui-mode", &[(InterfaceMode::Transparent, "Transparent"), (InterfaceMode::Solid, "Solid")], a.ui.interface_mode, {
        let this = this.clone();
        move |m, _, cx| {
            this.update(cx, |d, cx| {
                d.appearance.ui.interface_mode = m;
                apply_interface(d, true, cx);
            });
        }
    });
    let mut rows: Vec<AnyElement> = vec![row(
        "Panels",
        Some(match a.ui.interface_mode {
            InterfaceMode::Transparent => "Glass: the blurred game artwork shows through the panels.",
            InterfaceMode::Solid => "Opaque panels in a solid colour, no artwork.",
        }),
        mode,
    )
    .into_any_element()];
    match a.ui.interface_mode {
        InterfaceMode::Transparent => {
            rows.push(slider_row(format!("Opacity: {}%", a.ui.interface_opacity.round()), slider_of(a, Field::PanelOpacity)));
            rows.push(slider_row(format!("Blur: {} px", a.ui.interface_blur.round()), slider_of(a, Field::PanelBlur)));
        }
        InterfaceMode::Solid => {
            let items: Vec<(SolidTint, &'static str)> = SolidTint::ALL.iter().map(|t| (*t, t.label())).collect();
            let tint = tabs("ui-tint", &items, a.ui.solid_tint, move |t, _, cx| {
                this.update(cx, |d, cx| {
                    d.appearance.ui.solid_tint = t;
                    apply_interface(d, true, cx);
                });
            });
            rows.push(row("Colour", None, tint).into_any_element());
        }
    }
    let sample_item = |label: &'static str, selected: bool| {
        div()
            .px(px(10.))
            .py(px(6.))
            .rounded(px(theme::R_ITEM))
            .text_size(px(13.))
            .text_color(theme::text_primary())
            .when(selected, |d| d.bg(theme::menu_item_hover()))
            .child(label)
    };
    let sample = glass_menu_surface(theme::R_MENU)
        .flex()
        .flex_col()
        .gap(px(2.))
        .p(px(6.))
        .w(px(210.))
        .child(sample_item("Play", true))
        .child(sample_item("Add to favourites", false))
        .child(sample_item("Properties", false));
    let sample_row = div()
        .flex()
        .items_center()
        .justify_between()
        .gap(px(16.))
        .py(px(14.))
        .child(div().flex().flex_col().gap(px(1.)).child(div().text_size(px(14.)).text_color(theme::text_primary()).child("Preview")).child(meta_text("A menu with the current settings.")))
        .child(sample);
    rows.push(sample_row.into_any_element());
    card(rows)
}

pub fn render(d: &mut SettingsPage, _window: &mut Window, cx: &mut Context<SettingsPage>) -> AnyElement {
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
        .border_color(theme::white_a(0.08))
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
                .child(div().text_size(px(13.)).font_weight(theme::W_BOLD).text_color(white()).child("Live preview"))
                .child(
                    div()
                        .text_size(px(12.))
                        .text_color(theme::white_a(0.8))
                        .child("Titles, details and tags sit on top of this layer."),
                ),
        );

    let mut rows: Vec<AnyElement> = vec![toggle_row("bg-enabled", "Use the game artwork as page background", Some("Only behind the Collection list and the game details."), a.enabled)
        .on_click(cx.listener(|d, _, _, cx| {
            d.appearance.enabled = !d.appearance.enabled;
            save_background(d, cx);
        }))
        .into_any_element()];
    for (field, state) in &a.sliders {
        let label = match field {
            Field::CollectionOpacity => format!("Opacity: {}%", a.ui.collection_opacity.round()),
            Field::Blur => format!("Blur: {} px", a.blur.round()),
            Field::Dimming => format!("Darkening: {}%", a.dimming.round()),
            Field::Saturation => format!("Colour intensity: {}%", a.saturation.round()),
            Field::Scrim => format!("Edge shading: {}%", a.scrim.round()),
            Field::PanelOpacity | Field::PanelBlur => continue,
        };
        rows.push(slider_row(label, state));
    }
    let background = card(rows);

    let aspect = CoverAspect::parse(&d.settings.metadata.cover_aspect);
    let aspect_label = ASPECTS.iter().find(|(a, _)| *a == aspect).map(|(_, l)| *l).unwrap_or("Steam (2:3)");
    let aspect_options: Vec<(SharedString, bool)> = ASPECTS.iter().map(|(a, l)| ((*l).into(), *a == aspect)).collect();
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
            let res = settings::save_metadata(&d.ctx.heroic, &patch);
            d.saved(res, cx);
        },
    );

    let grey = d.settings.grey_uninstalled_games;
    let reset = button("bg-reset", ButtonKind::Glass, "Reset background").on_click(cx.listener(|d, _, window, cx| {
        let defaults = heronite_data::models::BackgroundSettings::default();
        let a = &mut d.appearance;
        a.enabled = defaults.enabled;
        a.blur = defaults.blur.get() as f32;
        a.dimming = defaults.dimming.get() as f32;
        a.saturation = defaults.saturation.get() as f32;
        a.scrim = defaults.scrim.get() as f32;
        a.ui.collection_opacity = Appearance::default().collection_opacity;
        set_sliders(a, &[
            (Field::CollectionOpacity, a.ui.collection_opacity),
            (Field::Blur, a.blur),
            (Field::Dimming, a.dimming),
            (Field::Saturation, a.saturation),
            (Field::Scrim, a.scrim),
        ], window, cx);
        schedule_preview(d, cx);
        apply_interface(d, true, cx);
        save_background(d, cx);
    }));
    let reset_interface = button("ui-reset", ButtonKind::Glass, "Reset interface").on_click(cx.listener(|d, _, window, cx| {
        let defaults = Appearance::default();
        let a = &mut d.appearance;
        a.ui.interface_mode = defaults.interface_mode;
        a.ui.interface_opacity = defaults.interface_opacity;
        a.ui.interface_blur = defaults.interface_blur;
        a.ui.solid_tint = defaults.solid_tint;
        set_sliders(a, &[(Field::PanelOpacity, defaults.interface_opacity), (Field::PanelBlur, defaults.interface_blur)], window, cx);
        apply_interface(d, true, cx);
    }));
    let interface = interface_group(a, cx);
    let covers = card([
        row("Cover aspect", Some("Shape of the covers in the grid, the details panel and related games."), div().w(px(200.)).child(aspect_select))
            .into_any_element(),
        toggle_row("grey-uninstalled", "Show uninstalled games in grey", None, grey)
            .on_click(cx.listener(move |d, _, _, cx| {
                let res = settings::save_grey_uninstalled(&d.ctx.heroic, !grey);
                d.saved(res, cx);
            }))
            .into_any_element(),
    ]);
    div()
        .flex()
        .flex_col()
        .child(caption("Collection background"))
        .child(preview)
        .child(div().h(px(10.)))
        .child(background)
        .child(div().flex().items_center().justify_between().gap(px(12.)).mt(px(10.)).child(
            div().flex_1().pl(px(6.)).child(meta_text("Edge shading darkens the left column and the top and bottom edges, where the game list and the toolbars sit.")),
        ).child(reset))
        .child(caption("Interface background"))
        .child(interface)
        .child(div().flex().items_center().justify_between().gap(px(12.)).mt(px(10.)).child(
            div().flex_1().pl(px(6.)).child(meta_text("Menus, the ☰ menu, context menus, dropdowns, dialogs, the drawer, pages like this one and the command palette.")),
        ).child(reset_interface))
        .child(caption("Covers"))
        .child(covers)
        .into_any_element()
}

fn img_el(img: Arc<RenderImage>) -> Img {
    gpui_kit::img(ImageSource::Render(img)).absolute().inset_0().size_full().object_fit(ObjectFit::Cover)
}
