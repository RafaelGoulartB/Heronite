//! Collection hooks of the metadata / game-settings dialogs: openers, the
//! bulk-download header chip, the idle auto-fill and the debug dialogs.

use std::time::Duration;

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_metadata::BulkItem;

use super::CollectionView;
use crate::dialogs::game_settings::{GameSettingsDialog, Tab as GameTab};
use crate::dialogs::metadata::MetadataDialog;
use crate::dialogs::metadata_wizard::{hub, HubEvent, HubGlobal, MetadataHub, MetadataWizard};
use crate::icons::Icon;
use crate::theme;
use crate::ui::icon;

impl CollectionView {
    /// Creates the metadata hub (service, bulk job, idle fill) and wires its
    /// events. Called once from `CollectionView::new`.
    pub(super) fn init_metadata(&self, window: &mut Window, cx: &mut Context<Self>) {
        let hub_entity = cx.new(|cx| MetadataHub::new(self.data_paths.clone(), cx));
        cx.set_global(HubGlobal(hub_entity.clone()));
        cx.subscribe_in(&hub_entity, window, |this, _, ev: &HubEvent, window, cx| match ev {
            HubEvent::DataChanged => this.reload(cx),
            HubEvent::OpenWizard => {
                this.dialog = None;
                this.dialog_sub = None;
                this.open_wizard(window, cx);
            }
        })
        .detach();
        // The header chip follows the job's progress.
        cx.observe(&hub_entity, |_, _, cx| cx.notify()).detach();
        let secs = crate::env::parse("HERONITE_METADATA_IDLE_SECS").unwrap_or(90u64);
        hub_entity.update(cx, |h, cx| h.schedule_idle(Duration::from_secs(secs), cx));
    }

    /// Fills the metadata of games that have none right away (new games
    /// arrived), instead of waiting for the idle fill.
    pub(super) fn fill_metadata_now(&self, cx: &mut Context<Self>) {
        if cx.has_global::<HubGlobal>() {
            hub(cx).update(cx, |h, cx| h.schedule_idle(Duration::ZERO, cx));
        }
    }

    pub(super) fn game_for(&self, key: &str) -> Option<heronite_data::collection::Game> {
        self.lib.as_ref()?.get(key).cloned()
    }

    /// Focus footer / context menu: "Collection metadata".
    pub(super) fn open_metadata_for(&mut self, key: &str, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(ctx), Some(game)) = (self.dialog_ctx(), self.game_for(key)) else { return };
        let view = cx.new(|cx| MetadataDialog::new(ctx, game, window, cx));
        self.open_dialog(view, cx);
    }

    pub(super) fn open_metadata(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(key) = self.selected.clone() {
            self.open_metadata_for(&key, window, cx);
        }
    }

    pub(super) fn open_game_settings_tab(&mut self, tab: GameTab, window: &mut Window, cx: &mut Context<Self>) {
        let Some(key) = self.selected.clone() else { return };
        let (Some(ctx), Some(game)) = (self.dialog_ctx(), self.game_for(&key)) else { return };
        let view = cx.new(|cx| GameSettingsDialog::new(ctx, game, tab, window, cx));
        self.open_dialog(view, cx);
    }

    /// Focus footer: "Game settings".
    pub(super) fn open_game_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open_game_settings_tab(GameTab::Game, window, cx);
    }

    /// Games the wizard checks: the ones the list currently shows.
    pub(super) fn open_wizard(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let (Some(ctx), Some(lib)) = (self.dialog_ctx(), self.lib.clone()) else { return };
        let games: Vec<BulkItem> = self
            .groups
            .iter()
            .flat_map(|g| g.cards.iter())
            .filter_map(|c| lib.get(&c.key))
            .map(|g| BulkItem {
                app_name: g.app_name.clone(),
                runner: if g.runner == "zoom" { "sideload".into() } else { g.runner.clone() },
                title: g.title.clone(),
                steam_app_id: g.steam_app_id.clone(),
            })
            .collect();
        hub(cx).update(cx, |h, cx| {
            h.dismissed = false;
            cx.notify();
        });
        // Debug: HERONITE_DEBUG_WIZARD_LIMIT=n runs the job on the first n games.
        let limit = crate::env::parse::<usize>("HERONITE_DEBUG_WIZARD_LIMIT");
        let games = match limit {
            Some(n) => games.into_iter().take(n).collect(),
            None => games,
        };
        let view = cx.new(|cx| MetadataWizard::new(ctx, games, cx));
        if limit.is_some() {
            view.update(cx, |w, cx| w.start_now(cx));
        }
        self.open_dialog(view, cx);
    }

    /// Header chip with the percentage of the running bulk download.
    pub(super) fn bulk_chip(&self, cx: &mut Context<Self>) -> Option<Stateful<Div>> {
        let hub_entity = hub(cx);
        let h = hub_entity.read(cx);
        if !h.chip_visible() {
            return None;
        }
        let percent = h.progress.percent;
        Some(
            div()
                .id("hdr-bulk")
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .gap(px(3.2))
                .h(theme::TOOL_BTN)
                .min_w(theme::TOOL_BTN)
                .px(px(6.4))
                .rounded(px(4.))
                .cursor_pointer()
                .hover(|s| s.bg(theme::white_a(0.08)))
                .on_click(cx.listener(|this, _, window, cx| this.open_wizard(window, cx)))
                .child(icon(Icon::CloudDownload, theme::TOOL_ICON, theme::white_a(0.82)))
                .child(
                    div()
                        .min_w(px(31.))
                        .flex()
                        .justify_center()
                        .text_size(px(11.))
                        .font_weight(theme::W_BOLD)
                        .text_color(theme::white_a(0.82))
                        .child(format!("{percent}%")),
                ),
        )
    }

    /// `HERONITE_DEBUG_DIALOG` values of the metadata dialogs.
    pub(super) fn open_metadata_debug(&mut self, which: &str, window: &mut Window, cx: &mut Context<Self>) {
        match which {
            "metadata" => self.open_metadata(window, cx),
            "gamesettings" => self.open_game_settings_tab(GameTab::Game, window, cx),
            "gamesettings:details" => self.open_game_settings_tab(GameTab::Game, window, cx),
            "gamesettings:launch" => self.open_game_settings_tab(GameTab::Actions, window, cx),
            "webimage" => {
                let Some(key) = self.selected.clone() else { return };
                let (Some(ctx), Some(game)) = (self.dialog_ctx(), self.game_for(&key)) else { return };
                let view = cx.new(|cx| GameSettingsDialog::new(ctx, game, GameTab::Game, window, cx));
                view.update(cx, |v, cx| v.debug_open_web(window, cx));
                self.open_dialog(view, cx);
            }
            "wizard" => self.open_wizard(window, cx),
            _ => {}
        }
    }
}
