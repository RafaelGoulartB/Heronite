//! Looking for newer versions of the installed mods on their sites.

use std::collections::HashMap;

use gpui_kit::*;
use heronite_i18n::{tr, trn};
use heronite_mods::nexus::{self, NexusFiles};
use heronite_mods::{thunderstore, ModList, ModSource, UpdateInfo};

use super::{ModsEvent, ModsService};

/// What the check found for a mod: `Some(None)` = up to date.
type Found = HashMap<String, Option<UpdateInfo>>;

/// Blocking: asks the sites about every mod of the list.
fn look(list: &ModList, api_key: Option<&str>) -> Found {
    let mut found = Found::new();
    if let Some(key) = api_key {
        let mut pages: HashMap<(String, u64), Option<NexusFiles>> = HashMap::new();
        for m in &list.mods {
            let ModSource::Nexus { domain, mod_id, .. } = &m.source else { continue };
            let files = pages.entry((domain.clone(), *mod_id)).or_insert_with(|| match nexus::files(key, domain, *mod_id) {
                Ok(files) => Some(files),
                Err(e) => {
                    log::debug!("mods: no update check for {domain}/{mod_id}: {e}");
                    None
                }
            });
            if let Some(files) = files {
                found.insert(m.id.clone(), nexus::check_update(files, m));
            }
        }
    }
    for (id, update) in thunderstore::latest_versions(list, &mut |ns, n| thunderstore::package(ns, n)) {
        found.insert(id, Some(update));
    }
    found
}

impl ModsService {
    pub(super) fn look_for_updates(&mut self, game_key: &str, cx: &mut Context<Self>) {
        let (Some(game), Some(shared)) = (self.games.get(game_key), self.shared.get(game_key).cloned()) else { return };
        let has_nexus = game.list.mods.iter().any(|m| matches!(m.source, ModSource::Nexus { .. }));
        let key_of_account = self.account.as_ref().map(|a| a.api_key.clone());
        let key = game_key.to_string();
        if has_nexus && key_of_account.is_none() {
            self.notice(game_key, tr!("Add your Nexus Mods API key in Settings → Mods to check the Nexus Mods updates"), false, cx);
        }
        cx.spawn(async move |this, cx| {
            let list = shared.list();
            let found = cx.background_spawn(async move { look(&list, key_of_account.as_deref()) }).await;
            this.update(cx, |s, cx| {
                let (n, list) = shared.edit(|l| {
                    for (id, update) in found {
                        if let Some(m) = l.get_mut(&id) {
                            m.update = update;
                        }
                    }
                    l.mods.iter().filter(|m| m.update.is_some()).count()
                });
                if let Some(game) = s.games.get_mut(&key) {
                    game.list = list;
                }
                cx.emit(ModsEvent::Changed { game_key: key.clone() });
                s.save_later(&key, cx);
                let message = match n {
                    0 => tr!("All the mods are up to date").to_string(),
                    n => trn!(n, "{n} mod has a newer version", "{n} mods have a newer version"),
                };
                s.notice(&key, message, false, cx);
            })
            .ok();
        })
        .detach();
    }
}

#[cfg(test)]
mod tests {
    use super::look;
    use heronite_mods::{ModEntry, ModList, ModSource};

    #[test]
    fn a_list_without_site_mods_asks_nobody() {
        let mut list = ModList::new("g", "G");
        list.mods.push(ModEntry { id: "a".into(), source: ModSource::File, ..Default::default() });
        assert!(look(&list, Some("key")).is_empty());
        assert!(look(&list, None).is_empty());
    }
}
