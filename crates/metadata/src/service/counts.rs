//! What a set of games has, before a job runs: the counts of the dialog's
//! first step and the dock's hint (with the reasons of [`super::Attention`]),
//! and the sources a job may ask. One read of `metadata.json` (plus the
//! Steam tags cache when that source is on, the review queue and
//! `art.json`), no network.

use anyhow::Result;
use heronite_data::cover_family::CoverFamilies;
use heronite_data::json_store;
use heronite_data::library::metadata_key;
use heronite_data::models::{ArtFile, CollectionGameMetadata, LocalMetaFile, MetadataSettings};
use heronite_data::settings::METADATA_FIELDS;
use heronite_data::steam_tags::SteamTagsFile;

use super::attention::classify;
use super::MetadataService;
use crate::job::{bulk_fields, GameCounts};
use crate::merge::has_field_value;
use crate::sources::{extra_fields, ids, is_on, SourceDescriptor, SourceGame, SourceKind};
use crate::store::{as_runner, get_all_game_metadata};
use crate::types::{BulkItem, MetadataApplyArgs};
use crate::{record, review};

/// The fields of `fields` that "Fill" would look at in `md`: empty, not
/// emptied by hand (a field recorded as `manual` stays as the user left it)
/// and not locked.
pub(super) fn missing_details<'a>(
    md: Option<&'a CollectionGameMetadata>,
    fields: &'a [&'static str],
) -> impl Iterator<Item = &'static str> + 'a {
    fields.iter().copied().filter(move |f| {
        !has_field_value(md, f)
            && md.and_then(|m| m.field_sources.get(*f)).map(String::as_str) != Some("manual")
            && !md.is_some_and(|m| record::is_locked(m, f))
    })
}

/// Whether a game whose record is `md` waits for the idle fill to fetch its
/// details: it has none and was never found in no source. A job asking for
/// pictures only leaves it (a record holding only a picture would keep the
/// idle fill away); one marked as found in no source keeps its mark, so its
/// pictures can be asked.
pub(super) fn waits_for_details(md: Option<&CollectionGameMetadata>) -> bool {
    !METADATA_FIELDS.iter().any(|f| has_field_value(md, f)) && md.is_none_or(|m| m.fetch_error.is_none())
}

/// The Steam app id a game's tags are asked with: the game's, else its
/// record's (digits only).
pub(super) fn tag_steam_id(game: &BulkItem, md: Option<&CollectionGameMetadata>) -> Option<String> {
    game.steam_app_id
        .as_deref()
        .filter(|s| !s.trim().is_empty())
        .or_else(|| md.and_then(|m| m.steam_app_id.as_deref()))
        .map(str::trim)
        .filter(|s| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()))
        .map(str::to_owned)
}

impl MetadataService {
    /// The fields of `fields` some source can fill: one that is on, has the
    /// field on, is asked by the merge and is in `sources` (`None`: any).
    pub(super) fn fillable_fields(
        &self,
        settings: &MetadataSettings,
        fields: &[&'static str],
        sources: Option<&[String]>,
    ) -> Vec<&'static str> {
        // A fallback source counts only when the job names it: otherwise it
        // is not asked about games the others found, and counting its
        // fields would fetch every game lacking them for nothing.
        let registry = self.sources();
        let fallback = |id: &String| registry.get(id).is_some_and(|d| d.is_fallback());
        let asked = |id: &String| {
            registry.has_lookup(id) && sources.is_none_or(|s| s.contains(id)) && (!fallback(id) || registry.names_fallback(id, sources))
        };
        fields.iter().copied().filter(|f| registry.field_order(settings, f).iter().any(asked)).collect()
    }

    /// The sources a job may ask, in the user's order: the details sources
    /// that are on and offered, and Steam tags when on. A pictures source
    /// (SteamGridDB) is asked by the fields it gives, not listed here.
    pub fn bulk_sources(&self, settings: &MetadataSettings) -> Vec<SourceDescriptor> {
        let sources = self.sources();
        sources
            .order(settings)
            .iter()
            .filter_map(|id| sources.get(id))
            .filter(|d| is_on(settings, &d.id) && !d.hidden && !d.manual_only && d.available_here())
            .filter(|d| (sources.has_lookup(&d.id) && d.kind == SourceKind::Details) || d.id == ids::STEAM_TAGS)
            .cloned()
            .collect()
    }

    /// The counts of `games` (see [`GameCounts`]). Fast enough for the whole
    /// Collection: one read of each file, nothing per game.
    pub fn bulk_counts(&self, games: &[BulkItem]) -> Result<GameCounts> {
        let settings = self.metadata_settings();
        let records = get_all_game_metadata(&self.paths)?;
        let registry = self.sources();
        let sources = self.bulk_sources(&settings);
        let families = CoverFamilies::load(&self.paths);
        let fields: Vec<&'static str> = METADATA_FIELDS.iter().copied().chain(families.kept_extra_fields()).collect();
        let fillable = self.fillable_fields(&settings, &fields, None);
        let tags = is_on(&settings, ids::STEAM_TAGS).then(|| SteamTagsFile::load(&self.paths));
        let overlay = registry.needs_overlay().then(|| json_store::read::<LocalMetaFile>(&self.paths.local_meta()).ok()).flatten();
        let needs_download: Vec<String> =
            sources.iter().filter(|d| !registry.downloaded(&self.paths, &d.id)).map(|d| d.id.clone()).collect();

        let art: ArtFile = json_store::read(&self.paths.art())?;
        let attention = classify(games, &records, &review::load(&self.paths), &art, &families.in_use());

        let mut counts = GameCounts {
            games: games.len(),
            no_details: attention.no_details.len(),
            failed_last_time: attention.failed.len(),
            to_review: attention.to_review.len(),
            no_description: attention.no_description.len(),
            no_cover: attention.no_cover.len(),
            cropped: attention.cropped.iter().map(|(f, keys)| (*f, keys.len())).collect(),
            in_no_source: attention.in_no_source.len(),
            without_match: attention.without_match().len(),
            needs_attention: attention.all().len(),
            missing_by_field: bulk_fields().into_iter().map(|f| (f, 0)).collect(),
            reach: sources.iter().map(|d| (d.id.clone(), 0)).collect(),
            needs_download: needs_download.clone(),
            ..Default::default()
        };
        for game in games {
            let md = records.get(&metadata_key(as_runner(&game.runner), &game.app_name));

            let mut missing: Vec<&str> = missing_details(md, &fillable).collect();
            let steam_id = tag_steam_id(game, md);
            if let (Some(file), Some(id)) = (&tags, &steam_id) {
                if !file.apps.contains_key(id) {
                    missing.push(extra_fields::STEAM_TAGS);
                }
            }
            for f in &missing {
                *counts.missing_by_field.entry((*f).to_owned()).or_default() += 1;
            }
            counts.fields_missing += missing.len();
            counts.games_missing += usize::from(!missing.is_empty());

            let args = MetadataApplyArgs {
                app_name: game.app_name.clone(),
                runner: game.runner.clone(),
                title: game.title.clone(),
                steam_app_id: steam_id,
                ..Default::default()
            };
            let platforms = md.map_or(&[][..], |m| m.platforms.as_slice());
            let view = SourceGame::new(&args, overlay.as_ref().and_then(|o| o.games.get(&game.app_name)), platforms);
            for d in sources.iter().filter(|d| !needs_download.contains(&d.id)) {
                if registry.applies(&self.paths, &settings, d, &view) {
                    *counts.reach.entry(d.id.clone()).or_default() += 1;
                }
            }
        }
        Ok(counts)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::blank_metadata;

    #[test]
    fn missing_leaves_out_filled_hand_emptied_and_locked_fields() {
        let mut md = blank_metadata("a", "sideload");
        md.description = Some("d".into());
        md.field_sources.insert("series".into(), "manual".into());
        let fields = ["description", "series", "genres", "themes"];
        record::set_locked(&mut md, "themes", true);
        assert_eq!(missing_details(Some(&md), &fields).collect::<Vec<_>>(), ["genres"]);
        assert_eq!(missing_details(None, &fields).count(), 4);
    }

    #[test]
    fn tags_are_asked_with_the_games_or_the_records_steam_id() {
        let game = |id: Option<&str>| BulkItem {
            app_name: "a".into(),
            runner: "sideload".into(),
            steam_app_id: id.map(Into::into),
            ..Default::default()
        };
        let mut md = blank_metadata("a", "sideload");
        md.steam_app_id = Some(" 620 ".into());
        assert_eq!(tag_steam_id(&game(Some("400")), Some(&md)).as_deref(), Some("400"));
        assert_eq!(tag_steam_id(&game(None), Some(&md)).as_deref(), Some("620"));
        assert_eq!(tag_steam_id(&game(Some("x1")), None), None);
        assert_eq!(tag_steam_id(&game(None), None), None);
    }
}
