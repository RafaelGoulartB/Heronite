//! Names that look like one: the suggestions of the fields manager. Two
//! names go together when any of their simplified forms match:
//! - the name without case, accents, spaces and punctuation ("Co-op" =
//!   "coop", "Multi-player" = "Multiplayer");
//! - the part before and the part inside brackets ("Role-playing (RPG)" =
//!   "RPG", "PC (Windows)" = "Windows");
//! - for companies, without legal and generic endings ("Valve Corporation" =
//!   "Valve", "Warner Bros. Interactive Entertainment" = "Warner Bros.");
//! - for platforms, without the maker in front ("Sony PlayStation 2" =
//!   "PlayStation 2");
//! - a few common spellings ("Co-operative" = "Co-op").
//!
//! They are only suggestions: the user merges them or says they differ.

use std::collections::{HashMap, HashSet};

use super::{Field, FieldValue};
use crate::text::facet_key;

/// Endings of company names that say nothing about which company it is.
const LEGAL: [&str; 26] = [
    "inc",
    "incorporated",
    "llc",
    "ltd",
    "limited",
    "corp",
    "corporation",
    "co",
    "company",
    "gmbh",
    "ag",
    "sa",
    "sas",
    "sarl",
    "srl",
    "spa",
    "plc",
    "pty",
    "bv",
    "nv",
    "ab",
    "oy",
    "as",
    "kk",
    "lp",
    "the",
];

/// Generic words at the end of company names ("2K Games", "Sega Games").
const COMPANY_WORDS: [&str; 16] = [
    "games",
    "game",
    "entertainment",
    "interactive",
    "studios",
    "studio",
    "software",
    "productions",
    "publishing",
    "digital",
    "media",
    "group",
    "usa",
    "america",
    "europe",
    "international",
];

/// Makers in front of platform names.
const MAKERS: [&str; 10] = ["microsoft", "sony", "nintendo", "sega", "atari", "nec", "snk", "commodore", "apple", "google"];

/// Spellings of one thing that the other rules miss.
const SAME: [(&str, &str); 7] = [
    ("cooperative", "coop"),
    ("roleplaying", "rpg"),
    ("roleplayinggame", "rpg"),
    ("realtimestrategy", "rts"),
    ("turnbasedstrategy", "tbs"),
    ("firstpersonshooter", "fps"),
    ("massivelymultiplayeronline", "mmo"),
];

fn words(text: &str) -> Vec<String> {
    facet_key(text).split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).map(str::to_owned).collect()
}

/// The simplified forms of one part of a name (no brackets).
fn forms_of(field: Field, text: &str, out: &mut Vec<String>) {
    let mut w = words(text);
    if w.is_empty() {
        return;
    }
    out.push(w.concat());
    if w.len() > 1 && w[0] == "the" {
        w.remove(0);
    }
    if field == Field::Company {
        while w.len() > 1 && w.last().is_some_and(|l| LEGAL.contains(&l.as_str()) || COMPANY_WORDS.contains(&l.as_str())) {
            w.pop();
        }
    }
    if field == Field::Platform
        && w.len() > 1
        && MAKERS.contains(&w[0].as_str())
        && w[1..].iter().any(|x| x.chars().any(char::is_alphabetic))
    {
        w.remove(0);
    }
    let squashed = w.concat();
    if let Some((_, same)) = SAME.iter().find(|(long, _)| *long == squashed) {
        out.push((*same).to_owned());
    }
    // "VR Supported" = "VR Support".
    if squashed.len() > 6 {
        if let Some(stem) = squashed.strip_suffix("ed") {
            out.push(stem.to_owned());
        }
    }
    out.push(squashed);
}

/// Every simplified form of a name, brackets included.
fn forms(field: Field, name: &str) -> Vec<String> {
    let mut out = Vec::new();
    forms_of(field, name, &mut out);
    if let (Some(open), Some(close)) = (name.find('('), name.rfind(')')) {
        if open < close {
            let outer = format!("{} {}", &name[..open], &name[close + 1..]);
            forms_of(field, &outer, &mut out);
            forms_of(field, &name[open + 1..close], &mut out);
        }
    }
    out.retain(|f| f.chars().count() >= 2);
    out.sort();
    out.dedup();
    out
}

fn find(parent: &mut [usize], i: usize) -> usize {
    let mut root = i;
    while parent[root] != root {
        root = parent[root];
    }
    let mut i = i;
    while parent[i] != root {
        let next = parent[i];
        parent[i] = root;
        i = next;
    }
    root
}

/// Groups of `values` that look like one name, as indexes: the most used name
/// first in each group, the groups with the most games first. Groups the user
/// kept apart (`apart`, from `names.json`) are left out.
pub fn similar_groups(field: Field, values: &[FieldValue], apart: &[Vec<String>]) -> Vec<Vec<usize>> {
    let mut parent: Vec<usize> = (0..values.len()).collect();
    let mut owner: HashMap<String, usize> = HashMap::new();
    for (i, value) in values.iter().enumerate() {
        for form in forms(field, &value.name) {
            match owner.get(&form) {
                Some(&j) => {
                    let (a, b) = (find(&mut parent, i), find(&mut parent, j));
                    if a != b {
                        parent[a] = b;
                    }
                }
                None => {
                    owner.insert(form, i);
                }
            }
        }
    }
    let mut groups: HashMap<usize, Vec<usize>> = HashMap::new();
    for i in 0..values.len() {
        let root = find(&mut parent, i);
        groups.entry(root).or_default().push(i);
    }
    let apart: Vec<HashSet<String>> = apart.iter().map(|g| g.iter().map(|n| facet_key(n)).collect()).collect();
    let mut out: Vec<Vec<usize>> = groups
        .into_values()
        .filter(|g| g.len() > 1)
        .filter(|g| !apart.iter().any(|set| g.iter().all(|&i| set.contains(&values[i].key))))
        .map(|mut g| {
            g.sort_by(|&a, &b| values[b].games.cmp(&values[a].games).then_with(|| values[a].key.cmp(&values[b].key)));
            g
        })
        .collect();
    let total = |g: &Vec<usize>| g.iter().map(|&i| values[i].games).sum::<usize>();
    out.sort_by(|a, b| total(b).cmp(&total(a)).then_with(|| values[a[0]].key.cmp(&values[b[0]].key)));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vals(names: &[(&str, usize)]) -> Vec<FieldValue> {
        names.iter().map(|(n, g)| FieldValue { name: n.to_string(), key: facet_key(n), games: *g }).collect()
    }

    fn named(field: Field, names: &[(&str, usize)]) -> Vec<Vec<String>> {
        let v = vals(names);
        similar_groups(field, &v, &[]).into_iter().map(|g| g.into_iter().map(|i| v[i].name.clone()).collect()).collect()
    }

    #[test]
    fn companies_without_legal_and_generic_endings() {
        let groups = named(
            Field::Company,
            &[
                ("Valve", 3),
                ("Valve Corporation", 1),
                ("Psyonix LLC", 1),
                ("Psyonix", 1),
                ("Warner Bros.", 3),
                ("Warner Bros. Interactive Entertainment", 1),
                ("Warner Bros. Entertainment Inc.", 1),
                ("Rockstar North", 2),
                ("Rockstar Games", 3),
                ("Ubisoft Montreal", 3),
            ],
        );
        assert!(groups.contains(&vec!["Valve".to_string(), "Valve Corporation".to_string()]));
        assert!(groups.contains(&vec!["Psyonix".to_string(), "Psyonix LLC".to_string()]));
        let wb = groups.iter().find(|g| g[0] == "Warner Bros.").unwrap();
        assert_eq!(wb.len(), 3);
        assert!(!groups.iter().any(|g| g.contains(&"Rockstar North".to_string())), "another studio of the group: {groups:?}");
        assert!(!groups.iter().any(|g| g.contains(&"Ubisoft Montreal".to_string())));
    }

    #[test]
    fn brackets_and_spellings() {
        let genres = named(Field::Genre, &[("Role-playing (RPG)", 71), ("RPG", 1), ("Shooter", 54), ("Strategy", 31)]);
        assert_eq!(genres, vec![vec!["Role-playing (RPG)".to_string(), "RPG".to_string()]]);

        let features = named(
            Field::Feature,
            &[("Co-op", 26), ("Co-Operative", 30), ("Multi-player", 34), ("Multiplayer", 41), ("VR Support", 2), ("VR Supported", 2)],
        );
        assert_eq!(features.len(), 3, "{features:?}");
        assert_eq!(features[0], ["Multiplayer", "Multi-player"]);
    }

    #[test]
    fn platforms_without_the_maker() {
        let groups = named(
            Field::Platform,
            &[
                ("PC (Microsoft Windows)", 170),
                ("PC (Windows)", 16),
                ("Windows", 1),
                ("Sony PlayStation 2", 2),
                ("PlayStation 2", 6),
                ("Nintendo 64", 1),
                ("Commodore 64", 1),
            ],
        );
        assert_eq!(groups[0], ["PC (Microsoft Windows)", "PC (Windows)", "Windows"]);
        assert_eq!(groups[1], ["PlayStation 2", "Sony PlayStation 2"]);
        assert_eq!(groups.len(), 2, "64 alone is not a platform: {groups:?}");
    }

    #[test]
    fn kept_apart_is_not_suggested() {
        let v = vals(&[("Valve", 3), ("Valve Corporation", 1)]);
        assert!(similar_groups(Field::Company, &v, &[vec!["valve corporation".into(), "Valve".into()]]).is_empty());
    }
}
