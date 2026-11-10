//! What a set of games has, before a job runs: the counts of the dialog's
//! first step and the dock's hint, the games that need attention, and the
//! sources a job may ask. One read of `metadata.json` (plus the Steam tags
//! cache when that source is on), no network.

use anyhow::Result;
use heronite_data::json_store;
use heronite_data::library::metadata_key;
use heronite_data::models::{CollectionGameMetadata, LocalMetaFile, MetadataSettings};
use heronite_data::settings::METADATA_FIELDS;
use heronite_data::steam_tags::SteamTagsFile;

use super::MetadataService;
use crate::job::{bulk_fields, GameCounts};
use crate::merge::has_field_value;
use crate::sources::{extra_fields, ids, is_on, SourceDescriptor, SourceGame};
use crate::store::{as_runner, get_all_game_metadata};
use crate::types::{BulkItem, MetadataApplyArgs};

/// The fields of `fields` that "Fill" would look at in `md`: empty and not
/// emptied by hand (a field recorded as `manual` stays as the user left it).
pub(super) fn missing_details<'a>(
    md: Option<&'a CollectionGameMetadata>,
    fields: &'a [&'static str],
) -> impl Iterator<Item = &'static str> + 'a {
    fields
        .iter()
        .copied()
        .filter(move |f| !has_field_value(md, f) && md.and_then(|m| m.field_sources.get(*f)).map(String::as_str) != Some("manual"))
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

/// No value in any field (or no record at all).
fn has_no_details(md: Option<&CollectionGameMetadata>) -> bool {
    !METADATA_FIELDS.iter().any(|f| has_field_value(md, f))
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
        let asked = |id: &String| self.sources.has_lookup(id) && sources.is_none_or(|s| s.contains(id));
        fields.iter().copied().filter(|f| self.sources.field_order(settings, f).iter().any(asked)).collect()
    }

    /// The sources a job may ask, in the user's order: the details sources
    /// that are on and offered, and Steam tags when on.
    pub fn bulk_sources(&self, settings: &MetadataSettings) -> Vec<SourceDescriptor> {
        self.sources
            .order(settings)
            .iter()
            .filter_map(|id| self.sources.get(id))
            .filter(|d| is_on(settings, &d.id) && !d.hidden && !d.manual_only && d.available_here())
            .filter(|d| self.sources.has_lookup(&d.id) || d.id == ids::STEAM_TAGS)
            .cloned()
            .collect()
    }

    /// The counts of `games` (see [`GameCounts`]). Fast enough for the whole
    /// Collection: one read of each file, nothing per game.
    pub fn bulk_counts(&self, games: &[BulkItem]) -> Result<GameCounts> {
        let settings = self.metadata_settings();
        let records = get_all_game_metadata(&self.paths)?;
        let sources = self.bulk_sources(&settings);
        let fillable = self.fillable_fields(&settings, &METADATA_FIELDS, None);
        let tags = is_on(&settings, ids::STEAM_TAGS).then(|| SteamTagsFile::load(&self.paths));
        let overlay = sources
            .iter()
            .any(|d| d.applies_to.needs_overlay())
            .then(|| json_store::read::<LocalMetaFile>(&self.paths.local_meta()).ok())
            .flatten();

        let mut counts = GameCounts {
            games: games.len(),
            missing_by_field: bulk_fields().into_iter().map(|f| (f, 0)).collect(),
            reach: sources.iter().map(|d| (d.id.clone(), 0)).collect(),
            ..Default::default()
        };
        for game in games {
            let md = records.get(&metadata_key(as_runner(&game.runner), &game.app_name));
            counts.no_details += usize::from(has_no_details(md));
            counts.failed_last_time += usize::from(md.is_some_and(|m| m.fetch_error.is_some()));

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
            let view = SourceGame::new(&args, overlay.as_ref().and_then(|o| o.games.get(&game.app_name)));
            for d in &sources {
                if d.applies_to.matches(&view) {
                    *counts.reach.entry(d.id.clone()).or_default() += 1;
                }
            }
        }
        Ok(counts)
    }

    /// The games of `games` that need attention: no details at all, or the
    /// last fetch found nothing. In their order.
    pub fn needs_attention(&self, games: &[BulkItem]) -> Result<Vec<BulkItem>> {
        let records = get_all_game_metadata(&self.paths)?;
        Ok(games
            .iter()
            .filter(|g| {
                let md = records.get(&metadata_key(as_runner(&g.runner), &g.app_name));
                has_no_details(md) || md.is_some_and(|m| m.fetch_error.is_some())
            })
            .cloned()
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::blank_metadata;

    #[test]
    fn missing_leaves_out_filled_and_hand_emptied_fields() {
        let mut md = blank_metadata("a", "sideload");
        md.description = Some("d".into());
        md.field_sources.insert("series".into(), "manual".into());
        let fields = ["description", "series", "genres"];
        assert_eq!(missing_details(Some(&md), &fields).collect::<Vec<_>>(), ["genres"]);
        assert_eq!(missing_details(None, &fields).count(), 3);
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
