//! Colours: the `nord-dark` palette and its light counterpart, text, and
//! the fills of cards, buttons, tabs and inputs. A token with two values
//! gives the light one while [`is_light`]; the light text and status colours
//! keep 4.5:1 on every light panel colour, cards included. With the user's
//! own theme ([`custom_palette`]) the tokens come from its six colours:
//! surfaces and text as picked, lines and fills in its text colour, status
//! colours moved until they read on its panels.

use gpui_kit::{hsla, rgb, rgba, Hsla, Rgba};
use heronite_data::appearance::{contrast as contrast_of, mix, readable_on, Palette, Rgb};

use super::{accent_rgb, contrast_a, custom_palette, high_contrast, is_light};

/// The light theme's ink: its text, lines and faint fills are this colour,
/// as the dark theme's are white.
const INK_LIGHT: (u8, u8, u8) = (20, 26, 36);

// The light theme's surfaces are soft greys rather than white, so a big
// window of it does not glare: the panels (`SolidTint::Paper`, keep them
// equal), the page and the fade over the art a step darker, and what is
// lifted off a panel (popovers, the active tab, inputs) a step lighter.
pub(super) const LIGHT_PAPER: (u8, u8, u8) = (232, 234, 238);
pub(super) const LIGHT_PAGE: (u8, u8, u8) = (223, 227, 232);
pub(super) const LIGHT_LIFT: (u8, u8, u8) = (241, 243, 246);

/// `c` at alpha `a`.
pub(super) fn with_alpha((r, g, b): (u8, u8, u8), a: f32) -> Hsla {
    rgba_hsla(r, g, b, a)
}

/// `dark` on the dark theme, `light` on the light one.
fn pick(dark: u32, light: u32) -> Rgba {
    rgb(if is_light() { light } else { dark })
}

fn solid((r, g, b): Rgb) -> Rgba {
    rgb_a(r, g, b, 1.)
}

/// `custom` of the custom palette while it is drawn, else [`pick`].
fn cpick(dark: u32, light: u32, custom: impl FnOnce(&Palette) -> Rgb) -> Rgba {
    match custom_palette() {
        Some(p) => solid(custom(&p)),
        None => pick(dark, light),
    }
}

/// A status colour: [`pick`], moved until it reads on the custom panels.
fn status(dark: u32, light: u32) -> Rgba {
    let c = pick(dark, light);
    match custom_palette() {
        Some(p) => solid(readable_on(to_rgb8(c), &p.backgrounds(), p.light)),
        None => c,
    }
}

fn to_rgb8(c: Rgba) -> Rgb {
    let to8 = |x: f32| (x * 255.).round().clamp(0., 255.) as u8;
    (to8(c.r), to8(c.g), to8(c.b))
}

/// `rgba(255,255,255,a)`
pub fn white_a(a: f32) -> Hsla {
    hsla(0., 0., 1., a)
}

/// `rgba(0,0,0,a)`
pub fn black_a(a: f32) -> Hsla {
    hsla(0., 0., 0., a)
}

/// `rgb(r,g,b)` with alpha.
pub fn rgb_a(r: u8, g: u8, b: u8, a: f32) -> Rgba {
    Rgba { r: r as f32 / 255., g: g as f32 / 255., b: b as f32 / 255., a }
}

/// `rgba(r,g,b,a)` as `Hsla` (what `white_a` returns).
pub fn rgba_hsla(r: u8, g: u8, b: u8, a: f32) -> Hsla {
    rgb_a(r, g, b, a).into()
}

/// The interface's ink at alpha `a`: white on the dark theme, near black on
/// the light one. Faint fills, borders, hovers and lines of the surfaces;
/// what sits on a picture keeps [`white_a`].
pub fn ink_a(a: f32) -> Hsla {
    if let Some(p) = custom_palette() {
        return with_alpha(p.text, a);
    }
    if is_light() {
        rgba_hsla(INK_LIGHT.0, INK_LIGHT.1, INK_LIGHT.2, a)
    } else {
        white_a(a)
    }
}

/// A recess darker than its surface (track, well): black at `a` on the dark
/// theme, a quarter of that in ink on the light one.
pub fn recess_a(a: f32) -> Hsla {
    if is_light() {
        ink_a(a * 0.25)
    } else {
        black_a(a)
    }
}

/// Shadow at alpha `a`: black on the dark theme, softer ink on the light one.
pub fn shadow_a(a: f32) -> Hsla {
    if is_light() {
        rgba_hsla(INK_LIGHT.0, INK_LIGHT.1, INK_LIGHT.2, a * 0.4)
    } else {
        black_a(a)
    }
}

// Palette (`nord-dark` and its light counterpart).
pub fn page_bg() -> Rgba {
    if let Some(p) = custom_palette() {
        return solid(p.page);
    }
    if is_light() {
        rgb_a(LIGHT_PAGE.0, LIGHT_PAGE.1, LIGHT_PAGE.2, 1.)
    } else {
        rgb(0x07090c)
    }
}
/// The accent colour (`#a5dceb` unless Settings → Appearance changes it).
pub fn accent() -> Rgba {
    let (r, g, b) = accent_rgb();
    rgb_a(r, g, b, 1.)
}
/// The accent under the pointer: lighter on the dark theme (`#cbeef8` for
/// the default), darker on the light one.
pub fn accent_overlay() -> Rgba {
    let (r, g, b) = accent_rgb();
    if is_light() {
        let sink = |c: u8| (c as f32 * 0.82).round() as u8;
        return rgb_a(sink(r), sink(g), sink(b), 1.);
    }
    let lift = |c: u8| (c as f32 + (255. - c as f32) * 0.45).round() as u8;
    rgb_a(lift(r), lift(g), lift(b), 1.)
}

/// The darkest light background text sits on: Lilac under a card's 5% ink.
const LIGHT_FLOOR: (u8, u8, u8) = (220, 216, 226);

/// WCAG relative luminance.
fn luminance((r, g, b): (u8, u8, u8)) -> f32 {
    let lin = |c: u8| {
        let c = c as f32 / 255.;
        if c <= 0.03928 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * lin(r) + 0.7152 * lin(g) + 0.0722 * lin(b)
}

fn contrast(a: (u8, u8, u8), b: (u8, u8, u8)) -> f32 {
    let (x, y) = (luminance(a), luminance(b));
    (x.max(y) + 0.05) / (x.min(y) + 0.05)
}

/// `c` with the same hue, darkened until it reads as text on the light
/// theme (4.5:1 on its darkest card). For the accent taken from a cover.
pub(super) fn readable_on_light(c: (u8, u8, u8)) -> (u8, u8, u8) {
    let to8 = |v: f32| (v * 255.).round().clamp(0., 255.) as u8;
    let mut hsl: Hsla = rgb_a(c.0, c.1, c.2, 1.).into();
    let mut out = c;
    while contrast(out, LIGHT_FLOOR) < 4.6 && hsl.l > 0. {
        hsl.l = (hsl.l - 0.02).max(0.);
        let v = hsl.to_rgb();
        out = (to8(v.r), to8(v.g), to8(v.b));
    }
    out
}

/// `c` as text or an icon on the interface: unchanged on the dark theme,
/// darkened like [`readable_on_light`] on the light one (status colours,
/// a group's colour, gold stars).
pub fn readable(c: impl Into<Hsla>) -> Hsla {
    let c: Hsla = c.into();
    if let Some(p) = custom_palette() {
        let (r, g, b) = readable_on(to_rgb8(c.to_rgb()), &p.backgrounds(), p.light);
        return rgba_hsla(r, g, b, c.a);
    }
    if !is_light() {
        return c;
    }
    let v = c.to_rgb();
    let to8 = |x: f32| (x * 255.).round().clamp(0., 255.) as u8;
    let (r, g, b) = readable_on_light((to8(v.r), to8(v.g), to8(v.b)));
    rgba_hsla(r, g, b, c.a)
}

/// The accent at alpha `a`.
pub fn accent_alpha(a: f32) -> Rgba {
    let (r, g, b) = accent_rgb();
    rgb_a(r, g, b, a)
}

fn accent_a(a: f32) -> Hsla {
    accent_alpha(a).into()
}
pub fn navbar_background() -> Rgba {
    cpick(0x2e3440, 0xf1f3f6, |p| p.lift)
}
pub fn background() -> Rgba {
    cpick(0x191c22, 0xe8eaee, |p| p.panel)
}
pub fn success() -> Rgba {
    status(0x94f1ab, 0x1a6946)
}
pub fn danger() -> Rgba {
    status(0xbf616a, 0x9a3e48)
}
/// Something the user may want to do, not an error ("Restart Steam once").
pub fn warning() -> Rgba {
    status(0xf5cd5b, 0x745808)
}
pub fn selected_ring() -> Rgba {
    cpick(0xe8dcc0, 0x2a313b, |p| p.text)
}

/// A console theme's colour (`console::look`) while console mode is on, else `default`.
fn themed(pick: fn() -> Option<Rgba>, default: u32) -> Rgba {
    if crate::console::active() {
        pick().unwrap_or_else(|| rgb(default))
    } else {
        rgb(default)
    }
}

/// Console mode with a light console theme on (dark text, washed-out art).
pub fn console_light() -> bool {
    crate::console::active() && crate::console::look::light()
}

/// Console mode: dark surfaces, a light pill for the focused row,
/// controller button colours (a console theme may replace them).
pub fn console_bg() -> Rgba {
    themed(crate::console::look::background, 0x0b0e13)
}
pub fn console_sidebar() -> Rgba {
    themed(crate::console::look::panel, 0x0e1218)
}
/// The in-game overlay's panels: the console sidebar, nearly opaque over the game.
pub fn overlay_fill() -> Rgba {
    rgba(0x0e1218f0)
}
/// A level bar of the overlay's mixer on the light focus pill, and muted.
pub fn overlay_bar_focus() -> Rgba {
    rgb(0x1976d2)
}
pub fn overlay_bar_muted() -> Rgba {
    rgb(0x6b7480)
}
pub fn console_focus() -> Rgba {
    if crate::console::active() {
        crate::console::look::focus()
    } else {
        rgb(0xeef2f7)
    }
}
pub fn console_on_focus() -> Rgba {
    themed(crate::console::look::focus_text, 0x0d1117)
}
pub fn console_on() -> Rgba {
    themed(crate::console::look::on, 0x3dd68c)
}
pub fn console_btn_a() -> Rgba {
    rgb(0x3ecf8e)
}
pub fn console_btn_b() -> Rgba {
    rgb(0xf0727a)
}
pub fn console_btn_x() -> Rgba {
    rgb(0x6cb6ff)
}
pub fn console_btn_y() -> Rgba {
    rgb(0xf5cd5b)
}

/// Statistics charts: one colour per top game (most played first); the rest
/// share [`chart_other`]. Order and steps checked for colour-blind
/// separation on the dark glass (`#161b24`); keep them in this order.
pub fn chart_series(i: usize) -> Rgba {
    match i {
        0 => rgb(0x3987e5),
        1 => rgb(0xd95926),
        2 => rgb(0x199e70),
        3 => rgb(0xc98500),
        4 => rgb(0xd55181),
        _ => rgb(0x9085e9),
    }
}
pub fn chart_other() -> Hsla {
    ink_a(0.22)
}

/// Sequential blue ramp of the statistics heatmaps (`t` 0..=1, more = brighter).
pub fn chart_ramp(t: f32) -> Rgba {
    const RAMP: [u32; 10] = [0x0d366b, 0x104281, 0x184f95, 0x1c5cab, 0x256abf, 0x2a78d6, 0x3987e5, 0x5598e7, 0x6da7ec, 0x86b6ef];
    let i = (t.clamp(0., 1.) * (RAMP.len() - 1) as f32).round() as usize;
    rgb(RAMP[i])
}
/// Card fill under the river (also the line drawn between its bands).
pub fn river_gap() -> Rgba {
    rgb(0x121721)
}
/// Empty heatmap cell, and the free part of Storage's bars.
pub fn chart_empty() -> Hsla {
    ink_a(if is_light() { 0.08 } else { 0.05 })
}
/// Background of the Statistics page and the Year in review (the hero art
/// fades into it).
pub fn stats_page_bg() -> Rgba {
    rgb(0x07090c)
}

/// Status colours (`statusColors.ts`).
pub fn status_color(slug: &str) -> Rgba {
    match slug {
        "playing" => rgb(0x3ecf8e),
        "plan-to-play" => rgb(0x6cb6ff),
        "on-hold" => rgb(0xf0a202),
        "played" => rgb(0x8b9cff),
        "endless" => rgb(0x2ec4b6),
        "beaten" => rgb(0xf4d35e),
        "completed" => rgb(0xc084fc),
        "abandoned" => rgb(0x9aa0a6),
        "not-played" => rgb(0x6b7280),
        _ => rgb(0xe5e7eb),
    }
}

// Text.
pub fn text_primary() -> Rgba {
    if let Some(c) = crate::console::active().then(crate::console::look::text).flatten() {
        return c;
    }
    cpick(0xe8edf4, 0x20262f, |p| p.text)
}
/// Captions, leads, labels above fields (`#9aa6b6`).
pub fn text_muted() -> Rgba {
    if let Some(c) = crate::console::active().then(crate::console::look::muted).flatten() {
        return c;
    }
    if high_contrast() {
        cpick(0xc8d0dc, 0x333b46, |p| mix(p.muted, p.text, 0.5))
    } else {
        cpick(0x9aa6b6, 0x475262, |p| p.muted)
    }
}
/// Row descriptions (`#8c97a8`).
pub fn text_dim() -> Rgba {
    if console_light() {
        // A step under the muted text, still readable on the light panels.
        let mut c = text_muted();
        let bg = console_bg();
        let k = 0.18;
        (c.r, c.g, c.b) = (c.r + (bg.r - c.r) * k, c.g + (bg.g - c.g) * k, c.b + (bg.b - c.b) * k);
        return c;
    }
    if high_contrast() {
        cpick(0xbcc6d3, 0x3c434e, |p| mix(p.muted, p.text, 0.4))
    } else {
        cpick(0x8c97a8, 0x545d6d, |p| p.muted)
    }
}
/// Text a step above [`text_primary`]: active tabs and pills, titles over
/// the Collection (white on the dark theme).
pub fn text_bright() -> Hsla {
    if console_light() {
        return text_primary().into();
    }
    if let Some(p) = custom_palette() {
        return solid(p.text).into();
    }
    if is_light() {
        rgb(0x171c23).into()
    } else {
        white_a(1.)
    }
}
/// "Might not be writable", "Not enough disk space", a script's notes.
pub fn caution_text() -> Rgba {
    status(0xebcb8b, 0x745808)
}
/// Plain lines of a log.
pub fn code_text() -> Rgba {
    cpick(0xd8dee9, 0x222831, |p| p.text)
}
/// Text drawn on an accent fill.
pub fn on_accent() -> Rgba {
    if custom_palette().is_some() {
        // Whichever of near black and white reads better on the accent.
        let a = accent_rgb();
        let (dark, white) = ((12, 26, 32), (255, 255, 255));
        return solid(if contrast_of(a, dark) >= contrast_of(a, white) { dark } else { white });
    }
    pick(0x0c1a20, 0xffffff)
}

// Card (a group of rows) and its rows.
pub fn card_fill() -> Hsla {
    ink_a(0.05)
}
pub fn card_border() -> Hsla {
    ink_a(if high_contrast() { 0.16 } else { 0.07 })
}
pub fn row_separator() -> Hsla {
    ink_a(if high_contrast() { 0.14 } else { 0.06 })
}

pub fn toggle_off() -> Hsla {
    ink_a(0.16)
}

// Buttons.
pub fn button_fill() -> Hsla {
    ink_a(0.09)
}
pub fn button_fill_hover() -> Hsla {
    ink_a(0.14)
}
pub fn button_border() -> Hsla {
    ink_a(if high_contrast() { 0.22 } else { 0.10 })
}
pub fn button_text() -> Rgba {
    cpick(0xeef2f7, 0x222831, |p| p.text)
}
pub fn danger_fill() -> Hsla {
    rgba_hsla(191, 97, 106, 0.20)
}
pub fn danger_fill_hover() -> Hsla {
    rgba_hsla(191, 97, 106, 0.30)
}
pub fn danger_text() -> Rgba {
    status(0xf2a5ad, 0xb51a2b)
}

// Segmented tabs.
pub fn seg_track() -> Hsla {
    recess_a(0.28)
}
/// The active tab, lifted off the track (white on the light theme).
pub fn seg_active() -> Hsla {
    if let Some(p) = custom_palette() {
        return with_alpha(p.lift, 0.95);
    }
    if is_light() {
        with_alpha(LIGHT_LIFT, 0.95)
    } else {
        white_a(0.14)
    }
}
pub fn seg_text() -> Rgba {
    if high_contrast() {
        cpick(0xdde3eb, 0x252c35, |p| mix(p.muted, p.text, 0.7))
    } else {
        cpick(0xb5bfcc, 0x3a4453, |p| mix(p.muted, p.text, 0.35))
    }
}

// Inputs and selects: a dark well on the dark theme, a white field on the
// light one.
pub fn input_fill() -> Hsla {
    if is_light() {
        with_alpha(custom_palette().map_or(LIGHT_LIFT, |p| p.lift), 0.6)
    } else {
        black_a(0.25)
    }
}
pub fn input_border() -> Hsla {
    match (is_light(), high_contrast()) {
        (false, hc) => white_a(if hc { 0.2 } else { 0.08 }),
        (true, hc) => ink_a(if hc { 0.3 } else { 0.14 }),
    }
}
/// Border of a focused input: the accent at 60%.
pub fn input_border_focus() -> Hsla {
    let a = accent();
    Rgba { a: 0.6, ..a }.into()
}

// Side navigation pills and the accent tint of active items.
pub fn nav_text() -> Rgba {
    if high_contrast() {
        cpick(0xe2e7ee, 0x181d25, |p| p.text)
    } else {
        cpick(0xc3cbd8, 0x2b3341, |p| mix(p.muted, p.text, 0.6))
    }
}
pub fn accent_tint() -> Hsla {
    accent_a(0.16)
}
pub fn accent_tint_border() -> Hsla {
    accent_a(0.40)
}
/// Fill and border of the journal's "Where I left off" card.
pub fn accent_wash() -> Hsla {
    accent_a(0.06)
}
pub fn accent_wash_border() -> Hsla {
    accent_a(0.2)
}
pub fn menu_item_hover() -> Hsla {
    ink_a(0.07)
}

// Collection header: icons, search pill and filter chips stay faint over the art.
pub fn tool_icon() -> Hsla {
    ink_a(contrast_a(0.55))
}
pub fn search_fill() -> Hsla {
    ink_a(0.06)
}
pub fn search_fill_hover() -> Hsla {
    ink_a(0.09)
}
pub fn chip_fill() -> Hsla {
    ink_a(0.07)
}

/// The Collection's scrims over the art at alpha `a`: the dark page colour,
/// or the light one (the light theme fades the art instead of darkening it).
pub fn stage_ink_a(a: f32) -> Hsla {
    if let Some(p) = custom_palette() {
        return with_alpha(p.page, a);
    }
    if is_light() {
        with_alpha(LIGHT_PAGE, a)
    } else {
        rgba_hsla(7, 9, 12, a)
    }
}

/// The strongest pill: white on the dark theme, ink on the light one (Play,
/// the active half of the Focus / Grid switch).
pub fn solid_pill() -> Hsla {
    if let Some(p) = custom_palette() {
        return with_alpha(p.text, 0.88);
    }
    if is_light() {
        ink_a(0.85)
    } else {
        white_a(0.9)
    }
}
/// Text on a [`solid_pill`].
pub fn on_solid_pill() -> Rgba {
    cpick(0x0b0e13, 0xffffff, |p| p.panel)
}

/// The panels of "Getting started"'s picture of the Collection, Transparent.
pub fn preview_glass() -> Hsla {
    if let Some(p) = custom_palette() {
        return with_alpha(p.panel, 0.72);
    }
    if is_light() {
        with_alpha(LIGHT_PAPER, 0.72)
    } else {
        rgba_hsla(14, 18, 24, 0.62)
    }
}

/// An opaque card straight on the Collection (the welcome's load error).
pub fn opaque_card() -> Hsla {
    if let Some(p) = custom_palette() {
        return with_alpha(p.panel, 0.96);
    }
    if is_light() {
        with_alpha(LIGHT_PAPER, 0.96)
    } else {
        rgba_hsla(14, 18, 24, 0.96)
    }
}

/// A toast at the bottom of the Collection.
pub fn toast_fill() -> Hsla {
    if let Some(p) = custom_palette() {
        return with_alpha(p.lift, 0.96);
    }
    if is_light() {
        with_alpha(LIGHT_LIFT, 0.96)
    } else {
        rgba_hsla(18, 22, 28, 0.94)
    }
}

/// The title of a ☰ page.
pub fn page_title() -> Rgba {
    cpick(0xcaf3fd, 0x20262f, |p| p.text)
}

/// What the grid drawer's art fades into (its glass, nearly opaque).
pub fn drawer_fade(a: f32) -> Hsla {
    if let Some(p) = custom_palette() {
        return with_alpha(p.panel, a);
    }
    if is_light() {
        with_alpha(LIGHT_PAPER, a)
    } else {
        rgba_hsla(16, 20, 27, a)
    }
}

/// A button sitting on a picture (the grid drawer's close button).
pub fn art_button_fill() -> Hsla {
    if let Some(p) = custom_palette() {
        return with_alpha(p.lift, 0.85);
    }
    if is_light() {
        with_alpha(LIGHT_LIFT, 0.85)
    } else {
        black_a(0.45)
    }
}

// Focus panel: Play, the round tool buttons, the cards and the chips of
// the details.
/// The Play button: a light pill on the dark theme, an ink one on the light.
pub fn play_fill() -> Rgba {
    cpick(0xf3f5f7, 0x2a313b, |p| p.text)
}
pub fn play_fill_hover() -> Rgba {
    cpick(0xffffff, 0x20262f, |p| mix(p.text, if p.light { (0, 0, 0) } else { (255, 255, 255) }, 0.15))
}
pub fn play_text() -> Rgba {
    cpick(0x161616, 0xffffff, |p| p.panel)
}
/// The launch option after Play's label.
pub fn play_note() -> Rgba {
    cpick(0x5a5f66, 0xb4bcc8, |p| mix(p.panel, p.text, 0.35))
}
/// A card of the focus panel over the art (`dense`: the facts card).
pub fn focus_card_fill(dense: bool) -> Hsla {
    if let Some(p) = custom_palette() {
        return with_alpha(if p.light { p.lift } else { p.panel }, if dense { 0.6 } else { 0.5 });
    }
    match (is_light(), dense) {
        (false, true) => rgba_hsla(18, 22, 28, 0.45),
        (false, false) => rgba_hsla(12, 14, 18, 0.5),
        (true, true) => with_alpha(LIGHT_LIFT, 0.6),
        (true, false) => with_alpha(LIGHT_LIFT, 0.5),
    }
}
/// Fill of a round button at `a` (`rgba(16,18,22,a)`; white on the light theme).
pub fn round_fill(a: f32) -> Hsla {
    if let Some(p) = custom_palette() {
        return with_alpha(p.lift, a);
    }
    if is_light() {
        with_alpha(LIGHT_LIFT, a)
    } else {
        rgba_hsla(16, 18, 22, a)
    }
}
pub fn round_fill_hover() -> Hsla {
    if let Some(p) = custom_palette() {
        return with_alpha(mix(p.lift, p.text, 0.06), 0.95);
    }
    if is_light() {
        rgba_hsla(248, 249, 251, 0.95)
    } else {
        rgba_hsla(10, 12, 14, 0.9)
    }
}
pub fn round_icon() -> Rgba {
    cpick(0xf4f6f8, 0x20262f, |p| p.text)
}
pub fn focus_chip_fill() -> Hsla {
    if is_light() {
        ink_a(0.08)
    } else {
        black_a(0.48)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_cover_accent_is_darkened_for_the_light_theme() {
        // The light accents `collection/look.rs` measures (hsl(h, .75, .74)).
        for hue in [0., 40., 60., 120., 200., 280.] {
            let v = hsla(hue / 360., 0.75, 0.74, 1.).to_rgb();
            let to8 = |x: f32| (x * 255.).round() as u8;
            let c = (to8(v.r), to8(v.g), to8(v.b));
            let dark = readable_on_light(c);
            assert!(contrast(dark, LIGHT_FLOOR) >= 4.5, "hue {hue}: {dark:?}");
            let h = |c: (u8, u8, u8)| Hsla::from(rgb_a(c.0, c.1, c.2, 1.)).h;
            assert!((h(dark) - h(c)).abs() < 0.02, "hue {hue} drifted");
        }
        assert_eq!(readable_on_light((0x19, 0x64, 0x79)), (0x19, 0x64, 0x79));
    }
}
