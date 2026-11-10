//! Metadata sources: what each one is ([`SourceDescriptor`]), how a details
//! source is asked about a game ([`MetadataSource`]) and the built-in ones in
//! one place ([`SourceRegistry`]).
//!
//! Details sources have a lookup and feed the merge. A source may be asked
//! only as a fallback ([`Asked`]; the rule is `service/gather`'s), and may
//! flag the pictures it gives as adult: they are dropped unless the user
//! allows adult covers (`settings.metadata.allowAdultCovers`). Pictures and extras
//! sources (SteamGridDB, web images, HowLongToBeat, Steam tags, ProtonDB) are
//! registered as descriptors only: they still run on their own paths, which
//! ask [`is_on`] first.
//!
//! What the user set per source (on or off, options, order, fields switched
//! off) is stored by `heronite_data::source_prefs`; [`options`] reads and
//! writes the options, [`order`] gives the order of each field and
//! [`rows`] the read model of Settings → Metadata.

mod builtin;
mod descriptor;
pub mod options;
pub mod order;
pub mod rows;
#[cfg(test)]
mod tests;

use std::sync::OnceLock;

use heronite_data::models::MetadataSettings;
use heronite_data::paths::DataPaths;
use heronite_data::settings;

pub(crate) use builtin::Shared;
pub use builtin::{builtin_descriptors, offered};
pub use descriptor::{extra_fields, AppliesTo, Asked, GameKind, Needs, SourceDescriptor, SourceGame, SourceKind};
pub use heronite_data::source_prefs::{ids, is_field_on, is_on};
pub use options::{OptionChoice, OptionKind, OptionValue, SourceOption};
pub use rows::{FieldRow, OptionRow, SourceRow, SourceStatus};

use crate::http::Http;
use crate::types::{IgdbCredentialTest, Lookup, MetadataCandidate, SourceHit};

/// The outcome of a source's test (IGDB's credentials check today).
pub type SourceTest = IgdbCredentialTest;

/// What a lookup may use besides the game. One per gather, shared by the
/// lookups running at once.
pub struct SourceContext<'a> {
    pub paths: &'a DataPaths,
    pub http: &'a Http,
    settings: OnceLock<MetadataSettings>,
}

impl<'a> SourceContext<'a> {
    pub fn new(paths: &'a DataPaths, http: &'a Http) -> Self {
        Self { paths, http, settings: OnceLock::new() }
    }

    /// The effective `settings.metadata` (credentials, options), read on
    /// first use.
    pub fn settings(&self) -> &MetadataSettings {
        self.settings.get_or_init(|| load_metadata_settings(self.paths))
    }
}

/// Effective (normalized) `settings.metadata`.
pub(crate) fn load_metadata_settings(paths: &DataPaths) -> MetadataSettings {
    settings::load(paths).map(|s| s.metadata).unwrap_or_else(|_| settings::normalize_metadata(None))
}

/// A source that can be asked about one game. Lookups of different sources
/// run at the same time on their own threads; one source may also be asked
/// about several games at once.
pub trait MetadataSource: Send + Sync {
    fn descriptor(&self) -> &SourceDescriptor;

    /// Only called for games the descriptor's `applies_to` matches.
    /// [`Lookup::Skipped`] when it cannot be asked (no credentials, no id).
    fn lookup(&self, ctx: &SourceContext<'_>, game: &SourceGame) -> Lookup<MetadataCandidate>;

    /// Its game `id` (a search hit's, or one the user pinned), as the
    /// candidate for `game` ([`MatchedBy::SourceId`](crate::types::MatchedBy)).
    /// Only for sources whose descriptor is `searchable`.
    fn lookup_id(&self, _ctx: &SourceContext<'_>, _game: &SourceGame, _id: &str) -> Lookup<MetadataCandidate> {
        Lookup::Skipped
    }

    /// Its games whose name is like `query`, best first. Only for sources
    /// whose descriptor is `searchable`; [`Lookup::Skipped`] when it cannot
    /// be asked (no key).
    fn search(&self, _ctx: &SourceContext<'_>, _query: &str) -> Lookup<Vec<SourceHit>> {
        Lookup::Skipped
    }

    /// Whether [`test`](Self::test) does something.
    fn has_test(&self) -> bool {
        false
    }

    /// Checks that the source can be asked with what the user set (keys,
    /// sign-in). Blocking. `None` when it has no test.
    fn test(&self, _ctx: &SourceContext<'_>) -> Option<SourceTest> {
        None
    }
}

enum Entry {
    Lookup(Box<dyn MetadataSource>),
    /// Known (listed, has settings) but fetched elsewhere.
    DescriptorOnly(Box<SourceDescriptor>),
}

impl Entry {
    fn descriptor(&self) -> &SourceDescriptor {
        match self {
            Self::Lookup(source) => source.descriptor(),
            Self::DescriptorOnly(d) => d,
        }
    }
}

/// The sources, in the order their candidates are listed and merged.
pub struct SourceRegistry {
    entries: Vec<Entry>,
}

impl SourceRegistry {
    /// The built-in sources over the service's shared state.
    pub(crate) fn builtin(shared: Shared) -> Self {
        builtin::registry(shared)
    }

    pub fn descriptors(&self) -> impl Iterator<Item = &SourceDescriptor> {
        self.entries.iter().map(Entry::descriptor)
    }

    pub fn get(&self, id: &str) -> Option<&SourceDescriptor> {
        self.descriptors().find(|d| d.id == id)
    }

    /// The sources with a lookup, in order.
    pub fn lookups(&self) -> impl Iterator<Item = &dyn MetadataSource> {
        self.entries.iter().filter_map(|e| match e {
            Entry::Lookup(source) => Some(source.as_ref()),
            Entry::DescriptorOnly(_) => None,
        })
    }

    /// Whether `id` is asked by the merge (`false` for descriptor-only and
    /// unknown ids).
    pub fn has_lookup(&self, id: &str) -> bool {
        self.lookups().any(|s| s.descriptor().id == id)
    }

    fn lookup(&self, id: &str) -> Option<&dyn MetadataSource> {
        self.lookups().find(|s| s.descriptor().id == id)
    }

    /// Asks `source` for its game `id` (see [`MetadataSource::lookup_id`]),
    /// without the pictures flagged adult unless the user allows them.
    pub fn lookup_id(&self, ctx: &SourceContext<'_>, source: &str, game: &SourceGame, id: &str) -> Lookup<MetadataCandidate> {
        match self.lookup(source) {
            Some(s) if s.descriptor().searchable => s.lookup_id(ctx, game, id).map(|mut c| {
                if !ctx.settings().allow_adult_covers {
                    c.drop_adult();
                }
                c
            }),
            _ => Lookup::Skipped,
        }
    }

    /// Searches `source` by name (see [`MetadataSource::search`]), without
    /// the thumbnails flagged adult unless the user allows them.
    pub fn search(&self, ctx: &SourceContext<'_>, source: &str, query: &str) -> Lookup<Vec<SourceHit>> {
        match self.lookup(source) {
            Some(s) if s.descriptor().searchable => s.search(ctx, query).map(|mut hits| {
                if !ctx.settings().allow_adult_covers {
                    hits.iter_mut().for_each(SourceHit::drop_adult);
                }
                hits
            }),
            _ => Lookup::Skipped,
        }
    }

    /// Whether a request asking only `sources` names fallback source `id`:
    /// it is among them, and every details source among them is a fallback
    /// one ("only this source"). A job that leaves another source out
    /// still asks a fallback source only as a fallback.
    pub fn names_fallback(&self, id: &str, sources: Option<&[String]>) -> bool {
        sources.is_some_and(|list| {
            list.iter().any(|s| s == id)
                && list.iter().filter_map(|s| self.get(s)).all(|d| d.kind != SourceKind::Details || d.is_fallback())
        })
    }

    /// Whether `id` has a test (Settings shows "Test" for it).
    pub fn has_test(&self, id: &str) -> bool {
        self.lookup(id).is_some_and(|s| s.has_test())
    }

    /// Runs the test of `id`; `None` when it has none. Blocking.
    pub fn test(&self, ctx: &SourceContext<'_>, id: &str) -> Option<SourceTest> {
        self.lookup(id)?.test(ctx)
    }

    /// The built-in sources over fresh state, for tests.
    #[cfg(test)]
    pub(crate) fn for_tests() -> Self {
        use std::sync::Arc;
        Self::builtin(Shared {
            igdb: Arc::new(crate::igdb::Igdb::new()),
            steam: Arc::new(crate::steam::SteamState::default()),
            stores: Arc::new(crate::providers::StoreLibraries::default()),
            playnite: Arc::new(parking_lot::RwLock::new(None)),
            vndb: Arc::new(crate::vndb::Vndb::new()),
        })
    }

    /// Whether some lookup's audience depends on the overlay entry.
    pub(crate) fn needs_overlay(&self) -> bool {
        self.lookups().any(|s| s.descriptor().applies_to.needs_overlay())
    }
}
