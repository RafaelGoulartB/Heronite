//! Console mode shell (see [`crate::console`]): while it is on, the
//! Collection draws this instead of the desktop screen. The console theme
//! (`crate::console::look`) decides what it draws: the tabs switched with
//! LB / RB, each a layout over a set of games (`layouts/`), the game page
//! (`game/`), the ☰ menu (`sidebar.rs`), the background (`backdrop.rs`),
//! the hint bar; and it routes every controller press:
//! dialog → overlay → Customize → menu → page → game page → tab.
//!
//! The desktop screen is untouched: nothing here runs while console mode is
//! off (the controller thread, the theme and its file only live while it is on).

mod actions;
mod art;
mod backdrop;
mod controls;
mod customize;
mod game;
mod hero;
mod layouts;
mod power;
mod sidebar;
mod sources;
mod tile;

use std::time::{Duration, Instant};

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::console_theme::{AButton, BackgroundSource, Clock, Headings, Hints, Layout, StartOn, TabBar, TabSpec};
use heronite_images::Variant;

use super::CollectionView;
use crate::console::input::{self, PadMsg};
use crate::console::kit::{self, u};
use crate::console::osk::{Osk, OskEvent};
use crate::console::{self, hint, look, motion, Btn, Hint, Overlay, Pad};
use crate::pages::Page;
use crate::theme;
use heronite_i18n::tr;

pub(crate) use backdrop::Scrim;

/// How long the hint bar of `Hints::Auto` stays after a press.
const HINTS_FOR: Duration = Duration::from_millis(3500);

#[derive(Clone, PartialEq, Debug, Default)]
pub(super) enum Screen {
    /// The tabs (LB / RB).
    #[default]
    Tabs,
    Game(SharedString),
}

#[derive(Default)]
pub(super) struct ConsoleUi {
    thread: Option<input::PadThread>,
    pub screen: Screen,
    /// The tab shown (index in the theme's tabs).
    pub tab: usize,
    /// The direction of the last tab switch (-1 / 1), for its slide.
    tab_dir: f32,
    /// Focus of each tab in each layout.
    pub tab_navs: Vec<layouts::TabNav>,
    /// Menu open, with its focused item.
    pub sidebar: Option<usize>,
    /// Keyboard, picker or sheet over the current screen.
    pub overlay: Option<Overlay>,
    overlay_sub: Option<Subscription>,
    game: game::GameNav,
    /// Search typed on the on-screen keyboard (the Grid filters by it).
    pub search: String,
    /// Connected controller name.
    pub controller: Option<String>,
    /// Game whose art fills the background (follows the focus after a short
    /// pause, so scrolling fast does not decode every hero).
    art_key: Option<SharedString>,
    /// Where the art is heading (set at once; `art_key` follows it).
    art_want: Option<SharedString>,
    art_task: Option<Task<()>>,
    /// The art on screen and the one it fades from (dropped once the fade
    /// is over).
    art_cur: Option<(SharedString, std::sync::Arc<RenderImage>)>,
    art_prev: Option<std::sync::Arc<RenderImage>>,
    art_fade: Option<Task<()>>,
    /// Colour of the focused game's cover (the `game` focus colour and the
    /// game-colour background).
    pub game_colour: Option<(u8, u8, u8)>,
    /// The game `game_colour` is sampled for, and whether the sample is in.
    colour_for: Option<SharedString>,
    colour_taken: bool,
    /// `Hints::Auto`: the bar shows until then.
    hints_until: Option<Instant>,
    hints_task: Option<Task<()>>,
    /// The theme seen last frame (tabs are clamped when it changes).
    theme_epoch: u64,
    ticker: Option<Task<()>>,
    script_started: bool,
    /// The games and shelves of the sources in use (see [`sources::SourceCache`]).
    sources: std::cell::RefCell<sources::SourceCache>,
    logos: art::LogoCache,
    controls: controls::ControlsState,
    customize: Option<customize::CustomizeNav>,
}

/// A tab's name as shown (built-in themes name their tabs with English keys).
fn tab_label(t: &TabSpec) -> SharedString {
    SharedString::from(tr(&t.name).to_string())
}

impl CollectionView {
    /// At start: reopens in console mode when it was left on
    /// (`HERONITE_DEBUG_CONSOLE=1` forces it, without saving).
    pub(super) fn console_init(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // `HERONITE_DEBUG_CONSOLE=1`, `=game:<key>` to open that game's page,
        // `=menu` for the menu, `=customize` for the Customize panel,
        // `=power` for the Power menu or `=power:reboot|poweroff` for its question.
        let debug = crate::env::value("HERONITE_DEBUG_CONSOLE");
        self.console_startup(window, cx);
        if (self.ui.console_mode || debug.is_some()) && !console::active() {
            self.enter_console(window, cx);
        }
        if let Some(key) = debug.as_deref().and_then(|d| d.strip_prefix("game:")) {
            self.open_game_page(key.to_string().into());
        }
        if debug.as_deref() == Some("menu") {
            self.open_sidebar();
        }
        if debug.as_deref() == Some("customize") {
            self.open_console_customize();
        }
        if let Some(which) = debug.as_deref().and_then(|d| d.strip_prefix("power")) {
            self.debug_power(which, cx);
        }
    }

    pub(super) fn set_console(&mut self, on: bool, window: &mut Window, cx: &mut Context<Self>) {
        if on == console::active() {
            return;
        }
        self.ui.console_mode = on;
        self.save_ui();
        if on {
            self.mark_onboarding(crate::dialogs::onboarding::Step::Console);
            self.enter_console(window, cx);
        } else {
            console::set_active(false);
            look::clear();
            console::themes::forget();
            self.console = ConsoleUi::default();
            // Textures only console mode draws: logos and wide covers.
            self.images.forget_prefix("l:");
            self.images.forget_prefix("cw:");
            self.images.forget_matching(|id| backdrop::is_backdrop_id(id) || id.starts_with("a:"));
            self.console_controls_ended(window, cx);
            self.close_console_dialog();
            self.page = Page::Collection;
            cx.notify();
        }
    }

    /// Closes the dialog over the console and its controller hook.
    fn close_console_dialog(&mut self) {
        self.dialog = None;
        self.dialog_sub = None;
        self.dialog_hook = None;
    }

    fn enter_console(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // The theme first: everything drawn from here on reads it.
        let mut prefs = console::themes::prefs(&self.app_paths);
        // `HERONITE_DEBUG_CONSOLE_PRESET=<theme id>` / `HERONITE_DEBUG_CONSOLE_LAYOUT=<layout>`:
        // start with that theme / every tab in that layout (not saved).
        if let Some(t) = crate::env::value("HERONITE_DEBUG_CONSOLE_PRESET")
            .and_then(|id| heronite_data::console_theme::store::find(&self.app_paths, &id))
        {
            prefs.theme = t;
        }
        if let Some(l) = crate::env::value("HERONITE_DEBUG_CONSOLE_LAYOUT").and_then(|l| Layout::parse(&l)) {
            prefs.theme.tabs.iter_mut().for_each(|t| t.layout = l);
        }
        look::set(prefs.theme.clone());
        console::set_active(true);
        self.open_menu = None;
        self.context_menu = None;
        self.close_console_dialog();
        self.close_palette(window, cx);
        self.page = Page::Collection;
        self.console = ConsoleUi::default();
        if prefs.theme.nav.start_on == StartOn::Last {
            self.console.tab = prefs.last_tab.min(prefs.theme.tabs.len().saturating_sub(1));
        }
        self.console.theme_epoch = look::epoch();
        if let Some((thread, rx)) = input::start() {
            self.console.thread = Some(thread);
            Self::spawn_pad_pump(rx, window, cx);
        }
        // The clock in the top bar (nothing to redraw while the theme hides it).
        self.console.ticker = Some(cx.spawn(async move |this, cx| loop {
            cx.background_executor().timer(Duration::from_secs(20)).await;
            let shown = this.update(cx, |_, cx| {
                if look::theme().interface.clock != Clock::Off {
                    cx.notify();
                }
            });
            if shown.is_err() {
                break;
            }
        }));
        self.show_console_hints(cx);
        self.console_controls_started(window, cx);
        cx.notify();
    }

    fn spawn_pad_pump(mut rx: futures::channel::mpsc::UnboundedReceiver<PadMsg>, window: &mut Window, cx: &mut Context<Self>) {
        use futures::StreamExt;
        cx.spawn_in(window, async move |this, cx| {
            while let Some(msg) = rx.next().await {
                let alive = this
                    .update_in(cx, |this, window, cx| {
                        if !console::active() {
                            return;
                        }
                        match msg {
                            // Presses only count while the window has the
                            // focus (not while a game is in front).
                            PadMsg::Press(pad) if window.is_window_active() => this.console_pad(pad, window, cx),
                            PadMsg::Press(_) => {}
                            PadMsg::Connected(name, plugged) => {
                                if plugged {
                                    this.show_toast(tr!("Controller connected: {name}", name), false, cx);
                                }
                                look::set_controller(Some(&name));
                                this.console.controller = Some(name);
                            }
                            PadMsg::Disconnected => {
                                look::set_controller(None);
                                this.console.controller = None;
                                this.show_toast(tr!("Controller disconnected"), true, cx);
                            }
                        }
                    })
                    .is_ok();
                if !alive {
                    break;
                }
            }
        })
        .detach();
    }

    /// `HERONITE_DEBUG_PAD=down,a,...`: scripted presses once the library is
    /// on screen (tests without a controller).
    fn console_script(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.console.script_started || self.lib.is_none() {
            return;
        }
        self.console.script_started = true;
        let Some(text) = crate::env::value("HERONITE_DEBUG_PAD") else { return };
        let steps = input::parse_script(&text);
        cx.spawn_in(window, async move |this, cx| {
            cx.background_executor().timer(Duration::from_millis(1200)).await;
            for (wait, pad) in steps {
                cx.background_executor().timer(wait).await;
                if this.update_in(cx, |this, window, cx| this.console_pad(pad, window, cx)).is_err() {
                    break;
                }
            }
        })
        .detach();
    }

    /// A key in console mode (called before anything else sees it).
    pub(super) fn console_key(&mut self, ks: &Keystroke, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if let Some(o) = self.console.overlay.clone() {
            if let Ok(osk) = o.view.downcast::<crate::console::osk::Osk>() {
                if osk.update(cx, |k, cx| k.type_key(ks, cx)) {
                    return true;
                }
            }
        }
        match input::pad_from_key(ks) {
            Some(pad) => {
                self.console_pad(pad, window, cx);
                true
            }
            // Ctrl K, Ctrl , ... stay desktop shortcuts: swallow them.
            None => ks.modifiers.control,
        }
    }

    /// Hosts `view` over the current screen until it closes itself through
    /// `close_console_overlay`.
    pub(super) fn open_console_overlay<V: console::PadTarget + Render>(
        &mut self,
        view: &Entity<V>,
        sub: Subscription,
        cx: &mut Context<Self>,
    ) {
        self.console.overlay = Some(Overlay::new(view));
        self.console.overlay_sub = Some(sub);
        cx.notify();
    }

    pub(super) fn close_console_overlay(&mut self, cx: &mut Context<Self>) {
        self.console.overlay = None;
        self.console.overlay_sub = None;
        cx.notify();
    }

    /// The theme's tab `i` (clamped).
    pub(super) fn console_tab_spec(&self) -> (usize, TabSpec) {
        let theme = look::theme();
        let i = self.console.tab.min(theme.tabs.len().saturating_sub(1));
        (i, theme.tabs.get(i).cloned().unwrap_or_default())
    }

    fn switch_tab(&mut self, delta: isize) {
        let n = look::theme().tabs.len();
        if n < 2 {
            return;
        }
        self.console.tab = layouts::step(self.console.tab.min(n - 1), n, delta, true);
        self.console.tab_dir = delta.signum() as f32;
        let tab = self.console.tab;
        if look::theme().nav.start_on == StartOn::Last {
            console::themes::remember_tab(&self.app_paths, tab);
        }
    }

    /// Routes one press.
    pub(super) fn console_pad(&mut self, pad: Pad, window: &mut Window, cx: &mut Context<Self>) {
        cx.notify();
        // A press that only woke the idle screen goes no further.
        if self.console_controls_press(pad, cx) {
            return;
        }
        if !matches!(pad, Pad::ScrollUp | Pad::ScrollDown) {
            motion::press();
        }
        self.show_console_hints(cx);
        if self.dialog.is_some() {
            let used = self.dialog_hook.clone().is_some_and(|h| h.pad(pad, window, cx));
            if !used && pad == Pad::B {
                self.close_console_dialog();
            }
            return;
        }
        if let Some(o) = self.console.overlay.clone() {
            if !o.hook.pad(pad, window, cx) && pad == Pad::B {
                self.close_console_overlay(cx);
            }
            return;
        }
        if self.console.customize.is_some() {
            if !self.customize_pad(pad, window, cx) && matches!(pad, Pad::B | Pad::Start) {
                self.console.customize = None;
            }
            return;
        }
        if self.console.sidebar.is_some() {
            self.sidebar_pad(pad, window, cx);
            return;
        }
        if pad == Pad::Start {
            // An overlay of the page (the keyboard's Done) gets it first.
            if self.page != Page::Collection && self.pages.hook(self.page).is_some_and(|h| h.pad(pad, window, cx)) {
                return;
            }
            self.open_sidebar();
            return;
        }
        if self.page != Page::Collection {
            let used = self.pages.hook(self.page).is_some_and(|h| h.pad(pad, window, cx));
            if !used && pad == Pad::B {
                self.navigate(Page::Collection, window, cx);
            }
            return;
        }
        if let Screen::Game(key) = self.console.screen.clone() {
            if !self.game_pad(&key, pad, window, cx) && pad == Pad::B {
                self.close_game_page();
            }
            return;
        }
        let (tab, spec) = self.console_tab_spec();
        if !self.layout_pad(tab, &spec, pad, window, cx) {
            match pad {
                Pad::LB => self.switch_tab(-1),
                Pad::RB => self.switch_tab(1),
                Pad::B => self.open_sidebar(),
                _ => {}
            }
        }
    }

    /// `Hints::Auto`: shows the bar now and hides it a moment after the last press.
    fn show_console_hints(&mut self, cx: &mut Context<Self>) {
        if look::theme().controls.hints != Hints::Auto {
            return;
        }
        self.console.hints_until = Some(Instant::now() + HINTS_FOR);
        self.console.hints_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(HINTS_FOR + Duration::from_millis(50)).await;
            this.update(cx, |_, cx| cx.notify()).ok();
        }));
    }

    fn hints_visible(&self) -> bool {
        match look::theme().controls.hints {
            Hints::Always => true,
            Hints::Off => false,
            Hints::Auto => self.console.hints_until.is_some_and(|t| Instant::now() < t),
        }
    }

    fn open_sidebar(&mut self) {
        self.console.sidebar = Some(self.sidebar_current());
    }

    pub(super) fn open_game_page(&mut self, key: SharedString) {
        self.console.game = game::GameNav::default();
        self.console.screen = Screen::Game(key);
    }

    fn close_game_page(&mut self) {
        self.console.screen = Screen::Tabs;
    }

    /// A press on a cover: A opens its page or plays it, X the other (the
    /// theme's `nav.aButton`).
    pub(super) fn console_activate(&mut self, key: SharedString, pad: Pad, _window: &mut Window, cx: &mut Context<Self>) {
        let details = (pad == Pad::A) == (look::theme().nav.a_button == AButton::Details);
        if details {
            self.open_game_page(key);
        } else {
            self.primary_action(&key, cx);
        }
    }

    /// The on-screen keyboard for the search; then the first Grid tab shows the results.
    pub(super) fn console_search(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let current = self.console.search.clone();
        let osk = cx.new(|_| Osk::new(tr!("Search games"), &current));
        let sub = cx.subscribe(&osk, |this, _, ev: &OskEvent, cx| {
            if let OskEvent::Done(text) = ev {
                this.console.search = text.trim().to_string();
                let theme = look::theme();
                if let Some(i) = theme.tabs.iter().position(|t| t.layout == Layout::Grid) {
                    this.console.tab = i;
                    let nav = &mut this.tab_nav(i).grid;
                    nav.index = 0;
                    nav.toolbar = None;
                    nav.restart = true;
                }
            }
            this.close_console_overlay(cx);
        });
        self.open_console_overlay(&osk, sub, cx);
    }

    /// LB / RB (when there are tabs) and Menu.
    pub(super) fn console_tab_hints(&self) -> Vec<Hint> {
        let theme = look::theme();
        let mut h = Vec::new();
        if theme.tabs.len() > 1 {
            let next = &theme.tabs[(self.console.tab + 1) % theme.tabs.len()];
            h.push(hint(Btn::Bumpers, if theme.tabs.len() == 2 { tab_label(next).to_string() } else { tr!("Tabs").to_string() }));
        }
        h.push(hint(Btn::Start, tr!("Menu")));
        h
    }

    /// A / X on a cover (as the theme's `nav.aButton` says), Y search, tabs, menu.
    pub(super) fn console_cover_hints(&self) -> Vec<Hint> {
        let primary = self.selected.as_ref().and_then(|k| self.lib.as_ref()?.get(k)).map(|g| self.primary_label(g)).unwrap_or(tr!("Play"));
        let mut h = if look::theme().nav.a_button == AButton::Details {
            vec![hint(Btn::A, tr!("Details")), hint(Btn::X, primary)]
        } else {
            vec![hint(Btn::A, primary), hint(Btn::X, tr!("Details"))]
        };
        h.push(hint(Btn::Y, tr!("Search")));
        h.extend(self.console_tab_hints());
        h
    }

    /// Hints of whatever takes the presses now.
    fn console_hints(&self, cx: &App) -> Vec<Hint> {
        if self.dialog.is_some() {
            return self.dialog_hook.as_ref().map(|h| h.hints(cx)).unwrap_or_else(|| vec![hint(Btn::B, tr!("Close"))]);
        }
        if let Some(o) = &self.console.overlay {
            return o.hook.hints(cx);
        }
        if self.console.customize.is_some() {
            return self.customize_hints();
        }
        if self.console.sidebar.is_some() {
            return vec![hint(Btn::A, tr!("Select")), hint(Btn::B, tr!("Back"))];
        }
        if self.page != Page::Collection {
            let mut h = self.pages.hook(self.page).map(|h| h.hints(cx)).unwrap_or_default();
            if !h.iter().any(|h| h.btn == Btn::B) {
                h.push(hint(Btn::B, tr!("Back")));
            }
            h.push(hint(Btn::Start, tr!("Menu")));
            return h;
        }
        match &self.console.screen {
            Screen::Game(key) => self.game_hints(key),
            Screen::Tabs => {
                let (tab, spec) = self.console_tab_spec();
                self.layout_hints(tab, &spec)
            }
        }
    }

    /// Moves the background art to `key` after a short pause.
    pub(super) fn console_focus_game(&mut self, key: Option<SharedString>, cx: &mut Context<Self>) {
        if key == self.console.art_want {
            return;
        }
        self.console.art_want = key.clone();
        // Keep the desktop selection in step (stage, glass, focus data).
        match &key {
            Some(k) if Some(k) != self.selected.as_ref() => self.select(k.clone(), false),
            Some(_) => {}
            None => {
                self.selected = None;
                self.focus = None;
            }
        }
        let wanted = key.clone();
        self.console.art_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_millis(220)).await;
            this.update(cx, |this, cx| {
                this.console.art_key = wanted;
                cx.notify();
            })
            .ok();
        }));
    }

    /// The colour of the focused cover, sampled only when the theme uses it:
    /// once per game, when its small cover arrives.
    fn sync_console_colour(&mut self) {
        let wanted = look::wants_game_accent() || look::theme().background.source == BackgroundSource::GameColour;
        let Some(key) = self.console.art_want.clone().filter(|_| wanted) else {
            if self.console.game_colour.take().is_some() {
                look::set_game_accent(None);
            }
            self.console.colour_for = None;
            return;
        };
        let (w, h) = super::look::ACCENT_SAMPLE;
        if self.console.colour_for.as_ref() == Some(&key) {
            // Sampled already, or still waiting for the cover.
            if self.console.colour_taken {
                return;
            }
        } else {
            self.console.colour_for = Some(key.clone());
            self.console.colour_taken = false;
        }
        let id: SharedString = format!("a:{key}").into();
        let chains = &self.cover_chains;
        let img = self.images.get_or_request(&id, || chains.get(&key).cloned().unwrap_or_default(), w, h, Variant::Cover, 200);
        // Keep the last colour while the next one loads.
        if let Some(rgb) = img.as_ref().and_then(|img| img.as_bytes(0)).and_then(super::look::accent_from_bgra) {
            self.console.colour_taken = true;
            self.console.game_colour = Some(rgb);
            look::set_game_accent(Some(rgb));
        }
    }

    /// Top bar: the theme's label (nothing unless it has one), then what
    /// `tab_bar` asks for: the tabs (`Top`), their names only (`Names`), only
    /// the current one's name, centred (`Title`) or nothing (`Bottom`: the
    /// tabs go along the bottom, see `console_bottom_tabs`); status icons,
    /// clock.
    fn console_top_bar(&self) -> Div {
        let theme = look::theme();
        let names: Vec<SharedString> = theme.tabs.iter().map(tab_label).collect();
        let label = theme.interface.label.trim().to_string();
        let tab = self.console.tab.min(names.len().saturating_sub(1));
        let tab_bar = theme.interface.tab_bar;
        div()
            .relative()
            .flex()
            .flex_none()
            .items_center()
            .gap(u(36.))
            .h(u(76.))
            .px(u(46.))
            .when(!label.is_empty(), |d| d.child(kit::wordmark(&label)))
            .when(names.len() > 1 && tab_bar == TabBar::Top, |d| d.child(kit::tabs(&names, tab)))
            .when(names.len() > 1 && tab_bar == TabBar::Names, |d| d.child(kit::tab_names(&names, tab)))
            .when_some(theme.tabs.get(tab).filter(|_| tab_bar == TabBar::Title), |d, t| {
                let spaced = theme.interface.headings == Headings::Spaced;
                let title = kit::tab_title(&tab_label(t), kit::tab_icon(&t.source, t.layout), names.len() > 1, spaced);
                d.child(div().absolute().top_0().bottom_0().left_0().right_0().flex().items_center().justify_center().child(title))
            })
            .child(div().flex_1())
            .when(theme.interface.status_icons, |d| {
                d.when(self.console.controller.is_some(), |d| {
                    d.child(crate::ui::icon(crate::icons::Icon::SportsEsports, u(20.), theme::text_muted()))
                })
            })
            .child(kit::clock())
    }

    /// The tabs along the bottom (`tab_bar: bottom`), when there are several.
    fn console_bottom_tabs(&self) -> Option<Div> {
        let theme = look::theme();
        (theme.interface.tab_bar == TabBar::Bottom && theme.tabs.len() > 1).then(|| {
            let items: Vec<(SharedString, crate::icons::Icon)> =
                theme.tabs.iter().map(|t| (tab_label(t), kit::tab_icon(&t.source, t.layout))).collect();
            kit::tabs_bottom(&items, self.console.tab.min(items.len() - 1))
        })
    }

    pub(super) fn render_console(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        kit::set_scale(window);
        self.console_script(window, cx);
        let theme = look::theme();
        if self.console.theme_epoch != look::epoch() {
            self.console.theme_epoch = look::epoch();
            self.console.tab = self.console.tab.min(theme.tabs.len().saturating_sub(1));
        }
        let (tab, spec) = self.console_tab_spec();
        let on_page = self.page != Page::Collection;
        let game_key = match &self.console.screen {
            Screen::Game(k) if !on_page => Some(k.clone()),
            _ => None,
        };
        let body: AnyElement = if on_page {
            match self.render_page_view() {
                // Pages rise in.
                Some(view) => motion::enter(
                    SharedString::from(format!("console-page-{}", self.page.title())),
                    div().relative().size_full().child(view),
                    300,
                    0.,
                    22.,
                ),
                None => div().into_any_element(),
            }
        } else if let Some(key) = &game_key {
            self.render_console_game(key, window, cx)
        } else {
            let dir = std::mem::take(&mut self.console.tab_dir);
            let layout = self.render_layout(tab, &spec, window, cx);
            motion::enter(
                SharedString::from(format!("console-tab-{tab}-{}-{}", spec.layout.as_str(), look::epoch())),
                div().relative().size_full().child(layout),
                280,
                36. * dir,
                0.,
            )
        };
        self.sync_console_colour();
        let keeps_tabs = !on_page && (game_key.is_none() || self.game_page_keeps_tabs());
        let top_bar = keeps_tabs.then(|| self.console_top_bar());
        let bottom_tabs = if keeps_tabs { self.console_bottom_tabs() } else { None };
        let scrim = if on_page {
            Scrim::Page
        } else if game_key.is_some() {
            self.game_scrim(&spec)
        } else {
            layouts::scrim(spec.layout)
        };
        let backdrop = self.render_console_backdrop(scrim, window, cx);
        let hints_on = self.hints_visible();
        let hints = if hints_on { self.console_hints(cx) } else { Vec::new() };
        let idle = self.render_idle(window, cx);
        let customize = self.render_console_customize();
        let (_, body_font) = look::fonts();
        div()
            .id("console")
            .relative()
            .size_full()
            .overflow_hidden()
            .bg(theme::console_bg())
            .font_family(body_font)
            .text_color(theme::text_primary())
            .child(backdrop)
            .child(
                div()
                    .absolute()
                    .inset_0()
                    // The Rail of the menu keeps its column.
                    .when_some(sidebar::rail_inset(), |d, (side, w)| {
                        if side == heronite_data::console_theme::Side::Right {
                            d.right(w)
                        } else {
                            d.left(w)
                        }
                    })
                    .flex()
                    .flex_col()
                    .children(top_bar)
                    .child(div().flex_1().min_h_0().child(body))
                    .children(bottom_tabs)
                    .when(hints_on, |d| d.child(div().flex_none().h(u(60.)))),
            )
            .children(self.render_sidebar(cx))
            .children(customize)
            .children(self.console.overlay.as_ref().map(|o| o.view.clone()))
            .children(self.dialog.clone().map(|d| {
                let id = SharedString::from(format!("console-dialog-{}", d.entity_id()));
                motion::layer_in(id, d, 240, 12.)
            }))
            .children(self.render_toast(cx))
            // Above the overlays: the hints of whatever has the focus.
            .when(hints_on, |d| {
                d.child(div().absolute().left_0().right_0().bottom_0().child(kit::hint_bar(&hints).bg(theme::console_bg())))
            })
            .children(idle)
            .into_any_element()
    }
}
