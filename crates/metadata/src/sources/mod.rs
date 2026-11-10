//! Metadata sources: what each one is ([`SourceDescriptor`]), how a details
//! source is asked about a game ([`MetadataSource`]) and the built-in ones in
//! one place ([`SourceRegistry`]).
//!
//! Details sources have a lookup and feed the merge. Pictures and extras
//! sources (SteamGridDB, web images, HowLongToBeat, Steam tags, ProtonDB) are
//! registered as descriptors only: they still run on their own paths.

mod builtin;
mod descriptor;
#[cfg(test)]
mod tests;

use std::sync::OnceLock;

use heronite_data::models::MetadataSettings;
use heronite_data::paths::DataPaths;
use heronite_data::settings;

pub(crate) use builtin::Shared;
pub use builtin::{builtin_descriptors, offered};
pub use descriptor::{extra_fields, AppliesTo, Needs, SourceDescriptor, SourceGame, SourceKind};

use crate::http::Http;
use crate::types::{Lookup, MetadataCandidate};

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
}

enum Entry {
    Lookup(Box<dyn MetadataSource>),
    /// Known (listed, has settings) but fetched elsewhere.
    DescriptorOnly(SourceDescriptor),
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

    /// Whether some lookup's audience depends on the overlay entry.
    pub(crate) fn needs_overlay(&self) -> bool {
        self.lookups().any(|s| s.descriptor().applies_to.needs_overlay())
    }
}
