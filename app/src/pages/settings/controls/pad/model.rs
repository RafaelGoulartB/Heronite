//! The controllers Settings → Controller draws, one per family of buttons
//! (`look::pad_glyphs`): an Xbox Series controller, a DualSense and a
//! Switch Pro Controller, from Zacksly's "Button Icons and Controls" packs
//! (CC BY 3.0) without the makers' logos; the triggers behind the bumpers
//! are ours. For each: where its parts are and where their callouts' lines
//! leave them (in its own units, those of the drawings, 4096 wide), and its
//! shapes, each with what it is (`Role`), painted in the theme's colours.

use std::sync::OnceLock;

use heronite_data::console_theme::Glyphs;

use super::{Part, Side};

/// A drawn controller.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Model {
    Xbox,
    PlayStation,
    Nintendo,
}

/// Where a model sits in the drawing: its point `top` (the middle of its
/// top) is drawn at `origin` in box units, its units times `scale`.
struct Frame {
    top: (f32, f32),
    origin: (f32, f32),
    scale: f32,
}

impl Model {
    #[cfg(test)]
    pub(super) const ALL: [Model; 3] = [Model::Xbox, Model::PlayStation, Model::Nintendo];

    /// The controller drawn for buttons drawn as `style` (the Steam Deck's
    /// are Xbox's).
    pub(super) fn of(style: Glyphs) -> Self {
        match style {
            Glyphs::PlayStation => Model::PlayStation,
            Glyphs::Nintendo => Model::Nintendo,
            _ => Model::Xbox,
        }
    }

    fn frame(self) -> Frame {
        match self {
            Model::Xbox => Frame { top: (2048.5, 482.), origin: (440., 50.), scale: 0.236 },
            // Wider, and its face buttons further out: smaller, so the
            // lines of the right column keep their lanes.
            Model::PlayStation => Frame { top: (2048., 453.), origin: (440., 60.), scale: 0.205 },
            Model::Nintendo => Frame { top: (2048.6, 484.), origin: (440., 50.), scale: 0.236 },
        }
    }

    /// Box units per pad unit.
    pub(super) fn scale(self) -> f32 {
        self.frame().scale
    }

    /// A pad point in box units.
    pub(super) fn at_box(self, x: f32, y: f32) -> (f32, f32) {
        let f = self.frame();
        (f.origin.0 + (x - f.top.0) * f.scale, f.origin.1 + (y - f.top.1) * f.scale)
    }

    /// `x` on the other side of the controller.
    fn mirror(self, x: f32) -> f32 {
        2. * self.frame().top.0 - x
    }

    /// A part's centre in pad units. The face buttons by position: A south,
    /// B east, X west, Y north.
    pub(super) fn centre(self, part: Part) -> (f32, f32) {
        let m = |x: f32| self.mirror(x);
        match self {
            Model::Xbox => match part {
                Part::LT => (1640., 475.),
                Part::RT => (m(1640.), 475.),
                Part::LB => (1617., 548.),
                Part::RB => (m(1617.), 548.),
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
            },
            Model::PlayStation => match part {
                Part::LT => (1445., 440.),
                Part::RT => (m(1445.), 440.),
                Part::LB => (1435., 497.),
                Part::RB => (m(1435.), 497.),
                Part::LStick => (1724.45, 1094.15),
                Part::RStick => (2369.53, 1093.15),
                Part::DPad => (1425.18, 817.6),
                Part::Y => (2667.9, 672.1),
                Part::A => (2667.9, 966.45),
                Part::X => (2520.73, 819.27),
                Part::B => (2815.08, 819.27),
                Part::View => (1582., 596.),
                Part::Menu => (m(1582.), 596.),
                Part::Guide => (2041.6, 1086.),
            },
            Model::Nintendo => match part {
                Part::LT => (1580., 485.),
                Part::RT => (m(1580.), 485.),
                Part::LB => (1585., 560.),
                Part::RB => (m(1585.), 560.),
                Part::LStick => (1578.38, 840.8),
                Part::RStick => (2274.71, 1072.),
                Part::DPad => (1784.2, 1076.5),
                Part::Y => (2501.16, 725.13),
                Part::A => (2501.16, 956.47),
                Part::X => (2370.5, 840.8),
                Part::B => (2636.83, 840.8),
                Part::View => (1837.08, 711.42),
                Part::Menu => (2260.38, 711.42),
                Part::Guide => (2171.81, 840.8),
            },
        }
    }

    /// A part's callout side, where its line leaves it (on its rim) and
    /// where the line bends, in pad units. The face buttons' lines leave
    /// between their neighbours, so no line crosses another button.
    pub(super) fn route(self, part: Part) -> (Side, (f32, f32), (f32, f32)) {
        let m = |x: f32| self.mirror(x);
        let top = |p: (f32, f32), r: f32| (Side::Top, (p.0, p.1 - r), (p.0, p.1 - r));
        match self {
            Model::Xbox => match part {
                Part::LT => (Side::Left, (1505., 520.), (1430., 470.)),
                Part::LB => (Side::Left, (1455., 610.), (1380., 600.)),
                Part::LStick => (Side::Left, (1507.4, 815.86), (1330., 818.)),
                Part::DPad => (Side::Left, (1676.4, 1091.48), (1450., 1100.)),
                Part::RT => (Side::Right, (m(1505.), 520.), (m(1430.), 470.)),
                Part::RB => (Side::Right, (2642., 610.), (2717., 600.)),
                Part::Y => (Side::Right, (2531.6, 654.5), (2580., 620.)),
                Part::X => (Side::Right, (2426., 781.3), (2560., 735.)),
                Part::B => (Side::Right, (2663.4, 810.52), (2700., 812.)),
                Part::A => (Side::Right, (2531.6, 965.9), (2580., 990.)),
                Part::RStick => (Side::Right, (2370.75, 1073.), (2560., 1110.)),
                Part::Menu | Part::View | Part::Guide => top(self.centre(part), self.small_r(part)),
            },
            Model::PlayStation => match part {
                Part::LT => (Side::Left, (1330., 478.), (1250., 445.)),
                Part::LB => (Side::Left, (1312., 515.), (1230., 530.)),
                Part::DPad => (Side::Left, (1265., 817.6), (1150., 830.)),
                Part::LStick => (Side::Left, (1609.3, 1094.15), (1300., 1180.)),
                Part::RT => (Side::Right, (m(1330.), 478.), (m(1250.), 445.)),
                Part::RB => (Side::Right, (m(1312.), 515.), (m(1230.), 530.)),
                Part::Y => (Side::Right, (2710.7, 629.3), (2770., 600.)),
                Part::X => (Side::Right, (2563.5, 776.5), (2700., 740.)),
                Part::B => (Side::Right, (2875.6, 819.27), (2885., 822.)),
                Part::A => (Side::Right, (2710.7, 1009.2), (2770., 1040.)),
                Part::RStick => (Side::Right, (2484.65, 1093.15), (2700., 1180.)),
                Part::Menu | Part::View | Part::Guide => top(self.centre(part), self.small_r(part)),
            },
            Model::Nintendo => match part {
                Part::LT => (Side::Left, (1440., 556.), (1357., 520.)),
                Part::LB => (Side::Left, (1400., 615.), (1320., 610.)),
                Part::LStick => (Side::Left, (1478.47, 840.8), (1300., 843.)),
                Part::DPad => (Side::Left, (1657.8, 1076.5), (1400., 1085.)),
                Part::RT => (Side::Right, (m(1440.), 556.), (m(1357.), 520.)),
                Part::RB => (Side::Right, (m(1400.), 615.), (m(1320.), 610.)),
                Part::Y => (Side::Right, (2543.7, 682.6), (2600., 650.)),
                Part::X => (Side::Right, (2422.6, 810.7), (2580., 765.)),
                Part::B => (Side::Right, (2696.96, 840.8), (2720., 843.)),
                Part::A => (Side::Right, (2543.7, 999.), (2600., 1025.)),
                Part::RStick => (Side::Right, (2374.62, 1072.), (2580., 1110.)),
                Part::Menu | Part::View | Part::Guide => top(self.centre(part), self.small_r(part)),
            },
        }
    }

    /// The radius of a face button.
    pub(super) fn face_r(self) -> f32 {
        match self {
            Model::Xbox => 57.76,
            Model::PlayStation => 60.53,
            Model::Nintendo => 60.13,
        }
    }

    /// The radius of View, Menu or Guide (a box's half height for
    /// PlayStation's Create and Options).
    fn small_r(self, part: Part) -> f32 {
        match (self, part) {
            (Model::Xbox, Part::Guide) => 65.67,
            (Model::Xbox, _) => 40.03,
            (Model::PlayStation, Part::Guide) => 46.,
            (Model::PlayStation, _) => 26.,
            (Model::Nintendo, _) => 35.34,
        }
    }

    /// The radius of a stick's well and cap.
    fn stick(self) -> (f32, f32) {
        match self {
            Model::Xbox => (94.87, 63.82),
            Model::PlayStation => (115.12, 69.96),
            Model::Nintendo => (99.91, 70.64),
        }
    }

    /// Guide is drawn blank and Settings puts its gamepad icon in it (the
    /// Xbox and PS logos are left out); the Pro Controller's Home is a house.
    pub(super) fn guide_icon(self) -> Option<f32> {
        (self != Model::Nintendo).then(|| self.small_r(Part::Guide))
    }

    /// The size of a part's click area, in pad units.
    pub(super) fn hit(self, part: Part) -> (f32, f32) {
        let (well, _) = self.stick();
        match (self, part) {
            (_, Part::LT | Part::RT) => (210., 90.),
            (Model::PlayStation, Part::LB | Part::RB) => (260., 70.),
            (_, Part::LB | Part::RB) => (330., 80.),
            (_, Part::LStick | Part::RStick) => (2. * well, 2. * well),
            (Model::PlayStation, Part::DPad) => (330., 330.),
            (_, Part::DPad) => (280., 280.),
            (Model::PlayStation, Part::View | Part::Menu) => (70., 90.),
            (_, Part::View | Part::Menu | Part::Guide) => {
                let r = self.small_r(part) + 6.;
                (2. * r, 2. * r)
            }
            _ => (2. * self.face_r(), 2. * self.face_r()),
        }
    }

    /// The drawing's shapes in pad units, in the order they are painted:
    /// read once from `svg` (`usvg` turns its circles, arcs and rounded
    /// boxes into curves), painted as paths by `pad::paint_controller` in
    /// the theme's colours. No picture is made: nothing to rasterize or
    /// keep when the lit buttons change.
    pub(super) fn shapes(self) -> &'static [Shape] {
        static XBOX: OnceLock<Vec<Shape>> = OnceLock::new();
        static PLAYSTATION: OnceLock<Vec<Shape>> = OnceLock::new();
        static NINTENDO: OnceLock<Vec<Shape>> = OnceLock::new();
        let cell = match self {
            Model::Xbox => &XBOX,
            Model::PlayStation => &PLAYSTATION,
            Model::Nintendo => &NINTENDO,
        };
        cell.get_or_init(|| {
            let (svg, roles) = self.svg();
            parse(&svg, &roles)
        })
    }

    /// The drawing as an SVG in pad units, only its geometry: each element's
    /// id (`s<n>`) is its index in the roles returned.
    fn svg(self) -> (String, Vec<Role>) {
        let mut s = Svg { o: String::with_capacity(16000), roles: Vec::new(), model: self };
        // Painted or not, `usvg` keeps only what has a fill or a stroke.
        s.o.push_str("<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 4096 2160'><g fill='#000' stroke='#000'>");
        match self {
            Model::Xbox => xbox(&mut s),
            Model::PlayStation => playstation(&mut s),
            Model::Nintendo => nintendo(&mut s),
        }
        for part in [Part::A, Part::B, Part::X, Part::Y] {
            let (cx, cy) = self.centre(part);
            s.el(Role::Seat, &format!("circle cx='{cx}' cy='{cy}' r='{:.1}'", self.face_r() + 6.));
        }
        s.o.push_str("</g></svg>");
        (s.o, s.roles)
    }
}

/// What a shape of a drawing is, which says how it is painted.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Role {
    /// A part: filled and outlined in its look.
    Part(Part),
    /// A button that is never lit (Share, the touchpad, Capture...).
    Idle,
    /// A trigger: in the shade behind the bumpers while idle.
    Trigger(Part),
    /// The line across a trigger.
    Ridge(Part),
    Body,
    /// A recess (a stick's or the d-pad's well).
    Well,
    /// Seams and edges of the shell: thin lines.
    Seam,
    /// The speaker's holes.
    Dots,
    /// The shade under a face button.
    Seat,
    /// Lines on parts (View's icon...), in the brighter look of the two
    /// (`None`: idle).
    Marks(Option<Part>, Option<Part>),
    /// The d-pad's arrows, filled.
    Arrows,
    /// One of the DualSense's d-pad buttons: up or down (`true`), or not.
    Arm(bool),
    /// A one-piece d-pad; its vertical arm, lit alone for up and down; and
    /// its outline again over that arm.
    Cross,
    CrossArm,
    CrossLine,
}

/// A piece of a shape's outline, in pad units.
#[derive(Clone, Copy, Debug)]
pub(super) enum Seg {
    Move(f32, f32),
    Line(f32, f32),
    Quad((f32, f32), (f32, f32)),
    Cubic((f32, f32), (f32, f32), (f32, f32)),
    Close,
}

/// A shape of a drawing.
pub(super) struct Shape {
    pub role: Role,
    pub segs: Vec<Seg>,
}

/// The shapes of `svg`, in order, with their roles (by id).
fn parse(svg: &str, roles: &[Role]) -> Vec<Shape> {
    fn walk(g: &usvg::Group, roles: &[Role], out: &mut Vec<Shape>) {
        for node in g.children() {
            match node {
                usvg::Node::Group(g) => walk(g, roles, out),
                usvg::Node::Path(p) => {
                    let Some(&role) = p.id().strip_prefix('s').and_then(|n| n.parse::<usize>().ok()).and_then(|i| roles.get(i)) else {
                        continue;
                    };
                    let ts = p.abs_transform();
                    let at = |q: usvg::tiny_skia_path::Point| (ts.sx * q.x + ts.kx * q.y + ts.tx, ts.ky * q.x + ts.sy * q.y + ts.ty);
                    use usvg::tiny_skia_path::PathSegment as S;
                    let segs = p
                        .data()
                        .segments()
                        .map(|seg| match seg {
                            S::MoveTo(a) => Seg::Move(at(a).0, at(a).1),
                            S::LineTo(a) => Seg::Line(at(a).0, at(a).1),
                            S::QuadTo(c, a) => Seg::Quad(at(c), at(a)),
                            S::CubicTo(c1, c2, a) => Seg::Cubic(at(c1), at(c2), at(a)),
                            S::Close => Seg::Close,
                        })
                        .collect();
                    out.push(Shape { role, segs });
                }
                _ => {}
            }
        }
    }
    let mut out = Vec::new();
    match usvg::Tree::from_str(svg, &usvg::Options::default()) {
        Ok(tree) => walk(tree.root(), roles, &mut out),
        Err(e) => log::warn!("the controller drawing does not read: {e}"),
    }
    out
}

/// A drawing's SVG being written: its elements, each with the id of its role.
struct Svg {
    o: String,
    roles: Vec<Role>,
    model: Model,
}

impl Svg {
    /// `el` (an element's name and geometry: "circle cx=…") as `role`.
    fn el(&mut self, role: Role, el: &str) {
        self.o.push_str(&format!("<{el} id='s{}'/>", self.roles.len()));
        self.roles.push(role);
    }

    fn path(&mut self, role: Role, d: &str) {
        self.el(role, &format!("path d='{d}'"));
    }

    fn circle(&mut self, role: Role, (cx, cy): (f32, f32), r: f32) {
        self.el(role, &format!("circle cx='{cx}' cy='{cy}' r='{r}'"));
    }

    /// Each element of `inner` ("<line …/><rect …/>") as `role`.
    fn each(&mut self, role: Role, inner: &str) {
        for el in inner.split('<').map(|e| e.trim().trim_end_matches("/>").trim_end_matches('/').trim()).filter(|e| !e.is_empty()) {
            self.el(role, el);
        }
    }

    /// The triggers: `pts` the left one's outline (a start, then three
    /// points a curve), `ridge` a line across it.
    fn triggers(&mut self, pts: &[(f32, f32)], ridge: &[(f32, f32)]) {
        for (part, right) in [(Part::LT, false), (Part::RT, true)] {
            let (outline, line) = (self.curves(pts, right, true), self.curves(ridge, right, false));
            self.path(Role::Trigger(part), &outline);
            self.path(Role::Ridge(part), &line);
        }
    }

    /// A path of cubic curves through `pts` (a start, then three points a
    /// curve), on the right side when `right`.
    fn curves(&self, pts: &[(f32, f32)], right: bool, close: bool) -> String {
        let x = |v: f32| if right { self.model.mirror(v) } else { v };
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

    /// The sticks: a well and a cap each.
    fn sticks(&mut self) {
        let (well, cap) = self.model.stick();
        for part in [Part::LStick, Part::RStick] {
            let c = self.model.centre(part);
            self.circle(Role::Well, c, well);
            self.circle(Role::Part(part), c, cap);
        }
    }

    /// A one-piece d-pad (`cross`) and its vertical arm (left, top, width,
    /// height, corner radius).
    fn cross(&mut self, cross: &str, arm: (f32, f32, f32, f32, f32)) {
        self.path(Role::Cross, cross);
        let (x, y, w, h, r) = arm;
        self.el(Role::CrossArm, &format!("rect x='{x}' y='{y}' width='{w}' height='{h}' rx='{r}'"));
        self.path(Role::CrossLine, cross);
    }
}

fn xbox(s: &mut Svg) {
    s.triggers(&XBOX_TRIGGER, &XBOX_RIDGE);
    for (part, right) in [(Part::LB, false), (Part::RB, true)] {
        let d = s.curves(&XBOX_BUMPER, right, true);
        s.path(Role::Part(part), &d);
    }
    s.path(Role::Body, XBOX_BODY);
    s.sticks();
    s.circle(Role::Well, Model::Xbox.centre(Part::DPad), 140.02);
    s.cross(XBOX_CROSS, (1770.8, 963.9, 91.8, 255.2, 14.));
    for part in [Part::View, Part::Menu] {
        s.circle(Role::Part(part), Model::Xbox.centre(part), 40.03);
    }
    s.path(Role::Idle, XBOX_SHARE);
    // View's and Menu's marks, and Share's (never lit).
    s.each(Role::Marks(Some(Part::View), Some(Part::Menu)), XBOX_MARKS);
    s.circle(Role::Part(Part::Guide), Model::Xbox.centre(Part::Guide), 65.67);
}

fn playstation(s: &mut Svg) {
    s.triggers(&PS_TRIGGER, &PS_RIDGE);
    s.path(Role::Part(Part::LB), PS_L1);
    s.path(Role::Part(Part::RB), PS_R1);
    s.path(Role::Body, PS_BODY);
    for d in [PS_SEAM_L, PS_SEAM_R] {
        s.path(Role::Seam, d);
    }
    // The touchpad, the microphone button and the speaker: never lit.
    s.path(Role::Idle, PS_TOUCHPAD);
    s.path(Role::Seam, PS_PAD_EDGE);
    s.path(Role::Idle, PS_MIC);
    s.each(Role::Dots, PS_DOTS);
    // Four separate arrows.
    for (d, vertical) in [(PS_UP, true), (PS_DOWN, true), (PS_LEFT, false), (PS_RIGHT, false)] {
        s.path(Role::Arm(vertical), d);
    }
    s.each(Role::Arrows, PS_ARROWS);
    s.path(Role::Part(Part::View), PS_CREATE);
    s.each(Role::Marks(Some(Part::View), None), PS_CREATE_MARKS);
    s.path(Role::Part(Part::Menu), PS_OPTIONS);
    s.each(Role::Marks(Some(Part::Menu), None), PS_OPTIONS_MARKS);
    s.sticks();
    s.circle(Role::Part(Part::Guide), Model::PlayStation.centre(Part::Guide), 46.);
}

fn nintendo(s: &mut Svg) {
    s.triggers(&PRO_TRIGGER, &PRO_RIDGE);
    s.path(Role::Part(Part::LB), PRO_L);
    s.path(Role::Part(Part::RB), PRO_R);
    s.path(Role::Body, PRO_BODY);
    for d in [PRO_SEAM_L, PRO_SEAM_R] {
        s.path(Role::Seam, d);
    }
    s.sticks();
    s.cross(PRO_CROSS, (1740.6, 950.1, 87.3, 252.8, 16.9));
    s.each(Role::Arrows, PRO_ARROWS);
    let n = Model::Nintendo;
    s.circle(Role::Part(Part::View), n.centre(Part::View), 35.34);
    s.each(Role::Marks(Some(Part::View), None), PRO_MINUS_MARKS);
    s.circle(Role::Part(Part::Menu), n.centre(Part::Menu), 35.34);
    s.each(Role::Marks(Some(Part::Menu), None), PRO_PLUS_MARKS);
    s.circle(Role::Part(Part::Guide), n.centre(Part::Guide), 35.34);
    s.each(Role::Marks(Some(Part::Guide), None), PRO_HOME_MARKS);
    // Capture: never lit.
    s.each(Role::Idle, PRO_CAPTURE);
    s.each(Role::Marks(None, None), PRO_CAPTURE_MARKS);
}

// The outlines, in pad units. Xbox: the body, the left bumper (mirrored for
// the right one), the d-pad's cross, View's, Menu's and Share's marks and
// Share's pill; the left trigger and its ridge are ours.
const XBOX_BODY: &str = "M2662,630.58c-136.49-129.36-351.3-108-351.3-108L2049,515.49v0l-.51,0-.5,0v0l-261.66,7.09s-214.81-21.36-351.3,108c0,0-265.38,583.77-246,815.34,0,0-9.13,188.61,171.26,231.58,0,0,95.74-55.93,242.11-252.94,0,0,68-93.36,177.23-84.66l268.39.79h1l268.39-.79c109.18-8.7,177.23,84.66,177.23,84.66,146.37,197,242.11,252.94,242.11,252.94,180.39-43,171.26-231.58,171.26-231.58C2927.35,1214.35,2662,630.58,2662,630.58Z";
const XBOX_BUMPER: [(f32, f32); 7] =
    [(1443., 623.26), (1486.32, 485.67), (1720., 482.5), (1737.41, 485.), (1742.55, 489.15), (1772.85, 519.33), (1791.65, 522.44)];
const XBOX_TRIGGER: [(f32, f32); 13] = [
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
const XBOX_RIDGE: [(f32, f32); 4] = [(1560., 500.), (1590., 468.), (1640., 452.), (1700., 450.)];
const XBOX_CROSS: &str = "M1940.49,1053.79s-3-7.32-12.85-7.91h-65v-65c-.6-9.89-7.91-12.86-7.91-12.86-7.12-4.15-37-4.15-37-4.15s-31.85,0-39,4.15c0,0-7.32,3-7.91,12.86v65h-65c-9.89.59-12.86,7.91-12.86,7.91-4.15,7.12-4.15,37-4.15,37s0,31.85,4.15,39c0,0,3,7.31,12.86,7.91h65v65c.59,9.890,7.91,12.85,7.91,12.85,7.12,4.16,37,4.16,37,4.16s31.85,0,39-4.16c0,0,7.31-3,7.91-12.85v-65h65c9.89-.6,12.85-7.91,12.85-7.91,4.16-7.12,4.16-37,4.16-37S1944.65,1060.91,1940.49,1053.79Z";
const XBOX_MARKS: &str = "<line x1='2151.97' y1='807.76' x2='2194.09' y2='807.76'/><line x1='2151.97' y1='796.43' x2='2194.09' y2='796.43'/><line x1='2151.97' y1='819.09' x2='2194.09' y2='819.09'/><rect x='1917.97' y='805.21' width='21.22' height='16.29'/><polyline points='1910.96 807.76 1904.33 807.76 1904.33 790.43 1925.93 790.43 1925.93 796.55'/><polyline points='2025.55 898.43 2025.55 914.16 2070.45 914.16 2070.45 898.43'/><line x1='2048' y1='905.39' x2='2048' y2='890.81'/><polyline points='2038.78 900.02 2048 890.81 2057.22 900.02'/>";
const XBOX_SHARE: &str = "M2069.1,879.86h-45.2a24.92,24.92,0,0,0,0,49.84h45.2a24.92,24.92,0,1,0,0-49.84Z";

// PlayStation (DualSense): body, touchpad, the d-pad's four arrows, L1 and
// R1, Create and Options, the microphone button, seams, the speaker's dots
// and the marks; the PS logo is left out. The left trigger (L2) is ours.
const PS_TRIGGER: [(f32, f32); 13] = [
    (1320., 500.),
    (1340., 448.),
    (1410., 410.),
    (1480., 405.),
    (1530., 402.),
    (1556., 418.),
    (1560., 448.),
    (1561., 458.),
    (1562., 466.),
    (1563., 474.),
    (1500., 466.),
    (1400., 472.),
    (1320., 500.),
];
const PS_RIDGE: [(f32, f32); 4] = [(1370., 462.), (1400., 440.), (1440., 428.), (1495., 426.)];

// Nintendo (Switch Pro Controller): L and R, body, seams, the d-pad's cross
// and arrows, Capture and the marks of −, + and Home; the Switch logo is
// left out. The left trigger (ZL) is ours.
const PRO_TRIGGER: [(f32, f32); 13] = [
    (1430., 560.),
    (1460., 500.),
    (1540., 452.),
    (1630., 446.),
    (1680., 443.),
    (1712., 456.),
    (1722., 488.),
    (1723.3, 494.),
    (1724.7, 500.),
    (1726., 506.),
    (1640., 500.),
    (1530., 515.),
    (1430., 560.),
];
const PRO_RIDGE: [(f32, f32); 4] = [(1500., 512.), (1530., 482.), (1580., 466.), (1640., 462.)];
const PS_BODY: &str = "M2821.66,546.32c-61.38-43.08-278-60.4-293.8-59.34s-32.05,9.49-32.05,9.49c-22.55-7.12-83.07-16.61-83.07-16.61C2376,466.8,2048,458.49,2048,458.49s-328,8.31-364.74,21.37c0,0-60.52,9.49-83.07,16.61,0,0-16.22-8.44-32-9.49s-232.42,16.26-293.8,59.34c-51.56,48.66-377.93,940-52.35,1171,0,0,8.31,4.48,26.5,0l109.46,14.5s29.53,0,136.14-322.38c0,0,30.93-107.9,148.82-123.73,0,0,55.78,14.25,87.83,14.25l317.26-2.38,317.26,2.38c32,0,87.83-14.25,87.83-14.25,117.89,15.83,148.82,123.73,148.82,123.73,106.61,322.38,136.14,322.38,136.14,322.38l109.46-14.5c18.19,4.48,26.5,0,26.5,0C3199.59,1486.27,2873.22,595,2821.66,546.32Z";
const PS_TOUCHPAD: &str = "M2446.36,528.91l-.06-.3c.6-17-2.64-40.77-23.6-49.54-7.28-3.05-28.57-5.62-57.54-7.79-96.28-9.84-316.91-10.41-316.91-10.41h-.45s-220.63.57-316.91,10.41c-29,2.17-50.27,4.74-57.54,7.79-21,8.77-24.2,32.57-23.6,49.54,0,.1,0,.2-.07.3l.1.48a105.85,105.85,0,0,0,2.21,17.72l52.22,255.56q1.63,5.87,3.63,11.19A112,112,0,0,0,1718,834.93a106.93,106.93,0,0,0,26,29.1,110.63,110.63,0,0,0,34.7,18.12,119.88,119.88,0,0,0,15.93,4c2.53.46,5.11.87,7.78,1.21h491.27c2.66-.34,5.25-.75,7.78-1.21a119.88,119.88,0,0,0,15.93-4,110.63,110.63,0,0,0,34.7-18.12,106.74,106.74,0,0,0,26-29.1,111.54,111.54,0,0,0,10.17-21.07c1.32-3.55,2.54-7.28,3.63-11.19l52.22-255.56a105.85,105.85,0,0,0,2.21-17.72Z";
const PS_UP: &str = "M1449.18,658.32s-11.48-1.24-24-1.24-24,1.24-24,1.24-26.51,1.17-28.49,23.66l1.59,48.59a24.8,24.8,0,0,0,5.44,14.73l35,38.58a6.74,6.74,0,0,0,4.16,2.45,40.6,40.6,0,0,0,6.35.53,41.32,41.32,0,0,0,7.07-.67,4.91,4.91,0,0,0,3-1.79l34.52-38a28.88,28.88,0,0,0,6.33-17.17l1.55-47.22C1475.15,659.49,1449.18,658.32,1449.18,658.32Z";
const PS_DOWN: &str = "M1401.18,976.94s11.47,1.25,24,1.25,24-1.25,24-1.25,26.5-1.17,28.48-23.65l-1.59-48.6a24.79,24.79,0,0,0-5.43-14.73l-35-38.58a6.68,6.68,0,0,0-4.16-2.44,39.28,39.28,0,0,0-6.34-.53,41.49,41.49,0,0,0-7.08.66,5,5,0,0,0-3,1.79l-34.51,38a28.85,28.85,0,0,0-6.34,17.17l-1.55,47.23C1375.2,975.77,1401.18,976.94,1401.18,976.94Z";
const PS_LEFT: &str = "M1265.87,793.63s-1.25,11.48-1.25,24,1.25,24,1.25,24,1.17,26.51,23.65,28.49l48.6-1.6a24.73,24.73,0,0,0,14.73-5.43l38.58-35a6.68,6.68,0,0,0,2.44-4.16,39.42,39.42,0,0,0,.53-6.35,41.46,41.46,0,0,0-.66-7.07,5.06,5.06,0,0,0-1.79-3l-38-34.51a28.87,28.87,0,0,0-17.18-6.33l-47.22-1.55C1267,767.65,1265.87,793.63,1265.87,793.63Z";
const PS_RIGHT: &str = "M1584.48,841.63s1.25-11.47,1.25-24-1.25-24-1.25-24-1.17-26.5-23.65-28.48l-48.6,1.59a24.79,24.79,0,0,0-14.72,5.43l-38.59,35a6.77,6.77,0,0,0-2.44,4.16,39.28,39.28,0,0,0-.53,6.34,41.21,41.21,0,0,0,.67,7.07,5,5,0,0,0,1.78,3l38,34.51a28.94,28.94,0,0,0,17.18,6.34l47.22,1.55C1583.31,867.61,1584.48,841.63,1584.48,841.63Z";
const PS_L1: &str =
    "M1306.92,530.22v-24s64.45-44.31,142-52.75,109.72,5,109.72,5a57.45,57.45,0,0,1,4.74,28.49S1402.82,497.78,1306.92,530.22Z";
const PS_R1: &str =
    "M2787.64,530.22v-24s-64.45-44.31-142-52.75-109.71,5-109.71,5a57.52,57.52,0,0,0-4.75,28.49S2691.74,497.78,2787.64,530.22Z";
const PS_CREATE: &str = "M1607.55,617.23h0l-9.67-43.31a21.52,21.52,0,0,0-1.8-8.07v0h0a21.62,21.62,0,0,0-39.59,17.37l8.73,41.78a21.42,21.42,0,0,0,1,5l0,.1h0a21.61,21.61,0,0,0,41.25-12.86Z";
const PS_OPTIONS: &str = "M2485.47,617.23h0l9.67-43.31a21.52,21.52,0,0,1,1.8-8.07v0h0a21.62,21.62,0,0,1,39.6,17.37L2527.81,625a21.43,21.43,0,0,1-1.06,5l0,.1h0a21.61,21.61,0,0,1-41.25-12.86Z";
const PS_MIC: &str = "M2086.58,1188.78h-77.16a9.1,9.1,0,0,0,0,18.2h77.16a9.1,9.1,0,0,0,0-18.2Z";
const PS_SEAM_L: &str = "M1600.19,496.47s27,16.09,28.06,40.35l47,224.18s25.58,87.41-28,144c-10.25,10.84-135,159.82-177.23,253.71s-42.2,72.27-119.74,272.18c0,0-55.91,173.54-67,214.15s-15.78,52.22-34.8,72.27";
const PS_SEAM_R: &str = "M2496.56,496.47s-27,16.09-28.07,40.35L2421.55,761S2396,848.41,2449.5,905c10.26,10.84,135,159.82,177.23,253.71s42.2,72.27,119.74,272.18c0,0,55.91,173.54,67,214.15s15.78,52.22,34.79,72.27";
const PS_PAD_EDGE: &str = "M1679.49,783s16.81,118.36,114.92,131h505.16s93.78-9.76,118-128.19";
const PS_DOTS: &str = "<circle cx='1974.8' cy='947.2' r='10.29'/><circle cx='2009.97' cy='947.2' r='10.29'/><circle cx='2048.51' cy='947.2' r='10.29'/><circle cx='2083.78' cy='947.2' r='10.29'/><circle cx='2119.04' cy='947.2' r='10.29'/><circle cx='2102.02' cy='978.03' r='10.29'/><circle cx='1992.53' cy='978.03' r='10.29'/><circle cx='2028.63' cy='978.03' r='10.29'/><circle cx='2065.33' cy='978.03' r='10.29'/>";
const PS_ARROWS: &str = "<path d='M1425.87,956.56l12.29-17.79a1.35,1.35,0,0,0-1.28-2H1412.3a1.34,1.34,0,0,0-1.28,2l12.29,17.79A1.6,1.6,0,0,0,1425.87,956.56Z'/><path d='M1423.31,682,1411,699.78a1.34,1.34,0,0,0,1.28,2h24.58a1.35,1.35,0,0,0,1.28-2L1425.87,682A1.6,1.6,0,0,0,1423.31,682Z'/><path d='M1553.23,813.85l-16-13.64c-.8-.68-1.83.12-1.83,1.42v27.28c0,1.31,1,2.11,1.83,1.42l16-13.63A2,2,0,0,0,1553.23,813.85Z'/><path d='M1296.12,819.06l16,13.63c.8.69,1.83-.11,1.83-1.42V804c0-1.3-1-2.1-1.83-1.42l-16,13.64A2,2,0,0,0,1296.12,819.06Z'/>";
const PS_CREATE_MARKS: &str = "<line x1='1573.88' y1='538.05' x2='1573.88' y2='520.75'/><line x1='1564.16' y1='540.2' x2='1556.07' y2='528.83'/><line x1='1582.85' y1='540.2' x2='1590.56' y2='528.83'/>";
const PS_OPTIONS_MARKS: &str = "<line x1='2505.94' y1='531.38' x2='2532.87' y2='531.38'/><line x1='2505.94' y1='521.21' x2='2532.87' y2='521.21'/><line x1='2505.94' y1='541.55' x2='2532.87' y2='541.55'/>";
const PRO_L: &str = "M1404.59,610.8c43.07-43.91,147.53-67.61,260.35-80.32l5.36-.59.62-.07c39.26-4.29,79.4-7.26,118.24-9.31C1759,511,1719,487.24,1719,487.24l-20-3.16c-210.46-1.59-285.89,86-285.89,86l-28.81,52.62A132.91,132.91,0,0,0,1404.59,610.8Z";
const PRO_R: &str = "M2424.51,529.82l.63.07,5.36.59c112.82,12.71,217.28,36.41,260.35,80.32a135.13,135.13,0,0,0,22.22,12.74l-29.3-53.49s-75.43-87.56-285.89-86l-20,3.16s-40.23,23.87-70.39,33.34C2345.93,522.63,2385.65,525.57,2424.51,529.82Z";
const PRO_BODY: &str = "M2906,1450.71a324.11,324.11,0,0,0-2.44-40.45q-3.27-26.61-8.06-53.23c-53.82-362.57-118-683.66-169-728a145.64,145.64,0,0,1-35.6-18.2c-43.07-43.91-147.53-67.61-260.35-80.32l-5.36-.59-.63-.07c-40.61-4.44-82.18-7.47-122.27-9.53l-2.29-.11-.87,0c-119.32-6-224.74-3.51-251.08-3.87h-.56c-26.34.36-131.77-2.14-251.08,3.87l-.87,0-2.3.11c-40.08,2.06-81.65,5.09-122.27,9.53l-.62.07-5.36.59c-112.82,12.71-217.28,36.41-260.35,80.32A145.64,145.64,0,0,1,1369,629c-52.36,45.54-118.58,382.55-173.24,756.74q-2.3,12.12-3.84,24.52a326.06,326.06,0,0,0-2.43,40.46,308.86,308.86,0,0,0,12.54,89.17c11.86,40.14,33.39,82.87,72.42,106.6,0,0,35.41,24.13,81.28,24.25,34.35.24,74.66-12.93,110.36-59.85L1583.57,1383s29.67-64.09,102.07-64.09H2409.8c72.4,0,102.07,64.09,102.07,64.09l117.49,227.87c35.7,46.92,76,60.09,110.36,59.85,45.87-.12,81.28-24.25,81.28-24.25,39-23.73,60.56-66.46,72.42-106.6A308.88,308.88,0,0,0,2906,1450.71Z";
const PRO_CROSS: &str = "M1740.6,967v58.28a7.64,7.64,0,0,1-7.64,7.65h-58.29a16.88,16.88,0,0,0-16.88,16.88v53.54a16.88,16.88,0,0,0,16.88,16.88H1733a7.64,7.64,0,0,1,7.64,7.65v58.28a16.88,16.88,0,0,0,16.88,16.88H1811a16.88,16.88,0,0,0,16.88-16.88v-58.28a7.65,7.65,0,0,1,7.65-7.65h58.29a16.87,16.87,0,0,0,16.87-16.88v-53.54a16.87,16.87,0,0,0-16.87-16.88h-58.29a7.65,7.65,0,0,1-7.65-7.65V967A16.88,16.88,0,0,0,1811,950.1h-53.54A16.88,16.88,0,0,0,1740.6,967Z";
const PRO_SEAM_L: &str = "M1287.24,862.64S1507.74,1212,1633,1332";
const PRO_SEAM_R: &str = "M2810.12,862.64S2589.62,1212,2464.33,1332";
const PRO_ARROWS: &str = "<path d='M1784.9,1174.15l10.48-17.11a1.28,1.28,0,0,0-1.09-2h-21a1.28,1.28,0,0,0-1.1,2l10.49,17.11A1.29,1.29,0,0,0,1784.9,1174.15Z'/><path d='M1782.71,979l-10.49,17.11a1.28,1.28,0,0,0,1.1,1.95h21a1.28,1.28,0,0,0,1.09-1.95L1784.9,979A1.29,1.29,0,0,0,1782.71,979Z'/><path d='M1882.89,1075.47l-17.1-10.49a1.28,1.28,0,0,0-2,1.09v21a1.28,1.28,0,0,0,2,1.09l17.1-10.48A1.28,1.28,0,0,0,1882.89,1075.47Z'/><path d='M1685.61,1077.66l17.1,10.48a1.28,1.28,0,0,0,2-1.09v-21a1.28,1.28,0,0,0-2-1.09l-17.1,10.49A1.28,1.28,0,0,0,1685.61,1077.66Z'/>";
const PRO_CAPTURE: &str = "<rect x='1896.21' y='811.37' width='63.56' height='63.56' rx='7.91'/>";
const PRO_MINUS_MARKS: &str = "<line x1='1819.2' y1='711.42' x2='1856.12' y2='711.42'/>";
const PRO_PLUS_MARKS: &str =
    "<line x1='2242.5' y1='711.42' x2='2279.43' y2='711.42'/><line x1='2260.96' y1='729.88' x2='2260.96' y2='692.96'/>";
const PRO_HOME_MARKS: &str = "<polygon points='2171.53 819.41 2153.45 835.29 2156.12 835.29 2156.12 857.16 2171.53 857.16 2186.94 857.16 2186.94 835.29 2189.6 835.29 2171.53 819.41'/><rect x='2165.05' y='838.29' width='12.96' height='11.76'/>";
const PRO_CAPTURE_MARKS: &str = "<circle cx='1927.99' cy='842.8' r='20.7'/>";

#[cfg(test)]
mod tests {
    use super::{Model, Part, Role};

    /// Every drawing reads, with each part it lights.
    #[test]
    fn every_drawing_reads_with_its_parts() {
        for model in Model::ALL {
            let shapes = model.shapes();
            let has = |role: Role| shapes.iter().any(|s| s.role == role && !s.segs.is_empty());
            assert!(has(Role::Body), "{model:?}: no body");
            for part in [Part::LB, Part::RB, Part::LStick, Part::RStick, Part::View, Part::Menu, Part::Guide] {
                assert!(has(Role::Part(part)), "{model:?}: no {part:?}");
            }
            assert!(has(Role::Trigger(Part::LT)) && has(Role::Trigger(Part::RT)), "{model:?}: no triggers");
            assert!(has(Role::Cross) || (has(Role::Arm(true)) && has(Role::Arm(false))), "{model:?}: no d-pad");
            assert_eq!(shapes.iter().filter(|s| s.role == Role::Seat).count(), 4, "{model:?}: face buttons' seats");
        }
    }
}
