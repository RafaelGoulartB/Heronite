//! Settings page (☰ → Settings): the app's GLOBAL configuration. Left: the
//! sections; right: the content. The header gear keeps opening the
//! Collection settings dialog (appearance, screenshots, metadata, backups).
//!
//! - General / Downloads / Advanced: one `HeroicSettingsPanel` bound to the
//!   global `config.json` (`defaultSettings`), switching its view.
//! - Game defaults: a second panel (Wine / Other / Advanced / Gamescope
//!   defaults new games inherit).
//! - Library: hidden games, favourites, recent games (`library.rs`).
//! - Advanced extras: folders, cache, copy settings (`advanced.rs`).
//! - About: version, folders and system information (`about.rs`).
//!
//! Debug: `HERONITE_DEBUG_PAGE=settings[:general|defaults[:wine|other|advanced|gamescope]|library|downloads|advanced|about]`.

mod about;
mod advanced;
mod library;
pub mod logs;
pub mod sysinfo;

use gpui_kit::prelude::*;
use gpui_kit::*;

use super::{Page, PageCtx, PageEvent};
use crate::dialogs::heroic_settings::logic::{Section as HSection, DEFAULT_SECTIONS};
use crate::dialogs::heroic_settings::{GlobalView, HeroicSettingsPanel, Target};
use crate::icons::Icon;
use crate::theme;
use crate::ui::controls::{close_button, help_text, nav_pill, page_heading, tabs};
use crate::ui::glass::glass_panel;
use crate::ui::icon;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Section {
    General,
    GameDefaults,
    Library,
    Downloads,
    Advanced,
    Logs,
    About,
}

impl Section {
    pub const ALL: [Section; 7] =
        [Section::General, Section::GameDefaults, Section::Library, Section::Downloads, Section::Advanced, Section::Logs, Section::About];

    pub fn label(self) -> &'static str {
        match self {
            Section::General => "General",
            Section::GameDefaults => "Game defaults",
            Section::Library => "Library",
            Section::Downloads => "Downloads",
            Section::Advanced => "Advanced",
            Section::Logs => "Logs",
            Section::About => "About",
        }
    }

    fn icon(self) -> Icon {
        match self {
            Section::General => Icon::Settings,
            Section::GameDefaults => Icon::Tune,
            Section::Library => Icon::GridView,
            Section::Downloads => Icon::Download,
            Section::Advanced => Icon::Build,
            Section::Logs => Icon::Description,
            Section::About => Icon::Memory,
        }
    }

    /// Short description under the section title.
    fn blurb(self) -> &'static str {
        match self {
            Section::General => "Folders, tray and shortcuts, Wine and Proton lookup. Saved automatically in the shared Heroic settings.",
            Section::GameDefaults => "Values new games start with. Settings of an installed game are changed from its own Settings.",
            Section::Library => "What the Collection shows: hidden games, favourites and recent games.",
            Section::Downloads => "Epic downloads and updates.",
            Section::Advanced => "Logs, folders and cache.",
            Section::Logs => "The logs of this app, Heroic and every game you launched. Switch logging off in General (Disable logs) or per game (Enable verbose logs).",
            Section::About => "This app, its data folders and the system it runs on.",
        }
    }

    fn global_view(self) -> Option<GlobalView> {
        match self {
            Section::General => Some(GlobalView::General),
            Section::Downloads => Some(GlobalView::Downloads),
            Section::Advanced => Some(GlobalView::Advanced),
            _ => None,
        }
    }

    pub fn parse(s: &str) -> Option<Section> {
        match s {
            "general" => Some(Section::General),
            "defaults" => Some(Section::GameDefaults),
            "library" => Some(Section::Library),
            "downloads" => Some(Section::Downloads),
            "advanced" => Some(Section::Advanced),
            "logs" => Some(Section::Logs),
            "about" => Some(Section::About),
            _ => None,
        }
    }
}

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

pub struct SettingsPage {
    pub(super) ctx: PageCtx,
    section: Section,
    global: Option<Entity<HeroicSettingsPanel>>,
    defaults: Option<Entity<HeroicSettingsPanel>>,
    defaults_section: HSection,
    pub(super) library: library::State,
    pub(super) advanced: advanced::State,
    pub(super) about: about::State,
    pub(super) logs: logs::State,
}

impl EventEmitter<PageEvent> for SettingsPage {}

impl SettingsPage {
    pub fn new(ctx: PageCtx, _window: &mut Window, cx: &mut Context<Self>) -> Self {
        // `HERONITE_DEBUG_PAGE=settings:<section>[:<defaults tab>]`.
        let debug = std::env::var("HERONITE_DEBUG_PAGE").ok();
        let mut parts = debug.as_deref().unwrap_or("").split(':').skip(1);
        let section = take_requested_section(cx).or_else(|| parts.next().and_then(Section::parse)).unwrap_or(Section::General);
        let defaults_section = match parts.next() {
            Some("other") => HSection::Other,
            Some("advanced") => HSection::Advanced,
            Some("gamescope") => HSection::Gamescope,
            _ => DEFAULT_SECTIONS[0],
        };
        let mut this = Self {
            library: library::State::new(&ctx),
            advanced: advanced::State::default(),
            about: about::State::default(),
            logs: logs::State::default(),
            ctx,
            section,
            global: None,
            defaults: None,
            defaults_section,
        };
        this.on_section_shown(cx);
        // After the Collection subscribed to this page's events.
        if std::env::var_os("HERONITE_DEBUG_SETTINGS_ACTION").is_some() {
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
        let Ok(action) = std::env::var("HERONITE_DEBUG_SETTINGS_ACTION") else { return };
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
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        if let Some(section) = take_requested_section(cx) {
            self.select(section, cx);
        }
        self.library.reload(&self.ctx, cx);
        self.logs.refresh(cx);
        self.on_section_shown(cx);
        cx.notify();
    }

    fn select(&mut self, section: Section, cx: &mut Context<Self>) {
        if self.section == section {
            return;
        }
        self.section = section;
        if let (Some(view), Some(panel)) = (section.global_view(), self.global.clone()) {
            panel.update(cx, |p, cx| p.set_global_view(view, cx));
        }
        self.on_section_shown(cx);
        cx.notify();
    }

    /// Starts the background work a section needs.
    fn on_section_shown(&mut self, cx: &mut Context<Self>) {
        match self.section {
            Section::Advanced => self.advanced.measure(&self.ctx, cx),
            Section::About => self.about.load(&self.ctx, cx),
            _ => {}
        }
    }

    fn panel_for(&mut self, view: GlobalView, window: &mut Window, cx: &mut Context<Self>) -> Entity<HeroicSettingsPanel> {
        if self.global.is_none() {
            let heroic = self.ctx.heroic.clone();
            let panel = cx.new(|cx| HeroicSettingsPanel::new_global(heroic, view, window, cx));
            cx.observe(&panel, |_, _, cx| cx.notify()).detach();
            self.global = Some(panel);
        }
        self.global.clone().expect("created above")
    }

    fn defaults_view(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let section = self.defaults_section;
        if self.defaults.is_none() {
            let heroic = self.ctx.heroic.clone();
            let panel = cx.new(|cx| HeroicSettingsPanel::new(heroic, Target::Defaults, section, window, cx));
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

    /// Left column of the sheet: title (gear + "Settings") and the section pills.
    fn nav(&self, cx: &mut Context<Self>) -> Div {
        let mut col = div()
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
        for s in Section::ALL {
            let active = self.section == s;
            col = col.child(
                nav_pill(SharedString::from(format!("settings-nav-{s:?}")), s.icon(), s.label(), active)
                    .on_click(cx.listener(move |this, _, _, cx| this.select(s, cx))),
            );
        }
        col
    }
}

impl Render for SettingsPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let section = self.section;
        let body: AnyElement = match section {
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
            Section::Logs => self.logs_section(cx),
            Section::About => self.about_section(cx).into_any_element(),
        };
        let title = div()
            .flex()
            .flex_col()
            .gap(px(4.))
            .pr(px(48.))
            .child(page_heading(section.label()))
            .child(help_text(section.blurb()));
        // The log viewer wants the width; the forms read better narrow.
        let max_w = if section == Section::Logs { px(1400.) } else { px(880.) };
        let column = div().flex().flex_col().w_full().max_w(max_w).mx_auto().gap(px(14.)).child(title).child(body);
        let content = div()
            .id("page-settings")
            .flex_1()
            .min_w_0()
            .h_full()
            .overflow_y_scroll()
            .px(px(36.))
            .pt(px(26.))
            .pb(px(32.))
            .child(column);
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
        div().size_full().p(px(12.)).pt(px(2.)).child(sheet)
    }
}

#[cfg(test)]
mod tests {
    use super::Section;

    #[test]
    fn sections_parse_for_debug_and_settings_page_is_a_debug_page() {
        for s in Section::ALL {
            assert!(!s.label().is_empty());
        }
        assert_eq!(Section::parse("library"), Some(Section::Library));
        assert_eq!(Section::parse("defaults"), Some(Section::GameDefaults));
        assert_eq!(Section::parse("nope"), None);
        assert_eq!(super::super::Page::from_debug("settings"), Some(super::super::Page::Settings));
        assert_eq!(super::super::Page::from_debug("settings:about"), Some(super::super::Page::Settings));
        assert_eq!(super::super::Page::from_debug("settingsx"), None);
    }
}
