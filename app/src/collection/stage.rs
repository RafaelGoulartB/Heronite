//! `.collection__stage`: the focused game's hero art behind everything,
//! blurred, saturated and with its highlights rolled off (pre-processed on
//! the CPU by `heronite-images`, `Variant::Backdrop`), under three scrim
//! gradients. CSS multi-stop gradients are split into two-stop bands because
//! GPUI gradients take exactly two stops. On the light theme the scrims are
//! the light page colour and fade the art for dark text.

use gpui_kit::prelude::*;
use gpui_kit::*;

/// `--collection-bg-dim` / `--collection-bg-scrim` (fractions, not %).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Scrim {
    pub dim: f32,
    pub scrim: f32,
}

fn ink(a: f32) -> Hsla {
    crate::theme::stage_ink_a(a.clamp(0., 1.))
}

/// The even fade under everything on the light theme, at least
/// [`LIGHT_FADE_FLOOR`]: dark text keeps 4.5:1 even over black art (the
/// dark theme's floor is the art's rolled-off highlights).
fn base_fade(dim: f32) -> f32 {
    if crate::theme::is_light() {
        LIGHT_FADE_FLOOR + (1. - LIGHT_FADE_FLOOR) * dim
    } else {
        dim
    }
}

const LIGHT_FADE_FLOOR: f32 = 0.3;

/// Bands for one multi-stop gradient: `(start%, end%, alpha_start, alpha_end)`.
fn bands(stops: &[(f32, f32)], scale: f32) -> Vec<(f32, f32, f32)> {
    stops.windows(2).map(|w| ((w[1].0 - w[0].0) / 100., w[0].1 * scale, w[1].1 * scale)).collect()
}

fn gradient_row(angle: f32, stops: &[(f32, f32)], scale: f32) -> Div {
    let horizontal = angle == 90.;
    let mut layer = div().absolute().inset_0().flex();
    layer = if horizontal { layer.flex_row() } else { layer.flex_col() };
    for (frac, a0, a1) in bands(stops, scale) {
        let band = div().flex_none().bg(linear_gradient(angle, linear_color_stop(ink(a0), 0.), linear_color_stop(ink(a1), 1.)));
        let band = if horizontal { band.h_full().w(relative(frac)) } else { band.w_full().h(relative(frac)) };
        layer = layer.child(band);
    }
    layer
}

pub fn scrim_layers(s: Scrim) -> Div {
    div()
        .absolute()
        .inset_0()
        .child(div().absolute().inset_0().bg(ink(base_fade(s.dim))))
        // Dark under the list column, then an even floor under the details:
        // no lighter dip in the middle, where the overview sits on the art.
        .child(gradient_row(90., &[(0., 0.82), (18., 0.6), (32., 0.34), (100., 0.28)], s.scrim))
        .child(gradient_row(180., &[(0., 0.42), (24., 0.14), (62., 0.2), (100., 0.62)], s.scrim))
}

/// Settings → Appearance → Background "Solid colour": the panels' colour
/// under the same scrims.
pub fn solid(s: Scrim) -> AnyElement {
    let (r, g, b) = crate::theme::solid_tint().rgb();
    div().absolute().inset_0().bg(crate::theme::rgb_a(r, g, b, 1.)).child(scrim_layers(s)).into_any_element()
}

/// The stage. `art` is already blurred/saturated; `None` shows the plain page
/// colour like Heronite does when the background is disabled or missing.
/// `opacity` (0-1, "Collection background → Opacity") is how visible the art
/// is over the page colour; the scrims stay as they are.
pub fn stage(art: Option<ImageSource>, art_id: SharedString, s: Scrim, opacity: f32) -> AnyElement {
    let opacity = opacity.clamp(0., 1.);
    let Some(art) = art else {
        return div().absolute().inset_0().into_any_element();
    };
    div()
        .absolute()
        .inset_0()
        .child({
            let el = img(art).absolute().inset_0().size_full().object_fit(ObjectFit::Cover).opacity(opacity);
            // `img.loaded { transition: opacity 500ms }`
            if crate::theme::fades() {
                el.with_animation(
                    ElementId::Name(format!("stage-{art_id}").into()),
                    Animation::new(std::time::Duration::from_millis(500)),
                    move |el, t| el.opacity(t * opacity),
                )
                .into_any_element()
            } else {
                el.into_any_element()
            }
        })
        .child(scrim_layers(s))
        .into_any_element()
}
