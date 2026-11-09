//! The GOG achievements cache of Heroic's layout
//! (`store_cache/gog_achievements.json`, keyed by product id) moved into the
//! local copy once, then removed.

use anyhow::Result;
use chrono::Utc;
use heronite_data::json_store;
use heronite_data::paths::DataPaths;

use crate::model::Source;
use crate::sources::gog;
use crate::store;

/// Moves every cached game that has no copy yet. Returns how many moved.
pub fn gog_cache(paths: &DataPaths) -> Result<usize> {
    let file = paths.store_cache("gog_achievements");
    if !file.exists() {
        return Ok(0);
    }
    let doc = json_store::read_value(&file)?;
    let mut moved = 0;
    for (product_id, items) in doc.as_object().into_iter().flatten().filter(|(k, _)| *k != "__timestamp") {
        let mut game = store::read(paths, "gog", product_id);
        if game.set(Source::Gog).is_some() {
            continue;
        }
        let Some(mut set) = gog::parse(product_id, &serde_json::json!({ "items": items })) else { continue };
        if set.achievements.is_empty() {
            continue;
        }
        // An old read: let the next refresh take its place.
        set.fetched_at = chrono::DateTime::<Utc>::UNIX_EPOCH;
        game.sets.push(set);
        store::write(paths, "gog", product_id, &game, &[Source::Gog])?;
        moved += 1;
    }
    std::fs::remove_file(&file)?;
    Ok(moved)
}

/// Whether there is anything to move (cheap: one `stat`).
pub fn needs_gog_cache(paths: &DataPaths) -> bool {
    paths.store_cache("gog_achievements").exists()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn moves_the_old_cache_and_removes_it() {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path());
        std::fs::create_dir_all(dir.path().join("store_cache")).unwrap();
        let item = json!({"achievement_id":"1","achievement_key":"K","visible":true,"name":"N","description":"",
            "image_url_unlocked":"u","image_url_locked":"l","rarity":3.0,"date_unlocked":"2026-01-02T03:04:05+0000"});
        json_store::write(&paths.store_cache("gog_achievements"), &json!({"7": [item], "8": [], "__timestamp": {"7": "x"}})).unwrap();
        assert!(needs_gog_cache(&paths));
        assert_eq!(gog_cache(&paths).unwrap(), 1);
        assert!(!needs_gog_cache(&paths));
        let game = store::read(&paths, "gog", "7");
        assert_eq!(game.set(Source::Gog).unwrap().unlocked(), 1);
    }
}
