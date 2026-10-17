//! Completion statuses: port of `src/local-library/status.ts` and
//! `Collection/statusColors.ts`.
//!
//! Reads are pure; mutations go through [`json_store::update`] on
//! `local_library/statuses.json` / `library.json`, so what Heronite wrote
//! meanwhile (and fields this crate does not model) survives.

use std::cmp::Ordering;

use anyhow::Result;
use heronite_i18n::n_;

use crate::json_store;
use crate::models::{CompletionStatus, LocalGameMeta, LocalMetaFile, StatusesFile};
use crate::num::JsNum;
use crate::paths::DataPaths;
use crate::text::locale_cmp;

/// Slug id of the group collecting games whose status id is unknown.
pub const UNKNOWN_GROUP_ID: &str = "ungrouped";

pub const SLUGS: [&str; 10] =
    ["playing", "plan-to-play", "on-hold", "played", "endless", "beaten", "completed", "abandoned", "not-played", "custom"];

/// `STATUS_COLORS` of `statusColors.ts` (`None` for an unknown slug, like the
/// `undefined` the TS produces).
pub fn status_color(slug: &str) -> Option<&'static str> {
    Some(match slug {
        "playing" => "#3ecf8e",
        "plan-to-play" => "#6cb6ff",
        "on-hold" => "#f0a202",
        "played" => "#8b9cff",
        "endless" => "#2ec4b6",
        "beaten" => "#f4d35e",
        "completed" => "#c084fc",
        "abandoned" => "#9aa0a6",
        "not-played" => "#6b7280",
        "custom" => "#e5e7eb",
        _ => return None,
    })
}

/// Same colors as `(r, g, b)`.
pub fn status_color_rgb(slug: &str) -> Option<(u8, u8, u8)> {
    let hex = status_color(slug)?.trim_start_matches('#');
    let v = u32::from_str_radix(hex, 16).ok()?;
    Some(((v >> 16) as u8, (v >> 8) as u8, v as u8))
}

fn status(id: &str, name: &str, slug: &str, order: i64) -> CompletionStatus {
    CompletionStatus {
        id: id.into(),
        name: name.into(),
        slug: slug.into(),
        sort_order: JsNum(order as f64),
        playnite_id: None,
        playnite_ids: None,
        extra: Default::default(),
    }
}

/// The statuses of a fresh install. Same ids and slugs as Heronite's
/// `DEFAULT_STATUSES` (so its colours and Playnite mapping apply), in this
/// app's own order.
pub fn default_statuses() -> Vec<CompletionStatus> {
    vec![
        status("beaten", n_!("Beaten"), "beaten", 10),
        status("playing", n_!("Playing"), "playing", 20),
        status("endless", n_!("On Going"), "endless", 30),
        status("plan-to-play", n_!("Plan to Play"), "plan-to-play", 40),
        status("played", n_!("Played"), "played", 50),
        status("on-hold", n_!("On Hold"), "on-hold", 60),
        status("not-played", n_!("Not Played"), "not-played", 70),
        status("completed", n_!("Completed"), "completed", 80),
        status("abandoned", n_!("Abandoned"), "abandoned", 90),
    ]
}

/// A status name as shown: the default ones follow the interface language,
/// the ones the user typed stay as they are.
pub fn status_label(name: &str) -> &str {
    const DEFAULT_NAMES: [&str; 9] =
        ["Beaten", "Playing", "On Going", "Plan to Play", "Played", "On Hold", "Not Played", "Completed", "Abandoned"];
    if DEFAULT_NAMES.contains(&name) {
        heronite_i18n::tr(name)
    } else {
        name
    }
}

/// `normalizeStatusName`.
fn normalize_status_name(name: &str) -> String {
    use unicode_normalization::UnicodeNormalization;
    let stripped: String = name.nfd().filter(|c| !('\u{0300}'..='\u{036f}').contains(c)).collect();
    // /^\d+\s*[.)-]?\s*/
    let digits = stripped.bytes().take_while(u8::is_ascii_digit).count();
    let mut rest = stripped.as_str();
    if digits > 0 {
        rest = rest[digits..].trim_start();
        if let Some(r) = rest.strip_prefix(['.', ')', '-']) {
            rest = r.trim_start();
        }
    }
    let lowered = rest.to_lowercase();
    lowered.split(|c: char| !c.is_ascii_alphanumeric()).filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" ")
}

/// `slugFromStatusName`: maps a Playnite status name (English/Portuguese
/// aliases, optional "3. " ranking prefix) to a slug, `custom` otherwise.
pub fn slug_from_status_name(name: &str) -> &'static str {
    match normalize_status_name(name).as_str() {
        "playing" | "jogando" => "playing",
        "plan to play" | "planejo jogar" => "plan-to-play",
        "on hold" | "em espera" | "em pausa" => "on-hold",
        "played" | "jogado" => "played",
        "endless" | "continuous" | "on going" | "ongoing" | "aendless" | "uso continuo" => "endless",
        "beaten" | "zerei" | "zerado" => "beaten",
        "completed" | "completado" => "completed",
        "abandoned" | "abandonado" => "abandoned",
        "not played" | "nao jogado" => "not-played",
        _ => "custom",
    }
}

/// `getCompletionStatuses` ordering: `sortOrder`, then name (`localeCompare`).
pub fn sort_statuses(statuses: &mut [CompletionStatus]) {
    statuses.sort_by(|a, b| {
        a.sort_order.get().partial_cmp(&b.sort_order.get()).unwrap_or(Ordering::Equal).then_with(|| locale_cmp(&a.name, &b.name))
    });
}

/// The statuses in display order. An empty file yields the defaults
/// (Heronite writes them on first read; this does not touch the disk).
pub fn load_statuses(paths: &DataPaths) -> Result<Vec<CompletionStatus>> {
    let file: StatusesFile = json_store::read(&paths.statuses())?;
    Ok(sorted_or_default(file.statuses))
}

pub(crate) fn sorted_or_default(mut statuses: Vec<CompletionStatus>) -> Vec<CompletionStatus> {
    if statuses.is_empty() {
        statuses = default_statuses();
    }
    sort_statuses(&mut statuses);
    statuses
}

/// `slugForMeta` of `Collection/index.tsx`: the id used for grouping.
/// A game without a stored status goes to the first "not-played" status.
pub fn resolve_status_id(stored: Option<&str>, statuses: &[CompletionStatus]) -> String {
    if let Some(id) = stored.filter(|s| !s.is_empty()) {
        return id.to_owned();
    }
    statuses
        .iter()
        .find(|s| s.slug == "not-played")
        .or_else(|| statuses.first())
        .map(|s| s.id.clone())
        .unwrap_or_else(|| "not-played".into())
}

/// The status shown by the Focus panel / card dot: `completionStatusId` (or
/// `not-played`) looked up by id; the Focus falls back to the first
/// `not-played` slug, the card to the one with id `not-played`.
pub fn display_status<'a>(stored: Option<&str>, statuses: &'a [CompletionStatus]) -> Option<&'a CompletionStatus> {
    let id = stored.filter(|s| !s.is_empty()).unwrap_or("not-played");
    statuses.iter().find(|s| s.id == id).or_else(|| statuses.iter().find(|s| s.slug == "not-played"))
}

/// `fallbackStatusId`.
pub fn fallback_status_id(preferred_slug: &str, statuses: &[CompletionStatus]) -> String {
    statuses
        .iter()
        .find(|s| s.slug == preferred_slug)
        .or_else(|| statuses.iter().find(|s| s.id == preferred_slug))
        .or_else(|| statuses.first())
        .map(|s| s.id.clone())
        .unwrap_or_else(|| preferred_slug.to_owned())
}

/// `inferredStatusId`: played if there is playtime, not-played otherwise.
pub fn inferred_status_id(playtime_minutes: f64, statuses: &[CompletionStatus]) -> String {
    fallback_status_id(if playtime_minutes > 0.0 { "played" } else { "not-played" }, statuses)
}

fn maps_playnite(status: &CompletionStatus, playnite_id: &str) -> bool {
    status.playnite_id.as_deref() == Some(playnite_id)
        || status.playnite_ids.as_ref().is_some_and(|ids| ids.iter().any(|i| i == playnite_id))
}

/// `statusIdForPlaynite`.
pub fn status_id_for_playnite(playnite_status_id: Option<&str>, statuses: &[CompletionStatus]) -> String {
    if let Some(pid) = playnite_status_id.filter(|s| !s.is_empty()) {
        if let Some(m) = statuses.iter().find(|s| maps_playnite(s, pid)) {
            return m.id.clone();
        }
    }
    fallback_status_id("not-played", statuses)
}

/// `resolvePlayniteStatusId`: `incoming` are the Playnite statuses
/// `(id, name)` of the library being imported.
pub fn resolve_playnite_status_id(
    playnite_status_id: Option<&str>,
    incoming: &[(String, String)],
    statuses: &[CompletionStatus],
) -> String {
    let Some(pid) = playnite_status_id.filter(|s| !s.is_empty()) else {
        return fallback_status_id("not-played", statuses);
    };
    if let Some(m) = statuses.iter().find(|s| maps_playnite(s, pid)) {
        return m.id.clone();
    }
    if let Some((_, name)) = incoming.iter().find(|(id, _)| id == pid) {
        let slug = slug_from_status_name(name);
        if slug != "custom" {
            if let Some(by_slug) = statuses.iter().find(|s| s.slug == slug) {
                return by_slug.id.clone();
            }
        }
    }
    fallback_status_id("not-played", statuses)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatusMergeDecision {
    Unchanged,
    TakePlaynite,
    Conflict,
}

/// `decideStatusMerge`.
pub fn decide_status_merge(
    local_status: Option<&str>,
    last_playnite_status: Option<&str>,
    playnite_status: Option<&str>,
    from_playnite: &str,
    statuses: &[CompletionStatus],
) -> (String, StatusMergeDecision) {
    let Some(local) = local_status.filter(|s| !s.is_empty()) else {
        return (from_playnite.to_owned(), StatusMergeDecision::TakePlaynite);
    };
    if playnite_status == last_playnite_status {
        return (local.to_owned(), StatusMergeDecision::Unchanged);
    }
    let last_imported = last_playnite_status.filter(|s| !s.is_empty()).map(|s| status_id_for_playnite(Some(s), statuses));
    if last_imported.as_deref() == Some(local) {
        let decision = if from_playnite == local { StatusMergeDecision::Unchanged } else { StatusMergeDecision::TakePlaynite };
        return (from_playnite.to_owned(), decision);
    }
    let decision = if from_playnite == local { StatusMergeDecision::Unchanged } else { StatusMergeDecision::Conflict };
    (local.to_owned(), decision)
}

/// Extra data for [`set_game_status`] when the game has no overlay entry yet.
#[derive(Debug, Clone, Default)]
pub struct SetStatusExtras {
    /// Runner of the game (`zoom` is stored as `sideload`).
    pub runner: Option<String>,
    pub title: Option<String>,
    pub playnite_id: Option<String>,
}

/// `setGameCompletionStatus`: sets `completionStatusId` on the overlay entry of
/// `app_name`. A game without an entry (an Epic game never imported, say) gets
/// a minimal one, exactly like Heronite: `playniteId = heroic_<appName>`,
/// `source = other`, `launchKind = unavailable`. Returns the stored entry.
pub fn set_game_status(paths: &DataPaths, app_name: &str, status_id: &str, extras: &SetStatusExtras) -> Result<LocalGameMeta> {
    json_store::update::<LocalMetaFile, _>(&paths.local_meta(), |file| Ok(put_status(file, app_name, status_id, extras)))
}

/// [`set_game_status`] for many games in one write: `(app_name, status_id,
/// extras)`, each game its own status (an undo puts back different ones).
pub fn set_games_status(paths: &DataPaths, games: &[(String, String, SetStatusExtras)]) -> Result<()> {
    if games.is_empty() {
        return Ok(());
    }
    json_store::update::<LocalMetaFile, _>(&paths.local_meta(), |file| {
        for (app_name, status_id, extras) in games {
            put_status(file, app_name, status_id, extras);
        }
        Ok(())
    })
}

fn put_status(file: &mut LocalMetaFile, app_name: &str, status_id: &str, extras: &SetStatusExtras) -> LocalGameMeta {
    if let Some(existing) = file.games.get_mut(app_name) {
        existing.completion_status_id = Some(status_id.to_owned());
        return existing.clone();
    }
    let runner = match extras.runner.as_deref() {
        Some("zoom") | None => "sideload",
        Some(other) => other,
    };
    let meta = LocalGameMeta {
        app_name: app_name.to_owned(),
        runner: runner.to_owned(),
        playnite_id: extras.playnite_id.clone().unwrap_or_else(|| format!("heroic_{app_name}")),
        source: "other".into(),
        launch_kind: "unavailable".into(),
        title: extras.title.clone().unwrap_or_else(|| app_name.to_owned()),
        completion_status_id: Some(status_id.to_owned()),
        ..Default::default()
    };
    file.games.insert(app_name.to_owned(), meta.clone());
    meta
}

/// `upsertCompletionStatus`: replaces the status with the same id or appends.
/// Returns the new list in display order.
pub fn upsert_status(paths: &DataPaths, status: CompletionStatus) -> Result<Vec<CompletionStatus>> {
    json_store::update::<StatusesFile, _>(&paths.statuses(), |file| {
        let mut statuses = sorted_or_default(std::mem::take(&mut file.statuses));
        match statuses.iter().position(|s| s.id == status.id) {
            Some(i) => statuses[i] = status,
            None => statuses.push(status),
        }
        file.statuses = statuses;
        Ok(sorted_or_default(file.statuses.clone()))
    })
}

/// A user-created status (slug `custom`) placed after the existing ones.
pub fn new_custom_status(name: &str, statuses: &[CompletionStatus]) -> CompletionStatus {
    let max = statuses.iter().map(|s| s.sort_order.get()).fold(0.0, f64::max);
    let id =
        format!("custom_{}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or_default());
    CompletionStatus {
        id,
        name: name.trim().to_owned(),
        slug: "custom".into(),
        sort_order: JsNum(max + 10.0),
        playnite_id: None,
        playnite_ids: None,
        extra: Default::default(),
    }
}

/// Renames the status `id`. `None` when it does not exist.
pub fn rename_status(paths: &DataPaths, id: &str, name: &str) -> Result<Option<Vec<CompletionStatus>>> {
    let current = load_statuses(paths)?;
    let Some(mut status) = current.into_iter().find(|s| s.id == id) else {
        return Ok(None);
    };
    status.name = name.trim().to_owned();
    upsert_status(paths, status).map(Some)
}

/// `deleteCompletionStatus`: the last status cannot be deleted; games using
/// the deleted one move to "not-played" (or the first other status).
pub fn delete_status(paths: &DataPaths, id: &str) -> Result<Vec<CompletionStatus>> {
    enum Outcome {
        Unchanged(Vec<CompletionStatus>),
        Deleted { statuses: Vec<CompletionStatus>, fallback: String },
    }
    let outcome = json_store::update::<StatusesFile, _>(&paths.statuses(), |file| {
        let statuses = sorted_or_default(std::mem::take(&mut file.statuses));
        if statuses.len() <= 1 {
            file.statuses = statuses.clone();
            return Ok(Outcome::Unchanged(statuses));
        }
        let fallback = statuses
            .iter()
            .find(|s| s.id != id && s.slug == "not-played")
            .or_else(|| statuses.iter().find(|s| s.id != id))
            .map(|s| s.id.clone());
        let Some(fallback) = fallback else {
            file.statuses = statuses.clone();
            return Ok(Outcome::Unchanged(statuses));
        };
        let remaining: Vec<_> = statuses.into_iter().filter(|s| s.id != id).collect();
        file.statuses = remaining.clone();
        Ok(Outcome::Deleted { statuses: remaining, fallback })
    })?;
    match outcome {
        Outcome::Unchanged(statuses) => Ok(statuses),
        Outcome::Deleted { statuses, fallback } => {
            json_store::update::<LocalMetaFile, _>(&paths.local_meta(), |file| {
                for meta in file.games.values_mut() {
                    if meta.completion_status_id.as_deref() == Some(id) {
                        meta.completion_status_id = Some(fallback.clone());
                    }
                }
                Ok(())
            })?;
            Ok(sorted_or_default(statuses))
        }
    }
}

/// `reorderCompletionStatuses`: `ids` first (in that order), then the rest in
/// their current order; `sortOrder` renumbered 10, 20, 30...
pub fn reorder_statuses(paths: &DataPaths, ids: &[String]) -> Result<Vec<CompletionStatus>> {
    json_store::update::<StatusesFile, _>(&paths.statuses(), |file| {
        let statuses = sorted_or_default(std::mem::take(&mut file.statuses));
        let mut next: Vec<CompletionStatus> = Vec::with_capacity(statuses.len());
        let mut seen = std::collections::HashSet::new();
        for id in ids {
            let Some(item) = statuses.iter().find(|s| &s.id == id) else { continue };
            if !seen.insert(id.clone()) {
                continue;
            }
            let mut item = item.clone();
            item.sort_order = JsNum(((next.len() + 1) * 10) as f64);
            next.push(item);
        }
        for item in &statuses {
            if seen.contains(&item.id) {
                continue;
            }
            let mut item = item.clone();
            item.sort_order = JsNum(((next.len() + 1) * 10) as f64);
            next.push(item);
        }
        file.statuses = next;
        Ok(sorted_or_default(file.statuses.clone()))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn ids(s: &[CompletionStatus]) -> Vec<&str> {
        s.iter().map(|s| s.id.as_str()).collect()
    }

    #[test]
    fn aliases_resolve_to_slugs() {
        assert_eq!(slug_from_status_name("2. Playing"), "playing");
        assert_eq!(slug_from_status_name("3. On Going"), "endless");
        assert_eq!(slug_from_status_name("Não Jogado"), "not-played");
        assert_eq!(slug_from_status_name("Uso contínuo"), "endless");
        assert_eq!(slug_from_status_name("1) Zerei"), "beaten");
        assert_eq!(slug_from_status_name("Wishlist"), "custom");
    }

    #[test]
    fn default_order() {
        let mut s = default_statuses();
        sort_statuses(&mut s);
        assert_eq!(ids(&s), ["beaten", "playing", "endless", "plan-to-play", "played", "on-hold", "not-played", "completed", "abandoned"]);
        assert!(s.iter().all(|s| !s.name.starts_with(|c: char| c.is_ascii_digit())));
    }

    #[test]
    fn resolution_follows_slug_for_meta() {
        let s = default_statuses();
        assert_eq!(resolve_status_id(Some("beaten"), &s), "beaten");
        assert_eq!(resolve_status_id(None, &s), "not-played");
        assert_eq!(resolve_status_id(Some(""), &s), "not-played");
        assert_eq!(resolve_status_id(None, &[]), "not-played");
        assert_eq!(display_status(Some("nope"), &s).unwrap().id, "not-played");
        assert_eq!(inferred_status_id(5.0, &s), "played");
        assert_eq!(status_color("beaten"), Some("#f4d35e"));
        assert_eq!(status_color_rgb("playing"), Some((0x3e, 0xcf, 0x8e)));
    }

    #[test]
    fn merge_decisions() {
        let mut s = default_statuses();
        s[1].playnite_id = Some("p-playing".into());
        // No Heroic status yet: take Playnite's.
        let (id, d) = decide_status_merge(None, None, Some("p-playing"), "playing", &s);
        assert_eq!((id.as_str(), d), ("playing", StatusMergeDecision::TakePlaynite));
        // Playnite unchanged since last import: keep Heroic's.
        let (id, d) = decide_status_merge(Some("beaten"), Some("p-playing"), Some("p-playing"), "playing", &s);
        assert_eq!((id.as_str(), d), ("beaten", StatusMergeDecision::Unchanged));
        // Heroic differs from last import and Playnite moved: conflict.
        let (id, d) = decide_status_merge(Some("beaten"), Some("p-playing"), Some("p-x"), "played", &s);
        assert_eq!((id.as_str(), d), ("beaten", StatusMergeDecision::Conflict));
    }

    fn temp_paths() -> (tempfile::TempDir, DataPaths) {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path());
        fs::create_dir_all(paths.local_library()).unwrap();
        (dir, paths)
    }

    #[test]
    fn set_status_creates_minimal_entry_like_heronite() {
        let (_d, paths) = temp_paths();
        let meta = set_game_status(
            &paths,
            "Calluna",
            "beaten",
            &SetStatusExtras { runner: Some("legendary".into()), title: Some("Control".into()), playnite_id: None },
        )
        .unwrap();
        assert_eq!(meta.playnite_id, "heroic_Calluna");
        let v = json_store::read_value(&paths.local_meta()).unwrap();
        let e = &v["games"]["Calluna"];
        assert_eq!(e["runner"], "legendary");
        assert_eq!(e["source"], "other");
        assert_eq!(e["launchKind"], "unavailable");
        assert_eq!(e["title"], "Control");
        assert_eq!(e["completionStatusId"], "beaten");
        assert_eq!(e["playniteId"], "heroic_Calluna");
        // Second call only updates the status and keeps the rest.
        set_game_status(&paths, "Calluna", "played", &SetStatusExtras::default()).unwrap();
        let v = json_store::read_value(&paths.local_meta()).unwrap();
        assert_eq!(v["games"]["Calluna"]["completionStatusId"], "played");
        assert_eq!(v["games"]["Calluna"]["title"], "Control");
    }

    #[test]
    fn many_games_get_their_status_in_one_write() {
        let (_d, paths) = temp_paths();
        set_game_status(&paths, "a", "playing", &SetStatusExtras::default()).unwrap();
        let extras = SetStatusExtras { runner: Some("zoom".into()), title: Some("B".into()), playnite_id: None };
        set_games_status(&paths, &[("a".into(), "beaten".into(), SetStatusExtras::default()), ("b".into(), "played".into(), extras)])
            .unwrap();
        let v = json_store::read_value(&paths.local_meta()).unwrap();
        assert_eq!(v["games"]["a"]["completionStatusId"], "beaten");
        assert_eq!(v["games"]["b"]["completionStatusId"], "played");
        assert_eq!(v["games"]["b"]["runner"], "sideload");
        assert_eq!(v["games"]["b"]["title"], "B");
        set_games_status(&paths, &[]).unwrap();
    }

    #[test]
    fn status_crud_round_trip() {
        let (_d, paths) = temp_paths();
        assert_eq!(load_statuses(&paths).unwrap().len(), 9);
        let custom = new_custom_status("Wishlist", &load_statuses(&paths).unwrap());
        let cid = custom.id.clone();
        let list = upsert_status(&paths, custom).unwrap();
        assert_eq!(list.len(), 10);
        assert_eq!(list.last().unwrap().id, cid);

        let list = rename_status(&paths, &cid, " Backlog ").unwrap().unwrap();
        assert_eq!(list.last().unwrap().name, "Backlog");
        assert!(rename_status(&paths, "missing", "x").unwrap().is_none());

        let order: Vec<String> = ["beaten", "playing"].iter().map(|s| s.to_string()).collect();
        let list = reorder_statuses(&paths, &order).unwrap();
        assert_eq!(&ids(&list)[..2], ["beaten", "playing"]);
        assert_eq!(list[0].sort_order.get(), 10.0);
        assert_eq!(list[1].sort_order.get(), 20.0);

        set_game_status(&paths, "g1", &cid, &SetStatusExtras::default()).unwrap();
        let list = delete_status(&paths, &cid).unwrap();
        assert_eq!(list.len(), 9);
        let v = json_store::read_value(&paths.local_meta()).unwrap();
        assert_eq!(v["games"]["g1"]["completionStatusId"], "not-played");
    }

    #[test]
    fn last_status_cannot_be_deleted() {
        let (_d, paths) = temp_paths();
        let only = default_statuses().remove(0);
        json_store::write(&paths.statuses(), &StatusesFile { statuses: vec![only], ..Default::default() }).unwrap();
        let list = delete_status(&paths, "beaten").unwrap();
        assert_eq!(list.len(), 1);
    }

    #[test]
    fn real_statuses_sort_like_heronite() {
        let Some(root) = std::env::var_os("HERONITE_FIXTURE") else { return };
        let list = load_statuses(&DataPaths::at(root)).unwrap();
        let names: Vec<_> = list.iter().map(|s| s.name.as_str()).collect();
        eprintln!("statuses: {names:?}");
        assert!(list.windows(2).all(|w| w[0].sort_order.get() <= w[1].sort_order.get()));
    }
}
