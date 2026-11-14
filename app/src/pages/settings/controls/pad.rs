//! The drawn controller of Settings → Controls → Controller: the body and
//! its buttons painted with paths, a callout for each button in use on both
//! sides, and a leader line from the button to its callout. "All" puts
//! every context of a button in its callout; the other choices of "Show"
//! one context each, with the Guide chord's free buttons dashed.

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::console_theme::{AButton, ConsoleButton};
use heronite_data::overlay_prefs::ChordButton;
use heronite_i18n::tr;
use heronite_platform::{supported, Feature};

use super::{clash, console_button, shares_guide, toggle_selected, Show};
use crate::pages::settings::{overlay, SettingsPage};
use crate::theme;

/// The drawing's own units: the controller sits in a 960 × 470 box, scaled
/// to the page's width.
const BOX_W: f32 = 960.;
const BOX_H: f32 = 470.;
/// The widest the drawing gets (the page's column).
const MAX_W: f32 = 880.;

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
    /// Every part, in the order the callouts are laid out.
    const ALL: [Part; 14] = [
        Part::LT,
        Part::LB,
        Part::LStick,
        Part::DPad,
        Part::View,
        Part::Guide,
        Part::RT,
        Part::RB,
        Part::Y,
        Part::B,
        Part::A,
        Part::X,
        Part::RStick,
        Part::Menu,
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

    /// Its centre in the drawing's units.
    fn centre(self) -> (f32, f32) {
        match self {
            Part::LT => (333., 111.),
            Part::RT => (627., 111.),
            Part::LB => (345., 143.),
            Part::RB => (615., 143.),
            Part::LStick => (378., 228.),
            Part::RStick => (542., 312.),
            Part::DPad => (425., 312.),
            Part::View => (448., 226.),
            Part::Menu => (512., 226.),
            Part::Guide => (480., 184.),
            Part::Y => (598., 200.),
            Part::B => (628., 228.),
            Part::A => (598., 256.),
            Part::X => (568., 228.),
        }
    }

    /// The side of its callout and the callout's top, in the drawing's units.
    fn slot(self) -> (Side, f32) {
        match self {
            Part::LT => (Side::Left, 36.),
            Part::LB => (Side::Left, 90.),
            Part::LStick => (Side::Left, 186.),
            Part::DPad => (Side::Left, 290.),
            Part::View => (Side::Left, 392.),
            Part::Guide => (Side::Top, 4.),
            Part::RT => (Side::Right, 36.),
            Part::RB => (Side::Right, 90.),
            Part::Y => (Side::Right, 144.),
            Part::B => (Side::Right, 198.),
            Part::A => (Side::Right, 252.),
            Part::X => (Side::Right, 306.),
            Part::RStick => (Side::Right, 362.),
            Part::Menu => (Side::Right, 420.),
        }
    }

    /// Its name before the callout's text (button names stay as printed).
    fn name(self) -> &'static str {
        match self {
            Part::LT => "LT",
            Part::RT => "RT",
            Part::LB => "LB",
            Part::RB => "RB",
            Part::A => "A",
            Part::B => "B",
            Part::X => "X",
            Part::Y => "Y",
            Part::LStick => tr!("L stick"),
            Part::RStick => tr!("R stick"),
            Part::DPad => tr!("D-pad"),
            Part::View => tr!("View"),
            Part::Menu => tr!("Menu"),
            Part::Guide => "Guide",
        }
    }
}

/// What a button does in one context: a line and a smaller one under it.
/// `free`: nothing yet (the Guide chord's buttons), drawn dashed.
pub(super) struct Callout {
    pub part: Part,
    pub title: String,
    pub sub: String,
    pub free: bool,
    /// The line in "All", where the context is said after it.
    short: String,
}

fn callout(part: Part, title: impl Into<String>, sub: impl Into<String>, short: impl Into<String>) -> Callout {
    Callout { part, title: title.into(), sub: sub.into(), free: false, short: short.into() }
}

/// What each button does during a game (the overlay's Guide chords).
fn in_game(d: &SettingsPage) -> Vec<Callout> {
    let mut v = Vec::new();
    if !supported(Feature::Overlay) {
        return v;
    }
    let p = &d.overlay.prefs;
    let waits = if p.enabled { String::new() } else { tr!("Waits for the overlay").to_string() };
    if cfg!(windows) {
        v.push(callout(Part::View, tr!("Back + Start: open the overlay"), tr!("The game also gets them"), tr!("Open the overlay")));
    } else {
        let sub = if p.enabled { tr!("Hold it for the shortcuts") } else { tr!("Waits for the overlay (off)") };
        v.push(callout(Part::Guide, tr!("Press: open the overlay"), sub, tr!("Open the overlay")));
    }
    let shot = overlay::screenshot_button(p);
    if p.screenshot_shortcut {
        v.push(callout(Part::of_chord(shot), overlay::shot_title(p), waits.clone(), tr!("Take a screenshot")));
    }
    if p.volume_shortcut {
        let steps = tr!("{step} steps", step = overlay::step_label(p.volume_step));
        let sub = if p.enabled { steps } else { waits.clone() };
        v.push(callout(Part::DPad, crate::overlay::labels::volume(), sub, tr!("Volume")));
    }
    if p.moment_shortcut && !(p.screenshot_shortcut && shot == ChordButton::Y) {
        v.push(callout(Part::Y, crate::overlay::labels::moment(), waits.clone(), tr!("Mark a moment")));
    }
    if supported(Feature::Replay) && p.replay_shortcut && !(p.screenshot_shortcut && shot == ChordButton::X) {
        v.push(callout(Part::X, tr!("Guide + X: save the replay"), waits, tr!("Save the replay")));
    }
    for b in overlay::screenshot_buttons() {
        let part = Part::of_chord(b);
        if part != Part::DPad && !v.iter().any(|c| c.part == part) {
            v.push(Callout { part, title: tr!("free").into(), sub: String::new(), free: true, short: String::new() });
        }
    }
    v
}

/// What each button does outside games: the one held for console mode, and
/// the overlay when it opens without a game.
fn outside(d: &SettingsPage) -> Vec<Callout> {
    let mut v = Vec::new();
    let button = console_button(d);
    let parts: &[Part] = match button {
        ConsoleButton::BackStart => &[Part::View],
        ConsoleButton::Bumpers => &[Part::LB, Part::RB],
        ConsoleButton::Off => &[],
        _ => &[Part::Guide],
    };
    let both = clash(d);
    for part in parts {
        let (title, short) = match (both, button) {
            (true, ConsoleButton::BackStart) => {
                (tr!("Back + Start: a tap opens the overlay, a hold console mode"), tr!("Tap: overlay · hold: console mode"))
            }
            (true, _) => (tr!("Tap: open the overlay · Hold: console mode"), tr!("Tap: overlay · hold: console mode")),
            (false, ConsoleButton::BackStart) => (tr!("Hold Back + Start: enter console mode"), tr!("Hold: console mode")),
            (false, ConsoleButton::Bumpers) => (tr!("Hold LB + RB: enter console mode"), tr!("Hold: console mode")),
            (false, _) => (tr!("Hold: enter console mode"), tr!("Hold: console mode")),
        };
        let sub = if both { tr!("Outside games too") } else { tr!("Not while a game runs · this PC") };
        v.push(callout(*part, title, sub, short));
    }
    let p = &d.overlay.prefs;
    if supported(Feature::Overlay) && p.enabled && p.without_game && !shares_guide(d) {
        let part = if cfg!(windows) { Part::View } else { Part::Guide };
        if !v.iter().any(|c| c.part == part) {
            v.push(callout(part, tr!("Open the overlay"), tr!("Outside games too"), tr!("Open the overlay")));
        }
    }
    v
}

/// What each button does while the overlay is open.
fn overlay_open() -> Vec<Callout> {
    if !supported(Feature::Overlay) {
        return Vec::new();
    }
    let close = if cfg!(windows) { Part::View } else { Part::Guide };
    vec![
        callout(Part::LStick, tr!("Move"), "", tr!("Move")),
        callout(Part::DPad, tr!("Move"), "", tr!("Move")),
        callout(Part::A, tr!("Select"), "", tr!("Select")),
        callout(Part::B, tr!("Back · close"), "", tr!("Back · close")),
        callout(Part::Y, tr!("Mark a moment in Notes"), "", tr!("Mark a moment in Notes")),
        callout(Part::LB, tr!("Previous tab"), "", tr!("Previous tab")),
        callout(Part::RB, tr!("Next tab"), "", tr!("Next tab")),
        callout(close, tr!("Close the overlay"), "", tr!("Close the overlay")),
    ]
}

/// What each button does in console mode, as its theme has it.
fn console() -> Vec<Callout> {
    let theme = crate::console::look::theme();
    let swap = theme.controls.swap_ab;
    let opens = if theme.nav.a_button == AButton::Details { tr!("Game page") } else { tr!("Play") };
    let other = if theme.nav.a_button == AButton::Details { tr!("Play") } else { tr!("Game page") };
    let (confirm, back) = if swap { (Part::B, Part::A) } else { (Part::A, Part::B) };
    vec![
        callout(confirm, opens, if swap { tr!("Confirms (east)") } else { "" }, opens),
        callout(back, tr!("Back · sidebar"), if swap { tr!("Confirm with the east button is on") } else { "" }, tr!("Back · sidebar")),
        callout(Part::X, other, tr!("The other one of A"), other),
        callout(Part::Y, tr!("Search and filters"), "", tr!("Search and filters")),
        callout(Part::LB, tr!("Previous tab"), "", tr!("Previous tab")),
        callout(Part::RB, tr!("Next tab"), "", tr!("Next tab")),
        callout(Part::LT, tr!("Jump back by letter"), "", tr!("Jump back by letter")),
        callout(Part::RT, tr!("Jump on by letter"), "", tr!("Jump on by letter")),
        callout(Part::Menu, tr!("Menu"), "", tr!("Menu")),
        callout(Part::LStick, tr!("Move"), "", tr!("Move")),
        callout(Part::DPad, tr!("Move"), "", tr!("Move")),
    ]
}

/// Every context of each button in one callout: the first in the title,
/// the next two under it, and how many more.
fn all(d: &SettingsPage) -> Vec<Callout> {
    let layers: [(Vec<Callout>, &'static str); 4] =
        [(in_game(d), ""), (outside(d), tr!("outside games")), (overlay_open(), tr!("overlay")), (console(), tr!("console"))];
    let mut by_part: Vec<(Part, Vec<String>)> = Vec::new();
    for (layer, context) in layers {
        for c in layer.into_iter().filter(|c| !c.free) {
            let text = if context.is_empty() { c.short } else { format!("{} ({context})", c.short) };
            match by_part.iter_mut().find(|(p, _)| *p == c.part) {
                Some((_, lines)) => lines.push(text),
                None => by_part.push((c.part, vec![text])),
            }
        }
    }
    by_part
        .into_iter()
        .map(|(part, lines)| {
            let mut sub = lines[1..lines.len().min(3)].join(" · ");
            if lines.len() > 3 {
                sub.push_str(&format!(" · +{}", lines.len() - 3));
            }
            Callout { part, title: lines[0].clone(), sub, free: false, short: String::new() }
        })
        .collect()
}

/// The callouts of `show`.
pub(super) fn callouts(d: &SettingsPage, show: Show) -> Vec<Callout> {
    match show {
        Show::All => all(d),
        Show::InGame => in_game(d),
        Show::Outside => outside(d),
        Show::Overlay => overlay_open(),
        Show::Console => console(),
    }
}

/// A callout's box in the drawing's units: left, top, width, height.
fn callout_box(c: &Callout) -> (f32, f32, f32, f32) {
    let (side, top) = c.part.slot();
    let h = if c.free {
        26.
    } else if c.sub.is_empty() {
        30.
    } else {
        44.
    };
    match side {
        Side::Left if c.free => (110., top, 120., h),
        Side::Left => (14., top, 250., h),
        Side::Right if c.free => (730., top, 120., h),
        Side::Right => (BOX_W - 14. - 250., top, 250., h),
        Side::Top => (BOX_W / 2. - 200., top, 400., h),
    }
}

/// Where the leader line meets the callout, in the drawing's units.
fn anchor(c: &Callout) -> (f32, f32) {
    let (x, y, w, h) = callout_box(c);
    match c.part.slot().0 {
        Side::Left => (x + w, y + h / 2.),
        Side::Right => (x, y + h / 2.),
        Side::Top => (x + w / 2., y + h),
    }
}

/// The controller's outline, in the drawing's units.
const BODY: [[f32; 6]; 10] = [
    [385., 140., 575., 140., 630., 160.],
    [690., 180., 722., 222., 738., 295.],
    [754., 368., 762., 425., 722., 450.],
    [690., 470., 650., 448., 625., 408.],
    [610., 383., 595., 368., 570., 368.],
    // A straight line drawn as a curve: 570,368 → 390,368.
    [510., 368., 450., 368., 390., 368.],
    [365., 368., 350., 383., 335., 408.],
    [310., 448., 270., 470., 238., 450.],
    [198., 425., 206., 368., 222., 295.],
    [238., 222., 270., 180., 330., 160.],
];

/// The painted look of a part: its shape in the drawing's units.
enum Shape {
    Circle(f32),
    /// Width, height, corner radius.
    Pill(f32, f32, f32),
    Cross,
}

fn shape(p: Part) -> Shape {
    match p {
        Part::LT | Part::RT => Shape::Pill(56., 22., 10.),
        Part::LB | Part::RB => Shape::Pill(80., 18., 9.),
        Part::LStick | Part::RStick => Shape::Circle(27.),
        Part::DPad => Shape::Cross,
        Part::View | Part::Menu => Shape::Circle(10.),
        Part::Guide => Shape::Circle(17.),
        Part::A | Part::B | Part::X | Part::Y => Shape::Circle(13.),
    }
}

/// The outline of `shape` around `(cx, cy)` as points, in the drawing's units.
fn outline(s: &Shape, cx: f32, cy: f32) -> Vec<(f32, f32)> {
    let arc = |ox: f32, oy: f32, r: f32, from: f32, out: &mut Vec<(f32, f32)>| {
        for i in 0..=8 {
            let a = (from + i as f32 * 90. / 8.).to_radians();
            out.push((ox + r * a.cos(), oy + r * a.sin()));
        }
    };
    match *s {
        Shape::Circle(r) => (0..40)
            .map(|i| {
                let a = i as f32 / 40. * std::f32::consts::TAU;
                (cx + r * a.cos(), cy + r * a.sin())
            })
            .collect(),
        Shape::Pill(w, h, r) => {
            let (l, t, rr, b) = (cx - w / 2., cy - h / 2., cx + w / 2., cy + h / 2.);
            let mut v = Vec::new();
            arc(rr - r, b - r, r, 0., &mut v);
            arc(l + r, b - r, r, 90., &mut v);
            arc(l + r, t + r, r, 180., &mut v);
            arc(rr - r, t + r, r, 270., &mut v);
            v
        }
        Shape::Cross => {
            let (x, y) = (cx - 10., cy - 28.);
            [
                (0., 0.),
                (20., 0.),
                (20., 18.),
                (38., 18.),
                (38., 38.),
                (20., 38.),
                (20., 56.),
                (0., 56.),
                (0., 38.),
                (-18., 38.),
                (-18., 18.),
                (0., 18.),
            ]
            .iter()
            .map(|(dx, dy)| (x + dx, y + dy))
            .collect()
        }
    }
}

/// What the painter needs, worked out before painting (colours resolved).
struct Paint {
    scale: f32,
    body_fill: Hsla,
    body_line: Hsla,
    /// Each part, whether it is in use, and whether it is picked.
    parts: Vec<(Part, bool, bool)>,
    /// From, to, dashed, picked.
    leads: Vec<((f32, f32), (f32, f32), bool, bool)>,
    fill: Hsla,
    line: Hsla,
    used_line: Hsla,
    on: Hsla,
    lead: Hsla,
}

fn paint(bounds: Bounds<Pixels>, p: &Paint, window: &mut Window) {
    let o = bounds.origin;
    let at = |(x, y): (f32, f32)| point(o.x + px(x * p.scale), o.y + px(y * p.scale));
    let polygon = |pts: &[(f32, f32)], fill: Option<Hsla>, line: Option<(Hsla, f32)>, window: &mut Window| {
        let points: Vec<Point<Pixels>> = pts.iter().map(|q| at(*q)).collect();
        if let Some(fill) = fill {
            let mut b = PathBuilder::fill();
            b.add_polygon(&points, true);
            if let Ok(path) = b.build() {
                window.paint_path(path, fill);
            }
        }
        if let Some((color, w)) = line {
            let mut b = PathBuilder::stroke(px(w));
            b.add_polygon(&points, true);
            if let Ok(path) = b.build() {
                window.paint_path(path, color);
            }
        }
    };
    // The body: cubic curves from 330,160.
    let body = |builder: PathBuilder| {
        let mut b = builder;
        b.move_to(at((330., 160.)));
        for [ax, ay, bx, by, x, y] in BODY {
            b.cubic_bezier_to(at((x, y)), at((ax, ay)), at((bx, by)));
        }
        b.close();
        b.build()
    };
    if let Ok(path) = body(PathBuilder::fill()) {
        window.paint_path(path, p.body_fill);
    }
    if let Ok(path) = body(PathBuilder::stroke(px(1.5))) {
        window.paint_path(path, p.body_line);
    }
    for (from, to, dashed, picked) in &p.leads {
        let mut b = PathBuilder::stroke(px(if *picked { 1.6 } else { 1.2 }));
        if *dashed {
            b = b.dash_array(&[px(3.), px(4.)]);
        }
        b.move_to(at(*from));
        b.line_to(at(*to));
        if let Ok(path) = b.build() {
            window.paint_path(path, if *picked { p.on } else { p.lead });
        }
    }
    for (part, used, picked) in &p.parts {
        let (cx, cy) = part.centre();
        let pts = outline(&shape(*part), cx, cy);
        let fill = if *picked { p.on } else { p.fill };
        let line = if *used { (p.used_line, 1.6) } else { (p.line, 1.2) };
        polygon(&pts, Some(fill), Some(line), window);
    }
}

/// The drawing, its callouts and their lines, for the page's "Show".
pub(super) fn map(d: &SettingsPage, window: &Window, cx: &mut Context<SettingsPage>) -> Div {
    let show = d.controls.show;
    let picked = d.controls.selected;
    let lines = callouts(d, show);
    // The page's column: the window less the left list and the margins.
    let avail = f32::from(window.viewport_size().width) - 230. - 72. - 24.;
    let w = avail.clamp(560., MAX_W);
    let scale = w / BOX_W;
    let used = |p: Part| lines.iter().any(|c| c.part == p && !c.free);
    let paint_data = Paint {
        scale,
        body_fill: theme::ink_a(0.05),
        body_line: theme::ink_a(0.14),
        parts: Part::ALL.iter().map(|p| (*p, used(*p), picked == Some(*p))).collect(),
        leads: lines.iter().map(|c| (c.part.centre(), anchor(c), c.free, picked == Some(c.part))).collect(),
        fill: theme::ink_a(0.10),
        line: theme::ink_a(0.16),
        used_line: theme::accent_alpha(0.55).into(),
        on: theme::accent().into(),
        lead: theme::ink_a(0.18),
    };
    let s = |v: f32| px(v * scale);
    let mut root = div()
        .relative()
        .flex_none()
        .w(px(w))
        .h(s(BOX_H))
        .mx_auto()
        .child(canvas(|_, _, _| (), move |bounds, _, window, _| paint(bounds, &paint_data, window)).absolute().inset_0());
    // The names on the buttons, and a click area on each.
    for part in Part::ALL {
        let (cx0, cy0) = part.centre();
        let (bw, bh) = match shape(part) {
            Shape::Circle(r) => (r * 2., r * 2.),
            Shape::Pill(w, h, _) => (w, h),
            Shape::Cross => (56., 56.),
        };
        let label = match part {
            Part::LT | Part::RT | Part::LB | Part::RB | Part::A | Part::B | Part::X | Part::Y => part.name(),
            Part::Guide => "⌂",
            _ => "",
        };
        let on = picked == Some(part);
        root = root.child(
            div()
                .id(SharedString::from(format!("controls-pad-{part:?}")))
                .absolute()
                .left(s(cx0 - bw / 2.))
                .top(s(cy0 - bh / 2.))
                .w(s(bw))
                .h(s(bh))
                .flex()
                .items_center()
                .justify_center()
                .text_size(px(10.5))
                .font_weight(theme::W_BOLD)
                .text_color(if on { theme::on_accent() } else { theme::text_muted() })
                .cursor_pointer()
                .child(label)
                .on_click(cx.listener(move |d, _, _, cx| toggle_selected(d, part, cx))),
        );
    }
    for c in &lines {
        let (x, y, cw, ch) = callout_box(c);
        let on = picked == Some(c.part);
        let title = if c.part.slot().0 == Side::Top { c.title.clone() } else { format!("{} · {}", c.part.name(), c.title) };
        let full = if c.sub.is_empty() { title.clone() } else { format!("{title}\n{}", c.sub) };
        let part = c.part;
        let mut b = div()
            .id(SharedString::from(format!("controls-callout-{:?}", c.part)))
            .absolute()
            .left(s(x))
            .top(s(y))
            .w(s(cw))
            .h(px(ch * scale.max(0.9)))
            .px(px(10.))
            .flex()
            .flex_col()
            .justify_center()
            .rounded(px(10.))
            .border_1()
            .cursor_pointer()
            .tooltip(crate::ui::tooltip(full))
            .on_click(cx.listener(move |d, _, _, cx| toggle_selected(d, part, cx)));
        b = if on {
            b.bg(theme::accent_tint()).border_color(theme::accent())
        } else if c.free {
            b.border_dashed().border_color(theme::ink_a(0.14)).opacity(0.75)
        } else {
            b.bg(theme::opaque_card()).border_color(theme::ink_a(0.10))
        };
        let text = |t: String, size: f32, color: Hsla| {
            div().text_size(px(size)).line_height(px(size + 4.)).text_color(color).whitespace_nowrap().truncate().child(t)
        };
        b = b.child(text(title, 12.5, if c.free { theme::text_dim().into() } else { theme::text_primary().into() }));
        if !c.sub.is_empty() {
            b = b.child(text(c.sub.clone(), 11.5, theme::text_muted().into()));
        }
        root = root.child(b);
    }
    let empty = lines.iter().all(|c| c.free);
    div()
        .flex()
        .flex_col()
        .gap(px(6.))
        .child(root)
        .when(empty, |c| c.child(div().flex().justify_center().child(crate::ui::fields::meta_text(tr!("Nothing is set for this.")))))
}
