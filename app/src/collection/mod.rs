//! The Collection screen: stage background, header, grouped list, focus panel.

mod achievements;
mod actions;
mod add_game;
mod backdrop;
mod card;
mod console;
mod context_menu;
mod data;
mod debug;
mod emulation;
mod epic;
mod extras;
mod focus;
mod focus_left;
mod game_facts;
mod header;
mod html;
mod list;
mod list_view;
mod load;
mod metadata_ui;
mod nav;
mod palette;
mod pumps;
mod render_focus;
mod screenshot_viewer;
mod screenshots;
mod scroll;
mod timeline;
mod system_glue;
pub use screenshots::screenshots_global;
pub mod stage;
mod vm;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use gpui_kit::component::input::InputState;
use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::hltb::WikiCache;
use heronite_data::ui_state::UiState;
use heronite_data::{AppPaths, FacetKind, HeroicPaths, Library};
use heronite_images::ImageRef;

use crate::images::ImageStore;
use crate::runtime::GameRuntime;
use crate::theme;
use header::Menu;
use scroll::WheelScroll;
use vm::{FocusVm, GroupVm, ListItem};

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
    /// Pending hover-intent open of a header menu.
    menu_open: Option<Task<()>>,
    menu_close: Option<Task<()>>,
    /// Hovered parts of the open header menu (`ui::dropdown::PART_*`).
    menu_parts: u8,
    /// Smooth mouse-wheel scrolling of the game list.
    wheel: WheelScroll,
    facet_flyout: Option<FacetKind>,
    refreshing: bool,
    runtime: Arc<GameRuntime>,
    /// app_name -> started tracking (Launching/Running).
    playing: HashMap<String, bool>,
    toast: Option<(SharedString, bool)>,
    context_menu: Option<context_menu::ContextMenuState>,
    focus_handle: FocusHandle,
    dialog: Option<AnyView>,
    /// Controller hook of the open dialog (console mode).
    dialog_hook: Option<crate::console::PadHook>,
    /// `HERONITE_DEBUG_DIALOG` (opened on the first render with a window).
    pending_debug_dialog: Option<String>,
    dialog_sub: Option<Subscription>,
    epic: epic::EpicState,
    extras: extras::Extras,
    /// Rating, save backup, Linux facts and timeline of the focused game.
    facts: game_facts::Facts,
    focus_scroll: ScrollHandle,
    shots: screenshots::Shots,
    toast_task: Option<Task<()>>,
    images: ImageStore,
    cover_chains: HashMap<SharedString, Vec<ImageRef>>,
    stage_chains: HashMap<SharedString, Vec<ImageRef>>,
    /// Last stage texture shown, kept while the next one loads.
    shown_stage: Option<(SharedString, Arc<RenderImage>)>,
    /// Last glass backdrop (blurred art behind menus and dialogs), kept while
    /// the next one loads.
    shown_glass: Option<(SharedString, Arc<RenderImage>)>,
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
    /// Page shown (☰ menu); the Collection itself is `Page::Collection`.
    page: crate::pages::Page,
    pages: nav::Pages,
    /// Command palette (Ctrl K), see `palette/mod.rs`.
    palette: palette::Host,
    /// Console mode state (`console/`).
    console: console::ConsoleUi,
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
        let app_paths = AppPaths::for_heroic(&heroic);
        let ui = load::load_ui_state(&heroic, &app_paths);
        // Interface / Collection-art appearance (this app's own file, tiny).
        crate::theme::set_appearance(heronite_data::appearance::Appearance::load(&app_paths));
        let (images, image_rx) = ImageStore::new(heroic.images_cache_dir(), &app_paths);
        Self::spawn_image_pump(image_rx, cx);

        let (runtime, events) = crate::system::attach_runtime(&heroic, cx);
        Self::spawn_launch_events(events, cx);

        // Results of the post-game work (toasts, saved captures).
        if let Some(session) = crate::services::session::get(cx) {
            cx.subscribe(&session, |this, _, ev: &crate::services::session::SessionEvent, cx| this.on_session_event(ev, cx)).detach();
        }

        let search_input = Self::new_search_input(window, cx);
        let mut this = Self {
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
            menu_open: None,
            menu_close: None,
            menu_parts: 0,
            wheel: WheelScroll::default(),
            facet_flyout: None,
            refreshing: false,
            runtime,
            playing: HashMap::new(),
            toast: None,
            toast_task: None,
            context_menu: None,
            focus_handle: cx.focus_handle(),
            dialog: None,
            dialog_hook: None,
            pending_debug_dialog: None,
            dialog_sub: None,
            epic: Default::default(),
            extras: Default::default(),
            facts: Default::default(),
            focus_scroll: ScrollHandle::new(),
            shots: Default::default(),
            images,
            cover_chains: HashMap::new(),
            stage_chains: HashMap::new(),
            shown_stage: None,
            shown_glass: None,
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
            page: crate::pages::Page::Collection,
            pages: nav::Pages::default(),
            palette: palette::Host::new(window, cx),
            console: Default::default(),
        };
        window.focus(&this.focus_handle, cx);
        this.console_init(window, cx);
        if let Some(page) = crate::env::value("HERONITE_DEBUG_PAGE").and_then(|p| crate::pages::Page::from_debug(&p)) {
            this.navigate(page, window, cx);
        }
        this.spawn_load(cx);
        this.start_idle_jobs(cx);
        this.init_metadata(window, cx);
        this.spawn_disk_watch(cx);
        this.spawn_debug_browse(cx);
        this
    }
}

impl CollectionView {
    fn render_toast(&self) -> Option<Div> {
        self.toast.clone().map(|(text, error)| {
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
        })
    }

    fn on_key_down(&mut self, ev: &KeyDownEvent, cx: &mut Context<Self>) {
        if self.shots_key(ev, cx) {
            cx.notify();
            return;
        }
        if ev.keystroke.key == "escape" {
            if self.dialog.is_some() {
                self.dialog = None;
                self.dialog_sub = None;
            } else if self.context_menu.is_some() {
                self.context_menu = None;
            } else if self.open_menu.is_some() {
                self.open_menu = None;
            } else {
                // Heronite clears the selected game on Esc.
                self.focus = None;
                self.selected = None;
            }
            cx.notify();
        }
    }
}

impl Render for CollectionView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.render_desktop(window, cx)
    }
}

impl CollectionView {
    fn render_desktop(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let _timer = RenderTimer(Instant::now());
        if !self.first_frame_logged {
            self.first_frame_logged = true;
            log::info!("first render after {:?}", self.started.elapsed());
        }
        self.images.drop_evicted(window);
        self.open_pending_debug_dialog(window, cx);
        self.drive_epic(window, cx);
        self.sync_extras(window, cx);
        self.sync_facts(cx);
        self.sync_shots(window, cx);
        self.drive_palette(window, cx);
        crate::ui::glass::set_backdrop(self.glass_image(window));
        self.sync_nav_meta(cx);
        if crate::console::active() {
            return self.render_console(window, cx);
        }
        let vw = f32::from(window.viewport_size().width);
        let scale = window.scale_factor();
        let (list_w, columns) = Self::list_column_width(vw);
        if columns != self.columns {
            self.columns = columns;
            self.rebuild_items();
        }

        let header = self.render_header(list_w, window, cx);
        let page_el = self.render_page(cx);
        let on_page = page_el.is_some();
        let stage_el = match self.stage_image(window) {
            Some((key, src)) => stage::stage(Some(src), key, self.scrim, crate::theme::appearance().collection_opacity / 100.),
            None => stage::stage(None, SharedString::default(), self.scrim, 1.),
        };
        let list_column = self.render_list_column(list_w, cx);
        let vh = f32::from(window.viewport_size().height);
        let focus_panel = self.render_focus_panel(vw, vh, list_w, scale, cx);
        let toast = self.render_toast();

        div()
            .id("collection")
            .track_focus(&self.focus_handle)
            .on_key_down(cx.listener(|this, ev: &KeyDownEvent, _, cx| this.on_key_down(ev, cx)))
            // Capture phase: runs before children, so a clicked input (search,
            // dialog fields) takes focus right after; clicks elsewhere blur it.
            .capture_any_mouse_down(cx.listener(|this, _, window, cx| {
                window.focus(&this.focus_handle, cx);
            }))
            .relative()
            .size_full()
            .overflow_hidden()
            .bg(theme::page_bg())
            .font_family(theme::FONT_BODY)
            .text_color(white())
            .child(stage_el)
            .children(page_el)
            .when(!on_page, |root| {
                root.child(
                    div().absolute().inset_0().flex().flex_col().child(header).child(
                        div()
                            .flex()
                            .flex_1()
                            .min_h_0()
                            .child(list_column)
                            .child(div().flex_1().min_w_0().h_full().child(focus_panel)),
                    ),
                )
            })
            .children(toast)
            .children(self.render_context_menu(window, cx))
            .children(self.dialog.clone())
            .children(self.render_gallery(window, cx))
            .children(self.render_viewer(window, cx))
            .children(self.render_palette())
            .into_any_element()
    }
}

/// Logs how long `CollectionView::render` took (RUST_LOG=debug).
struct RenderTimer(Instant);

impl Drop for RenderTimer {
    fn drop(&mut self) {
        log::debug!("render {:?}", self.0.elapsed());
    }
}
