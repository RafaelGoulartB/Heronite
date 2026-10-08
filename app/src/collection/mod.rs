//! The Collection screen: stage background, header, grouped list, focus panel.

mod card;
mod data;
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

use heronite_data::hltb::WikiCache;
use heronite_data::ui_state::UiState;
use heronite_data::{art, AppPaths, ArtContext, FacetKind, HeroicPaths, Library};

use crate::{images, theme};
use vm::{CardVm, FocusVm, GroupVm, ListItem};

/// Everything loaded from disk off the UI thread.
struct Loaded {
    lib: Library,
    wiki: WikiCache,
    covers: HashMap<SharedString, std::path::PathBuf>,
    stages: HashMap<SharedString, std::path::PathBuf>,
}

fn load_all(heroic: &HeroicPaths) -> anyhow::Result<Loaded> {
    let t = Instant::now();
    let lib = Library::load(heroic)?;
    let wiki = WikiCache::load(heroic);
    let shots = (!lib.settings.screenshots.folder.is_empty())
        .then(|| std::path::PathBuf::from(&lib.settings.screenshots.folder));
    let ctx = ArtContext::new(heroic, shots.as_deref());
    let cache = heroic.images_cache_dir();
    let mut covers = HashMap::new();
    let mut stages = HashMap::new();
    for g in &lib.games {
        let key: SharedString = g.key.clone().into();
        if let Some(p) = images::first_local(&art::card_cover(&ctx, g), &cache) {
            covers.insert(key.clone(), p);
        }
        if let Some(p) = images::first_local(&art::stage_art(&ctx, g), &cache) {
            stages.insert(key, p);
        }
    }
    log::info!("library: {} games loaded in {:?}", lib.games.len(), t.elapsed());
    Ok(Loaded { lib, wiki, covers, stages })
}

pub struct CollectionView {
    started: Instant,
    first_frame_logged: bool,
    heroic: HeroicPaths,
    app_paths: AppPaths,
    lib: Option<Arc<Library>>,
    wiki: Arc<WikiCache>,
    ui: UiState,
    search: String,
    stages: HashMap<SharedString, ImageSource>,
    groups: Vec<GroupVm>,
    items: Vec<ListItem>,
    columns: usize,
    list_state: ListState,
    selected: Option<SharedString>,
    hovered: Option<SharedString>,
    leaving: Option<SharedString>,
    focus: Option<FocusVm>,
    covers: HashMap<SharedString, ImageSource>,
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
        let heroic = HeroicPaths::detect();
        let app_paths = AppPaths::detect();
        let ui = if app_paths.ui_state().exists() {
            UiState::load(&app_paths)
        } else {
            // First run: start where Heronite left off.
            let ui = heronite_data::heronite_ui_import::import(&heroic).unwrap_or_default();
            if let Err(err) = ui.save(&app_paths) {
                log::warn!("saving imported ui state: {err:#}");
            }
            ui
        };
        let this = Self {
            started,
            first_frame_logged: false,
            heroic: heroic.clone(),
            app_paths,
            lib: None,
            wiki: Arc::new(WikiCache::default()),
            ui,
            search: String::new(),
            stages: HashMap::new(),
            groups: Vec::new(),
            items: Vec::new(),
            columns: 3,
            list_state,
            selected: None,
            hovered: None,
            leaving: None,
            focus: None,
            covers: HashMap::new(),
            scrim: stage::Scrim { dim: 0.14, scrim: 1. },
            grey_uninstalled: true,
            cover_aspect: 1.5,
        };
        cx.spawn(async move |this, cx| {
            let loaded = cx.background_spawn(async move { load_all(&heroic) }).await;
            this.update(cx, |this, cx| match loaded {
                Ok(l) => this.apply_loaded(l, cx),
                Err(err) => log::error!("loading library: {err:#}"),
            })
            .ok();
        })
        .detach();
        this
    }

    fn apply_loaded(&mut self, l: Loaded, cx: &mut Context<Self>) {
        let bg = &l.lib.settings.background;
        self.scrim = stage::Scrim { dim: bg.dimming.get() as f32 / 100., scrim: bg.scrim.get() as f32 / 100. };
        self.grey_uninstalled = l.lib.settings.grey_uninstalled_games && !l.lib.settings.all_tiles_in_color;
        self.covers = l.covers.into_iter().map(|(k, p)| (k, ImageSource::from(p))).collect();
        self.stages = l.stages.into_iter().map(|(k, p)| (k, ImageSource::from(p))).collect();
        self.wiki = Arc::new(l.wiki);
        self.lib = Some(Arc::new(l.lib));
        self.refresh_groups();
        let focused = std::env::var("HERONITE_DEBUG_SELECT").ok().or_else(|| self.ui.focused_game.clone());
        if let Some(key) = focused {
            self.select(key.into(), false);
        }
        log::info!("library on screen after {:?}", self.started.elapsed());
        cx.notify();
    }

    fn refresh_groups(&mut self) {
        let Some(lib) = self.lib.clone() else { return };
        let groups = data::build_groups(&lib, &self.ui, &self.search, &|_| false);
        self.set_groups(groups);
    }

    fn save_ui(&self) {
        if let Err(err) = self.ui.save(&self.app_paths) {
            log::warn!("saving ui state: {err:#}");
        }
    }

    fn toggle_facet(&mut self, kind: &str, value: &str) {
        if let Some(kind) = FacetKind::parse(kind) {
            self.ui.toggle_facet(kind, value);
            self.save_ui();
            self.refresh_groups();
        }
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
                        this.ui.toggle_group(&gid);
                        this.save_ui();
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
                this.select(k1.clone(), true);
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

    fn select(&mut self, key: SharedString, persist: bool) {
        let Some(lib) = self.lib.clone() else { return };
        let Some(game) = lib.get(&key) else { return };
        self.focus = Some(data::focus_vm(&lib, &self.wiki, game, false));
        self.selected = Some(key.clone());
        if persist {
            self.ui.focused_game = Some(key.to_string());
            self.save_ui();
        }
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

        let weak = cx.entity().downgrade();
        let active_facets: Vec<(&'static str, SharedString)> = heronite_data::collection::FACET_KINDS
            .iter()
            .filter_map(|k| self.ui.facet_filters.get(*k).map(|v| (k.as_str(), SharedString::from(v.to_string()))))
            .collect();
        let focus_panel = match self.focus.clone() {
            Some(vm) => {
                let (w1, w2) = (weak.clone(), weak.clone());
                let covers = self.covers.clone();
                focus::focus_panel(
                    focus::FocusProps {
                        vm,
                        cover_for: Rc::new(move |k| covers.get(k).cloned()),
                        viewport_w: vw,
                        panel_w: vw - list_w,
                        active_facets,
                    },
                    focus::FocusActions {
                        on_close: Box::new(cx.listener(|this, _: &(), _, cx| {
                            this.focus = None;
                            this.selected = None;
                            cx.notify();
                        }).into_fn()),
                        on_play: Box::new(|_, _| log::info!("play")),
                        on_favourite: Box::new(|_, _| log::info!("favourite")),
                        on_facet: Rc::new(move |f, v, _, cx| {
                            w1.update(cx, |this, cx| {
                                this.toggle_facet(f, &v);
                                cx.notify();
                            })
                            .ok();
                        }),
                        on_select: Rc::new(move |k, _, cx| {
                            w2.update(cx, |this, cx| {
                                this.select(k, true);
                                cx.notify();
                            })
                            .ok();
                        }),
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
            .child(stage::stage(
                self.selected.as_ref().and_then(|k| self.stages.get(k).cloned()),
                self.selected.clone().unwrap_or_default(),
                self.scrim,
            ))
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

