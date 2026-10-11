//! Console mode shell (see [`crate::console`]): while it is on, the
//! Collection draws this instead of the desktop screen — the library
//! redesigned for a TV (`home.rs`: Home shelves and the Library grid), the
//! game page (`game.rs`), the full-height menu (`sidebar.rs`) and the hint
//! bar — and routes every controller press:
//! dialog → overlay → menu → page → library screens.
//!
//! The desktop screen is untouched: nothing here runs while console mode is
//! off (the controller thread only lives while it is on).

mod actions;
mod game;
mod home;
mod sidebar;

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_images::Variant;

use super::CollectionView;
use crate::console::input::{self, PadMsg};
use crate::console::kit::{self, u};
use crate::console::motion;
use crate::console::{self, hint, Btn, Hint, Overlay, Pad};
use crate::pages::Page;
use crate::theme;

/// Cross-fade of the background art.
const ART_FADE_MS: u64 = 520;

#[derive(Clone, PartialEq, Debug, Default)]
pub(super) enum Screen {
    #[default]
    Home,
    Library,
    Game(SharedString),
}

#[derive(Default)]
pub(super) struct ConsoleUi {
    thread: Option<input::PadThread>,
    pub screen: Screen,
    /// Where B on the game page goes back to.
    back_to: Screen,
    /// Menu open, with its focused item.
    pub sidebar: Option<usize>,
    /// Keyboard, picker or sheet over the current screen.
    pub overlay: Option<Overlay>,
    overlay_sub: Option<Subscription>,
    home: home::HomeNav,
    library: home::LibraryNav,
    game: game::GameNav,
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
    ticker: Option<Task<()>>,
    script_started: bool,
}

impl CollectionView {
    /// At start: reopens in console mode when it was left on
    /// (`HERONITE_DEBUG_CONSOLE=1` forces it, without saving).
    pub(super) fn console_init(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // `HERONITE_DEBUG_CONSOLE=1`, or `=game:<key>` to open that game's page.
        let debug = crate::env::value("HERONITE_DEBUG_CONSOLE");
        if self.ui.console_mode || debug.is_some() {
            self.enter_console(window, cx);
        }
        if let Some(key) = debug.as_deref().and_then(|d| d.strip_prefix("game:")) {
            self.open_game_page(key.to_string().into());
        }
    }

    pub(super) fn set_console(&mut self, on: bool, window: &mut Window, cx: &mut Context<Self>) {
        if on == console::active() {
            return;
        }
        self.ui.console_mode = on;
        self.save_ui();
        if on {
            self.enter_console(window, cx);
        } else {
            console::set_active(false);
            self.console = ConsoleUi::default();
            self.dialog = None;
            self.dialog_sub = None;
            self.dialog_hook = None;
            self.page = Page::Collection;
            cx.notify();
        }
    }

    fn enter_console(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        console::set_active(true);
        self.open_menu = None;
        self.context_menu = None;
        self.dialog = None;
        self.dialog_sub = None;
        self.dialog_hook = None;
        self.close_palette(window, cx);
        self.page = Page::Collection;
        self.console = ConsoleUi::default();
        if let Some((thread, rx)) = input::start() {
            self.console.thread = Some(thread);
            Self::spawn_pad_pump(rx, window, cx);
        }
        // The clock in the top bar.
        self.console.ticker = Some(cx.spawn(async move |this, cx| loop {
            cx.background_executor().timer(std::time::Duration::from_secs(20)).await;
            if this.update(cx, |_, cx| cx.notify()).is_err() {
                break;
            }
        }));
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
                                    this.show_toast(format!("Controller connected: {name}"), false, cx);
                                }
                                this.console.controller = Some(name);
                            }
                            PadMsg::Disconnected => {
                                this.console.controller = None;
                                this.show_toast("Controller disconnected", true, cx);
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
            cx.background_executor().timer(std::time::Duration::from_millis(1200)).await;
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

    /// Routes one press.
    pub(super) fn console_pad(&mut self, pad: Pad, window: &mut Window, cx: &mut Context<Self>) {
        cx.notify();
        if !matches!(pad, Pad::ScrollUp | Pad::ScrollDown) {
            motion::press();
        }
        if self.dialog.is_some() {
            let used = self.dialog_hook.clone().is_some_and(|h| h.pad(pad, window, cx));
            if !used && pad == Pad::B {
                self.dialog = None;
                self.dialog_sub = None;
                self.dialog_hook = None;
            }
            return;
        }
        if let Some(o) = self.console.overlay.clone() {
            if !o.hook.pad(pad, window, cx) && pad == Pad::B {
                self.close_console_overlay(cx);
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
        let used = match self.console.screen.clone() {
            Screen::Home => self.home_pad(pad, window, cx),
            Screen::Library => self.library_pad(pad, window, cx),
            Screen::Game(key) => self.game_pad(&key, pad, window, cx),
        };
        if !used {
            match pad {
                Pad::LB | Pad::RB if !matches!(self.console.screen, Screen::Game(_)) => {
                    self.console.screen = if self.console.screen == Screen::Home { Screen::Library } else { Screen::Home };
                }
                Pad::B => match self.console.screen.clone() {
                    Screen::Game(_) => self.close_game_page(),
                    _ => self.open_sidebar(),
                },
                _ => {}
            }
        }
    }

    fn open_sidebar(&mut self) {
        let current = sidebar::ITEMS.iter().position(|i| i.page() == Some(self.page)).unwrap_or(0);
        self.console.sidebar = Some(current);
    }

    pub(super) fn open_game_page(&mut self, key: SharedString) {
        if !matches!(self.console.screen, Screen::Game(_)) {
            self.console.back_to = self.console.screen.clone();
        }
        self.console.game = game::GameNav::default();
        self.console.screen = Screen::Game(key);
    }

    fn close_game_page(&mut self) {
        self.console.screen = std::mem::take(&mut self.console.back_to);
    }

    /// Hints of whatever takes the presses now.
    fn console_hints(&self, cx: &App) -> Vec<Hint> {
        if self.dialog.is_some() {
            return self.dialog_hook.as_ref().map(|h| h.hints(cx)).unwrap_or_else(|| vec![hint(Btn::B, "Close")]);
        }
        if let Some(o) = &self.console.overlay {
            return o.hook.hints(cx);
        }
        if self.console.sidebar.is_some() {
            return vec![hint(Btn::A, "Select"), hint(Btn::B, "Back")];
        }
        if self.page != Page::Collection {
            let mut h = self.pages.hook(self.page).map(|h| h.hints(cx)).unwrap_or_default();
            if !h.iter().any(|h| h.btn == Btn::B) {
                h.push(hint(Btn::B, "Back"));
            }
            h.push(hint(Btn::Start, "Menu"));
            return h;
        }
        match &self.console.screen {
            Screen::Home => self.home_hints(),
            Screen::Library => self.library_hints(),
            Screen::Game(key) => self.game_hints(key),
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
            cx.background_executor().timer(std::time::Duration::from_millis(220)).await;
            this.update(cx, |this, cx| {
                this.console.art_key = wanted;
                cx.notify();
            })
            .ok();
        }));
    }

    /// The focused game's hero art, sharp, sized to the window, and the
    /// art it is fading from.
    fn console_art(
        &mut self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> (Option<std::sync::Arc<RenderImage>>, Option<(SharedString, std::sync::Arc<RenderImage>)>) {
        let Some(key) = self.console.art_key.clone() else {
            return (self.console.art_prev.clone(), self.console.art_cur.clone());
        };
        let scale = window.scale_factor();
        let size = window.viewport_size();
        // Under the gradients a smaller texture looks the same and keeps the
        // covers in the cache budget (a full ultrawide frame is ~20 MB).
        let (w, h) = (f32::from(size.width) * scale, f32::from(size.height) * scale);
        let shrink = (1920. / w).min(1.);
        let (pw, ph) = ((w * shrink) as u32, (h * shrink) as u32);
        let id: SharedString = format!("s:{key}:{pw}x{ph}:console").into();
        let chains = &self.stage_chains;
        let ready = self.images.get_or_request(
            &id,
            || chains.get(&key).cloned().unwrap_or_default(),
            pw,
            ph,
            Variant::Stage { blur_px: 0., saturation: 1. },
            1000,
        );
        if let Some(img) = ready {
            let same = self.console.art_cur.as_ref().is_some_and(|(k, _)| *k == key);
            if !same {
                // Cross-fade: the old art stays under the new one until it
                // is fully in.
                self.console.art_prev = self.console.art_cur.take().map(|(_, i)| i);
                self.console.art_fade = Some(cx.spawn(async move |this, cx| {
                    cx.background_executor().timer(std::time::Duration::from_millis(ART_FADE_MS + 100)).await;
                    this.update(cx, |this, cx| {
                        this.console.art_prev = None;
                        cx.notify();
                    })
                    .ok();
                }));
            }
            self.console.art_cur = Some((key.clone(), img.clone()));
            self.shown_stage = Some((key, img));
        }
        (self.console.art_prev.clone(), self.console.art_cur.clone())
    }

    /// Top bar: wordmark, LB/RB tabs, clock.
    fn console_top_bar(&self, tabs: Option<(&[SharedString], usize)>) -> Div {
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap(u(36.))
            .h(u(76.))
            .px(u(46.))
            .child(kit::wordmark("HERONITE"))
            .children(tabs.map(|(items, active)| kit::tabs(items, active)))
            .child(div().flex_1())
            .when(self.console.controller.is_some(), |d| {
                d.child(crate::ui::icon(crate::icons::Icon::SportsEsports, u(20.), theme::text_muted()))
            })
            .child(kit::clock())
    }

    pub(super) fn render_console(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        kit::set_scale(window);
        self.console_script(window, cx);
        let (art_prev, art) = self.console_art(window, cx);
        let body: AnyElement = if self.page != Page::Collection {
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
        } else {
            match self.console.screen.clone() {
                Screen::Home => self.render_console_home(window, cx),
                Screen::Library => self.render_console_library(window, cx),
                Screen::Game(key) => {
                    let page = self.render_console_game(&key, window, cx);
                    motion::enter(SharedString::from(format!("console-game-{key}")), div().relative().size_full().child(page), 320, 0., 26.)
                }
            }
        };
        let on_page = self.page != Page::Collection;
        let hints = self.console_hints(cx);
        let art_img =
            |i: std::sync::Arc<RenderImage>| img(ImageSource::Render(i)).absolute().inset_0().size_full().object_fit(ObjectFit::Cover);
        let backdrop = div()
            .absolute()
            .inset_0()
            .when(!on_page, |d| {
                d.children(art_prev.map(art_img))
                    .children(art.map(|(k, i)| motion::fade_in(SharedString::from(format!("console-art-{k}")), art_img(i), ART_FADE_MS)))
            })
            // Readable text on any art: dark from the left and the bottom.
            .child(div().absolute().inset_0().bg(linear_gradient(
                90.,
                linear_color_stop(theme::rgb_a(11, 14, 19, 0.96), 0.),
                linear_color_stop(theme::rgb_a(11, 14, 19, if on_page { 0.96 } else { 0.35 }), 1.),
            )))
            .child(div().absolute().inset_0().bg(linear_gradient(
                0.,
                linear_color_stop(theme::rgb_a(11, 14, 19, 0.98), 0.),
                linear_color_stop(theme::rgb_a(11, 14, 19, 0.), 0.75),
            )));
        div()
            .id("console")
            .relative()
            .size_full()
            .overflow_hidden()
            .bg(theme::console_bg())
            .font_family(theme::FONT_BODY)
            .text_color(theme::text_primary())
            .child(backdrop)
            .child(
                div().absolute().inset_0().flex().flex_col().child(div().flex_1().min_h_0().child(body)).child(div().flex_none().h(u(60.))),
            )
            .children(self.render_sidebar(cx))
            .children(self.console.overlay.as_ref().map(|o| o.view.clone()))
            .children(self.dialog.clone().map(|d| {
                let id = SharedString::from(format!("console-dialog-{}", d.entity_id()));
                motion::layer_in(id, d, 240, 12.)
            }))
            .children(self.render_toast())
            // Above the overlays: the hints of whatever has the focus.
            .child(div().absolute().left_0().right_0().bottom_0().child(kit::hint_bar(&hints).bg(theme::console_bg())))
            .into_any_element()
    }
}
