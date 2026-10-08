//! `local_library/metadata.json`: port of `metadata/store.ts`. Every write is
//! a read-modify-write through `json_store::update`, so fields Heronite wrote
//! that this crate does not model survive.

use anyhow::Result;
use heronite_data::json_store;
use heronite_data::library::metadata_key;
use heronite_data::models::{CollectionGameMetadata, MetadataFile};
use heronite_data::paths::HeroicPaths;
use indexmap::IndexMap;

use crate::util::blank_metadata;

pub fn get_game_metadata(paths: &HeroicPaths, runner: &str, app_name: &str) -> Result<Option<CollectionGameMetadata>> {
    let file: MetadataFile = json_store::read(&paths.metadata())?;
    Ok(file.games.get(&metadata_key(runner, app_name)).cloned())
}

pub fn get_all_game_metadata(paths: &HeroicPaths) -> Result<IndexMap<String, CollectionGameMetadata>> {
    Ok(json_store::read::<MetadataFile>(&paths.metadata())?.games)
}

/// `upsertGameMetadata`; returns what was stored.
pub fn upsert_game_metadata(paths: &HeroicPaths, metadata: CollectionGameMetadata) -> Result<CollectionGameMetadata> {
    json_store::update::<MetadataFile, _>(&paths.metadata(), |file| {
        file.games.insert(metadata_key(&metadata.runner, &metadata.app_name), metadata.clone());
        Ok(())
    })?;
    Ok(metadata)
}

pub fn delete_game_metadata(paths: &HeroicPaths, runner: &str, app_name: &str) -> Result<()> {
    json_store::update::<MetadataFile, _>(&paths.metadata(), |file| {
        file.games.shift_remove(&metadata_key(runner, app_name));
        Ok(())
    })
}

/// `listGameMetadata`: the same map without the heavy fields (description,
/// notes, websites), for the screen's bulk load.
pub fn list_game_metadata(paths: &HeroicPaths) -> Result<IndexMap<String, CollectionGameMetadata>> {
    let mut games = get_all_game_metadata(paths)?;
    for md in games.values_mut() {
        md.description = None;
        md.notes = None;
        md.websites.clear();
    }
    Ok(games)
}

/// `currentOrBlank`.
pub fn current_or_blank(paths: &HeroicPaths, runner: &str, app_name: &str) -> Result<CollectionGameMetadata> {
    Ok(get_game_metadata(paths, runner, app_name)?.unwrap_or_else(|| blank_metadata(app_name, runner)))
}

/// `asRunner`: `zoom` is stored as `sideload`.
pub fn as_runner(runner: &str) -> &str {
    if runner == "zoom" {
        "sideload"
    } else {
        runner
    }
}
