//! Metadata sources: what each one is ([`SourceDescriptor`]), how a details
//! source is asked about a game ([`MetadataSource`]) and the built-in ones in
//! one place ([`SourceRegistry`]).
//!
//! Details sources have a lookup and feed the merge. A source may be asked
//! only as a fallback ([`Asked`]; the rule is `service/gather`'s), and may
//! flag the pictures it gives as adult: they are dropped unless the user
//! allows adult covers (`settings.metadata.allowAdultCovers`). A source may
//! need a database downloaded first ([`download`]: the descriptor's
//! [`SourceDownload`], the trait's `download_state` / `download` /
//! `remove_download`); it is not asked until it has it. Pictures and extras
//! sources (SteamGridDB, web images, HowLongToBeat, Steam tags, ProtonDB) are
//! registered as descriptors only: they still run on their own paths, which
//! ask [`is_on`] first.
//!
//! What the user set per source (on or off, options, order, fields switched
//! off) is stored by `heronite_data::source_prefs`; [`options`] reads and
//! writes the options, [`order`] gives the order of each field and
//! [`rows`] the read model of Settings → Metadata.
//!
//! Extensions (`heronite-metadata-ext`) are sources too: their descriptor
//! carries [`ExtensionInfo`], their [`MetadataSource`] runs a script, and
//! the service adds and removes them at runtime
//! ([`SourceRegistry::with_source`]). An extension never takes a built-in
//! id. A source that failed to load or keeps failing says so through
//! [`MetadataSource::health`] and is not asked.

mod builtin;
mod descriptor;
pub mod download;
pub mod options;
pub mod order;
pub mod rows;
#[cfg(test)]
mod tests;

use std::sync::{Arc, OnceLock};

use anyhow::{bail, Result};

use heronite_data::models::MetadataSettings;
use heronite_data::paths::DataPaths;
use heronite_data::settings;

pub(crate) use builtin::{box_regions, Shared};
pub use builtin::{builtin_descriptors, offered};
pub use descriptor::{extra_fields, AlsoFor, AppliesTo, Asked, ExtensionInfo, GameKind, Needs, SourceDescriptor, SourceGame, SourceKind};
pub use download::{DownloadProgress, DownloadState, Phase, ProgressSnapshot, SourceDownload};
pub use heronite_data::source_prefs::{ids, is_field_on, is_on};
pub use options::{OptionChoice, OptionKind, OptionValue, SourceOption};
pub use rows::{DownloadRow, FieldRow, OptionRow, SourceRow, SourceStatus};

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

/// Whether a source can be asked at all.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum SourceHealth {
    #[default]
    Ok,
    /// It could not be loaded (an extension whose files are broken or need
    /// a newer app); the text says why, for the log and the details.
    LoadFailed(String),
    /// It failed too many times in a row and is not asked until it is
    /// switched on again or updated; the text is the last failure.
    Failing(String),
}

impl SourceHealth {
    pub fn is_ok(&self) -> bool {
        *self == Self::Ok
    }
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

    /// Where its database stands, for a source whose descriptor has a
    /// `download`. Cheap (a file's metadata, no network): rows and every
    /// gather ask it.
    fn download_state(&self, _paths: &DataPaths) -> DownloadState {
        DownloadState::default()
    }

    /// Downloads (or refreshes) its database, reporting to `progress` and
    /// stopping when it is cancelled ([`download::Cancelled`]). The copy in
    /// use stays until the new one is complete and checked. Blocking.
    fn download(&self, _ctx: &SourceContext<'_>, _progress: &DownloadProgress) -> anyhow::Result<()> {
        anyhow::bail!("{} downloads nothing", self.descriptor().id)
    }

    /// Frees the space its database takes; the source needs a download again.
    fn remove_download(&self, _ctx: &SourceContext<'_>) -> anyhow::Result<()> {
        Ok(())
    }

    /// Whether it can be asked; a source that is not ok is not asked.
    fn health(&self) -> SourceHealth {
        SourceHealth::Ok
    }

    /// Forgets the failures that put it in [`SourceHealth::Failing`] (the
    /// user switched it on again).
    fn reset_health(&self) {}
}

#[derive(Clone)]
enum Entry {
    Lookup(Arc<dyn MetadataSource>),
    /// Known (listed, has settings) but fetched elsewhere.
    DescriptorOnly(Arc<SourceDescriptor>),
}

impl Entry {
    fn descriptor(&self) -> &SourceDescriptor {
        match self {
            Self::Lookup(source) => source.descriptor(),
            Self::DescriptorOnly(d) => d,
        }
    }
}

/// The sources, in the order their candidates are listed and merged: the
/// built-in ones, then the extensions in the order they were added. A
/// registry does not change; adding or removing a source makes a new one
/// (the service swaps it, a gather keeps the one it started with).
#[derive(Clone)]
pub struct SourceRegistry {
    entries: Vec<Entry>,
}

/// Whether `id` is a built-in source's, or a value the saved settings give
/// a meaning of their own (`manual`, `keep`, `auto`): an extension cannot
/// take it.
pub fn is_reserved_id(id: &str) -> bool {
    builtin_descriptors().iter().any(|d| d.id == id) || settings::SOURCE_IDS.contains(&id) || matches!(id, "keep" | "auto")
}

impl SourceRegistry {
    /// The built-in sources over the service's shared state.
    pub(crate) fn builtin(shared: Shared) -> Self {
        builtin::registry(shared)
    }

    /// This registry with `source` added at the end, or in the place of the
    /// extension it updates. A reserved id ([`is_reserved_id`]) is refused.
    pub fn with_source(&self, source: Arc<dyn MetadataSource>) -> Result<Self> {
        let id = source.descriptor().id.clone();
        if is_reserved_id(&id) {
            bail!("{id} is the id of a built-in source");
        }
        let mut entries = self.entries.clone();
        match entries.iter().position(|e| e.descriptor().id == id) {
            Some(i) => entries[i] = Entry::Lookup(source),
            None => entries.push(Entry::Lookup(source)),
        }
        Ok(Self { entries })
    }

    /// This registry without the extension `id`; `None` when it has none
    /// (a built-in source is never removed).
    pub fn without_source(&self, id: &str) -> Option<Self> {
        if is_reserved_id(id) || self.get(id).is_none() {
            return None;
        }
        Some(Self { entries: self.entries.iter().filter(|e| e.descriptor().id != id).cloned().collect() })
    }

    /// Whether `id` can be asked (registered with a lookup and healthy).
    pub fn health(&self, id: &str) -> SourceHealth {
        self.lookup(id).map_or(SourceHealth::Ok, |s| s.health())
    }

    /// Clears the failures of `id` ([`MetadataSource::reset_health`]).
    pub fn reset_health(&self, id: &str) {
        if let Some(s) = self.lookup(id) {
            s.reset_health();
        }
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
            Entry::Lookup(source) => Some(source.as_ref() as &dyn MetadataSource),
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
            Some(s) if s.descriptor().searchable && s.health().is_ok() => s.lookup_id(ctx, game, id).map(|mut c| {
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
            Some(s) if s.descriptor().searchable && s.health().is_ok() => s.search(ctx, query).map(|mut hits| {
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

    /// Whether `d` is for `game`: its audience, or the one an option of it
    /// adds while on ([`AlsoFor`]).
    pub fn applies(&self, paths: &DataPaths, m: &MetadataSettings, d: &SourceDescriptor, game: &SourceGame) -> bool {
        d.applies_to.matches(game) || also_for_on(paths, m, d).is_some_and(|a| a.matches(game))
    }

    /// Where the database of `id` stands (`None`: it downloads none).
    pub fn download_state(&self, paths: &DataPaths, id: &str) -> Option<DownloadState> {
        let source = self.lookup(id)?;
        source.descriptor().download.as_ref()?;
        Some(source.download_state(paths))
    }

    /// Whether `id` can be asked as far as its download goes: it needs none,
    /// or has it.
    pub fn downloaded(&self, paths: &DataPaths, id: &str) -> bool {
        self.download_state(paths, id).is_none_or(|s| s.ready)
    }

    /// Downloads the database of `id` (see [`MetadataSource::download`]).
    pub fn download(&self, ctx: &SourceContext<'_>, id: &str, progress: &DownloadProgress) -> anyhow::Result<()> {
        match self.lookup(id) {
            Some(s) if s.descriptor().download.is_some() => s.download(ctx, progress),
            _ => anyhow::bail!("{id} downloads nothing"),
        }
    }

    /// Removes the database of `id` (see [`MetadataSource::remove_download`]).
    pub fn remove_download(&self, ctx: &SourceContext<'_>, id: &str) -> anyhow::Result<()> {
        match self.lookup(id) {
            Some(s) if s.descriptor().download.is_some() => s.remove_download(ctx),
            _ => Ok(()),
        }
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
        Self::builtin(Shared {
            igdb: Arc::new(crate::igdb::Igdb::new()),
            steam: Arc::new(crate::steam::SteamState::default()),
            stores: Arc::new(crate::providers::StoreLibraries::default()),
            playnite: Arc::new(parking_lot::RwLock::new(None)),
            launchbox: Arc::new(crate::launchbox::LaunchBox::new()),
        })
    }

    /// [`Self::for_tests`] with [`test_fallback`] added, as an extension.
    #[cfg(test)]
    pub(crate) fn for_tests_with_fallback() -> Self {
        Self::for_tests().with_source(Arc::new(test_fallback())).expect("not a built-in id")
    }

    /// Whether some lookup's audience depends on the overlay entry.
    pub(crate) fn needs_overlay(&self) -> bool {
        self.lookups().any(|s| {
            let d = s.descriptor();
            d.applies_to.needs_overlay() || d.also_for.as_ref().is_some_and(|a| a.applies_to.needs_overlay())
        })
    }
}

/// The audience `d`'s option adds, while that option is on.
fn also_for_on<'d>(paths: &DataPaths, m: &MetadataSettings, d: &'d SourceDescriptor) -> Option<&'d AppliesTo> {
    let also = d.also_for.as_ref()?;
    let opt = d.options.iter().find(|o| o.key == also.option)?;
    options::option_value(paths, m, d, opt).as_bool().then_some(&also.applies_to)
}

/// A details source asked as a fallback for visual novels, declared the
/// way an extension declares one, with a select and a switch option. It
/// finds nothing: the tests of the fallback rule, the orders and the rows
/// need its descriptor only.
#[cfg(test)]
pub(crate) struct TestFallback(SourceDescriptor);

#[cfg(test)]
pub(crate) const TEST_FALLBACK: &str = "novels";

#[cfg(test)]
pub(crate) fn test_fallback() -> TestFallback {
    let d = serde_json::from_value(serde_json::json!({
        "id": TEST_FALLBACK, "name": "Novels", "kind": "details", "network": true, "searchable": true,
        "provides": ["description", "releaseDate", "developers", "genres", "themes", "criticScore", "cover", "hero"],
        "asked": "asFallback", "gameKind": {"name": "Visual novels", "signals": ["Visual Novel"]},
        "options": [
            {"key": "titleOrder", "label": "Title to show", "kind": "select",
             "choices": [{"value": "latin", "label": "Romaji"}, {"value": "original", "label": "Original"}]},
            {"key": "screenshotBackground", "label": "Screenshot background", "kind": "switch", "default": true}
        ],
        "extension": {"version": "1.0.0", "author": "tests"}
    }))
    .expect("a test descriptor");
    TestFallback(d)
}

#[cfg(test)]
impl MetadataSource for TestFallback {
    fn descriptor(&self) -> &SourceDescriptor {
        &self.0
    }

    fn lookup(&self, _: &SourceContext<'_>, _: &SourceGame) -> Lookup<MetadataCandidate> {
        Lookup::NotFound
    }
}
