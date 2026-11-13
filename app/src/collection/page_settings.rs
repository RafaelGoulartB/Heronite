//! The game settings in the game page's body: on a wide Focus page they take
//! the place of the overview and details (the title and the left column
//! stay, with the Setup card showing the group open), instead of opening a
//! drawer. The Grid view, console mode and narrow windows keep the drawer.

use std::time::Duration;

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::collection::Game;

use super::CollectionView;
use crate::dialogs::game_settings::{GameSettingsDialog, Tab};
use crate::dialogs::DialogEvent;

/// From this window width the settings open in the page (narrower: the
/// drawer). The focus panel's own narrow layout starts at 1100.
pub(super) const PAGE_FROM_W: f32 = 1100.;

pub(super) struct PageSettings {
    /// The game whose settings these are (the selected card).
    pub key: SharedString,
    pub view: Entity<GameSettingsDialog>,
    _events: Subscription,
    _changes: Subscription,
    /// The Setup card reads the settings again once they settle.
    setup_task: Option<Task<()>>,
}

impl CollectionView {
    /// Whether the selected game's settings open in the page now.
    fn page_fits(&self, vw: f32) -> bool {
        !crate::console::active() && !self.ui.grid_view && self.page == crate::pages::Page::Collection && vw >= PAGE_FROM_W
    }

    /// `game`'s settings on `tab`: in the page when `key` is the game on the
    /// Focus page and it is wide enough, else in the drawer. "Edit details"
    /// (`Tab::Game`) always opens the drawer.
    pub(super) fn show_game_settings(&mut self, key: &str, game: Game, tab: Tab, window: &mut Window, cx: &mut Context<Self>) {
        let vw = f32::from(crate::ui::content_size(window).width);
        if tab != Tab::Game && self.selected.as_deref() == Some(key) && self.page_fits(vw) {
            if let Some(page) = self.page_settings.as_ref().filter(|p| p.key.as_ref() == key) {
                page.view.update(cx, |v, cx| v.show_tab(tab, window, cx));
                return;
            }
            self.open_page_settings(key.to_string().into(), game, tab, window, cx);
            return;
        }
        let Some(ctx) = self.dialog_ctx() else { return };
        let view = cx.new(|cx| GameSettingsDialog::new(ctx, game, tab, window, cx));
        self.open_dialog(view, cx);
    }

    fn open_page_settings(&mut self, key: SharedString, game: Game, tab: Tab, window: &mut Window, cx: &mut Context<Self>) {
        let Some(ctx) = self.dialog_ctx() else { return };
        self.open_menu = None;
        self.context_menu = None;
        let view = cx.new(|cx| GameSettingsDialog::new_in_page(ctx, game, tab, window, cx));
        let events = cx.subscribe(&view, |this, _, ev: &DialogEvent, cx| {
            match ev {
                DialogEvent::Close => this.close_page_settings(cx),
                DialogEvent::Changed => this.reload(cx),
                DialogEvent::ChangedAndClose => {
                    this.close_page_settings(cx);
                    this.reload(cx);
                }
            }
            cx.notify();
        });
        let changes = cx.observe(&view, |this, _, cx| this.setup_soon(cx));
        self.page_settings = Some(PageSettings { key, view, _events: events, _changes: changes, setup_task: None });
        cx.notify();
    }

    /// Back to the game's overview.
    pub(super) fn close_page_settings(&mut self, cx: &mut Context<Self>) {
        if self.page_settings.take().is_some() {
            self.load_setup(cx);
            cx.notify();
        }
    }

    /// The Setup card again once the settings have been saved.
    fn setup_soon(&mut self, cx: &mut Context<Self>) {
        let task = cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_millis(1500)).await;
            this.update(cx, |this, cx| this.load_setup(cx)).ok();
        });
        if let Some(page) = self.page_settings.as_mut() {
            page.setup_task = Some(task);
        }
    }

    /// Every render: the page follows the selected game (on the same tab)
    /// and closes in the Grid view, console mode or a narrow window.
    pub(super) fn sync_page_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(page) = self.page_settings.as_ref() else { return };
        let vw = f32::from(crate::ui::content_size(window).width);
        if !self.page_fits(vw) {
            self.close_page_settings(cx);
            return;
        }
        if self.selected.as_ref() == Some(&page.key) {
            return;
        }
        let tab = page.view.read(cx).current_tab();
        self.page_settings = None;
        if let Some(key) = self.selected.clone() {
            if let Some(game) = self.acting_copy(&key) {
                self.open_page_settings(key, game, tab, window, cx);
            }
        }
    }

    /// The page for the focus panel's body, sized to it.
    pub(super) fn page_settings_body(&self, width: f32, cx: &mut Context<Self>) -> Option<AnyElement> {
        let page = self.page_settings.as_ref()?;
        page.view.update(cx, |v, _| v.set_page_width(width));
        Some(page.view.clone().into_any_element())
    }

    /// The group the page shows, for the Setup card.
    pub(super) fn page_settings_tab(&self, key: &SharedString, cx: &App) -> Option<Tab> {
        self.page_settings.as_ref().filter(|p| &p.key == key).map(|p| p.view.read(cx).current_tab())
    }

    /// Esc while the page is open: the page's own Esc (search, the reset
    /// question), else back to the game.
    pub(super) fn page_settings_escape(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some(view) = self.page_settings.as_ref().map(|p| p.view.clone()) else { return false };
        view.update(cx, |v, cx| v.page_escape(window, cx))
    }
}
