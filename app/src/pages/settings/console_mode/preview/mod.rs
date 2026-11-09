//! Pictures of a console theme: a small TV drawn from the theme's own values
//! (a tab's layout, the covers' shape, size, corners, focus, badges and
//! store strip, the background, the tab bar, the clock, the hint bar, the
//! menu, the colours and the font) with the covers and art of games of the
//! library, so the theme cards and the editor show what the console will
//! look like and follow every change at once. A [`Screen`] picks what is
//! drawn: a tab, the game page or the open menu. Without a library the
//! covers are tinted blocks.
//!
//! Everything is laid out on a 640 x 360 canvas scaled to the picture's
//! width; `layouts.rs` draws each layout, `game_page.rs` the game pages and
//! `menu.rs` the menu; `tile.rs` draws the covers and `chrome.rs` the bars.

mod chrome;
mod game_page;
mod layouts;
mod menu;
mod tile;

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::console_theme::{
    parse_hex, BackgroundSource, ConsoleFont, ConsoleTheme, Corners, GamePage, Hints, Side, TabBar, TileShape, TileSize,
};
use heronite_data::Library;
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

/// What a picture shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Screen {
    /// Tab `n` (clamped to the theme's tabs), highlighted in the tab bar.
    Tab(usize),
    /// A game's page in the theme's `game_page` style.
    GamePage,
    /// The first tab dimmed with the ☰ menu open.
    Menu,
}

impl Screen {
    /// `HERONITE_DEBUG_CONSOLE_SCREEN`: `tab:<n>`, `game` or `menu`.
    fn parse(s: &str) -> Option<Self> {
        match s {
            "game" => Some(Screen::GamePage),
            "menu" => Some(Screen::Menu),
            _ => s.strip_prefix("tab:")?.parse().ok().map(Screen::Tab),
        }
    }
}

/// The picture of `t`'s first tab, `w` px wide (16:9), its corners rounded
/// by `r` px. `HERONITE_DEBUG_CONSOLE_SCREEN=tab:<n>|game|menu` draws that
/// screen instead (to look at every state of a theme).
pub(super) fn picture(t: &ConsoleTheme, w: f32, r: f32, art: &mut Art) -> Div {
    let which = crate::env::value("HERONITE_DEBUG_CONSOLE_SCREEN").and_then(|v| Screen::parse(&v)).unwrap_or(Screen::Tab(0));
    screen(t, which, w, r, art)
}

/// The picture of `which` screen of `t`, `w` px wide (16:9), its corners
/// rounded by `r` px.
pub(super) fn screen(t: &ConsoleTheme, which: Screen, w: f32, r: f32, art: &mut Art) -> Div {
    let mut p = Pic::new(t, w, art);
    p.frame_r = r;
    if let Screen::Tab(i) = which {
        p.tab = i.min(t.tabs.len().saturating_sub(1));
    }
    let h = w * CANVAS_H / CANVAS_W;
    let background = p.background();
    let page = which == Screen::GamePage;
    let body = if page { game_page::content(&mut p) } else { layouts::content(&mut p) };
    // The Sheet floats over a tab, which keeps its bars.
    let bars = !page || t.game_page == GamePage::Sheet;
    let (left, right) = p.inset;
    // The rail keeps its column: the bars and the content sit beside it.
    let mut stage = div().absolute().top_0().bottom_0().left(p.u(left)).right(p.u(right)).child(body);
    if bars {
        stage = stage.child(p.top_bar()).children(p.bottom_bar());
    }
    let overlay = match which {
        Screen::Menu => Some(menu::open(&mut p)),
        Screen::GamePage => game_page::sheet(&mut p),
        Screen::Tab(_) => None,
    };
    let rail = if which == Screen::Menu { None } else { menu::rail(&p) };
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
        .child(stage)
        .children(rail)
        .children(overlay)
        .children(p.hint_bar())
}

/// `e` with its top and/or bottom corners rounded by `r`.
fn round_sides<E: Styled>(e: E, top: bool, bottom: bool, r: Pixels) -> E {
    let e = if top { e.rounded_tl(r).rounded_tr(r) } else { e };
    if bottom {
        e.rounded_bl(r).rounded_br(r)
    } else {
        e
    }
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
    /// Panels over the art (the hint bar, cards, the stats).
    pub panel: Hsla,
    /// Solid surfaces: the menu, sheets, strips on covers.
    pub sidebar: Rgba,
    pub accent: Rgba,
    /// Text on the focus colour.
    pub focus_text: Rgba,
    /// The "On" switches.
    pub on: Rgba,
    pub light: bool,
    pub head_font: &'static str,
    pub body_font: &'static str,
    n: usize,
    /// The frame's corner radius, px.
    frame_r: f32,
    /// The tab shown (an index in the theme's tabs).
    pub tab: usize,
    /// Canvas units the Rail takes at the left and at the right.
    pub inset: (f32, f32),
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
        let sidebar = colour(&t.colors.panel).unwrap_or(rgb(if light { 0xffffff } else { 0x0e1218 }));
        let name = t.interface.accent.as_str();
        // The focus colour of the game: the sample's tint, lifted like the console does.
        let game = {
            let tint = rgb(TINTS[0]);
            let (k, to) = if light { (0.55, 0.) } else { (0.45, 1.) };
            let lift = |c: f32| c * k + to * (1. - k);
            Rgba { r: lift(tint.r), g: lift(tint.g), b: lift(tint.b), a: 1. }
        };
        let accent = match name {
            "game" => game,
            "white" if light => rgb(0x1f232a),
            _ => ACCENTS.iter().find(|(n, _)| *n == name).map(|(_, c)| rgb(*c)).or_else(|| colour(name)).unwrap_or(rgb(0xeef2f7)),
        };
        // The dark focus of a light theme takes white text.
        let white_focus = light && matches!(name, "white" | "game");
        let (head_font, body_font) = match t.interface.font {
            ConsoleFont::Heronite => ("Rubik", "Cabin"),
            ConsoleFont::Cabin => ("Cabin", "Cabin"),
            ConsoleFont::Rubik => ("Rubik", "Rubik"),
            ConsoleFont::System => (".SystemUIFont", ".SystemUIFont"),
            ConsoleFont::Mono => (theme::FONT_MONO, theme::FONT_MONO),
        };
        let inset = match (t.menu.style == heronite_data::console_theme::MenuStyle::Rail, t.menu.side) {
            (true, Side::Left) => (menu::RAIL_W, 0.),
            (true, Side::Right) => (0., menu::RAIL_W),
            _ => (0., 0.),
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
            sidebar,
            accent,
            focus_text: colour(&t.colors.focus_text).unwrap_or(rgb(if white_focus { 0xffffff } else { 0x0d1117 })),
            on: colour(&t.colors.on).unwrap_or(rgb(0x3dd68c)),
            light,
            head_font,
            body_font,
            n,
            frame_r: 0.,
            tab: 0,
            inset,
        }
    }

    /// Canvas units to px.
    pub fn u(&self, v: f32) -> Pixels {
        px(v * self.k)
    }

    /// The width the content has, the rail's column left out.
    pub fn width(&self) -> f32 {
        CANVAS_W - self.inset.0 - self.inset.1
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

    /// What washes out the screen behind an overlay: black, or the light background.
    pub fn veil(&self, a: f32) -> Hsla {
        if self.light {
            Rgba { a, ..self.bg }.into()
        } else {
            theme::black_a(a)
        }
    }

    /// The text colour on `fill` (dark on a pale fill of a light theme).
    pub fn on_fill(&self, fill: Rgba) -> Rgba {
        if self.light && 0.2126 * fill.r + 0.7152 * fill.g + 0.0722 * fill.b > 0.6 {
            rgb(INK)
        } else {
            self.focus_text
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

    /// The covers' size choice as a factor (the interface size included).
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
        self.picture_part(i, wide, w, h, r, (true, true), false)
    }

    /// [`Self::picture_box`] rounding only the top and/or the bottom corners
    /// (a strip joins the other side), in greys when `grey`.
    pub fn picture_part(&mut self, i: usize, wide: bool, w: f32, h: f32, r: f32, sides: (bool, bool), grey: bool) -> Div {
        let src = self.image(i, wide, w, h);
        let tint = rgb(TINTS[i % TINTS.len()]);
        let tint = if grey {
            let l = 0.3 * tint.r + 0.59 * tint.g + 0.11 * tint.b;
            Rgba { r: l, g: l, b: l, a: 1. }
        } else {
            tint
        };
        let fill = linear_gradient(
            150.,
            linear_color_stop(Rgba { a: 1., ..tint }, 0.),
            linear_color_stop(Rgba { a: 1., r: tint.r * 0.45, g: tint.g * 0.45, b: tint.b * 0.45 }, 1.),
        );
        let r = self.u(r);
        let image = src.map(|s| {
            round_sides(img(s).grayscale(grey).absolute().inset_0().size_full().object_fit(ObjectFit::Cover), sides.0, sides.1, r)
        });
        round_sides(div().flex_none().relative().w(self.u(w)).h(self.u(h)).overflow_hidden().bg(fill), sides.0, sides.1, r).children(image)
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

    /// A layer over the whole picture, with its corners; inside the stage it
    /// reaches under the rail's column too, as the console's scrims do.
    pub fn cover_all(&self) -> Div {
        div().absolute().top_0().bottom_0().left(self.u(-self.inset.0)).right(self.u(-self.inset.1)).rounded(px(self.frame_r))
    }

    /// A layer over the whole picture, outside the stage.
    pub fn full(&self) -> Div {
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
}
