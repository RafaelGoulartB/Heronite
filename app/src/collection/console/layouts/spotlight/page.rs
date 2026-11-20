//! The focused game's page under Spotlight's row: what it has (`Shape`),
//! where each section sits (`Geo`, worked out from counts and fixed
//! heights, never measured, so nothing jumps), how far the screen rises for
//! each focus, and how the focus moves between and inside the sections.
//! Everything here is plain numbers, so it is tested without a window.

use gpui_kit::SharedString;

use super::parts::{BUTTON_H, CARD_H, CHIPS_H, FACTS_H, TITLE_H};

/// Where the page's focus is, top to bottom.
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub(crate) enum Zone {
    /// The row of icons.
    #[default]
    Row,
    /// Play, Options, ♥.
    Actions,
    /// The store's screenshots (else the user's).
    Media,
    /// About this game | Achievements and Details with its links.
    Mid,
    /// Playtime, How Long To Beat, journal.
    Cards,
    /// The user's screenshots (a card under the strip).
    Shots,
    /// More like this.
    Related,
}

impl Zone {
    /// For `HERONITE_DEBUG_SPOTLIGHT`.
    pub(super) fn parse(s: &str) -> Option<(Zone, Option<Mid>)> {
        Some(match s {
            "row" => (Zone::Row, None),
            "actions" => (Zone::Actions, None),
            "media" | "viewer" => (Zone::Media, None),
            "about" | "sheet" => (Zone::Mid, Some(Mid::About)),
            "achievements" | "mid" => (Zone::Mid, Some(Mid::Ach)),
            "links" | "details" => (Zone::Mid, Some(Mid::Link)),
            "hltb" => (Zone::Mid, Some(Mid::Hltb)),
            "cards" => (Zone::Cards, None),
            "shots" => (Zone::Shots, None),
            "related" => (Zone::Related, None),
            _ => return None,
        })
    }
}

/// The focus inside the middle section.
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub(crate) enum Mid {
    /// "Read more" under About this game.
    #[default]
    About,
    /// The Achievements card.
    Ach,
    /// A link button under Details (`Nav::link`).
    Link,
    /// The How Long To Beat card.
    Hltb,
}

/// What the How Long To Beat card under Details shows.
#[derive(Clone, Default, Debug, PartialEq)]
pub(super) enum HltbCard {
    /// Not asked yet (or the section is off): no card.
    #[default]
    None,
    /// Asked, and HowLongToBeat has no times for the game.
    Missing,
    /// The times; `note`: how it was matched shows under them; `link`: its page.
    Times { note: bool, link: Option<String> },
}

/// What a link button under Details does.
#[derive(Clone, PartialEq, Debug)]
pub(super) enum Link {
    Folder,
    Store,
}

/// The sizes the page is laid out with, design px.
pub(super) const SIDE: f32 = 46.;
/// The widest the page's column gets (a very wide window keeps the art on the right).
const MAX_W: f32 = 1640.;
pub(super) const TILE_W: f32 = 300.;
pub(super) const TILE_H: f32 = 169.;
pub(super) const STRIP_GAP: f32 = 16.;
pub(super) const SHOT_W: f32 = 240.;
/// The padding of the screenshots' card, design px.
pub(super) const SHOTS_PAD: f32 = 12.;
pub(super) const SHOT_H: f32 = 135.;
pub(super) const COVER_W: f32 = 128.;
pub(super) const COLUMN_W: f32 = 430.;
pub(super) const COLUMN_GAP: f32 = 64.;
pub(super) const ACH_H: f32 = 78.;
pub(super) const DETAIL_H: f32 = 29.;
pub(super) const LINK_H: f32 = 32.;
pub(super) const LINE_H: f32 = 21.;
pub(super) const ABOUT_SIZE: f32 = 13.5;
pub(super) const MORE_H: f32 = 30.;
/// The How Long To Beat card with its times (and the note under them), and without.
pub(super) const HLTB_H: f32 = 96.;
pub(super) const HLTB_NOTE_H: f32 = 20.;
pub(super) const HLTB_MISSING_H: f32 = 46.;
/// Between two blocks of the right column.
pub(super) const COLUMN_STEP: f32 = 26.;
const SECTION_GAP: f32 = 46.;
/// The logo's box by the theme's size.
pub(super) fn logo_h(size: heronite_data::console_theme::Size3) -> f32 {
    use heronite_data::console_theme::Size3;
    match size {
        Size3::Small => 96.,
        Size3::Medium => 120.,
        Size3::Large => 150.,
    }
}
/// Under the top bar (the window's top) for a focused section's title, and
/// above the bottom edge for the page's end, design px.
const TOP_LINE: f32 = 86.;
const BOTTOM_LINE: f32 = 70.;
/// Where the logo sits from the window's top once the page has risen.
const RISEN_TOP: f32 = 36.;

/// What the focused game's page has, as last drawn: what the presses and
/// the hints go by.
#[derive(Clone, Default, Debug)]
pub(super) struct Shape {
    pub key: SharedString,
    /// Tiles of the media strip, and whether they are the user's screenshots.
    pub media: usize,
    pub media_shots: bool,
    /// The store screenshots' full pictures (the viewer's).
    pub store: Vec<String>,
    /// About this game has text, and more than fits (Read more).
    pub about: bool,
    pub about_more: bool,
    /// About's lines shown.
    pub about_lines: usize,
    pub ach: bool,
    pub details: usize,
    pub links: Vec<Link>,
    pub hltb: HltbCard,
    pub cards: usize,
    /// The user's screenshots, and how many thumbnails the card has room
    /// for (the last of them says "+N" for the rest).
    pub shots: usize,
    pub shots_shown: usize,
    pub related: Vec<SharedString>,
    pub geo: Geo,
}

/// The tops of the page's parts, design px from the body's top with the
/// page at rest (the row at the top).
#[derive(Clone, Copy, Default, Debug, PartialEq)]
pub(super) struct Geo {
    pub logo: f32,
    pub logo_h: f32,
    pub buttons: f32,
    pub facts: f32,
    pub chips: f32,
    pub media: f32,
    pub mid: f32,
    pub mid_h: f32,
    pub cards: f32,
    pub shots: f32,
    pub related: f32,
    pub related_h: f32,
    pub end: f32,
    /// The page's width.
    pub width: f32,
}

/// The page's width in a view `view` design px wide.
pub(super) fn page_width(view: f32) -> f32 {
    (view - 2. * SIDE).clamp(600., MAX_W)
}

/// About's text width on a page `width` wide.
pub(super) fn about_width(width: f32) -> f32 {
    width - COLUMN_W - COLUMN_GAP
}

/// How many screenshots the card has room for on a page `width` wide.
pub(super) fn shots_room(width: f32) -> usize {
    (((width - 2. * SHOTS_PAD + STRIP_GAP) / (SHOT_W + STRIP_GAP)).floor() as usize).max(1)
}

/// The right column's height: Achievements, then Details with its links,
/// then How Long To Beat.
fn column_h(s: &Shape) -> f32 {
    let mut blocks = Vec::new();
    if s.ach {
        blocks.push(TITLE_H + ACH_H);
    }
    if s.details > 0 || !s.links.is_empty() {
        blocks.push(TITLE_H + s.details as f32 * DETAIL_H + if s.links.is_empty() { 0. } else { 14. + LINK_H });
    }
    match &s.hltb {
        HltbCard::None => {}
        HltbCard::Missing => blocks.push(TITLE_H + HLTB_MISSING_H),
        HltbCard::Times { note, .. } => blocks.push(TITLE_H + HLTB_H + if *note { HLTB_NOTE_H } else { 0. }),
    }
    let gaps = blocks.len().saturating_sub(1) as f32 * COLUMN_STEP;
    blocks.into_iter().sum::<f32>() + gaps
}

impl Shape {
    /// The lines of About and whether it needs Read more, for a text of
    /// `chars` characters on a page `width` wide: at least as tall as the
    /// column beside it.
    pub fn fit_about(&mut self, chars: usize, width: f32) {
        let column = column_h(self);
        // Characters a line holds: about this many on average, and at least
        // this many (a line of wide letters).
        let average = (about_width(width) / (ABOUT_SIZE * 0.6)).max(20.);
        let least = (about_width(width) / (ABOUT_SIZE * 0.72)).max(16.);
        let room = (column - TITLE_H - MORE_H - 12.).max(0.);
        let lines = ((room / LINE_H).floor() as usize).clamp(5, 12);
        self.about_more = chars as f32 > lines as f32 * average;
        self.about_lines = if self.about_more { lines } else { ((chars as f32 / least).ceil() as usize).clamp(1, lines) };
    }

    /// Lays the page out: `row` the row's height, `logo_h` the logo's box.
    pub fn layout(&mut self, row: f32, logo_h: f32, width: f32, cover_h: f32) {
        let mut g = Geo { logo: row + 2., logo_h, width, ..Geo::default() };
        g.buttons = g.logo + logo_h + 16.;
        g.facts = g.buttons + BUTTON_H + 18.;
        g.chips = g.facts + FACTS_H + 10.;
        g.media = g.chips + CHIPS_H + 24.;
        let mut y = if self.media > 0 { g.media + TILE_H } else { g.chips + CHIPS_H };
        self.shots_shown = self.shots.min(shots_room(width));
        if self.shots > 0 {
            y += SECTION_GAP;
            g.shots = y;
            y += TITLE_H + SHOT_H + 2. * SHOTS_PAD;
        }
        let about_h =
            if self.about { TITLE_H + self.about_lines as f32 * LINE_H + if self.about_more { 12. + MORE_H } else { 0. } } else { 0. };
        g.mid_h = about_h.max(column_h(self));
        if g.mid_h > 0. {
            y += SECTION_GAP;
            g.mid = y;
            y += g.mid_h;
        }
        if self.cards > 0 {
            y += SECTION_GAP;
            g.cards = y;
            y += CARD_H;
        }
        if !self.related.is_empty() {
            y += SECTION_GAP;
            g.related = y;
            g.related_h = TITLE_H + cover_h;
            y += g.related_h;
        }
        g.end = y + 30.;
        self.geo = g;
    }

    /// Whether the middle section has `m` to focus.
    pub fn mid_has(&self, m: Mid) -> bool {
        match m {
            Mid::About => self.about && self.about_more,
            Mid::Ach => self.ach,
            Mid::Link => !self.links.is_empty(),
            Mid::Hltb => matches!(self.hltb, HltbCard::Times { link: Some(_), .. }),
        }
    }

    /// The zones the focus can be in, top to bottom.
    pub fn zones(&self) -> Vec<Zone> {
        let mut z = vec![Zone::Row, Zone::Actions];
        if self.media > 0 {
            z.push(Zone::Media);
        }
        if self.shots > 0 {
            z.push(Zone::Shots);
        }
        if [Mid::About, Mid::Ach, Mid::Link, Mid::Hltb].into_iter().any(|m| self.mid_has(m)) {
            z.push(Zone::Mid);
        }
        if self.cards > 0 {
            z.push(Zone::Cards);
        }
        if !self.related.is_empty() {
            z.push(Zone::Related);
        }
        z
    }

    /// How many the zone has along it (← →).
    pub fn count(&self, zone: Zone) -> usize {
        match zone {
            Zone::Media => self.media,
            Zone::Cards => self.cards,
            Zone::Shots => self.shots_shown,
            Zone::Related => self.related.len(),
            _ => 1,
        }
    }

    /// The top of the zone's section, design px from the body's top at rest.
    fn top(&self, zone: Zone) -> f32 {
        let g = &self.geo;
        match zone {
            Zone::Row => 0.,
            Zone::Actions => g.logo,
            Zone::Media => g.media,
            Zone::Mid => g.mid,
            Zone::Cards => g.cards,
            Zone::Shots => g.shots,
            Zone::Related => g.related,
        }
    }

    /// How far the screen has risen for `zone`, design px, the window being
    /// `height` design px tall with a top bar `bar` tall: nothing on the
    /// row; off it the logo goes to the top; a section's title comes up to
    /// near the top, until the page's end sits above the bottom.
    pub fn rise(&self, zone: Zone, height: f32, bar: f32) -> f32 {
        if zone == Zone::Row {
            return 0.;
        }
        let g = &self.geo;
        let first = g.logo + bar - RISEN_TOP;
        let last = (g.end + bar - (height - BOTTOM_LINE)).max(first);
        match zone {
            Zone::Row => 0.,
            Zone::Actions => first,
            // The strip rises only when it would be under the bottom edge.
            Zone::Media => first.max(g.media + TILE_H + bar - (height - BOTTOM_LINE)).min(last),
            z => (self.top(z) + bar - TOP_LINE).clamp(first, last),
        }
    }
}

/// A move of the focus inside the middle section.
pub(super) enum MidMove {
    /// It moved there (`link` for a link).
    To(Mid, usize),
    /// It leaves the section up (-1) or down (1).
    Out(isize),
    Stay,
}

impl Shape {
    /// Where the focus enters the middle section from above (`down`) or below.
    pub fn mid_entry(&self, down: bool) -> Mid {
        let order = if down { [Mid::Ach, Mid::About, Mid::Link, Mid::Hltb] } else { [Mid::Hltb, Mid::Link, Mid::Ach, Mid::About] };
        order.into_iter().find(|m| self.mid_has(*m)).unwrap_or_default()
    }

    /// The middle section's focus `at` (`link`) moved by `pad`: About on the
    /// left; on the right, top to bottom, Achievements, the links under
    /// Details and How Long To Beat.
    pub fn mid_move(&self, at: Mid, link: usize, pad: crate::console::Pad) -> MidMove {
        use crate::console::Pad;
        let n = self.links.len();
        // The right column, top to bottom.
        let column: Vec<Mid> = [Mid::Ach, Mid::Link, Mid::Hltb].into_iter().filter(|m| self.mid_has(*m)).collect();
        let row = column.iter().position(|m| *m == at);
        let to = |m: Mid| MidMove::To(m, if m == Mid::Link { link.min(n.saturating_sub(1)) } else { 0 });
        match (at, pad) {
            (Mid::About, Pad::Right) => column.first().map_or(MidMove::Stay, |m| MidMove::To(*m, 0)),
            (Mid::Link, Pad::Left) if link > 0 => MidMove::To(Mid::Link, link - 1),
            (Mid::Link, Pad::Right) if link + 1 < n => MidMove::To(Mid::Link, link + 1),
            (_, Pad::Left) if at != Mid::About && self.mid_has(Mid::About) => MidMove::To(Mid::About, 0),
            (_, Pad::Up) => match row.filter(|r| *r > 0) {
                Some(r) => to(column[r - 1]),
                None => MidMove::Out(-1),
            },
            (_, Pad::Down) => match row.map(|r| r + 1).filter(|r| *r < column.len()) {
                Some(r) => to(column[r]),
                None => MidMove::Out(1),
            },
            _ => MidMove::Stay,
        }
    }
}

/// How far a strip of `n` tiles `step` design px apart (each `w` wide) has
/// slid for its tile `focus` to show in a view `view` wide, with a little
/// of the next one in sight.
pub(super) fn strip_offset(focus: usize, n: usize, w: f32, step: f32, view: f32) -> f32 {
    if n == 0 {
        return 0.;
    }
    let end = (n - 1) as f32 * step + w;
    let right = focus as f32 * step + w + if focus + 1 < n { step * 0.35 } else { 0. };
    (right - view).clamp(0., (end - view).max(0.))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::console::Pad;

    fn shape() -> Shape {
        let mut s = Shape {
            media: 6,
            about: true,
            ach: true,
            details: 5,
            links: vec![Link::Folder, Link::Store],
            related: vec!["a".into(), "b".into()],
            ..Shape::default()
        };
        s.fit_about(2000, 1500.);
        s.layout(186., 150., 1500., 192.);
        s
    }

    #[test]
    fn the_sections_follow_one_another() {
        let s = shape();
        let g = s.geo;
        assert!(g.logo < g.buttons && g.buttons < g.facts && g.facts < g.chips && g.chips < g.media);
        assert!(g.media + TILE_H < g.mid && g.mid + g.mid_h < g.related && g.related < g.end);
        assert_eq!(s.zones(), [Zone::Row, Zone::Actions, Zone::Media, Zone::Mid, Zone::Related]);
        // The user's screenshots come right under the strip, as many as fit.
        let mut t = Shape { shots: 20, ..s.clone() };
        t.layout(186., 150., 1500., 192.);
        assert_eq!(t.zones()[3], Zone::Shots);
        assert!(t.geo.media + TILE_H < t.geo.shots && t.geo.shots < t.geo.mid);
        assert_eq!(t.shots_shown, shots_room(1500.));
        assert_eq!(t.count(Zone::Shots), t.shots_shown);
        assert!(s.about_more);
    }

    #[test]
    fn the_head_always_sits_under_the_row() {
        use heronite_data::console_theme::Size3;
        for row in [150., 186., 240.] {
            for size in [Size3::Small, Size3::Medium, Size3::Large] {
                for media in [0, 6] {
                    let mut s = Shape { media, ..Shape::default() };
                    s.layout(row, logo_h(size), 1200., 192.);
                    let g = s.geo;
                    assert!(g.logo >= row, "{row} {size:?}");
                    assert!(g.buttons >= g.logo + g.logo_h && g.facts >= g.buttons + BUTTON_H, "{row} {size:?}");
                    assert!(g.chips >= g.facts + FACTS_H && g.media >= g.chips + CHIPS_H, "{row} {size:?}");
                }
            }
        }
    }

    #[test]
    fn the_screen_rises_with_the_focus_and_stops_at_the_end() {
        let s = shape();
        let (h, bar) = (800., 76.);
        assert_eq!(s.rise(Zone::Row, h, bar), 0.);
        let first = s.rise(Zone::Actions, h, bar);
        assert!(first > 0.);
        assert!(s.rise(Zone::Media, h, bar) >= first);
        let mid = s.rise(Zone::Mid, h, bar);
        let related = s.rise(Zone::Related, h, bar);
        assert!(mid > first && related >= mid);
        // The page's end never rises above the bottom line.
        assert!(s.geo.end + bar - related >= h - BOTTOM_LINE - 0.5);
    }

    #[test]
    fn the_middle_moves_between_about_achievements_and_links() {
        let s = shape();
        assert_eq!(s.mid_entry(true), Mid::Ach);
        assert_eq!(s.mid_entry(false), Mid::Link);
        assert!(matches!(s.mid_move(Mid::Ach, 0, Pad::Left), MidMove::To(Mid::About, _)));
        assert!(matches!(s.mid_move(Mid::Ach, 0, Pad::Down), MidMove::To(Mid::Link, 0)));
        assert!(matches!(s.mid_move(Mid::Link, 0, Pad::Right), MidMove::To(Mid::Link, 1)));
        assert!(matches!(s.mid_move(Mid::Link, 1, Pad::Right), MidMove::Stay));
        assert!(matches!(s.mid_move(Mid::Link, 1, Pad::Up), MidMove::To(Mid::Ach, _)));
        assert!(matches!(s.mid_move(Mid::Link, 1, Pad::Down), MidMove::Out(1)));
        assert!(matches!(s.mid_move(Mid::About, 0, Pad::Up), MidMove::Out(-1)));
        // How Long To Beat under the links, when it has a page to open.
        let mut h = s.clone();
        h.hltb = HltbCard::Times { note: false, link: Some("https://howlongtobeat.com/game/1".into()) };
        assert!(matches!(h.mid_move(Mid::Link, 1, Pad::Down), MidMove::To(Mid::Hltb, _)));
        assert!(matches!(h.mid_move(Mid::Hltb, 1, Pad::Up), MidMove::To(Mid::Link, 1)));
        assert!(matches!(h.mid_move(Mid::Hltb, 0, Pad::Down), MidMove::Out(1)));
        assert!(matches!(h.mid_move(Mid::Hltb, 0, Pad::Left), MidMove::To(Mid::About, _)));
        assert_eq!(h.mid_entry(false), Mid::Hltb);
        // Without times it shows but takes no focus.
        h.hltb = HltbCard::Missing;
        assert!(matches!(h.mid_move(Mid::Link, 1, Pad::Down), MidMove::Out(1)));
    }

    #[test]
    fn a_strip_slides_only_to_keep_the_focus_in_sight() {
        assert_eq!(strip_offset(0, 6, 100., 110., 400.), 0.);
        assert!(strip_offset(5, 6, 100., 110., 400.) > 0.);
        // Never past its end.
        assert_eq!(strip_offset(5, 6, 100., 110., 400.), 5. * 110. + 100. - 400.);
        assert_eq!(strip_offset(3, 3, 100., 110., 400.), 0.);
    }
}
