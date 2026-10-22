//! Games in the Collection twice: added here by hand, and brought in from
//! Lutris, Bottles or Faugus. The usual way there: the game was installed with Lutris,
//! added here by pointing at its executable in Lutris' Wine prefix, then the
//! import brought Lutris' entry in. [`find`] pairs them so the user can merge
//! the two (`heronite_data::carry_over`), keep only the launcher's, or keep
//! both ([`ImportPrefs::kept_apart`](heronite_data::launchers::ImportPrefs)).

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use heronite_data::launchers::{kept_apart_key, Launcher, LAUNCH_KIND, PREFIX_KEY};
use heronite_data::{fuzzy, game_config, json_store, CollectionGame, DataPaths, Library};
use serde_json::Value;

/// What tells the two are the same game, strongest first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Why {
    /// The game added here runs in the launcher's Wine prefix.
    Prefix,
    /// Its executable is in the launcher's game folder (or prefix).
    Folder,
    /// Same title.
    Title,
}

/// One side of a pair.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Side {
    pub app_name: String,
    /// `CollectionGame.key`.
    pub key: String,
    pub title: String,
    pub minutes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Duplicate {
    /// The game added here by hand.
    pub mine: Side,
    /// The launcher's.
    pub theirs: Side,
    pub launcher: Launcher,
    pub why: Why,
}

impl Duplicate {
    /// Its name in [`ImportPrefs::kept_apart`](heronite_data::launchers::ImportPrefs).
    pub fn id(&self) -> String {
        kept_apart_key(&self.mine.app_name, &self.theirs.app_name)
    }
}

fn side(g: &CollectionGame) -> Side {
    Side { app_name: g.app_name.clone(), key: g.key.clone(), title: g.title.clone(), minutes: g.playtime_minutes }
}

/// Title as compared: folded, `&` as "and", letters and digits only.
fn title_key(title: &str) -> String {
    fuzzy::fold(&title.replace('&', " and ")).chars().filter(|c| c.is_alphanumeric()).collect()
}

fn absolute(p: &str, home: &str) -> Option<PathBuf> {
    let p = game_config::expand_tilde(p.trim(), home);
    let p = PathBuf::from(p.trim_end_matches('/'));
    (p.is_absolute() && p.components().count() > 2).then_some(p)
}

/// The prefix set in the game's own settings (not the default one).
fn own_prefix(paths: &DataPaths, app: &str, home: &str) -> Option<PathBuf> {
    let doc = json_store::read_value(&paths.game_config(app)).ok()?;
    absolute(doc.get(app)?.get("winePrefix")?.as_str()?, home)
}

/// A game added here by hand that runs a program of this computer.
fn is_mine(g: &CollectionGame) -> bool {
    g.runner == "sideload"
        && g.steam_launch_id.is_none()
        && g.meta.as_ref().is_none_or(|m| Launcher::from_source(&m.source).is_none())
        && matches!(g.launch_kind.as_deref(), None | Some("" | "executable"))
        && !g.game.install.platform.as_deref().is_some_and(|p| p.eq_ignore_ascii_case("browser"))
}

/// The game of Lutris, Bottles or Faugus behind `g`, when it is one still installed.
fn launcher_of(g: &CollectionGame) -> Option<Launcher> {
    let m = g.meta.as_ref()?;
    (g.installed && m.launch_kind == LAUNCH_KIND).then(|| Launcher::from_source(&m.source)).flatten()
}

fn same_or_inside(path: &Path, dir: &Path) -> bool {
    path.starts_with(dir) || matches!((path.canonicalize(), dir.canonicalize()), (Ok(p), Ok(d)) if p.starts_with(&d))
}

/// The pairs in `lib` (loaded with its hidden games), but the ones in
/// `kept_apart`. A game added here pairs with one launcher game at most, the
/// strongest match (the same name breaks a tie); a prefix or folder shared by
/// several launcher games only matches by name, and a tie the name does not
/// break pairs nothing.
pub fn find(lib: &Library, paths: &DataPaths, home: &str, kept_apart: &BTreeSet<String>) -> Vec<Duplicate> {
    struct Theirs<'a> {
        game: &'a CollectionGame,
        launcher: Launcher,
        folder: Option<PathBuf>,
        prefix: Option<PathBuf>,
        title: String,
    }
    let theirs: Vec<Theirs> = lib
        .games
        .iter()
        .filter_map(|g| {
            let launcher = launcher_of(g)?;
            let prefix = g.meta.as_ref().and_then(|m| m.extra.get(PREFIX_KEY)).and_then(Value::as_str).and_then(|p| absolute(p, home));
            let folder = g.game.install.install_path.as_deref().and_then(|p| absolute(p, home));
            Some(Theirs { game: g, launcher, folder, prefix, title: title_key(&g.title) })
        })
        .collect();
    if theirs.is_empty() {
        return Vec::new();
    }
    // A prefix or folder several launcher games share (a Bottles bottle, a
    // Lutris prefix for many games) does not tell which one a game is.
    fn own(all: &[Option<&Path>], p: Option<&Path>) -> bool {
        p.is_some_and(|p| all.iter().flatten().filter(|q| same_or_inside(q, p) || same_or_inside(p, q)).count() == 1)
    }
    let prefixes: Vec<Option<&Path>> = theirs.iter().map(|t| t.prefix.as_deref()).collect();
    let folders: Vec<Option<&Path>> = theirs.iter().map(|t| t.folder.as_deref()).collect();
    let unique: Vec<(bool, bool)> =
        theirs.iter().map(|t| (own(&prefixes, t.prefix.as_deref()), own(&folders, t.folder.as_deref()))).collect();
    let mut out = Vec::new();
    for mine in lib.games.iter().filter(|g| is_mine(g)) {
        let exe = mine
            .meta
            .as_ref()
            .and_then(|m| m.remapped_executable.clone())
            .or_else(|| mine.game.install.executable.clone())
            .and_then(|e| absolute(&e, home));
        let prefix = own_prefix(paths, &mine.app_name, home);
        let title = title_key(&mine.title);
        let mut found: Vec<((Why, bool), &Theirs)> = theirs
            .iter()
            .zip(&unique)
            .filter_map(|(t, &(own_prefix, own_folder))| {
                let in_prefix = |p: &Path| own_prefix && t.prefix.as_deref().is_some_and(|tp| same_or_inside(p, tp));
                let in_folder = |p: &Path| own_folder && t.folder.as_deref().is_some_and(|tf| same_or_inside(p, tf));
                let same_title = !title.is_empty() && title == t.title;
                let why = if prefix.as_deref().is_some_and(in_prefix) {
                    Why::Prefix
                } else if exe.as_deref().is_some_and(|e| in_folder(e) || in_prefix(e)) {
                    Why::Folder
                } else if same_title {
                    Why::Title
                } else {
                    return None;
                };
                Some(((why, !same_title), t))
            })
            .collect();
        found.sort_by_key(|(rank, _)| *rank);
        // Two launcher games as likely: no telling which one it is.
        let best = match found.as_slice() {
            [first, second, ..] if first.0 == second.0 => None,
            [first, ..] => Some((first.0 .0, first.1)),
            [] => None,
        };
        if let Some((why, t)) = best {
            let pair = Duplicate { mine: side(mine), theirs: side(t.game), launcher: t.launcher, why };
            if !kept_apart.contains(&pair.id()) {
                out.push(pair);
            }
        }
    }
    out.sort_by_cached_key(|d| (d.theirs.title.to_lowercase(), d.mine.title.to_lowercase()));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{collection, Game};
    use serde_json::json;

    fn lutris(id: &str, title: &str, folder: &str, prefix: Option<&str>) -> Game {
        Game {
            id: id.into(),
            title: title.into(),
            folder: Some(folder.into()),
            windows: true,
            command: format!("lutris lutris:rungameid/{id}"),
            prefix: prefix.map(Into::into),
            ..Default::default()
        }
    }

    fn add_mine(paths: &DataPaths, games: &[(&str, &str, &str)]) {
        let mut doc = json_store::read_value(&paths.sideload_library()).ok().filter(Value::is_object).unwrap_or(json!({}));
        let list = doc.as_object_mut().unwrap().entry("games").or_insert(json!([])).as_array_mut().unwrap();
        for (app, title, exe) in games {
            list.push(json!({"runner": "sideload", "app_name": app, "title": title, "is_installed": true,
                             "install": {"executable": exe, "platform": "windows"}}));
        }
        json_store::write(&paths.sideload_library(), &doc).unwrap();
    }

    #[test]
    fn pairs_by_prefix_folder_and_title() {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path().join("data"));
        let home = "/home/u";
        collection::sync(
            &paths,
            Launcher::Lutris,
            &[
                lutris("7", "Heroes of Might & Magic III", "/home/u/Games/homm3", Some("/home/u/Games/homm3")),
                lutris("8", "Celeste", "/home/u/Games/celeste", Some("/home/u/Games/celeste")),
                lutris("9", "Hades", "/home/u/Games/hades", Some("/home/u/Games/hades-prefix")),
                lutris("10", "Portal", "/home/u/Games/portal", None),
            ],
            &BTreeSet::new(),
        )
        .unwrap();
        add_mine(
            &paths,
            &[
                // In Lutris' prefix, under another name (the user's case).
                ("m1", "HoMM3 Launcher", "/home/u/Games/homm3/drive_c/HoMM3/HD_Launcher.exe"),
                ("m2", "My Celeste", "/home/u/Downloads/celeste.exe"),
                ("m3", "hades", "/opt/hades/Hades.exe"),
                ("m4", "Unrelated", "/opt/x/x.exe"),
                ("m5", "Heroes of Might and Magic III", "/opt/h3/h3.exe"),
            ],
        );
        // m2 runs in Celeste's prefix.
        game_config::set_game_setting(&paths, "m2", home, "winePrefix", json!("~/Games/celeste")).unwrap();

        let lib = Library::load_with(&paths, true).unwrap();
        let found = find(&lib, &paths, home, &BTreeSet::new());
        let pairs: Vec<_> = found.iter().map(|d| (d.mine.app_name.as_str(), d.theirs.app_name.as_str(), d.why)).collect();
        assert_eq!(
            pairs,
            [
                ("m2", "lutris_8", Why::Prefix),
                ("m3", "lutris_9", Why::Title),
                ("m5", "lutris_7", Why::Title),
                ("m1", "lutris_7", Why::Folder)
            ]
        );
        assert_eq!(found[0].launcher, Launcher::Lutris);
        assert_eq!(found[0].theirs.key, "sideload_lutris_8");

        // Kept apart: not offered again.
        let kept: BTreeSet<String> = [kept_apart_key("m3", "lutris_9")].into();
        assert_eq!(find(&lib, &paths, home, &kept).len(), 3);
    }

    /// A prefix several launcher games share (a Bottles bottle, one Lutris
    /// prefix for many games) never pairs a game with the wrong one.
    #[test]
    fn a_shared_prefix_only_pairs_by_name_or_own_folder() {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path().join("data"));
        let home = "/home/u";
        collection::sync(
            &paths,
            Launcher::Lutris,
            &[
                lutris("1", "Hollow Knight", "/g/Hollow Knight", Some("/g/Prex")),
                lutris("2", "Celeste", "/g/Celeste", Some("/g/Prex")),
                lutris("3", "Celeste", "/h/Celeste", Some("/h/pfx")),
            ],
            &BTreeSet::new(),
        )
        .unwrap();
        add_mine(
            &paths,
            &[
                ("m1", "Hades", "/g/Prex/drive_c/Hades/Hades.exe"),
                ("m2", "My HK", "/g/Hollow Knight/hollow_knight.exe"),
                ("m3", "Celeste", "/opt/celeste/Celeste.exe"),
            ],
        );
        for app in ["m1", "m2", "m3"] {
            game_config::set_game_setting(&paths, app, home, "winePrefix", json!("/g/Prex")).unwrap();
        }
        let lib = Library::load_with(&paths, true).unwrap();
        let pairs: Vec<_> = find(&lib, &paths, home, &BTreeSet::new())
            .iter()
            .map(|d| (d.mine.app_name.clone(), d.theirs.app_name.clone(), d.why))
            .collect();
        // m1: only the shared prefix, so no pair. m2: Hollow Knight's own
        // folder. m3: two launcher games named Celeste, so no telling which.
        assert_eq!(pairs, [("m2".to_string(), "lutris_1".to_string(), Why::Folder)]);
    }

    #[test]
    fn nothing_without_launcher_games() {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path());
        add_mine(&paths, &[("m1", "Celeste", "/opt/c/c.exe"), ("m2", "Celeste", "/opt/d/c.exe")]);
        let lib = Library::load_with(&paths, true).unwrap();
        assert!(find(&lib, &paths, "/home/u", &BTreeSet::new()).is_empty());
    }
}
