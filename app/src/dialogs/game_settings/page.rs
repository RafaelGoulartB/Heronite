//! The game settings in the game page's body (Focus, wide windows), in place
//! of the drawer: the groups as tabs, a search over every group's settings,
//! "Changed" (what this game set, in every group, each with its reset), and
//! "How the game starts" in a column beside them, built again as settings
//! change. The page closes with "Back to game" or Esc.

use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::collection::Game;
use heronite_i18n::tr;

use super::{GameSettingsDialog, Tab};
use crate::dialogs::settings_panel::{heading, rows, Found, Section};
use crate::dialogs::DialogCtx;
use crate::icons::Icon;
use crate::theme;
use crate::ui::controls::{action_button, ButtonKind};
use crate::ui::fields::{new_input, search_box};

/// From this width of the page, "How the game starts" sits beside the
/// settings; below, under them.
const ASIDE_FROM_W: f32 = 1000.;
const ASIDE_W: f32 = 400.;

pub(super) struct PageState {
    search: Entity<InputState>,
    query: String,
    /// Only the settings the game changed.
    changed_only: bool,
    /// The page's width (the host's), for where the preview goes.
    width: f32,
}

impl GameSettingsDialog {
    /// The settings of `game` for the game page's body, on `tab`.
    pub fn new_in_page(ctx: DialogCtx, game: Game, tab: Tab, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut this = Self::new(ctx, game, tab, window, cx);
        let search = new_input(window, cx, "", tr!("Search settings"));
        cx.subscribe(&search, |this, input, ev: &InputEvent, cx| {
            if matches!(ev, InputEvent::Change) {
                if let Some(page) = this.page.as_mut() {
                    page.query = input.read(cx).value().trim().to_string();
                    cx.notify();
                }
            }
        })
        .detach();
        // `HERONITE_DEBUG_SETUP_SEARCH=<words>` / `HERONITE_DEBUG_SETUP_CHANGED=1` (screenshots).
        let query = crate::env::value("HERONITE_DEBUG_SETUP_SEARCH").unwrap_or_default();
        if !query.is_empty() {
            search.update(cx, |s, cx| s.set_value(query.clone(), window, cx));
        }
        let changed_only = crate::env::flag("HERONITE_DEBUG_SETUP_CHANGED");
        this.page = Some(PageState { search, query, changed_only, width: 1200. });
        if let Some((panel, _)) = &this.panel {
            panel.update(cx, |p, cx| p.set_preview_aside(cx));
        }
        this
    }

    /// The host's width for the page.
    pub fn set_page_width(&mut self, width: f32) {
        if let Some(page) = self.page.as_mut() {
            page.width = width;
        }
    }

    /// Esc in the page: clears the search or "Changed" first.
    pub fn page_escape(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if self.filtering() {
            self.clear_filter(window, cx);
            return true;
        }
        self.on_escape(cx)
    }

    fn filtering(&self) -> bool {
        self.page.as_ref().is_some_and(|p| p.changed_only || !p.query.is_empty())
    }

    fn clear_filter(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(page) = self.page.as_mut() else { return };
        page.changed_only = false;
        page.query.clear();
        let input = page.search.clone();
        input.update(cx, |s, cx| s.set_value(String::new(), window, cx));
        cx.notify();
    }

    /// A found setting: its group, with the rarely used ones shown.
    fn go_to(&mut self, section: Section, rare: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.clear_filter(window, cx);
        self.show_tab(Tab::of_section(section), window, cx);
        if let (true, Some((panel, _))) = (rare, &self.panel) {
            panel.update(cx, |p, cx| p.show_rarely_used(cx));
        }
    }

    /// The settings the search or "Changed" found.
    fn found(&self, cx: &App) -> Vec<Found> {
        let (Some(page), Some((panel, _))) = (self.page.as_ref(), self.panel.as_ref()) else { return Vec::new() };
        let p = panel.read(cx);
        let mut found = if page.query.is_empty() { p.changed_settings(cx) } else { p.search(&page.query, cx) };
        if page.changed_only {
            found.retain(|f| f.changed);
        }
        found
    }

    /// The found settings, by group: a click opens the group, ↺ resets a
    /// changed one.
    fn results(&mut self, cx: &mut Context<Self>) -> Div {
        let found = self.found(cx);
        if found.is_empty() {
            let searching = self.page.as_ref().is_some_and(|p| !p.query.is_empty());
            let text = if searching { tr!("No setting matches") } else { tr!("This game uses the defaults") };
            return div().py(px(24.)).text_size(px(14.)).text_color(theme::text_muted()).child(text);
        }
        let mut sections: Vec<Section> = Vec::new();
        for f in &found {
            if !sections.contains(&f.section) {
                sections.push(f.section);
            }
        }
        let panel = self.panel.as_ref().map(|(p, _)| p.clone());
        let mut col = div().flex().flex_col().gap(px(16.));
        for section in sections {
            let mut card = rows();
            for f in found.iter().filter(|f| f.section == section) {
                let (id, rare) = (f.setting.id, f.setting.rare);
                let reset = f.changed.then(|| {
                    let panel = panel.clone();
                    div()
                        .id(SharedString::from(format!("gs-found-reset-{id}")))
                        .p(px(4.))
                        .rounded(px(6.))
                        .cursor_pointer()
                        .hover(|s| s.bg(theme::ink_a(0.08)))
                        .child(crate::ui::icon(Icon::Replay, px(15.), theme::text_dim()))
                        .on_click(cx.listener(move |_, _, _, cx| {
                            cx.stop_propagation();
                            if let Some(panel) = &panel {
                                panel.update(cx, |p, cx| p.reset_setting(id, cx));
                            }
                        }))
                });
                let value = div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .children(f.value.clone().map(|v| {
                        div()
                            .max_w(px(220.))
                            .truncate()
                            .text_size(px(13.))
                            .text_color(if f.changed { theme::text_primary() } else { theme::text_muted() })
                            .child(v)
                    }))
                    .children(reset)
                    .child(crate::ui::icon(Icon::ChevronRight, px(14.), theme::ink_a(0.4)));
                let label = heronite_i18n::tr(f.setting.label).to_string();
                card = card.child(
                    div()
                        .id(SharedString::from(format!("gs-found-{id}")))
                        .cursor_pointer()
                        .child(crate::ui::controls::row(label, None, value))
                        .on_click(cx.listener(move |this, _, window, cx| this.go_to(section, rare, window, cx))),
                );
            }
            col = col.child(div().flex().flex_col().gap(px(6.)).child(heading(section.label())).child(card.card()));
        }
        col
    }

    /// The top bar: the groups, "Changed · N", the search and "Back to game".
    fn page_bar(&mut self, cx: &mut Context<Self>) -> Div {
        let Some(page) = self.page.as_ref() else { return div() };
        let (on, search) = (page.changed_only, page.search.clone());
        let n = self.panel.as_ref().map_or(0, |(p, _)| p.read(cx).changed_settings(cx).len());
        let changed = div()
            .id("gs-changed")
            .flex_none()
            .px(px(12.))
            .py(px(6.))
            .rounded_full()
            .border_1()
            .cursor_pointer()
            .text_size(px(13.))
            .map(|d| {
                if on {
                    d.bg(theme::accent_tint()).border_color(theme::accent()).text_color(theme::text_primary())
                } else {
                    d.border_color(theme::ink_a(0.16)).text_color(theme::text_primary()).hover(|s| s.bg(theme::ink_a(0.06)))
                }
            })
            .child(format!("{} \u{b7} {n}", tr!("Changed")))
            .on_click(cx.listener(|this, _, _, cx| {
                if let Some(page) = this.page.as_mut() {
                    page.changed_only = !page.changed_only;
                    cx.notify();
                }
            }));
        let back = action_button("gs-back", ButtonKind::Glass, tr!("Back to game"), true, cx.listener(|this, _, _, cx| this.close(cx)));
        let tabs = div().flex_1().min_w_0().when(self.filtering(), |d| d.opacity(0.5)).child(self.tab_bar(cx));
        div()
            .flex()
            .items_center()
            .gap(px(10.))
            .pb(px(14.))
            .child(tabs)
            .child(changed)
            .child(div().w(px(240.)).flex_none().child(search_box(&search)))
            .child(back)
    }

    pub(super) fn render_page(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let width = self.page.as_ref().map_or(1200., |p| p.width);
        let bar = self.page_bar(cx);
        let main = if self.filtering() {
            self.results(cx)
        } else {
            let (reset, _) = self.reset_controls(cx);
            self.tab_body(cx).child(div().flex().justify_end().pt(px(6.)).child(reset))
        };
        let preview = self.panel.clone().and_then(|(p, _)| p.update(cx, |p, cx| p.preview_aside(cx)));
        let (beside, under) = if width >= ASIDE_FROM_W { (preview, None) } else { (None, preview) };
        let body = div()
            .id("gs-page-body")
            .flex_1()
            .min_w_0()
            .h_full()
            .overflow_y_scroll()
            .pr(px(8.))
            .pb(px(24.))
            .child(div().flex().flex_col().gap(px(16.)).child(main).children(under));
        let column =
            beside.map(|p| div().id("gs-page-preview").flex_none().w(px(ASIDE_W)).h_full().overflow_y_scroll().pb(px(24.)).child(p));
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(bar)
            .child(div().flex().flex_1().min_h_0().gap(px(24.)).child(body).children(column))
            .into_any_element()
    }
}
