//! The drawn controller of Settings → Controller: an Xbox-style pad (the
//! body, bumpers and triggers painted with curves; sticks, face buttons,
//! Guide, View and Menu as round boxes), the buttons the tab uses lit, and a
//! callout for each in a column on its side (Guide's on top) joined to the
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
/// The pad's own units (the outline below) map into the box at this scale,
/// with its point (440, 260) at `ORIGIN`.
const PAD_SCALE: f32 = 0.78;
const ORIGIN: (f32, f32) = (440., 184.);
/// A callout's width and height, and the gap between two, in box units.
const CALLOUT_W: f32 = 212.;
const CALLOUT_H: f32 = 32.;
const CALLOUT_GAP: f32 = 6.;

/// A pad point in box units.
fn at_box(x: f32, y: f32) -> (f32, f32) {
    (ORIGIN.0 + (x - 440.) * PAD_SCALE, ORIGIN.1 + (y - 260.) * PAD_SCALE)
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
            Part::LT => (300., 84.),
            Part::RT => (580., 84.),
            Part::LB => (300., 108.),
            Part::RB => (580., 108.),
            Part::LStick => (322., 174.),
            Part::RStick => (520., 250.),
            Part::DPad => (382., 250.),
            Part::Y => (568., 142.),
            Part::A => (568., 206.),
            Part::X => (536., 174.),
            Part::B => (600., 174.),
            Part::View => (408., 174.),
            Part::Menu => (472., 174.),
            Part::Guide => (440., 136.),
        }
    }

    /// Its callout's side, where the line leaves the part (on its rim) and
    /// where it bends, in pad units. The face buttons' lines leave between
    /// their neighbours, so no line crosses another button.
    fn route(self) -> (Side, (f32, f32), (f32, f32)) {
        match self {
            Part::LT => (Side::Left, (268., 78.), (240., 62.)),
            Part::LB => (Side::Left, (240., 106.), (216., 94.)),
            Part::LStick => (Side::Left, (292., 174.), (250., 176.)),
            Part::DPad => (Side::Left, (352., 254.), (300., 272.)),
            Part::RT => (Side::Right, (612., 78.), (640., 62.)),
            Part::RB => (Side::Right, (640., 106.), (664., 94.)),
            Part::Y => (Side::Right, (581., 137.), (612., 124.)),
            Part::X => (Side::Right, (548., 170.), (606., 150.)),
            Part::B => (Side::Right, (613., 174.), (640., 178.)),
            Part::A => (Side::Right, (580., 212.), (612., 226.)),
            Part::RStick => (Side::Right, (548., 258.), (600., 280.)),
            Part::Menu => (Side::Top, (472., 168.), (472., 168.)),
            Part::View => (Side::Top, (408., 168.), (408., 168.)),
            Part::Guide => (Side::Top, (440., 121.), (440., 121.)),
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

/// While the overlay is open: fixed.
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
    vec![
        close,
        callout(Part::LB, "LB", tr!("Previous tab")),
        callout(Part::RB, "RB", tr!("Next tab")),
        callout(Part::LStick, tr!("L stick"), tr!("Move")),
        callout(Part::DPad, tr!("D-pad"), tr!("Move")),
        callout(Part::A, "A", tr!("Select")),
        callout(Part::B, "B", tr!("Back · close")),
        callout(Part::Y, "Y", tr!("Mark a moment in Notes")),
    ]
}

/// In console mode, as its theme has it.
fn console(d: &SettingsPage) -> Vec<Callout> {
    let theme = super::console_theme(d);
    let swap = theme.controls.swap_ab;
    let details = theme.nav.a_button == AButton::Details;
    let (opens, other) = if details { (tr!("Open the game page"), tr!("Play")) } else { (tr!("Play"), tr!("Open the game page")) };
    let (confirm, back) = if swap { (Part::B, Part::A) } else { (Part::A, Part::B) };
    let name = |p: Part| if p == Part::A { "A" } else { "B" };
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
        callout(Part::Menu, tr!("Menu"), tr!("Open the menu")),
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

/// The controller's outline in pad units: cubic curves from (440, 118).
const BODY: [[f32; 6]; 12] = [
    [492., 118., 540., 102., 592., 106.],
    [642., 110., 668., 138., 682., 186.],
    [698., 240., 712., 330., 708., 380.],
    [704., 424., 672., 444., 642., 428.],
    [618., 414., 602., 380., 584., 350.],
    [566., 322., 530., 306., 440., 306.],
    [350., 306., 314., 322., 296., 350.],
    [278., 380., 262., 414., 238., 428.],
    [208., 444., 176., 424., 172., 380.],
    [168., 330., 182., 240., 198., 186.],
    [212., 138., 238., 110., 288., 106.],
    [340., 102., 388., 118., 440., 118.],
];

/// A shoulder part's outline (left side; the right one is mirrored): a start
/// point and cubic curves or lines (a line has its control points on it).
fn shoulder(part: Part) -> ((f32, f32), Vec<[f32; 6]>) {
    let (start, segs): ((f32, f32), Vec<[f32; 6]>) = match part {
        Part::LT | Part::RT => (
            (250., 104.),
            vec![
                [256., 82., 276., 72., 304., 70.],
                [314., 70., 326., 70., 336., 70.],
                [352., 70., 358., 82., 352., 100.],
                [318., 101., 284., 102., 250., 104.],
            ],
        ),
        _ => (
            (206., 146.),
            vec![
                [220., 114., 252., 98., 296., 96.],
                [321., 97., 347., 99., 372., 100.],
                [386., 101., 386., 116., 372., 116.],
                [347., 115., 321., 114., 296., 114.],
                [262., 116., 236., 130., 220., 154.],
                [215., 151., 210., 149., 206., 146.],
            ],
        ),
    };
    let flip = matches!(part, Part::RT | Part::RB);
    let m = |x: f32| if flip { 880. - x } else { x };
    let segs = segs.into_iter().map(|[a, b, c, d2, e, f]| [m(a), b, m(c), d2, m(e), f]).collect();
    ((m(start.0), start.1), segs)
}

/// The d-pad's cross as a polygon in pad units.
fn dpad() -> Vec<(f32, f32)> {
    let (cx, cy) = Part::DPad.centre();
    let (a, l) = (11., 30.);
    vec![
        (cx - a, cy - l),
        (cx + a, cy - l),
        (cx + a, cy - a),
        (cx + l, cy - a),
        (cx + l, cy + a),
        (cx + a, cy + a),
        (cx + a, cy + l),
        (cx - a, cy + l),
        (cx - a, cy + a),
        (cx - l, cy + a),
        (cx - l, cy - a),
        (cx - a, cy - a),
    ]
}

/// How a part looks: not used in this tab, used, or picked.
#[derive(Clone, Copy, PartialEq, Eq)]
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
            body_line: theme::ink_a(0.24),
            well: base.blend(theme::ink_a(0.03)),
            part: body.blend(theme::ink_a(0.05)),
            part_line: theme::ink_a(0.26),
            used: body.blend(theme::ink_a(0.18)),
            used_line: theme::ink_a(0.80),
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

/// What the painter needs.
struct Paint {
    tones: Tones,
    /// The look of each painted part (shoulders and d-pad).
    looks: Vec<(Part, Look)>,
    /// Only the d-pad's vertical arms are lit.
    dpad_vertical: bool,
    leads: Vec<Lead>,
}

fn paint(bounds: Bounds<Pixels>, p: &Paint, window: &mut Window) {
    let o = bounds.origin;
    // Box units to pixels.
    let k = f32::from(bounds.size.width) / BOX_W;
    let px_of = |(x, y): (f32, f32)| point(o.x + px(x * k), o.y + px(y * k));
    let pad = |(x, y): (f32, f32)| px_of(at_box(x, y));
    let w = |v: f32| px(v * PAD_SCALE * k);
    let look = |part: Part| p.looks.iter().find(|(q, _)| *q == part).map(|(_, l)| *l).unwrap_or(Look::Idle);
    let curves = |start: (f32, f32), segs: &[[f32; 6]], builder: PathBuilder| {
        let mut b = builder;
        b.move_to(pad(start));
        for [ax, ay, bx, by, x, y] in segs {
            b.cubic_bezier_to(pad((*x, *y)), pad((*ax, *ay)), pad((*bx, *by)));
        }
        b.close();
        b.build()
    };
    let fill_and_line = |start: (f32, f32), segs: &[[f32; 6]], fill: Hsla, line: (Hsla, f32), window: &mut Window| {
        if let Ok(path) = curves(start, segs, PathBuilder::fill()) {
            window.paint_path(path, fill);
        }
        if let Ok(path) = curves(start, segs, PathBuilder::stroke(w(line.1))) {
            window.paint_path(path, line.0);
        }
    };
    let polygon = |pts: &[(f32, f32)], fill: Hsla, line: (Hsla, f32), window: &mut Window| {
        let points: Vec<Point<Pixels>> = pts.iter().map(|q| pad(*q)).collect();
        let mut b = PathBuilder::fill();
        b.add_polygon(&points, true);
        if let Ok(path) = b.build() {
            window.paint_path(path, fill);
        }
        if line.1 > 0. {
            let mut b = PathBuilder::stroke(w(line.1));
            b.add_polygon(&points, true);
            if let Ok(path) = b.build() {
                window.paint_path(path, line.0);
            }
        }
    };
    let t = &p.tones;
    // Triggers and bumpers behind the body, then the body. (Lit bumpers are
    // drawn again over the body and the lines, whole, at the end.)
    let shoulders = [Part::LT, Part::RT, Part::LB, Part::RB];
    for part in shoulders {
        let (start, segs) = shoulder(part);
        let l = look(part);
        fill_and_line(start, &segs, t.fill(l), t.line(l), window);
    }
    fill_and_line((440., 118.), &BODY, t.body, (t.body_line, 2.), window);

    // The d-pad: the cross; lit only up and down, its vertical bar is
    // filled under the cross's outline.
    let dl = look(Part::DPad);
    let cross = &dpad();
    if p.dpad_vertical && dl != Look::Idle {
        polygon(cross, t.fill(Look::Idle), (t.fill(Look::Idle), 0.), window);
        let (cx, cy) = Part::DPad.centre();
        let bar = [(cx - 11., cy - 30.), (cx + 11., cy - 30.), (cx + 11., cy + 30.), (cx - 11., cy + 30.)];
        polygon(&bar, t.fill(dl), (t.fill(dl), 0.), window);
        let mut b = PathBuilder::stroke(w(t.line(dl).1));
        b.add_polygon(&cross.iter().map(|q| pad(*q)).collect::<Vec<_>>(), true);
        if let Ok(path) = b.build() {
            window.paint_path(path, t.line(dl).0);
        }
    } else {
        polygon(cross, t.fill(dl), t.line(dl), window);
    }
    // Leader lines, and a dot where each meets its part.
    for lead in &p.leads {
        let color = if lead.picked { t.picked } else { t.lead };
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
    for part in [Part::LB, Part::RB] {
        let l = look(part);
        if l != Look::Idle {
            let (start, segs) = shoulder(part);
            fill_and_line(start, &segs, t.fill(l), t.line(l), window);
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
pub(super) fn map(d: &SettingsPage, cx: &mut Context<SettingsPage>) -> Div {
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
    let mut root = div()
        .relative()
        .w_full()
        .max_w(px(BOX_W))
        // Narrower, the callouts get too short for their text.
        .min_w(px(MIN_W))
        .aspect_ratio(BOX_W / BOX_H)
        .mx_auto()
        .child(canvas(|_, _, _| (), move |bounds, _, window, _| paint(bounds, &paint_data, window)).absolute().inset_0());
    // The round parts as boxes (crisp at any size), and a click area on every part.
    let pad_box = |x: f32, y: f32, wd: f32, ht: f32| {
        let (bx, by) = at_box(x - wd / 2., y - ht / 2.);
        (fx(bx), fy(by), fx(wd * PAD_SCALE), fy(ht * PAD_SCALE))
    };
    for (part, (cx0, cy0)) in [(Part::LStick, Part::LStick.centre()), (Part::RStick, Part::RStick.centre())] {
        let (l, t, wd, ht) = pad_box(cx0, cy0, 60., 60.);
        root =
            root.child(div().absolute().left(l).top(t).w(wd).h(ht).rounded_full().bg(tones.well).border_1().border_color(tones.body_line));
        let look = look_of(part);
        let (lc, lw) = tones.line(look);
        let (l, t, wd, ht) = pad_box(cx0, cy0, 40., 40.);
        root = root.child(div().absolute().left(l).top(t).w(wd).h(ht).rounded_full().bg(tones.fill(look)).border(px(lw)).border_color(lc));
    }
    for part in [Part::A, Part::B, Part::X, Part::Y] {
        let (cx0, cy0) = part.centre();
        let (l, t, wd, ht) = pad_box(cx0, cy0, 26., 26.);
        let look = look_of(part);
        let face: Hsla = match part {
            Part::A => theme::console_btn_a(),
            Part::B => theme::console_btn_b(),
            Part::X => theme::console_btn_x(),
            _ => theme::console_btn_y(),
        }
        .into();
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
            .font_weight(theme::W_BOLD)
            .child(part.cap());
        root = root.child(match look {
            Look::Idle => b.bg(tones.part).border_1().border_color(tones.part_line).text_color(theme::text_dim()),
            Look::Used => b.bg(face).text_color(theme::console_on_focus()),
            Look::Picked => b.bg(face).border_2().border_color(tones.picked).text_color(theme::console_on_focus()),
        });
    }
    for part in [Part::View, Part::Menu] {
        let (cx0, cy0) = part.centre();
        let (l, t, wd, ht) = pad_box(cx0, cy0, 18., 11.);
        let look = look_of(part);
        let (lc, lw) = tones.line(look);
        root = root.child(div().absolute().left(l).top(t).w(wd).h(ht).rounded_full().bg(tones.fill(look)).border(px(lw)).border_color(lc));
    }
    {
        let (cx0, cy0) = Part::Guide.centre();
        let (l, t, wd, ht) = pad_box(cx0, cy0, 30., 30.);
        let look = look_of(Part::Guide);
        let (lc, lw) = tones.line(look);
        let ic = if look == Look::Idle { theme::text_dim() } else { theme::text_primary() };
        root = root.child(
            div()
                .absolute()
                .left(l)
                .top(t)
                .w(wd)
                .h(ht)
                .rounded_full()
                .flex()
                .items_center()
                .justify_center()
                .bg(tones.fill(look))
                .border(px(lw))
                .border_color(lc)
                .child(crate::ui::icon(crate::icons::Icon::SportsEsports, px(12.), ic)),
        );
    }
    // Click areas over every part (the shoulders and the d-pad are painted).
    for part in Part::ALL {
        let (cx0, cy0) = part.centre();
        let (bw, bh) = match part {
            Part::LT | Part::RT => (90., 30.),
            Part::LB | Part::RB => (120., 22.),
            Part::LStick | Part::RStick => (60., 60.),
            Part::DPad => (60., 60.),
            Part::View | Part::Menu => (24., 18.),
            Part::Guide => (30., 30.),
            _ => (28., 28.),
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
        let full = format!("{} · {}", c.cap, c.title);
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
    /// The letter on a face button.
    fn cap(self) -> &'static str {
        match self {
            Part::A => "A",
            Part::B => "B",
            Part::X => "X",
            Part::Y => "Y",
            _ => "",
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
