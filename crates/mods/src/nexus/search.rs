//! Nexus GraphQL v2 (`api.nexusmods.com/v2/graphql`): searching mods and
//! games. It answers without an API key.

use heronite_i18n::tr;
use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::{json, Value};

use super::null_default;
use crate::http;

const ENDPOINT: &str = "https://api.nexusmods.com/v2/graphql";
/// The most mods one page may hold (a larger `count` is cut to this).
const MAX_COUNT: u32 = 80;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SearchSort {
    #[default]
    Endorsements,
    Downloads,
    Updated,
}

impl SearchSort {
    fn field(self) -> &'static str {
        match self {
            SearchSort::Endorsements => "endorsements",
            SearchSort::Downloads => "downloads",
            SearchSort::Updated => "updatedAt",
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SearchPage {
    /// Mods matching the search, over all pages.
    pub total: u64,
    pub mods: Vec<SearchMod>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct SearchMod {
    pub mod_id: u64,
    #[serde(deserialize_with = "null_default")]
    pub name: String,
    #[serde(deserialize_with = "null_default")]
    pub summary: String,
    #[serde(deserialize_with = "null_default")]
    pub version: String,
    #[serde(deserialize_with = "null_default")]
    pub author: String,
    /// Small picture URL.
    #[serde(rename = "thumbnailUrl")]
    pub thumbnail: Option<String>,
    /// Full-size picture URL.
    #[serde(rename = "pictureUrl")]
    pub picture: Option<String>,
    pub endorsements: u64,
    pub downloads: u64,
    /// RFC 3339.
    #[serde(deserialize_with = "null_default")]
    pub updated_at: String,
    #[serde(rename = "adultContent")]
    pub adult: bool,
}

const MODS_QUERY: &str = "query($f: ModsFilter, $s: [ModsSort!], $o: Int, $c: Int) { mods(filter: $f, sort: $s, offset: $o, count: $c) { \
    totalCount nodes { modId name summary version author pictureUrl thumbnailUrl endorsements downloads updatedAt adultContent } } }";

/// Mods of a game by name (`query` empty: the whole game), best `sort`
/// first. `offset` and `count` page through the results.
pub fn search(domain: &str, query: &str, sort: SearchSort, offset: u32, count: u32, include_adult: bool) -> Result<SearchPage, String> {
    let mut filter = json!({ "gameDomainName": [{ "value": domain }] });
    let query = query.trim();
    if !query.is_empty() {
        filter["name"] = json!([{ "value": query, "op": "WILDCARD" }]);
    }
    if !include_adult {
        filter["adultContent"] = json!([{ "value": false }]);
    }
    let vars = json!({
        "f": filter,
        "s": [{ sort.field(): { "direction": "DESC" } }],
        "o": offset,
        "c": count.clamp(1, MAX_COUNT),
    });
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Data {
        mods: Mods,
    }
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Mods {
        #[serde(default)]
        total_count: u64,
        #[serde(default, deserialize_with = "null_default")]
        nodes: Vec<SearchMod>,
    }
    let data: Data = graphql(MODS_QUERY, vars)?;
    Ok(SearchPage { total: data.mods.total_count, mods: data.mods.nodes })
}

/// Games whose name matches the words of `text`, as `(domain, name)`.
pub fn games_matching(text: &str) -> Result<Vec<(String, String)>, String> {
    #[derive(Deserialize)]
    struct Data {
        games: Games,
    }
    #[derive(Deserialize)]
    struct Games {
        #[serde(default, deserialize_with = "null_default")]
        nodes: Vec<Game>,
    }
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Game {
        domain_name: String,
        name: String,
    }
    let query = "query($f: GamesSearchFilter) { games(filter: $f, count: 40) { nodes { domainName name } } }";
    let vars = json!({ "f": { "name": [{ "value": text.trim(), "op": "WILDCARD" }] } });
    let data: Data = graphql(query, vars)?;
    Ok(data.games.nodes.into_iter().map(|g| (g.domain_name, g.name)).collect())
}

/// The name of the game with this Nexus domain; `None` when there is none.
pub fn game_name(domain: &str) -> Result<Option<String>, String> {
    #[derive(Deserialize)]
    struct Data {
        game: Option<Game>,
    }
    #[derive(Deserialize)]
    struct Game {
        name: String,
    }
    let data: Data = graphql("query($d: String!) { game(domainName: $d) { name } }", json!({ "d": domain }))?;
    Ok(data.game.map(|g| g.name).filter(|n| !n.is_empty()))
}

fn graphql<T: DeserializeOwned>(query: &str, variables: Value) -> Result<T, String> {
    #[derive(Deserialize)]
    struct Envelope<T> {
        data: Option<T>,
        #[serde(default, deserialize_with = "null_default")]
        errors: Vec<GqlError>,
    }
    #[derive(Deserialize)]
    struct GqlError {
        #[serde(default)]
        message: String,
    }
    let reply: Envelope<T> = http::post_json(ENDPOINT, &[], &json!({ "query": query, "variables": variables }))?;
    match (reply.data, reply.errors.first()) {
        (Some(data), _) => Ok(data),
        (None, Some(e)) => Err(tr!("Nexus Mods could not search: {error}", error = e.message)),
        (None, None) => Err(tr!("Nexus Mods could not search: {error}", error = "empty answer")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_nodes_parse() {
        let page: serde_json::Value = serde_json::from_str(
            r#"{"modId":12604,"name":"SkyUI","summary":"Elegant interface","version":"6.9","author":"SkyUI Team",
                "pictureUrl":"https://staticdelivery.nexusmods.com/p.png","thumbnailUrl":"https://staticdelivery.nexusmods.com/t.png",
                "endorsements":506202,"downloads":26211667,"updatedAt":"2026-05-05T22:41:21Z","adultContent":false,"game":{"domainName":"x"}}"#,
        )
        .unwrap();
        let m: SearchMod = serde_json::from_value(page).unwrap();
        assert_eq!((m.mod_id, m.name.as_str(), m.endorsements, m.downloads, m.adult), (12604, "SkyUI", 506_202, 26_211_667, false));
        assert_eq!(m.thumbnail.as_deref(), Some("https://staticdelivery.nexusmods.com/t.png"));
        let sparse: SearchMod =
            serde_json::from_str(r#"{"modId":1,"name":"A","summary":null,"version":null,"author":null,"pictureUrl":null}"#).unwrap();
        assert_eq!((sparse.summary.as_str(), sparse.picture), ("", None));
    }

    #[test]
    #[ignore = "network"]
    fn live_search_and_games() {
        let page = search("skyrimspecialedition", "skyui", SearchSort::Endorsements, 0, 3, false).unwrap();
        assert!(page.total > 0 && page.mods.iter().any(|m| m.name == "SkyUI"), "{page:?}");
        let top = search("skyrimspecialedition", "", SearchSort::Updated, 5, 2, true).unwrap();
        assert_eq!(top.mods.len(), 2);
        assert!(games_matching("elden ring").unwrap().iter().any(|(d, _)| d == "eldenring"));
        assert_eq!(game_name("skyrimspecialedition").unwrap().as_deref(), Some("Skyrim Special Edition"));
        assert_eq!(game_name("nosuchgame-xyz").unwrap(), None);
    }

    #[test]
    fn sorts_name_their_graphql_field() {
        assert_eq!(SearchSort::Updated.field(), "updatedAt");
        assert_eq!(SearchSort::default().field(), "endorsements");
    }
}
