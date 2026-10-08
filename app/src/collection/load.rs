//! Loading the library off the UI thread and applying it to the view.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::hltb::WikiCache;
use heronite_data::{art, ArtContext, HeroicPaths, Library};
use heronite_images::ImageRef;

use super::{stage, CollectionView};
use crate::images::to_chain;

/// Everything loaded from disk off the UI thread.
struct Loaded {
    lib: Library,
    wiki: WikiCache,
    covers: HashMap<SharedString, Vec<ImageRef>>,
    stages: HashMap<SharedString, Vec<ImageRef>>,
}

fn load_all(heroic: &HeroicPaths, include_hidden: bool) -> anyhow::Result<Loaded> {
    let t = Instant::now();
    let lib = Library::load_with(heroic, include_hidden)?;
    let wiki = WikiCache::load(heroic);
    let shots = (!lib.settings.screenshots.folder.is_empty())
        .then(|| std::path::PathBuf::from(&lib.settings.screenshots.folder));
    let ctx = ArtContext::new(heroic, shots.as_deref());
    let mut covers = HashMap::new();
    let mut stages = HashMap::new();
    for g in &lib.games {
        let key: SharedString = g.key.clone().into();
        covers.insert(key.clone(), to_chain(&art::card_cover(&ctx, g)));
        stages.insert(key, to_chain(&art::stage_art(&ctx, g)));
    }
    log::info!("library: {} games loaded in {:?}", lib.games.len(), t.elapsed());
    Ok(Loaded { lib, wiki, covers, stages })
}

impl CollectionView {
    pub(super) fn spawn_load(&self, cx: &mut Context<Self>) {
        let heroic = self.heroic.clone();
        let include_hidden = self.ui.show_hidden;
        cx.spawn(async move |this, cx| {
            let loaded = cx.background_spawn(async move { load_all(&heroic, include_hidden) }).await;
            this.update(cx, |this, cx| {
                this.refreshing = false;
                match loaded {
                    Ok(l) => this.apply_loaded(l),
                    Err(err) => log::error!("loading library: {err:#}"),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub(super) fn reload(&mut self, cx: &mut Context<Self>) {
        if self.refreshing {
            return;
        }
        self.refreshing = true;
        self.images.clear_failed();
        self.spawn_load(cx);
        cx.notify();
    }

    fn apply_loaded(&mut self, l: Loaded) {
        let first = self.lib.is_none();
        let s = &l.lib.settings;
        let bg = &s.background;
        self.scrim = stage::Scrim { dim: bg.dimming.get() as f32 / 100., scrim: bg.scrim.get() as f32 / 100. };
        self.blur_px = bg.blur.get() as f32;
        self.saturation = bg.saturation.get() as f32 / 100.;
        self.stage_enabled = bg.enabled;
        self.grey_uninstalled = s.grey_uninstalled_games && !s.all_tiles_in_color;
        self.cover_chains = l.covers;
        self.stage_chains = l.stages;
        self.wiki = Arc::new(l.wiki);
        self.lib = Some(Arc::new(l.lib));
        self.refresh_groups();
        let focused = if first {
            crate::env::value("HERONITE_DEBUG_SELECT").or_else(|| self.ui.focused_game.clone())
        } else {
            self.selected.as_ref().map(|s| s.to_string())
        };
        if let Some(key) = focused {
            self.select(key.into(), false);
            if first {
                self.reveal_selected();
            }
        }
        if first {
            log::info!("library on screen after {:?}", self.started.elapsed());
            self.apply_debug_env();
            crate::trim_heap_later();
        }
    }
}
