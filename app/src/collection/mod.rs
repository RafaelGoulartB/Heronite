//! The Collection screen: stage background, header, grouped list, focus panel.

mod card;
mod focus;
mod header;
mod list;
mod stage;
mod vm;

use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Instant;

use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::theme;
use vm::{CardVm, FocusVm, GroupVm, ListItem};

pub struct CollectionView {
    started: Instant,
    first_frame_logged: bool,
    groups: Vec<GroupVm>,
    items: Vec<ListItem>,
    columns: usize,
    list_state: ListState,
    selected: Option<SharedString>,
    hovered: Option<SharedString>,
    leaving: Option<SharedString>,
    focus: Option<FocusVm>,
    covers: HashMap<SharedString, ImageSource>,
    stage_art: Option<Arc<RenderImage>>,
    scrim: stage::Scrim,
    grey_uninstalled: bool,
    cover_aspect: f32,
}

impl CollectionView {
    pub fn new(started: Instant, _window: &mut Window, cx: &mut Context<Self>) -> Self {
        let list_state = ListState::new(0, ListAlignment::Top, px(500.));
        let weak = cx.entity().downgrade();
        list_state.set_scroll_handler(move |_, _, cx| {
            // The sticky header overlay depends on the scroll position.
            if let Some(view) = weak.upgrade() {
                view.update(cx, |_, cx| cx.notify());
            }
        });
        let mut this = Self {
            started,
            first_frame_logged: false,
            groups: Vec::new(),
            items: Vec::new(),
            columns: 3,
            list_state,
            selected: None,
            hovered: None,
            leaving: None,
            focus: None,
            covers: HashMap::new(),
            stage_art: None,
            scrim: stage::Scrim { dim: 0.14, scrim: 1. },
            grey_uninstalled: true,
            cover_aspect: 1.5,
        };
        this.set_groups(demo_groups());
        if let Ok(key) = std::env::var("HERONITE_DEBUG_SELECT") {
            this.select(key.into());
        }
        this
    }

    fn set_groups(&mut self, groups: Vec<GroupVm>) {
        self.groups = groups;
        self.rebuild_items();
    }

    fn rebuild_items(&mut self) {
        self.items = vm::build_items(&self.groups, self.columns);
        self.list_state.reset(self.items.len());
    }

    fn list_column_width(viewport_w: f32) -> (f32, usize) {
        if viewport_w <= 780. {
            (theme::LIST_COL_W_SM, 1)
        } else if viewport_w <= 1100. {
            (theme::LIST_COL_W_MD, 2)
        } else {
            (theme::LIST_COL_W, 3)
        }
    }

    fn cover_h(&self) -> f32 {
        theme::CARD_W * self.cover_aspect
    }

    fn render_item(&mut self, ix: usize, _window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let Some(item) = self.items.get(ix).copied() else {
            return div().into_any_element();
        };
        match item {
            ListItem::Header { group } => {
                let g = &self.groups[group];
                let gid = g.id.clone();
                list::group_header(g, false)
                    .id(SharedString::from(format!("group-{gid}")))
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if let Some(g) = this.groups.iter_mut().find(|g| g.id == gid) {
                            g.collapsed = !g.collapsed;
                        }
                        this.rebuild_items();
                        cx.notify();
                    }))
                    .into_any_element()
            }
            ListItem::Row { group, start } => {
                let g = &self.groups[group];
                let end = (start + self.columns).min(g.cards.len());
                let last_row = end == g.cards.len();
                let mut row = div()
                    .flex()
                    .gap(px(theme::CARD_GAP_X))
                    .pt(px(if start == 0 { 7.2 } else { 0. }))
                    .pb(px(if last_row { 5.6 + 13.6 } else { theme::CARD_GAP_Y }));
                for c in &g.cards[start..end] {
                    row = row.child(self.render_card(c.clone(), cx));
                }
                row.into_any_element()
            }
            ListItem::End => div().h(px(32.)).into_any_element(),
        }
    }

    fn render_card(&self, c: Arc<CardVm>, cx: &mut Context<Self>) -> AnyElement {
        let key = c.key.clone();
        let (k1, k2, k3) = (key.clone(), key.clone(), key.clone());
        card::card(
            card::CardState {
                vm: &c,
                cover: self.covers.get(&c.cover_key).cloned(),
                cover_h: self.cover_h(),
                selected: self.selected.as_ref() == Some(&key),
                hovered: self.hovered.as_ref() == Some(&key),
                leaving: self.leaving.as_ref() == Some(&key),
                grey_uninstalled: self.grey_uninstalled,
            },
            cx.listener(move |this, _, _, cx| {
                this.select(k1.clone());
                cx.notify();
            }),
            cx.listener(move |this, hovered: &bool, _, cx| {
                if *hovered {
                    this.hovered = Some(k2.clone());
                } else if this.hovered.as_ref() == Some(&k2) {
                    this.hovered = None;
                    this.leaving = Some(k2.clone());
                }
                cx.notify();
            }),
            cx.listener(move |_this, _, _, _cx| {
                log::info!("play {k3}");
            }),
        )
    }

    fn select(&mut self, key: SharedString) {
        self.selected = Some(key.clone());
        let card = self.groups.iter().flat_map(|g| g.cards.iter()).find(|c| c.key == key).cloned();
        self.focus = card.map(|c| demo_focus(&c));
    }

    /// Group whose header should be pinned at the top of the list, and how
    /// far it is pushed up by the next group's header.
    fn sticky(&self) -> Option<(usize, f32)> {
        let top = self.list_state.logical_scroll_top();
        let item = self.items.get(top.item_ix)?;
        let group = match *item {
            ListItem::Header { group } => {
                if top.offset_in_item <= px(0.) {
                    return None; // header itself is at the top, not stuck
                }
                group
            }
            ListItem::Row { group, .. } => group,
            ListItem::End => return None,
        };
        // Push the pinned header up when the next header approaches.
        let next_header = self.items[top.item_ix + 1..]
            .iter()
            .position(|it| matches!(it, ListItem::Header { .. }))
            .map(|p| top.item_ix + 1 + p);
        let mut shift = 0.;
        if let (Some(nh), Some(vp)) = (next_header, Some(self.list_state.viewport_bounds())) {
            if let Some(b) = self.list_state.bounds_for_item(nh) {
                let dist = f32::from(b.origin.y - vp.origin.y);
                if dist < list::GROUP_HEADER_H {
                    shift = dist - list::GROUP_HEADER_H;
                }
            }
        }
        Some((group, shift))
    }
}

impl Render for CollectionView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.first_frame_logged {
            self.first_frame_logged = true;
            log::info!("first render after {:?}", self.started.elapsed());
        }
        let viewport = window.viewport_size();
        let vw = f32::from(viewport.width);
        let (list_w, columns) = Self::list_column_width(vw);
        if columns != self.columns {
            self.columns = columns;
            self.rebuild_items();
        }

        let list_el = div()
            .size_full()
            .pl(px(13.6))
            .pr(px(16.))
            .child(list(self.list_state.clone(), cx.processor(Self::render_item)).size_full());

        let sticky = self.sticky().map(|(g, shift)| {
            div()
                .absolute()
                .top(px(shift))
                .left(px(13.6))
                .right(px(16.))
                .child(list::group_header(&self.groups[g], true))
        });

        let focus_panel = match self.focus.clone() {
            Some(vm) => {
                let covers = self.covers.clone();
                focus::focus_panel(
                    focus::FocusProps {
                        vm,
                        cover_for: Rc::new(move |k| covers.get(k).cloned()),
                        viewport_w: vw,
                        panel_w: vw - list_w,
                        active_facets: Vec::new(),
                    },
                    focus::FocusActions {
                        on_close: Box::new(cx.listener(|this, _: &(), _, cx| {
                            this.focus = None;
                            this.selected = None;
                            cx.notify();
                        }).into_fn()),
                        on_play: Box::new(|_, _| log::info!("play")),
                        on_favourite: Box::new(|_, _| log::info!("favourite")),
                        on_facet: Rc::new(|f, v, _, _| log::info!("facet {f}={v}")),
                        on_select: Rc::new(|k, _, _| log::info!("select {k}")),
                    },
                )
            }
            None => div()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .text_size(px(15.2))
                .text_color(theme::white_a(0.52))
                .child("No game selected")
                .into_any_element(),
        };

        div()
            .id("collection")
            .relative()
            .size_full()
            .overflow_hidden()
            .bg(theme::page_bg())
            .font_family(theme::FONT_BODY)
            .text_color(white())
            .child(stage::stage(self.stage_art.clone(), self.selected.clone().unwrap_or_default(), self.scrim))
            .child(
                div()
                    .absolute()
                    .inset_0()
                    .flex()
                    .flex_col()
                    .child(header::header())
                    .child(
                        div()
                            .flex()
                            .flex_1()
                            .min_h_0()
                            .child(
                                div()
                                    .relative()
                                    .w(px(list_w))
                                    .h_full()
                                    .flex_none()
                                    .bg(linear_gradient(
                                        90.,
                                        linear_color_stop(theme::rgb_a(7, 9, 12, 0.28), 0.),
                                        linear_color_stop(theme::rgb_a(7, 9, 12, 0.1), 0.72),
                                    ))
                                    .child(list_el)
                                    .children(sticky),
                            )
                            .child(div().flex_1().min_w_0().h_full().child(focus_panel)),
                    ),
            )
    }
}

trait IntoFn {
    fn into_fn(self) -> Box<dyn Fn(&mut Window, &mut App)>;
}

impl<F: Fn(&(), &mut Window, &mut App) + 'static> IntoFn for F {
    fn into_fn(self) -> Box<dyn Fn(&mut Window, &mut App)> {
        Box::new(move |w, cx| self(&(), w, cx))
    }
}

// --- temporary demo data until heronite-data's Library lands -------------

fn demo_groups() -> Vec<GroupVm> {
    let mk = |k: &str, t: &str, p: &str, inst: bool| {
        Arc::new(CardVm {
            key: k.to_string().into(),
            title: t.to_string().into(),
            playtime: p.to_string().into(),
            installed: inst,
            playing: false,
            cover_key: k.to_string().into(),
        })
    };
    vec![
        GroupVm {
            id: "playing".into(),
            name: "2. Playing".into(),
            color: theme::status_color("playing"),
            cards: vec![mk("a", "Elden Ring", "15h 38m", true), mk("b", "Control", "3h 20m", true)],
            collapsed: false,
        },
        GroupVm {
            id: "on-going".into(),
            name: "3. On Going".into(),
            color: theme::status_color("endless"),
            cards: vec![
                mk("c", "Megabonk", "47h 15m", true),
                mk("d", "The Finals", "70h 53m", false),
                mk("e", "Brotato", "6h 16m", true),
                mk("f", "Cult of the Lamb", "26m", true),
            ],
            collapsed: false,
        },
        GroupVm {
            id: "plan".into(),
            name: "4. Plan to Play".into(),
            color: theme::status_color("plan-to-play"),
            cards: vec![mk("g", "Metal Gear Solid 4", "Not Played", false)],
            collapsed: false,
        },
    ]
}

fn demo_focus(c: &CardVm) -> FocusVm {
    use vm::*;
    FocusVm {
        key: c.key.clone(),
        title: c.title.clone(),
        meta_line: "2/24/2022 · Xbox Series X|S · Role-playing (RPG) · Adventure".into(),
        cover_key: c.cover_key.clone(),
        overview: Some("Elden Ring is an action RPG developed by FromSoftware.".into()),
        related: vec![],
        details: vec![
            DetailRow {
                label: "Status".into(),
                value: DetailValue::Status { name: "2. Playing".into(), color: theme::status_color("playing") },
                split: false,
            },
            DetailRow {
                label: "Developer".into(),
                value: DetailValue::Chips { values: vec!["FromSoftware".into()], facet: Some("developer"), clamp: false },
                split: false,
            },
            DetailRow { label: "Playtime".into(), value: DetailValue::Text(c.playtime.clone()), split: false },
            DetailRow {
                label: "Installation Folder".into(),
                value: DetailValue::Folder { path: "/data/Games/x".into(), size: None },
                split: true,
            },
        ],
        hltb: Some(HltbVm { main: Some(6.7), extra: Some(18.8), complete: None, played_hours: 0.9 }),
        installed: c.installed,
        playing: c.playing,
        favourite: false,
    }
}
