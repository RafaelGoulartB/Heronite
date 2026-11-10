//! libretro-thumbnails' adapter: one lookup, no search, no options.

use heronite_data::cover_family::CoverFamily;

use crate::libretro;
use crate::sources::{MetadataSource, SourceContext, SourceDescriptor, SourceGame};
use crate::types::{ArtKind, Lookup, MetadataCandidate};
use crate::util::blank_metadata;

/// The box art of an emulated game, in the cover family its shape belongs
/// to ([`libretro::family_of`]), when that family is one asked.
pub(super) struct LibretroSource {
    pub(super) d: &'static SourceDescriptor,
    /// Kept while extensions come and go: the registry keeps this source.
    pub(super) client: libretro::Libretro,
}

impl MetadataSource for LibretroSource {
    fn descriptor(&self) -> &SourceDescriptor {
        self.d
    }

    fn lookup(&self, ctx: &SourceContext<'_>, game: &SourceGame) -> Lookup<MetadataCandidate> {
        let asked = |field: &String| game.fields.as_ref().is_none_or(|list| list.contains(field));
        let wanted: Vec<CoverFamily> = self.d.provides.iter().filter(|f| asked(f)).filter_map(|f| CoverFamily::from_field(f)).collect();
        self.client.find(ctx.http, game, heronite_i18n::current(), &wanted).map(|found| {
            // No title: the record keeps the one the details sources give.
            let mut md = blank_metadata(&game.app_name, &game.runner);
            *ArtKind::cover(found.family).of_record_mut(&mut md) = Some(found.url);
            MetadataCandidate {
                source: self.d.id.clone(),
                match_name: Some(found.title),
                match_year: None,
                matched_by: Some(found.matched_by),
                match_id: None,
                metadata: md,
                adult_pictures: Vec::new(),
            }
        })
    }
}
