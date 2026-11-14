//! The drawn controller of Settings → Controller: an Xbox Series controller
//! (`controller.svg`, built from Zacksly's "Xbox Series Button Icons and
//! Controls", CC BY 3.0, without the Xbox logo; the triggers are ours) in
//! the theme's colours, the buttons the tab uses lit, its face buttons drawn
//! as the controller prints them (`look::pad_glyphs`), and a callout for
//! each button in a column on its side (Guide's on top) joined to the
//! button's rim by an elbow line. A binding that is off has a dashed callout
//! and line. A click on a button or a callout picks it here and in the
//! table.

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::console_theme::{AButton, ConsoleButton};
use heronite_data::overlay_prefs::ChordButton;
use heronite_i18n::tr;
use heronite_platform::{supported, Feature};

use super::{console_button, glyph, shares_guide, toggle_selected, Show};
use crate::pages::settings::{overlay, SettingsPage};
use crate::theme;

/// The drawing's own units: callouts and lines live in an 880 × 340 box,
/// scaled to the page's width.
const BOX_W: f32 = 880.;
const BOX_H: f32 = 340.;
/// The pad's own units (those of the drawing it comes from, 4096 wide) map
/// into the box at this scale, its top middle (2048.5, 482) at `ORIGIN`.
const PAD_SCALE: f32 = 0.236;
const PAD_TOP: (f32, f32) = (2048.5, 482.);
const ORIGIN: (f32, f32) = (440., 50.);
/// A callout's width and height, and the gap between two, in box units.
const CALLOUT_W: f32 = 212.;
const CALLOUT_H: f32 = 32.;
const CALLOUT_GAP: f32 = 6.;

/// A pad point in box units.
fn at_box(x: f32, y: f32) -> (f32, f32) {
    (ORIGIN.0 + (x - PAD_TOP.0) * PAD_SCALE, ORIGIN.1 + (y - PAD_TOP.1) * PAD_SCALE)
}

/// A part of the drawn controller.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Part {
    LT,
    RT,
    LB,
    RB,
    LStick,
    RStick,
    DPad,
    A,
    B,
    X,
    Y,
    View,
    Menu,
    Guide,
}

/// Where a callout goes.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Side {
    Left,
    Right,
    Top,
}

impl Part {
    const ALL: [Part; 14] = [
        Part::LT,
        Part::RT,
        Part::LB,
        Part::RB,
        Part::LStick,
        Part::RStick,
        Part::DPad,
        Part::A,
        Part::B,
        Part::X,
        Part::Y,
        Part::View,
        Part::Menu,
        Part::Guide,
    ];

    pub(super) fn of_chord(b: ChordButton) -> Part {
        match b {
            ChordButton::RB => Part::RB,
            ChordButton::LB => Part::LB,
            ChordButton::A => Part::A,
            ChordButton::B => Part::B,
            ChordButton::X => Part::X,
            ChordButton::Y => Part::Y,
            ChordButton::Left | ChordButton::Right => Part::DPad,
            ChordButton::Start => Part::Menu,
            ChordButton::Select => Part::View,
        }
    }

    /// Its centre in pad units.
    fn centre(self) -> (f32, f32) {
        match self {
            Part::LT => (1640., 475.),
            Part::RT => (2457., 475.),
            Part::LB => (1617., 548.),
            Part::RB => (2480., 548.),
            Part::LStick => (1602.26, 815.86),
            Part::RStick => (2275.88, 1073.),
            Part::DPad => (1816.41, 1091.48),
            Part::Y => (2490.79, 695.37),
            Part::A => (2490.79, 925.05),
            Part::X => (2375.95, 810.21),
            Part::B => (2605.63, 810.52),
            Part::View => (1921.91, 808.15),
            Part::Menu => (2173.03, 808.15),
            Part::Guide => (2048.5, 636.52),
        }
    }

    /// Its callout's side, where the line leaves the part (on its rim) and
    /// where it bends, in pad units. The face buttons' lines leave between
    /// their neighbours, so no line crosses another button.
    fn route(self) -> (Side, (f32, f32), (f32, f32)) {
        match self {
            Part::LT => (Side::Left, (1505., 520.), (1430., 470.)),
            Part::LB => (Side::Left, (1455., 610.), (1380., 600.)),
            Part::LStick => (Side::Left, (1507.4, 815.86), (1330., 818.)),
            Part::DPad => (Side::Left, (1676.4, 1091.48), (1450., 1100.)),
            Part::RT => (Side::Right, (2592., 520.), (2667., 470.)),
            Part::RB => (Side::Right, (2642., 610.), (2717., 600.)),
            Part::Y => (Side::Right, (2531.6, 654.5), (2580., 620.)),
            Part::X => (Side::Right, (2426., 781.3), (2560., 735.)),
            Part::B => (Side::Right, (2663.4, 810.52), (2700., 812.)),
            Part::A => (Side::Right, (2531.6, 965.9), (2580., 990.)),
            Part::RStick => (Side::Right, (2370.75, 1073.), (2560., 1110.)),
            Part::Menu => (Side::Top, (2173.03, 768.1), (2173.03, 768.1)),
            Part::View => (Side::Top, (1921.91, 768.1), (1921.91, 768.1)),
            Part::Guide => (Side::Top, (2048.5, 570.85), (2048.5, 570.85)),
        }
    }
}

/// What a button does in one tab: the button's name as a key cap and the
/// action. `off`: the binding is off (dashed); `vertical`: only the d-pad's
/// up and down arms light.
pub(super) struct Callout {
    pub part: Part,
    /// More parts lit with it (Back + Start, LB + RB).
    also: Option<Part>,
    cap: String,
    title: String,
    off: bool,
    vertical: bool,
}

fn callout(part: Part, cap: impl Into<String>, title: impl Into<String>) -> Callout {
    Callout { part, also: None, cap: cap.into(), title: title.into(), off: false, vertical: false }
}

impl Callout {
    fn off(mut self, off: bool) -> Self {
        self.off = off;
        self
    }
}

/// During a game: Guide opens the overlay; held with a button, a shortcut.
fn in_game(d: &SettingsPage) -> Vec<Callout> {
    let mut v = Vec::new();
    if !supported(Feature::Overlay) {
        return v;
    }
    let p = &d.overlay.prefs;
    let off = !p.enabled;
    if cfg!(windows) {
        let mut c = callout(Part::View, "Back + Start", tr!("Tap: open the overlay")).off(off);
        c.also = Some(Part::Menu);
        v.push(c);
    } else {
        v.push(callout(Part::Guide, "Guide", tr!("Tap: open the overlay")).off(off));
    }
    let shot = overlay::screenshot_button(p);
    v.push(callout(Part::of_chord(shot), overlay::button_short(shot), tr!("Take a screenshot")).off(off || !p.screenshot_shortcut));
    let taken = |b: ChordButton| p.screenshot_shortcut && shot == b;
    if supported(Feature::Replay) && !taken(ChordButton::X) {
        v.push(callout(Part::X, "X", tr!("Save the replay")).off(off || !p.replay_shortcut));
    }
    if !taken(ChordButton::Y) {
        v.push(callout(Part::Y, "Y", tr!("Mark a moment")).off(off || !p.moment_shortcut));
    }
    let mut volume = callout(Part::DPad, "↑↓", tr!("Volume up / down")).off(off || !p.volume_shortcut);
    volume.vertical = true;
    v.push(volume);
    v
}

/// Outside games: the button held for console mode, and the overlay when it
/// opens without a game.
fn outside(d: &SettingsPage) -> Vec<Callout> {
    let mut v = Vec::new();
    let both = super::tap_and_hold(d);
    let hold = if both { tr!("Hold: console mode · tap: overlay") } else { tr!("Hold: console mode") };
    match console_button(d) {
        ConsoleButton::Off => {}
        ConsoleButton::BackStart => {
            let mut c = callout(Part::View, "Back + Start", hold);
            c.also = Some(Part::Menu);
            v.push(c);
        }
        ConsoleButton::Bumpers => {
            v.push(callout(Part::LB, "LB + RB", hold));
            v.push(callout(Part::RB, "LB + RB", hold));
        }
        _ => v.push(callout(Part::Guide, "Guide", hold)),
    }
    if supported(Feature::Overlay) && !shares_guide(d) {
        let p = &d.overlay.prefs;
        let on = p.enabled && p.without_game;
        let mut c = if cfg!(windows) {
            let mut c = callout(Part::View, "Back + Start", tr!("Tap: open the overlay"));
            c.also = Some(Part::Menu);
            c
        } else {
            callout(Part::Guide, "Guide", tr!("Tap: open the overlay"))
        };
        c = c.off(!on);
        if !v.iter().any(|o| o.part == c.part) {
            v.push(c);
        }
    }
    v
}

/// While the overlay is open: fixed but for which button confirms.
fn overlay_open() -> Vec<Callout> {
    if !supported(Feature::Overlay) {
        return Vec::new();
    }
    let close = if cfg!(windows) {
        let mut c = callout(Part::View, "Back + Start", tr!("Close the overlay"));
        c.also = Some(Part::Menu);
        c
    } else {
        callout(Part::Guide, "Guide", tr!("Close the overlay"))
    };
    let (confirm, back) = super::table::confirm_parts();
    vec![
        close,
        callout(Part::LB, "LB", tr!("Previous tab")),
        callout(Part::RB, "RB", tr!("Next tab")),
        callout(Part::LStick, tr!("L stick"), tr!("Move")),
        callout(Part::DPad, tr!("D-pad"), tr!("Move")),
        callout(confirm, confirm.cap(), tr!("Select")),
        callout(back, back.cap(), tr!("Back · close")),
        callout(Part::Y, "Y", tr!("Mark a moment in Notes")),
    ]
}

/// In console mode, as its theme has it.
fn console(d: &SettingsPage) -> Vec<Callout> {
    let theme = super::console_theme(d);
    let details = theme.nav.a_button == AButton::Details;
    let (opens, other) = if details { (tr!("Open the game page"), tr!("Play")) } else { (tr!("Play"), tr!("Open the game page")) };
    let (confirm, back) = super::table::confirm_parts();
    let name = |p: Part| p.cap();
    vec![
        callout(Part::LT, "LT", tr!("Jump back by letter")),
        callout(Part::LB, "LB", tr!("Previous tab")),
        callout(Part::LStick, tr!("L stick"), tr!("Move")),
        callout(Part::DPad, tr!("D-pad"), tr!("Move")),
        callout(Part::RT, "RT", tr!("Jump on by letter")),
        callout(Part::RB, "RB", tr!("Next tab")),
        callout(Part::Y, "Y", tr!("Search and filters")),
        callout(Part::X, "X", other),
        callout(back, name(back), tr!("Back · sidebar")),
        callout(confirm, name(confirm), opens),
        callout(Part::Menu, "Menu", tr!("Open the menu")),
    ]
}

/// The callouts of `show`.
pub(super) fn callouts(d: &SettingsPage, show: Show) -> Vec<Callout> {
    match show {
        Show::InGame => in_game(d),
        Show::Outside => outside(d),
        Show::Overlay => overlay_open(),
        Show::Console => console(d),
    }
}

/// The controller's outlines in pad units (SVG paths): the body, the left
/// bumper and the d-pad's cross from the Zacksly drawing; the left trigger
/// and its ridge are ours. The right ones are mirrored (`mirror`).
const BODY: &str = "M2662,630.58c-136.49-129.36-351.3-108-351.3-108L2049,515.49v0l-.51,0-.5,0v0l-261.66,7.09s-214.81-21.36-351.3,108c0,0-265.38,583.77-246,815.34,0,0-9.13,188.61,171.26,231.58,0,0,95.74-55.93,242.11-252.94,0,0,68-93.36,177.23-84.66l268.39.79h1l268.39-.79c109.18-8.7,177.23,84.66,177.23,84.66,146.37,197,242.11,252.94,242.11,252.94,180.39-43,171.26-231.58,171.26-231.58C2927.35,1214.35,2662,630.58,2662,630.58Z";
const BUMPER: [(f32, f32); 7] =
    [(1443., 623.26), (1486.32, 485.67), (1720., 482.5), (1737.41, 485.), (1742.55, 489.15), (1772.85, 519.33), (1791.65, 522.44)];
const TRIGGER: [(f32, f32); 13] = [
    (1490., 548.),
    (1515., 488.),
    (1590., 446.),
    (1680., 440.),
    (1730., 437.),
    (1762., 452.),
    (1772., 488.),
    (1773.3, 497.),
    (1774.7, 506.),
    (1776., 515.),
    (1690., 505.),
    (1580., 515.),
    (1490., 548.),
];
const RIDGE: [(f32, f32); 4] = [(1560., 500.), (1590., 468.), (1640., 452.), (1700., 450.)];
const CROSS: &str = "M1940.49,1053.79s-3-7.32-12.85-7.91h-65v-65c-.6-9.89-7.91-12.86-7.91-12.86-7.12-4.15-37-4.15-37-4.15s-31.85,0-39,4.15c0,0-7.32,3-7.91,12.86v65h-65c-9.89.59-12.86,7.91-12.86,7.91-4.15,7.12-4.15,37-4.15,37s0,31.85,4.15,39c0,0,3,7.31,12.86,7.91h65v65c.59,9.890,7.91,12.85,7.91,12.85,7.12,4.16,37,4.16,37,4.16s31.85,0,39-4.16c0,0,7.31-3,7.91-12.85v-65h65c9.89-.6,12.85-7.91,12.85-7.91,4.16-7.12,4.16-37,4.16-37S1944.65,1060.91,1940.49,1053.79Z";
/// View's, Menu's and Share's marks (lines), and Share's pill.
const MARKS: &str = "<line x1='2151.97' y1='807.76' x2='2194.09' y2='807.76'/><line x1='2151.97' y1='796.43' x2='2194.09' y2='796.43'/><line x1='2151.97' y1='819.09' x2='2194.09' y2='819.09'/><rect x='1917.97' y='805.21' width='21.22' height='16.29'/><polyline points='1910.96 807.76 1904.33 807.76 1904.33 790.43 1925.93 790.43 1925.93 796.55'/><polyline points='2025.55 898.43 2025.55 914.16 2070.45 914.16 2070.45 898.43'/><line x1='2048' y1='905.39' x2='2048' y2='890.81'/><polyline points='2038.78 900.02 2048 890.81 2057.22 900.02'/>";
const SHARE: &str = "M2069.1,879.86h-45.2a24.92,24.92,0,0,0,0,49.84h45.2a24.92,24.92,0,1,0,0-49.84Z";
/// The radius of a stick's well and of its cap, of the d-pad's well, of a
/// face button, of View and Menu, and of Guide, in pad units.
const STICK_WELL: f32 = 94.87;
const STICK_CAP: f32 = 63.82;
const DPAD_WELL: f32 = 140.02;
const FACE_R: f32 = 57.76;
const SMALL_R: f32 = 40.03;
const GUIDE_R: f32 = 65.67;

/// `x` on the other side of the controller.
fn mirror(x: f32) -> f32 {
    2. * PAD_TOP.0 - x
}

/// A closed path of cubic curves through `pts` (a start, then three points
/// a curve), on the right side when `right`.
fn curves(pts: &[(f32, f32)], right: bool, close: bool) -> String {
    let x = |v: f32| if right { mirror(v) } else { v };
    let mut d = format!("M{:.1},{:.1}", x(pts[0].0), pts[0].1);
    for c in pts[1..].chunks(3) {
        d.push('C');
        d.push_str(&c.iter().map(|(px, py)| format!("{:.1},{:.1}", x(*px), py)).collect::<Vec<_>>().join(" "));
    }
    if close {
        d.push('Z');
    }
    d
}

/// How a part looks: not used in this tab, used, or picked.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Look {
    Idle,
    Used,
    Picked,
}

/// The colours, resolved before painting.
#[derive(Clone, Copy)]
struct Tones {
    body: Hsla,
    body_line: Hsla,
    well: Hsla,
    part: Hsla,
    part_line: Hsla,
    used: Hsla,
    used_line: Hsla,
    picked: Hsla,
    picked_fill: Hsla,
    lead: Hsla,
}

impl Tones {
    fn new() -> Self {
        let base: Hsla = theme::opaque_card();
        let body = base.blend(theme::ink_a(0.07));
        Tones {
            body,
            body_line: theme::ink_a(0.36),
            well: base.blend(theme::ink_a(0.03)),
            part: body.blend(theme::ink_a(0.05)),
            part_line: theme::ink_a(0.42),
            used: body.blend(theme::ink_a(0.26)),
            used_line: theme::ink_a(0.85),
            picked: theme::accent().into(),
            picked_fill: body.blend(theme::accent_alpha(0.22).into()),
            lead: theme::ink_a(0.30),
        }
    }

    fn fill(&self, look: Look) -> Hsla {
        match look {
            Look::Idle => self.part,
            Look::Used => self.used,
            Look::Picked => self.picked_fill,
        }
    }

    fn line(&self, look: Look) -> (Hsla, f32) {
        match look {
            Look::Idle => (self.part_line, 1.2),
            Look::Used => (self.used_line, 1.5),
            Look::Picked => (self.picked, 2.),
        }
    }
}

/// A leader line in box units: its points, dashed, picked.
struct Lead {
    points: Vec<(f32, f32)>,
    dashed: bool,
    picked: bool,
}

/// What the controller's picture and the lines need.
struct Paint {
    tones: Tones,
    /// The look of each part.
    looks: Vec<(Part, Look)>,
    /// Only the d-pad's vertical arms are lit.
    dpad_vertical: bool,
    leads: Vec<Lead>,
}

impl Paint {
    fn look(&self, part: Part) -> Look {
        self.looks.iter().find(|(q, _)| *q == part).map(|(_, l)| *l).unwrap_or(Look::Idle)
    }
}

/// `#rrggbb` and its opacity, as SVG attributes `name` and `name-opacity`.
fn paint_attr(name: &str, c: Hsla) -> String {
    let c = Rgba::from(c);
    let b = |v: f32| (v.clamp(0., 1.) * 255.).round() as u8;
    format!("{name}='#{:02x}{:02x}{:02x}' {name}-opacity='{:.3}'", b(c.r), b(c.g), b(c.b), c.a)
}

/// The controller as an SVG `px_w` pixels wide, in box units (its picture
/// fills the drawing): triggers and bumpers behind the body, then the
/// sticks, the d-pad, View, Menu, Share and Guide in their looks. The face
/// buttons are drawn over it (`face_button`), in the controller's style.
fn controller_svg(p: &Paint, px_w: f32) -> String {
    let t = &p.tones;
    let k = PAD_SCALE;
    // Strokes in box units, made pad units.
    let fill_line = |look: Look| {
        let (c, w) = t.line(look);
        format!("{} {} stroke-width='{:.1}'", paint_attr("fill", t.fill(look)), paint_attr("stroke", c), w / k)
    };
    let mut o = String::with_capacity(6000);
    o.push_str(&format!(
        "<svg xmlns='http://www.w3.org/2000/svg' width='{w:.0}' height='{h:.0}' viewBox='0 0 {BOX_W} {BOX_H}'>",
        w = px_w,
        h = px_w * BOX_H / BOX_W,
    ));
    // The body is lighter in the middle, as a moulded shell.
    o.push_str(&format!(
        "<defs><linearGradient id='b' gradientUnits='userSpaceOnUse' x1='1188' y1='0' x2='2909' y2='0'>\
         <stop offset='0' {e}/><stop offset='0.5' {m}/><stop offset='1' {e}/></linearGradient>\
         <clipPath id='dp'><path d='{CROSS}'/></clipPath></defs>",
        e = paint_attr("stop-color", t.body).replace("stop-color-opacity", "stop-opacity"),
        m = paint_attr("stop-color", t.body.blend(theme::ink_a(0.05))).replace("stop-color-opacity", "stop-opacity"),
    ));
    o.push_str(&format!(
        "<g transform='translate({:.3},{:.3}) scale({k})' stroke-linejoin='round' stroke-linecap='round'>",
        ORIGIN.0 - PAD_TOP.0 * k,
        ORIGIN.1 - PAD_TOP.1 * k,
    ));
    // Triggers (idle: in the shade behind the bumpers), with a ridge.
    for (part, right) in [(Part::LT, false), (Part::RT, true)] {
        let look = p.look(part);
        let fill = if look == Look::Idle { t.well } else { t.fill(look) };
        let (c, w) = t.line(look);
        o.push_str(&format!(
            "<path d='{}' {} {} stroke-width='{:.1}'/><path d='{}' fill='none' {} stroke-width='{:.1}' opacity='0.7'/>",
            curves(&TRIGGER, right, true),
            paint_attr("fill", fill),
            paint_attr("stroke", c),
            w / k,
            curves(&RIDGE, right, false),
            paint_attr("stroke", c),
            w * 0.8 / k,
        ));
    }
    for (part, right) in [(Part::LB, false), (Part::RB, true)] {
        o.push_str(&format!("<path d='{}' {}/>", curves(&BUMPER, right, true), fill_line(p.look(part))));
    }
    o.push_str(&format!("<path d='{BODY}' fill='url(#b)' {} stroke-width='{:.1}'/>", paint_attr("stroke", t.body_line), 2. / k));
    let well = |cx: f32, cy: f32, r: f32| {
        format!(
            "<circle cx='{cx}' cy='{cy}' r='{r}' {} {} stroke-width='{:.1}'/>",
            paint_attr("fill", t.well),
            paint_attr("stroke", t.body_line),
            1.2 / k
        )
    };
    for part in [Part::LStick, Part::RStick] {
        let (cx, cy) = part.centre();
        o.push_str(&well(cx, cy, STICK_WELL));
        o.push_str(&format!("<circle cx='{cx}' cy='{cy}' r='{STICK_CAP}' {}/>", fill_line(p.look(part))));
    }
    let (dx, dy) = Part::DPad.centre();
    o.push_str(&well(dx, dy, DPAD_WELL));
    let dl = p.look(Part::DPad);
    if p.dpad_vertical && dl != Look::Idle {
        // Only up and down lit: the cross idle, its vertical bar filled.
        o.push_str(&format!("<path d='{CROSS}' {}/>", paint_attr("fill", t.fill(Look::Idle))));
        o.push_str(&format!(
            "<rect x='{:.1}' y='{:.1}' width='78' height='270' clip-path='url(#dp)' {}/>",
            dx - 39.,
            dy - 135.,
            paint_attr("fill", t.fill(dl))
        ));
        let (c, w) = t.line(dl);
        o.push_str(&format!("<path d='{CROSS}' fill='none' {} stroke-width='{:.1}'/>", paint_attr("stroke", c), w / k));
    } else {
        o.push_str(&format!("<path d='{CROSS}' {}/>", fill_line(dl)));
    }
    for part in [Part::View, Part::Menu] {
        let (cx, cy) = part.centre();
        o.push_str(&format!("<circle cx='{cx}' cy='{cy}' r='{SMALL_R}' {}/>", fill_line(p.look(part))));
    }
    o.push_str(&format!("<path d='{SHARE}' {}/>", fill_line(Look::Idle)));
    let mark = |look: Look| if look == Look::Idle { theme::text_dim() } else { theme::text_primary() };
    // The marks follow View and Menu (Share is never lit).
    o.push_str(&format!(
        "<g fill='none' {} stroke-width='{:.1}'>{MARKS}</g>",
        paint_attr("stroke", mark(p.look(Part::View).max(p.look(Part::Menu))).into()),
        1.1 / k
    ));
    let (gx, gy) = Part::Guide.centre();
    o.push_str(&format!("<circle cx='{gx}' cy='{gy}' r='{GUIDE_R}' {}/>", fill_line(p.look(Part::Guide))));
    for part in [Part::A, Part::B, Part::X, Part::Y] {
        // A shade under each face button (drawn over it): its seat.
        let (cx, cy) = part.centre();
        o.push_str(&format!("<circle cx='{cx}' cy='{cy}' r='{:.1}' {}/>", FACE_R + 6., paint_attr("fill", t.well)));
    }
    o.push_str("</g></svg>");
    o
}

thread_local! {
    /// The pictures made (one per set of looks and colours), so a frame
    /// draws the one it had.
    static MADE: std::cell::RefCell<std::collections::HashMap<String, std::sync::Arc<Image>>> = Default::default();
}

/// The controller's picture for `p`, `px_w` pixels wide.
fn picture(p: &Paint, px_w: f32) -> std::sync::Arc<Image> {
    let src = controller_svg(p, px_w);
    MADE.with(|m| {
        let mut m = m.borrow_mut();
        if m.len() > 24 {
            m.clear();
        }
        m.entry(src).or_insert_with_key(|src| std::sync::Arc::new(Image::from_bytes(ImageFormat::Svg, src.clone().into_bytes()))).clone()
    })
}

/// The leader lines, and a dot where each meets its part.
fn paint_leads(bounds: Bounds<Pixels>, leads: &[Lead], tones: &Tones, window: &mut Window) {
    let o = bounds.origin;
    // Box units to pixels.
    let k = f32::from(bounds.size.width) / BOX_W;
    let px_of = |(x, y): (f32, f32)| point(o.x + px(x * k), o.y + px(y * k));
    for lead in leads {
        let color = if lead.picked { tones.picked } else { tones.lead };
        let mut b = PathBuilder::stroke(px(if lead.picked { 1.6 } else { 1.2 }));
        if lead.dashed {
            b = b.dash_array(&[px(3.), px(4.)]);
        }
        let mut pts = lead.points.iter();
        if let Some(first) = pts.next() {
            b.move_to(px_of(*first));
            for q in pts {
                b.line_to(px_of(*q));
            }
        }
        if let Ok(path) = b.build() {
            window.paint_path(path, color);
        }
        if let Some(first) = lead.points.first() {
            let c = px_of(*first);
            let r = 2.6;
            let dot: Vec<Point<Pixels>> = (0..12)
                .map(|i| {
                    let a = i as f32 / 12. * std::f32::consts::TAU;
                    point(c.x + px(r * a.cos()), c.y + px(r * a.sin()))
                })
                .collect();
            let mut b = PathBuilder::fill();
            b.add_polygon(&dot, true);
            if let Ok(path) = b.build() {
                window.paint_path(path, color);
            }
        }
    }
}

/// A callout's place in box units (left, top, width) and, on a side, its
/// rank in its column from the top.
struct Placed {
    index: usize,
    x: f32,
    y: f32,
    w: f32,
    rank: usize,
}

/// The narrowest the drawing gets.
const MIN_W: f32 = 680.;

/// A lone top callout's width (it has the room).
const TOP_W: f32 = 290.;

/// Stacks each side's callouts by where their lines bend, keeping them in
/// the box; the top ones go side by side, in the order of their buttons.
fn place(parts: &[Part]) -> Vec<Placed> {
    let mut out = Vec::new();
    for side in [Side::Left, Side::Right] {
        let mut list: Vec<(usize, f32)> = parts
            .iter()
            .enumerate()
            .filter(|(_, p)| p.route().0 == side)
            .map(|(i, p)| (i, at_box(p.route().2 .0, p.route().2 .1).1))
            .collect();
        list.sort_by(|a, b| a.1.total_cmp(&b.1));
        let mut tops: Vec<f32> = Vec::new();
        let mut next = 0f32;
        for (_, ky) in &list {
            let top = (ky - CALLOUT_H / 2.).max(next);
            tops.push(top);
            next = top + CALLOUT_H + CALLOUT_GAP;
        }
        // Pushed past the bottom: move the stack up.
        let mut limit = BOX_H - CALLOUT_H;
        for top in tops.iter_mut().rev() {
            if *top > limit {
                *top = limit;
            }
            limit = *top - CALLOUT_H - CALLOUT_GAP;
        }
        let x = if side == Side::Left { 0. } else { BOX_W - CALLOUT_W };
        out.extend(list.iter().zip(tops).enumerate().map(|(rank, ((i, _), y))| Placed { index: *i, x, y: y.max(0.), w: CALLOUT_W, rank }));
    }
    let mut top: Vec<(usize, f32)> =
        parts.iter().enumerate().filter(|(_, p)| p.route().0 == Side::Top).map(|(i, p)| (i, at_box(p.route().1 .0, 0.).0)).collect();
    top.sort_by(|a, b| a.1.total_cmp(&b.1));
    match top.as_slice() {
        [] => {}
        [(i, _)] => out.push(Placed { index: *i, x: (BOX_W - TOP_W) / 2., y: 0., w: TOP_W, rank: 0 }),
        // Two: they meet halfway between their buttons, so each line goes
        // straight up into its own.
        [(i, a), (j, b)] => {
            let split = (a + b) / 2.;
            out.push(Placed { index: *i, x: split - CALLOUT_GAP / 2. - CALLOUT_W, y: 0., w: CALLOUT_W, rank: 0 });
            out.push(Placed { index: *j, x: split + CALLOUT_GAP / 2., y: 0., w: CALLOUT_W, rank: 1 });
        }
        more => {
            let total = more.len() as f32 * (CALLOUT_W + CALLOUT_GAP) - CALLOUT_GAP;
            for (n, (i, _)) in more.iter().enumerate() {
                let x = BOX_W / 2. - total / 2. + n as f32 * (CALLOUT_W + CALLOUT_GAP);
                out.push(Placed { index: *i, x, y: 0., w: CALLOUT_W, rank: n });
            }
        }
    }
    out
}

/// The leader line of each callout, in box units. On a side: from the
/// part's rim out to its bend, across to its own lane, along the lane to the
/// callout's height and into the callout; the lane of a higher callout is
/// nearer the column, so no line crosses another. On top: up from the part,
/// across to the callout's middle and up into it.
fn routes(parts: &[Part], placed: &[Placed]) -> Vec<Vec<(f32, f32)>> {
    parts
        .iter()
        .enumerate()
        .map(|(i, part)| {
            let (side, anchor, knee) = part.route();
            let a = at_box(anchor.0, anchor.1);
            let Some(pl) = placed.iter().find(|p| p.index == i) else { return Vec::new() };
            if side == Side::Top {
                if a.0 > pl.x && a.0 < pl.x + pl.w {
                    return vec![a, (a.0, CALLOUT_H)];
                }
                let mid = CALLOUT_H + 12.;
                let cx = pl.x + pl.w / 2.;
                return vec![a, (a.0, mid), (cx, mid), (cx, CALLOUT_H)];
            }
            let k = at_box(knee.0, knee.1);
            let cy = pl.y + CALLOUT_H / 2.;
            let step = 12. + LANE * pl.rank as f32;
            let (edge, lane) = if side == Side::Left {
                (CALLOUT_W, (CALLOUT_W + step).min(k.0 - 6.))
            } else {
                (BOX_W - CALLOUT_W, (BOX_W - CALLOUT_W - step).max(k.0 + 6.))
            };
            vec![a, k, (lane, k.1), (lane, cy), (edge, cy)]
        })
        .collect()
}

/// The space between two lanes, in box units.
const LANE: f32 = 9.;

/// The drawing, its callouts and their lines, for the page's tab.
pub(super) fn map(d: &SettingsPage, window: &Window, cx: &mut Context<SettingsPage>) -> Div {
    let show = d.controls.show;
    let picked = d.controls.selected;
    let lines = callouts(d, show);
    // Places in box units, as fractions of the drawing (it keeps its shape
    // at any width).
    let fx = |v: f32| relative(v / BOX_W);
    let fy = |v: f32| relative(v / BOX_H);
    let look_of = |part: Part| {
        if picked == Some(part) {
            Look::Picked
        } else if lines.iter().any(|c| !c.off && (c.part == part || c.also == Some(part))) {
            Look::Used
        } else {
            Look::Idle
        }
    };
    let parts: Vec<Part> = lines.iter().map(|c| c.part).collect();
    let placed = place(&parts);
    let leads: Vec<Lead> = routes(&parts, &placed)
        .into_iter()
        .zip(&lines)
        .map(|(points, c)| Lead { points, dashed: c.off, picked: picked == Some(c.part) })
        .collect();
    let dpad_vertical = lines.iter().any(|c| c.part == Part::DPad && c.vertical && !c.off)
        && !lines.iter().any(|c| c.part == Part::DPad && !c.vertical && !c.off);
    let paint_data = Paint { tones: Tones::new(), looks: Part::ALL.iter().map(|p| (*p, look_of(*p))).collect(), dpad_vertical, leads };
    let tones = paint_data.tones;
    // Drawn at the window's pixels for the widest the drawing gets.
    let image = picture(&paint_data, (BOX_W * window.scale_factor()).round());
    let leads = paint_data.leads;
    let mut root = div()
        .relative()
        .w_full()
        .max_w(px(BOX_W))
        // Narrower, the callouts get too short for their text.
        .min_w(px(MIN_W))
        .aspect_ratio(BOX_W / BOX_H)
        .mx_auto()
        .child(img(image).absolute().inset_0().size_full())
        .child(canvas(|_, _, _| (), move |bounds, _, window, _| paint_leads(bounds, &leads, &tones, window)).absolute().inset_0());
    // The face buttons as boxes (drawn as the controller has them), Guide's
    // mark, and a click area on every part.
    let pad_box = |x: f32, y: f32, wd: f32, ht: f32| {
        let (bx, by) = at_box(x - wd / 2., y - ht / 2.);
        (fx(bx), fy(by), fx(wd * PAD_SCALE), fy(ht * PAD_SCALE))
    };
    let style = crate::console::look::pad_glyphs();
    for part in [Part::A, Part::B, Part::X, Part::Y] {
        let (cx0, cy0) = part.centre();
        let (l, t, wd, ht) = pad_box(cx0, cy0, 2. * FACE_R, 2. * FACE_R);
        let look = look_of(part);
        let f = part.face();
        let (colour, letter) = super::face_look(style, f);
        // A neutral face (PlayStation, Nintendo, Deck) lights like the other parts.
        let lit = colour.unwrap_or(tones.used);
        let on_lit: Hsla = if colour.is_some() { theme::console_on_focus().into() } else { theme::text_primary().into() };
        let b = div()
            .absolute()
            .left(l)
            .top(t)
            .w(wd)
            .h(ht)
            .rounded_full()
            .flex()
            .items_center()
            .justify_center()
            .text_size(px(10.5))
            .font_weight(theme::W_BOLD);
        let b = match look {
            Look::Idle => b.bg(tones.part).border_1().border_color(tones.part_line).text_color(theme::text_dim()),
            Look::Used => b.bg(lit).when(colour.is_none(), |b| b.border_1().border_color(tones.used_line)).text_color(on_lit),
            Look::Picked => b.bg(lit).border_2().border_color(tones.picked).text_color(on_lit),
        };
        let b = match letter {
            Some(l) => b.child(l),
            None => {
                let shape: Hsla = if look == Look::Idle { theme::text_dim().into() } else { crate::console::kit::shape_color(f).into() };
                b.child(
                    canvas(|_, _, _| (), move |bounds, _, window, _| crate::console::kit::paint_shape(bounds, f, shape, window))
                        .absolute()
                        .inset_0(),
                )
            }
        };
        root = root.child(b);
    }
    {
        let (cx0, cy0) = Part::Guide.centre();
        let (l, t, wd, ht) = pad_box(cx0, cy0, 2. * GUIDE_R, 2. * GUIDE_R);
        let ic = if look_of(Part::Guide) == Look::Idle { theme::text_dim() } else { theme::text_primary() };
        root = root.child(div().absolute().left(l).top(t).w(wd).h(ht).flex().items_center().justify_center().child(crate::ui::icon(
            crate::icons::Icon::SportsEsports,
            px(13.),
            ic,
        )));
    }
    // Click areas over every part (most are in the picture).
    for part in Part::ALL {
        let (cx0, cy0) = part.centre();
        let (bw, bh) = match part {
            Part::LT | Part::RT => (210., 90.),
            Part::LB | Part::RB => (330., 80.),
            Part::LStick | Part::RStick => (2. * STICK_WELL, 2. * STICK_WELL),
            Part::DPad => (2. * DPAD_WELL, 2. * DPAD_WELL),
            Part::View | Part::Menu => (2. * SMALL_R + 12., 2. * SMALL_R + 12.),
            Part::Guide => (2. * GUIDE_R, 2. * GUIDE_R),
            _ => (2. * FACE_R, 2. * FACE_R),
        };
        let (l, t, wd, ht) = pad_box(cx0, cy0, bw, bh);
        root = root.child(
            div()
                .id(SharedString::from(format!("controls-pad-{part:?}")))
                .absolute()
                .left(l)
                .top(t)
                .w(wd)
                .h(ht)
                .cursor_pointer()
                .on_click(cx.listener(move |d, _, _, cx| toggle_selected(d, part, cx))),
        );
    }
    // The callouts.
    let callout_el = |i: usize, c: &Callout, cx: &mut Context<SettingsPage>| {
        let on = picked == Some(c.part);
        let part = c.part;
        let full = format!("{} · {}", super::shown_name(&c.cap), c.title);
        let b = div()
            .id(SharedString::from(format!("controls-callout-{i}")))
            .flex()
            .items_center()
            .gap(px(8.))
            .size_full()
            .px(px(8.))
            .rounded(px(10.))
            .border_1()
            .cursor_pointer()
            .tooltip(crate::ui::tooltip(full))
            .on_click(cx.listener(move |d, _, _, cx| toggle_selected(d, part, cx)))
            .child(glyph(&c.cap))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_size(px(12.5))
                    .text_color(if c.off { theme::text_dim() } else { theme::text_primary() })
                    .whitespace_nowrap()
                    .truncate()
                    .child(c.title.clone()),
            );
        if on {
            b.bg(theme::accent_tint()).border_color(theme::accent())
        } else if c.off {
            b.border_dashed().border_color(theme::ink_a(0.16)).opacity(0.7)
        } else {
            b.bg(theme::opaque_card()).border_color(theme::ink_a(0.10))
        }
    };
    for p in &placed {
        let el = callout_el(p.index, &lines[p.index], cx);
        root = root.child(div().absolute().left(fx(p.x)).top(fy(p.y)).w(fx(p.w)).h(fy(CALLOUT_H)).child(el));
    }
    let empty = lines.is_empty();
    div()
        .flex()
        .flex_col()
        .gap(px(6.))
        .child(root)
        .when(empty, |c| c.child(div().flex().justify_center().child(crate::ui::fields::meta_text(tr!("Nothing is set for this.")))))
}

impl Part {
    /// A face button's Xbox letter (by position: `glyph` draws it as the
    /// controller has it).
    pub(super) fn cap(self) -> &'static str {
        match self {
            Part::A => "A",
            Part::B => "B",
            Part::X => "X",
            Part::Y => "Y",
            _ => "",
        }
    }

    /// A face button's position.
    fn face(self) -> crate::console::kit::Face {
        use crate::console::kit::Face;
        match self {
            Part::B => Face::East,
            Part::X => Face::West,
            Part::Y => Face::North,
            _ => Face::South,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{place, routes, Part, Side, CALLOUT_H};

    /// `p`–`q` and `r`–`s` cross (touching ends do not count).
    fn cross(p: (f32, f32), q: (f32, f32), r: (f32, f32), s: (f32, f32)) -> bool {
        let d = |a: (f32, f32), b: (f32, f32), c: (f32, f32)| (b.0 - a.0) * (c.1 - a.1) - (b.1 - a.1) * (c.0 - a.0);
        let (d1, d2, d3, d4) = (d(r, s, p), d(r, s, q), d(p, q, r), d(p, q, s));
        (d1 * d2 < -1e-3) && (d3 * d4 < -1e-3)
    }

    /// No two leader lines cross, in any tab's set of callouts.
    #[test]
    fn leader_lines_never_cross() {
        use Part::*;
        let tabs: &[&[Part]] = &[
            // In a game, with the screenshot on each button it can take.
            &[Guide, RB, X, Y, DPad],
            &[Guide, LB, X, Y, DPad],
            &[Guide, A, X, Y, DPad],
            &[Guide, B, X, Y, DPad],
            &[Guide, DPad, X, Y, DPad],
            &[View, Menu, X, Y, DPad],
            &[Guide, View, X, Y, DPad],
            &[Guide, Menu, X, Y, DPad],
            // Outside games.
            &[Guide],
            &[LB, RB, Guide],
            // Overlay open.
            &[Guide, LB, RB, LStick, DPad, A, B, Y],
            // Console mode, and with A and B swapped.
            &[LT, LB, LStick, DPad, RT, RB, Y, X, B, A, Menu],
            &[LT, LB, LStick, DPad, RT, RB, Y, X, A, B, Menu],
        ];
        for parts in tabs {
            let placed = place(parts);
            let lines = routes(parts, &placed);
            // No line runs through another callout.
            for (i, line) in lines.iter().enumerate() {
                for other in placed.iter().filter(|p| p.index != i) {
                    for seg in line.windows(2) {
                        for n in 0..=50 {
                            let t = n as f32 / 50.;
                            let (x, y) = (seg[0].0 + (seg[1].0 - seg[0].0) * t, seg[0].1 + (seg[1].1 - seg[0].1) * t);
                            let inside =
                                x > other.x + 0.5 && x < other.x + other.w - 0.5 && y > other.y + 0.5 && y < other.y + CALLOUT_H - 0.5;
                            assert!(!inside, "{:?}'s line runs through {:?}'s callout in {parts:?}", parts[i], parts[other.index]);
                        }
                    }
                }
            }
            // Each line ends in its own callout.
            for (i, line) in lines.iter().enumerate() {
                let p = placed.iter().find(|p| p.index == i).expect("placed");
                let (x, y) = *line.last().expect("a line");
                let inside = x >= p.x - 0.01 && x <= p.x + p.w + 0.01 && y >= p.y - 0.01 && y <= p.y + CALLOUT_H + 0.01;
                assert!(inside, "{:?}'s line misses its callout in {parts:?}", parts[i]);
                // One or two on top: each line goes straight up.
                let tops = parts.iter().filter(|p| p.route().0 == Side::Top).count();
                if parts[i].route().0 == Side::Top && tops <= 2 {
                    assert_eq!(line.len(), 2, "{:?}'s top line bends in {parts:?}", parts[i]);
                }
            }
            for (i, a) in lines.iter().enumerate() {
                for (j, b) in lines.iter().enumerate().skip(i + 1) {
                    if parts[i] == parts[j] {
                        continue;
                    }
                    for sa in a.windows(2) {
                        for sb in b.windows(2) {
                            assert!(!cross(sa[0], sa[1], sb[0], sb[1]), "{:?} and {:?} cross in {parts:?}", parts[i], parts[j]);
                        }
                    }
                }
            }
        }
    }
}
