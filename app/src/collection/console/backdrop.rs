//! The picture behind console mode, as the theme's `background` says: the
//! focused game's wide art, its cover blurred, one of its screenshots, a
//! gradient of its colour, a plain colour or the theme's own picture; then
//! the extra darkening and the shading the current screen wants ([`Scrim`]).
//! The art follows the focus after a short pause and cross-fades.

use std::borrow::Cow;
use std::sync::Arc;

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::console_theme::{BackgroundMotion, BackgroundSource};
use heronite_images::{ImageRef, Variant};

use super::super::CollectionView;
use crate::console::{look, motion};
use crate::theme;

/// Cross-fade of the background art.
pub(super) const ART_FADE_MS: u64 = 520;

/// How the screen shades the art so its text reads.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Scrim {
    /// Dark from the left and the bottom (text on the left, covers below).
    Classic,
    /// Lighter: the art shows most (Spotlight, Xbox).
    Spot,
    /// Dark at the bottom only (Cinema).
    Cinema,
    /// Evenly dark (grids and lists over busy art).
    Full,
    /// Nearly opaque (pages).
    Page,
}

/// The console background colour at alpha `a`.
fn backing(a: f32) -> Rgba {
    let mut c = theme::console_bg();
    c.a = a;
    c
}

/// The even wash a light theme puts over the art under each shading (the
/// background colour at this alpha), so dark text reads anywhere on it; the
/// shading then adds more where the text sits.
fn light_wash(scrim: Scrim) -> f32 {
    match scrim {
        Scrim::Classic => 0.6,
        Scrim::Spot => 0.5,
        Scrim::Cinema => 0.62,
        Scrim::Full => 0.68,
        Scrim::Page => 0.,
    }
}

fn scrim_layers(scrim: Scrim) -> Vec<Div> {
    let layer = |angle: f32, from: f32, to: f32, until: f32| {
        div().absolute().inset_0().bg(linear_gradient(angle, linear_color_stop(backing(from), 0.), linear_color_stop(backing(to), until)))
    };
    match scrim {
        Scrim::Classic => vec![layer(90., 0.96, 0.35, 1.), layer(0., 0.98, 0., 0.75)],
        Scrim::Spot => vec![layer(90., 0.88, 0.1, 0.7), layer(0., 0.92, 0., 0.5), layer(180., 0.6, 0., 0.25)],
        Scrim::Cinema => vec![layer(0., 0.97, 0., 0.62), layer(90., 0.55, 0., 0.5), layer(180., 0.45, 0., 0.16)],
        Scrim::Full => vec![layer(0., 0.86, 0.7, 1.)],
        Scrim::Page => vec![layer(90., 0.96, 0.96, 1.)],
    }
}

/// The size of the background texture: the window's, shrunk to 1920 wide at
/// most. Under the gradients a smaller texture looks the same and keeps the
/// covers in the cache budget (a full ultrawide frame is ~20 MB).
pub(super) fn backdrop_size(window: &Window) -> (u32, u32) {
    let scale = window.scale_factor();
    let size = window.viewport_size();
    let (w, h) = (f32::from(size.width) * scale, f32::from(size.height) * scale);
    let shrink = (1920. / w).min(1.);
    ((w * shrink) as u32, (h * shrink) as u32)
}

/// The texture id of the background `tag` ("art", "cover"...) of game
/// `owner` (the theme's own picture when `None`) at `size` and `blur`.
pub(super) fn backdrop_id(owner: Option<&str>, tag: &str, (pw, ph): (u32, u32), blur: f32) -> SharedString {
    match owner {
        Some(key) => format!("s:{key}:{tag}:{pw}x{ph}:{blur}:console"),
        None => format!("s:console-bg:{tag}:{pw}x{ph}:{blur}"),
    }
    .into()
}

/// Whether `id` is one of the textures of [`backdrop_id`].
pub(super) fn is_backdrop_id(id: &str) -> bool {
    id.starts_with("s:console-bg:") || (id.starts_with("s:") && id.ends_with(":console"))
}

/// Where the image of a background comes from (resolved only when it loads).
enum Pick {
    Stage,
    Cover,
    Own(ImageRef),
}

impl CollectionView {
    /// The texture behind the screen for `key`, sized to the window.
    fn backdrop_image(&mut self, key: &SharedString, window: &Window) -> Option<Arc<RenderImage>> {
        let theme = look::theme();
        let size = backdrop_size(window);
        let blur = f32::from(theme.background.blur);
        let plain = Variant::Stage { blur_px: blur, saturation: 1. };
        // What to load, how, under which tag, and the id when already built.
        let (pick, variant, tag, mut id): (Pick, Variant, Cow<'static, str>, Option<SharedString>) = match theme.background.source {
            BackgroundSource::Solid | BackgroundSource::GameColour => return None,
            BackgroundSource::Art => {
                let art_id = self.stage_chains.get(key).is_some_and(|c| !c.is_empty()).then(|| backdrop_id(Some(key), "art", size, blur));
                match art_id {
                    Some(id) if !self.images.is_failed(&id) => (Pick::Stage, plain, "art".into(), Some(id)),
                    // No wide art: the cover, blurred, rather than nothing.
                    _ => (Pick::Cover, Variant::Stage { blur_px: blur.max(30.), saturation: 1.2 }, "cover".into(), None),
                }
            }
            BackgroundSource::Cover => (Pick::Cover, Variant::Stage { blur_px: blur.max(28.), saturation: 1.3 }, "cover".into(), None),
            BackgroundSource::Screenshots => {
                let shot = (self.shots.game.as_ref() == Some(key)).then(|| self.shots.items.first().map(|s| s.path.clone())).flatten();
                match shot {
                    Some(p) => (Pick::Own(ImageRef::File(p.clone())), plain, format!("shot:{}", p.display()).into(), None),
                    None => (Pick::Stage, plain, "art".into(), None),
                }
            }
            BackgroundSource::Image => {
                let src = theme.background.image.trim();
                if src.is_empty() {
                    return None;
                }
                let r = if src.starts_with("http://") || src.starts_with("https://") {
                    ImageRef::Url(src.to_string())
                } else {
                    ImageRef::File(src.into())
                };
                (Pick::Own(r), plain, format!("img:{src}").into(), None)
            }
        };
        let owner = (theme.background.source != BackgroundSource::Image).then_some(key.as_ref());
        let id = id.take().unwrap_or_else(|| backdrop_id(owner, &tag, size, blur));
        let (stage, covers) = (&self.stage_chains, &self.cover_chains);
        let chain = || match pick {
            Pick::Stage => stage.get(key).cloned().unwrap_or_default(),
            Pick::Cover => covers.get(key).cloned().unwrap_or_default(),
            Pick::Own(r) => vec![r],
        };
        self.images.get_or_request(&id, chain, size.0, size.1, variant, 1000)
    }

    /// The art behind the screen and the one it fades from.
    pub(super) fn console_art(
        &mut self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> (Option<Arc<RenderImage>>, Option<(SharedString, Arc<RenderImage>)>) {
        let Some(key) = self.console.art_key.clone() else {
            return (self.console.art_prev.clone(), self.console.art_cur.clone());
        };
        if let Some(img) = self.backdrop_image(&key, window) {
            let same = self.console.art_cur.as_ref().is_some_and(|(_, i)| Arc::ptr_eq(i, &img));
            if !same {
                // Cross-fade: the old art stays under the new one until it
                // is fully in.
                self.console.art_prev = self.console.art_cur.take().map(|(_, i)| i);
                self.console.art_fade = Some(cx.spawn(async move |this, cx| {
                    cx.background_executor().timer(std::time::Duration::from_millis(ART_FADE_MS + 100)).await;
                    this.update(cx, |this, cx| {
                        this.console.art_prev = None;
                        cx.notify();
                    })
                    .ok();
                }));
            }
            self.console.art_cur = Some((key.clone(), img.clone()));
            self.shown_stage = Some((key, img));
        } else if matches!(look::theme().background.source, BackgroundSource::Solid | BackgroundSource::GameColour) {
            self.console.art_cur = None;
            self.console.art_prev = None;
        }
        (self.console.art_prev.clone(), self.console.art_cur.clone())
    }

    /// The whole background layer: art (or colour), darkening, shading.
    pub(super) fn render_console_backdrop(&mut self, scrim: Scrim, window: &Window, cx: &mut Context<Self>) -> Div {
        let theme = look::theme();
        let (art_prev, art) = self.console_art(window, cx);
        let drift = theme.background.motion == BackgroundMotion::Drift && motion::on();
        let art_img = move |i: Arc<RenderImage>| img(ImageSource::Render(i)).absolute().inset_0().size_full().object_fit(ObjectFit::Cover);
        let colour =
            (theme.background.source == BackgroundSource::GameColour).then_some(self.console.game_colour).flatten().map(|(r, g, b)| {
                div().absolute().inset_0().bg(linear_gradient(
                    200.,
                    linear_color_stop(rgba(u32::from(r) << 24 | u32::from(g) << 16 | u32::from(b) << 8 | 0xb0), 0.),
                    linear_color_stop(theme::console_bg(), 0.85),
                ))
            });
        let mut layer = div().absolute().inset_0().children(colour);
        if scrim != Scrim::Page {
            let current = art.map(|(k, i)| {
                let el = motion::fade_in(SharedString::from(format!("console-art-{k}-{}", look::epoch())), art_img(i), ART_FADE_MS);
                if drift {
                    // A slow pan and zoom, at a low frame rate (it runs as
                    // long as the screen shows).
                    div()
                        .absolute()
                        .inset_0()
                        .child(el)
                        .with_animation(
                            SharedString::from(format!("console-drift-{k}")),
                            Animation::new(std::time::Duration::from_secs(40)).repeat().with_max_fps(24.),
                            |d, t| {
                                let p = 1. - (2. * t - 1.).abs();
                                let z = px(60. * p);
                                d.top(-z).bottom(-z * 0.3).left(-z).right(-z * 0.3)
                            },
                        )
                        .into_any_element()
                } else {
                    el
                }
            });
            layer = layer.children(art_prev.map(art_img)).children(current);
        }
        let dim = f32::from(theme.background.dim) / 100.;
        let wash = if look::light() { light_wash(scrim) } else { 0. };
        layer
            .when(dim > 0., |d| d.child(div().absolute().inset_0().bg(backing(dim))))
            .when(wash > 0., |d| d.child(div().absolute().inset_0().bg(backing(wash))))
            .children(scrim_layers(scrim))
    }
}
