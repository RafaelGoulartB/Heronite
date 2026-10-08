//! Settings → Appearance: page background (live preview) and covers.

use std::sync::Arc;
use std::time::Duration;

use gpui_kit::component::slider::{Slider, SliderEvent, SliderState, SliderValue};
use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::settings::{self, CoverAspect, EffectiveSettings, PartialBackground};
use heronite_images::{ImageRef, Opts};

use super::SettingsPage;
use crate::collection::stage::{scrim_layers, Scrim};
use crate::pages::PageCtx;
use crate::theme;
use crate::ui::controls::{button, caption, card, row, toggle_row, ButtonKind};
use crate::ui::fields::{meta_text, select_field};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Field {
    Blur,
    Dimming,
    Saturation,
    Scrim,
}

pub struct State {
    enabled: bool,
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
        let mut sliders = Vec::new();
        for (field, max, value) in [
            (Field::Blur, 40., b.blur.get()),
            (Field::Dimming, 90., b.dimming.get()),
            (Field::Saturation, 150., b.saturation.get()),
            (Field::Scrim, 150., b.scrim.get()),
        ] {
            let state = cx.new(|_| SliderState::new().min(0.).max(max).step(1.).default_value(value as f32));
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
                }
                if matches!(field, Field::Blur | Field::Saturation) {
                    schedule_preview(d, cx);
                }
                if release {
                    save_background(d, cx);
                }
                cx.notify();
            })
            .detach();
            sliders.push((field, state));
        }
        let mut this = Self {
            enabled: b.enabled,
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
            el.child(img_el(img)).child(scrim_layers(scrim))
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

    let mut rows: Vec<AnyElement> = vec![toggle_row("bg-enabled", "Use the game artwork as page background", None, a.enabled)
        .on_click(cx.listener(|d, _, _, cx| {
            d.appearance.enabled = !d.appearance.enabled;
            save_background(d, cx);
        }))
        .into_any_element()];
    for (field, state) in &a.sliders {
        let label = match field {
            Field::Blur => format!("Blur: {} px", a.blur.round()),
            Field::Dimming => format!("Darkening: {}%", a.dimming.round()),
            Field::Saturation => format!("Colour intensity: {}%", a.saturation.round()),
            Field::Scrim => format!("Edge shading: {}%", a.scrim.round()),
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
        let values = [a.blur, a.dimming, a.saturation, a.scrim];
        for ((_, state), v) in a.sliders.clone().iter().zip(values) {
            state.update(cx, |s, cx| s.set_value(v, window, cx));
        }
        schedule_preview(d, cx);
        save_background(d, cx);
    }));
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
        .child(caption("Page background"))
        .child(preview)
        .child(div().h(px(10.)))
        .child(background)
        .child(div().flex().items_center().justify_between().gap(px(12.)).mt(px(10.)).child(
            div().flex_1().pl(px(6.)).child(meta_text("Edge shading darkens the left column and the top and bottom edges, where the game list and the toolbars sit.")),
        ).child(reset))
        .child(caption("Covers"))
        .child(covers)
        .into_any_element()
}

fn img_el(img: Arc<RenderImage>) -> Img {
    gpui_kit::img(ImageSource::Render(img)).absolute().inset_0().size_full().object_fit(ObjectFit::Cover)
}
