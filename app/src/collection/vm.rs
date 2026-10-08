//! View models: what the Collection widgets render, precomputed from the
//! data layer so `render` does no formatting or lookups.

use std::sync::Arc;

use gpui_kit::{Rgba, SharedString};

#[derive(Clone)]
pub(super) struct CardVm {
    pub key: SharedString,
    pub title: SharedString,
    pub playtime: SharedString,
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
    pub cards: Vec<Arc<CardVm>>,
    pub collapsed: bool,
    /// False when "Group by status" is off (one flat grid).
    pub show_header: bool,
}

/// Rows of the virtual list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ListItem {
    Header { group: usize },
    Row { group: usize, start: usize },
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

#[derive(Clone)]
pub(super) enum DetailValue {
    Text(SharedString),
    Status { name: SharedString, color: Rgba },
    /// Chips; `facet` makes them clickable filters (`"developer"`, ...).
    Chips { values: Vec<SharedString>, facet: Option<&'static str>, clamp: bool },
    Folder { path: SharedString, size: Option<SharedString> },
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

#[derive(Clone)]
pub(super) struct RelatedGroupVm {
    pub title: SharedString,
    pub games: Vec<Arc<CardVm>>,
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
    pub installed: bool,
    pub playing: bool,
    pub favourite: bool,
}
