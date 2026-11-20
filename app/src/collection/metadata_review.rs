//! Opening "Review matches" (`dialogs/metadata_review`) from the
//! Collection: on the whole queue (the header pill, the palette, Settings →
//! Metadata), on a job's games sent to review (its results), or on the
//! selection ("Fix matches…": the queued ones first, the others asked
//! live). Also the debug openers of the review and of the one-game dialog's
//! made-up states.

use std::collections::HashMap;

use gpui_kit::*;
use heronite_data::library::metadata_key;
use heronite_metadata::{BulkItem, BulkProgress, GameOutcome};

use super::metadata_ui::bulk_item;
use super::CollectionView;
use crate::dialogs::metadata::{origin, MetadataDialog};
use crate::dialogs::metadata_review::{MetadataReview, ReviewGame};
use crate::dialogs::metadata_update::hub::{hub, HubGlobal, ReviewRequest};

/// Covers the made-up proposals show (debug only).
const FAKE_THUMBS: usize = 6;

impl CollectionView {
    /// A game of the review, with its cover and where it comes from.
    fn review_game(&self, item: BulkItem, queued: &dyn Fn(&str) -> bool) -> ReviewGame {
        let runner = if item.runner == "zoom" { "sideload" } else { item.runner.as_str() };
        let key = metadata_key(runner, &item.app_name);
        let ckey = self.collection_key(runner, &item.app_name);
        let game = self.lib.as_ref().and_then(|l| l.get(&ckey));
        let mut item = item.clone();
        if item.steam_app_id.is_none() {
            item.steam_app_id = game.and_then(|g| g.steam_app_id.clone());
        }
        ReviewGame {
            queued: queued(&key),
            cover: self.cover_chains.get(ckey.as_ref()).cloned().unwrap_or_default(),
            origin: game.map(|g| origin(g)).unwrap_or_default(),
            key,
            item,
        }
    }

    /// Addresses of covers the Collection has, for made-up proposals.
    fn fake_thumbs(&self) -> Vec<String> {
        let Some(lib) = self.lib.as_ref() else { return Vec::new() };
        lib.games
            .iter()
            .filter_map(|g| g.metadata.as_ref().and_then(|m| m.cover_url.clone()))
            .filter(|u| u.starts_with("http"))
            .take(FAKE_THUMBS)
            .collect()
    }

    /// "Review matches" on `request`.
    pub(super) fn open_review(&mut self, request: ReviewRequest, window: &mut Window, cx: &mut Context<Self>) {
        self.open_review_with(request, None, window, cx);
    }

    fn open_review_with(&mut self, request: ReviewRequest, state: Option<&str>, window: &mut Window, cx: &mut Context<Self>) {
        if !cx.has_global::<HubGlobal>() {
            return;
        }
        let Some(ctx) = self.dialog_ctx() else { return };
        self.context_menu = None;
        self.sel.pop = None;
        let hub_entity = hub(cx);
        let queue: HashMap<String, BulkItem> = hub_entity
            .read(cx)
            .queue
            .iter()
            .map(|q| {
                let item = BulkItem { app_name: q.app_name.clone(), runner: q.runner.clone(), title: q.title.clone(), steam_app_id: None };
                (q.key.clone(), item)
            })
            .collect();
        let items = request.games.clone().unwrap_or_else(|| hub_entity.read(cx).queue.iter().map(|q| queue[&q.key].clone()).collect());
        let is_queued = |key: &str| queue.contains_key(key);
        let games: Vec<ReviewGame> = items.into_iter().map(|item| self.review_game(item, &is_queued)).collect();
        let fake = (hub_entity.read(cx).faking() || state.is_some()).then(|| self.fake_thumbs());
        hub_entity.update(cx, |h, cx| h.review_seen(cx));
        let start = request.start;
        let view = cx.new(|cx| MetadataReview::new(ctx, games, start, fake, window, cx));
        if let Some(state) = state.filter(|s| !s.is_empty()) {
            view.update(cx, |d, cx| d.debug_state(state, window, cx));
        }
        self.open_dialog(view, cx);
        cx.notify();
    }

    /// "Fix matches…" on the selection: the games waiting for review first,
    /// then the others (whose proposals are asked live).
    pub(super) fn fix_selection_matches(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !cx.has_global::<HubGlobal>() {
            return;
        }
        let items: Vec<BulkItem> = self.selected_games().iter().map(|g| bulk_item(g)).collect();
        let h = hub(cx);
        let queued = |g: &BulkItem| h.read(cx).queued(&metadata_key(&g.runner, &g.app_name));
        let (mut first, rest): (Vec<_>, Vec<_>) = items.into_iter().partition(queued);
        first.extend(rest);
        self.open_review(ReviewRequest { games: Some(first), start: 0 }, window, cx);
    }

    /// The games `p` sent to review, as a job takes them.
    pub(super) fn review_items_of(&self, p: &BulkProgress) -> Vec<BulkItem> {
        p.results
            .iter()
            .filter(|r| matches!(r.outcome, GameOutcome::ToReview(_)))
            .map(|r| {
                let game = self.lib.as_ref().and_then(|l| l.get(&self.collection_key(&r.runner, &r.app_name)));
                BulkItem {
                    app_name: r.app_name.clone(),
                    runner: r.runner.clone(),
                    title: r.title.clone(),
                    steam_app_id: game.and_then(|g| g.steam_app_id.clone()),
                }
            })
            .collect()
    }

    /// `HERONITE_DEBUG_DIALOG=review[:<state>]`: a made-up review of the
    /// selection (`HERONITE_DEBUG_MULTI`), else of the first three games on
    /// screen. States: see `MetadataReview::debug_state`.
    pub(super) fn open_review_debug(&mut self, which: &str, window: &mut Window, cx: &mut Context<Self>) {
        let state = which.strip_prefix("review").unwrap_or("").trim_start_matches(':').to_string();
        let mut items: Vec<BulkItem> = self.selected_games().iter().map(|g| bulk_item(g)).collect();
        if items.is_empty() {
            let shown = self.shown_keys();
            items = shown.iter().take(3).filter_map(|k| self.lib.as_ref().and_then(|l| l.get(k))).map(|g| bulk_item(g)).collect();
        }
        if state == "empty" {
            items.truncate(1);
        }
        self.open_review_with(ReviewRequest { games: Some(items), start: 0 }, Some(&state), window, cx);
    }

    /// `HERONITE_DEBUG_DIALOG=metadata:<state>`: the selected game's
    /// "Metadata" over a made-up preview (see the dialog's `fake.rs`).
    pub(super) fn open_metadata_fake(&mut self, state: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(key) = self.selected.clone() else { return };
        let (Some(ctx), Some(game)) = (self.dialog_ctx(), self.game_for(&key)) else { return };
        let thumbs = self.fake_thumbs();
        let state = state.to_string();
        let view = cx.new(|cx| MetadataDialog::made_up(ctx, game, &state, thumbs, window, cx));
        self.open_dialog(view, cx);
    }
}
