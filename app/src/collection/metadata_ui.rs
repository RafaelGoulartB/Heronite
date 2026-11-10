//! Collection hooks of the metadata / game-settings dialogs: openers (the
//! "Update metadata" dialog included), the hub's events (reload, the toast of
//! a finished one-click update), the idle auto-fill and the debug dialogs.

use std::time::Duration;

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_i18n::{tr, trn};
use heronite_metadata::{BulkArgs, BulkItem, BulkProgress, JobKind, UpdateMode};

use super::{CollectionView, ToastAction};
use crate::dialogs::covers::CoversDialog;
use crate::dialogs::extensions::{ExtensionsDialog, SourcesRequest};
use crate::dialogs::game_settings::{GameSettingsDialog, Tab as GameTab};
use crate::dialogs::metadata::MetadataDialog;
use crate::dialogs::metadata_update::hub::{hub, HubEvent, HubGlobal, MetadataHub};
use crate::dialogs::metadata_update::{words, MetadataUpdate, UpdateGames};

impl CollectionView {
    /// Creates the metadata hub (service, bulk job, idle fill) and wires its
    /// events. Called once from `CollectionView::new`.
    pub(super) fn init_metadata(&self, window: &mut Window, cx: &mut Context<Self>) {
        let hub_entity = cx.new(|cx| MetadataHub::new(self.data_paths.clone(), cx));
        cx.set_global(HubGlobal(hub_entity.clone()));
        cx.subscribe_in(&hub_entity, window, |this, _, ev: &HubEvent, window, cx| match ev {
            HubEvent::DataChanged => this.reload(cx),
            HubEvent::OpenUpdate => {
                this.dialog = None;
                this.dialog_sub = None;
                this.open_metadata_update(false, window, cx);
            }
            HubEvent::JobDone(p) => this.metadata_job_done(p, cx),
            HubEvent::Undone(report) => {
                this.show_toast(trn!(report.games, "Update undone: {n} game put back", "Update undone: {n} games put back"), false, cx)
            }
            // The selection's "Fix matches…" says how many of its games wait.
            HubEvent::ReviewChanged => this.forget_meta_counts(cx),
            HubEvent::OpenReview(request) => {
                this.dialog = None;
                this.dialog_sub = None;
                this.open_review(request.clone(), window, cx);
            }
            HubEvent::OpenAttention => {
                this.dialog = None;
                this.dialog_sub = None;
                if let Some(view) = this.open_metadata_update(false, window, cx) {
                    view.update(cx, |d, cx| d.show_attention(cx));
                }
            }
            HubEvent::OpenSettings => {
                this.dialog = None;
                this.dialog_sub = None;
                this.open_settings_section(crate::pages::settings::Section::Metadata, window, cx);
            }
            HubEvent::OpenSources(request) => {
                this.dialog = None;
                this.dialog_sub = None;
                this.open_sources(request.clone(), window, cx);
            }
            // Settings → Metadata reads them.
            HubEvent::SourceDownloaded | HubEvent::ExtensionsChanged => {}
        })
        .detach();
        // The header pill follows the job's progress.
        cx.observe(&hub_entity, |_, _, cx| cx.notify()).detach();
        let secs = crate::env::parse("HERONITE_METADATA_IDLE_SECS").unwrap_or(90u64);
        hub_entity.update(cx, |h, cx| h.schedule_idle(Duration::from_secs(secs), cx));
    }

    /// The toast of a one-click update ("Fill what is missing") once it ended.
    fn metadata_job_done(&mut self, p: &BulkProgress, cx: &mut Context<Self>) {
        if p.kind != JobKind::FillMissing {
            return;
        }
        let t = p.tally();
        let problems = words::problems(&t);
        if t.updated > 0 {
            self.show_toast(trn!(t.updated, "Updated {n} game", "Updated {n} games"), false, cx);
            if p.can_undo {
                self.toast_action = Some(ToastAction::UndoMetadata);
            }
        } else if problems > 0 {
            self.show_toast(trn!(problems, "{n} game was not found", "{n} games were not found"), false, cx);
        } else {
            self.show_toast(tr!("Nothing new for these games"), false, cx);
        }
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

    /// "Get more sources", or the installer of a file
    /// (`dialogs/extensions`).
    pub(super) fn open_sources(&mut self, request: SourcesRequest, window: &mut Window, cx: &mut Context<Self>) {
        let view = match request {
            SourcesRequest::Gallery { pick } => cx.new(|cx| ExtensionsDialog::gallery(pick, window, cx)),
            SourcesRequest::File(path) => cx.new(|cx| ExtensionsDialog::file(path, window, cx)),
        };
        self.open_dialog(view, cx);
    }

    /// The focus panel's button, the context menu and the palette: "Metadata".
    pub(super) fn open_metadata_for(&mut self, key: &str, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(ctx), Some(game)) = (self.dialog_ctx(), self.game_for(key)) else { return };
        let view = cx.new(|cx| MetadataDialog::new(ctx, game, window, cx));
        self.open_dialog(view, cx);
    }

    /// "Covers…": the game's picture of each cover family and its background.
    pub(super) fn open_covers_for(&mut self, key: &str, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(ctx), Some(game)) = (self.dialog_ctx(), self.game_for(key)) else { return };
        let view = cx.new(|cx| CoversDialog::new(ctx, game, None, window, cx));
        self.open_dialog(view, cx);
    }

    /// `HERONITE_DEBUG_DIALOG=covers[:<state>]`: "Covers" of the selected
    /// game over made-up searches (see the dialog's `fake.rs`).
    fn open_covers_debug(&mut self, state: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(key) = self.selected.clone() else { return };
        let (Some(ctx), Some(game)) = (self.dialog_ctx(), self.game_for(&key)) else { return };
        let thumbs: Vec<String> = self
            .lib
            .as_ref()
            .map(|lib| {
                let urls = lib.games.iter().filter_map(|g| g.metadata.as_ref()).flat_map(|m| [m.cover_url.clone(), m.hero_url.clone()]);
                urls.flatten().filter(|u| u.starts_with("http")).take(24).collect()
            })
            .unwrap_or_default();
        let state = state.to_string();
        let view = cx.new(|cx| CoversDialog::made_up(ctx, game, &state, thumbs, window, cx));
        self.open_dialog(view, cx);
    }

    pub(super) fn open_metadata(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(key) = self.selected.clone() {
            self.open_metadata_for(&key, window, cx);
        }
    }

    pub(super) fn open_game_settings_tab(&mut self, tab: GameTab, window: &mut Window, cx: &mut Context<Self>) {
        let Some(key) = self.selected.clone() else { return };
        // The settings of the copy Play starts (the installed one of a card of copies).
        let (Some(ctx), Some(game)) = (self.dialog_ctx(), self.acting_copy(&key)) else { return };
        let view = cx.new(|cx| GameSettingsDialog::new(ctx, game, tab, window, cx));
        self.open_dialog(view, cx);
    }

    /// Focus footer: "Game settings".
    pub(super) fn open_game_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open_game_settings_tab(GameTab::Game, window, cx);
    }

    /// The list's search and filters as words ("Platform: PlayStation 2").
    fn filters_in_words(&self) -> Option<String> {
        let mut words: Vec<String> = Vec::new();
        if !self.search.trim().is_empty() {
            words.push(format!("\u{201c}{}\u{201d}", self.search.trim()));
        }
        words.extend(self.view_tokens().into_iter().map(|(kind, choice)| match kind {
            Some(kind) => format!("{kind}: {}", choice.label),
            None => choice.label,
        }));
        (!words.is_empty()).then(|| words.join(", "))
    }

    /// The games the dialog offers: the selection (when `selection`), the
    /// games on screen and the whole Collection, with their covers.
    fn update_games(&self, selection: bool) -> Option<UpdateGames> {
        let lib = self.lib.clone()?;
        let shown = self.groups.iter().flat_map(|g| g.cards.iter()).filter_map(|c| lib.get(&c.key)).map(|g| bulk_item(g)).collect();
        let selected: Vec<BulkItem> = if selection { self.selected_games().iter().map(|g| bulk_item(g)).collect() } else { Vec::new() };
        let covers =
            lib.games.iter().filter_map(|g| self.cover_chains.get(g.key.as_str()).map(|chain| (g.metadata_key(), chain.clone()))).collect();
        Some(UpdateGames {
            selected: (!selected.is_empty()).then_some(selected),
            shown,
            all: lib.games.iter().map(|g| bulk_item(g)).collect(),
            filters: self.filters_in_words(),
            covers,
        })
    }

    /// "Update metadata": on the selection when `selection` and games are
    /// selected, else on the games on screen; straight to the running job's
    /// results when one runs.
    pub(super) fn open_metadata_update(
        &mut self,
        selection: bool,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<Entity<MetadataUpdate>> {
        self.context_menu = None;
        self.sel.pop = None;
        let (Some(ctx), Some(mut games)) = (self.dialog_ctx(), self.update_games(selection)) else { return None };
        // Debug: HERONITE_DEBUG_WIZARD_LIMIT=n starts an update of the first
        // n games on screen at once (empty fields, every field and source).
        let limit = crate::env::parse::<usize>("HERONITE_DEBUG_WIZARD_LIMIT");
        if let Some(n) = limit {
            games.shown.truncate(n);
            let args = BulkArgs::update(games.shown.clone(), UpdateMode::Fill, None, None);
            hub(cx).update(cx, |h, cx| h.start(args, cx));
        }
        let view = cx.new(|cx| MetadataUpdate::new(ctx, games, cx));
        self.open_dialog(view.clone(), cx);
        Some(view)
    }

    /// The dialog on the last job's results (the header pill's "See all").
    pub(super) fn open_metadata_results(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(view) = self.open_metadata_update(false, window, cx) {
            view.update(cx, |d, cx| d.show_results(cx));
        }
    }

    /// `HERONITE_DEBUG_DIALOG` values of the metadata dialogs:
    /// `wizard[:<step>[:<state>]]` is "Update metadata" (on the selection
    /// when `HERONITE_DEBUG_MULTI` made one; see `MetadataUpdate::debug_state`);
    /// `metadata` the selected game's "Metadata" (asking the sources),
    /// `metadata:<state>` the same over a made-up preview; `review[:<state>]`
    /// a made-up "Review matches"; `covers[:<state>]` a made-up "Covers".
    pub(super) fn open_metadata_debug(&mut self, which: &str, window: &mut Window, cx: &mut Context<Self>) {
        match which {
            "metadata" => self.open_metadata(window, cx),
            w if w.starts_with("metadata:") => self.open_metadata_fake(&w["metadata:".len()..], window, cx),
            w if w == "review" || w.starts_with("review:") => self.open_review_debug(w, window, cx),
            w if w == "covers" || w.starts_with("covers:") => self.open_covers_debug(w.strip_prefix("covers:").unwrap_or(""), window, cx),
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
            w if w == "wizard" || w.starts_with("wizard:") => {
                let state = w.strip_prefix("wizard:").unwrap_or("").to_string();
                if let Some(view) = self.open_metadata_update(self.sel.active(), window, cx) {
                    if !state.is_empty() {
                        view.update(cx, |d, cx| d.debug_state(&state, cx));
                    }
                }
            }
            _ => {}
        }
    }

    /// `HERONITE_DEBUG_METADATA_JOB`: a made-up job over the games on screen
    /// as soon as the library is shown (see the dialog's `fake.rs`).
    pub(super) fn debug_metadata_job(&mut self, cx: &mut Context<Self>) {
        if !cx.has_global::<HubGlobal>() {
            return;
        }
        if self.debug_aspect_notice(cx) || !hub(cx).read(cx).faking() {
            return;
        }
        let Some(games) = self.update_games(false) else { return };
        if hub(cx).read(cx).fake_idle() {
            hub(cx).update(cx, |h, cx| h.fake_idle_queue(&games.shown, cx));
            return;
        }
        let args = BulkArgs::update(games.shown, UpdateMode::Fill, None, None);
        hub(cx).update(cx, |h, cx| h.start(args, cx));
    }
}

impl CollectionView {
    /// `HERONITE_DEBUG_ASPECT_NOTICE=offer[:wide]|started[:wide]`: the
    /// Cover aspect notice of the square (or wide) family, counted on the
    /// real data; `started` with a made-up download of the games on screen
    /// (only while `HERONITE_DEBUG_METADATA_JOB` makes jobs up). True when
    /// it took the place of the made-up job.
    fn debug_aspect_notice(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(spec) = crate::env::value("HERONITE_DEBUG_ASPECT_NOTICE") else { return false };
        let (what, family) = spec.split_once(':').unwrap_or((spec.as_str(), "square"));
        let family = heronite_data::cover_family::CoverFamily::parse(family).unwrap_or(heronite_data::cover_family::CoverFamily::Square);
        let games = self.update_games(false).map(|g| g.shown).unwrap_or_default();
        self.view_look.open_covers();
        hub(cx).update(cx, |h, cx| {
            h.show_notice(family, cx);
            if what == "started" && h.faking() {
                h.start(BulkArgs::download_covers(games, family), cx);
            }
        });
        true
    }
}

/// A game as a bulk job takes it.
pub(super) fn bulk_item(g: &heronite_data::CollectionGame) -> BulkItem {
    BulkItem {
        app_name: g.app_name.clone(),
        runner: if g.runner == "zoom" { "sideload".into() } else { g.runner.clone() },
        title: g.title.clone(),
        steam_app_id: g.steam_app_id.clone(),
    }
}
