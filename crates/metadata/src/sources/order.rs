//! The order sources are asked in, for every field.
//!
//! A field's order is the user's own list (`fieldPriority`) when they set
//! one, else its base order: [`default_field_priority`] while the sources
//! are in the registry's order, and once the user has moved one, the
//! sources with a lookup that provide the field in the user's order (so a
//! move shows in every field they did not set by hand). A pictures source
//! (SteamGridDB) joins a base order only where the default one names it
//! (the square and wide covers): the tall cover and the background keep
//! the details sources they always had. Sources that are off, or have the
//! field switched off, are then left out. A source added after the lists
//! were saved (`ADDED_SOURCES`, VNDB) joins a saved list at its end.

use heronite_data::models::MetadataSettings;
use heronite_data::settings::{default_field_priority, ADDED_SOURCES, ALL_FIELDS};
use heronite_data::source_prefs::{is_field_customised, is_field_on, is_on};
use indexmap::IndexMap;

use super::{SourceDescriptor, SourceKind, SourceRegistry};

impl SourceRegistry {
    fn registered_ids(&self) -> impl Iterator<Item = &str> {
        self.descriptors().map(|d| d.id.as_str())
    }

    /// Every source id in the user's order: the saved `sourceOrder` (with the
    /// ids this build does not know, in place), then the registered ones it
    /// lacks, in registry order.
    pub fn order(&self, m: &MetadataSettings) -> Vec<String> {
        let mut out = m.source_order.clone();
        for id in self.registered_ids() {
            if !out.iter().any(|o| o == id) {
                out.push(id.to_owned());
            }
        }
        out
    }

    /// Whether the user moved a registered source away from the registry's order.
    pub fn is_order_customised(&self, m: &MetadataSettings) -> bool {
        let order = self.order(m);
        !order.iter().filter(|id| self.get(id).is_some()).map(String::as_str).eq(self.registered_ids())
    }

    /// The registered sources of `kind` in the user's order.
    pub fn ordered(&self, m: &MetadataSettings, kind: SourceKind) -> Vec<&SourceDescriptor> {
        self.order(m).iter().filter_map(|id| self.get(id)).filter(|d| d.kind == kind).collect()
    }

    /// The order of `field` when the user did not set one (see the module docs).
    pub fn base_order(&self, m: &MetadataSettings, field: &str) -> Vec<String> {
        if !self.is_order_customised(m) {
            return default_field_priority(field);
        }
        let default = default_field_priority(field);
        let joins = |d: &SourceDescriptor| d.kind != SourceKind::Pictures || default.contains(&d.id);
        self.order(m).into_iter().filter(|id| self.has_lookup(id) && self.get(id).is_some_and(|d| d.provides(field) && joins(d))).collect()
    }

    /// The user's list for `field` when set, else [`Self::base_order`],
    /// before leaving out the sources that are off. A list set before one of
    /// the [`ADDED_SOURCES`] existed ends with it, where the field's default
    /// order names it.
    fn field_list(&self, m: &MetadataSettings, field: &str) -> Vec<String> {
        match m.field_priority.get(field) {
            Some(list) if is_field_customised(m, field) => {
                let mut list = list.clone();
                let default = default_field_priority(field);
                for id in ADDED_SOURCES.iter().filter(|id| default.iter().any(|d| d == *id)) {
                    if !list.iter().any(|l| l == id) {
                        list.push(id.to_string());
                    }
                }
                list
            }
            _ => self.base_order(m, field),
        }
    }

    /// The sources the merge takes `field` from, first to last. An id this
    /// build does not know stays in a list the user set (it never answers).
    pub fn field_order(&self, m: &MetadataSettings, field: &str) -> Vec<String> {
        let mut list = self.field_list(m, field);
        list.retain(|id| is_on(m, id) && is_field_on(m, id, field));
        list
    }

    /// [`Self::field_order`] of every `ALL_FIELDS` entry.
    pub fn field_orders(&self, m: &MetadataSettings) -> IndexMap<String, Vec<String>> {
        ALL_FIELDS.iter().map(|f| (f.to_string(), self.field_order(m, f))).collect()
    }

    /// The `sourceOrder` to save after moving `id` to place `to` among the
    /// visible sources of its kind (the rows Settings shows); the others
    /// keep their places. Empty when that is the registry's order again.
    /// `None` when `id` is not a visible registered source.
    pub(crate) fn order_after_move(&self, m: &MetadataSettings, id: &str, to: usize) -> Option<Vec<String>> {
        let kind = self.get(id).filter(|d| !d.hidden)?.kind;
        let mut order = self.order(m);
        let slots: Vec<usize> = (0..order.len()).filter(|i| self.get(&order[*i]).is_some_and(|d| d.kind == kind && !d.hidden)).collect();
        let mut group: Vec<String> = slots.iter().map(|i| order[*i].clone()).collect();
        let from = group.iter().position(|g| g == id)?;
        let moved = group.remove(from);
        group.insert(to.min(group.len()), moved);
        for (slot, gid) in slots.into_iter().zip(group) {
            order[slot] = gid;
        }
        let only_registered = order.iter().all(|id| self.get(id).is_some());
        let default = order.iter().map(String::as_str).eq(self.registered_ids());
        Some(if only_registered && default { Vec::new() } else { order })
    }

    /// The `fieldPriority` list to save when the user orders `field` as
    /// `order`: that order, then the sources of the field's current list it
    /// leaves out (the ones that are off now keep their place for later).
    pub(crate) fn field_list_to_save(&self, m: &MetadataSettings, field: &str, order: &[String]) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for id in order.iter().cloned().chain(self.field_list(m, field)) {
            if !out.contains(&id) {
                out.push(id);
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use heronite_data::settings::{normalize_metadata, METADATA_FIELDS};
    use heronite_data::source_prefs::{reset_field, set_field_on, set_on};

    use super::*;

    fn untouched() -> MetadataSettings {
        normalize_metadata(None)
    }

    fn ids(list: &[&SourceDescriptor]) -> Vec<String> {
        list.iter().map(|d| d.id.clone()).collect()
    }

    /// Applies a move the way the service saves it.
    fn moved(reg: &SourceRegistry, m: &mut MetadataSettings, id: &str, to: usize) {
        m.source_order = reg.order_after_move(m, id, to).unwrap();
        *m = normalize_metadata(Some(m));
    }

    #[test]
    fn untouched_settings_give_todays_default_order_for_every_field() {
        let reg = SourceRegistry::for_tests();
        for m in [untouched(), normalize_metadata(Some(&MetadataSettings::default()))] {
            assert!(!reg.is_order_customised(&m));
            for field in ALL_FIELDS {
                assert_eq!(reg.field_order(&m, field), default_field_priority(field), "{field}");
                assert_eq!(reg.base_order(&m, field), default_field_priority(field), "{field}");
            }
            assert_eq!(reg.field_orders(&m), m.field_priority, "what the merge used before");
        }
        assert_eq!(reg.order(&untouched()), reg.descriptors().map(|d| d.id.clone()).collect::<Vec<_>>());
    }

    #[test]
    fn a_move_shows_in_every_field_the_user_did_not_set() {
        let reg = SourceRegistry::for_tests();
        let mut m = untouched();
        m.field_priority.insert("genres".into(), vec!["steam".into(), "igdb".into()]);
        moved(&reg, &mut m, "lutris", 0);
        assert!(reg.is_order_customised(&m));
        assert_eq!(ids(&reg.ordered(&m, SourceKind::Details)), ["lutris", "igdb", "steam", "store", "playnite", "vndb"]);
        for field in METADATA_FIELDS {
            let order = reg.field_order(&m, field);
            if field == "genres" {
                assert_eq!(order, ["steam", "igdb", "vndb"], "a field the user set keeps its order, VNDB joins its end");
            } else if reg.get("lutris").unwrap().provides(field) {
                assert_eq!(order.first().map(String::as_str), Some("lutris"), "{field}");
            } else {
                assert!(!order.contains(&"lutris".to_string()), "{field}: lutris does not fill it");
            }
            // Only details sources that fill the field, in the user's order.
            let base = reg.base_order(&m, field);
            assert!(base.iter().all(|id| reg.has_lookup(id) && reg.get(id).unwrap().provides(field)), "{field}");
        }
        assert_eq!(reg.field_order(&m, "cover"), ["lutris", "igdb", "steam", "store", "playnite", "vndb"]);
        assert_eq!(reg.field_order(&m, "gameModes"), ["igdb"]);
        // SteamGridDB joins only the families whose default names it.
        assert_eq!(reg.field_order(&m, "hero"), ["lutris", "igdb", "steam", "store", "playnite", "vndb"]);
        assert_eq!(reg.field_order(&m, "coverWide"), ["steam", "store", "steamgriddb"]);
        assert_eq!(reg.field_order(&m, "coverSquare"), ["steamgriddb"]);

        // Pictures and extras keep their own places; moving back is the default again.
        assert_eq!(m.source_order.len(), reg.descriptors().count());
        moved(&reg, &mut m, "lutris", 3);
        assert!(m.source_order.is_empty() && !reg.is_order_customised(&m));
        assert_eq!(reg.field_order(&m, "cover"), default_field_priority("cover"));
        // A hidden or unknown source cannot be moved.
        assert!(reg.order_after_move(&m, "playnite", 0).is_none());
        assert!(reg.order_after_move(&m, "elsewhere", 0).is_none());
    }

    #[test]
    fn moves_stay_within_a_kind_and_keep_unknown_ids() {
        let reg = SourceRegistry::for_tests();
        let mut m = untouched();
        m.source_order = vec!["elsewhere".into(), "igdb".into()];
        assert!(!reg.is_order_customised(&m), "an unknown id alone is not a move");
        moved(&reg, &mut m, "webimages", 0);
        assert_eq!(ids(&reg.ordered(&m, SourceKind::Pictures)), ["webimages", "steamgriddb"]);
        assert_eq!(ids(&reg.ordered(&m, SourceKind::Details)), ["igdb", "steam", "store", "lutris", "playnite", "vndb"]);
        assert_eq!(m.source_order.first().map(String::as_str), Some("elsewhere"));
        // Back in order, the unknown id still keeps the list.
        moved(&reg, &mut m, "webimages", 1);
        assert_eq!(m.source_order.first().map(String::as_str), Some("elsewhere"));
        assert!(!reg.is_order_customised(&m));
        // Past the end: last.
        moved(&reg, &mut m, "igdb", 99);
        assert_eq!(ids(&reg.ordered(&m, SourceKind::Details)), ["steam", "store", "lutris", "vndb", "playnite", "igdb"]);
    }

    #[test]
    fn sources_off_and_fields_off_are_left_out() {
        let reg = SourceRegistry::for_tests();
        let mut m = untouched();
        set_on(&mut m, "igdb", false);
        set_field_on(&mut m, "steam", "description", false);
        assert_eq!(reg.field_order(&m, "description"), ["store", "lutris", "playnite", "vndb"]);
        assert_eq!(reg.field_order(&m, "releaseDate"), ["steam", "store", "lutris", "playnite", "vndb"]);
        assert!(reg.field_order(&m, "gameModes").is_empty());
        // Also from a list the user set, which keeps them for later.
        m.field_priority.insert("description".into(), vec!["steam".into(), "igdb".into(), "elsewhere".into()]);
        assert_eq!(reg.field_order(&m, "description"), ["elsewhere", "vndb"], "an unknown id stays and never answers");
        assert_eq!(m.field_priority["description"], ["steam", "igdb", "elsewhere"]);
    }

    #[test]
    fn a_field_set_by_hand_keeps_the_sources_it_does_not_name() {
        let reg = SourceRegistry::for_tests();
        let mut m = untouched();
        set_on(&mut m, "store", false);
        let shown = reg.field_order(&m, "cover");
        assert_eq!(shown, ["igdb", "steam", "lutris", "playnite", "vndb"]);
        let list = reg.field_list_to_save(&m, "cover", &["lutris".into(), "steam".into()]);
        assert_eq!(list, ["lutris", "steam", "igdb", "playnite", "store", "vndb"]);
        m.field_priority.insert("cover".into(), list);
        assert!(is_field_customised(&m, "cover"));
        assert_eq!(reg.field_order(&m, "cover"), ["lutris", "steam", "igdb", "playnite", "vndb"]);
        set_on(&mut m, "store", true);
        assert_eq!(reg.field_order(&m, "cover"), ["lutris", "steam", "igdb", "playnite", "store", "vndb"]);
        reset_field(&mut m, "cover");
        assert!(!is_field_customised(&m, "cover"));
        assert_eq!(reg.field_order(&m, "cover"), default_field_priority("cover"));
    }

    #[test]
    fn a_list_saved_before_vndb_still_follows_the_default_and_gets_it() {
        let reg = SourceRegistry::for_tests();
        let mut m = untouched();
        // What every settings file saved before VNDB: the default of then.
        m.field_priority.insert("cover".into(), ["igdb", "steam", "lutris", "playnite", "store"].map(String::from).to_vec());
        assert!(!is_field_customised(&m, "cover"));
        assert_eq!(reg.field_order(&m, "cover"), default_field_priority("cover"));
        // A list the user set then: VNDB joins its end, and only where it fills.
        m.field_priority.insert("cover".into(), ["store", "igdb"].map(String::from).to_vec());
        assert_eq!(reg.field_order(&m, "cover"), ["store", "igdb", "vndb"]);
        m.field_priority.insert("series".into(), ["playnite", "igdb"].map(String::from).to_vec());
        assert_eq!(reg.field_order(&m, "series"), ["playnite", "igdb"]);
        // Placed by the user: kept where it is.
        m.field_priority.insert("cover".into(), ["vndb", "igdb"].map(String::from).to_vec());
        assert_eq!(reg.field_order(&m, "cover"), ["vndb", "igdb"]);
    }
}
