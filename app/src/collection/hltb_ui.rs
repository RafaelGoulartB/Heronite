//! HowLongToBeat in the Collection: the times of the selected game are looked
//! up in the background (cache first), and the buttons of the "How Long To
//! Beat" section (page, "Change match…", "Refresh").

use std::sync::Arc;

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::hltb::WikiCache;
use heronite_i18n::tr;
use heronite_metadata::hltb::HltbQuery;

use super::focus::HltbAction;
use super::CollectionView;
use crate::dialogs::hltb_match::HltbMatchDialog;
use crate::dialogs::metadata_wizard::{hub, HubGlobal};

impl CollectionView {
    /// What HowLongToBeat is asked about the selected game.
    fn hltb_query(&self) -> Option<HltbQuery> {
        let vm = self.focus.as_ref()?;
        let game = self.lib.as_ref()?.get(&vm.key)?;
        Some(HltbQuery {
            title: game.title.trim().to_owned(),
            store_title: Some(game.game.title.trim().to_owned()).filter(|t| !t.is_empty() && *t != game.title.trim()),
            year: vm.release_year,
        })
    }

    /// Looks up the selected game once per session: served from the cache
    /// when fresh, else asked in the background; the section shows the
    /// answer when it arrives.
    pub(super) fn ensure_hltb_for_focus(&mut self, cx: &mut Context<Self>) {
        let Some(key) = self.focus.as_ref().map(|vm| vm.key.to_string()) else { return };
        if self.hltb_tried.contains(&key) || !cx.has_global::<HubGlobal>() {
            return;
        }
        if !self.lib.as_ref().is_some_and(|lib| lib.settings.metadata.how_long_to_beat) {
            return;
        }
        let Some(query) = self.hltb_query().filter(|q| !q.title.is_empty()) else { return };
        self.hltb_tried.insert(key);
        self.run_hltb(query, false, cx);
    }

    fn run_hltb(&mut self, query: HltbQuery, refresh: bool, cx: &mut Context<Self>) {
        let service = hub(cx).read(cx).service.clone();
        cx.spawn(async move |this, cx| {
            let res = cx
                .background_spawn(async move {
                    if refresh {
                        service.refresh_how_long_to_beat(&query)
                    } else {
                        service.how_long_to_beat_for(&query)
                    }
                })
                .await;
            this.update(cx, |this, cx| match res {
                Ok(found) => {
                    if !found.from_cache {
                        this.reload_wiki(cx);
                    }
                    if refresh {
                        let text = if found.entry.is_some() {
                            tr!("HowLongToBeat times updated.")
                        } else {
                            tr!("HowLongToBeat has no times for this game.")
                        };
                        this.show_toast(text, false, cx);
                    }
                }
                Err(err) => {
                    log::info!("HowLongToBeat: {err:#}");
                    if refresh {
                        this.show_toast(tr!("Couldn't reach HowLongToBeat. Try again later."), true, cx);
                    }
                }
            })
            .ok();
        })
        .detach();
    }

    /// Reads `wikigameinfo.json` again (new HowLongToBeat times).
    fn reload_wiki(&mut self, cx: &mut Context<Self>) {
        self.wiki = Arc::new(WikiCache::load(&self.data_paths));
        self.group_members.get_mut().clear();
        self.refresh_focus();
        cx.notify();
    }

    pub(super) fn on_hltb_action(&mut self, action: HltbAction, window: &mut Window, cx: &mut Context<Self>) {
        match action {
            HltbAction::Open(link) => cx.open_url(&link),
            HltbAction::Refresh => {
                if let Some(query) = self.hltb_query() {
                    self.run_hltb(query, true, cx);
                }
            }
            HltbAction::ChangeMatch => self.open_hltb_match(window, cx),
        }
    }

    /// "Change match…" of the selected game.
    pub(super) fn open_hltb_match(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(query) = self.hltb_query() else { return };
        if !cx.has_global::<HubGlobal>() {
            return;
        }
        let current = self.focus.as_ref().and_then(|vm| vm.hltb_game_id);
        let view = cx.new(|cx| HltbMatchDialog::new(query.title, current, window, cx));
        self.open_dialog(view, cx);
    }
}
