//! What the Focus panel shows for a game: the fallback chains of
//! `CollectionFocus.tsx` (Collection metadata, then Steam store details, then
//! the store's own `extra` info, then the wiki cache).
//!
//! [`Details::base`] (used when the library loads) has no Steam or wiki data.
//! [`focus_details`] reads the Steam details cache of the focused game, which
//! is the only per-game file read, and merges it in.

use serde::Deserialize;
use serde_json::Value;

use crate::collection::format_playtime_minutes;
use crate::hltb::{HltbEntry, WikiCache, WikiEntry};
use crate::library::{CollectionGame, Library};
use crate::models::{CollectionGameMetadata, GameInfo, LocalGameMeta};
use crate::paths::DataPaths;
use crate::text::{format_timestamp, split_names, to_plain_text, DateLocale};

/// `local_library/steam-details/{appId}.{lang}.json` (the `CollectionSteamDetails`
/// of the TS plus `fetchedAt`, ignored here).
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SteamDetails {
    #[serde(default)]
    pub steam_app_id: String,
    #[serde(default)]
    pub language: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub short_description: String,
    /// Raw Steam HTML (not sanitized).
    #[serde(default)]
    pub description_html: String,
    #[serde(default)]
    pub developers: Vec<String>,
    #[serde(default)]
    pub publishers: Vec<String>,
    #[serde(default)]
    pub genres: Vec<String>,
    #[serde(default)]
    pub features: Vec<String>,
    #[serde(default)]
    pub release_date: Option<String>,
    #[serde(default)]
    pub website: Option<String>,
    #[serde(default)]
    pub store_url: String,
}

/// `steamLanguage` of `steam-details.ts`: Heroic language to Steam language name.
pub fn steam_language(language: &str) -> &'static str {
    fn lookup(key: &str) -> Option<&'static str> {
        Some(match key {
            "en" | "en-US" => "english",
            "pt" | "pt-PT" => "portuguese",
            "pt-BR" | "pt_BR" => "brazilian",
            "es" => "spanish",
            "fr" => "french",
            "de" => "german",
            "it" => "italian",
            "ja" => "japanese",
            "ko" => "koreana",
            "ru" => "russian",
            "pl" => "polish",
            "nl" => "dutch",
            "sv" => "swedish",
            "tr" => "turkish",
            "uk" => "ukrainian",
            "hu" => "hungarian",
            "cs" => "czech",
            "zh" | "zh_Hans" | "zh-Hans" => "schinese",
            "zh_Hant" | "zh-Hant" => "tchinese",
            _ => return None,
        })
    }
    let raw = language.trim();
    let raw = if raw.is_empty() { "en" } else { raw };
    if let Some(v) = lookup(raw) {
        return v;
    }
    let dashed = raw.replacen('_', "-", 1);
    if let Some(v) = lookup(&dashed) {
        return v;
    }
    dashed.split('-').next().and_then(lookup).unwrap_or("english")
}

/// Reads a cached Steam details file regardless of its age (Heronite refetches
/// after 30 days, this crate never goes to the network).
pub fn load_steam_details(paths: &DataPaths, steam_app_id: &str, language: &str) -> Option<SteamDetails> {
    let lang = steam_language(language);
    let bytes = std::fs::read(paths.steam_details(steam_app_id, lang)).ok()?;
    serde_json::from_slice(&bytes).ok()
}

/// Everything the Focus panel's header, overview and details list display.
#[derive(Debug, Clone, Default)]
pub struct Details {
    /// As stored (`2024-09-19`, `19 Sep, 2022`...); see [`Details::meta_line`].
    pub release_date: Option<String>,
    /// First platform, `PC` when unknown.
    pub platform: String,
    pub platforms: Vec<String>,
    /// Shown under "Tags".
    pub genres: Vec<String>,
    pub developers: Vec<String>,
    pub publishers: Vec<String>,
    pub features: Vec<String>,
    pub themes: Vec<String>,
    pub game_modes: Vec<String>,
    pub series: Option<String>,
    pub critic_score: Option<f64>,
    /// Plain-text overview.
    pub description: String,
    /// Raw Steam HTML, only when the Focus shows it instead of `description`
    /// (needs sanitizing before rendering).
    pub description_html: Option<String>,
    /// `metadata.notes || meta.notes`.
    pub notes: Option<String>,
    /// `meta.remappedExecutable || meta.windowsInstallDirectory ||
    /// install.install_path || folder_name`.
    pub install_path: Option<String>,
    /// Hidden when `0`, like the TS.
    pub install_size: Option<String>,
}

fn non_empty(value: Option<&str>) -> Option<&str> {
    value.filter(|s| !s.is_empty())
}

fn extra_strings(extra: Option<&Value>, key: &str) -> Option<Vec<String>> {
    extra?.get(key)?.as_array().map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_owned)).collect())
}

fn extra_about<'a>(extra: Option<&'a Value>, key: &str) -> Option<&'a str> {
    extra?.get("about")?.get(key)?.as_str()
}

impl Details {
    /// Computes the Focus values from the sources that are always available
    /// (`steam` and `wiki` default to none).
    pub fn compute(
        game: &GameInfo,
        meta: Option<&LocalGameMeta>,
        metadata: Option<&CollectionGameMetadata>,
        steam: Option<&SteamDetails>,
        wiki: Option<&WikiEntry>,
    ) -> Details {
        let extra = game.extra.as_ref();

        let description_source = non_empty(metadata.and_then(|m| m.description.as_deref()))
            .or_else(|| non_empty(extra_about(extra, "shortDescription")))
            .or_else(|| non_empty(extra_about(extra, "description")))
            .or_else(|| non_empty(steam.map(|s| s.short_description.as_str())))
            .or_else(|| non_empty(game.description.as_deref()));
        let description = description_source.map(to_plain_text).unwrap_or_default();

        let metadata_description = metadata.and_then(|m| m.description.as_deref()).unwrap_or("");
        let uses_steam_html = metadata.map(|m| m.field_sources.get("description").map(String::as_str) == Some("steam")).unwrap_or(false)
            || metadata_description.is_empty();
        let description_html = steam.filter(|s| uses_steam_html && !s.description_html.is_empty()).map(|s| s.description_html.clone());

        let metadata_genres = metadata.filter(|m| !m.genres.is_empty()).map(|m| m.genres.clone());
        let genres: Vec<String> = metadata_genres
            .or_else(|| steam.map(|s| s.genres.clone()))
            .or_else(|| extra_strings(extra, "genres"))
            .or_else(|| wiki.map(|w| w.pcgw_genres.clone()).filter(|g| !g.is_empty()))
            .unwrap_or_default()
            .into_iter()
            .filter(|g| !g.is_empty())
            .collect();

        let release_date = non_empty(metadata.and_then(|m| m.release_date.as_deref()))
            .or_else(|| non_empty(steam.and_then(|s| s.release_date.as_deref())))
            .or_else(|| non_empty(extra.and_then(|e| e.get("releaseDate")).and_then(Value::as_str)))
            .or_else(|| non_empty(wiki.and_then(|w| w.pcgw_release_date.as_deref())))
            .map(str::to_owned);

        let developer_text = non_empty(metadata.map(|m| m.developers.join(", ")).as_deref())
            .map(str::to_owned)
            .or_else(|| non_empty(steam.map(|s| s.developers.join(", ")).as_deref()).map(str::to_owned))
            .or_else(|| game.developer.clone());
        let developers = match metadata.map(|m| &m.developers).filter(|d| !d.is_empty()) {
            Some(d) => d.clone(),
            None => match steam.map(|s| &s.developers).filter(|d| !d.is_empty()) {
                Some(d) => d.clone(),
                None => split_names(developer_text.as_deref()),
            },
        };
        let publisher_text = non_empty(metadata.map(|m| m.publishers.join(", ")).as_deref())
            .map(str::to_owned)
            .or_else(|| non_empty(steam.map(|s| s.publishers.join(", ")).as_deref()).map(str::to_owned));
        let publishers = match metadata.map(|m| &m.publishers).filter(|d| !d.is_empty()) {
            Some(d) => d.clone(),
            None => match steam.map(|s| &s.publishers).filter(|d| !d.is_empty()) {
                Some(d) => d.clone(),
                None => split_names(publisher_text.as_deref()),
            },
        };

        let features = match metadata.map(|m| &m.features).filter(|f| !f.is_empty()) {
            Some(f) => f.clone(),
            None => steam.map(|s| s.features.clone()).unwrap_or_default(),
        };
        let platforms = match metadata.map(|m| &m.platforms).filter(|f| !f.is_empty()) {
            Some(p) => p.clone(),
            None => vec![non_empty(game.install.platform.as_deref()).unwrap_or("PC").to_owned()],
        };
        let platform = non_empty(platforms.first().map(String::as_str)).unwrap_or("PC").to_owned();

        let series = metadata.and_then(|m| m.series.as_deref()).map(str::to_owned).filter(|s| !s.is_empty());
        let notes = non_empty(metadata.and_then(|m| m.notes.as_deref()))
            .or_else(|| non_empty(meta.and_then(|m| m.notes.as_deref())))
            .map(str::to_owned);
        let install_path = non_empty(meta.and_then(|m| m.remapped_executable.as_deref()))
            .or_else(|| non_empty(meta.and_then(|m| m.windows_install_directory.as_deref())))
            .or_else(|| non_empty(game.install.install_path.as_deref()))
            .or_else(|| non_empty(game.folder_name.as_deref()))
            .map(str::to_owned);
        let install_size = non_empty(game.install.install_size.as_deref()).filter(|s| *s != "0").map(str::to_owned);

        Details {
            release_date,
            platform,
            platforms,
            genres,
            developers,
            publishers,
            features,
            themes: metadata.map(|m| m.themes.clone()).unwrap_or_default(),
            game_modes: metadata.map(|m| m.game_modes.clone()).unwrap_or_default(),
            series,
            critic_score: metadata.and_then(|m| m.critic_score).map(|n| n.get()),
            description,
            description_html,
            notes,
            install_path,
            install_size,
        }
    }

    /// Release date, platform and up to 4 genres, joined with ` · `
    /// (`collectionFocus__meta`).
    pub fn meta_line(&self, locale: DateLocale) -> String {
        focus_meta_line(self.release_date.as_deref(), &self.platform, &self.genres, locale)
    }
}

/// `[formatTimestamp(releaseDate), platform, ...genres.slice(0, 4)]
/// .filter(Boolean).join(' · ')`.
pub fn focus_meta_line(release_date: Option<&str>, platform: &str, genres: &[String], locale: DateLocale) -> String {
    let mut parts: Vec<String> = Vec::with_capacity(6);
    parts.push(format_timestamp(release_date.unwrap_or(""), locale));
    parts.push(platform.to_owned());
    parts.extend(genres.iter().take(4).cloned());
    parts.retain(|p| !p.is_empty());
    parts.join(" · ")
}

/// Details of `game` for the Focus panel. Reads the Steam details cache only
/// when the TS would fetch it: the game has no Collection metadata entry at
/// all and has a Steam app id.
pub fn focus_details(paths: &DataPaths, game: &CollectionGame, language: &str, wiki: Option<&WikiEntry>) -> Details {
    let steam = match (&game.metadata, &game.steam_app_id) {
        (None, Some(id)) => load_steam_details(paths, id, language).or_else(|| load_steam_details(paths, id, "en")),
        _ => None,
    };
    Details::compute(&game.game, game.meta.as_deref(), game.metadata.as_deref(), steam.as_ref(), wiki)
}

/// Everything the Focus panel of one game shows, in one call.
#[derive(Debug, Clone)]
pub struct FocusData {
    pub details: Details,
    /// Release date, platform and up to 4 genres (` · ` separated).
    pub meta_line: String,
    /// `formatTimestamp(lastPlayed)`; `None` is shown as "Never".
    pub last_played: Option<String>,
    /// `formatPlaytimeMinutes` ("0m", "2h 5m").
    pub playtime: String,
    /// The HowLongToBeat card (only when it has times).
    pub hltb: Option<HltbEntry>,
}

/// Merges Steam details (read from the cache, only when Heronite would show
/// them) and the wiki cache into the Focus values of `game`.
pub fn focus_data(library: &Library, game: &CollectionGame, wiki: &WikiCache) -> FocusData {
    let entry = wiki.lookup(&game.title, &game.game.title);
    let details = focus_details(&library.paths, game, &library.settings.language, entry.as_ref());
    let locale = library.date_locale;
    FocusData {
        meta_line: details.meta_line(locale),
        last_played: game.last_played.as_deref().map(|v| format_timestamp(v, locale)).filter(|v| !v.is_empty()),
        playtime: format_playtime_minutes(game.playtime_minutes),
        hltb: entry.and_then(|e| e.hltb).filter(HltbEntry::is_displayable),
        details,
    }
}

impl Library {
    /// The wiki cache (`wikigameinfo.json`), to pass to [`Library::focus`].
    pub fn wiki_cache(&self) -> WikiCache {
        WikiCache::load(&self.paths)
    }

    /// Focus panel data of `game` (one small file read for Steam details when
    /// they apply).
    pub fn focus(&self, game: &CollectionGame, wiki: &WikiCache) -> FocusData {
        focus_data(self, game, wiki)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn real_focus_data() {
        let Some(root) = std::env::var_os("HERONITE_FIXTURE") else { return };
        let lib = Library::load(&DataPaths::at(root)).unwrap();
        let wiki = lib.wiki_cache();
        let mut with_hltb = 0;
        let mut with_steam_html = 0;
        for game in &lib.games {
            let f = lib.focus(game, &wiki);
            assert!(!f.details.platform.is_empty());
            with_hltb += f.hltb.is_some() as usize;
            with_steam_html += f.details.description_html.is_some() as usize;
        }
        eprintln!("focus: hltb={with_hltb} steamHtml={with_steam_html}");
        let portal = lib.get("sideload_steam_400").unwrap();
        let f = lib.focus(portal, &wiki);
        eprintln!("Portal: {} | {:?} | last={:?} play={}", f.meta_line, f.details.genres, f.last_played, f.playtime);
    }

    #[test]
    fn steam_details_are_used_only_for_games_without_metadata() {
        use crate::library::tests_support::game_with;
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path());
        std::fs::create_dir_all(paths.steam_details("400", "english").parent().unwrap()).unwrap();
        std::fs::write(
            paths.steam_details("400", "english"),
            r#"{"steamAppId":"400","language":"english","name":"Portal","shortDescription":"Short","descriptionHtml":"<p>Long</p>","developers":["Valve"],"publishers":["Valve"],"genres":["Action"],"features":["Single-player"],"releaseDate":"Oct 10, 2007","storeUrl":"u","fetchedAt":1}"#,
        )
        .unwrap();
        let with_meta = game_with("steam_400", "sideload", |_, _, _, meta, _| meta.steam_app_id = Some("400".into()));
        let d = focus_details(&paths, &with_meta, "en", None);
        assert!(d.description_html.is_none(), "metadata entry present: no Steam fetch in Heronite");

        let mut bare = game_with("steam_400", "sideload", |_, _, _, meta, _| meta.steam_app_id = Some("400".into()));
        std::sync::Arc::get_mut(&mut bare).unwrap().metadata = None;
        let d = focus_details(&paths, &bare, "en", None);
        assert_eq!(d.description_html.as_deref(), Some("<p>Long</p>"));
        assert_eq!(d.description, "Short");
        assert_eq!(d.genres, ["Action"]);
        assert_eq!(d.developers, ["Valve"]);
        assert_eq!(d.release_date.as_deref(), Some("Oct 10, 2007"));
        assert_eq!(d.features, ["Single-player"]);
        // other language: no cache file, nothing merged
        assert!(load_steam_details(&paths, "400", "pt-BR").is_none());
    }

    fn game(extra: Value) -> GameInfo {
        serde_json::from_value(json!({
            "runner": "legendary", "app_name": "x", "title": "X",
            "developer": "Dev A, Dev B", "install": {"platform": "Windows"},
            "extra": extra
        }))
        .unwrap()
    }

    #[test]
    fn steam_language_mapping() {
        assert_eq!(steam_language("en"), "english");
        assert_eq!(steam_language("pt-BR"), "brazilian");
        assert_eq!(steam_language("pt_BR"), "brazilian");
        assert_eq!(steam_language("de-AT"), "german");
        assert_eq!(steam_language("xx"), "english");
        assert_eq!(steam_language(""), "english");
    }

    #[test]
    fn chains_fall_back_to_store_extra_and_game_info() {
        let g = game(
            json!({"about": {"description": "<p>Hi&nbsp;there</p>", "shortDescription": ""}, "genres": ["Action", ""], "releaseDate": "2019-08-27"}),
        );
        let d = Details::compute(&g, None, None, None, None);
        assert_eq!(d.description, "Hi there");
        assert_eq!(d.genres, ["Action"]);
        assert_eq!(d.release_date.as_deref(), Some("2019-08-27"));
        assert_eq!(d.developers, ["Dev A", "Dev B"]);
        assert_eq!(d.platform, "Windows");
        assert_eq!(d.meta_line(DateLocale::En), "8/27/2019 · Windows · Action");
    }

    #[test]
    fn metadata_wins_and_steam_html_is_conditional() {
        let g = game(json!({}));
        let mut md = CollectionGameMetadata {
            description: Some("From IGDB".into()),
            genres: vec!["RPG".into()],
            platforms: vec!["PC (Microsoft Windows)".into(), "PS5".into()],
            ..Default::default()
        };
        md.field_sources.insert("description".into(), "igdb".into());
        let steam = SteamDetails { description_html: "<p>steam</p>".into(), genres: vec!["Indie".into()], ..Default::default() };
        let d = Details::compute(&g, None, Some(&md), Some(&steam), None);
        assert_eq!(d.description, "From IGDB");
        assert!(d.description_html.is_none());
        assert_eq!(d.genres, ["RPG"]);
        assert_eq!(d.platform, "PC (Microsoft Windows)");

        md.field_sources.insert("description".into(), "steam".into());
        let d = Details::compute(&g, None, Some(&md), Some(&steam), None);
        assert_eq!(d.description_html.as_deref(), Some("<p>steam</p>"));
        // No Collection description at all also shows the Steam HTML.
        md.description = None;
        md.field_sources.clear();
        let d = Details::compute(&g, None, Some(&md), Some(&steam), None);
        assert_eq!(d.description_html.as_deref(), Some("<p>steam</p>"));
    }

    #[test]
    fn meta_line_skips_empty_parts_and_caps_genres() {
        let genres: Vec<String> = ["A", "B", "C", "D", "E"].iter().map(|s| s.to_string()).collect();
        assert_eq!(focus_meta_line(None, "PC", &genres, DateLocale::En), "PC · A · B · C · D");
        assert_eq!(focus_meta_line(Some("2010"), "PC", &[], DateLocale::En), "2010 · PC");
    }
}
