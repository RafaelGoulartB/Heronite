//! `.collection__stage`: the focused game's hero art behind everything,
//! blurred and saturated (pre-processed on the CPU by `heronite-images`),
//! under three scrim gradients. CSS multi-stop gradients are split into
//! two-stop bands because GPUI gradients take exactly two stops.

use gpui_kit::prelude::*;
use gpui_kit::*;

/// `--collection-bg-dim` / `--collection-bg-scrim` (fractions, not %).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Scrim {
    pub dim: f32,
    pub scrim: f32,
}

const INK: (u8, u8, u8) = (7, 9, 12);

fn ink(a: f32) -> Hsla {
    Rgba { r: INK.0 as f32 / 255., g: INK.1 as f32 / 255., b: INK.2 as f32 / 255., a: a.clamp(0., 1.) }
        .into()
}

/// Bands for one multi-stop gradient: `(start%, end%, alpha_start, alpha_end)`.
fn bands(stops: &[(f32, f32)], scale: f32) -> Vec<(f32, f32, f32)> {
    stops
        .windows(2)
        .map(|w| ((w[1].0 - w[0].0) / 100., w[0].1 * scale, w[1].1 * scale))
        .collect()
}

fn gradient_row(angle: f32, stops: &[(f32, f32)], scale: f32) -> Div {
    let horizontal = angle == 90.;
    let mut layer = div().absolute().inset_0().flex();
    layer = if horizontal { layer.flex_row() } else { layer.flex_col() };
    for (frac, a0, a1) in bands(stops, scale) {
        let band = div()
            .flex_none()
            .bg(linear_gradient(angle, linear_color_stop(ink(a0), 0.), linear_color_stop(ink(a1), 1.)));
        let band = if horizontal {
            band.h_full().w(relative(frac))
        } else {
            band.w_full().h(relative(frac))
        };
        layer = layer.child(band);
    }
    layer
}

pub fn scrim_layers(s: Scrim) -> Div {
    div()
        .absolute()
        .inset_0()
        .child(div().absolute().inset_0().bg(ink(s.dim)))
        .child(gradient_row(
            90.,
            &[(0., 0.82), (18., 0.58), (38., 0.22), (52., 0.12), (100., 0.28)],
            s.scrim,
        ))
        .child(gradient_row(180., &[(0., 0.42), (28., 0.12), (62., 0.2), (100., 0.62)], s.scrim))
}

/// The stage. `art` is already blurred/saturated; `None` shows the plain page
/// colour like Heronite does when the background is disabled or missing.
pub fn stage(art: Option<ImageSource>, art_id: SharedString, s: Scrim) -> AnyElement {
    let Some(art) = art else {
        return div().absolute().inset_0().into_any_element();
    };
    div()
        .absolute()
        .inset_0()
        .child({
            let el = img(art).absolute().inset_0().size_full().object_fit(ObjectFit::Cover);
            // `img.loaded { transition: opacity 500ms }`
            if crate::theme::animations() {
                el.with_animation(
                    ElementId::Name(format!("stage-{art_id}").into()),
                    Animation::new(std::time::Duration::from_millis(500)),
                    |el, t| el.opacity(t),
                )
                .into_any_element()
            } else {
                el.into_any_element()
            }
        })
        .child(scrim_layers(s))
        .into_any_element()
}
