//! Game tab state: the three link fields, kept in step with the service and
//! saved when one of them loses focus or gets Enter.

use gpui_kit::component::input::InputState;
use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_i18n::tr;
use heronite_mods::{links, GameLinks};

use super::ModsDialog;
use crate::ui::fields;

pub(super) struct LinksForm {
    pub nexus: Entity<InputState>,
    pub store: Entity<InputState>,
    pub root: Entity<InputState>,
    /// The fields hold the loaded list's links.
    synced: bool,
    /// The Nexus game's name on the site, once looked up.
    pub nexus_name: Option<String>,
    /// The Thunderstore community's name and icon URL.
    pub store_name: Option<String>,
    pub store_icon: Option<String>,
}

impl LinksForm {
    pub fn new(window: &mut Window, cx: &mut Context<ModsDialog>) -> Self {
        let make = |placeholder: &str, window: &mut Window, cx: &mut Context<ModsDialog>| {
            let input = fields::new_input(window, cx, "", placeholder);
            fields::on_commit(&input, window, cx, |this, _, _, cx| this.commit_links(cx));
            input
        };
        Self {
            nexus: make("skyrimspecialedition", window, cx),
            store: make("lethal-company", window, cx),
            root: make(tr!("The game folder"), window, cx),
            synced: false,
            nexus_name: None,
            store_name: None,
            store_icon: None,
        }
    }
}

fn opt(s: String) -> Option<String> {
    (!s.is_empty()).then_some(s)
}

impl ModsDialog {
    /// Puts the loaded list's links in the fields, once.
    pub(super) fn sync_links_form(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.links.synced {
            return;
        }
        let Some(v) = self.snapshot(cx) else { return };
        self.links.synced = true;
        let root_hint =
            if v.g.info.profile.mod_root.is_empty() { tr!("The game folder").to_string() } else { v.g.info.profile.mod_root.clone() };
        let values = [
            (&self.links.nexus, v.g.info.nexus_domain.clone().unwrap_or_default(), None),
            (&self.links.store, v.g.info.thunderstore.clone().unwrap_or_default(), None),
            (&self.links.root, v.g.list.links.mod_root.clone().unwrap_or_default(), Some(root_hint)),
        ];
        for (input, value, hint) in values {
            input.update(cx, |s, cx| {
                s.set_value(value, window, cx);
                if let Some(h) = hint {
                    s.set_placeholder(h, window, cx);
                }
            });
        }
        self.look_up_names(cx);
    }

    /// Saves the links the user typed (nothing happens when they match what the list already uses).
    pub(super) fn commit_links(&mut self, cx: &mut Context<Self>) {
        let Some(v) = self.snapshot(cx) else { return };
        let read = |input: &Entity<InputState>| input.read(cx).value().trim().to_string();
        let (nexus, store, root) = (read(&self.links.nexus), read(&self.links.store), read(&self.links.root));
        let effective_nexus = v.g.info.nexus_domain.clone().unwrap_or_default();
        let effective_store = v.g.info.thunderstore.clone().unwrap_or_default();
        let current_root = v.g.list.links.mod_root.clone().unwrap_or_default();
        if nexus == effective_nexus && store == effective_store && root.trim_matches('/') == current_root {
            return;
        }
        let links = GameLinks { nexus_domain: opt(nexus), thunderstore: opt(store), mod_root: opt(root.trim_matches('/').to_string()) };
        let key = self.key.clone();
        self.service.update(cx, |s, cx| s.set_links(&key, links, cx));
        self.links.nexus_name = None;
        self.links.store_name = None;
        self.links.store_icon = None;
        self.browse.reset_results();
        self.look_up_names(cx);
        cx.notify();
    }

    /// The names the sites give the linked game and community (off the UI thread).
    pub(super) fn look_up_names(&mut self, cx: &mut Context<Self>) {
        let nexus = opt(self.links.nexus.read(cx).value().trim().to_string());
        let store = opt(self.links.store.read(cx).value().trim().to_string());
        let cache = self.service.read(cx).root().cache_dir();
        cx.spawn(async move |this, cx| {
            let (name, art) = cx
                .background_spawn(async move {
                    (nexus.and_then(|d| links::nexus_game_name(&cache, &d)), store.and_then(|c| links::community_art(&c)))
                })
                .await;
            this.update(cx, |this, cx| {
                this.links.nexus_name = name;
                if let Some(art) = art {
                    this.links.store_name = Some(art.name).filter(|n| !n.is_empty());
                    this.links.store_icon = art.icon;
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }
}
