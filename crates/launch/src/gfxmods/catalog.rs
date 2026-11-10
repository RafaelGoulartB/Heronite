//! Heronite's catalog of graphics mods per game, `data/graphics-mods.json`
//! in the repository. It is fetched from its raw address at most daily, like
//! AreWeAntiCheatYet and ReShade's `Compatibility.ini`, so it changes without
//! a release; the copy built into the app is the fallback (first start,
//! offline). `scripts/graphics-mods-catalog.py` builds it from RenoDX's mod
//! list; its output lands only through a reviewed pull request.
//!
//! A game is matched by its Steam app id, then its executable's file name,
//! then its title (compared without case, marks or punctuation). A game
//! with no entry gets the generic mod of its engine when the files say
//! Unreal or Unity.

use std::path::{Path, PathBuf};

use heronite_data::DataPaths;
use serde::Deserialize;

use super::download::{download, fresh};
use crate::net::Net;

pub const URL: &str = "https://raw.githubusercontent.com/RafaelGoulartB/Heronite/main/data/graphics-mods.json";
/// The newest format this build reads; a catalog with a higher `schema` is
/// ignored (an older app keeps its cached or built-in copy).
pub const SCHEMA: u32 = 1;
const BUILT_IN: &str = include_str!("../../../../data/graphics-mods.json");

#[derive(Debug, Clone, Default, Deserialize)]
pub struct Catalog {
    pub schema: u32,
    #[serde(default)]
    pub generic: Generic,
    #[serde(default)]
    pub games: Vec<Entry>,
}

/// Mods that fit every game of an engine.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Generic {
    pub unreal: Option<RenoDx>,
    pub unity: Option<RenoDx>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Entry {
    pub id: String,
    #[serde(rename = "match")]
    pub matcher: Match,
    pub renodx: Option<RenoDx>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct Match {
    #[serde(default)]
    pub steam: Vec<String>,
    /// Executable file names.
    #[serde(default)]
    pub exe: Vec<String>,
    #[serde(default)]
    pub title: Vec<String>,
}

/// A RenoDX mod: a ReShade add-on, one file per architecture.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct RenoDx {
    pub slug: String,
    /// The generic mods' name (a game's mod goes by the game's title).
    #[serde(default)]
    pub name: Option<String>,
    /// `working` or `testing` (RenoDX's "in progress, lacking testing").
    pub status: String,
    pub addon64: Option<String>,
    pub addon32: Option<String>,
    /// RenoDX's note for the game.
    #[serde(default)]
    pub note: Option<String>,
}

impl RenoDx {
    pub fn working(&self) -> bool {
        self.status == "working"
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Engine {
    Unreal,
    Unity,
}

/// How a game was matched.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum How {
    Steam,
    Exe,
    Title,
    /// No entry: the generic mod of the engine.
    Engine(Engine),
}

/// The RenoDX mod for a game.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    /// The entry's id, or `generic-unreal` / `generic-unity`.
    pub id: String,
    /// The game's title in the catalog, or the generic mod's name.
    pub name: String,
    pub mod_: RenoDx,
    pub how: How,
}

/// The catalog in `text` (`None`: unreadable, or a newer format).
pub fn parse(text: &str) -> Option<Catalog> {
    match serde_json::from_str::<Catalog>(text) {
        Ok(c) if c.schema <= SCHEMA => Some(c),
        Ok(c) => {
            log::info!("graphics mods catalog: schema {} is newer than {SCHEMA}; ignored", c.schema);
            None
        }
        Err(e) => {
            log::warn!("graphics mods catalog unreadable: {e}");
            None
        }
    }
}

fn file(paths: &DataPaths) -> PathBuf {
    paths.tools_dir().join("gfxmods").join("graphics-mods.json")
}

/// The catalog, refreshed when older than a day (`refresh`: now). Offline
/// or on any problem: the cached copy, then the built-in one.
pub fn load(paths: &DataPaths, net: &dyn Net, refresh: bool) -> Catalog {
    let path = file(paths);
    if (refresh || !fresh(&path)) && net.is_online() {
        let tmp = path.with_extension("new");
        let res = std::fs::create_dir_all(path.parent().unwrap_or(Path::new(".")))
            .map_err(|e| e.to_string())
            .and_then(|_| download(net, URL, &tmp, &|| false));
        match res {
            // A catalog this build cannot read never replaces one it can.
            Ok(()) if std::fs::read_to_string(&tmp).ok().as_deref().and_then(parse).is_some() => {
                let _ = std::fs::rename(&tmp, &path);
            }
            Ok(()) => {
                let _ = std::fs::remove_file(&tmp);
                checked(&path);
            }
            Err(e) => {
                log::info!("graphics mods catalog not refreshed: {e}");
                checked(&path);
            }
        }
    }
    cached(paths)
}

/// Marks the cached copy as checked now (the built-in one when there is
/// none), so a failed or unreadable download is tried again the next day,
/// not at every look at a game.
fn checked(path: &Path) {
    let touched = std::fs::File::options().write(true).open(path).and_then(|f| f.set_modified(std::time::SystemTime::now()));
    if touched.is_err() {
        let _ = std::fs::write(path, BUILT_IN);
    }
}

/// The cached catalog without asking the network, else the built-in one.
pub fn cached(paths: &DataPaths) -> Catalog {
    std::fs::read_to_string(file(paths)).ok().as_deref().and_then(parse).unwrap_or_else(built_in)
}

pub fn built_in() -> Catalog {
    parse(BUILT_IN).unwrap_or_default()
}

/// A title as the catalog compares it: lowercase letters and digits only,
/// `&` as `and`, without ™ and ®.
pub fn norm(title: &str) -> String {
    title.to_lowercase().replace('&', "and").chars().filter(|c| c.is_alphanumeric()).collect()
}

/// A catalog title without RenoDX's qualifiers in parentheses ("(Remake)",
/// "(DX12)", "(2018)"), which store titles do not carry. A year before 2010
/// stays: "Most Wanted (2005)" is not the 2012 game of the same name.
fn strip_qualifiers(title: &str) -> String {
    let (mut depth, mut group, mut out) = (0u32, String::new(), String::new());
    for c in title.chars() {
        match c {
            '(' => {
                depth += 1;
                group.clear();
            }
            ')' if depth > 0 => {
                depth -= 1;
                let old_year = group.len() == 4 && group.parse::<u32>().is_ok_and(|y| y < 2010);
                if depth == 0 && old_year {
                    out.push_str(&format!("({group})"));
                }
            }
            _ if depth > 0 => group.push(c),
            _ => out.push(c),
        }
    }
    out
}

/// The looser keys of a catalog title: without its qualifiers, and each
/// game of a title that joins several with `&` ("Borderlands 2 & The
/// Pre-Sequel").
fn loose_keys(title: &str) -> Vec<String> {
    let plain = strip_qualifiers(title);
    let mut keys = vec![norm(&plain)];
    keys.extend(plain.split('&').map(norm).filter(|k| k.len() >= 6));
    keys
}

/// The engine the game's files show: Unreal's `<name>-<platform>-Shipping`
/// executable or its `Engine` folder, Unity's `UnityPlayer.dll`.
pub fn engine(exe: &Path) -> Option<Engine> {
    let dir = exe.parent()?;
    if dir.join("UnityPlayer.dll").is_file() {
        return Some(Engine::Unity);
    }
    let name = exe.file_name()?.to_string_lossy().to_ascii_lowercase();
    // `<root>/<Game>/Binaries/Win64/<exe>`: the engine sits beside `<Game>`.
    let root = dir.parent().and_then(Path::parent).and_then(Path::parent);
    if name.ends_with("-shipping.exe") || root.is_some_and(|r| r.join("Engine").join("Binaries").is_dir()) {
        return Some(Engine::Unreal);
    }
    None
}

impl Catalog {
    /// The RenoDX mod for a game: by Steam app id, executable name or
    /// title, then by engine.
    pub fn renodx_for(&self, steam_app_id: Option<&str>, exe: Option<&Path>, title: &str) -> Option<Found> {
        let exe_name = exe.and_then(Path::file_name).map(|n| n.to_string_lossy().to_ascii_lowercase());
        let title = norm(title);
        let with_mod = || self.games.iter().filter_map(|e| e.renodx.as_ref().map(|r| (e, r)));
        let by = |how: How, hit: &dyn Fn(&Entry) -> bool| {
            with_mod().find(|(e, _)| hit(e)).map(|(e, r)| Found {
                id: e.id.clone(),
                name: e.matcher.title.first().cloned().unwrap_or_else(|| e.id.clone()),
                mod_: r.clone(),
                how,
            })
        };
        steam_app_id
            .filter(|id| !id.is_empty())
            .and_then(|id| by(How::Steam, &|e| e.matcher.steam.iter().any(|s| s == id)))
            .or_else(|| exe_name.as_ref().and_then(|n| by(How::Exe, &|e| e.matcher.exe.iter().any(|x| x.eq_ignore_ascii_case(n)))))
            // The exact title first, then without RenoDX's qualifiers.
            .or_else(|| (!title.is_empty()).then(|| by(How::Title, &|e| e.matcher.title.iter().any(|t| norm(t) == title))).flatten())
            .or_else(|| {
                (!title.is_empty()).then(|| by(How::Title, &|e| e.matcher.title.iter().any(|t| loose_keys(t).contains(&title)))).flatten()
            })
            .or_else(|| {
                let engine = exe.and_then(engine)?;
                let (id, generic) = match engine {
                    Engine::Unreal => ("generic-unreal", self.generic.unreal.as_ref()?),
                    Engine::Unity => ("generic-unity", self.generic.unity.as_ref()?),
                };
                Some(Found {
                    id: id.into(),
                    name: generic.name.clone().unwrap_or_else(|| generic.slug.clone()),
                    mod_: generic.clone(),
                    how: How::Engine(engine),
                })
            })
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::net::FakeNet;

    pub(crate) const SAMPLE: &str = r#"{
 "schema": 1,
 "generic": {"unreal": {"name": "Unreal Engine Extended", "slug": "ue-extended", "status": "working", "addon64": "https://marat569.github.io/renodx/renodx-ue-extended.addon64"}},
 "games": [
  {"id": "stalker2", "match": {"steam": ["1643320"], "exe": ["Stalker2-Win64-Shipping.exe"], "title": ["S.T.A.L.K.E.R. 2: Heart of Chornobyl"]},
   "renodx": {"slug": "stalker2", "status": "working", "addon64": "https://clshortfuse.github.io/renodx/renodx-stalker2.addon64"}},
  {"id": "acii", "match": {"title": ["Assassin’s Creed® II", "Assassin’s Creed® Brotherhood"]},
   "renodx": {"slug": "acii", "status": "testing", "addon32": "https://github.com/x/renodx/releases/download/snapshot/renodx-acii.addon32", "note": "Disable AA"}},
  {"id": "nomod", "match": {"title": ["No Mod"]}}
 ]
}"#;

    fn sample() -> Catalog {
        parse(SAMPLE).unwrap()
    }

    #[test]
    fn matches_by_id_then_exe_then_title() {
        let c = sample();
        let f = c.renodx_for(Some("1643320"), None, "whatever").unwrap();
        assert_eq!((f.id.as_str(), f.how, f.name.as_str()), ("stalker2", How::Steam, "S.T.A.L.K.E.R. 2: Heart of Chornobyl"));
        let f = c.renodx_for(Some("1"), Some(Path::new("/g/bin/stalker2-win64-shipping.exe")), "x").unwrap();
        assert_eq!(f.how, How::Exe);
        let f = c.renodx_for(None, None, "Assassin's Creed Brotherhood").unwrap();
        assert_eq!((f.id.as_str(), f.how, f.mod_.working()), ("acii", How::Title, false));
        assert_eq!(f.mod_.note.as_deref(), Some("Disable AA"));
        assert!(c.renodx_for(None, None, "No Mod").is_none());
        assert!(c.renodx_for(Some(""), None, "").is_none());
    }

    #[test]
    fn engines_get_their_generic_mod() {
        let dir = tempfile::tempdir().unwrap();
        let ue = dir.path().join("Game/Binaries/Win64/Game-Win64-Shipping.exe");
        let unity = dir.path().join("Other/Other.exe");
        let plain = dir.path().join("Plain/Plain.exe");
        for f in [&ue, &unity, &plain] {
            std::fs::create_dir_all(f.parent().unwrap()).unwrap();
            std::fs::write(f, b"MZ").unwrap();
        }
        std::fs::write(dir.path().join("Other/UnityPlayer.dll"), b"MZ").unwrap();
        assert_eq!(engine(&ue), Some(Engine::Unreal));
        assert_eq!(engine(&unity), Some(Engine::Unity));
        assert_eq!(engine(&plain), None);
        let c = sample();
        let f = c.renodx_for(None, Some(&ue), "Unknown Game").unwrap();
        assert_eq!((f.id.as_str(), f.how, f.name.as_str()), ("generic-unreal", How::Engine(Engine::Unreal), "Unreal Engine Extended"));
        // No generic Unity mod in the sample.
        assert!(c.renodx_for(None, Some(&unity), "Unknown Game").is_none());
    }

    #[test]
    fn store_titles_match_entries_with_qualifiers() {
        let c = built_in();
        let id = |t: &str| c.renodx_for(None, None, t).map(|f| f.id);
        for (title, entry) in [
            ("RESIDENT EVIL 2", "re7-2r-3r-village"),
            ("Resident Evil 4", "re4remake"),
            ("God of War", "gow2018"),
            ("Red Dead Redemption 2", "rdr2vk"),
            ("SILENT HILL 2", "silenthill2remake"),
            ("Borderlands 2", "borderlands2"),
            ("The Witcher 3: Wild Hunt", "thewitcher3"),
            ("Dead Island Definitive Edition", "deadislandde"),
            ("Atelier Ryza: Ever Darkness & the Secret Hideout", "ryza"),
            ("Atelier Ryza: Ever Darkness & the Secret Hideout DX", "atelierryzadx"),
        ] {
            assert_eq!(id(title).as_deref(), Some(entry), "{title}");
        }
        // An old year stays part of the title: the 2012 game is another one.
        assert_eq!(id("Need for Speed: Most Wanted"), None);
        assert_eq!(strip_qualifiers("Tomb Raider (2013) - Definitive Edition"), "Tomb Raider  - Definitive Edition");
        assert_eq!(strip_qualifiers("Need for Speed Most Wanted (2005)"), "Need for Speed Most Wanted (2005)");
    }

    #[test]
    fn a_newer_or_broken_catalog_is_ignored() {
        assert!(parse(r#"{"schema": 2, "games": []}"#).is_none());
        assert!(parse("not json").is_none());
        assert_eq!(parse(r#"{"schema": 1}"#).unwrap().games.len(), 0);
    }

    #[test]
    fn the_built_in_catalog_reads() {
        let c = built_in();
        assert!(c.schema == SCHEMA && !c.games.is_empty());
        let hosts_ok = |u: &Option<String>| u.as_deref().is_none_or(super::super::renodx::allowed_url);
        for e in &c.games {
            let r = e.renodx.as_ref().unwrap();
            assert!(r.addon64.is_some() || r.addon32.is_some(), "{}", e.id);
            assert!(hosts_ok(&r.addon64) && hosts_ok(&r.addon32), "{}", e.id);
        }
    }

    #[test]
    fn a_download_this_build_cannot_read_keeps_the_cached_copy() {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path());
        let net = FakeNet::online().with_file(URL, SAMPLE.as_bytes().to_vec());
        assert_eq!(load(&paths, &net, false).games.len(), 3);
        let newer = FakeNet::online().with_file(URL, br#"{"schema": 9, "games": []}"#.to_vec());
        assert_eq!(load(&paths, &newer, true).games.len(), 3);
        assert_eq!(load(&paths, &FakeNet::offline(), true).games.len(), 3);
        // A failed download waits a day: the copy counts as checked.
        let failed = DataPaths::at(dir.path().join("failed"));
        assert_eq!(load(&failed, &FakeNet::online(), false).games.len(), built_in().games.len());
        assert_eq!(load(&failed, &net, false).games.len(), built_in().games.len());
        let empty = DataPaths::at(dir.path().join("none"));
        assert_eq!(cached(&empty).games.len(), built_in().games.len());
    }
}
