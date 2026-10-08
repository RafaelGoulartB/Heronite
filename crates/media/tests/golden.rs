//! Differential tests against values produced by the original `match.ts`
//! (run with Node over the same inputs; see `tests/golden/*.tsv`).

use heronite_media::matching::{
    core_screenshot_name, normalize_screenshot_name, pick_screenshot_folder, score_screenshot_folder, MIN_SCORE,
};
use std::collections::{HashMap, HashSet};

const SCORES: &str = include_str!("golden/score_golden.tsv");
const NORMS: &str = include_str!("golden/norm_golden.tsv");
const PICKS: &str = include_str!("golden/pick_golden.tsv");

const REAL_FOLDERS: &[&str] = &[
    "Baldur's Gate 3", "Black Myth - Wukong", "Brotato", "Clair Obscur - Expedition 33", "Control AWE",
    "Crimson Desert", "Cyberpunk 2077", "Death Stranding - Director's Cut", "Death Stranding 2 - On the Beach",
    "Elden Ring", "Forza Horizon 6", "Ghost of Tsushima - Director's Cut", "God of War",
    "Guitar Hero III - Legends of Rock", "Kingdom Come - Deliverance", "LEGO Batman - Legacy of the Dark Knight",
    "Lies of P", "Luna Abyss", "Metal Gear Solid Delta - Snake Eater", "Mortal Shell 2", "NieR - Automata",
    "Nioh 3", "Onimusha - Way of the Sword", "PRAGMATA", "Satisfactory", "Stellar Blade",
    "The First Berserker - Khazan", "Tomb Raider", "Wuchang - Fallen Feathers",
];
const EXTRA_FOLDERS: &[&str] = &[
    "Control", "Death Stranding", "Mortal Shell", "Nioh", "Nioh 2", "Tomb Raider (2013)", "Shadow of the Tomb Raider",
    "Rise of the Tomb Raider", "God of War Ragnarok", "God of War (2018)", "Half-Life 2", "Half Life", "Halflife 2",
    "The Witcher 3 - Wild Hunt", "Witcher 3", "Dark Souls III", "Dark Souls 3", "Dark Souls Remastered", "Pokémon",
    "Fallout 4 GOTY", "Fallout 4", "Fallout New Vegas", "Resident Evil 4", "Resident Evil 4 Remake", "1245620",
    "1245620 Elden Ring", "Tom & Jerry", "Tom and Jerry", "Metal Gear Solid Δ", "Metal Gear Solid Delta",
    "Game of the Year Edition", "Gold", "The", "A", "Divinity Original Sin 2 - Definitive Edition",
    "Divinity: Original Sin II", "Doom", "DOOM Eternal", "DOOM (2016)", "Grand Theft Auto V", "GTA V",
    "Grand Theft Auto IV", "Portal", "Portal 2", "Left 4 Dead 2", "Left 4 Dead", "Lego Batman", "Batman Arkham Knight",
    "Batman: Arkham City - Game of the Year Edition", "Alan Wake 2", "Alan Wake", "Remastered", "Edition Edition",
];

fn folders() -> Vec<&'static str> {
    REAL_FOLDERS.iter().chain(EXTRA_FOLDERS).copied().collect()
}

fn rows(text: &str) -> impl Iterator<Item = Vec<&str>> {
    text.lines().filter(|l| !l.is_empty()).map(|l| l.split('\t').collect())
}

#[test]
fn normalize_and_core_match_the_original() {
    let mut n = 0;
    for r in rows(NORMS) {
        assert_eq!(normalize_screenshot_name(r[0]), r[1], "normalize({:?})", r[0]);
        assert_eq!(core_screenshot_name(r[0]), r[2], "core({:?})", r[0]);
        n += 1;
    }
    assert!(n > 200);
}

#[test]
fn every_pair_score_matches_the_original() {
    let expected: HashMap<(&str, &str), i32> =
        rows(SCORES).map(|r| ((r[0], r[1]), r[2].parse().unwrap())).collect();
    // Titles = distinct first columns of the picks table (all candidate titles).
    let titles: HashSet<&str> = rows(PICKS).map(|r| r[0]).collect();
    let folders = folders();
    let mut pairs = 0;
    for t in &titles {
        for f in &folders {
            let want = expected.get(&(*t, *f)).copied().unwrap_or(0);
            assert_eq!(score_screenshot_folder(t, f), want, "score({t:?}, {f:?})");
            pairs += 1;
        }
    }
    assert!(pairs > 10_000, "{pairs}");
    assert!(expected.len() > 300);
}

#[test]
fn picks_match_the_original() {
    let folders = folders();
    let mut matched = 0;
    for r in rows(PICKS) {
        let got = pick_screenshot_folder(&[r[0]], &folders, None, MIN_SCORE);
        match (r.get(1).filter(|s| !s.is_empty()), got) {
            (None, None) => {}
            (Some(f), Some(m)) => {
                assert_eq!(&m.folder, f, "pick({:?})", r[0]);
                assert_eq!(m.score.to_string(), r[2], "score of pick({:?})", r[0]);
                matched += 1;
            }
            (want, got) => panic!("pick({:?}): want {want:?}, got {got:?}", r[0]),
        }
    }
    assert!(matched > 100);
}
