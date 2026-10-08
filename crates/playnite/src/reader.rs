//! Playnite library dump: port of `playnite/reader.ts`. Reads the `*.db`
//! LiteDB files of a (portable) Playnite library plus the GameActivity
//! extension's session JSON. Never writes.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde_json::Value;

use crate::litedb::{read_litedb_documents, Bson, BsonDoc, DocExt};

pub const STEAM_PLUGIN_ID: &str = "cb91dfc9-b977-43bf-8e70-55f46e410fab";
pub const EPIC_PLUGIN_ID: &str = "00000002-dbd1-46c6-b5d0-b1ba559d10e4";
pub const GOG_PLUGIN_ID: &str = "aebe8b7c-6dc3-4a66-af31-e7375c6b5e9e";
pub const XBOX_PLUGIN_ID: &str = "7e4fbb5e-2ae3-48d4-8ba0-6b30e7a4e287";
pub const AMAZON_PLUGIN_ID: &str = "402674cd-4af6-4886-b6ec-0e695bfa0688";
pub const EMPTY_GUID: &str = "00000000-0000-0000-0000-000000000000";
pub const GAME_ACTIVITY_ID: &str = "afbb1a0d-04a1-4d0c-9afa-c6e42ca855b4";

#[derive(Debug, Clone, Default, PartialEq)]
pub struct PlayniteGameAction {
    /// Playnite `GameActionType`: 0 file, 1 url, 2 emulator, 3 script.
    pub action_type: i64,
    pub name: Option<String>,
    pub path: Option<String>,
    pub arguments: Option<String>,
    pub working_dir: Option<String>,
    pub is_play_action: bool,
    pub emulator_id: Option<String>,
    pub emulator_profile_id: Option<String>,
    pub additional_arguments: Option<String>,
    pub override_default_args: bool,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct PlayniteRom {
    pub name: Option<String>,
    pub path: String,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct PlayniteEmulatorProfile {
    pub id: Option<String>,
    pub name: Option<String>,
    pub executable: Option<String>,
    pub arguments: Option<String>,
    pub working_dir: Option<String>,
    pub startup_executable: Option<String>,
    pub custom_arguments: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct PlayniteEmulator {
    pub id: String,
    pub name: Option<String>,
    pub install_dir: Option<String>,
    pub profiles: Vec<PlayniteEmulatorProfile>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct PlayniteGame {
    pub id: String,
    pub name: String,
    pub game_id: Option<String>,
    pub plugin_id: String,
    pub source_id: Option<String>,
    pub install_directory: Option<String>,
    pub is_installed: bool,
    pub playtime_seconds: f64,
    pub play_count: f64,
    pub last_activity: Option<String>,
    pub added: Option<String>,
    pub notes: Option<String>,
    pub description: Option<String>,
    pub cover_image: Option<String>,
    pub background_image: Option<String>,
    pub icon: Option<String>,
    pub release_date: Option<String>,
    pub developer_ids: Vec<String>,
    pub developers: Vec<String>,
    pub publishers: Vec<String>,
    pub genres: Vec<String>,
    pub tags: Vec<String>,
    pub features: Vec<String>,
    pub platforms: Vec<String>,
    pub series: Option<String>,
    pub game_actions: Vec<PlayniteGameAction>,
    pub roms: Vec<PlayniteRom>,
    pub completion_status_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PlayniteCompletionStatus {
    pub id: String,
    pub name: String,
}

/// One GameActivity item.
#[derive(Debug, Clone, PartialEq)]
pub struct PlayniteActivitySession {
    pub date_session: String,
    pub elapsed_seconds: f64,
    pub game_action_name: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct PlayniteLibraryDump {
    pub library_path: String,
    pub playnite_root: String,
    pub games: Vec<PlayniteGame>,
    pub emulators: Vec<PlayniteEmulator>,
    pub completion_statuses: Vec<PlayniteCompletionStatus>,
    /// Keyed by lower-case game id (the GameActivity file name).
    pub sessions_by_game_id: HashMap<String, Vec<PlayniteActivitySession>>,
}

fn read_db(library_path: &Path, file_name: &str) -> Result<Vec<BsonDoc>> {
    let file_path = library_path.join(file_name);
    if !file_path.exists() {
        return Ok(Vec::new());
    }
    let bytes = fs::read(&file_path).with_context(|| format!("reading {}", file_path.display()))?;
    read_litedb_documents(&bytes).with_context(|| format!("parsing {}", file_path.display()))
}

fn doc_id(doc: &BsonDoc) -> Option<String> {
    doc.guid("_id").or_else(|| doc.guid("Id"))
}

fn name_map(docs: &[BsonDoc]) -> HashMap<String, String> {
    let mut map = HashMap::new();
    for doc in docs {
        if let (Some(id), Some(name)) = (doc_id(doc), doc.string("Name")) {
            map.insert(id, name);
        }
    }
    map
}

fn guids(values: &[Bson]) -> Vec<String> {
    values.iter().filter_map(Bson::as_guid).collect()
}

fn names_for(ids: &[Bson], lookup: &HashMap<String, String>) -> Vec<String> {
    guids(ids).iter().filter_map(|id| lookup.get(id).cloned()).collect()
}

fn parse_action(doc: &BsonDoc) -> PlayniteGameAction {
    PlayniteGameAction {
        action_type: doc.number("Type") as i64,
        name: doc.string("Name"),
        path: doc.string("Path"),
        arguments: doc.string("Arguments"),
        working_dir: doc.string("WorkingDir"),
        is_play_action: doc.boolean("IsPlayAction"),
        emulator_id: doc.guid("EmulatorId"),
        emulator_profile_id: doc.string("EmulatorProfileId"),
        additional_arguments: doc.string("AdditionalArguments"),
        override_default_args: doc.boolean("OverrideDefaultArgs"),
    }
}

fn parse_rom(doc: &BsonDoc) -> Option<PlayniteRom> {
    let path = doc.string("Path")?;
    Some(PlayniteRom { name: doc.string("Name"), path })
}

fn parse_profiles(values: &[Bson]) -> Vec<PlayniteEmulatorProfile> {
    values
        .iter()
        .filter_map(Bson::as_doc)
        .map(|doc| PlayniteEmulatorProfile {
            id: doc.string("Id").or_else(|| doc.guid("Id")),
            name: doc.string("Name"),
            executable: doc.string("Executable"),
            arguments: doc.string("Arguments"),
            working_dir: doc.string("WorkingDirectory").or_else(|| doc.string("WorkingDir")),
            startup_executable: doc.string("StartupExecutable"),
            custom_arguments: doc.string("CustomArguments"),
        })
        .collect()
}

fn parse_emulator(doc: &BsonDoc) -> Option<PlayniteEmulator> {
    let id = doc_id(doc)?;
    let mut profiles = parse_profiles(doc.array("CustomProfiles"));
    profiles.extend(parse_profiles(doc.array("BuiltinProfiles")));
    Some(PlayniteEmulator {
        id,
        name: doc.string("Name"),
        install_dir: doc.string("InstallDir"),
        profiles,
    })
}

/// `parseReleaseDate`: a string as is, or `{Year, Month, Day}`.
fn parse_release_date(value: Option<&Bson>) -> Option<String> {
    let value = value?;
    if let Some(direct) = value.as_string() {
        return Some(direct);
    }
    let doc = value.as_doc()?;
    let year = doc.number("Year");
    if year == 0.0 {
        return None;
    }
    let year_text = crate::litedb::js_number_to_string(year);
    let month = doc.number("Month");
    if month == 0.0 {
        return Some(year_text);
    }
    let day = doc.number("Day");
    if day == 0.0 {
        return Some(format!("{year_text}-{:02}", month as i64));
    }
    Some(format!("{year_text}-{:02}-{:02}", month as i64, day as i64))
}

struct NameMaps {
    companies: HashMap<String, String>,
    genres: HashMap<String, String>,
    tags: HashMap<String, String>,
    features: HashMap<String, String>,
    platforms: HashMap<String, String>,
    series: HashMap<String, String>,
}

fn parse_game(doc: &BsonDoc, maps: &NameMaps) -> Option<PlayniteGame> {
    let id = doc_id(doc)?;
    let name = doc.string("Name")?;
    let developer_ids = guids(doc.array("DeveloperIds"));
    Some(PlayniteGame {
        id,
        name,
        game_id: doc.string("GameId"),
        plugin_id: doc.guid("PluginId").unwrap_or_else(|| EMPTY_GUID.to_owned()),
        source_id: doc.guid("SourceId"),
        install_directory: doc.string("InstallDirectory"),
        is_installed: doc.boolean("IsInstalled"),
        playtime_seconds: doc.number("Playtime"),
        play_count: doc.number("PlayCount"),
        last_activity: doc.string("LastActivity"),
        added: doc.string("Added"),
        notes: doc.string("Notes"),
        description: doc.string("Description"),
        cover_image: doc.string("CoverImage"),
        background_image: doc.string("BackgroundImage"),
        icon: doc.string("Icon"),
        release_date: parse_release_date(doc.get("ReleaseDate")).or_else(|| doc.string("ReleaseYear")),
        developers: developer_ids.iter().filter_map(|d| maps.companies.get(d).cloned()).collect(),
        developer_ids,
        publishers: names_for(doc.array("PublisherIds"), &maps.companies),
        genres: names_for(doc.array("GenreIds"), &maps.genres),
        tags: names_for(doc.array("TagIds"), &maps.tags),
        features: names_for(doc.array("FeatureIds"), &maps.features),
        platforms: names_for(doc.array("PlatformIds"), &maps.platforms),
        series: doc.string("Series").or_else(|| {
            maps.series.get(&doc.guid("SeriesId").unwrap_or_default()).cloned()
        }),
        game_actions: doc.array("GameActions").iter().filter_map(Bson::as_doc).map(parse_action).collect(),
        roms: doc.array("Roms").iter().filter_map(Bson::as_doc).filter_map(parse_rom).collect(),
        completion_status_id: doc.guid("CompletionStatusId"),
    })
}

fn parse_completion_status(doc: &BsonDoc) -> Option<PlayniteCompletionStatus> {
    Some(PlayniteCompletionStatus { id: doc_id(doc)?, name: doc.string("Name")? })
}

/// `resolvePlayniteRoot`: the folder holding `ExtensionsData`.
pub fn resolve_playnite_root(library_path: &Path) -> PathBuf {
    let parent = library_path.parent().map(Path::to_path_buf).unwrap_or_else(|| PathBuf::from("."));
    if parent.join("ExtensionsData").exists() {
        return parent;
    }
    if library_path.join("ExtensionsData").exists() {
        return library_path.to_path_buf();
    }
    parent
}

fn read_game_activity(playnite_root: &Path) -> HashMap<String, Vec<PlayniteActivitySession>> {
    let folder = playnite_root.join("ExtensionsData").join(GAME_ACTIVITY_ID).join("GameActivity");
    let mut out = HashMap::new();
    let Ok(entries) = fs::read_dir(&folder) else { return out };
    for entry in entries.flatten() {
        let file_name = entry.file_name().to_string_lossy().into_owned();
        if !file_name.ends_with(".json") {
            continue;
        }
        let game_id = file_name[..file_name.len() - 5].to_lowercase();
        let Ok(text) = fs::read_to_string(entry.path()) else { continue };
        // JSON.parse refuses a BOM; so does serde_json.
        let Ok(parsed) = serde_json::from_str::<Value>(&text) else { continue };
        let items = parsed.get("Items").and_then(Value::as_array).cloned().unwrap_or_default();
        let sessions = items
            .iter()
            .filter_map(|item| {
                let date = item.get("DateSession").and_then(Value::as_str).filter(|d| !d.is_empty())?;
                let elapsed = json_number(item.get("ElapsedSeconds"));
                if elapsed == 0.0 {
                    return None;
                }
                Some(PlayniteActivitySession {
                    date_session: date.to_owned(),
                    elapsed_seconds: elapsed,
                    game_action_name: item
                        .get("GameActionName")
                        .and_then(Value::as_str)
                        .map(str::to_owned),
                })
            })
            .collect();
        out.insert(game_id, sessions);
    }
    out
}

/// `Number(x) || 0`.
fn json_number(value: Option<&Value>) -> f64 {
    match value {
        Some(Value::Number(n)) => n.as_f64().filter(|f| f.is_finite()).unwrap_or(0.0),
        Some(Value::String(s)) => s.trim().parse::<f64>().ok().filter(|f| f.is_finite()).unwrap_or(0.0),
        Some(Value::Bool(true)) => 1.0,
        _ => 0.0,
    }
}

/// `loadPlayniteLibrary`.
pub fn load_playnite_library(library_path: &str) -> Result<PlayniteLibraryDump> {
    let lib = Path::new(library_path);
    let games_docs = read_db(lib, "games.db")?;
    let maps = NameMaps {
        companies: name_map(&read_db(lib, "companies.db")?),
        genres: name_map(&read_db(lib, "genres.db")?),
        tags: name_map(&read_db(lib, "tags.db")?),
        features: name_map(&read_db(lib, "features.db")?),
        platforms: name_map(&read_db(lib, "platforms.db")?),
        series: name_map(&read_db(lib, "series.db")?),
    };
    let emulators = read_db(lib, "emulators.db")?.iter().filter_map(parse_emulator).collect();
    let playnite_root = resolve_playnite_root(lib);
    let completion_statuses =
        read_db(lib, "completionstatuses.db")?.iter().filter_map(parse_completion_status).collect();
    let sessions_by_game_id = read_game_activity(&playnite_root);

    Ok(PlayniteLibraryDump {
        library_path: library_path.to_owned(),
        playnite_root: playnite_root.to_string_lossy().into_owned(),
        games: games_docs.iter().filter_map(|d| parse_game(d, &maps)).collect(),
        emulators,
        completion_statuses,
        sessions_by_game_id,
    })
}
