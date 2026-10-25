//! A picture of a console theme: a small TV drawn from the theme's own
//! values (its first tab's layout, the covers' shape, size, corners and
//! focus, the background, the tab bar, the clock, the hint bar, the colours
//! and the font) with the covers and art of games of the library, so the
//! theme cards and the editor show what the console will look like and
//! follow every change at once. Without a library the covers are tinted
//! blocks.
//!
//! Everything is laid out on a 640 x 360 canvas scaled to the picture's
//! width; `layouts.rs` draws each layout.

mod layouts;

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::console_theme::{
    parse_hex, BackgroundSource, Clock, ConsoleFont, ConsoleTheme, Corners, FocusStyle, Glyphs, Hints, TabBar, TileShape, TileSize,
    TileTitles,
};
use heronite_data::Library;
use heronite_i18n::tr;
use heronite_images::ImageRef;
use std::collections::HashMap;

use crate::console::look::{ACCENTS, INK};
use crate::services::widgets::Covers;
use crate::theme;

/// The canvas the pictures are laid out on.
const CANVAS_W: f32 = 640.;
const CANVAS_H: f32 = 360.;
/// Textures are requested in steps of this many pixels, so pictures of
/// nearly the same size share them.
const TEXTURE_STEP: f32 = 24.;
/// How many games the pictures use.
const SAMPLES: usize = 12;

/// A game of the library whose art the pictures use.
#[derive(Clone)]
pub(super) struct Sample {
    key: SharedString,
    title: SharedString,
    cover: Vec<ImageRef>,
    art: Vec<ImageRef>,
}

/// The games the pictures show: the ones played last that have a cover,
/// then any with a cover.
pub(super) fn samples(
    lib: &Library,
    chains: &HashMap<SharedString, Vec<ImageRef>>,
    stages: &HashMap<SharedString, Vec<ImageRef>>,
) -> Vec<Sample> {
    let mut games: Vec<_> = lib
        .games
        .iter()
        .filter(|g| !g.hidden)
        .filter_map(|g| {
            let key = SharedString::from(g.key.clone());
            let cover = chains.get(&key).filter(|c| !c.is_empty())?.clone();
            let art = stages.get(&key).filter(|c| !c.is_empty()).cloned().unwrap_or_else(|| cover.clone());
            Some((g.last_played.clone().unwrap_or_default(), Sample { key, title: g.title.clone().into(), cover, art }))
        })
        .collect();
    // ISO dates: the latest sorts last.
    games.sort_by(|a, b| b.0.cmp(&a.0));
    games.into_iter().take(SAMPLES).map(|(_, s)| s).collect()
}

/// What a picture draws with.
pub(super) struct Art<'a> {
    pub samples: &'a [Sample],
    pub covers: Option<&'a mut Covers>,
    /// The window's scale factor.
    pub scale: f32,
}

/// Placeholder tints for covers when the library has no art.
const TINTS: [u32; 6] = [0x3d5a80, 0x7a4b63, 0x3f6f5e, 0x806a3d, 0x5b4a86, 0x3e6878];

/// The picture of `t`, `w` px wide (16:9), its corners rounded by `r` px.
pub(super) fn picture(t: &ConsoleTheme, w: f32, r: f32, art: &mut Art) -> Div {
    let mut p = Pic::new(t, w, art);
    p.frame_r = r;
    let h = w * CANVAS_H / CANVAS_W;
    let background = p.background();
    let body = layouts::content(&mut p);
    // `overflow_hidden` clips to a rectangle, not to the rounded corners:
    // the layers that reach the edges round their own.
    div()
        .relative()
        .flex_none()
        .w(px(w))
        .h(px(h))
        .overflow_hidden()
        .rounded(px(r))
        .bg(p.bg)
        .font_family(p.body_font)
        .child(background)
        .child(body)
        .child(p.top_bar())
        .children(p.bottom_bar())
        .children(p.hint_bar())
}

/// The theme's values, worked out once per picture.
pub(super) struct Pic<'a, 'b> {
    pub t: &'a ConsoleTheme,
    /// Canvas units to px.
    k: f32,
    /// The theme's interface size (1.0 = 100%).
    pub s: f32,
    art: &'a mut Art<'b>,
    pub bg: Rgba,
    pub text: Rgba,
    pub muted: Rgba,
    pub panel: Hsla,
    pub accent: Rgba,
    pub light: bool,
    pub head_font: &'static str,
    pub body_font: &'static str,
    n: usize,
    /// The frame's corner radius, px.
    frame_r: f32,
}

impl<'a, 'b> Pic<'a, 'b> {
    fn new(t: &'a ConsoleTheme, w: f32, art: &'a mut Art<'b>) -> Self {
        let colour = |s: &str| parse_hex(s).map(|(r, g, b)| theme::rgb_a(r, g, b, 1.));
        let light = t.colors.is_light();
        let bg = colour(&t.colors.background).unwrap_or(rgb(0x0b0e13));
        let text = colour(&t.colors.text).unwrap_or(if light { rgb(INK) } else { rgb(0xeef2f7) });
        let muted = colour(&t.colors.muted).unwrap_or(if light { rgb(0x58606e) } else { Rgba { a: 0.62, ..rgb(0xeef2f7) } });
        let panel = colour(&t.colors.panel).map(|c| Hsla::from(Rgba { a: 0.9, ..c })).unwrap_or(if light {
            theme::white_a(0.85)
        } else {
            theme::black_a(0.42)
        });
        let accent = ACCENTS
            .iter()
            .find(|(n, _)| *n == t.interface.accent)
            .map(|(_, c)| rgb(*c))
            .or_else(|| colour(&t.interface.accent))
            .unwrap_or(rgb(0xeef2f7));
        let (head_font, body_font) = match t.interface.font {
            ConsoleFont::Heronite => ("Rubik", "Cabin"),
            ConsoleFont::Cabin => ("Cabin", "Cabin"),
            ConsoleFont::Rubik => ("Rubik", "Rubik"),
            ConsoleFont::System => (".SystemUIFont", ".SystemUIFont"),
            ConsoleFont::Mono => (theme::FONT_MONO, theme::FONT_MONO),
        };
        let n = art.samples.len();
        Self {
            t,
            k: w / CANVAS_W,
            s: f32::from(t.interface.scale) / 100.,
            art,
            bg,
            text,
            muted,
            panel,
            accent: if light && matches!(t.interface.accent.as_str(), "white" | "game") { rgb(0x1f232a) } else { accent },
            light,
            head_font,
            body_font,
            n,
            frame_r: 0.,
        }
    }

    /// Canvas units to px.
    pub fn u(&self, v: f32) -> Pixels {
        px(v * self.k)
    }

    pub fn title(&self, i: usize) -> Option<SharedString> {
        (self.n > 0).then(|| self.art.samples[i % self.n].title.clone())
    }

    /// Shadow ink: black, or white on a light theme.
    pub fn shade(&self, a: f32) -> Hsla {
        if self.light {
            theme::white_a(a)
        } else {
            theme::black_a(a)
        }
    }

    /// Text in the theme's colour, `size` canvas units (times the interface size).
    pub fn label(&self, text: impl Into<SharedString>, size: f32, color: impl Into<Hsla>) -> Div {
        div()
            .text_size(self.u(size * self.s))
            .line_height(self.u(size * self.s * 1.25))
            .text_color(color.into())
            .truncate()
            .child(text.into())
    }

    /// A heading in the theme's heading font.
    pub fn heading(&self, text: impl Into<SharedString>, size: f32) -> Div {
        self.label(text, size, self.text).font_family(self.head_font).font_weight(theme::W_BOLD)
    }

    /// The name of game `i` as a heading, a bar when there is no library.
    pub fn game_name(&self, i: usize, size: f32, w: f32) -> Div {
        match self.title(i) {
            Some(t) => self.heading(t, size).max_w(self.u(w)),
            None => self.line(w * 0.7, size * 0.8, 0.85),
        }
    }

    /// A rounded bar standing for a line of text.
    pub fn line(&self, w: f32, h: f32, a: f32) -> Div {
        let c = if self.light { Rgba { a: a * 0.6, ..self.text } } else { Rgba { a: a * 0.55, ..self.text } };
        div().flex_none().w(self.u(w * self.s)).h(self.u(h * self.s)).rounded(self.u(h)).bg(c)
    }

    /// The corner radius of a cover `h` units tall.
    pub fn radius(&self, h: f32) -> f32 {
        match self.t.tiles.corners {
            Corners::Square => 1.5,
            Corners::Soft => (h * 0.05).clamp(2., 5.),
            Corners::Round => (h * 0.09).clamp(4., 12.),
        }
    }

    /// Width over height of the theme's covers.
    pub fn aspect(&self, shape: TileShape) -> f32 {
        match shape {
            TileShape::Portrait => 0.72,
            TileShape::Landscape => 1.78,
            TileShape::Square => 1.,
        }
    }

    /// The covers' size choice as a factor.
    pub fn size(&self) -> f32 {
        let f = match self.t.tiles.size {
            TileSize::Small => 0.84,
            TileSize::Medium => 1.,
            TileSize::Large => 1.16,
            TileSize::Huge => 1.32,
        };
        f * self.s
    }

    /// Requested sizes, rounded up to [`TEXTURE_STEP`] device pixels.
    fn texture_size(&self, w: f32, h: f32) -> (f32, f32) {
        let step = TEXTURE_STEP / self.art.scale;
        let up = |v: f32| ((v * self.k / step).ceil() * step).max(step);
        (up(w), up(h))
    }

    /// The cover (portrait, square) or wide art (landscape) of game `i`.
    fn image(&mut self, i: usize, wide: bool, w: f32, h: f32) -> Option<ImageSource> {
        if self.n == 0 {
            return None;
        }
        let (tw, th) = self.texture_size(w, h);
        let scale = self.art.scale;
        let s = &self.art.samples[i % self.n];
        let covers = self.art.covers.as_mut()?;
        if wide {
            covers.get_art(&s.key, &s.art, tw, th, scale)
        } else {
            covers.get_chain(&s.key, &s.cover, tw, th, scale)
        }
    }

    /// Game `i`'s picture filling a box `w` x `h` with corners `r`.
    pub fn picture_box(&mut self, i: usize, wide: bool, w: f32, h: f32, r: f32) -> Div {
        let src = self.image(i, wide, w, h);
        let tint = rgb(TINTS[i % TINTS.len()]);
        let fill = linear_gradient(
            150.,
            linear_color_stop(Rgba { a: 1., ..tint }, 0.),
            linear_color_stop(Rgba { a: 1., r: tint.r * 0.45, g: tint.g * 0.45, b: tint.b * 0.45 }, 1.),
        );
        div()
            .flex_none()
            .relative()
            .w(self.u(w))
            .h(self.u(h))
            .rounded(self.u(r))
            .overflow_hidden()
            .bg(fill)
            .children(src.map(|s| img(s).absolute().inset_0().size_full().rounded(self.u(r)).object_fit(ObjectFit::Cover)))
    }

    /// A cover of game `i`, `h` units tall in the theme's shape (or `shape`),
    /// with its focus and, as the theme says, its name.
    pub fn tile(&mut self, i: usize, h: f32, shape: Option<TileShape>, focused: bool) -> Div {
        let shape = shape.unwrap_or(self.t.tiles.shape);
        let w = h * self.aspect(shape);
        let r = self.radius(h);
        let mut cover = self.picture_box(i, shape == TileShape::Landscape, w, h, r);
        let titles = self.t.tiles.titles;
        if titles == TileTitles::Bar {
            let name = self.title(i);
            let strip = div().absolute().left_0().right_0().bottom_0().px(self.u(4.)).py(self.u(2.5)).bg(self.panel).child(match name {
                Some(n) => self.label(n, 7., self.text),
                None => self.line(w * 0.6, 4., 0.7),
            });
            cover = cover.child(strip);
        }
        let mut frame = div().flex_none().relative().child(cover);
        if focused {
            let a = self.accent;
            frame = match self.t.tiles.focus {
                FocusStyle::Ring => frame.child(
                    div()
                        .absolute()
                        .top(self.u(-3.5))
                        .left(self.u(-3.5))
                        .w(self.u(w + 7.))
                        .h(self.u(h + 7.))
                        .rounded(self.u(r + 3.5))
                        .border(self.u(1.6).max(px(1.)))
                        .border_color(a),
                ),
                FocusStyle::Glow => frame.rounded(self.u(r)).shadow(vec![BoxShadow {
                    color: Hsla { a: 0.7, ..Hsla::from(a) },
                    offset: point(px(0.), px(0.)),
                    blur_radius: self.u(16.),
                    spread_radius: self.u(1.),
                    inset: false,
                }]),
                FocusStyle::Lift => frame.mt(self.u(-7.)).mb(self.u(7.)).rounded(self.u(r)).shadow(vec![BoxShadow {
                    color: theme::black_a(0.6),
                    offset: point(px(0.), self.u(6.)),
                    blur_radius: self.u(12.),
                    spread_radius: px(0.),
                    inset: false,
                }]),
                FocusStyle::Underline => frame.child(
                    div().absolute().left(self.u(w * 0.2)).bottom(self.u(-6.)).w(self.u(w * 0.6)).h(self.u(2.5)).rounded(self.u(2.)).bg(a),
                ),
            };
        }
        let named = matches!((titles, focused), (TileTitles::Always, _) | (TileTitles::Focused, true));
        let mut col = div().flex_none().flex().flex_col().gap(self.u(5.)).w(self.u(w)).child(frame);
        if named {
            col = col.child(match self.title(i) {
                Some(n) => self.label(n, 8., if focused { self.text } else { self.muted }),
                None => self.line(w * 0.7, 4., 0.6),
            });
        }
        col
    }

    /// A row of covers from `first`, the first one focused, starting `x`
    /// units in; it runs off the right edge like the console's.
    pub fn tile_row(&mut self, first: usize, h: f32, gap: f32, count: usize, focus: bool) -> Div {
        let mut row = div().flex().items_start().gap(self.u(gap));
        for j in 0..count {
            row = row.child(self.tile(first + j, h, None, focus && j == 0));
        }
        row
    }

    /// The background: the art the theme asks for, its blur and darkening.
    fn background(&mut self) -> Div {
        let bgd = &self.t.background;
        let (w, h) = (CANVAS_W, CANVAS_H);
        let r = px(self.frame_r);
        let mut layer = div().absolute().inset_0().rounded(r);
        let src = match bgd.source {
            BackgroundSource::Art | BackgroundSource::Screenshots if self.n > 0 => {
                if bgd.blur > 0 {
                    self.blurred(0, true, f32::from(bgd.blur))
                } else {
                    self.image(0, true, w, h)
                }
            }
            BackgroundSource::Cover if self.n > 0 => self.blurred(0, false, 40.),
            BackgroundSource::Image if !bgd.image.trim().is_empty() => {
                let (tw, th) = self.texture_size(w, h);
                let scale = self.art.scale;
                self.art.covers.as_mut().and_then(|c| c.get_file(std::path::Path::new(bgd.image.trim()), tw, th, scale))
            }
            _ => None,
        };
        layer = match (src, bgd.source) {
            (Some(s), _) => layer.child(img(s).absolute().inset_0().size_full().rounded(r).object_fit(ObjectFit::Cover)),
            (None, BackgroundSource::Solid) => layer,
            (None, BackgroundSource::GameColour) => {
                let tint = rgb(TINTS[0]);
                layer.bg(linear_gradient(
                    160.,
                    linear_color_stop(Rgba { a: 0.9, ..tint }, 0.),
                    linear_color_stop(Rgba { a: 0., ..tint }, 0.85),
                ))
            }
            // No art (no library, or still loading): a soft glow stands for it.
            (None, _) => layer.bg(linear_gradient(
                135.,
                linear_color_stop(theme::rgb_a(58, 74, 104, 1.), 0.),
                linear_color_stop(theme::rgb_a(14, 17, 24, 1.), 0.9),
            )),
        };
        if bgd.dim > 0 {
            layer = layer.child(div().absolute().inset_0().rounded(r).bg(self.shade(f32::from(bgd.dim) / 100.)));
        }
        layer
    }

    /// Game `i`'s art (or cover) blurred by `blur` console px.
    fn blurred(&mut self, i: usize, wide: bool, blur: f32) -> Option<ImageSource> {
        let s = &self.art.samples[i % self.n];
        let chain = if wide { &s.art } else { &s.cover };
        // The cache id carries the blur, so pictures with another blur do not share it.
        let key = format!("{}:{}:{}", s.key, if wide { "a" } else { "c" }, blur as u32);
        // Small: the blur hides the detail and the GPU scales it up.
        self.art.covers.as_mut()?.get_blurred(&key, chain, 160., 90., blur * 160. / 1280., 1.2)
    }

    /// A layer over the whole picture, with its corners.
    pub fn cover_all(&self) -> Div {
        div().absolute().inset_0().rounded(px(self.frame_r))
    }

    /// A scrim over the background on the side the layout puts its text.
    pub fn scrim_left(&self, a: f32) -> Div {
        self.cover_all().bg(linear_gradient(90., linear_color_stop(self.shade(a), 0.), linear_color_stop(self.shade(0.), 0.75)))
    }

    pub fn scrim_bottom(&self, a: f32) -> Div {
        self.cover_all().bg(linear_gradient(180., linear_color_stop(self.shade(0.), 0.35), linear_color_stop(self.shade(a), 1.)))
    }

    /// Where the content may start (under the top bar).
    pub fn top(&self) -> f32 {
        if self.t.interface.tab_bar == TabBar::Bottom {
            34.
        } else {
            48.
        }
    }

    /// Where the content must end (above the bottom bar or hint bar).
    pub fn bottom(&self) -> f32 {
        let bar = self.t.interface.tab_bar == TabBar::Bottom;
        let hints = self.t.controls.hints == Hints::Always;
        CANVAS_H
            - if bar {
                40.
            } else if hints {
                28.
            } else {
                12.
            }
    }

    fn tab_names(&self) -> Vec<SharedString> {
        self.t.tabs.iter().map(|t| SharedString::from(tr(&t.name).to_string())).collect()
    }

    /// A controller button: `text` in a small circle (a key on the keyboard).
    fn glyph(&self, text: &'static str, fill: Rgba) -> Div {
        let wide = text.chars().count() > 1;
        div()
            .flex_none()
            .h(self.u(11.))
            .min_w(self.u(11.))
            .when(wide, |d| d.px(self.u(3.)))
            .flex()
            .items_center()
            .justify_center()
            .rounded(self.u(if wide { 3. } else { 6. }))
            .bg(fill)
            .text_size(self.u(6.5))
            .text_color(theme::white_a(0.95))
            .font_weight(theme::W_BOLD)
            .child(text)
    }

    fn top_bar(&self) -> Div {
        let i = &self.t.interface;
        let names = self.tab_names();
        let mut left = div().flex().items_center().gap(self.u(10.));
        if !i.label.is_empty() {
            left = left.child(self.heading(i.label.clone(), 10.));
        }
        let grey = Rgba { a: 0.5, ..self.text };
        match i.tab_bar {
            TabBar::Top => {
                let mut tabs = div().flex().items_center().gap(self.u(4.)).child(self.glyph("LB", grey));
                for (n, name) in names.iter().enumerate() {
                    let on = n == 0;
                    tabs = tabs.child(
                        div()
                            .px(self.u(6.))
                            .py(self.u(2.))
                            .rounded(self.u(8.))
                            .when(on, |d| d.bg(Rgba { a: 0.16, ..self.text }))
                            .child(self.label(name.clone(), 9., if on { self.text } else { self.muted })),
                    );
                }
                left = left.child(tabs.child(self.glyph("RB", grey)));
            }
            TabBar::Names => {
                for (n, name) in names.iter().enumerate() {
                    let on = n == 0;
                    left = left.child(
                        self.label(name.clone(), 11., if on { self.text } else { self.muted }).when(on, |d| d.font_weight(theme::W_BOLD)),
                    );
                }
            }
            TabBar::Title | TabBar::Bottom => {}
        }
        let mut right = div().flex().items_center().gap(self.u(6.));
        if i.status_icons {
            right = right.child(div().size(self.u(7. * self.s)).rounded_full().border(self.u(1.)).border_color(self.muted)).child(
                div().w(self.u(10. * self.s)).h(self.u(5. * self.s)).rounded(self.u(1.5)).border(self.u(1.)).border_color(self.muted),
            );
        }
        if i.date && i.clock != Clock::Off {
            right = right.child(self.line(18., 4., 0.7));
        }
        match i.clock {
            Clock::H24 => right = right.child(self.label("21:30", 10., self.text)),
            Clock::H12 => right = right.child(self.label("9:30", 10., self.text)),
            Clock::Off => {}
        }
        let title = (i.tab_bar == TabBar::Title).then(|| {
            div()
                .absolute()
                .left_0()
                .right_0()
                .top(self.u(14.))
                .flex()
                .justify_center()
                .child(self.heading(names.first().cloned().unwrap_or_default(), 11.))
        });
        div()
            .absolute()
            .top_0()
            .left_0()
            .right_0()
            .child(div().flex().items_center().justify_between().px(self.u(22.)).pt(self.u(14.)).child(left).child(right))
            .children(title)
    }

    fn bottom_bar(&self) -> Option<Div> {
        (self.t.interface.tab_bar == TabBar::Bottom).then(|| {
            let mut bar = div().flex().items_center().justify_center().gap(self.u(20.));
            for (n, name) in self.tab_names().into_iter().enumerate() {
                let on = n == 0;
                bar = bar.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(self.u(4.))
                        .px(self.u(7.))
                        .py(self.u(3.))
                        .rounded(self.u(8.))
                        .when(on, |d| d.bg(Rgba { a: 0.16, ..self.text }))
                        .child(div().size(self.u(7.)).rounded(self.u(2.)).bg(if on { self.text } else { self.muted }))
                        .child(self.label(name, 8.5, if on { self.text } else { self.muted })),
                );
            }
            div()
                .absolute()
                .left_0()
                .right_0()
                .bottom_0()
                .h(self.u(32.))
                .flex()
                .items_center()
                .justify_center()
                .rounded_b(px(self.frame_r))
                .bg(self.panel)
                .child(bar)
        })
    }

    fn hint_bar(&self) -> Option<Div> {
        let c = &self.t.controls;
        if c.hints == Hints::Off {
            return None;
        }
        let (a, b): ((&'static str, Rgba), (&'static str, Rgba)) = match c.glyphs {
            Glyphs::Xbox | Glyphs::Auto => (("A", rgb(0x4f9f45)), ("B", rgb(0xc8453c))),
            Glyphs::PlayStation => (("\u{2715}", rgb(0x5a7bd6)), ("\u{25cb}", rgb(0xd0545f))),
            Glyphs::Nintendo => (("A", rgb(0x4b4f58)), ("B", rgb(0x4b4f58))),
            Glyphs::Deck => (("A", rgb(0x3a3f48)), ("B", rgb(0x3a3f48))),
            Glyphs::Keyboard => (("Enter", rgb(0x3a3f48)), ("Esc", rgb(0x3a3f48))),
        };
        let floating = c.hints == Hints::Auto;
        let bottom = if self.t.interface.tab_bar == TabBar::Bottom { 38. } else { 9. };
        let hint = |g: (&'static str, Rgba), text: &'static str| {
            div().flex().items_center().gap(self.u(4.)).child(self.glyph(g.0, g.1)).child(self.label(text, 8., self.muted))
        };
        Some(
            div().absolute().right(self.u(18.)).bottom(self.u(bottom)).child(
                div()
                    .flex()
                    .items_center()
                    .gap(self.u(10.))
                    .when(floating, |d| d.px(self.u(8.)).py(self.u(4.)).rounded(self.u(10.)).bg(self.panel))
                    .child(hint(a, tr!("Select")))
                    .child(hint(b, tr!("Back"))),
            ),
        )
    }
}
