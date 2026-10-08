//! Writes into the overlay stores, equivalent to `stores.ts`, the Playnite
//! parts of `status.ts` and `upsertSideloadGame`/`writePlaytime` of
//! `import.ts`. All go through `heronite_data::json_store`.

use std::collections::HashMap;

use anyhow::Result;
use heronite_data::json_store;
use heronite_data::models::*;
use heronite_data::status::default_statuses;
use heronite_data::{HeroicPaths, JsNum};
use serde::Deserialize;

use crate::litedb::js_number_to_string;
use crate::mapper::normalize_title;
use crate::remap::DriveRemap;

// ---------------------------------------------------------------------------
// Statuses
// ---------------------------------------------------------------------------

/// `/^(\d+)\s*[.)]/` -> the number.
fn leading_rank(name: &str) -> Option<u64> {
    let digits = name.bytes().take_while(u8::is_ascii_digit).count();
    if digits == 0 {
        return None;
    }
    let rest = name[digits..].trim_start();
    if rest.starts_with(['.', ')']) {
        name[..digits].parse().ok()
    } else {
        None
    }
}

/// `preferStatusName`.
fn prefer_status_name(current: &str, incoming: &str) -> String {
    let incoming_ranked = leading_rank(incoming).is_some();
    let current_ranked = leading_rank(current).is_some();
    if incoming_ranked || !current_ranked { incoming.to_owned() } else { current.to_owned() }
}

/// `mergePlayniteStatuses` on an in-memory list (stored order, defaults when
/// empty).
pub fn merge_playnite_statuses_into(statuses: &mut Vec<CompletionStatus>, incoming: &[(String, String)]) {
    use heronite_data::status::slug_from_status_name;
    if statuses.is_empty() {
        *statuses = default_statuses();
    }
    for (id, name) in incoming {
        let slug = slug_from_status_name(name);
        if slug == "custom" {
            if let Some(existing) = statuses.iter_mut().find(|s| s.playnite_id.as_deref() == Some(id)) {
                existing.name = name.clone();
                existing.playnite_id = Some(id.clone());
                continue;
            }
            let sort_order = 100 + statuses.len();
            statuses.push(CompletionStatus {
                id: format!("playnite_{id}"),
                name: name.clone(),
                slug: "custom".into(),
                sort_order: JsNum(sort_order as f64),
                playnite_id: Some(id.clone()),
                playnite_ids: None,
                extra: Default::default(),
            });
            continue;
        }
        if let Some(m) = statuses.iter_mut().find(|s| s.slug == slug) {
            if m.playnite_id.is_none() {
                m.playnite_id = Some(id.clone());
            }
            let ids = m.playnite_ids.get_or_insert_with(Vec::new);
            if !ids.contains(id) {
                ids.push(id.clone());
            }
            m.name = prefer_status_name(&m.name, name);
            if let Some(order) = leading_rank(name) {
                m.sort_order = JsNum((order * 10) as f64);
            }
        }
    }
}

/// `mergePlayniteStatuses` + `setLastPlayniteLibraryPath` +
/// `setLastPlayniteDriveMap` in one read-modify-write of `statuses.json`.
pub fn save_import_context(
    paths: &HeroicPaths,
    incoming: &[(String, String)],
    library_path: &str,
    drive_map: &[DriveRemap],
) -> Result<()> {
    json_store::update::<StatusesFile, _>(&paths.statuses(), |file| {
        merge_playnite_statuses_into(&mut file.statuses, incoming);
        file.last_playnite_library_path = Some(library_path.to_owned());
        file.last_playnite_drive_map = Some(serde_json::to_value(drive_map)?);
        Ok(())
    })
}

/// `getLastPlayniteLibraryPath`.
pub fn last_playnite_library_path(paths: &HeroicPaths) -> Result<Option<String>> {
    let file: StatusesFile = json_store::read(&paths.statuses())?;
    Ok(file.last_playnite_library_path.filter(|p| !p.is_empty()))
}

/// `getLastPlayniteDriveMap`.
pub fn last_playnite_drive_map(paths: &HeroicPaths) -> Result<Vec<DriveRemap>> {
    let file: StatusesFile = json_store::read(&paths.statuses())?;
    Ok(file
        .last_playnite_drive_map
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default())
}

// ---------------------------------------------------------------------------
// Sessions
// ---------------------------------------------------------------------------

fn session_key(s: &LocalGameSession) -> String {
    format!("{}:{}", s.started_at, js_number_to_string(s.elapsed_seconds.get()))
}

/// `countNewLocalSessions`.
pub fn count_new_sessions(existing: &[LocalGameSession], incoming: &[LocalGameSession]) -> usize {
    let keys: std::collections::HashSet<String> = existing.iter().map(session_key).collect();
    incoming.iter().filter(|s| !keys.contains(&session_key(s))).count()
}

/// `mergeLocalSessions` + `newestSessions` on one game's list.
pub fn merge_sessions_into(existing: &mut Vec<LocalGameSession>, incoming: &[LocalGameSession]) {
    let mut keys: std::collections::HashSet<String> = existing.iter().map(session_key).collect();
    for s in incoming {
        if keys.insert(session_key(s)) {
            existing.push(s.clone());
        }
    }
    if existing.len() > MAX_SESSIONS_PER_GAME {
        // stable sort, newest first, keep 200
        existing.sort_by(|a, b| b.started_at.cmp(&a.started_at));
        existing.truncate(MAX_SESSIONS_PER_GAME);
    }
}

/// `getLocalSessions` order: newest first.
pub fn sorted_newest_first(sessions: &[LocalGameSession]) -> Vec<LocalGameSession> {
    let mut v = sessions.to_vec();
    v.sort_by(|a, b| b.started_at.cmp(&a.started_at));
    v
}

// ---------------------------------------------------------------------------
// Playtime (store/timestamp.json)
// ---------------------------------------------------------------------------

/// `writePlaytime` applied to a loaded timestamp file.
pub fn write_playtime_into(
    file: &mut TimestampFile,
    app_name: &str,
    playtime_minutes: f64,
    first_played: Option<&str>,
    last_played: Option<&str>,
) {
    let existing = file.get(app_name);
    let total = existing.map(|e| e.total_played.get()).unwrap_or(0.0).max(playtime_minutes);
    let non_empty = |s: Option<&str>| s.filter(|s| !s.is_empty()).map(str::to_owned);
    let mut firsts: Vec<String> = [non_empty(existing.and_then(|e| e.first_played.as_deref())), non_empty(first_played)]
        .into_iter()
        .flatten()
        .collect();
    firsts.sort();
    let mut lasts: Vec<String> = [non_empty(existing.and_then(|e| e.last_played.as_deref())), non_empty(last_played)]
        .into_iter()
        .flatten()
        .collect();
    lasts.sort();
    let first = firsts.into_iter().next().or_else(|| non_empty(first_played)).unwrap_or_default();
    let last = lasts.pop().or_else(|| non_empty(last_played)).unwrap_or_default();
    file.insert(
        app_name.to_owned(),
        TimestampEntry {
            first_played: Some(first),
            last_played: Some(last),
            total_played: JsNum(total),
            extra: Default::default(),
        },
    );
}

// ---------------------------------------------------------------------------
// Sideload library
// ---------------------------------------------------------------------------

/// `upsertSideloadGame`: `{...current[i], ...game}`. Keys the importer always
/// sets (even to undefined, which drops them) are replaced; everything else on
/// the stored entry survives.
pub fn upsert_sideload_game(file: &mut SideloadLibraryFile, game: GameInfo) {
    match file.games.iter_mut().find(|g| g.app_name == game.app_name) {
        Some(current) => {
            current.runner = game.runner;
            current.title = game.title;
            // Deviation: the TS would refetch artwork from the web right after
            // blanking it; without a fetcher, empty artwork must not erase a
            // cover the entry already has.
            let keep = |new: Option<String>, old: &Option<String>| match new {
                Some(n) if !n.is_empty() => Some(n),
                _ if old.as_deref().is_some_and(|o| !o.is_empty()) => old.clone(),
                other => other,
            };
            current.art_cover = keep(game.art_cover, &current.art_cover);
            current.art_square = keep(game.art_square, &current.art_square);
            current.developer = game.developer;
            current.description = game.description;
            current.is_installed = game.is_installed;
            current.is_linux_native = game.is_linux_native;
            current.folder_name = game.folder_name;
            current.install = game.install;
            match game.rest.get("canRunOffline") {
                Some(v) => {
                    current.rest.insert("canRunOffline".into(), v.clone());
                }
                None => {
                    current.rest.shift_remove("canRunOffline");
                }
            }
        }
        None => file.games.push(game),
    }
}

// ---------------------------------------------------------------------------
// Heroic's own store caches (legendary / gog) for Epic and GOG matching
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct Slim {
    app_name: String,
    #[serde(default)]
    title: String,
}

#[derive(Deserialize, Default)]
struct SlimLibrary {
    #[serde(default)]
    library: Vec<Slim>,
    #[serde(default)]
    games: Vec<Slim>,
}

/// Lookup tables over one store library with `matchStoreGame`'s semantics
/// (`find` = first match wins).
#[derive(Debug, Default, Clone)]
pub struct StoreIndex {
    by_id: HashMap<String, String>,
    by_title: HashMap<String, String>,
}

impl StoreIndex {
    fn from_games(games: Vec<Slim>) -> Self {
        let mut idx = StoreIndex::default();
        for g in games {
            idx.by_id.entry(g.app_name.to_lowercase()).or_insert_with(|| g.app_name.clone());
            idx.by_title.entry(normalize_title(&g.title)).or_insert(g.app_name);
        }
        idx
    }

    /// App name of the matching store game: by id (case-insensitive), then by
    /// normalised title.
    pub fn find(&self, store_game_id: Option<&str>, title: &str) -> Option<String> {
        if let Some(id) = store_game_id.filter(|s| !s.is_empty()) {
            if let Some(hit) = self.by_id.get(&id.to_lowercase()) {
                return Some(hit.clone());
            }
        }
        self.by_title.get(&normalize_title(title)).cloned()
    }
}

#[derive(Debug, Default, Clone)]
pub struct StoreIndexes {
    pub legendary: StoreIndex,
    pub gog: StoreIndex,
}

impl StoreIndexes {
    /// `store_cache/legendary_library.json` (`library`) and
    /// `store_cache/gog_library.json` (`games`). Missing files are empty.
    pub fn load(paths: &HeroicPaths) -> Self {
        let read = |p: std::path::PathBuf, gog: bool| -> StoreIndex {
            let lib: SlimLibrary = json_store::read(&p).unwrap_or_default();
            StoreIndex::from_games(if gog { lib.games } else { lib.library })
        };
        Self { legendary: read(paths.legendary_library(), false), gog: read(paths.gog_library(), true) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn st(id: &str, name: &str, slug: &str, order: i64) -> CompletionStatus {
        CompletionStatus { id: id.into(), name: name.into(), slug: slug.into(), sort_order: JsNum(order as f64), ..Default::default() }
    }

    #[test]
    fn status_merge_maps_ranked_names_and_custom() {
        let mut list = default_statuses();
        merge_playnite_statuses_into(
            &mut list,
            &[
                ("p-beaten".into(), "1. Beaten".into()),
                ("p-endless".into(), "3. On Going".into()),
                ("p-endless2".into(), "Endless".into()),
                ("p-wish".into(), "Wishlist".into()),
                ("p-wish".into(), "Wishlist 2".into()),
            ],
        );
        let beaten = list.iter().find(|s| s.id == "beaten").unwrap();
        assert_eq!(beaten.name, "1. Beaten");
        assert_eq!(beaten.sort_order.get(), 10.0);
        assert_eq!(beaten.playnite_id.as_deref(), Some("p-beaten"));
        let endless = list.iter().find(|s| s.id == "endless").unwrap();
        assert_eq!(endless.name, "3. On Going"); // a plain name does not replace a ranked one
        assert_eq!(endless.playnite_ids.as_ref().unwrap(), &["p-endless", "p-endless2"]);
        let wish = list.iter().find(|s| s.id == "playnite_p-wish").unwrap();
        assert_eq!(wish.name, "Wishlist 2");
        assert_eq!(wish.sort_order.get(), 109.0);
        assert_eq!(list.len(), 10);
    }

    #[test]
    fn leading_rank_matches_the_regex() {
        assert_eq!(leading_rank("12 ) x"), Some(12));
        assert_eq!(leading_rank("3.Played"), Some(3));
        assert_eq!(leading_rank("3 Played"), None);
        assert_eq!(leading_rank("Played"), None);
        let _ = st("a", "b", "c", 1);
    }

    fn sess(at: &str, secs: f64) -> LocalGameSession {
        LocalGameSession { started_at: at.into(), ended_at: at.into(), elapsed_seconds: JsNum(secs), source: "playnite".into(), ..Default::default() }
    }

    #[test]
    fn sessions_dedupe_and_cap() {
        let mut existing = vec![sess("2024-01-01T00:00:00.000Z", 60.0)];
        let incoming = vec![sess("2024-01-01T00:00:00.000Z", 60.0), sess("2024-01-02T00:00:00.000Z", 30.0)];
        assert_eq!(count_new_sessions(&existing, &incoming), 1);
        merge_sessions_into(&mut existing, &incoming);
        assert_eq!(existing.len(), 2);

        let many: Vec<_> = (0..250).map(|i| sess(&format!("2024-02-{:02}T00:00:{:02}.000Z", 1 + i / 60, i % 60), 10.0)).collect();
        let mut v = Vec::new();
        merge_sessions_into(&mut v, &many);
        assert_eq!(v.len(), MAX_SESSIONS_PER_GAME);
        assert!(v[0].started_at >= v[199].started_at);
    }

    #[test]
    fn playtime_takes_max_min_max() {
        let mut f = TimestampFile::new();
        write_playtime_into(&mut f, "a", 10.0, Some("2024-05-01T00:00:00.000Z"), Some("2024-06-01T00:00:00.000Z"));
        write_playtime_into(&mut f, "a", 5.0, Some("2024-04-01T00:00:00.000Z"), Some("2024-05-15T00:00:00.000Z"));
        let e = &f["a"];
        assert_eq!(e.total_played.get(), 10.0);
        assert_eq!(e.first_played.as_deref(), Some("2024-04-01T00:00:00.000Z"));
        assert_eq!(e.last_played.as_deref(), Some("2024-06-01T00:00:00.000Z"));
        write_playtime_into(&mut f, "b", 0.0, None, None);
        assert_eq!(f["b"].first_played.as_deref(), Some(""));
    }

    #[test]
    fn sideload_upsert_replaces_importer_keys_only() {
        let mut file = SideloadLibraryFile::default();
        let mut old = GameInfo { runner: "sideload".into(), app_name: "g".into(), title: "Old".into(), developer: Some("Dev".into()), art_background: Some("bg".into()), ..Default::default() };
        old.art_cover = Some("http://x/cover".into());
        file.games.push(old);
        let new = GameInfo { runner: "sideload".into(), app_name: "g".into(), title: "New".into(), art_cover: Some(String::new()), ..Default::default() };
        upsert_sideload_game(&mut file, new);
        let g = &file.games[0];
        assert_eq!(g.title, "New");
        assert_eq!(g.art_cover.as_deref(), Some("http://x/cover")); // empty art keeps the stored one
        assert_eq!(g.developer, None);
        assert_eq!(g.art_background.as_deref(), Some("bg"));
        let other = GameInfo { runner: "sideload".into(), app_name: "h".into(), ..Default::default() };
        upsert_sideload_game(&mut file, other);
        assert_eq!(file.games.len(), 2);
    }

    #[test]
    fn store_index_matches_id_then_title() {
        let idx = StoreIndex::from_games(vec![
            Slim { app_name: "AbCd".into(), title: "The Game: Part 1".into() },
            Slim { app_name: "zzzz".into(), title: "Other".into() },
        ]);
        assert_eq!(idx.find(Some("abcd"), "nope").as_deref(), Some("AbCd"));
        assert_eq!(idx.find(Some("unknown"), "the game part 1!").as_deref(), Some("AbCd"));
        assert_eq!(idx.find(None, "missing"), None);
    }
}
