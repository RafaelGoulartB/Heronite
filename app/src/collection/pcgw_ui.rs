//! PCGamingWiki in the Collection: the PC facts of the selected game are
//! looked up in the background (cache first, one request a second), and the
//! buttons of the "PC" section.

use std::sync::Arc;

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::pcgw::{PcgwCache, PcgwQuery};
use heronite_i18n::tr;

use super::focus::PcAction;
use super::CollectionView;
use crate::dialogs::metadata_wizard::{hub, HubGlobal};

impl CollectionView {
    /// Looks up the selected game once per session (the cache when fresh).
    pub(super) fn ensure_pcgw_for_focus(&mut self, cx: &mut Context<Self>) {
        let Some(key) = self.focus.as_ref().map(|vm| vm.key.to_string()) else { return };
        if self.pcgw_tried.contains(&key) || !cx.has_global::<HubGlobal>() {
            return;
        }
        let Some(lib) = self.lib.clone().filter(|lib| lib.settings.metadata.pc_gaming_wiki) else { return };
        let Some(game) = lib.get(&key) else { return };
        let query = PcgwQuery::for_game(game);
        if query.keys().is_empty() {
            return;
        }
        self.pcgw_tried.insert(key);
        let service = hub(cx).read(cx).service.clone();
        cx.spawn(async move |this, cx| {
            let res = cx.background_spawn(async move { service.pc_gaming_wiki(&query) }).await;
            this.update(cx, |this, cx| match res {
                Ok(found) if !found.from_cache => {
                    this.pcgw = Arc::new(PcgwCache::load(&this.data_paths));
                    this.refresh_focus();
                    cx.notify();
                }
                Ok(_) => {}
                Err(err) => log::info!("PCGamingWiki: {err:#}"),
            })
            .ok();
        })
        .detach();
    }

    pub(super) fn on_pc_action(&mut self, action: PcAction, cx: &mut Context<Self>) {
        match action {
            PcAction::Open(url) => cx.open_url(&url),
            PcAction::Copy(text) => {
                cx.write_to_clipboard(ClipboardItem::new_string(text.to_string()));
                self.show_toast(tr!("Save folder copied."), false, cx);
            }
        }
    }
}
