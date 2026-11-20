//! libretro-thumbnails' adapter: one lookup, no search, no options.

use heronite_data::cover_family::CoverFamily;

use crate::libretro;
use crate::sources::{MetadataSource, SourceContext, SourceDescriptor, SourceGame};
use crate::types::{ArtKind, Lookup, MetadataCandidate};
use crate::util::blank_metadata;

/// The box art of an emulated game, in the cover family its shape belongs
/// to ([`libretro::family_of`]), when that family is one asked, and the logo
/// of the same name beside it when the logo is asked.
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
        let asked = |field: &str| {
            self.d.provides.iter().any(|f| f == field) && game.fields.as_ref().is_none_or(|list| list.iter().any(|f| f == field))
        };
        let wanted: Vec<CoverFamily> = CoverFamily::ALL.into_iter().filter(|f| asked(f.field())).collect();
        let logo = asked(ArtKind::Logo.as_str());
        // The logo is named like the box art, whatever its shape: any box
        // art found names it.
        let look_for: &[CoverFamily] = if logo { &CoverFamily::ALL } else { &wanted };
        self.client.find(ctx.http, game, heronite_i18n::current(), look_for).and_then(|found| {
            // No title: the record keeps the one the details sources give.
            let mut md = blank_metadata(&game.app_name, &game.runner);
            if wanted.contains(&found.family) {
                *ArtKind::cover(found.family).of_record_mut(&mut md) = Some(found.url.clone());
            }
            if logo {
                match self.client.logo(ctx.http, &found) {
                    Lookup::Found(url) => md.logo_url = Some(url),
                    Lookup::Unreachable => log::warn!("libretro-thumbnails did not answer for the logo of {}", found.name),
                    _ => {}
                }
            }
            if ArtKind::ALL.iter().all(|k| k.of_record(&md).is_none()) {
                return Lookup::NotFound;
            }
            Lookup::Found(MetadataCandidate {
                source: self.d.id.clone(),
                match_name: Some(found.title),
                match_year: None,
                matched_by: Some(found.matched_by),
                match_id: None,
                metadata: md,
                adult_pictures: Vec::new(),
            })
        })
    }
}
