//! Settings page (☰ → Settings, Ctrl ,): the ONE place for configuration.
//! Left: the sections grouped under captions (App, Games, Collection, Help);
//! right: the content.
//!
//! - General / Downloads / Advanced: one `SettingsPanel` bound to the
//!   global `config.json` (`defaultSettings`), switching its view.
//! - Game defaults: a second panel (Wine / Other / Advanced / Gamescope
//!   defaults new games inherit).
//! - Library: hidden games, favourites, recent games (`library.rs`).
//! - Advanced extras: folders, cache, copy settings (`advanced.rs`).
//! - Appearance, Metadata, Screenshots, Backups: Collection settings, backed
//!   by `local_library/settings.json` (`appearance.rs`, `metadata.rs`,
//!   `screenshots.rs`, `saves.rs` + `backup.rs`). Any write ends in
//!   [`SettingsPage::saved`].
//! - About: updates (`updates.rs`), version, folders and system information
//!   (`about.rs`).
//!
//! Emulators and ROM folders are not here: they live on the ☰ Emulators page.
//!
//! Debug: `HERONITE_DEBUG_PAGE=settings[:general|appearance|advanced|defaults[:wine|other|advanced|gamescope]|downloads|library|metadata|screenshots|backups|logs|about]`.

mod about;
mod advanced;
mod appearance;
mod backup;
mod console;
mod library;
pub mod logs;
mod metadata;
mod palette;
mod saves;
mod screenshots;
mod section;
pub mod sysinfo;
mod updates;

pub use section::Section;

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::settings::{self as store, EffectiveSettings};
use heronite_images::ImageRef;

use super::{Page, PageCtx, PageEvent};
use crate::dialogs::settings_panel::logic::{Section as HSection, DEFAULT_SECTIONS};
use crate::dialogs::settings_panel::{GlobalView, SettingsPanel, Target};
use crate::icons::Icon;
use crate::theme;
use crate::ui::controls::{close_button, help_text, nav_pill, page_heading, tabs};
use crate::ui::glass::glass_panel;
use crate::ui::icon;

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
/// already loaded (so the page does no file I/O to build its inputs) and the
/// stage artwork of the focused game (Appearance live preview).
struct Offer {
    settings: EffectiveSettings,
    chain: Vec<ImageRef>,
}
impl Global for Offer {}

/// Called by the Collection right before it shows the Settings page.
pub fn offer(settings: EffectiveSettings, chain: Vec<ImageRef>, cx: &mut App) {
    cx.set_global(Offer { settings, chain });
}

pub struct SettingsPage {
    ctx: PageCtx,
    /// `local_library/settings.json` as last read or written by this page.
    settings: EffectiveSettings,
    /// Last write error of the Collection sections.
    error: Option<SharedString>,
    appearance: appearance::State,
    screenshots: screenshots::State,
    metadata: metadata::State,
    backup: backup::State,
    saves: saves::State,
    section: Section,
    global: Option<Entity<SettingsPanel>>,
    defaults: Option<Entity<SettingsPanel>>,
    defaults_section: HSection,
    library: library::State,
    advanced: advanced::State,
    about: about::State,
    logs: logs::State,
    /// Console mode focus, keyboard and pickers.
    console: console::ConsoleState,
}

impl EventEmitter<PageEvent> for SettingsPage {}

impl SettingsPage {
    pub fn new(ctx: PageCtx, window: &mut Window, cx: &mut Context<Self>) -> Self {
        // `HERONITE_DEBUG_PAGE=settings:<section>[:<defaults tab>]`.
        let debug = crate::env::value("HERONITE_DEBUG_PAGE");
        let mut parts = debug.as_deref().unwrap_or("").split(':').skip(1);
        let section = take_requested_section(cx).or_else(|| parts.next().and_then(Section::parse)).unwrap_or(Section::General);
        let defaults_section = match parts.next() {
            Some("other") => HSection::Other,
            Some("advanced") => HSection::Advanced,
            Some("gamescope") => HSection::Gamescope,
            _ => DEFAULT_SECTIONS[0],
        };
        let (settings, chain) = match cx.has_global::<Offer>().then(|| cx.remove_global::<Offer>()) {
            Some(o) => (o.settings, o.chain),
            // The Collection has not loaded its library yet: read the file.
            None => (store::load(&ctx.data_paths).unwrap_or_default(), Vec::new()),
        };
        let appearance = appearance::State::new(&ctx, &settings, chain, window, cx);
        let screenshots = screenshots::State::new(&ctx, &settings, window, cx);
        let metadata = metadata::State::new(&ctx, &settings, window, cx);
        let backup = backup::State::new(&ctx, &settings, window, cx);
        let saves = saves::State::new(&ctx, &settings, window, cx);
        let mut this = Self {
            settings,
            error: None,
            appearance,
            screenshots,
            metadata,
            backup,
            saves,
            library: library::State::new(&ctx),
            advanced: advanced::State::default(),
            about: about::State::default(),
            logs: logs::State::default(),
            console: Default::default(),
            ctx,
            section,
            global: None,
            defaults: None,
            defaults_section,
        };
        this.on_section_shown(window, cx);
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
    /// `clear-cache`.
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
            _ => {}
        }
    }

    /// Called when the page is shown again: lists and flags may have changed
    /// from the Collection (context menu, header).
    pub fn refresh(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if cx.has_global::<Offer>() {
            // Another game may be focused now; the settings are re-read below.
            let chain = cx.remove_global::<Offer>().chain;
            self.appearance.set_chain(chain);
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

    fn select(&mut self, section: Section, window: &mut Window, cx: &mut Context<Self>) {
        if self.section == section {
            return;
        }
        self.section = section;
        self.error = None;
        if let (Some(view), Some(panel)) = (section.global_view(), self.global.clone()) {
            panel.update(cx, |p, cx| p.set_global_view(view, cx));
        }
        self.on_section_shown(window, cx);
        cx.notify();
    }

    /// Starts the background work a section needs.
    fn on_section_shown(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.section {
            Section::Advanced => self.advanced.measure(&self.ctx, cx),
            Section::About => self.about.load(&self.ctx, cx),
            Section::Screenshots => self.screenshots.shown(cx),
            Section::Backups => saves::State::shown(self, window, cx),
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

    /// Left column of the sheet: title (gear + "Settings") and the section pills.
    fn nav(&self, cx: &mut Context<Self>) -> Stateful<Div> {
        let mut col = div()
            .id("settings-nav")
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .flex_none()
            .w(px(230.))
            .h_full()
            .px(px(14.))
            .pt(px(26.))
            .pb(px(16.))
            .border_r_1()
            .border_color(theme::row_separator())
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .px(px(12.))
                    .pb(px(18.))
                    .child(icon(Icon::Settings, px(18.), theme::text_primary()))
                    .child(
                        div()
                            .font_family(theme::FONT_HEADING)
                            .text_size(px(19.))
                            .font_weight(theme::W_BOLD)
                            .text_color(theme::text_primary())
                            .child("Settings"),
                    ),
            );
        for (group, sections) in Section::GROUPS {
            col = col.child(div().px(px(12.)).pt(px(12.)).pb(px(5.)).text_size(px(12.)).text_color(theme::text_muted()).child(group));
            for s in sections.iter().copied() {
                let active = self.section == s;
                col = col.child(
                    nav_pill(SharedString::from(format!("settings-nav-{s:?}")), s.icon(), s.label(), active)
                        .on_click(cx.listener(move |this, _, window, cx| this.select(s, window, cx))),
                );
            }
        }
        col
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
            Section::Appearance => Self::collection_section(appearance::render(self, window, cx), self.error.clone()),
            Section::Metadata => Self::collection_section(metadata::render(self, window, cx), self.error.clone()),
            Section::Screenshots => Self::collection_section(screenshots::render(self, window, cx), self.error.clone()),
            Section::Backups => {
                let body = div()
                    .flex()
                    .flex_col()
                    .child(saves::render(self, window, cx))
                    .child(div().mt(px(12.)).child(backup::render(self, window, cx)))
                    .into_any_element();
                Self::collection_section(body, self.error.clone())
            }
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
        let section = self.section;
        let body = self.render_body(window, cx);
        let title = div().flex().flex_col().gap(px(4.)).pr(px(48.)).child(page_heading(section.label())).child(help_text(section.blurb()));
        // The log viewer wants the width; the forms read better narrow.
        let max_w = if section == Section::Logs { px(1400.) } else { px(880.) };
        let column = div().flex().flex_col().w_full().max_w(max_w).mx_auto().gap(px(14.)).child(title).child(body);
        let content =
            div().id("page-settings").flex_1().min_w_0().h_full().overflow_y_scroll().px(px(36.)).pt(px(26.)).pb(px(32.)).child(column);
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
            .font_family(theme::FONT_BODY)
            .child(self.nav(cx))
            .child(content)
            .child(close);
        div().size_full().p(px(12.)).pt(px(2.)).child(sheet).into_any_element()
    }
}
