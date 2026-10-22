//! Which enabled mod wins each file: the later mod of the load order does.

use std::collections::{HashMap, HashSet};

use crate::fsutil::walk_files;
use crate::model::ModList;
use crate::paths::GameDirs;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ModConflicts {
    /// Mods this one wins files from, and how many files, most files first.
    pub overrides: Vec<(String, usize)>,
    /// Mods that win files from this one.
    pub overridden_by: Vec<(String, usize)>,
    /// Every file of the mod is won by another mod.
    pub fully_overridden: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Conflicts {
    /// One entry per enabled mod.
    pub per_mod: HashMap<String, ModConflicts>,
}

/// Compares the files of the enabled mods (paths compared ignoring case, as
/// the game would see them). One pass over the mod folders.
pub fn compute(dirs: &GameDirs, list: &ModList) -> Conflicts {
    let mods: Vec<&str> = list.enabled().map(|m| m.id.as_str()).collect();
    // Lower-cased path to the mods that have it, in load order.
    let mut owners: HashMap<String, Vec<u32>> = HashMap::new();
    let mut counts = vec![0usize; mods.len()];
    for (i, id) in mods.iter().enumerate() {
        walk_files(&dirs.mod_dir(id), &mut |rel, _| {
            counts[i] += 1;
            let list = owners.entry(rel.to_lowercase()).or_default();
            if list.last() != Some(&(i as u32)) {
                list.push(i as u32);
            }
        });
    }

    let mut over: Vec<HashMap<u32, usize>> = vec![HashMap::new(); mods.len()];
    let mut under: Vec<HashMap<u32, usize>> = vec![HashMap::new(); mods.len()];
    let mut lost = vec![0usize; mods.len()];
    for who in owners.values().filter(|w| w.len() > 1) {
        for (k, &w) in who.iter().enumerate() {
            if k + 1 < who.len() {
                lost[w as usize] += 1;
            }
            for &l in &who[..k] {
                *over[w as usize].entry(l).or_default() += 1;
                *under[l as usize].entry(w).or_default() += 1;
            }
        }
    }

    let named = |m: &HashMap<u32, usize>| {
        let mut v: Vec<(String, usize)> = m.iter().map(|(&i, &n)| (mods[i as usize].to_string(), n)).collect();
        v.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        v
    };
    let per_mod = mods
        .iter()
        .enumerate()
        .map(|(i, id)| {
            let c = ModConflicts {
                overrides: named(&over[i]),
                overridden_by: named(&under[i]),
                fully_overridden: counts[i] > 0 && lost[i] == counts[i],
            };
            (id.to_string(), c)
        })
        .collect();
    Conflicts { per_mod }
}

/// The files (as `winner` spells them) that `winner` and `loser` both have,
/// sorted.
pub fn files_between(dirs: &GameDirs, winner: &str, loser: &str) -> Vec<String> {
    let mut theirs: HashSet<String> = HashSet::new();
    walk_files(&dirs.mod_dir(loser), &mut |rel, _| {
        theirs.insert(rel.to_lowercase());
    });
    let mut both = Vec::new();
    walk_files(&dirs.mod_dir(winner), &mut |rel, _| {
        if theirs.contains(&rel.to_lowercase()) {
            both.push(rel.to_string());
        }
    });
    both.sort();
    both
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::ModEntry;
    use crate::paths::ModsRoot;
    use std::fs;

    fn setup(mods: &[(&str, bool, &[&str])]) -> (tempfile::TempDir, GameDirs, ModList) {
        let t = tempfile::tempdir().unwrap();
        let dirs = ModsRoot::at(t.path()).game("g");
        let mut list = ModList::new("g", "G");
        for (id, enabled, files) in mods {
            for f in *files {
                let p = dirs.mod_dir(id).join(f);
                fs::create_dir_all(p.parent().unwrap()).unwrap();
                fs::write(p, id).unwrap();
            }
            fs::create_dir_all(dirs.mod_dir(id)).unwrap();
            list.mods.push(ModEntry { id: id.to_string(), enabled: *enabled, ..Default::default() });
        }
        (t, dirs, list)
    }

    #[test]
    fn the_later_mod_wins_ignoring_case() {
        let (_t, dirs, list) = setup(&[
            ("a", true, &["Data/x.esp", "Data/Textures/t.dds", "Data/only-a.txt"]),
            ("b", true, &["data/textures/T.DDS", "Data/x.esp"]),
            ("off", false, &["Data/x.esp"]),
            ("c", true, &["Data/x.esp"]),
        ]);
        let c = compute(&dirs, &list);
        assert_eq!(c.per_mod.len(), 3, "disabled mods are left out");
        let a = &c.per_mod["a"];
        assert_eq!(a.overridden_by, [("b".to_string(), 2), ("c".to_string(), 1)]);
        assert!(a.overrides.is_empty() && !a.fully_overridden, "only-a.txt is still its own");
        let b = &c.per_mod["b"];
        assert_eq!((b.overrides.clone(), b.overridden_by.clone()), (vec![("a".to_string(), 2)], vec![("c".to_string(), 1)]));
        assert!(!b.fully_overridden);
        let c_ = &c.per_mod["c"];
        assert_eq!(c_.overrides, [("a".to_string(), 1), ("b".to_string(), 1)]);
        assert!(c_.overridden_by.is_empty() && !c_.fully_overridden);
    }

    #[test]
    fn a_mod_whose_files_are_all_taken_is_fully_overridden() {
        let (_t, dirs, list) = setup(&[("a", true, &["f1", "d/f2"]), ("b", true, &["F1", "D/F2", "extra"]), ("empty", true, &[])]);
        let c = compute(&dirs, &list);
        assert!(c.per_mod["a"].fully_overridden);
        assert!(!c.per_mod["b"].fully_overridden);
        assert!(!c.per_mod["empty"].fully_overridden);
    }

    #[test]
    fn lists_the_files_two_mods_share() {
        let (_t, dirs, _list) = setup(&[("a", true, &["Data/B.esp", "Data/a.esp", "x"]), ("b", true, &["data/b.esp", "Data/A.esp", "y"])]);
        assert_eq!(files_between(&dirs, "b", "a"), ["Data/A.esp", "data/b.esp"]);
        assert!(files_between(&dirs, "b", "missing").is_empty());
    }

    #[test]
    fn is_fast_for_a_big_list() {
        let t = tempfile::tempdir().unwrap();
        let dirs = ModsRoot::at(t.path()).game("g");
        let mut list = ModList::new("g", "G");
        for m in 0..40 {
            let id = format!("m{m}");
            for d in 0..20 {
                let dir = dirs.mod_dir(&id).join(format!("Data/dir{d}"));
                fs::create_dir_all(&dir).unwrap();
                for f in 0..60 {
                    fs::write(dir.join(format!("f{}.dds", f + m % 7)), "").unwrap();
                }
            }
            list.mods.push(ModEntry { id, enabled: true, ..Default::default() });
        }
        let start = std::time::Instant::now();
        let c = compute(&dirs, &list);
        assert_eq!(c.per_mod.len(), 40);
        assert!(start.elapsed() < std::time::Duration::from_secs(5), "{:?}", start.elapsed());
        assert!(!c.per_mod["m0"].overridden_by.is_empty());
    }
}
