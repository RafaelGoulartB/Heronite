//! The Collection screen: stage background, header, grouped list, focus panel.

mod card;
mod context_menu;
mod data;
mod emulation;
mod focus;
mod html;
mod header;
mod list;
mod metadata_ui;
mod playnite_menu;
mod screenshot_viewer;
mod screenshots;
pub use screenshots::screenshots_global;
pub mod stage;
mod vm;

use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Instant;

use futures::StreamExt;
use gpui_kit::component::input::InputState;
use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::hltb::WikiCache;
use heronite_data::ui_state::UiState;
use heronite_data::{art, AppPaths, ArtContext, FacetKind, HeroicPaths, Library};
use heronite_images::{ImageRef, Variant};

use crate::images::{to_chain, ImageStore};
use crate::runtime::GameRuntime;
use heronite_launch::LaunchEvent;
use crate::theme;
use header::Menu;
use vm::{CardVm, FocusVm, GroupVm, ListItem};

/// Everything loaded from disk off the UI thread.
struct Loaded {
    lib: Library,
    wiki: WikiCache,
    covers: HashMap<SharedString, Vec<ImageRef>>,
    stages: HashMap<SharedString, Vec<ImageRef>>,
}

fn load_all(heroic: &HeroicPaths) -> anyhow::Result<Loaded> {
    let t = Instant::now();
    let lib = Library::load(heroic)?;
    let wiki = WikiCache::load(heroic);
    let shots = (!lib.settings.screenshots.folder.is_empty())
        .then(|| std::path::PathBuf::from(&lib.settings.screenshots.folder));
    let ctx = ArtContext::new(heroic, shots.as_deref());
    let mut covers = HashMap::new();
    let mut stages = HashMap::new();
    for g in &lib.games {
        let key: SharedString = g.key.clone().into();
        covers.insert(key.clone(), to_chain(&art::card_cover(&ctx, g)));
        stages.insert(key, to_chain(&art::stage_art(&ctx, g)));
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
    search_input: Entity<InputState>,
    open_menu: Option<Menu>,
    menu_close: Option<Task<()>>,
    facet_flyout: Option<FacetKind>,
    refreshing: bool,
    runtime: Arc<GameRuntime>,
    /// app_name -> started tracking (Launching/Running).
    playing: HashMap<String, bool>,
    toast: Option<(SharedString, bool)>,
    context_menu: Option<context_menu::ContextMenuState>,
    focus_handle: FocusHandle,
    dialog: Option<AnyView>,
    /// `HERONITE_DEBUG_DIALOG` (opened on the first render with a window).
    pending_debug_dialog: Option<String>,
    dialog_sub: Option<Subscription>,
    emu: emulation::EmuState,
    playnite: playnite_menu::PlayniteState,
    focus_scroll: ScrollHandle,
    shots: screenshots::Shots,
    toast_task: Option<Task<()>>,
    images: ImageStore,
    cover_chains: HashMap<SharedString, Vec<ImageRef>>,
    stage_chains: HashMap<SharedString, Vec<ImageRef>>,
    /// Last stage texture shown, kept while the next one loads.
    shown_stage: Option<(SharedString, Arc<RenderImage>)>,
    groups: Vec<GroupVm>,
    items: Vec<ListItem>,
    columns: usize,
    list_state: ListState,
    selected: Option<SharedString>,
    hovered: Option<SharedString>,
    leaving: Option<SharedString>,
    focus: Option<FocusVm>,
    scrim: stage::Scrim,
    blur_px: f32,
    saturation: f32,
    stage_enabled: bool,
    grey_uninstalled: bool,
    cover_aspect: f32,
}

impl CollectionView {
    pub fn new(started: Instant, window: &mut Window, cx: &mut Context<Self>) -> Self {
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
        let (images, mut rx) = ImageStore::new(heroic.images_cache_dir(), &app_paths);
        cx.spawn(async move |this, cx| {
            while let Some(first) = rx.next().await {
                // Batch whatever else already arrived into one notify.
                let mut batch = vec![first];
                while let Ok(Some(more)) = rx.try_next() {
                    batch.push(more);
                }
                let alive = this
                    .update(cx, |this, cx| {
                        for item in batch {
                            this.images.on_loaded(item);
                        }
                        cx.notify();
                    })
                    .is_ok();
                if !alive {
                    break;
                }
            }
        })
        .detach();

        let (runtime, mut events) = GameRuntime::new(heroic.clone());
        cx.spawn(async move |this, cx| {
            while let Some(ev) = events.next().await {
                if this.update(cx, |this, cx| this.on_launch_event(ev, cx)).is_err() {
                    break;
                }
            }
        })
        .detach();

        let search_input = Self::new_search_input(window, cx);
        let this = Self {
            started,
            first_frame_logged: false,
            heroic,
            app_paths,
            lib: None,
            wiki: Arc::new(WikiCache::default()),
            ui,
            search: String::new(),
            search_input,
            open_menu: None,
            menu_close: None,
            facet_flyout: None,
            refreshing: false,
            runtime: Arc::new(runtime),
            playing: HashMap::new(),
            toast: None,
            toast_task: None,
            context_menu: None,
            focus_handle: cx.focus_handle(),
            dialog: None,
            pending_debug_dialog: None,
            dialog_sub: None,
            emu: Default::default(),
            playnite: Default::default(),
            focus_scroll: ScrollHandle::new(),
            shots: Default::default(),
            images,
            cover_chains: HashMap::new(),
            stage_chains: HashMap::new(),
            shown_stage: None,
            groups: Vec::new(),
            items: Vec::new(),
            columns: 3,
            list_state,
            selected: None,
            hovered: None,
            leaving: None,
            focus: None,
            scrim: stage::Scrim { dim: 0.14, scrim: 1. },
            blur_px: 6.,
            saturation: 0.94,
            stage_enabled: true,
            grey_uninstalled: true,
            cover_aspect: 1.5,
        };
        window.focus(&this.focus_handle, cx);
        this.spawn_load(cx);
        this.start_idle_jobs(cx);
        this.init_metadata(window, cx);
        // Heronite may write the same files: reload when they change.
        cx.spawn(async move |this, cx| loop {
            cx.background_executor().timer(std::time::Duration::from_secs(2)).await;
            let Ok(changed) = this.update(cx, |this, _| {
                !this.refreshing && this.lib.as_ref().is_some_and(|l| l.changed_on_disk())
            }) else {
                break;
            };
            if changed {
                if this.update(cx, |this, cx| this.reload(cx)).is_err() {
                    break;
                }
            }
        })
        .detach();
        this
    }

    fn spawn_load(&self, cx: &mut Context<Self>) {
        let heroic = self.heroic.clone();
        cx.spawn(async move |this, cx| {
            let loaded = cx.background_spawn(async move { load_all(&heroic) }).await;
            this.update(cx, |this, cx| {
                this.refreshing = false;
                match loaded {
                    Ok(l) => this.apply_loaded(l),
                    Err(err) => log::error!("loading library: {err:#}"),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub(super) fn reload(&mut self, cx: &mut Context<Self>) {
        if self.refreshing {
            return;
        }
        self.refreshing = true;
        self.images.clear_failed();
        self.spawn_load(cx);
        cx.notify();
    }

    fn apply_loaded(&mut self, l: Loaded) {
        let first = self.lib.is_none();
        let s = &l.lib.settings;
        let bg = &s.background;
        self.scrim = stage::Scrim { dim: bg.dimming.get() as f32 / 100., scrim: bg.scrim.get() as f32 / 100. };
        self.blur_px = bg.blur.get() as f32;
        self.saturation = bg.saturation.get() as f32 / 100.;
        self.stage_enabled = bg.enabled;
        self.grey_uninstalled = s.grey_uninstalled_games && !s.all_tiles_in_color;
        self.cover_chains = l.covers;
        self.stage_chains = l.stages;
        self.wiki = Arc::new(l.wiki);
        self.lib = Some(Arc::new(l.lib));
        self.refresh_groups();
        let focused = if first {
            std::env::var("HERONITE_DEBUG_SELECT").ok().or_else(|| self.ui.focused_game.clone())
        } else {
            self.selected.as_ref().map(|s| s.to_string())
        };
        if let Some(key) = focused {
            self.select(key.into(), false);
            if first {
                self.reveal_selected();
            }
        }
        if first {
            log::info!("library on screen after {:?}", self.started.elapsed());
            self.apply_debug_env();
        }
    }

    fn dialog_ctx(&self) -> Option<crate::dialogs::DialogCtx> {
        Some(crate::dialogs::DialogCtx {
            heroic: self.heroic.clone(),
            app: self.app_paths.clone(),
            lib: self.lib.clone()?,
        })
    }

    /// Hosts `view` as the open dialog and reacts to its events.
    pub(super) fn open_dialog<V>(&mut self, view: Entity<V>, cx: &mut Context<Self>)
    where
        V: Render + EventEmitter<crate::dialogs::DialogEvent>,
    {
        use crate::dialogs::DialogEvent;
        self.open_menu = None;
        self.context_menu = None;
        self.dialog_sub = Some(cx.subscribe(&view, |this, _, ev: &DialogEvent, cx| {
            match ev {
                DialogEvent::Close => {
                    this.dialog = None;
                    this.dialog_sub = None;
                }
                DialogEvent::Changed => this.reload(cx),
                DialogEvent::ChangedAndClose => {
                    this.dialog = None;
                    this.dialog_sub = None;
                    this.reload(cx);
                }
                DialogEvent::Open(req) => {
                    this.dialog = None;
                    this.dialog_sub = None;
                    this.emu.requested = Some(req.clone());
                }
            }
            cx.notify();
        }));
        self.dialog = Some(view.into());
        cx.notify();
    }

    pub(super) fn open_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(ctx) = self.dialog_ctx() else { return };
        let chain = self.selected.as_ref().and_then(|k| self.stage_chains.get(k).cloned()).unwrap_or_default();
        let view = cx.new(|cx| crate::dialogs::settings::SettingsDialog::new(ctx, chain, window, cx));
        self.open_dialog(view, cx);
    }

    pub(super) fn open_manage_statuses(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(ctx) = self.dialog_ctx() else { return };
        let view = cx.new(|cx| crate::dialogs::manage_statuses::ManageStatusesDialog::new(ctx, window, cx));
        self.open_dialog(view, cx);
    }

    /// Screenshot helpers: `HERONITE_DEBUG_MENU=<card key>` opens its context
    /// menu (with the status flyout), `HERONITE_DEBUG_DROPDOWN=install|sort|
    /// status|facets` opens a header menu.
    fn apply_debug_env(&mut self) {
        if let Ok(key) = std::env::var("HERONITE_DEBUG_MENU") {
            self.context_menu = Some(context_menu::ContextMenuState {
                key: key.into(),
                position: point(px(200.), px(160.)),
                status_open: true,
            });
        }
        if let Ok(which) = std::env::var("HERONITE_DEBUG_DIALOG") {
            self.pending_debug_dialog = Some(which);
        }
        if let Ok(menu) = std::env::var("HERONITE_DEBUG_DROPDOWN") {
            self.open_menu = match menu.as_str() {
                "install" => Some(Menu::Install),
                "sort" => Some(Menu::Sort),
                "status" => Some(Menu::Status),
                "playnite" => Some(Menu::Playnite),
                "facets" => {
                    self.facet_flyout = Some(FacetKind::Developer);
                    Some(Menu::Facets)
                }
                _ => None,
            };
        }
    }

    fn refresh_groups(&mut self) {
        let Some(lib) = self.lib.clone() else { return };
        let playing = &self.playing;
        let groups = data::build_groups(&lib, &self.ui, &self.search, &|key| {
            lib.get(key).is_some_and(|g| playing.contains_key(&g.app_name))
        });
        self.set_groups(groups);
        self.refresh_focus();
    }

    fn refresh_focus(&mut self) {
        let (Some(lib), Some(key)) = (self.lib.clone(), self.selected.clone()) else { return };
        if let Some(game) = lib.get(&key) {
            let playing = self.playing.contains_key(&game.app_name);
            self.focus = Some(data::focus_vm(&lib, &self.wiki, game, playing));
        }
    }

    fn show_toast(&mut self, text: impl Into<SharedString>, error: bool, cx: &mut Context<Self>) {
        self.toast = Some((text.into(), error));
        self.toast_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(std::time::Duration::from_secs(5)).await;
            this.update(cx, |this, cx| {
                this.toast = None;
                cx.notify();
            })
            .ok();
        }));
        cx.notify();
    }

    fn on_launch_event(&mut self, ev: LaunchEvent, cx: &mut Context<Self>) {
        self.shots_launch_event(&ev, cx);
        match ev {
            LaunchEvent::Launching { app_name } => {
                self.playing.insert(app_name, false);
            }
            LaunchEvent::Running { app_name, pid } => {
                log::info!("{app_name} running as pid {pid}");
                self.playing.insert(app_name, true);
            }
            LaunchEvent::Exited { app_name, elapsed, .. } => {
                log::info!("{app_name} exited after {elapsed:?}");
                self.playing.remove(&app_name);
                self.emu_session_ended(&app_name, elapsed, cx);
                // Playtime, last played and recent games changed on disk.
                self.reload(cx);
            }
            LaunchEvent::Error { app_name, message } => {
                self.playing.remove(&app_name);
                self.show_toast(message, true, cx);
            }
        }
        self.refresh_groups();
        cx.notify();
    }

    /// Card play button / focus primary button.
    fn primary_action(&mut self, key: &str, cx: &mut Context<Self>) {
        let Some(lib) = self.lib.clone() else { return };
        let Some(game) = lib.get(key).cloned() else { return };
        if self.playing.contains_key(&game.app_name) {
            if !self.runtime.stop(&game.app_name) {
                self.runtime.force_clear(&game.app_name);
            }
            return;
        }
        if game.installed && self.start_emulated(&game, cx) {
            return;
        }
        let result = if game.installed { self.runtime.play(&game) } else { self.runtime.install(&game) };
        if let Err(err) = result {
            self.show_toast(err, true, cx);
        }
    }

    fn toggle_favourite(&mut self, cx: &mut Context<Self>) {
        let (Some(lib), Some(key)) = (self.lib.clone(), self.selected.clone()) else { return };
        let Some(game) = lib.get(&key) else { return };
        if let Err(err) = heronite_data::config_games::toggle_favourite(&self.heroic, &game.app_name, &game.title) {
            self.show_toast(format!("Could not update favourites: {err:#}"), true, cx);
        }
        self.reload(cx);
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

    /// Scrolls the list so the selected card's row is visible
    /// (`scrollIntoView({ block: 'nearest' })`).
    fn reveal_selected(&mut self) {
        let Some(sel) = self.selected.clone() else { return };
        let ix = self.items.iter().position(|it| match *it {
            ListItem::Row { group, start } => {
                let g = &self.groups[group];
                g.cards[start..(start + self.columns).min(g.cards.len())].iter().any(|c| c.key == sel)
            }
            _ => false,
        });
        if let Some(ix) = ix {
            let viewport = self.list_state.viewport_bounds();
            if viewport.size.height > px(0.) && self.list_state.bounds_for_item(ix).is_some() {
                self.list_state.scroll_to_reveal_item(ix);
            } else {
                // Not laid out yet: put the row (and its group header when it
                // is the group's first row) at the top.
                let top = if ix > 0 && matches!(self.items[ix - 1], ListItem::Header { .. }) { ix - 1 } else { ix };
                self.list_state.scroll_to(ListOffset { item_ix: top, offset_in_item: px(0.) });
            }
        }
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

    /// Cover texture for `key` at `w`×`h` logical px (requested on demand).
    fn cover(&mut self, key: &SharedString, w: f32, h: f32, scale: f32, priority: i32) -> Option<ImageSource> {
        let (pw, ph) = ((w * scale).round() as u32, (h * scale).round() as u32);
        let id: SharedString = format!("c:{key}:{pw}x{ph}").into();
        let chains = &self.cover_chains;
        self.images
            .get_or_request(&id, || chains.get(key).cloned().unwrap_or_default(), pw, ph, Variant::Cover, priority)
            .map(ImageSource::Render)
    }

    fn render_item(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let Some(item) = self.items.get(ix).copied() else {
            return div().into_any_element();
        };
        match item {
            ListItem::Header { group } => {
                let g = &self.groups[group];
                let gid = g.id.clone();
                let header = list::group_header(g, false)
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
                    }));
                div().pl(px(13.6)).pr(px(16.)).child(header).into_any_element()
            }
            ListItem::Row { group, start } => {
                let cards: Vec<Arc<CardVm>> = {
                    let g = &self.groups[group];
                    g.cards[start..(start + self.columns).min(g.cards.len())].to_vec()
                };
                let last_row = start + cards.len() == self.groups[group].cards.len();
                let mut row = div()
                    .flex()
                    .pl(px(13.6))
                    .pr(px(16.))
                    .gap(px(theme::CARD_GAP_X))
                    .pt(px(if start == 0 { 7.2 } else { 0. }))
                    .pb(px(if last_row { 5.6 + 13.6 } else { theme::CARD_GAP_Y }));
                let scale = window.scale_factor();
                for c in cards {
                    row = row.child(self.render_card(c, scale, cx));
                }
                row.into_any_element()
            }
            ListItem::End => div().h(px(32.)).into_any_element(),
        }
    }

    fn render_card(&mut self, c: Arc<CardVm>, scale: f32, cx: &mut Context<Self>) -> AnyElement {
        let key = c.key.clone();
        let cover_h = self.cover_h();
        let cover = self.cover(&c.cover_key, theme::CARD_W, cover_h, scale, 100);
        let (k1, k2, k3, k4) = (key.clone(), key.clone(), key.clone(), key.clone());
        let el = card::card(
            card::CardState {
                vm: &c,
                cover,
                cover_h,
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
            cx.listener(move |this, _, _, cx| {
                this.primary_action(&k3, cx);
                cx.notify();
            }),
        );
        div()
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, ev: &MouseDownEvent, _, cx| {
                    this.open_context_menu(k4.clone(), ev.position, cx);
                }),
            )
            .child(el)
            .into_any_element()
    }

    fn select(&mut self, key: SharedString, persist: bool) {
        let Some(lib) = self.lib.clone() else { return };
        let Some(game) = lib.get(&key) else { return };
        let playing = self.playing.contains_key(&game.app_name);
        self.focus = Some(data::focus_vm(&lib, &self.wiki, game, playing));
        if self.selected.as_ref() != Some(&key) {
            self.focus_scroll.set_offset(point(px(0.), px(0.)));
        }
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
        if !self.groups[group].show_header {
            return None;
        }
        let next_header = self.items[top.item_ix + 1..]
            .iter()
            .position(|it| matches!(it, ListItem::Header { .. }))
            .map(|p| top.item_ix + 1 + p);
        let mut shift = 0.;
        if let Some(nh) = next_header {
            let vp = self.list_state.viewport_bounds();
            if let Some(b) = self.list_state.bounds_for_item(nh) {
                let dist = f32::from(b.origin.y - vp.origin.y);
                if dist < list::GROUP_HEADER_H {
                    shift = dist - list::GROUP_HEADER_H;
                }
            }
        }
        Some((group, shift))
    }

    /// Current stage texture: the selected game's, or the previous one while
    /// it loads.
    fn stage_image(&mut self, window: &Window) -> Option<(SharedString, ImageSource)> {
        if !self.stage_enabled {
            return None;
        }
        let key = self.selected.clone()?;
        let scale = window.scale_factor();
        let size = window.viewport_size();
        let (pw, ph) = ((f32::from(size.width) * scale) as u32, (f32::from(size.height) * scale) as u32);
        let blur = self.blur_px * scale;
        let sat = self.saturation;
        let id: SharedString = format!("s:{key}:{pw}x{ph}:{blur}:{sat}").into();
        let chains = &self.stage_chains;
        let ready = self.images.get_or_request(
            &id,
            || chains.get(&key).cloned().unwrap_or_default(),
            pw,
            ph,
            Variant::Stage { blur_px: blur, saturation: sat },
            1000,
        );
        if let Some(img) = ready {
            self.shown_stage = Some((key, img));
        } else if self.stage_chains.get(&key).is_none_or(|c| c.is_empty()) {
            self.shown_stage = None;
        }
        self.shown_stage.as_ref().map(|(k, img)| (k.clone(), ImageSource::Render(img.clone())))
    }
}

impl Render for CollectionView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.first_frame_logged {
            self.first_frame_logged = true;
            log::info!("first render after {:?}", self.started.elapsed());
        }
        self.images.drop_evicted(window);
        if let Some(which) = self.pending_debug_dialog.take() {
            match which.as_str() {
                "settings" => self.open_settings(window, cx),
                "statuses" => self.open_manage_statuses(window, cx),
                "gallery" | "viewer" | "viewer-zoom" | "viewer-fit" | "confirm" | "delete" => self.shots.debug = Some(which),
                "settings:screenshots" => {
                    self.open_settings(window, cx);
                    if let Some(d) = self.dialog.clone().and_then(|v| v.downcast::<crate::dialogs::settings::SettingsDialog>().ok()) {
                        d.update(cx, |d, _| d.tab = crate::dialogs::settings::Tab::Screenshots);
                    }
                }
                "metadata" | "gamesettings" | "gamesettings:details" | "gamesettings:launch" | "webimage"
                | "settings:metadata" | "wizard" => self.open_metadata_debug(which.as_str(), window, cx),
                "playnite:import" => self.open_playnite_import(window, cx),
                "playnite:merge" => self.open_playnite_merge(window, cx),
                other => {
                    self.open_emu_debug_dialog(other, window, cx);
                }
            }
        }
        self.drive_emulation(window, cx);
        self.sync_shots(window, cx);
        let viewport = window.viewport_size();
        let vw = f32::from(viewport.width);
        let scale = window.scale_factor();
        let (list_w, columns) = Self::list_column_width(vw);
        if columns != self.columns {
            self.columns = columns;
            self.rebuild_items();
        }

        let header = self.render_header(cx);
        let stage_el = match self.stage_image(window) {
            Some((key, src)) => stage::stage(Some(src), key, self.scrim),
            None => stage::stage(None, SharedString::default(), self.scrim),
        };

        // The list's padding lives inside each row: GPUI clips list content to
        // the list bounds, and the selection ring sits 2px outside the cover.
        let list_el = div()
            .size_full()
            .child(list(self.list_state.clone(), cx.processor(Self::render_item)).size_full());

        let list_scrollbar = crate::ui::scrollbar(
            self.list_state.scroll_px_offset_for_scrollbar().y,
            self.list_state.max_offset_for_scrollbar().y,
            self.list_state.viewport_bounds().size.height,
        );
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
                // Resolve every cover the panel shows up front.
                let mut covers: HashMap<SharedString, ImageSource> = HashMap::new();
                let narrow = vw - list_w <= 1100. - theme::LIST_COL_W_MD;
                let fw = if narrow { 72. } else { 86. };
                if let Some(src) = self.cover(&vm.cover_key, fw, fw * 1.5, scale, 200) {
                    covers.insert(vm.cover_key.clone(), src);
                }
                for g in &vm.related {
                    for c in &g.games {
                        if let Some(src) = self.cover(&c.cover_key, 78., 117., scale, 50) {
                            covers.insert(c.cover_key.clone(), src);
                        }
                    }
                }
                let mut html_images: HashMap<String, ImageSource> = HashMap::new();
                if let Some(blocks) = vm.overview_html.clone() {
                    let column = focus::main_column_width(vw - list_w);
                    for b in blocks.iter() {
                        if let html::Block::Image { url, width, height } = b {
                            let (w, h) = focus::html_image_size(*width, *height, column);
                            let (pw, ph) = ((w * scale).round() as u32, (h * scale).round() as u32);
                            let id: SharedString = format!("h:{url}:{pw}x{ph}").into();
                            let chain = vec![ImageRef::Url(url.clone())];
                            if let Some(img) =
                                self.images.get_or_request(&id, || chain, pw, ph, Variant::Cover, 20)
                            {
                                html_images.insert(url.clone(), ImageSource::Render(img));
                            }
                        }
                    }
                }
                let (w1, w2, w3, w4) = (weak.clone(), weak.clone(), weak.clone(), weak.clone());
                let (w5, w6) = (weak.clone(), weak.clone());
                let focus_key = vm.key.clone();
                let screenshots = self.screenshots_section(cx);
                focus::focus_panel(
                    focus::FocusProps {
                        vm,
                        cover_for: Rc::new(move |k| covers.get(k).cloned()),
                        html_images: Rc::new(move |u| html_images.get(u).cloned()),
                        scroll: self.focus_scroll.clone(),
                        viewport_w: vw,
                        panel_w: vw - list_w,
                        active_facets,
                        screenshots,
                    },
                    focus::FocusActions {
                        on_scroll: Box::new(cx.listener(|_, _: &(), _, cx| cx.notify()).into_fn()),
                        on_close: Box::new(
                            cx.listener(|this, _: &(), _, cx| {
                                this.focus = None;
                                this.selected = None;
                                cx.notify();
                            })
                            .into_fn(),
                        ),
                        on_play: Box::new(move |_, cx| {
                            w3.update(cx, |this, cx| {
                                this.primary_action(&focus_key, cx);
                                cx.notify();
                            })
                            .ok();
                        }),
                        on_favourite: Box::new(move |_, cx| {
                            w4.update(cx, |this, cx| this.toggle_favourite(cx)).ok();
                        }),
                        on_metadata: Box::new(move |window, cx| {
                            w5.update(cx, |this, cx| this.open_metadata(window, cx)).ok();
                        }),
                        on_game_settings: Box::new(move |window, cx| {
                            w6.update(cx, |this, cx| this.open_game_settings(window, cx)).ok();
                        }),
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
                                this.reveal_selected();
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

        let toast = self.toast.clone().map(|(text, error)| {
            div()
                .absolute()
                .bottom(px(84.))
                .left_0()
                .right_0()
                .flex()
                .justify_center()
                .child(
                    div()
                        .max_w(px(560.))
                        .px(px(16.))
                        .py(px(10.))
                        .rounded(px(10.))
                        .bg(theme::rgb_a(18, 22, 28, 0.94))
                        .border_1()
                        .border_color(if error { theme::danger().into() } else { theme::white_a(0.12) })
                        .text_size(px(13.33))
                        .text_color(theme::white_a(0.92))
                        .child(text),
                )
        });

        div()
            .id("collection")
            .track_focus(&self.focus_handle)
            .on_key_down(cx.listener(|this, ev: &KeyDownEvent, _, cx| {
                if this.shots_key(ev, cx) {
                    cx.notify();
                    return;
                }
                if ev.keystroke.key == "escape" {
                    if this.dialog.is_some() {
                        this.dialog = None;
                        this.dialog_sub = None;
                    } else if this.context_menu.is_some() {
                        this.context_menu = None;
                    } else if this.open_menu.is_some() {
                        this.open_menu = None;
                    } else {
                        // Heronite clears the selected game on Esc.
                        this.focus = None;
                        this.selected = None;
                    }
                    cx.notify();
                }
            }))
            .on_mouse_down(MouseButton::Left, cx.listener(|this, _, window, cx| {
                window.focus(&this.focus_handle, cx);
            }))
            .relative()
            .size_full()
            .overflow_hidden()
            .bg(theme::page_bg())
            .font_family(theme::FONT_BODY)
            .text_color(white())
            .child(stage_el)
            .child(
                div()
                    .absolute()
                    .inset_0()
                    .flex()
                    .flex_col()
                    .child(header)
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
                                    .children(sticky)
                                    .children(list_scrollbar),
                            )
                            .child(div().flex_1().min_w_0().h_full().child(focus_panel)),
                    ),
            )
            .children(toast)
            .children(self.render_context_menu(window, cx))
            .children(self.dialog.clone())
            .children(self.render_gallery(window, cx))
            .children(self.render_viewer(window, cx))
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
