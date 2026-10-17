//! Library fields: the names of genres, companies, series, themes, features
//! and platforms, renamed, merged and removed across the Collection. IGDB,
//! Steam, Lutris and the stores spell the same thing in several ways
//! ("Valve" / "Valve Corporation", "RPG" / "Role-playing (RPG)"), and every
//! spelling becomes its own value in the View menu.
//!
//! Renaming, merging or removing a name rewrites `metadata.json` and keeps a
//! rule in `local_library/names.json` (name -> new name, `""` = removed). The
//! rules also apply to metadata downloaded later (`heronite-metadata`) and,
//! at load, to the facets of names a store gives that are not in
//! `metadata.json` (Epic's developer, for example).

mod similar;

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use anyhow::{bail, Result};
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

pub use similar::similar_groups;

use crate::collection::{FacetItem, FacetKind, GameFacets};
use crate::json_store;
use crate::library::CollectionGame;
use crate::models::{CollectionGameMetadata, Extra, MetadataFile};
use crate::paths::DataPaths;
use crate::text::facet_key;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Field {
    Genre,
    /// Developers and publishers together: renaming a company changes both.
    Company,
    Series,
    Theme,
    Feature,
    Platform,
}

pub const FIELDS: [Field; 6] = [Field::Genre, Field::Company, Field::Series, Field::Theme, Field::Feature, Field::Platform];

impl Field {
    /// Key in `names.json`.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Genre => "genres",
            Self::Company => "companies",
            Self::Series => "series",
            Self::Theme => "themes",
            Self::Feature => "features",
            Self::Platform => "platforms",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        FIELDS.into_iter().find(|f| f.as_str() == value)
    }

    /// The View menu kinds that show the field's names.
    pub fn facet_kinds(self) -> &'static [FacetKind] {
        match self {
            Self::Genre => &[FacetKind::Genre],
            Self::Company => &[FacetKind::Developer, FacetKind::Publisher],
            Self::Series => &[FacetKind::Series],
            Self::Theme => &[FacetKind::Theme],
            Self::Feature => &[FacetKind::Feature],
            Self::Platform => &[FacetKind::Platform],
        }
    }
}

// local_library/names.json
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NamesFile {
    /// Field -> name as downloaded -> name to show (`""` = removed).
    #[serde(default)]
    pub renames: IndexMap<String, IndexMap<String, String>>,
    /// Field -> groups of names the user said are different things; they are
    /// no longer suggested as duplicates.
    #[serde(default)]
    pub apart: IndexMap<String, Vec<Vec<String>>>,
    #[serde(flatten)]
    pub extra: Extra,
}

impl NamesFile {
    pub fn load(paths: &DataPaths) -> Result<Self> {
        json_store::read(&paths.field_names())
    }

    /// The remembered changes, field by field, in the order they were made.
    pub fn rules(&self) -> Vec<Rule> {
        let mut out = Vec::new();
        for field in FIELDS {
            for (from, to) in self.renames.get(field.as_str()).into_iter().flatten() {
                out.push(Rule { field, from: from.clone(), to: (!to.is_empty()).then(|| to.clone()) });
            }
        }
        out
    }

    pub fn apart(&self, field: Field) -> &[Vec<String>] {
        self.apart.get(field.as_str()).map(Vec::as_slice).unwrap_or(&[])
    }
}

/// One remembered change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rule {
    pub field: Field,
    pub from: String,
    /// `None` = removed.
    pub to: Option<String>,
}

/// The rules of `names.json`, looked up by [`facet_key`].
#[derive(Debug, Clone, Default)]
pub struct NameRules {
    /// (field, key of the old name) -> new name; `None` = removed.
    map: HashMap<(Field, String), Option<String>>,
}

impl NameRules {
    /// The rules on disk; none when the file cannot be read.
    pub fn load(paths: &DataPaths) -> Self {
        match NamesFile::load(paths) {
            Ok(file) => Self::from_file(&file),
            Err(err) => {
                log::warn!("Field names unreadable, ignored: {err:#}");
                Self::default()
            }
        }
    }

    pub fn from_file(file: &NamesFile) -> Self {
        let map = file.rules().into_iter().map(|r| ((r.field, facet_key(&r.from)), r.to)).filter(|((_, key), _)| !key.is_empty()).collect();
        Self { map }
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    /// `None` when no rule names `value`; `Some(None)` when it is removed.
    fn lookup(&self, field: Field, value: &str) -> Option<Option<String>> {
        if self.map.is_empty() {
            return None;
        }
        self.map.get(&(field, facet_key(value))).cloned()
    }

    /// Applies the rules to every field of a metadata record. True when it changed.
    pub fn apply_metadata(&self, md: &mut CollectionGameMetadata) -> bool {
        let mut changed = false;
        for field in FIELDS {
            changed |= rewrite_metadata(md, field, &|v| self.lookup(field, v));
        }
        changed
    }

    /// Applies the rules to the facets of a game (names a store gave).
    pub fn apply_facets(&self, facets: &mut GameFacets) {
        if self.map.is_empty() {
            return;
        }
        for field in FIELDS {
            for kind in field.facet_kinds() {
                let slot = &mut facets[kind.index()];
                let mut names: Vec<String> = slot.iter().map(|i| i.value.clone()).collect();
                if rewrite(&mut names, &|v| self.lookup(field, v)) {
                    *slot = names.into_iter().map(|value| FacetItem { key: facet_key(&value), value }).collect();
                }
            }
        }
    }
}

/// Maps each name of `list` (`None` from `map` = kept as it is, `Some(None)`
/// = dropped) and drops repeats. True when the list changed.
fn rewrite(list: &mut Vec<String>, map: &dyn Fn(&str) -> Option<Option<String>>) -> bool {
    let mut seen = HashSet::new();
    let mut out = Vec::with_capacity(list.len());
    for value in list.iter() {
        let next = match map(value) {
            None => Some(value.clone()),
            Some(next) => next,
        };
        if let Some(next) = next.filter(|n| !n.trim().is_empty()) {
            if seen.insert(facet_key(&next)) {
                out.push(next);
            }
        }
    }
    let changed = out != *list;
    *list = out;
    changed
}

/// [`rewrite`] over the lists of `field` in a metadata record.
fn rewrite_metadata(md: &mut CollectionGameMetadata, field: Field, map: &dyn Fn(&str) -> Option<Option<String>>) -> bool {
    match field {
        Field::Genre => rewrite(&mut md.genres, map),
        Field::Company => rewrite(&mut md.developers, map) | rewrite(&mut md.publishers, map),
        Field::Theme => rewrite(&mut md.themes, map),
        Field::Feature => rewrite(&mut md.features, map),
        Field::Platform => rewrite(&mut md.platforms, map),
        Field::Series => {
            let mut list: Vec<String> = md.series.iter().cloned().collect();
            let changed = rewrite(&mut list, map);
            if changed {
                md.series = list.into_iter().next();
            }
            changed
        }
    }
}

/// Renames `from` to `to` in every game; several names are a merge. The
/// change is remembered for metadata downloaded later. Returns how many
/// games changed.
pub fn rename(paths: &DataPaths, field: Field, from: &[String], to: &str) -> Result<usize> {
    let to = to.trim();
    if to.is_empty() {
        bail!("the new name is empty");
    }
    change(paths, field, from, Some(to))
}

/// Takes the names out of every game, now and in later downloads.
pub fn remove(paths: &DataPaths, field: Field, names: &[String]) -> Result<usize> {
    change(paths, field, names, None)
}

fn change(paths: &DataPaths, field: Field, from: &[String], to: Option<&str>) -> Result<usize> {
    let from: Vec<String> = from.iter().map(|s| s.trim().to_owned()).filter(|s| !s.is_empty() && Some(s.as_str()) != to).collect();
    if from.is_empty() {
        return Ok(0);
    }
    json_store::update::<NamesFile, _>(&paths.field_names(), |file| {
        remember(file, field, &from, to);
        Ok(())
    })?;
    let keys: HashSet<String> = from.iter().map(|s| facet_key(s)).collect();
    let to = to.map(str::to_owned);
    let map = move |v: &str| keys.contains(&facet_key(v)).then(|| to.clone());
    json_store::update::<MetadataFile, _>(&paths.metadata(), |file| {
        Ok(file.games.values_mut().filter_map(|md| rewrite_metadata(md, field, &map).then_some(())).count())
    })
}

/// Records `from` -> `to` and keeps the older rules consistent with it.
fn remember(file: &mut NamesFile, field: Field, from: &[String], to: Option<&str>) {
    let rules = file.renames.entry(field.as_str().to_owned()).or_default();
    let from_keys: HashSet<String> = from.iter().map(|s| facet_key(s)).collect();
    let target = to.unwrap_or("").to_owned();
    // Names that used to become one of these now become the new one.
    for t in rules.values_mut() {
        if !t.is_empty() && from_keys.contains(&facet_key(t)) {
            *t = target.clone();
        }
    }
    // The new name is wanted as it is: a rule that changed it goes.
    if let Some(to) = to {
        let key = facet_key(to);
        rules.retain(|name, t| facet_key(name) != key || t == to);
    }
    for name in from {
        let key = facet_key(name);
        rules.retain(|n, _| facet_key(n) != key);
        rules.insert(name.clone(), target.clone());
    }
    rules.retain(|name, t| name != t);
    if rules.is_empty() {
        file.renames.shift_remove(field.as_str());
    }
}

/// Forgets the rule of `from`: metadata downloaded later keeps that name.
/// The games already changed stay as they are.
pub fn forget(paths: &DataPaths, field: Field, from: &str) -> Result<()> {
    json_store::update::<NamesFile, _>(&paths.field_names(), |file| {
        if let Some(rules) = file.renames.get_mut(field.as_str()) {
            rules.shift_remove(from);
            if rules.is_empty() {
                file.renames.shift_remove(field.as_str());
            }
        }
        Ok(())
    })
}

/// The names are different things: no longer suggested as duplicates.
pub fn keep_apart(paths: &DataPaths, field: Field, names: &[String]) -> Result<()> {
    json_store::update::<NamesFile, _>(&paths.field_names(), |file| {
        file.apart.entry(field.as_str().to_owned()).or_default().push(names.to_vec());
        Ok(())
    })
}

/// One name of a field with the games that have it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldValue {
    /// The most common spelling.
    pub name: String,
    pub key: String,
    pub games: usize,
}

/// The names of `field` across `games`, sorted by name. Spellings that only
/// differ in case or accents are one name, as in the View menu.
pub fn values(games: &[Arc<CollectionGame>], field: Field) -> Vec<FieldValue> {
    let mut by_key: HashMap<String, (IndexMap<String, usize>, usize)> = HashMap::new();
    for game in games {
        let mut seen = HashSet::new();
        for kind in field.facet_kinds() {
            for item in &game.facets[kind.index()] {
                let entry = by_key.entry(item.key.clone()).or_default();
                *entry.0.entry(item.value.clone()).or_default() += 1;
                if seen.insert(item.key.as_str()) {
                    entry.1 += 1;
                }
            }
        }
    }
    let mut out: Vec<FieldValue> = by_key
        .into_iter()
        .map(|(key, (spellings, games))| {
            // `rev`: on a tie the first spelling seen wins.
            let name = spellings.iter().rev().max_by_key(|(_, n)| **n).map(|(s, _)| s.clone()).unwrap_or_default();
            FieldValue { name, key, games }
        })
        .collect();
    out.sort_by(|a, b| a.key.cmp(&b.key).then_with(|| a.name.cmp(&b.name)));
    out
}

#[cfg(test)]
mod tests;
