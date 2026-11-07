//! A game's categories on its cover (Settings → Appearance → Categories on
//! covers), drawn in one of the [`CategoryStyle`]s. The Collection's cards
//! and the style picker's previews draw them with the same code.
//!
//! The ribbon, the bookmarks and the folded corner are shapes GPUI's boxes
//! cannot draw (turned text, a notch, a triangle): each is an SVG made here,
//! rendered at the screen's scale so it stays sharp, and kept for reuse.

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::Arc;

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::appearance::CategoryStyle;

use crate::theme;

/// One category of a game, as its badge shows it.
#[derive(Clone, Debug, PartialEq)]
pub struct CategoryBadge {
    pub name: SharedString,
    pub color: Rgba,
}

impl CategoryBadge {
    /// `index`: the category's place among all of them (sorted by name),
    /// which picks its colour.
    pub fn new(name: &str, index: usize) -> Self {
        Self { name: name.to_string().into(), color: colour_of(index) }
    }
}

/// Six colours that tell each other apart on most art (the categorical
/// palette of Statistics, lighter where white text would not read).
const COLOURS: [u32; 6] = [0x9085e9, 0xf2c75c, 0x199e70, 0xd55181, 0x3987e5, 0xd95926];

/// The colour of the category at `index` among all of them: the first six
/// never share one.
pub fn colour_of(index: usize) -> Rgba {
    rgb(COLOURS[index % COLOURS.len()])
}

/// Text that reads on `bg`: near-black on light colours, white on the rest.
fn ink_on(bg: Rgba) -> Rgba {
    let lum = 0.2126 * bg.r + 0.7152 * bg.g + 0.0722 * bg.b;
    if lum > 0.55 {
        rgb(0x1a1407)
    } else {
        rgb(0xffffff)
    }
}

/// A short mark for a category where only a letter or two fit: a short name
/// whole ("VR", "RPG"), else the first letters of its first two words
/// ("Co-op" → "CO", "Kids games" → "KG"), else its first letter.
pub fn initials(name: &str) -> String {
    let name = name.trim();
    let words: Vec<&str> = name.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).collect();
    if words.len() == 1 && name.chars().count() <= 3 {
        return name.to_uppercase();
    }
    let take = if words.len() >= 2 { 2 } else { 1 };
    words.iter().take(take).filter_map(|w| w.chars().next()).flat_map(char::to_uppercase).collect()
}

/// The space the badges are drawn in. Sizes are design px, drawn `unit`
/// times bigger (console mode's scale; 1 on the desktop).
#[derive(Clone, Copy)]
pub struct Area {
    /// The cover's width and corner radius.
    pub w: f32,
    pub radius: f32,
    /// The window's scale factor (the SVG shapes' resolution).
    pub scale: f32,
    pub unit: f32,
    /// A play button sits at the bottom-right (the desktop's cards): the
    /// tags keep clear of it.
    pub play_room: bool,
}

impl Area {
    /// `v` design px on screen.
    fn u(&self, v: f32) -> Pixels {
        px(v * self.unit)
    }
}

/// Styles drawn in the cover's top-right corner, where the copies badge and
/// the selection's tick also go.
pub fn takes_top_right(style: CategoryStyle) -> bool {
    matches!(style, CategoryStyle::Ribbon | CategoryStyle::Bookmarks | CategoryStyle::Corner)
}

/// Everything a style draws over the cover, or `None` when the game has no
/// category. `hovered`: the play bar is up (the band makes room for it).
/// The circles go in the top-left row with the favourite star: a card puts
/// [`circles`] there instead; this draws them alone (the previews).
pub fn overlay(style: CategoryStyle, cats: &[CategoryBadge], a: Area, hovered: bool) -> Option<AnyElement> {
    if cats.is_empty() {
        return None;
    }
    Some(match style {
        CategoryStyle::Ribbon => ribbon(cats, a),
        CategoryStyle::Tags => tags(cats, a).into_any_element(),
        CategoryStyle::Band if hovered => return None,
        CategoryStyle::Band => band(cats, a).into_any_element(),
        CategoryStyle::Circles => {
            div().absolute().top(a.u(6.)).left(a.u(6.)).flex().gap(a.u(4.)).children(circles(cats, a)).into_any_element()
        }
        CategoryStyle::Bookmarks => bookmarks(cats, a),
        CategoryStyle::Corner => corner(cats, a),
    })
}

/// "+2" for the categories past the first `shown`.
fn more(cats: &[CategoryBadge], shown: usize) -> Option<String> {
    (cats.len() > shown).then(|| format!("+{}", cats.len() - shown))
}

/// Dark, see-through little box with white text: the copies and "Hidden"
/// badges' look, so the tags sit with them.
fn chip(a: Area) -> Div {
    div()
        .flex()
        .items_center()
        .gap(a.u(4.))
        .h(a.u(18.))
        .px(a.u(6.))
        .rounded(a.u(4.))
        .bg(theme::black_a(0.72))
        .text_size(a.u(10.5))
        .line_height(a.u(14.))
        .font_weight(theme::W_BOLD)
        .text_color(theme::white_a(0.92))
}

/// Tags at the bottom-left, clear of the play button on the right.
fn tags(cats: &[CategoryBadge], a: Area) -> Div {
    let shown = if a.w >= 160. && cats.len() <= 2 || a.w >= 200. { 2 } else { 1 };
    div()
        .absolute()
        .left(a.u(6.))
        .bottom(a.u(6.))
        .right(a.u(if a.play_room { 34. } else { 6. }))
        .flex()
        .gap(a.u(3.))
        .overflow_hidden()
        .children(cats.iter().take(shown).map(|c| {
            chip(a)
                .min_w_0()
                .flex_shrink(1.)
                .child(div().flex_none().size(a.u(6.)).rounded_full().bg(c.color))
                .child(div().min_w_0().truncate().child(c.name.clone()))
        }))
        .children(more(cats, shown).map(|m| chip(a).flex_none().text_color(theme::white_a(0.7)).child(m)))
}

/// A strip along the bottom, one part per category (three at most).
fn band(cats: &[CategoryBadge], a: Area) -> Div {
    let mut parts: Vec<(String, Rgba)> = if cats.len() <= 3 {
        cats.iter().map(|c| (c.name.to_uppercase(), c.color)).collect()
    } else {
        cats.iter().take(2).map(|c| (c.name.to_uppercase(), c.color)).collect()
    };
    if let Some(m) = more(cats, parts.len()) {
        parts.push((m, rgb(0x20242c)));
    }
    let last = parts.len() - 1;
    let r = a.u(a.radius);
    div().absolute().left_0().right_0().bottom_0().h(a.u(20.)).flex().children(parts.into_iter().enumerate().map(|(i, (text, bg))| {
        div()
            .flex_1()
            .min_w_0()
            .h_full()
            .flex()
            .items_center()
            .justify_center()
            .px(a.u(3.))
            .bg(bg)
            .when(i == 0, |d| d.rounded_bl(r))
            .when(i == last, |d| d.rounded_br(r))
            .when(i > 0, |d| d.border_l_1().border_color(theme::black_a(0.25)))
            .text_size(a.u(9.5))
            .font_weight(theme::W_BOLD)
            .text_color(ink_on(bg))
            .child(div().min_w_0().truncate().child(text))
    }))
}

/// Round badges with the categories' initials (three, then "+N").
pub fn circles(cats: &[CategoryBadge], a: Area) -> Vec<AnyElement> {
    let shown = if cats.len() <= 3 { cats.len() } else { 2 };
    let dot = || {
        div()
            .flex_none()
            .h(a.u(18.))
            .min_w(a.u(18.))
            .px(a.u(3.))
            .rounded_full()
            .flex()
            .items_center()
            .justify_center()
            .text_size(a.u(9.))
            .font_weight(theme::W_BOLD)
            .shadow(vec![BoxShadow {
                color: theme::black_a(0.45),
                offset: point(a.u(0.), a.u(1.)),
                blur_radius: a.u(4.),
                spread_radius: a.u(0.),
                inset: false,
            }])
    };
    cats.iter()
        .take(shown)
        .map(|c| dot().bg(c.color).text_color(ink_on(c.color)).child(initials(&c.name)).into_any_element())
        .chain(more(cats, shown).map(|m| dot().bg(theme::black_a(0.62)).text_color(theme::white_a(0.92)).child(m).into_any_element()))
        .collect()
}

/// A ribbon across the top-right corner with the first category's name.
fn ribbon(cats: &[CategoryBadge], a: Area) -> AnyElement {
    let first = &cats[0];
    let s = (a.w * 0.5).clamp(52., 84.);
    let (c, len, thick) = (s * 0.3, s * 1.7, (s * 0.24).clamp(13., 17.));
    let (cx, cy) = (s - c, c);
    let mut label: String = first.name.to_uppercase().chars().take(9).collect();
    if first.name.chars().count() > 9 {
        label.push('…');
    }
    if let Some(m) = more(cats, 1) {
        label = format!("{label} {m}");
    }
    let body = format!(
        "<g transform='rotate(45 {cx} {cy})'>\
         <rect x='{x}' y='{y2}' width='{len}' height='{thick}' fill='#000' opacity='.3'/>\
         <rect x='{x}' y='{y}' width='{len}' height='{thick}' fill='{bg}'/>\
         <text x='{cx}' y='{cy}' dy='.36em' text-anchor='middle' font-family='Rubik, Cabin, sans-serif' \
         font-weight='600' font-size='{fs}' letter-spacing='.6' fill='{fg}'>{text}</text></g>",
        x = cx - len / 2.,
        y = cy - thick / 2.,
        y2 = cy - thick / 2. + 1.5,
        bg = hex(first.color),
        fg = hex(ink_on(first.color)),
        fs = thick * 0.56,
        text = xml(&label),
    );
    svg_img(&body, s, s, a).absolute().top_0().right_0().into_any_element()
}

/// Bookmark ribbons hanging from the top edge, right to left (three, the
/// last a dark "+N" when there are more).
fn bookmarks(cats: &[CategoryBadge], a: Area) -> AnyElement {
    let shown = if cats.len() <= 3 { cats.len() } else { 2 };
    let (w, h) = (18., 28.);
    let mark = |text: &str, bg: Rgba, fg: Rgba| {
        let body = format!(
            "<path d='M0 0H{w}V{h}L{m} {notch}L0 {h}Z' fill='{bg}' stroke='#000' stroke-opacity='.25' stroke-width='.8'/>\
             <text x='{m}' y='10.5' dy='.36em' text-anchor='middle' font-family='Rubik, Cabin, sans-serif' \
             font-weight='600' font-size='{fs}' fill='{fg}'>{text}</text>",
            m = w / 2.,
            notch = h - 6.5,
            bg = hex(bg),
            fg = hex(fg),
            fs = if text.chars().count() > 2 { 7. } else { 8.5 },
            text = xml(text),
        );
        svg_img(&body, w, h, a).into_any_element()
    };
    div()
        .absolute()
        .top_0()
        .right(a.u(8.))
        .flex()
        .gap(a.u(3.))
        .children(cats.iter().take(shown).map(|c| mark(&initials(&c.name), c.color, ink_on(c.color))))
        .children(more(cats, shown).map(|m| mark(&m, rgb(0x20242c), rgb(0xffffff))))
        .into_any_element()
}

/// The top-right corner folded down, in the first category's colour, with
/// its initials (and "+N").
fn corner(cats: &[CategoryBadge], a: Area) -> AnyElement {
    let first = &cats[0];
    let s = (a.w * 0.32).clamp(36., 52.);
    let r = a.radius.min(s / 3.);
    let fg = hex(ink_on(first.color));
    let rest = more(cats, 1)
        .map(|m| {
            format!(
                "<text x='{x}' y='{y}' dy='.36em' text-anchor='middle' font-family='Rubik, Cabin, sans-serif' \
                 font-weight='600' font-size='{fs}' fill='{fg}' opacity='.85'>{m}</text>",
                x = s * 0.8,
                y = s * 0.56,
                fs = s * 0.17,
            )
        })
        .unwrap_or_default();
    let body = format!(
        "<path d='M0 0H{a}A{r} {r} 0 0 1 {s} {r}V{s}Z' fill='{bg}'/>\
         <path d='M0 0L{s} {s}' stroke='#000' stroke-opacity='.22' stroke-width='1.2'/>\
         <text x='{tx}' y='{ty}' dy='.36em' text-anchor='middle' font-family='Rubik, Cabin, sans-serif' \
         font-weight='600' font-size='{fs}' fill='{fg}'>{text}</text>{rest}",
        a = s - r,
        bg = hex(first.color),
        tx = s * 0.68,
        ty = s * 0.28,
        fs = s * 0.24,
        text = xml(&initials(&first.name)),
    );
    svg_img(&body, s, s, a).absolute().top_0().right_0().into_any_element()
}

fn hex(c: Rgba) -> String {
    let b = |v: f32| (v.clamp(0., 1.) * 255.).round() as u8;
    format!("#{:02x}{:02x}{:02x}", b(c.r), b(c.g), b(c.b))
}

/// `s` safe inside an SVG's text.
fn xml(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('\'', "&apos;").replace('"', "&quot;")
}

thread_local! {
    /// The SVGs made so far, by their source: a card drawn again reuses its
    /// picture (and GPUI its texture).
    static MADE: RefCell<HashMap<String, Arc<Image>>> = RefCell::new(HashMap::new());
}

/// An SVG of `body`, `w`×`h` design px, drawn in `a` and rasterized at its
/// size on screen (GPUI draws an SVG picture at its own pixel size).
fn svg_img(body: &str, w: f32, h: f32, a: Area) -> Img {
    let scale = a.scale.max(1.) * a.unit;
    let src = format!(
        "<svg xmlns='http://www.w3.org/2000/svg' width='{pw}' height='{ph}' viewBox='0 0 {w} {h}'>{body}</svg>",
        pw = (w * scale).round(),
        ph = (h * scale).round(),
    );
    let image = MADE.with(|m| {
        let mut m = m.borrow_mut();
        if m.len() > 512 {
            m.clear();
        }
        m.entry(src).or_insert_with_key(|src| Arc::new(Image::from_bytes(ImageFormat::Svg, src.clone().into_bytes()))).clone()
    });
    img(image).w(a.u(w)).h(a.u(h)).flex_none()
}

#[cfg(test)]
mod tests {
    use gpui_kit::rgb;

    use super::{colour_of, hex, initials, ink_on, xml};

    #[test]
    fn initials_keep_short_names_and_take_first_letters() {
        assert_eq!(initials("VR"), "VR");
        assert_eq!(initials("rpg"), "RPG");
        assert_eq!(initials("Demo"), "D");
        assert_eq!(initials("Co-op"), "CO");
        assert_eq!(initials("  kids games  "), "KG");
        assert_eq!(initials("Ação"), "A");
    }

    #[test]
    fn the_first_six_categories_get_six_colours() {
        let all: std::collections::HashSet<_> = (0..6).map(|i| hex(colour_of(i))).collect();
        assert_eq!(all.len(), 6);
        assert_eq!(hex(colour_of(6)), hex(colour_of(0)));
    }

    #[test]
    fn text_is_dark_on_light_colours() {
        assert_eq!(hex(ink_on(rgb(0xf2c75c))), "#1a1407");
        assert_eq!(hex(ink_on(rgb(0x199e70))), "#ffffff");
    }

    #[test]
    fn names_are_escaped_for_the_svg() {
        assert_eq!(xml("R&D <'x'>"), "R&amp;D &lt;&apos;x&apos;&gt;");
    }
}
