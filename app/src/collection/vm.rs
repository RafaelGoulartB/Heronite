//! View models: what the Collection widgets render, precomputed from the
//! data layer so `render` does no formatting or lookups.

use std::sync::Arc;

use gpui_kit::{Pixels, Rgba, SharedString};

use crate::icons::Icon;

/// The line under a card's cover (`data::caption`).
#[derive(Clone)]
pub(super) struct Caption {
    pub icon: Icon,
    pub text: SharedString,
    /// The game has no value for the sort ("Never played", "Not rated").
    pub muted: bool,
    /// Star rating, drawn instead of the text.
    pub stars: Option<u8>,
}

#[derive(Clone)]
pub(super) struct CardVm {
    pub key: SharedString,
    pub title: SharedString,
    /// The playtime, or the value the list is sorted by.
    pub caption: Caption,
    pub installed: bool,
    pub playing: bool,
    /// Hidden in Heroic and shown because "Show hidden games" is on.
    pub hidden: bool,
    /// Index into the images the view keeps (cover chain id).
    pub cover_key: SharedString,
}

#[derive(Clone)]
pub(super) struct GroupVm {
    pub id: SharedString,
    pub name: SharedString,
    pub color: Rgba,
    /// Field icon of a header that is not a status (no colour dot then).
    pub icon: Option<Icon>,
    pub cards: Vec<Arc<CardVm>>,
    pub collapsed: bool,
    /// False when the list is not grouped (one flat grid).
    pub show_header: bool,
}

/// Rows of the virtual list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ListItem {
    Header {
        group: usize,
    },
    Row {
        group: usize,
        start: usize,
    },
    /// Bottom padding of the list (2rem).
    End,
}

pub(super) fn build_items(groups: &[GroupVm], columns: usize) -> Vec<ListItem> {
    let mut items = Vec::new();
    for (gi, g) in groups.iter().enumerate() {
        if g.show_header {
            items.push(ListItem::Header { group: gi });
        }
        if g.collapsed && g.show_header {
            continue;
        }
        let mut start = 0;
        while start < g.cards.len() {
            items.push(ListItem::Row { group: gi, start });
            start += columns;
        }
    }
    items.push(ListItem::End);
    items
}

/// What the top of the list shows, in terms that survive a rebuild of the
/// rows: the group, and the first card of the row (none for a header).
#[derive(Clone, Debug, PartialEq)]
pub(super) struct ScrollAnchor {
    group: SharedString,
    card: Option<SharedString>,
    /// Column of the row's first card when the anchor was taken, to tell a
    /// row that still holds the same cards (keep the pixel offset) from one
    /// that got re-wrapped.
    start: usize,
    offset: Pixels,
}

/// Anchors the item at the top of the list; `None` for the end padding or an
/// index out of range.
pub(super) fn scroll_anchor(items: &[ListItem], groups: &[GroupVm], ix: usize, offset: Pixels) -> Option<ScrollAnchor> {
    match *items.get(ix)? {
        ListItem::Header { group } => Some(ScrollAnchor { group: groups[group].id.clone(), card: None, start: 0, offset }),
        ListItem::Row { group, start } => {
            let g = &groups[group];
            Some(ScrollAnchor { group: g.id.clone(), card: Some(g.cards.get(start)?.key.clone()), start, offset })
        }
        ListItem::End => None,
    }
}

/// Where `anchor` lives in the rebuilt `items`, with the offset to keep. The
/// row holding the anchor card, else (the card is gone or its group
/// collapsed) the group's header; `None` when the group is gone, and the list
/// goes back to the top.
pub(super) fn resolve_anchor(items: &[ListItem], groups: &[GroupVm], columns: usize, anchor: &ScrollAnchor) -> Option<(usize, Pixels)> {
    let gi = groups.iter().position(|g| g.id == anchor.group)?;
    let header = || items.iter().position(|it| *it == ListItem::Header { group: gi }).map(|ix| (ix, Pixels::ZERO));
    let Some(card) = &anchor.card else {
        return header().map(|(ix, _)| (ix, anchor.offset));
    };
    let Some(pos) = groups[gi].cards.iter().position(|c| c.key == *card) else { return header() };
    let start = pos / columns.max(1) * columns.max(1);
    match items.iter().position(|it| *it == ListItem::Row { group: gi, start }) {
        Some(ix) => Some((ix, if start == anchor.start { anchor.offset } else { Pixels::ZERO })),
        None => header(),
    }
}

#[derive(Clone)]
pub(super) enum DetailValue {
    Text(SharedString),
    /// Chips; `facet` makes them clickable filters (`"developer"`, ...).
    Chips {
        values: Vec<SharedString>,
        facet: Option<&'static str>,
    },
    Folder {
        path: SharedString,
        size: Option<SharedString>,
    },
}

#[derive(Clone)]
pub(super) struct DetailRow {
    pub label: SharedString,
    pub value: DetailValue,
    /// Border-top separator (`.is-split`).
    pub split: bool,
}

#[derive(Clone, Default)]
pub(super) struct HltbVm {
    pub main: Option<f32>,
    pub extra: Option<f32>,
    pub complete: Option<f32>,
    pub played_hours: f32,
}

/// A game of "Related games": its cover, title and a line about it.
#[derive(Clone)]
pub(super) struct RelatedGameVm {
    pub key: SharedString,
    pub title: SharedString,
    pub cover_key: SharedString,
    /// Colour of its status, `None` without one.
    pub status_color: Option<Rgba>,
    /// The time played, else its status ("Plan to Play", "Never played").
    pub line: SharedString,
}

#[derive(Clone)]
pub(super) struct RelatedGroupVm {
    pub title: SharedString,
    pub games: Vec<RelatedGameVm>,
}

#[derive(Clone)]
pub(super) struct StatusVm {
    pub id: SharedString,
    pub name: SharedString,
    pub color: Rgba,
}

#[derive(Clone)]
pub(super) struct FocusVm {
    pub key: SharedString,
    pub title: SharedString,
    pub meta_line: SharedString,
    pub cover_key: SharedString,
    pub overview: Option<SharedString>,
    /// Steam store description (takes precedence over `overview`).
    pub overview_html: Option<Arc<Vec<super::html::Block>>>,
    pub related: Vec<RelatedGroupVm>,
    pub details: Vec<DetailRow>,
    pub hltb: Option<HltbVm>,
    /// Current status (left column selector), `None` when the stored id is unknown.
    pub status: Option<StatusVm>,
    /// Critic score of the left column's facts card, `0..=100`.
    pub score: Option<f64>,
    /// Where the score comes from ("IGDB", "Steam"), `None` when typed by hand.
    pub score_source: Option<SharedString>,
    /// "Library" of the facts card: Steam, Epic, GOG...
    pub library: SharedString,
    /// Size shown under the install folder (the folders themselves are
    /// resolved on disk, see `game_facts.rs`).
    pub install_size: Option<SharedString>,
    pub installed: bool,
    pub playing: bool,
    pub favourite: bool,
    /// "12h 27m", the same as the Playtime row of Details.
    pub playtime: SharedString,
    /// When the last session was ("4 days ago", else its date), `None`
    /// when never played.
    pub last_played: Option<SharedString>,
}

#[cfg(test)]
mod tests {
    use gpui_kit::px;

    use super::*;

    fn group(id: &str, cards: &[&str], collapsed: bool) -> GroupVm {
        let card = |key: &str| {
            Arc::new(CardVm {
                key: key.to_string().into(),
                title: key.to_string().into(),
                caption: Caption { icon: Icon::Add, text: "".into(), muted: false, stars: None },
                installed: true,
                playing: false,
                hidden: false,
                cover_key: "".into(),
            })
        };
        GroupVm {
            id: id.to_string().into(),
            name: id.to_string().into(),
            color: Rgba { r: 0., g: 0., b: 0., a: 1. },
            icon: None,
            cards: cards.iter().map(|k| card(k)).collect(),
            collapsed,
            show_header: true,
        }
    }

    fn at(groups: &[GroupVm], columns: usize, ix: usize, offset: f32) -> Option<ScrollAnchor> {
        scroll_anchor(&build_items(groups, columns), groups, ix, px(offset))
    }

    #[test]
    fn same_rows_keep_the_pixel_offset() {
        let groups = [group("a", &["1", "2", "3", "4", "5"], false), group("b", &["6", "7"], false)];
        // items: Header a, Row a 0, Row a 2, Row a 4, Header b, Row b 0, End
        let anchor = at(&groups, 2, 2, 17.).unwrap();
        let items = build_items(&groups, 2);
        assert_eq!(resolve_anchor(&items, &groups, 2, &anchor), Some((2, px(17.))));
    }

    #[test]
    fn a_new_column_count_lands_on_the_row_with_the_card() {
        let groups = [group("a", &["1", "2", "3", "4", "5", "6"], false)];
        let anchor = at(&groups, 2, 3, 9.).unwrap(); // row of cards 5 and 6
        let items = build_items(&groups, 3);
        // Header, Row 0 (1-3), Row 3 (4-6): card 5 is in the last row.
        assert_eq!(resolve_anchor(&items, &groups, 3, &anchor), Some((2, Pixels::ZERO)));
    }

    #[test]
    fn a_header_keeps_its_offset() {
        let groups = [group("a", &["1"], false), group("b", &["2"], false)];
        let anchor = at(&groups, 4, 2, 5.).unwrap();
        assert_eq!(anchor.card, None);
        let items = build_items(&groups, 4);
        assert_eq!(resolve_anchor(&items, &groups, 4, &anchor), Some((2, px(5.))));
    }

    #[test]
    fn a_collapsed_group_falls_back_to_its_header() {
        let anchor = at(&[group("a", &["1", "2"], false)], 4, 1, 3.).unwrap();
        let groups = [group("x", &["9"], false), group("a", &["1", "2"], true)];
        let items = build_items(&groups, 4);
        assert_eq!(resolve_anchor(&items, &groups, 4, &anchor), Some((2, Pixels::ZERO)));
    }

    #[test]
    fn a_moved_card_keeps_its_row_and_a_filtered_out_one_falls_back_to_the_header() {
        let anchor = at(&[group("a", &["1", "2"], false)], 4, 1, 3.).unwrap();
        // The card moved but is still in the group: its row.
        let groups = [group("a", &["2", "3", "4", "5", "1"], false)];
        let items = build_items(&groups, 4);
        assert_eq!(resolve_anchor(&items, &groups, 4, &anchor), Some((2, Pixels::ZERO)));
        let groups = [group("a", &["7"], false)];
        let items = build_items(&groups, 4);
        assert_eq!(resolve_anchor(&items, &groups, 4, &anchor), Some((0, Pixels::ZERO)));
    }

    #[test]
    fn a_vanished_group_goes_to_the_top() {
        let anchor = at(&[group("a", &["1"], false)], 4, 1, 0.).unwrap();
        let groups = [group("b", &["1"], false)];
        assert_eq!(resolve_anchor(&build_items(&groups, 4), &groups, 4, &anchor), None);
    }

    #[test]
    fn the_end_padding_has_no_anchor() {
        let groups = [group("a", &["1"], false)];
        assert_eq!(at(&groups, 4, 2, 0.), None);
    }
}
