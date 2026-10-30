//! Settings page (☰ → Settings, Ctrl ,): the ONE place for configuration.
//! Left: the sections grouped under captions (App, Games, Collection, Help);
//! right: the content.
//!
//! - General / Downloads / Advanced: one `SettingsPanel` bound to the
//!   global `config.json` (`defaultSettings`), switching its view.
//! - Game defaults: a second panel (Wine / Other / Advanced / Gamescope
//!   defaults new games inherit).
//! - Library: hidden games, favourites, recent games (`library.rs`).
//! - Storage: what the games take on disk and cleaning it up (`storage/`).
//! - Advanced extras: folders, cache, copy settings (`advanced.rs`).
//! - Appearance, Metadata, Screenshots, Backups: Collection settings, backed
//!   by `local_library/settings.json` (`appearance.rs`, `metadata.rs`,
//!   `screenshots.rs`, `saves.rs` + `backup/`). Any write ends in
//!   [`SettingsPage::saved`].
//! - Console mode: the console themes and every option of console mode
//!   (`console_mode/`; its state exists only once the section was shown).
//! - About: updates (`updates.rs`), version, folders and system information
//!   (`about.rs`).
//!
//! Emulators and ROM folders are not here: they live on the ☰ Emulators page.
//!
//! The search above the sections (Ctrl F) filters them by label, palette keywords, blurb and group;
//! Enter opens the best match, Esc empties it.
//!
//! Debug: `HERONITE_DEBUG_SETTINGS_SEARCH=<text>` types it in the search (`<text>!`: and presses Enter),
//! `HERONITE_DEBUG_SETTINGS_KEYS=down,down,enter` then presses those keys in it (`lang:pt-BR` switches the language);
//! `HERONITE_DEBUG_PAGE=settings[:general|appearance|console-mode|advanced|defaults[:wine|other|advanced|gamescope]|downloads|library|launchers|storage|overlay|replay|metadata|screenshots|backups|logs|about]`.

mod about;
mod achievements;
mod advanced;
mod appearance;
mod backup;
mod console;
mod console_mode;
mod launchers;
mod library;
pub mod logs;
mod metadata;
mod nav_keys;
mod overlay;
mod palette;
mod replay;
mod saves;
mod saves_restore;
mod screenshots;
mod search;
mod section;
mod storage;
pub mod sysinfo;
mod updates;

pub use advanced::open_folder;
pub use backup::format_time;
pub use launchers::launcher_icon;
pub use section::Section;

use std::collections::HashMap;
use std::sync::Arc;

use gpui_kit::component::input::{InputEvent, InputState, MoveDown};
use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::settings::{self as store, EffectiveSettings};
use heronite_data::Library;
use heronite_i18n::tr;
use heronite_images::ImageRef;

use super::{Page, PageCtx, PageEvent};
use crate::dialogs::settings_panel::logic::{Section as HSection, DEFAULT_SECTIONS};
use crate::dialogs::settings_panel::{GlobalView, SettingsPanel, Target};
use crate::icons::Icon;
use crate::theme;
use crate::ui::controls::{close_button, help_text, icon_button, kbd, nav_pill, page_heading, tabs};
use crate::ui::fields::search_box_with;
use crate::ui::glass::glass_panel;

/// Frames the page waits for a setting picked in the search to be drawn.
const FIND_FRAMES: u8 = 30;

/// Section the command palette asked for; taken by the page when it is
/// created or shown again (see [`request_section`]).
struct PendingSection(Section);
impl Global for PendingSection {}

/// Makes the Settings page open on `section` the next time it is shown
/// (call before navigating to it).
pub fn request_section(section: Section, cx: &mut App) {
    cx.set_global(PendingSection(section));
}

fn take_requested_section(cx: &mut App) -> Option<Section> {
    cx.has_global::<PendingSection>().then(|| cx.remove_global::<PendingSection>().0)
}

/// What the Collection hands over when it shows the page: the settings it
/// already loaded (so the page does no file I/O to build its inputs), the
/// stage artwork of the focused game (Appearance live preview), and the
/// library with its art (Storage).
struct Offer {
    settings: EffectiveSettings,
    chain: Vec<ImageRef>,
    library: Option<LibraryOffer>,
}
impl Global for Offer {}

/// The Collection's library and the art chains of its games (covers, wide art).
pub struct LibraryOffer {
    pub lib: Arc<Library>,
    pub chains: HashMap<SharedString, Vec<ImageRef>>,
    pub stages: HashMap<SharedString, Vec<ImageRef>>,
}

/// Called by the Collection right before it shows the Settings page.
pub fn offer(settings: EffectiveSettings, chain: Vec<ImageRef>, library: Option<LibraryOffer>, cx: &mut App) {
    cx.set_global(Offer { settings, chain, library });
}

pub struct SettingsPage {
    ctx: PageCtx,
    /// `local_library/settings.json` as last read or written by this page.
    settings: EffectiveSettings,
    /// Last write error of the Collection sections.
    error: Option<SharedString>,
    appearance: appearance::State,
    screenshots: screenshots::State,
    achievements: achievements::State,
    metadata: metadata::State,
    backup: backup::State,
    saves: saves::State,
    saves_restore: saves_restore::State,
    section: Section,
    global: Option<Entity<SettingsPanel>>,
    defaults: Option<Entity<SettingsPanel>>,
    defaults_section: HSection,
    library: library::State,
    launchers: launchers::State,
    overlay: overlay::State,
    replay: replay::State,
    storage: storage::State,
    advanced: advanced::State,
    about: about::State,
    logs: logs::State,
    console_mode: Option<console_mode::State>,
    /// Console mode focus, keyboard and pickers.
    console: console::ConsoleState,
    /// The section list's scroll, for its scrollbar.
    nav_scroll: ScrollHandle,
    /// The search above the section list, and what is typed in it.
    nav_search: Entity<InputState>,
    nav_query: String,
    /// The language of the search's placeholder (set again after a switch).
    nav_lang: heronite_i18n::Lang,
    /// The section list's keyboard focus and its cursor (an index in
    /// `nav_items`), and where its items were drawn (`nav_keys`).
    nav_focus: FocusHandle,
    nav_cursor: Option<usize>,
    nav_bounds: nav_keys::ItemBounds,
    /// The content's scroll, to bring a setting picked in the search into view.
    content_scroll: ScrollHandle,
    /// Frames left to wait for the picked setting to be drawn before
    /// scrolling to it (0: nothing to scroll to).
    find_frames: u8,
    /// Ends the picked setting's flash.
    find_task: Option<Task<()>>,
}

impl EventEmitter<PageEvent> for SettingsPage {}

impl SettingsPage {
    pub fn new(ctx: PageCtx, window: &mut Window, cx: &mut Context<Self>) -> Self {
        // `HERONITE_DEBUG_PAGE=settings:<section>[:<defaults tab>]`.
        let debug = crate::env::value("HERONITE_DEBUG_PAGE");
        let mut parts = debug.as_deref().unwrap_or("").split(':').skip(1);
        let section = take_requested_section(cx).or_else(|| parts.next().and_then(Section::parse)).unwrap_or(Section::General);
        let defaults_section = match parts.next() {
            Some("performance" | "other" | "gamescope") => HSection::Performance,
            Some("launch" | "advanced") => HSection::Launch,
            Some("gpu") => HSection::Gpu,
            _ => DEFAULT_SECTIONS[0],
        };
        let (settings, chain, library) = match cx.has_global::<Offer>().then(|| cx.remove_global::<Offer>()) {
            Some(o) => (o.settings, o.chain, o.library),
            // The Collection has not loaded its library yet: read the file.
            None => (store::load(&ctx.data_paths).unwrap_or_default(), Vec::new(), None),
        };
        let mut storage = storage::State::new(&ctx);
        if let Some(l) = library {
            storage.set_library(l.lib, l.chains, l.stages);
        }
        let appearance = appearance::State::new(&ctx, &settings, chain, window, cx);
        let screenshots = screenshots::State::new(&ctx, &settings, window, cx);
        let achievements = achievements::State::new(window, cx);
        let metadata = metadata::State::new(&ctx, &settings, window, cx);
        let backup = backup::State::new(&ctx, &settings, window, cx);
        let saves = saves::State::new(&ctx, &settings, window, cx);
        // `HERONITE_DEBUG_SETTINGS_SEARCH=<text>[!]`: the section list filtered by it (`!`: and Enter).
        let debug_search = crate::env::value("HERONITE_DEBUG_SETTINGS_SEARCH").unwrap_or_default();
        let debug_enter = debug_search.ends_with('!');
        let nav_query = debug_search.trim_end_matches('!').trim().to_string();
        let nav_search = crate::ui::fields::new_input(window, cx, &nav_query, tr!("Search settings"));
        cx.subscribe_in(&nav_search, window, |this, state, ev: &InputEvent, window, cx| match ev {
            InputEvent::Change => {
                this.nav_query = state.read(cx).value().trim().to_string();
                this.nav_cursor = None;
                this.nav_scroll.set_offset(point(px(0.), px(0.)));
                cx.notify();
            }
            // Enter opens the best match.
            InputEvent::PressEnter { .. } => {
                if let Some(hit) = search::search(&this.nav_query).first() {
                    this.pick(hit.pick(), window, cx);
                }
            }
            _ => {}
        })
        .detach();
        let mut this = Self {
            settings,
            error: None,
            appearance,
            screenshots,
            achievements,
            metadata,
            backup,
            saves,
            saves_restore: Default::default(),
            library: library::State::new(&ctx),
            launchers: launchers::State::new(cx),
            overlay: overlay::State::new(&ctx, cx),
            replay: replay::State::new(&ctx, cx),
            storage,
            advanced: advanced::State::default(),
            about: about::State::default(),
            logs: logs::State::default(),
            console_mode: None,
            console: Default::default(),
            nav_scroll: ScrollHandle::new(),
            nav_search,
            nav_query,
            nav_lang: heronite_i18n::current(),
            nav_focus: cx.focus_handle(),
            nav_cursor: None,
            nav_bounds: Default::default(),
            content_scroll: ScrollHandle::new(),
            find_frames: 0,
            find_task: None,
            ctx,
            section,
            global: None,
            defaults: None,
            defaults_section,
        };
        this.on_section_shown(window, cx);
        if let Some(hit) = search::search(&this.nav_query).first().filter(|_| debug_enter) {
            this.pick(hit.pick(), window, cx);
        }
        // `HERONITE_DEBUG_SETTINGS_KEYS=down,down,enter`: keystrokes typed in the search, a moment apart.
        if let Some(keys) = crate::env::value("HERONITE_DEBUG_SETTINGS_KEYS") {
            cx.spawn_in(window, async move |this, cx| {
                let pause = || std::time::Duration::from_millis(400);
                cx.background_executor().timer(pause()).await;
                this.update_in(cx, |this, window, cx| this.focus_search(window, cx)).ok();
                for k in keys.split(',').filter(|k| !k.is_empty()) {
                    cx.background_executor().timer(pause()).await;
                    // `lang:<code>` switches the interface language instead.
                    if let Some(lang) = k.strip_prefix("lang:").and_then(|c| heronite_i18n::Lang::ALL.into_iter().find(|l| l.code() == c)) {
                        heronite_i18n::set(lang);
                        cx.update(|window, _| window.refresh()).ok();
                    } else if let Ok(ks) = Keystroke::parse(k) {
                        cx.update(|window, cx| window.dispatch_keystroke(ks, cx)).ok();
                    }
                }
            })
            .detach();
        }
        if let Some(updater) = crate::services::update::get(cx) {
            cx.observe(&updater, |_, _, cx| cx.notify()).detach();
        }
        // After the Collection subscribed to this page's events.
        if crate::env::value("HERONITE_DEBUG_SETTINGS_ACTION").is_some() {
            cx.spawn(async move |this, cx| {
                cx.background_executor().timer(std::time::Duration::from_millis(600)).await;
                this.update(cx, |this, cx| this.debug_action(cx)).ok();
            })
            .detach();
        }
        this
    }

    /// `HERONITE_DEBUG_SETTINGS_ACTION=<action>` runs a write as if clicked (fixtures only):
    /// `unhide:<app>`, `unhide-all`, `fav-clear`, `recent-clear`, `max-recent:<n>`, `show-hidden:on|off`,
    /// `clear-cache`, and on Backups `backup-now`, `backup-open:<n>` (the n-th backup's details),
    /// `backup-confirm:<n>` (its restore question). `ask:<hidden|favourites|recent|launcher id>`
    /// only opens that question.
    fn debug_action(&mut self, cx: &mut Context<Self>) {
        let Some(action) = crate::env::value("HERONITE_DEBUG_SETTINGS_ACTION") else { return };
        match action.split_once(':').unwrap_or((action.as_str(), "")) {
            ("unhide", app) => self.library_unhide(app.to_string(), cx),
            ("unhide-all", _) => self.library_clear("hidden", cx),
            ("fav-clear", _) => self.library_clear("favourites", cx),
            ("recent-clear", _) => self.library_clear("recent", cx),
            ("max-recent", n) => self.set_max_recent(n.parse().unwrap_or(5), cx),
            ("show-hidden", v) => self.set_show_hidden(v == "on", cx),
            ("clear-cache", _) => self.clear_cache(cx),
            ("ask", which) => {
                if !library::debug_ask(self, which) {
                    self.launchers.debug_ask(which);
                }
                cx.notify();
            }
            (a @ ("backup-now" | "backup-open" | "backup-confirm"), n) => backup::debug_action(self, a, n.parse().unwrap_or(0), cx),
            _ => {}
        }
    }

    /// Called when the page is shown again: lists and flags may have changed
    /// from the Collection (context menu, header).
    pub fn refresh(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if cx.has_global::<Offer>() {
            // Another game may be focused now; the settings are re-read below.
            let offer = cx.remove_global::<Offer>();
            self.appearance.set_chain(offer.chain);
            if let Some(l) = offer.library {
                self.storage.set_library(l.lib, l.chains, l.stages);
                console_mode::library_changed(self);
            }
        }
        if let Some(section) = take_requested_section(cx) {
            self.select(section, window, cx);
        }
        self.library.reload(&self.ctx, cx);
        self.logs.refresh(cx);
        // Ludusavi's "last backup" lines change while a game runs.
        let data_paths = self.ctx.data_paths.clone();
        cx.spawn(async move |this, cx| {
            if let Ok(fresh) = cx.background_spawn(async move { store::load(&data_paths) }).await {
                this.update(cx, |this, cx| {
                    this.settings = fresh;
                    cx.notify();
                })
                .ok();
            }
        })
        .detach();
        self.on_section_shown(window, cx);
        cx.notify();
    }

    /// The Collection hid the page: its sections free what only a visible one
    /// needs (`refresh` brings it back when the page is shown).
    pub fn hidden(&mut self, window: &mut Window) {
        self.storage.hidden(window);
        console_mode::hidden(self, window);
        self.logs.release();
    }

    /// The Collection reloaded its library while the page is on screen (a
    /// game was uninstalled from Storage): Storage looks again.
    pub fn set_library(&mut self, offer: LibraryOffer, cx: &mut Context<Self>) {
        let changed = self.storage.set_library(offer.lib, offer.chains, offer.stages);
        console_mode::library_changed(self);
        if changed && self.section == Section::Storage {
            storage::State::shown(self, cx);
        }
    }

    /// Records the result of a settings write (any Collection section) and
    /// tells the Collection to reload.
    fn saved(&mut self, result: anyhow::Result<EffectiveSettings>, cx: &mut Context<Self>) {
        match result {
            Ok(s) => {
                self.settings = s;
                self.error = None;
                cx.emit(PageEvent::LibraryChanged);
            }
            Err(err) => self.error = Some(format!("{err:#}").into()),
        }
        cx.notify();
    }

    /// Drops the questions before a destructive action (Other launchers,
    /// Library); true when one was open.
    fn dismiss_confirms(&mut self) -> bool {
        let launchers = self.launchers.dismiss();
        let library = self.library.dismiss();
        launchers || library
    }

    fn select(&mut self, section: Section, window: &mut Window, cx: &mut Context<Self>) {
        if self.section == section {
            return;
        }
        self.section = section;
        self.error = None;
        self.dismiss_confirms();
        if let (Some(view), Some(panel)) = (section.global_view(), self.global.clone()) {
            panel.update(cx, |p, cx| p.set_global_view(view, cx));
        }
        self.on_section_shown(window, cx);
        cx.notify();
    }

    /// Starts the background work a section needs.
    fn on_section_shown(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.section != Section::Storage {
            self.storage.hidden(window);
        }
        if self.section != Section::Logs {
            self.logs.release();
        }
        if self.section != Section::ConsoleMode {
            console_mode::hidden(self, window);
        }
        match self.section {
            Section::Advanced => self.advanced.measure(&self.ctx, cx),
            Section::About => self.about.load(&self.ctx, cx),
            Section::Screenshots => self.screenshots.shown(cx),
            Section::Achievements => achievements::shown(self, cx),
            Section::Overlay => overlay::shown(cx),
            Section::Launchers => launchers::shown(self, cx),
            Section::Replay => replay::shown(self, cx),
            Section::Saves => saves::State::shown(self, window, cx),
            Section::Storage => storage::State::shown(self, cx),
            Section::Backups => backup::shown(self, cx),
            Section::ConsoleMode => console_mode::shown(self, window, cx),
            _ => {}
        }
    }

    fn panel_for(&mut self, view: GlobalView, window: &mut Window, cx: &mut Context<Self>) -> Entity<SettingsPanel> {
        if self.global.is_none() {
            let data_paths = self.ctx.data_paths.clone();
            let panel = cx.new(|cx| SettingsPanel::new_global(data_paths, view, window, cx));
            cx.observe(&panel, |_, _, cx| cx.notify()).detach();
            self.global = Some(panel);
        }
        self.global.clone().expect("created above")
    }

    fn defaults_view(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let section = self.defaults_section;
        if self.defaults.is_none() {
            let data_paths = self.ctx.data_paths.clone();
            let panel = cx.new(|cx| SettingsPanel::new(data_paths, Target::Defaults, section, window, cx));
            cx.observe(&panel, |_, _, cx| cx.notify()).detach();
            self.defaults = Some(panel);
        }
        let panel = self.defaults.clone().expect("created above");
        let items: Vec<(HSection, &'static str)> = DEFAULT_SECTIONS.iter().map(|s| (*s, s.label())).collect();
        let entity = cx.entity().downgrade();
        let sub = panel.clone();
        let bar = tabs("gdef", &items, section, move |s, window, cx| {
            sub.update(cx, |p, cx| p.set_section(s, window, cx));
            entity
                .update(cx, |this, cx| {
                    this.defaults_section = s;
                    cx.notify();
                })
                .ok();
        });
        div().flex().flex_col().gap(px(16.)).child(bar).child(panel).into_any_element()
    }

    /// Body of a Collection section plus the last write error, if any.
    fn collection_section(body: AnyElement, error: Option<SharedString>) -> AnyElement {
        let error = error.map(|e| library::notice_line(e, true));
        div().flex().flex_col().child(body).children(error).into_any_element()
    }

    /// Left column of the sheet: the search and the section pills, with a
    /// scrollbar when the window is too short for all of them.
    fn nav(&self, window: &Window, cx: &mut Context<Self>) -> Div {
        let mut col = div()
            .id("settings-nav")
            .track_focus(&self.nav_focus)
            .on_key_down(cx.listener(Self::nav_key))
            .size_full()
            .overflow_y_scroll()
            .track_scroll(&self.nav_scroll)
            .on_scroll_wheel(cx.listener(|_, _, _, cx| cx.notify()))
            .flex()
            .flex_col()
            .px(px(14.))
            .pt(px(26.))
            .pb(px(16.))
            .child(self.nav_search_box(cx));
        // Items are numbered in `nav_items` order: the keyboard cursor is one of them.
        let cursor = self.nav_cursor_shown(window);
        let mut ix = 0;
        let mut next = || {
            ix += 1;
            ix - 1
        };
        let layer = |ix: usize, radius: f32| nav_keys::item_layer(&self.nav_bounds, ix, cursor == Some(ix), px(radius));
        let pill = |s: Section, ix: usize, cx: &mut Context<Self>| {
            nav_pill(SharedString::from(format!("settings-nav-{s:?}")), s.icon(), s.label(), self.section == s)
                .relative()
                .child(layer(ix, 999.))
                .on_click(cx.listener(move |this, _, window, cx| this.nav_clicked(ix, search::Pick::Section(s), window, cx)))
        };
        if self.nav_query.is_empty() {
            for (group, sections) in Section::GROUPS {
                if !sections.iter().any(|s| s.available()) {
                    continue;
                }
                col =
                    col.child(div().px(px(12.)).pt(px(12.)).pb(px(5.)).text_size(px(12.)).text_color(theme::text_muted()).child(tr(group)));
                for s in sections.iter().copied().filter(|s| s.available()) {
                    col = col.child(pill(s, next(), cx));
                }
            }
        } else {
            // The matches, best first, without the group captions: each
            // section with the settings found in it. Enter opens the first.
            let hits = search::search(&self.nav_query);
            if hits.is_empty() {
                col = col
                    .child(div().px(px(12.)).pt(px(8.)).text_size(px(13.)).text_color(theme::text_muted()).child(tr!("No settings match")));
            }
            // What Enter opens: the cursor's item, else the best match.
            let best = hits.first().map(search::Hit::pick);
            let enter = |ix: usize, pick: search::Pick| cursor.map_or(best == Some(pick), |c| c == ix);
            // Its ↵ (a pill's label does not fill it: a spacer pushes the ↵ right).
            let hint = |d: Stateful<Div>, on: bool, spacer: bool| {
                d.when(on, |d| d.when(spacer, |d| d.child(div().flex_1())).child(kbd("\u{21b5}")))
            };
            for hit in hits {
                let section = hit.section;
                let ix = next();
                let pick = search::Pick::Section(section);
                col = col.child(hint(pill(section, ix, cx), enter(ix, pick), true));
                if hit.settings.is_empty() {
                    continue;
                }
                let mut sub = div().flex().flex_col().ml(px(20.)).mb(px(4.)).pl(px(6.)).border_l_1().border_color(theme::ink_a(0.1));
                for setting in hit.settings {
                    let pick = search::Pick::Setting(section, setting);
                    let ix = next();
                    let row = div()
                        .id(SharedString::from(format!("settings-find-{section:?}-{}", setting.label)))
                        .relative()
                        .child(layer(ix, 10.))
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .px(px(10.))
                        .py(px(6.))
                        .rounded(px(10.))
                        .text_size(px(13.))
                        .line_height(px(17.))
                        .text_color(theme::nav_text())
                        .cursor_pointer()
                        .hover(|s| s.bg(theme::ink_a(0.05)).text_color(theme::text_bright()))
                        .child(div().flex_1().min_w_0().child(tr(setting.label)))
                        .on_click(cx.listener(move |this, _, window, cx| this.nav_clicked(ix, pick, window, cx)));
                    sub = sub.child(hint(row, enter(ix, pick), false));
                }
                col = col.child(sub);
            }
        }
        // The list's size is known only after it is laid out: draw once more
        // so the scrollbar shows from the start.
        if self.nav_scroll.bounds().size.height <= px(0.) {
            let view = cx.entity_id();
            cx.defer(move |cx| cx.notify(view));
        }
        let scroll = self.nav_scroll.clone();
        let scrollbar = crate::ui::scrollbar_draggable(
            "settings-nav-scrollbar",
            scroll.offset().y,
            scroll.max_offset().y,
            scroll.bounds().size.height,
            {
                let (scroll, weak) = (scroll.clone(), cx.entity().downgrade());
                move |distance, _, cx| {
                    scroll.set_offset(point(px(0.), px(-distance)));
                    weak.update(cx, |_, cx| cx.notify()).ok();
                }
            },
            None,
        );
        div()
            .relative()
            .flex_none()
            .w(px(230.))
            .h_full()
            .border_r_1()
            .border_color(theme::row_separator())
            .child(col)
            .children(scrollbar.map(|b| b.right(px(2.))))
    }

    /// The search above the section list, with a clear button once something is typed. Esc
    /// in it empties it first, then leaves the field; ↓ moves to the list (`nav_keys`).
    fn nav_search_box(&self, cx: &mut Context<Self>) -> Div {
        let clear = (!self.nav_query.is_empty()).then(|| {
            icon_button("settings-search-clear", Icon::Close, 14., true)
                .on_click(cx.listener(|this, _, window, cx| this.clear_nav_search(window, cx)))
                .into_any_element()
        });
        div()
            .pb(px(6.))
            // ↓: the keyboard on the list.
            .on_action(cx.listener(|this, _: &MoveDown, window, cx| this.nav_enter_list(window, cx)))
            .child(search_box_with(&self.nav_search, clear.unwrap_or_else(|| div().into_any_element())))
    }

    /// Opens what the search found: a section, or a setting (its section and
    /// Game defaults tab, scrolled to it, flashing for a moment).
    fn pick(&mut self, pick: search::Pick, window: &mut Window, cx: &mut Context<Self>) {
        let (section, setting) = match pick {
            search::Pick::Section(section) => (section, None),
            search::Pick::Setting(section, setting) => (section, Some(setting)),
        };
        self.select(section, window, cx);
        let Some(setting) = setting else {
            crate::ui::find::clear();
            self.find_frames = 0;
            return;
        };
        if let Some(tab) = setting.tab {
            self.defaults_section = tab;
            if let Some(panel) = self.defaults.clone() {
                panel.update(cx, |p, cx| p.set_section(tab, window, cx));
            }
        }
        crate::ui::find::set_target(tr(setting.label).to_string());
        self.find_frames = FIND_FRAMES;
        self.find_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(crate::ui::find::FLASH).await;
            this.update(cx, |_, cx| {
                crate::ui::find::clear();
                cx.notify();
            })
            .ok();
        }));
        cx.notify();
    }

    /// Scrolls the content to the setting picked in the search once it was
    /// drawn (its bounds come from the frame before), a quarter down the view.
    fn scroll_to_found(&mut self, window: &mut Window) {
        if self.find_frames == 0 {
            return;
        }
        let Some(target) = crate::ui::find::target_bounds() else {
            // Not drawn yet (a section that loads first), or not at all.
            self.find_frames -= 1;
            window.request_animation_frame();
            return;
        };
        self.find_frames = 0;
        let view = self.content_scroll.bounds();
        let offset = f32::from(self.content_scroll.offset().y);
        let max = f32::from(self.content_scroll.max_offset().y);
        let top = f32::from(target.top() - view.top()) - offset;
        let y = (top - f32::from(view.size.height) / 4.).clamp(0., max.max(0.));
        self.content_scroll.set_offset(point(px(0.), px(-y)));
        window.request_animation_frame();
    }

    /// Esc in a field of the page (the Collection asks): empties the search
    /// when it has the keyboard and text. True when it did.
    pub fn search_escape(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let focused = self.nav_search.read(cx).focus_handle(cx).is_focused(window);
        if !focused || self.nav_query.is_empty() {
            return false;
        }
        self.clear_nav_search(window, cx);
        true
    }

    /// Empties the nav's search: every section shows again.
    fn clear_nav_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // `set_value` sends no change event.
        self.nav_search.update(cx, |s, cx| s.set_value("", window, cx));
        self.nav_query.clear();
        self.nav_cursor = None;
        cx.notify();
    }

    /// Ctrl F on the page: the keyboard in the nav's search, its text selected.
    pub fn focus_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.nav_search.update(cx, |s, cx| {
            s.select_all(window, cx);
            s.focus(window, cx);
        });
        cx.notify();
    }

    /// The content of the current section.
    fn render_body(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let section = self.section;
        match section {
            Section::General | Section::Downloads | Section::Advanced => {
                let view = section.global_view().expect("global view");
                let panel = self.panel_for(view, window, cx);
                let mut col = div().flex().flex_col().gap(px(16.)).child(panel);
                if section == Section::Advanced {
                    col = col.child(self.advanced_extras(cx));
                }
                col.into_any_element()
            }
            Section::GameDefaults => self.defaults_view(window, cx),
            Section::Library => self.library_section(cx).into_any_element(),
            Section::Storage => storage::render(self, window, cx),
            Section::Overlay => overlay::render(self, window, cx),
            Section::Launchers => launchers::render(self, cx),
            Section::Replay => replay::render(self, window, cx),
            Section::Appearance => Self::collection_section(appearance::render(self, window, cx), self.error.clone()),
            Section::Metadata => Self::collection_section(metadata::render(self, window, cx), self.error.clone()),
            Section::Achievements => achievements::render(self, window, cx),
            Section::Screenshots => Self::collection_section(screenshots::render(self, window, cx), self.error.clone()),
            Section::Saves => {
                let body = div()
                    .flex()
                    .flex_col()
                    .child(saves::render(self, window, cx))
                    // Restoring needs Ludusavi: the card above says how to get it.
                    .when(!self.saves.missing(), |d| d.child(div().mt(px(12.)).child(saves_restore::render(self, window, cx))))
                    .into_any_element();
                Self::collection_section(body, self.error.clone())
            }
            Section::Backups => Self::collection_section(backup::render(self, window, cx), self.error.clone()),
            Section::ConsoleMode => console_mode::render(self, window, cx),
            Section::Logs => self.logs_section(cx),
            Section::About => self.about_section(cx).into_any_element(),
        }
    }
}

impl Render for SettingsPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if crate::console::active() {
            return self.render_console(window, cx);
        }
        self.scroll_to_found(window);
        let lang = heronite_i18n::current();
        if self.nav_lang != lang {
            self.nav_lang = lang;
            self.nav_search.update(cx, |s, cx| s.set_placeholder(tr!("Search settings"), window, cx));
        }
        let section = self.section;
        // The console theme editor fills the height and scrolls its parts itself.
        let studio = section == Section::ConsoleMode && console_mode::is_studio(self);
        let body = self.render_body(window, cx);
        let title = div()
            .flex()
            .flex_col()
            .gap(px(4.))
            .pr(px(48.))
            .child(page_heading(section.label()))
            .when(!studio, |d| d.child(help_text(section.blurb())));
        // The log viewer and the theme editor want the width; the forms read better narrow.
        let max_w = match section {
            Section::Logs => px(1400.),
            Section::Storage => px(storage::MAX_W),
            Section::ConsoleMode if studio => px(console_mode::STUDIO_MAX_W),
            _ => px(880.),
        };
        let column = div().flex().flex_col().w_full().max_w(max_w).mx_auto().gap(px(14.)).child(title).child(body);
        let content = if studio {
            div()
                .id("page-settings")
                .flex_1()
                .min_w_0()
                .h_full()
                .flex()
                .flex_col()
                .px(px(36.))
                .pt(px(26.))
                .pb(px(18.))
                .child(column.flex_1().min_h_0())
        } else {
            div()
                .id("page-settings")
                .flex_1()
                .min_w_0()
                .h_full()
                .overflow_y_scroll()
                .track_scroll(&self.content_scroll)
                .px(px(36.))
                .pt(px(26.))
                .pb(px(32.))
                .child(column)
        };
        let close = close_button("settings-close")
            .absolute()
            .top(px(20.))
            .right(px(20.))
            .on_click(cx.listener(|_, _, _, cx| cx.emit(PageEvent::Navigate(Page::Collection))));
        // The sheet floats over the blurred art, inset from the window edges.
        let sheet = glass_panel(theme::R_SHEET)
            .flex()
            .size_full()
            .text_color(theme::text_primary())
            .font_family(theme::font_body())
            .child(self.nav(window, cx))
            .child(content)
            .child(close);
        div().size_full().p(px(12.)).pt(px(2.)).child(sheet).into_any_element()
    }
}
