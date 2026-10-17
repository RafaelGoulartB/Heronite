//! Manual details editing: port of `metadata/edit.ts`.

use anyhow::Result;
use heronite_data::field_names::NameRules;
use heronite_data::json_store;
use heronite_data::library::metadata_key;
use heronite_data::models::{CollectionGameMetadata, LocalMetaFile, MetadataFile};
use heronite_data::num::JsNum;
use heronite_data::paths::DataPaths;

use crate::store::{as_runner, current_or_blank, upsert_game_metadata};
use crate::types::{DetailsPatch, DetailsResult};
use crate::util::{blank_metadata, unique_names};

/// `setOptionalString`: trimmed value, or removed when blank.
fn trimmed_or_none(value: &str) -> Option<String> {
    let v = value.trim();
    (!v.is_empty()).then(|| v.to_owned())
}

fn set_manual_array(
    target: &mut CollectionGameMetadata,
    field: &str,
    slot: fn(&mut CollectionGameMetadata) -> &mut Vec<String>,
    value: &Option<Vec<String>>,
) {
    if let Some(v) = value {
        *slot(target) = unique_names(v.iter().map(|s| Some(s.as_str())), 40);
        target.field_sources.insert(field.to_owned(), "manual".into());
    }
}

/// `updateCollectionGameDetails`: patch semantics of the details editor. Only
/// the fields present in the patch change; edited fields get the `manual`
/// source so later fetches keep them. `title`/`notes` are also mirrored into
/// the overlay's `LocalGameMeta` when the game has one.
pub fn update_collection_game_details(paths: &DataPaths, patch: &DetailsPatch) -> Result<DetailsResult> {
    let runner = as_runner(&patch.runner).to_owned();
    let mut next = current_or_blank(paths, &runner, &patch.app_name)?;
    next.app_name = patch.app_name.clone();
    next.runner = runner;

    if let Some(v) = &patch.title {
        next.title = trimmed_or_none(v);
    }
    if let Some(v) = &patch.notes {
        next.notes = trimmed_or_none(v);
    }
    let manual = |next: &mut CollectionGameMetadata, field: &str| {
        next.field_sources.insert(field.to_owned(), "manual".into());
    };
    if let Some(v) = &patch.description {
        next.description = trimmed_or_none(v);
        manual(&mut next, "description");
    }
    if let Some(v) = &patch.release_date {
        next.release_date = trimmed_or_none(v);
        manual(&mut next, "releaseDate");
    }
    if let Some(v) = &patch.series {
        next.series = trimmed_or_none(v);
        manual(&mut next, "series");
    }
    set_manual_array(&mut next, "developers", |m| &mut m.developers, &patch.developers);
    set_manual_array(&mut next, "publishers", |m| &mut m.publishers, &patch.publishers);
    set_manual_array(&mut next, "genres", |m| &mut m.genres, &patch.genres);
    set_manual_array(&mut next, "themes", |m| &mut m.themes, &patch.themes);
    set_manual_array(&mut next, "gameModes", |m| &mut m.game_modes, &patch.game_modes);
    set_manual_array(&mut next, "platforms", |m| &mut m.platforms, &patch.platforms);
    set_manual_array(&mut next, "features", |m| &mut m.features, &patch.features);

    if let Some(v) = &patch.websites {
        next.websites = unique_names(v.iter().map(|s| Some(s.as_str())), 20);
    }
    match patch.critic_score {
        Some(None) => {
            next.critic_score = None;
            manual(&mut next, "criticScore");
        }
        Some(Some(n)) if n.is_finite() => {
            next.critic_score = Some(JsNum(n));
            manual(&mut next, "criticScore");
        }
        _ => {}
    }

    let metadata = upsert_game_metadata(paths, next)?;
    let meta = sync_local_game_meta(paths, patch)?;
    Ok(DetailsResult { metadata, meta })
}

/// `syncLocalGameMeta`.
fn sync_local_game_meta(paths: &DataPaths, patch: &DetailsPatch) -> Result<Option<heronite_data::models::LocalGameMeta>> {
    let current: LocalMetaFile = json_store::read(&paths.local_meta())?;
    let Some(existing) = current.games.get(&patch.app_name) else { return Ok(None) };
    if patch.title.is_none() && patch.notes.is_none() {
        return Ok(Some(existing.clone()));
    }
    json_store::update::<LocalMetaFile, _>(&paths.local_meta(), |file| {
        let Some(existing) = file.games.get_mut(&patch.app_name) else { return Ok(None) };
        if let Some(title) = &patch.title {
            if let Some(t) = trimmed_or_none(title) {
                existing.title = t;
            }
        }
        if let Some(notes) = &patch.notes {
            existing.notes = trimmed_or_none(notes);
        }
        Ok(Some(existing.clone()))
    })
}

/// A tag field of the details: the lists the batch editor changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ListField {
    Genres,
    Developers,
    Publishers,
    Themes,
    GameModes,
    Platforms,
    Features,
}

impl ListField {
    pub const ALL: [ListField; 7] =
        [Self::Genres, Self::Developers, Self::Publishers, Self::Themes, Self::GameModes, Self::Platforms, Self::Features];

    /// The field's key in `fieldSources`.
    pub fn key(self) -> &'static str {
        match self {
            Self::Genres => "genres",
            Self::Developers => "developers",
            Self::Publishers => "publishers",
            Self::Themes => "themes",
            Self::GameModes => "gameModes",
            Self::Platforms => "platforms",
            Self::Features => "features",
        }
    }

    pub fn get(self, md: &CollectionGameMetadata) -> &[String] {
        match self {
            Self::Genres => &md.genres,
            Self::Developers => &md.developers,
            Self::Publishers => &md.publishers,
            Self::Themes => &md.themes,
            Self::GameModes => &md.game_modes,
            Self::Platforms => &md.platforms,
            Self::Features => &md.features,
        }
    }

    fn apply_to(self, md: &mut CollectionGameMetadata, delta: &ListDelta) {
        let next = delta.apply(self.get(md));
        *self.slot(md) = next;
    }

    fn slot(self, md: &mut CollectionGameMetadata) -> &mut Vec<String> {
        match self {
            Self::Genres => &mut md.genres,
            Self::Developers => &mut md.developers,
            Self::Publishers => &mut md.publishers,
            Self::Themes => &mut md.themes,
            Self::GameModes => &mut md.game_modes,
            Self::Platforms => &mut md.platforms,
            Self::Features => &mut md.features,
        }
    }
}

/// What the batch editor does to one tag field of every game: names given to
/// all of them and names taken from all of them. The rest of each game's list
/// stays as it is.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ListDelta {
    pub add: Vec<String>,
    pub remove: Vec<String>,
}

impl ListDelta {
    pub fn is_empty(&self) -> bool {
        self.add.is_empty() && self.remove.is_empty()
    }

    fn apply(&self, list: &[String]) -> Vec<String> {
        let gone = |v: &String| self.remove.iter().any(|r| r.eq_ignore_ascii_case(v));
        let kept = list.iter().filter(|v| !gone(v));
        unique_names(kept.chain(self.add.iter()).map(|s| Some(s.as_str())), 40)
    }
}

/// The batch editor's changes (F3 with several games selected).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BatchDetails {
    /// A series for all of them; `Some("")` clears it.
    pub series: Option<String>,
    pub lists: Vec<(ListField, ListDelta)>,
}

impl BatchDetails {
    pub fn is_empty(&self) -> bool {
        self.series.is_none() && self.lists.iter().all(|(_, d)| d.is_empty())
    }
}

/// What a game's metadata entry was before a batch edit (`None`: it had none),
/// under its `metadata.json` key.
pub type DetailsBefore = Vec<(String, Option<CollectionGameMetadata>)>;

/// Applies `edit` to every `(runner, app_name)` in one write of
/// `metadata.json`. Changed fields get the `manual` source, as in the details
/// editor. The library fields manager's rules are applied to each game first,
/// so the names it shows are the names removed. Returns what each game had
/// before, for [`restore_details`].
pub fn update_many_details(paths: &DataPaths, games: &[(String, String)], edit: &BatchDetails) -> Result<DetailsBefore> {
    if games.is_empty() || edit.is_empty() {
        return Ok(Vec::new());
    }
    let rules = NameRules::load(paths);
    json_store::update::<MetadataFile, _>(&paths.metadata(), |file| {
        let mut before = Vec::with_capacity(games.len());
        for (runner, app_name) in games {
            let runner = as_runner(runner);
            let key = metadata_key(runner, app_name);
            let prev = file.games.get(&key).cloned();
            let mut next = prev.clone().unwrap_or_else(|| blank_metadata(app_name, runner));
            rules.apply_metadata(&mut next);
            if let Some(v) = &edit.series {
                next.series = trimmed_or_none(v);
                next.field_sources.insert("series".into(), "manual".into());
            }
            for (field, delta) in edit.lists.iter().filter(|(_, d)| !d.is_empty()) {
                field.apply_to(&mut next, delta);
                next.field_sources.insert(field.key().into(), "manual".into());
            }
            file.games.insert(key.clone(), next);
            before.push((key, prev));
        }
        Ok(before)
    })
}

/// Puts back the entries [`update_many_details`] returned (undo).
pub fn restore_details(paths: &DataPaths, before: &DetailsBefore) -> Result<()> {
    if before.is_empty() {
        return Ok(());
    }
    json_store::update::<MetadataFile, _>(&paths.metadata(), |file| {
        for (key, prev) in before {
            match prev {
                Some(md) => {
                    file.games.insert(key.clone(), md.clone());
                }
                None => {
                    file.games.shift_remove(key);
                }
            }
        }
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use heronite_data::json_store::read_value;

    fn patch(f: impl FnOnce(&mut DetailsPatch)) -> DetailsPatch {
        let mut p = DetailsPatch { app_name: "g".into(), runner: "sideload".into(), ..Default::default() };
        f(&mut p);
        p
    }

    #[test]
    fn patch_semantics_and_manual_sources() {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path());
        std::fs::create_dir_all(paths.local_library()).unwrap();
        std::fs::write(
            paths.local_meta(),
            "{\n\t\"games\": {\n\t\t\"g\": {\n\t\t\t\"appName\": \"g\",\n\t\t\t\"runner\": \"sideload\",\n\t\t\t\"title\": \"Old\",\n\t\t\t\"notes\": \"n\",\n\t\t\t\"playniteId\": \"p\",\n\t\t\t\"zzUnknown\": true\n\t\t}\n\t}\n}",
        )
        .unwrap();

        let r = update_collection_game_details(
            &paths,
            &patch(|p| {
                p.title = Some("  New Title ".into());
                p.notes = Some("  ".into());
                p.description = Some(" Desc ".into());
                p.genres = Some(vec!["A".into(), "a".into(), " ".into(), "B".into()]);
                p.websites = Some(vec!["https://x".into()]);
                p.critic_score = Some(Some(77.0));
            }),
        )
        .unwrap();
        let md = r.metadata;
        assert_eq!(md.title.as_deref(), Some("New Title"));
        assert_eq!(md.notes, None);
        assert_eq!(md.description.as_deref(), Some("Desc"));
        assert_eq!(md.genres, ["A", "B"]);
        assert_eq!(md.field_sources["genres"], "manual");
        assert_eq!(md.field_sources["description"], "manual");
        assert_eq!(md.critic_score, Some(JsNum(77.0)));
        assert!(!md.field_sources.contains_key("publishers"));
        assert_eq!(md.websites, ["https://x"]);
        let meta = r.meta.unwrap();
        assert_eq!(meta.title, "New Title");
        assert_eq!(meta.notes, None);
        // The overlay entry keeps fields we do not model.
        assert_eq!(read_value(&paths.local_meta()).unwrap()["games"]["g"]["zzUnknown"], true);

        // null clears the score but still marks it manual; absent fields stay.
        let json = r#"{"appName":"g","runner":"zoom","criticScore":null,"series":""}"#;
        let p: DetailsPatch = serde_json::from_str(json).unwrap();
        let md = update_collection_game_details(&paths, &p).unwrap().metadata;
        assert_eq!(md.runner, "sideload");
        assert_eq!(md.critic_score, None);
        assert_eq!(md.field_sources["criticScore"], "manual");
        assert_eq!(md.series, None);
        assert_eq!(md.field_sources["series"], "manual");
        assert_eq!(md.description.as_deref(), Some("Desc"));

        let stored = read_value(&paths.metadata()).unwrap();
        assert_eq!(stored["games"]["sideload:g"]["title"], "New Title");
        assert!(stored["games"]["sideload:g"].get("criticScore").is_none());
    }

    #[test]
    fn games_without_overlay_meta_only_get_metadata() {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path());
        let r = update_collection_game_details(&paths, &patch(|p| p.notes = Some("hello".into()))).unwrap();
        assert_eq!(r.metadata.notes.as_deref(), Some("hello"));
        assert!(r.meta.is_none());
    }

    #[test]
    fn batch_edit_adds_removes_and_restores() {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path());
        std::fs::create_dir_all(paths.local_library()).unwrap();
        update_collection_game_details(
            &paths,
            &patch(|p| {
                p.genres = Some(vec!["RPG".into(), "Action".into()]);
                p.series = Some("Souls".into());
            }),
        )
        .unwrap();
        let games = [("sideload".to_string(), "g".to_string()), ("zoom".to_string(), "h".to_string())];
        let edit = BatchDetails {
            series: Some(String::new()),
            lists: vec![
                (ListField::Genres, ListDelta { add: vec!["Indie".into(), "rpg".into()], remove: vec!["action".into()] }),
                (ListField::Platforms, ListDelta::default()),
            ],
        };
        let before = update_many_details(&paths, &games, &edit).unwrap();
        let v = read_value(&paths.metadata()).unwrap();
        assert_eq!(v["games"]["sideload:g"]["genres"], serde_json::json!(["RPG", "Indie"]));
        assert_eq!(v["games"]["sideload:h"]["genres"], serde_json::json!(["Indie", "rpg"]));
        assert!(v["games"]["sideload:g"]["series"].is_null());
        assert_eq!(v["games"]["sideload:h"]["fieldSources"]["genres"], "manual");
        assert!(v["games"]["sideload:h"]["fieldSources"].get("platforms").is_none(), "untouched field");
        restore_details(&paths, &before).unwrap();
        let v = read_value(&paths.metadata()).unwrap();
        assert_eq!(v["games"]["sideload:g"]["genres"], serde_json::json!(["RPG", "Action"]));
        assert_eq!(v["games"]["sideload:g"]["series"], "Souls");
        assert!(v["games"].get("sideload:h").is_none());
    }
}
