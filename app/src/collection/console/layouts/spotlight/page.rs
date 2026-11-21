//! The focused game's page under Spotlight's row: what it has (`Shape`),
//! where each section sits (`Geo`, worked out from counts and fixed
//! heights, never measured, so nothing jumps), how far the screen rises for
//! each focus, and how the focus moves between and inside the sections.
//! Everything here is plain numbers, so it is tested without a window.
//!
//! Under the head the page is two columns: on the left the store's
//! screenshots, the player's own, About this game, the cards and More like
//! this, one under the other; on the right a sidebar with Achievements,
//! Details (and its links) and How Long To Beat, from the top of the first
//! left section down. Nothing spans both, so the sidebar is never pushed down.
//! On a page narrower than [`STACK_BELOW`] (a 4:3 or small window) the
//! sidebar goes under the left column instead, and ↓ / ↑ cross between them.

use gpui_kit::SharedString;

use super::parts::{BUTTON_H, CARD_H, CHIPS_H, FACTS_H, TITLE_H};
use crate::console::Pad;

/// Where the page's focus is: the row, the buttons, a section of the left
/// column (top to bottom) or the sidebar.
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub(crate) enum Zone {
    /// The row of icons.
    #[default]
    Row,
    /// Play, Options, ♥.
    Actions,
    /// The store's screenshots (else the user's).
    Media,
    /// The user's screenshots, in a card.
    Shots,
    /// "Read more" under About this game.
    About,
    /// Playtime, How Long To Beat, journal.
    Cards,
    /// More like this.
    Related,
    /// The sidebar (`Nav::mid`).
    Side,
}

impl Zone {
    /// For `HERONITE_DEBUG_SPOTLIGHT`.
    pub(super) fn parse(s: &str) -> Option<(Zone, Option<Mid>)> {
        Some(match s {
            "row" => (Zone::Row, None),
            "actions" => (Zone::Actions, None),
            "media" | "viewer" => (Zone::Media, None),
            "about" | "sheet" => (Zone::About, None),
            "achievements" | "mid" => (Zone::Side, Some(Mid::Ach)),
            "links" | "details" => (Zone::Side, Some(Mid::Link)),
            "hltb" => (Zone::Side, Some(Mid::Hltb)),
            "cards" => (Zone::Cards, None),
            "shots" => (Zone::Shots, None),
            "related" => (Zone::Related, None),
            _ => return None,
        })
    }

    /// A section of the left column.
    fn is_left(self) -> bool {
        matches!(self, Zone::Media | Zone::Shots | Zone::About | Zone::Cards | Zone::Related)
    }
}

/// The focus inside the sidebar, top to bottom.
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub(crate) enum Mid {
    /// The Achievements card.
    #[default]
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

/// On the home, how much of the page's first sections shows above the
/// body's bottom, and the room between the row and the buttons of a game
/// without a logo, design px.
/// Between the head and the first section, design px.
pub(super) const HEAD_GAP: f32 = 24.;
pub(super) const PEEK_SHOWS: f32 = 150.;
const NO_LOGO_GAP: f32 = 48.;

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
/// A page narrower than this (design px) puts the sidebar under the left column.
pub(super) const STACK_BELOW: f32 = 1000.;
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
/// Between two blocks of the sidebar.
pub(super) const COLUMN_STEP: f32 = 26.;
/// The most lines About this game shows (Read more opens the rest).
const ABOUT_LINES: usize = 9;
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
    /// The achievements are still being read: the card holds their place
    /// (as the last game had it), empty.
    pub ach_wait: bool,
    pub details: usize,
    pub links: Vec<Link>,
    pub hltb: HltbCard,
    pub cards: usize,
    /// The user's screenshots, and how many thumbnails the card has room
    /// for (the last of them says "+N" for the rest).
    pub shots: usize,
    pub shots_shown: usize,
    pub related: Vec<SharedString>,
    /// On the console home: the body's height, design px. The page's first
    /// sections then start [`PEEK_SHOWS`] above its bottom, however tall the
    /// head is (never over it). 0: right under the head.
    pub pin: f32,
    /// The head has a line under its facts: Showcase's genre chips, Cinema's
    /// genres (the console home has none).
    pub chips: bool,
    /// A head laid out in one column of its own (Cinema's page: the logo or
    /// a name line, the facts, the genres, the buttons): its whole height,
    /// design px; the sections start under it. 0: the parts above.
    pub head: f32,
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
    /// The left column's sections.
    pub media: f32,
    pub shots: f32,
    pub about: f32,
    pub about_h: f32,
    pub cards: f32,
    pub related: f32,
    /// The sidebar's top, and its blocks' (each from its title): the
    /// Achievements card, Details (its links' row at `links`) and How Long To Beat.
    pub side: f32,
    pub ach: f32,
    pub details: f32,
    pub links: f32,
    pub hltb: f32,
    pub end: f32,
    /// The page's width, and its left column's.
    pub width: f32,
    pub left: f32,
    /// The sidebar is under the left column (a narrow page).
    pub stacked: bool,
}

/// The page's width in a view `view` design px wide.
pub(super) fn page_width(view: f32) -> f32 {
    (view - 2. * SIDE).clamp(600., MAX_W)
}

/// Whether a page `width` wide puts the sidebar under the left column.
pub(super) fn stacks(width: f32) -> bool {
    width < STACK_BELOW
}

/// The left column's width on a page `width` wide (the whole page when the
/// sidebar goes under it).
pub(super) fn left_width(width: f32) -> f32 {
    if stacks(width) {
        width
    } else {
        width - COLUMN_W - COLUMN_GAP
    }
}

/// How many screenshots the card has room for in a column `width` wide.
pub(super) fn shots_room(width: f32) -> usize {
    (((width - 2. * SHOTS_PAD + STRIP_GAP) / (SHOT_W + STRIP_GAP)).floor() as usize).max(1)
}

impl Shape {
    /// The lines of About and whether it needs Read more, for a text of
    /// `chars` characters in a column `width` wide.
    pub fn fit_about(&mut self, chars: usize, width: f32) {
        // Characters a line holds: about this many on average, and at least
        // this many (a line of wide letters).
        let average = (width / (ABOUT_SIZE * 0.6)).max(20.);
        let least = (width / (ABOUT_SIZE * 0.72)).max(16.);
        self.about_more = chars as f32 > ABOUT_LINES as f32 * average;
        self.about_lines = if self.about_more { ABOUT_LINES } else { ((chars as f32 / least).ceil() as usize).clamp(1, ABOUT_LINES) };
    }

    /// Lays the page out: `row` the row's height, `logo_h` the logo's box (0:
    /// no logo, the buttons start right under the row), `width` the page's,
    /// `cover_h` a cover's of More like this.
    pub fn layout(&mut self, row: f32, logo_h: f32, width: f32, cover_h: f32) {
        // Without a sidebar the left column takes the whole page (and the
        // sidebar has the left's place when that has nothing).
        let stacked = stacks(width) && self.has_side() && self.first_left().is_some();
        let left = if self.has_side() && self.first_left().is_some() { left_width(width) } else { width };
        // Without a logo the buttons keep a clear gap under the row (and the
        // name beside its focused icon).
        let logo = row + if logo_h > 0. { 2. } else { NO_LOGO_GAP };
        let mut g = Geo { logo, logo_h, width, left, stacked, ..Geo::default() };
        g.buttons = g.logo + if logo_h > 0. { logo_h + 16. } else { 0. };
        g.facts = g.buttons + BUTTON_H + 18.;
        g.chips = g.facts + FACTS_H + 10.;
        let under_head = if self.head > 0. {
            // Its own column, buttons last: the sections start under it.
            g.buttons = g.logo + self.head - BUTTON_H;
            g.logo + self.head + HEAD_GAP
        } else if self.chips {
            g.chips + CHIPS_H + HEAD_GAP
        } else {
            g.facts + FACTS_H + HEAD_GAP
        };
        // On the home the peek of the page sits at the bottom, just above the
        // hints, whatever the head's height; a short window pushes it down
        // (out of sight) rather than over the buttons.
        let top = if self.pin > 0. { under_head.max(self.pin - PEEK_SHOWS) } else { under_head };
        g.media = top;
        g.side = top;
        self.shots_shown = self.shots.min(shots_room(left));
        // The left column, one section under the other.
        let mut y = top;
        let mut first = true;
        let mut place = |h: f32| {
            if !first {
                y += SECTION_GAP;
            }
            first = false;
            let at = y;
            y += h;
            at
        };
        if self.media > 0 {
            g.media = place(TILE_H);
        }
        if self.shots > 0 {
            g.shots = place(TITLE_H + SHOT_H + 2. * SHOTS_PAD);
        }
        if self.about {
            g.about_h = TITLE_H + self.about_lines as f32 * LINE_H + if self.about_more { 12. + MORE_H } else { 0. };
            g.about = place(g.about_h);
        }
        if self.cards > 0 {
            g.cards = place(CARD_H);
        }
        if !self.related.is_empty() {
            g.related = place(TITLE_H + cover_h);
        }
        let left_end = if first { top } else { y };
        // The sidebar, from the same top (under the left column when stacked).
        let mut sy = if stacked { left_end + SECTION_GAP } else { top };
        if stacked {
            g.side = sy;
        }
        let mut blocks = 0;
        let mut block = |h: f32| {
            if blocks > 0 {
                sy += COLUMN_STEP;
            }
            blocks += 1;
            let at = sy;
            sy += h;
            at
        };
        if self.ach {
            g.ach = block(TITLE_H + ACH_H);
        }
        if self.details > 0 || !self.links.is_empty() {
            let rows = TITLE_H + self.details as f32 * DETAIL_H;
            g.details = block(rows + if self.links.is_empty() { 0. } else { 14. + LINK_H });
            g.links = g.details + rows + 14.;
        }
        match &self.hltb {
            HltbCard::None => {}
            HltbCard::Missing => g.hltb = block(TITLE_H + HLTB_MISSING_H),
            HltbCard::Times { note, .. } => g.hltb = block(TITLE_H + HLTB_H + if *note { HLTB_NOTE_H } else { 0. }),
        }
        g.end = left_end.max(sy) + 30.;
        self.geo = g;
    }

    /// The first section of the left column that shows anything, in page
    /// order (what peeks in at the bottom of the home).
    pub fn first_left(&self) -> Option<Zone> {
        [
            (Zone::Media, self.media > 0),
            (Zone::Shots, self.shots > 0),
            (Zone::About, self.about),
            (Zone::Cards, self.cards > 0),
            (Zone::Related, !self.related.is_empty()),
        ]
        .into_iter()
        .find_map(|(z, has)| has.then_some(z))
    }

    /// Whether the sidebar has anything to show (Achievements, Details, How Long To Beat).
    pub fn has_side(&self) -> bool {
        self.ach || self.details > 0 || !self.links.is_empty() || self.hltb != HltbCard::None
    }

    /// Whether the sidebar has `m` to focus.
    pub fn mid_has(&self, m: Mid) -> bool {
        match m {
            Mid::Ach => self.ach,
            Mid::Link => !self.links.is_empty(),
            Mid::Hltb => matches!(self.hltb, HltbCard::Times { link: Some(_), .. }),
        }
    }

    /// The sidebar's targets, top to bottom.
    fn side_items(&self) -> Vec<Mid> {
        [Mid::Ach, Mid::Link, Mid::Hltb].into_iter().filter(|m| self.mid_has(*m)).collect()
    }

    /// The left column's sections that take the focus, top to bottom.
    fn left_zones(&self) -> Vec<Zone> {
        [Zone::Media, Zone::Shots, Zone::About, Zone::Cards, Zone::Related].into_iter().filter(|z| self.has(*z)).collect()
    }

    fn has(&self, zone: Zone) -> bool {
        match zone {
            Zone::Row | Zone::Actions => true,
            Zone::Media => self.media > 0,
            Zone::Shots => self.shots > 0,
            Zone::About => self.about && self.about_more,
            Zone::Cards => self.cards > 0,
            Zone::Related => !self.related.is_empty(),
            Zone::Side => !self.side_items().is_empty(),
        }
    }

    /// The zones the focus can be in.
    pub fn zones(&self) -> Vec<Zone> {
        let mut z = vec![Zone::Row, Zone::Actions];
        z.extend(self.left_zones());
        if self.has(Zone::Side) {
            z.push(Zone::Side);
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

    /// The top of what has the focus (`side` in the sidebar), design px from
    /// the body's top at rest.
    pub fn top(&self, zone: Zone, side: Mid) -> f32 {
        let g = &self.geo;
        match zone {
            Zone::Row => 0.,
            Zone::Actions => g.logo,
            Zone::Media => g.media,
            Zone::Shots => g.shots,
            Zone::About => g.about,
            Zone::Cards => g.cards,
            Zone::Related => g.related,
            Zone::Side => match side {
                Mid::Ach => g.ach,
                Mid::Link => g.details,
                Mid::Hltb => g.hltb,
            },
        }
    }

    /// How far the screen has risen for the focus, design px, the window
    /// being `height` design px tall with a top bar `bar` tall: nothing on
    /// the row; off it the logo goes to the top; a section's title comes up
    /// to near the top, until the page's end sits above the bottom.
    pub fn rise(&self, zone: Zone, side: Mid, height: f32, bar: f32) -> f32 {
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
            z => (self.top(z, side) + bar - TOP_LINE).clamp(first, last),
        }
    }

    /// The left section whose top is nearest to `y`, design px.
    fn left_near(&self, y: f32) -> Option<Zone> {
        let top = |z: Zone| self.top(z, Mid::Ach);
        self.left_zones().into_iter().min_by(|a, b| (top(*a) - y).abs().total_cmp(&(top(*b) - y).abs()))
    }

    /// The sidebar's target nearest to `y`.
    fn side_near(&self, y: f32) -> Option<Mid> {
        self.side_items().into_iter().min_by(|a, b| (self.top(Zone::Side, *a) - y).abs().total_cmp(&(self.top(Zone::Side, *b) - y).abs()))
    }

    /// Where `pad` takes the focus on the page (`at` in `zone`; `side` and
    /// `link` the sidebar's; `along`: the focus along the zone, `n` of them).
    /// ↑ ↓ go along a column (above the first section: the buttons); ← → move
    /// along a strip or a row of links and cross between the columns, to the
    /// part nearest in height, at their ends.
    pub fn step(&self, zone: Zone, side: Mid, link: usize, along: usize, pad: Pad) -> Move {
        let n = self.count(zone);
        let links = self.links.len();
        let left = self.left_zones();
        let items = self.side_items();
        if zone.is_left() {
            let at = left.iter().position(|z| *z == zone).unwrap_or(0);
            return match pad {
                Pad::Up => Move::To(left.get(at.wrapping_sub(1)).copied().unwrap_or(Zone::Actions), side, link),
                Pad::Down => match left.get(at + 1) {
                    Some(z) => Move::To(*z, side, link),
                    // Stacked: on down into the sidebar.
                    None if self.geo.stacked => items.first().map_or(Move::Stay, |m| Move::To(Zone::Side, *m, 0)),
                    None => Move::Stay,
                },
                Pad::Left if along > 0 => Move::Along(along - 1),
                Pad::Right if along + 1 < n => Move::Along(along + 1),
                Pad::Right if self.geo.stacked => Move::Stay,
                Pad::Right => self
                    .side_near(self.top(zone, side))
                    .map_or(Move::Stay, |m| Move::To(Zone::Side, m, if m == Mid::Link { 0 } else { link })),
                _ => Move::Stay,
            };
        }
        if zone == Zone::Side {
            let at = items.iter().position(|m| *m == side).unwrap_or(0);
            let to = |m: Mid| Move::To(Zone::Side, m, link.min(links.saturating_sub(1)));
            return match pad {
                Pad::Up => match items.get(at.wrapping_sub(1)) {
                    Some(m) => to(*m),
                    // Stacked: back up to the left column's last section.
                    None if self.geo.stacked => left.last().map_or(Move::To(Zone::Actions, side, link), |z| Move::To(*z, side, link)),
                    None => Move::To(Zone::Actions, side, link),
                },
                Pad::Down => items.get(at + 1).map_or(Move::Stay, |m| to(*m)),
                Pad::Right if side == Mid::Link && link + 1 < links => Move::To(Zone::Side, side, link + 1),
                Pad::Left if side == Mid::Link && link > 0 => Move::To(Zone::Side, side, link - 1),
                Pad::Left if self.geo.stacked => Move::Stay,
                Pad::Left => self.left_near(self.top(Zone::Side, side)).map_or(Move::Stay, |z| Move::To(z, side, link)),
                _ => Move::Stay,
            };
        }
        Move::Stay
    }
}

/// Where a press takes the page's focus.
#[derive(Clone, Copy, PartialEq, Debug)]
pub(super) enum Move {
    /// To a zone (the sidebar's target and link with it).
    To(Zone, Mid, usize),
    /// Along the zone, to this one.
    Along(usize),
    Stay,
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
        s.fit_about(4000, left_width(1500.));
        s.layout(172., 150., 1500., 192.);
        s
    }

    #[test]
    fn the_sections_follow_one_another() {
        let s = shape();
        let g = s.geo;
        assert!(g.logo < g.buttons && g.buttons < g.facts && g.facts < g.chips && g.chips < g.media);
        assert!(g.media + TILE_H < g.about && g.about + g.about_h < g.related && g.related < g.end);
        assert_eq!(s.zones(), [Zone::Row, Zone::Actions, Zone::Media, Zone::About, Zone::Related, Zone::Side]);
        assert!(s.about_more);
        // The user's screenshots come right under the strip, as many as fit the left column.
        let mut t = Shape { shots: 20, ..s.clone() };
        t.layout(172., 150., 1500., 192.);
        assert_eq!(t.zones()[3], Zone::Shots);
        assert!(t.geo.media + TILE_H < t.geo.shots && t.geo.shots < t.geo.about);
        assert_eq!(t.shots_shown, shots_room(left_width(1500.)));
        assert_eq!(t.count(Zone::Shots), t.shots_shown);
    }

    #[test]
    fn the_sidebar_starts_beside_the_first_section_and_is_never_pushed_down() {
        for (media, shots) in [(6, 0), (0, 12), (6, 12), (0, 0)] {
            let mut s = Shape { media, shots, ..shape() };
            s.layout(172., 150., 1500., 192.);
            let g = s.geo;
            let first = [(media > 0).then_some(g.media), (shots > 0).then_some(g.shots), Some(g.about)].into_iter().flatten().next();
            assert_eq!(Some(g.side), first, "{media} {shots}");
            assert_eq!(g.ach, g.side);
            assert!(g.details > g.ach && g.links > g.details);
            // Nothing on the left reaches into the sidebar.
            assert!(g.left + COLUMN_GAP + COLUMN_W <= g.width + 0.5);
        }
    }

    #[test]
    fn without_a_sidebar_the_left_column_takes_the_page() {
        // Achievements, else Details, else How Long To Beat start the sidebar beside the strip.
        let s = shape();
        assert!(s.has_side() && s.geo.left < s.geo.width && s.geo.ach == s.geo.media);
        let mut d = Shape { ach: false, ..shape() };
        d.layout(172., 150., 1500., 192.);
        assert!(d.has_side() && d.geo.details == d.geo.media);
        let mut h = Shape { ach: false, details: 0, links: Vec::new(), hltb: HltbCard::Missing, ..shape() };
        h.layout(172., 150., 1500., 192.);
        assert!(h.has_side() && h.geo.hltb == h.geo.media);
        // Nothing for it: the strip and the rest span the whole page.
        let mut n = Shape { ach: false, details: 0, links: Vec::new(), ..shape() };
        n.layout(172., 150., 1500., 192.);
        assert!(!n.has_side() && n.geo.left == n.geo.width);
        assert!(!n.zones().contains(&Zone::Side));
        assert_eq!(n.shots_shown, 0);
    }

    #[test]
    fn whatever_comes_first_on_the_left_starts_beside_the_sidebar() {
        // Media off: the screenshots card comes first, level with the sidebar.
        let mut s = Shape { media: 0, shots: 12, ..shape() };
        s.layout(172., 150., 1500., 192.);
        assert_eq!(s.first_left(), Some(Zone::Shots));
        assert_eq!(s.geo.shots, s.geo.side);
        assert_eq!(s.zones()[2], Zone::Shots);
        // Media and screenshots off: About this game.
        let mut a = Shape { media: 0, shots: 0, ..shape() };
        a.layout(172., 150., 1500., 192.);
        assert_eq!(a.first_left(), Some(Zone::About));
        assert_eq!(a.geo.about, a.geo.side);
        // Nothing on the left: the sidebar takes the whole width.
        let mut n = Shape { media: 0, shots: 0, about: false, related: Vec::new(), ..shape() };
        n.layout(172., 150., 1500., 192.);
        assert_eq!(n.first_left(), None);
        assert!(n.has_side() && n.geo.left == n.geo.width);
    }

    #[test]
    fn the_home_pins_the_peek_to_the_bottom_and_keeps_the_head_clear_of_the_row() {
        // Whatever the logo, the peek starts the same distance above the bottom.
        for logo in [0., 96., 150.] {
            let mut s = Shape { media: 6, pin: 1300., ..Shape::default() };
            s.layout(172., logo, 1500., 192.);
            assert_eq!(s.geo.media, 1300. - PEEK_SHOWS, "{logo}");
            assert_eq!(s.geo.side, s.geo.media);
        }
        // A short body: the peek goes below the head, never over it.
        let mut short = Shape { media: 6, pin: 500., ..Shape::default() };
        short.layout(172., 150., 1500., 192.);
        assert!(short.geo.media >= short.geo.facts + FACTS_H + 24.);
        // No logo: a clear gap between the row and the buttons.
        let mut bare = Shape { media: 6, ..Shape::default() };
        bare.layout(172., 0., 1500., 192.);
        assert!(bare.geo.buttons - 172. >= 40.);
    }

    #[test]
    fn a_head_of_its_own_pushes_the_sections_below_its_whole_height() {
        // Cinema's page: a short head (a name line) and a tall one (a logo).
        for head in [190., 400.] {
            let mut s = Shape { media: 6, chips: true, head, ..shape() };
            s.layout(40., 0., 1500., 192.);
            let g = s.geo;
            assert_eq!(g.media, g.logo + head + HEAD_GAP, "{head}");
            assert_eq!(g.side, g.media);
            // The buttons are the head's last line, above the strip.
            assert!(g.buttons + BUTTON_H <= g.media - HEAD_GAP + 0.01, "{head}");
        }
    }

    #[test]
    fn the_head_always_sits_under_the_row() {
        use heronite_data::console_theme::Size3;
        for row in [150., 172., 240.] {
            for size in [Size3::Small, Size3::Medium, Size3::Large] {
                for media in [0, 6] {
                    // With the logo, and without one (nothing in its place);
                    // with the line of genres (the game page) and without.
                    for (logo, chips) in [(logo_h(size), false), (0., false), (logo_h(size), true), (0., true)] {
                        let mut s = Shape { media, chips, ..Shape::default() };
                        s.layout(row, logo, 1200., 192.);
                        let g = s.geo;
                        assert!(g.logo >= row, "{row} {size:?}");
                        assert!(g.buttons >= g.logo + g.logo_h && g.facts >= g.buttons + BUTTON_H, "{row} {size:?}");
                        let under_facts = if chips { g.chips + CHIPS_H } else { g.facts + FACTS_H };
                        assert!(g.chips >= g.facts + FACTS_H && g.media >= under_facts, "{row} {size:?}");
                        if !chips {
                            // Nothing between the facts and the strip but the gap.
                            assert_eq!(g.media, g.facts + FACTS_H + 24., "{row} {size:?}");
                        }
                        if logo == 0. {
                            // No gap where the logo would be.
                            assert_eq!(g.buttons, g.logo, "{row} {size:?}");
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn the_screen_rises_with_the_focus_and_stops_at_the_end() {
        let s = shape();
        let (h, bar) = (800., 76.);
        assert_eq!(s.rise(Zone::Row, Mid::Ach, h, bar), 0.);
        let first = s.rise(Zone::Actions, Mid::Ach, h, bar);
        assert!(first > 0.);
        assert!(s.rise(Zone::Media, Mid::Ach, h, bar) >= first);
        let about = s.rise(Zone::About, Mid::Ach, h, bar);
        let related = s.rise(Zone::Related, Mid::Ach, h, bar);
        assert!(about > first && related >= about);
        // The page's end never rises above the bottom line.
        assert!(s.geo.end + bar - related >= h - BOTTOM_LINE - 0.5);
    }

    #[test]
    fn the_focus_moves_along_a_column_and_across_to_the_nearest_part() {
        let s = shape();
        // ↓ goes on down the left column; ↑ from its first section, to the buttons.
        assert_eq!(s.step(Zone::Media, Mid::Ach, 0, 0, Pad::Down), Move::To(Zone::About, Mid::Ach, 0));
        assert_eq!(s.step(Zone::Media, Mid::Ach, 0, 0, Pad::Up), Move::To(Zone::Actions, Mid::Ach, 0));
        // Along the strip, then across to the sidebar at its end.
        assert_eq!(s.step(Zone::Media, Mid::Ach, 0, 2, Pad::Right), Move::Along(3));
        assert_eq!(s.step(Zone::Media, Mid::Ach, 0, 5, Pad::Right), Move::To(Zone::Side, Mid::Ach, 0));
        // The sidebar goes down its own column, and ← comes back at the nearest height.
        assert_eq!(s.step(Zone::Side, Mid::Ach, 0, 0, Pad::Down), Move::To(Zone::Side, Mid::Link, 0));
        assert_eq!(s.step(Zone::Side, Mid::Link, 0, 0, Pad::Right), Move::To(Zone::Side, Mid::Link, 1));
        assert_eq!(s.step(Zone::Side, Mid::Ach, 0, 0, Pad::Left), Move::To(Zone::Media, Mid::Ach, 0));
        assert_eq!(s.step(Zone::Side, Mid::Ach, 0, 0, Pad::Up), Move::To(Zone::Actions, Mid::Ach, 0));
        let near = s.step(Zone::Side, Mid::Link, 0, 0, Pad::Left);
        assert!(matches!(near, Move::To(Zone::Media | Zone::About, ..)), "{near:?}");
        // How Long To Beat under the links, when it has a page to open.
        let mut h = s.clone();
        h.hltb = HltbCard::Times { note: false, link: Some("https://howlongtobeat.com/game/1".into()) };
        h.layout(172., 150., 1500., 192.);
        assert_eq!(h.step(Zone::Side, Mid::Link, 1, 0, Pad::Down), Move::To(Zone::Side, Mid::Hltb, 1));
        assert_eq!(h.step(Zone::Side, Mid::Hltb, 1, 0, Pad::Up), Move::To(Zone::Side, Mid::Link, 1));
        assert_eq!(h.step(Zone::Side, Mid::Hltb, 1, 0, Pad::Down), Move::Stay);
        // Without times it shows but takes no focus.
        h.hltb = HltbCard::Missing;
        assert_eq!(h.step(Zone::Side, Mid::Link, 1, 0, Pad::Down), Move::Stay);
    }

    #[test]
    fn a_narrow_page_puts_the_sidebar_under_the_left_column() {
        let mut s = shape();
        s.fit_about(4000, left_width(900.));
        s.layout(172., 150., 900., 192.);
        let g = s.geo;
        assert!(g.stacked && g.left == 900.);
        // Under the last left section, and the page ends after it.
        assert!(g.side > g.related && g.ach == g.side && g.end > g.details);
        // ↓ from the last left section into the sidebar, ↑ back; no crossing sideways.
        assert_eq!(s.step(Zone::Related, Mid::Ach, 0, 0, Pad::Down), Move::To(Zone::Side, Mid::Ach, 0));
        assert_eq!(s.step(Zone::Side, Mid::Ach, 0, 0, Pad::Up), Move::To(Zone::Related, Mid::Ach, 0));
        assert_eq!(s.step(Zone::Media, Mid::Ach, 0, 5, Pad::Right), Move::Stay);
        assert_eq!(s.step(Zone::Side, Mid::Ach, 0, 0, Pad::Left), Move::Stay);
        // A wide page keeps the two columns.
        assert!(!shape().geo.stacked);
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
