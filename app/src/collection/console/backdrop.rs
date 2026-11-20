//! The picture behind console mode, as the theme's `background` says: the
//! focused game's wide art, its cover blurred, one of its screenshots, a
//! gradient of its colour, a plain colour or the theme's own picture; then
//! the extra darkening and the shading the current screen wants ([`Scrim`]).
//! The art follows the focus after a short pause ([`ART_SETTLE`], so a held
//! stick does not load every game it passes) and cross-fades ([`ArtLayers`]).

use std::borrow::Cow;
use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::console_theme::{BackgroundMotion, BackgroundSource};
use heronite_images::{ImageRef, Variant};

use super::super::CollectionView;
use crate::console::{look, motion};
use crate::theme;

/// How long the art waits for the focus to rest before it loads.
pub(super) const ART_SETTLE: Duration = Duration::from_millis(150);
/// The cross-fade of the background art: the new art fades in over what
/// is on screen, which stays as it is.
const ART_FADE: Duration = Duration::from_millis(400);
/// A layer that shows less than this through the ones above it is dropped.
const SHOWS: f32 = 0.02;

/// Ease in and out, the curve (0.42, 0, 0.58, 1).
fn ease_in_out(t: f32) -> f32 {
    // Solve x(u) = t on the bezier, then y(u).
    let t = t.clamp(0., 1.);
    let bez = |u: f32, a: f32, b: f32| 3. * a * u * (1. - u) * (1. - u) + 3. * b * u * u * (1. - u) + u * u * u;
    let (mut lo, mut hi) = (0f32, 1f32);
    for _ in 0..24 {
        let mid = (lo + hi) / 2.;
        if bez(mid, 0.42, 0.58) < t {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    bez((lo + hi) / 2., 0., 1.)
}

/// One picture of the cross-fade.
struct Layer {
    key: SharedString,
    img: Arc<RenderImage>,
    /// Its opacity when it started fading in, and when (`None`: it stays at `from`).
    from: f32,
    start: Option<Instant>,
}

impl Layer {
    fn opacity(&self, now: Instant) -> f32 {
        match self.start {
            Some(at) => {
                let t = now.duration_since(at).as_secs_f32() / ART_FADE.as_secs_f32();
                self.from + (1. - self.from) * ease_in_out(t)
            }
            None => self.from,
        }
    }
}

/// The background's cross-fade, bottom to top: the newest art fades in over
/// the others, which keep the opacity they had (nothing under it dips to the
/// backdrop); a new game while a fade runs freezes it where it is and fades
/// in on top (a game still in the stack fades on from where it is). Layers
/// go once covered, so browsing fast keeps only a few.
#[derive(Default)]
pub(crate) struct ArtLayers {
    layers: Vec<Layer>,
}

impl ArtLayers {
    /// `img` is the art of `key` now: the same game's texture changing is no
    /// new picture (it is swapped in place); another game fades in on top.
    fn show(&mut self, key: &SharedString, img: Arc<RenderImage>, now: Instant, animate: bool) {
        if let Some(top) = self.layers.last_mut().filter(|l| l.key == *key) {
            top.img = img;
            return;
        }
        if !animate {
            self.layers = vec![Layer { key: key.clone(), img, from: 1., start: None }];
            return;
        }
        // Everything stops where it is.
        for l in &mut self.layers {
            l.from = l.opacity(now);
            l.start = None;
        }
        let from = match self.layers.iter().position(|l| l.key == *key) {
            Some(i) => self.layers.remove(i).from,
            None => 0.,
        };
        self.layers.push(Layer { key: key.clone(), img, from, start: Some(now) });
        self.prune(now);
    }

    /// Drops the layers that no longer show through the ones above.
    fn prune(&mut self, now: Instant) {
        let mut through = 1.;
        let mut keep = vec![false; self.layers.len()];
        for (i, l) in self.layers.iter().enumerate().rev() {
            keep[i] = through > SHOWS;
            through *= 1. - l.opacity(now).clamp(0., 1.);
        }
        let mut i = 0;
        self.layers.retain(|_| {
            i += 1;
            keep[i - 1]
        });
    }

    /// Whether the top layer is still fading in.
    fn moving(&self, now: Instant) -> bool {
        self.layers.last().is_some_and(|l| l.start.is_some_and(|at| now.duration_since(at) < ART_FADE))
    }

    fn clear(&mut self) {
        self.layers.clear();
    }
}

/// How the screen shades the art so its text reads.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Scrim {
    /// Dark from the left, the bottom and the top (text on the left, covers below).
    Classic,
    /// Classic's, denser where the grid of covers and its filters sit, so
    /// the art stays a colour behind them rather than a second picture.
    Library,
    /// Lighter: the art shows most (Xbox).
    Spot,
    /// A console's home (Spotlight): the art bright in the middle and to the
    /// right, darker along the top (the tabs and the row of icons), the left
    /// (the logo and Play) and the bottom (the cards).
    Home,
    /// Dark at the bottom only (Cinema).
    Cinema,
    /// A vignette: the art glows behind the covers in the middle, darker
    /// at the edges and most under the name at the bottom (Carousel).
    Carousel,
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
        Scrim::Library => 0.66,
        Scrim::Spot => 0.5,
        Scrim::Home => 0.52,
        Scrim::Cinema => 0.62,
        Scrim::Carousel => 0.56,
        Scrim::Full => 0.68,
        Scrim::Page => 0.,
    }
}

fn scrim_layers(scrim: Scrim) -> Vec<Div> {
    let layer = |angle: f32, from: f32, to: f32, until: f32| {
        div().absolute().inset_0().bg(linear_gradient(angle, linear_color_stop(backing(from), 0.), linear_color_stop(backing(to), until)))
    };
    match scrim {
        // The top too, where the name and the overview sit over bright art.
        Scrim::Classic => vec![layer(90., 0.96, 0.35, 1.), layer(0., 0.98, 0., 0.75), layer(180., 0.6, 0., 0.45)],
        Scrim::Library => vec![layer(90., 0.95, 0.58, 1.), layer(0., 0.97, 0.3, 0.85), layer(180., 0.55, 0., 0.3)],
        Scrim::Spot => vec![layer(90., 0.88, 0.1, 0.7), layer(0., 0.92, 0., 0.5), layer(180., 0.6, 0., 0.25)],
        Scrim::Home => vec![layer(180., 0.78, 0., 0.36), layer(90., 0.82, 0., 0.58), layer(0., 0.9, 0., 0.42)],
        Scrim::Cinema => vec![layer(0., 0.97, 0., 0.62), layer(90., 0.55, 0., 0.5), layer(180., 0.45, 0., 0.16)],
        Scrim::Carousel => vec![layer(0., 0.94, 0., 0.5), layer(180., 0.72, 0., 0.34), layer(90., 0.6, 0., 0.3), layer(270., 0.6, 0., 0.3)],
        Scrim::Full => vec![layer(0., 0.86, 0.7, 1.)],
        Scrim::Page => vec![layer(90., 0.96, 0.96, 1.)],
    }
}

/// The size of the background texture: the window's, shrunk to 1920 wide at
/// most. Under the gradients a smaller texture looks the same and keeps the
/// covers in the cache budget (a full ultrawide frame is ~20 MB).
pub(super) fn backdrop_size(window: &Window) -> (u32, u32) {
    let scale = window.scale_factor();
    let size = crate::ui::content_size(window);
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

    /// The frame of the background for `key` while it moves (Settings →
    /// Appearance → "Animated backgrounds", `live_stage.rs`): the game's art,
    /// or the theme's own picture, when that is a video or an animated
    /// picture. Its still stays [`Self::backdrop_image`], which the
    /// cross-fade between games follows.
    fn console_live(&mut self, key: &SharedString, window: &Window, cx: &mut Context<Self>) -> Option<Arc<RenderImage>> {
        // The console theme's motion too, as its Drift.
        if !motion::on() || motion::reduced() {
            return None;
        }
        let theme = look::theme();
        let size = backdrop_size(window);
        let blur = f32::from(theme.background.blur);
        let (owner, tag, chain): (Option<&str>, Cow<'static, str>, Vec<ImageRef>) = match theme.background.source {
            BackgroundSource::Art => (Some(key.as_ref()), "art".into(), self.stage_chains.get(key).cloned().unwrap_or_default()),
            BackgroundSource::Image => {
                let src = theme.background.image.trim();
                if src.is_empty() || src.starts_with("http://") || src.starts_with("https://") {
                    return None;
                }
                (None, format!("img:{src}").into(), vec![ImageRef::File(src.into())])
            }
            // Covers and screenshots are pictures; no picture, no motion.
            _ => return None,
        };
        let id = backdrop_id(owner, &tag, size, blur);
        // The art failed: the still is the blurred cover, which does not move.
        if self.images.is_failed(&id) {
            return None;
        }
        let picture = format!("console:{}:{tag}", owner.unwrap_or(""));
        let look = super::super::live_stage::StageLook { blur, saturation: 1., rolloff: 0. };
        match self.live_stage(&id, &picture, &chain, size, look, window, cx)?.1 {
            ImageSource::Render(frame) => Some(frame),
            _ => None,
        }
    }

    /// The layers of the art behind the screen, bottom to top, each with
    /// its opacity now. `drawn`: the screen shows the art (not a page over it).
    pub(super) fn console_art(
        &mut self,
        drawn: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<(SharedString, Arc<RenderImage>, f32)> {
        let now = Instant::now();
        let animate = motion::on();
        if let Some(key) = self.console.art_key.clone() {
            if let Some(img) = self.backdrop_image(&key, window) {
                self.console.art_layers.show(&key, img.clone(), now, animate);
                self.console.art_cur = Some((key.clone(), img.clone()));
                self.shown_stage = Some((key, img));
            } else if matches!(look::theme().background.source, BackgroundSource::Solid | BackgroundSource::GameColour) {
                self.console.art_cur = None;
                self.console.art_layers.clear();
            }
        }
        self.console.art_layers.prune(now);
        // Frames only while a fade runs.
        if self.console.art_layers.moving(now) {
            window.request_animation_frame();
        }
        // A background that moves: its frame drawn in place of the still of
        // the top layer once that is this game's (not the last game's while
        // the new one loads). Not while nothing shows it (a page, the idle
        // screen): it then stops.
        let key = self.console.art_key.clone();
        let live = match &key {
            Some(k) if drawn && !self.console_idle_shown() => self.console_live(k, window, cx),
            _ => None,
        };
        let top = self.console.art_layers.layers.len().saturating_sub(1);
        self.console
            .art_layers
            .layers
            .iter()
            .enumerate()
            .map(|(i, l)| {
                let img = match &live {
                    Some(frame) if i == top && Some(&l.key) == key.as_ref() => frame.clone(),
                    _ => l.img.clone(),
                };
                (l.key.clone(), img, l.opacity(now).clamp(0., 1.))
            })
            .collect()
    }

    /// The whole background layer: art (or colour), darkening, shading.
    pub(super) fn render_console_backdrop(&mut self, scrim: Scrim, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let theme = look::theme();
        let layers = self.console_art(scrim != Scrim::Page, window, cx);
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
        if scrim != Scrim::Page && !layers.is_empty() {
            let top_key = layers.last().map(|(k, ..)| k.clone()).unwrap_or_default();
            let stack = div().absolute().inset_0().children(layers.into_iter().map(|(_, i, o)| art_img(i).opacity(o)));
            let stack = if drift {
                // A slow pan and zoom, at a low frame rate (it runs as long
                // as the screen shows).
                stack
                    .with_animation(
                        SharedString::from(format!("console-drift-{top_key}")),
                        Animation::new(std::time::Duration::from_secs(40)).repeat().with_max_fps(24.),
                        |d, t| {
                            let p = 1. - (2. * t - 1.).abs();
                            let z = px(60. * p);
                            d.top(-z).bottom(-z * 0.3).left(-z).right(-z * 0.3)
                        },
                    )
                    .into_any_element()
            } else {
                stack.into_any_element()
            };
            layer = layer.child(stack);
        }
        let dim = f32::from(theme.background.dim) / 100.;
        let wash = if look::light() { light_wash(scrim) } else { 0. };
        layer
            .when(dim > 0., |d| d.child(div().absolute().inset_0().bg(backing(dim))))
            .when(wash > 0., |d| d.child(div().absolute().inset_0().bg(backing(wash))))
            .children(scrim_layers(scrim))
    }
}

#[cfg(test)]
mod tests {
    // Explicit imports: `use super::*` would bring in gpui's `test` macro and shadow `#[test]`.
    use super::{ease_in_out, ArtLayers, ART_FADE};
    use gpui_kit::{RenderImage, SharedString};
    use std::sync::Arc;
    use std::time::Instant;

    fn pic() -> Arc<RenderImage> {
        Arc::new(RenderImage::new(smallvec::smallvec![image::Frame::new(image::RgbaImage::new(1, 1))]))
    }

    fn keys(a: &ArtLayers) -> Vec<String> {
        a.layers.iter().map(|l| l.key.to_string()).collect()
    }

    #[test]
    fn the_curve_starts_and_ends_still() {
        assert!(ease_in_out(0.).abs() < 1e-3 && (ease_in_out(1.) - 1.).abs() < 1e-3);
        assert!(ease_in_out(0.1) < 0.1 && (ease_in_out(0.5) - 0.5).abs() < 0.01);
    }

    #[test]
    fn a_new_game_fades_in_over_what_shows_which_stays_still() {
        let (a, b, c): (SharedString, SharedString, SharedString) = ("a".into(), "b".into(), "c".into());
        let t0 = Instant::now();
        let mut l = ArtLayers::default();
        l.show(&a, pic(), t0, true);
        // The same game's texture again: no new layer, no new fade.
        l.show(&a, pic(), t0 + ART_FADE / 4, true);
        assert_eq!(keys(&l), ["a"]);
        let at = t0 + ART_FADE * 2;
        l.prune(at);
        l.show(&b, pic(), at, true);
        assert_eq!(keys(&l), ["a", "b"]);
        // `a` under it stays fully there while `b` fades in.
        assert_eq!(l.layers[0].opacity(at + ART_FADE / 2), 1.);
        // Interrupted halfway: `b` freezes where it is, `c` comes on top.
        let mid = at + ART_FADE / 2;
        l.show(&c, pic(), mid, true);
        assert_eq!(keys(&l), ["a", "b", "c"]);
        let frozen = l.layers[1].opacity(mid + ART_FADE);
        assert!(frozen > 0.3 && frozen < 0.7, "{frozen}");
        // Once `c` is in, what it covers goes.
        l.prune(mid + ART_FADE * 2);
        assert_eq!(keys(&l), ["c"]);
        // Without motion it jumps.
        l.show(&a, pic(), mid, false);
        assert_eq!(keys(&l), ["a"]);
        assert_eq!(l.layers[0].opacity(mid), 1.);
    }
}
