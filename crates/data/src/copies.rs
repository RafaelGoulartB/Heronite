//! Copies of one game from several stores (`local_library/copies.json`):
//! the same game owned on GOG and Epic, say, shown as one Collection card
//! that plays the preferred copy.
//!
//! A group only links Collection entries; each copy keeps its own records
//! (library entry, playtime, sessions, achievements, saves, metadata), and
//! ungrouping only edits this file.
//!
//! Groups are found again at every load, so they follow metadata refreshes.
//! Entries link by themselves only on confident evidence: different stores,
//! a shared IGDB id or Steam app id, **and** the same title (case,
//! punctuation and ™/® aside). A shared id alone is not enough because IGDB
//! ids can come from a fuzzy title search, and the title alone never is;
//! editions with other titles ("… GOTY Edition") link only by hand. The file
//! keeps only the user's decisions: links made by hand, pairs never to link
//! again, and the preferred copies. Keys of games that are not in the
//! library are kept, so a game that comes back joins its group again.

use std::cmp::Ordering;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::sync::Arc;

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::json_store;
use crate::library::CollectionGame;
use crate::models::Extra;
use crate::paths::DataPaths;
use crate::steam_owned::title_key;

/// `copies.json`. Every list holds Collection keys (`runner_appName`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct CopiesFile {
    /// Groups linked by hand, whatever their titles or ids.
    #[serde(default)]
    pub links: Vec<Vec<String>>,
    /// Pairs the user ungrouped: never linked again by themselves.
    #[serde(default)]
    pub apart: Vec<[String; 2]>,
    /// Copies chosen as their group's default.
    #[serde(default)]
    pub preferred: Vec<String>,
    #[serde(flatten)]
    pub extra: Extra,
}

impl CopiesFile {
    /// The user's decisions; a missing or broken file has none.
    pub fn read(paths: &DataPaths) -> Self {
        json_store::read(&paths.copies()).unwrap_or_else(|e| {
            log::warn!("{}: {e:#}", paths.copies().display());
            Self::default()
        })
    }

    fn is_apart(&self, a: &str, b: &str) -> bool {
        self.apart.iter().any(|[x, y]| (x == a && y == b) || (x == b && y == a))
    }
}

/// Links `keys` as copies of one game (with the groups they are already in).
pub fn link(paths: &DataPaths, keys: &[String]) -> Result<()> {
    if keys.len() < 2 {
        return Ok(());
    }
    json_store::update::<CopiesFile, _>(&paths.copies(), |file| {
        file.apart.retain(|[a, b]| !(keys.contains(a) && keys.contains(b)));
        let mut merged: BTreeSet<String> = keys.iter().cloned().collect();
        file.links.retain(|group| {
            let touches = group.iter().any(|k| merged.contains(k));
            if touches {
                merged.extend(group.iter().cloned());
            }
            !touches
        });
        file.links.push(merged.into_iter().collect());
        Ok(())
    })
}

/// Ungroups `members` (a whole group): each becomes a game of its own again,
/// and no two of them link by themselves any more. Their records are not touched.
pub fn unlink(paths: &DataPaths, members: &[String]) -> Result<()> {
    json_store::update::<CopiesFile, _>(&paths.copies(), |file| {
        for group in &mut file.links {
            group.retain(|k| !members.contains(k));
        }
        file.links.retain(|g| g.len() > 1);
        file.preferred.retain(|k| !members.contains(k));
        for (i, a) in members.iter().enumerate() {
            for b in &members[i + 1..] {
                if !file.is_apart(a, b) {
                    file.apart.push([a.clone(), b.clone()]);
                }
            }
        }
        Ok(())
    })
}

/// Makes `key` the default copy of its group (`members`).
pub fn set_preferred(paths: &DataPaths, key: &str, members: &[String]) -> Result<()> {
    json_store::update::<CopiesFile, _>(&paths.copies(), |file| {
        file.preferred.retain(|k| !members.contains(k));
        file.preferred.push(key.to_owned());
        Ok(())
    })
}

/// The store a copy comes from, as groups compare them (`steam` for every
/// Steam entry, whatever imported it).
pub fn store_of(game: &CollectionGame) -> &str {
    if game.meta.as_deref().is_some_and(|m| m.launch_kind == "steam-uri") {
        return "steam";
    }
    match game.meta.as_deref().map(|m| m.source.as_str()).filter(|s| !s.is_empty()) {
        Some(source) => source,
        None => game.runner.as_str(),
    }
}

/// Order of the stores when nothing else decides which copy plays.
fn store_rank(store: &str) -> usize {
    ["steam", "gog", "epic", "legendary", "amazon", "nile", "xbox"].iter().position(|s| *s == store).unwrap_or(usize::MAX)
}

/// Every store a game stands for: its own, and Steam for a Steam twin.
fn stores(game: &CollectionGame) -> BTreeSet<&str> {
    let mut out = BTreeSet::from([store_of(game)]);
    if game.steam_link.is_some() {
        out.insert("steam");
    }
    out
}

/// Emulated and browser games never link by themselves (same rule as Steam twins).
fn joinable(game: &CollectionGame) -> bool {
    let browser = game.game.install.platform.as_deref().is_some_and(|p| p.eq_ignore_ascii_case("browser"));
    !browser && game.launch_kind.as_deref() != Some("emulator")
}

/// The ids two copies may share: `igdb:<id>` and `steam:<app id>`.
fn ids(game: &CollectionGame) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(id) = game.metadata.as_deref().and_then(|m| m.igdb_id).map(|n| n.get()).filter(|n| *n >= 1.0) {
        out.push(format!("igdb:{}", id as i64));
    }
    let steam =
        [game.meta.as_deref().and_then(|m| m.steam_app_id.as_deref()), game.metadata.as_deref().and_then(|m| m.steam_app_id.as_deref())];
    for id in steam.into_iter().flatten().map(str::trim).filter(|s| !s.is_empty()) {
        let id = format!("steam:{id}");
        if !out.contains(&id) {
            out.push(id);
        }
    }
    out
}

/// One game's copies, the default first, then the others in the order they
/// would play: installed first, the most recently played, the most played,
/// then by store.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CopyGroup {
    pub members: Vec<String>,
    /// The user chose the default copy (otherwise it follows the order).
    pub chosen: bool,
    /// Linked by hand (at least in part).
    pub manual: bool,
}

/// The groups of a loaded Collection.
#[derive(Debug, Clone, Default)]
pub struct CopyGroups {
    pub groups: Vec<CopyGroup>,
    by_key: HashMap<String, usize>,
    /// Entries of other stores that share an id with a game but not its
    /// title (an edition, or a wrong match): offered, never linked by themselves.
    suggestions: HashMap<String, Vec<String>>,
}

impl CopyGroups {
    /// The group of a game, if it has other copies.
    pub fn get(&self, key: &str) -> Option<&CopyGroup> {
        self.by_key.get(key).map(|i| &self.groups[*i])
    }

    pub fn is_empty(&self) -> bool {
        self.groups.is_empty()
    }

    /// The possible copies of the game `key` (or of its group) that are not in it.
    pub fn suggested(&self, key: &str) -> Vec<String> {
        let own: Vec<String> = self.get(key).map_or_else(|| vec![key.to_owned()], |g| g.members.clone());
        let mut out: Vec<String> = Vec::new();
        for k in own.iter().flat_map(|k| self.suggestions.get(k).into_iter().flatten()) {
            if !own.contains(k) && !out.contains(k) {
                out.push(k.clone());
            }
        }
        out
    }

    /// The list with each group's copies shown once: by its first copy (in
    /// the group's order) that is in `filtered`, at that copy's place. So a
    /// filter that only one copy matches ("Store: GOG", "Installed") still
    /// shows that copy.
    pub fn collapse(&self, filtered: Vec<Arc<CollectionGame>>) -> Vec<Arc<CollectionGame>> {
        if self.groups.is_empty() {
            return filtered;
        }
        let shown: HashSet<&str> = filtered.iter().map(|g| g.key.as_str()).collect();
        let representative: HashMap<usize, &str> = self
            .groups
            .iter()
            .enumerate()
            .filter_map(|(i, g)| g.members.iter().find(|k| shown.contains(k.as_str())).map(|k| (i, k.as_str())))
            .collect();
        let keep: HashSet<String> = filtered
            .iter()
            .filter(|g| self.by_key.get(&g.key).is_none_or(|i| representative.get(i) == Some(&g.key.as_str())))
            .map(|g| g.key.clone())
            .collect();
        filtered.into_iter().filter(|g| keep.contains(&g.key)).collect()
    }
}

/// Union-find over the games' indexes that also keeps each set's members and stores.
struct Sets<'a> {
    parent: Vec<usize>,
    members: Vec<Vec<usize>>,
    stores: Vec<BTreeSet<&'a str>>,
    manual: Vec<bool>,
}

impl Sets<'_> {
    fn find(&mut self, i: usize) -> usize {
        let mut root = i;
        while self.parent[root] != root {
            root = self.parent[root];
        }
        let mut i = i;
        while self.parent[i] != root {
            let next = self.parent[i];
            self.parent[i] = root;
            i = next;
        }
        root
    }

    /// Joins the sets of two roots.
    fn join(&mut self, ra: usize, rb: usize, manual: bool) {
        if ra == rb {
            return;
        }
        let stores = std::mem::take(&mut self.stores[rb]);
        self.stores[ra].extend(stores);
        let members = std::mem::take(&mut self.members[rb]);
        self.members[ra].extend(members);
        self.manual[ra] |= self.manual[rb] | manual;
        self.parent[rb] = ra;
    }
}

/// Finds the groups among `games` (the loaded Collection) with the user's
/// decisions in `file`.
pub fn groups(games: &[Arc<CollectionGame>], file: &CopiesFile) -> CopyGroups {
    let index: HashMap<&str, usize> = games.iter().enumerate().map(|(i, g)| (g.key.as_str(), i)).collect();
    let mut sets = Sets {
        parent: (0..games.len()).collect(),
        members: (0..games.len()).map(|i| vec![i]).collect(),
        stores: games.iter().map(|g| stores(g)).collect(),
        manual: vec![false; games.len()],
    };

    for link in &file.links {
        let present: Vec<usize> = link.iter().filter_map(|k| index.get(k.as_str()).copied()).collect();
        for pair in present.windows(2) {
            let (ra, rb) = (sets.find(pair[0]), sets.find(pair[1]));
            sets.join(ra, rb, true);
        }
    }

    // Candidates by (title, id); entries of one store sharing a title and id
    // are ambiguous (maybe two editions) and link nothing.
    let mut buckets: HashMap<(String, String), Vec<usize>> = HashMap::new();
    for (i, g) in games.iter().enumerate().filter(|(_, g)| joinable(g)) {
        let title = title_key(&g.title);
        if title.is_empty() {
            continue;
        }
        for id in ids(g) {
            buckets.entry((title.clone(), id)).or_default().push(i);
        }
    }
    let mut bucket_list: Vec<Vec<usize>> = buckets.into_values().filter(|b| b.len() > 1).collect();
    bucket_list.sort();
    for bucket in bucket_list {
        let mut seen = HashSet::new();
        if !bucket.iter().all(|i| seen.insert(store_of(&games[*i]))) {
            continue;
        }
        for &b in &bucket[1..] {
            let (ra, rb) = (sets.find(bucket[0]), sets.find(b));
            if ra == rb || !sets.stores[ra].is_disjoint(&sets.stores[rb]) {
                continue;
            }
            let apart = sets.members[ra].iter().any(|x| sets.members[rb].iter().any(|y| file.is_apart(&games[*x].key, &games[*y].key)));
            if !apart {
                sets.join(ra, rb, false);
            }
        }
    }

    let preferred: HashSet<&str> = file.preferred.iter().map(String::as_str).collect();
    let mut out = CopyGroups { suggestions: suggestions(games, &mut sets, file), ..Default::default() };
    for root in 0..games.len() {
        if sets.parent[root] != root || sets.members[root].len() < 2 {
            continue;
        }
        let mut members = std::mem::take(&mut sets.members[root]);
        members.sort_by(|a, b| play_order(&games[*a], &games[*b], &preferred));
        let chosen = preferred.contains(games[members[0]].key.as_str());
        let group = CopyGroup { members: members.iter().map(|i| games[*i].key.clone()).collect(), chosen, manual: sets.manual[root] };
        for k in &group.members {
            out.by_key.insert(k.clone(), out.groups.len());
        }
        out.groups.push(group);
    }
    out
}

/// Pairs of other stores that share an id but are not in one group. An id
/// shared by many entries says little (a wrong match), so it offers nothing.
fn suggestions(games: &[Arc<CollectionGame>], sets: &mut Sets<'_>, file: &CopiesFile) -> HashMap<String, Vec<String>> {
    let mut by_id: HashMap<String, Vec<usize>> = HashMap::new();
    for (i, g) in games.iter().enumerate().filter(|(_, g)| joinable(g)) {
        for id in ids(g) {
            by_id.entry(id).or_default().push(i);
        }
    }
    let mut out: HashMap<String, Vec<String>> = HashMap::new();
    for ids in by_id.values().filter(|v| (2..=4).contains(&v.len())) {
        for (n, &a) in ids.iter().enumerate() {
            for &b in &ids[n + 1..] {
                let (ka, kb) = (&games[a].key, &games[b].key);
                if store_of(&games[a]) == store_of(&games[b]) || sets.find(a) == sets.find(b) || file.is_apart(ka, kb) {
                    continue;
                }
                for (from, to) in [(ka, kb), (kb, ka)] {
                    let list = out.entry(from.clone()).or_default();
                    if !list.contains(to) {
                        list.push(to.clone());
                    }
                }
            }
        }
    }
    out
}

/// Which of two copies comes first in a group.
fn play_order(a: &CollectionGame, b: &CollectionGame, preferred: &HashSet<&str>) -> Ordering {
    let chosen = |g: &CollectionGame| preferred.contains(g.key.as_str());
    chosen(b)
        .cmp(&chosen(a))
        .then(b.installed.cmp(&a.installed))
        // ISO 8601 dates of one format sort as text.
        .then_with(|| b.last_played.cmp(&a.last_played))
        .then(b.playtime_minutes.cmp(&a.playtime_minutes))
        .then_with(|| store_rank(store_of(a)).cmp(&store_rank(store_of(b))))
        .then_with(|| a.key.cmp(&b.key))
}

/// The figures of a group as one game.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Totals {
    pub playtime_minutes: u64,
    pub last_played: Option<String>,
    pub first_played: Option<String>,
}

/// Sums the copies' own figures. Copies that start the same Steam install
/// count once (the larger), so one play is never counted twice.
pub fn totals<'a>(copies: impl IntoIterator<Item = &'a CollectionGame>) -> Totals {
    let mut by_install: HashMap<String, u64> = HashMap::new();
    let mut t = Totals::default();
    for g in copies {
        let install = g.steam_launch_id.as_ref().map_or_else(|| g.key.clone(), |id| format!("steam:{id}"));
        let minutes = by_install.entry(install).or_default();
        *minutes = (*minutes).max(g.playtime_minutes);
        if g.last_played > t.last_played {
            t.last_played = g.last_played.clone();
        }
        if let Some(first) = &g.first_played {
            if t.first_played.as_ref().is_none_or(|f| first < f) {
                t.first_played = Some(first.clone());
            }
        }
    }
    t.playtime_minutes = by_install.values().sum();
    t
}

#[cfg(test)]
mod tests;
