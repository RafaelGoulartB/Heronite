//! shadPS4: when a game first runs, the emulator decrypts its trophy pack
//! (it needs the trophy key in `keys.json`) into
//! `<user>/trophy/<NP comm id>/` (`Xml/TROP.XML`, `Xml/TROP_01.XML`… one per
//! language, `Icons/TROP000.PNG`…), and keeps each console user's unlocks
//! in a copy of the list, `<home>/<user id>/trophy/<NP comm id>.xml`.
//! Versions before mid-2026 kept one list with the unlocks in
//! `<user>/game_data/<title id>/TrophyFiles/trophy00/`; both are read.
//!
//! The game names its comm id in `sce_sys/npbind.dat`; without it, the list
//! whose name is the game's is used.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};

use super::psxml::{self, TrophyConf};
use super::{entries, game_dirs, newest, plausible, read_capped, same_title, Env, Install, MAX_XML};
use crate::model::Source;
use crate::sources::Fetch;
use crate::target::TrophyTarget;

const FLATPAK_ID: &str = "net.shadps4.shadPS4";

/// The folders shadPS4 may keep its files in: a portable copy's `user`
/// folder, its Flatpak's, `~/.local/share/shadPS4`, `%APPDATA%\shadPS4`.
pub fn user_roots(env: &Env, installs: &[Install]) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    let mut push = |p: PathBuf| {
        if !out.contains(&p) && p.is_dir() {
            out.push(p);
        }
    };
    for i in installs {
        if let Some(dir) = &i.exe_dir {
            push(dir.join("user"));
        }
        if let Some(id) = &i.flatpak_id {
            push(env.flatpak(id, "data").join("shadPS4"));
        }
    }
    push(env.flatpak(FLATPAK_ID, "data").join("shadPS4"));
    for d in &env.data_homes {
        push(d.join("shadPS4"));
    }
    if let Some(a) = &env.appdata {
        push(a.join("shadPS4"));
    }
    out
}

/// The users' folders: `General.home_dir` of `config.json` when set, else
/// `<user>/home`.
fn home_of(user: &Path) -> PathBuf {
    let configured = read_capped(&user.join("config.json"), 1 << 20)
        .ok()
        .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
        .and_then(|v| v.get("General")?.get("home_dir")?.as_str().map(PathBuf::from))
        .filter(|p| p.is_absolute());
    configured.unwrap_or_else(|| user.join("home"))
}

/// The comm ids the game's `sce_sys/npbind.dat` names.
pub fn game_comm_ids(roms: &[PathBuf]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for dir in game_dirs(roms, 2) {
        if let Ok(b) = read_capped(&dir.join("sce_sys/npbind.dat"), 1 << 20) {
            for id in psxml::comm_ids(&b) {
                if !out.contains(&id) {
                    out.push(id);
                }
            }
        }
    }
    out
}

/// Where a game's trophies are.
#[derive(Debug, Clone, PartialEq)]
pub enum Found {
    /// `<user>/trophy/<id>` and the console user's copy with the unlocks
    /// (none before the first unlock).
    Pack { pack: PathBuf, save: Option<PathBuf> },
    /// The older layout: `…/TrophyFiles/trophy00`, unlocks in its `TROP.XML`.
    Old { dir: PathBuf },
}

impl Found {
    fn xml_dir(&self) -> PathBuf {
        match self {
            Found::Pack { pack, .. } => pack.join("Xml"),
            Found::Old { dir } => dir.join("Xml"),
        }
    }

    fn state_file(&self) -> Option<PathBuf> {
        match self {
            Found::Pack { save, .. } => save.clone(),
            Found::Old { dir } => Some(dir.join("Xml/TROP.XML")),
        }
    }

    fn icons(&self) -> PathBuf {
        match self {
            Found::Pack { pack, .. } => pack.join("Icons"),
            Found::Old { dir } => dir.join("Icons"),
        }
    }
}

/// Each console user's copy of a pack's list (`<home>/<id>/trophy/<comm id>.xml`).
fn saves_of(user: &Path, comm_id: &str) -> Vec<PathBuf> {
    entries(&home_of(user))
        .into_iter()
        .filter(|d| d.file_name().and_then(|n| n.to_str()).is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit())))
        .map(|d| d.join("trophy").join(format!("{comm_id}.xml")))
        .collect()
}

/// The older layout's lists: `<user>/game_data/*/TrophyFiles/trophy*`.
fn old_dirs(user: &Path) -> Vec<PathBuf> {
    entries(&user.join("game_data"))
        .into_iter()
        .flat_map(|game| entries(&game.join("TrophyFiles")))
        .filter(|d| d.join("Xml/TROP.XML").is_file())
        .collect()
}

fn conf_of(file: &Path) -> Option<TrophyConf> {
    read_capped(file, MAX_XML).ok().and_then(|b| psxml::parse(&b).ok())
}

fn locate(users: &[PathBuf], target: &TrophyTarget) -> Option<Found> {
    let ids = game_comm_ids(&target.roms);
    let mut found: Vec<Found> = Vec::new();
    for user in users {
        for id in &ids {
            let pack = user.join("trophy").join(id);
            if pack.join("Xml").is_dir() {
                found.push(Found::Pack { save: newest(saves_of(user, id)), pack });
            }
        }
    }
    if found.is_empty() && !target.title.trim().is_empty() {
        for user in users {
            for pack in entries(&user.join("trophy")) {
                let Some(id) = pack.file_name().and_then(|n| n.to_str()).filter(|n| psxml::is_comm_id(n)).map(str::to_string) else {
                    continue;
                };
                if conf_of(&pack.join("Xml/TROP.XML")).is_some_and(|c| same_title(&c.title, &target.title)) {
                    found.push(Found::Pack { save: newest(saves_of(user, &id)), pack });
                }
            }
        }
    }
    if found.is_empty() {
        for user in users {
            for dir in old_dirs(user) {
                let Some(conf) = conf_of(&dir.join("Xml/TROP.XML")) else { continue };
                if ids.contains(&conf.npcommid) || same_title(&conf.title, &target.title) {
                    found.push(Found::Old { dir });
                }
            }
        }
    }
    // The one played last.
    let last = |f: &Found| f.state_file().and_then(|s| super::modified(&s)).or_else(|| super::modified(&f.xml_dir().join("TROP.XML")));
    found.into_iter().max_by_key(last)
}

/// The list in the interface language, else the language-neutral one
/// (shadPS4's `GetTrophyXmlPath`; `TROP_XX.XML` by the console's language
/// number).
fn list_file(xml_dir: &Path, lang: &str) -> PathBuf {
    let numbers: &[&str] = match lang {
        "pt-BR" => &["17", "07"],
        "es" => &["20", "03"],
        "de" => &["04"],
        "fr" => &["02", "22"],
        _ => &["01", "18"],
    };
    numbers.iter().map(|n| xml_dir.join(format!("TROP_{n}.XML"))).find(|f| f.is_file()).unwrap_or_else(|| xml_dir.join("TROP.XML"))
}

/// shadPS4 writes Unix seconds; older builds wrote finer units. A value is
/// read in the unit that makes it a plausible date.
pub fn unlock_time(value: u64) -> Option<DateTime<Utc>> {
    let secs = |v: u64| i64::try_from(v).ok().and_then(|s| DateTime::from_timestamp(s, 0)).and_then(plausible);
    secs(value).or_else(|| secs(value / 1_000)).or_else(|| secs(value / 1_000_000)).or_else(|| secs(value / 1_000_000_000))
}

pub fn read(found: &Found, game_title: &str, lang: &str) -> Fetch {
    let failed = |e: String| Fetch::Failed(format!("{}: {e}", found.xml_dir().display()));
    let state = match found.state_file().filter(|f| f.is_file()) {
        Some(f) => match read_capped(&f, MAX_XML).and_then(|b| psxml::parse(&b)) {
            Ok(c) => Some(c),
            Err(e) => return failed(e),
        },
        None => None,
    };
    let list = list_file(&found.xml_dir(), lang);
    let conf = match read_capped(&list, MAX_XML).and_then(|b| psxml::parse(&b)) {
        Ok(c) if !c.trophies.is_empty() => c,
        // The user's copy is a whole list of its own.
        _ if state.as_ref().is_some_and(|s| !s.trophies.is_empty()) => state.clone().unwrap_or_default(),
        Ok(_) => return Fetch::None,
        Err(e) => return failed(e),
    };
    let unlocks: HashMap<u32, Option<u64>> =
        state.iter().flat_map(|s| s.trophies.iter()).filter(|t| t.unlocked).map(|t| (t.id, t.timestamp)).collect();
    let icons = found.icons();
    let set = psxml::build(
        Source::ShadPs4,
        &conf,
        game_title,
        |t| unlocks.get(&t.id).map_or((false, None), |ts| (true, ts.and_then(unlock_time))),
        |id| psxml::icon_in(&icons, id),
    );
    Fetch::Set(set)
}

pub fn fetch(env: &Env, installs: &[Install], target: &TrophyTarget, lang: &str) -> Fetch {
    match locate(&user_roots(env, installs), target) {
        Some(found) => read(&found, &target.title, lang),
        None => Fetch::None,
    }
}

pub fn watched_files(env: &Env, installs: &[Install], target: &TrophyTarget) -> Vec<PathBuf> {
    let users = user_roots(env, installs);
    let mut files: Vec<PathBuf> = match locate(&users, target) {
        Some(Found::Pack { pack, .. }) => {
            let id = pack.file_name().and_then(|n| n.to_str()).unwrap_or_default().to_string();
            users.iter().flat_map(|u| saves_of(u, &id)).collect()
        }
        Some(Found::Old { dir }) => vec![dir.join("Xml/TROP.XML")],
        None => Vec::new(),
    };
    if files.is_empty() {
        // Not played yet: the user's copy appears with the first session.
        let ids = game_comm_ids(&target.roms);
        files = users.iter().flat_map(|u| ids.iter().flat_map(move |id| saves_of(u, id))).collect();
    }
    files
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Grade;
    use crate::target::TrophyEmulator;

    const LIST: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<trophyconf version="1.1" platform="ps4">
<npcommid>NPWR10000_00</npcommid>
<title-name>Bloodborne</title-name>
<trophy id="000" hidden="no" ttype="P" pid="-1"><name>Bloodborne</name><detail>All trophies.</detail></trophy>
<trophy id="001" hidden="no" ttype="B" pid="000"><name>Yharnam</name><detail>Arrive.</detail></trophy>
</trophyconf>"#;

    fn with_unlock(ts: &str) -> String {
        LIST.replace(r#"ttype="B" pid="000">"#, &format!(r#"ttype="B" pid="000" unlockstate="true" timestamp="{ts}">"#))
    }

    fn game(home: &Path) -> PathBuf {
        let game = home.join("games/CUSA00900");
        std::fs::create_dir_all(game.join("sce_sys")).unwrap();
        std::fs::write(game.join("sce_sys/npbind.dat"), b"\x00\x01NPWR10000_00\x00").unwrap();
        let eboot = game.join("eboot.bin");
        std::fs::write(&eboot, b"x").unwrap();
        eboot
    }

    fn target(eboot: PathBuf, title: &str) -> TrophyTarget {
        TrophyTarget { emulator: TrophyEmulator::ShadPs4, roms: vec![eboot], title: title.into(), emulator_id: None }
    }

    #[test]
    fn reads_the_pack_and_the_users_copy_in_the_interface_language() {
        let d = tempfile::tempdir().unwrap();
        let env = Env { home: d.path().into(), data_homes: vec![d.path().join(".local/share")], ..Default::default() };
        let user = d.path().join(".local/share/shadPS4");
        let pack = user.join("trophy/NPWR10000_00");
        std::fs::create_dir_all(pack.join("Xml")).unwrap();
        std::fs::create_dir_all(pack.join("Icons")).unwrap();
        std::fs::write(pack.join("Xml/TROP.XML"), LIST).unwrap();
        std::fs::write(pack.join("Xml/TROP_17.XML"), LIST.replace("Arrive.", "Chegue.")).unwrap();
        std::fs::write(pack.join("Icons/TROP001.PNG"), b"png").unwrap();
        std::fs::create_dir_all(user.join("home/1000/trophy")).unwrap();
        std::fs::create_dir_all(user.join("home/1001/trophy")).unwrap();
        let save = user.join("home/1000/trophy/NPWR10000_00.xml");
        std::fs::write(&save, with_unlock("1700000000")).unwrap();
        let t = target(game(d.path()), "Bloodborne");
        let Fetch::Set(set) = fetch(&env, &[], &t, "pt-BR") else { panic!("expected a set") };
        assert_eq!((set.source, set.total(), set.unlocked()), (Source::ShadPs4, 2, 1));
        let a = &set.achievements[1];
        assert_eq!((a.description.as_str(), a.grade, a.unlocked_at.unwrap().timestamp()), ("Chegue.", Some(Grade::Bronze), 1_700_000_000));
        assert_eq!(std::path::Path::new(&a.icon), pack.join("Icons/TROP001.PNG"));
        let Fetch::Set(en) = fetch(&env, &[], &t, "en") else { panic!() };
        assert_eq!(en.achievements[1].description, "Arrive.");
        let mut watched = watched_files(&env, &[], &t);
        watched.sort();
        assert_eq!(watched, [save, user.join("home/1001/trophy/NPWR10000_00.xml")]);
    }

    #[test]
    fn reads_the_older_layout_by_name() {
        let d = tempfile::tempdir().unwrap();
        let env = Env { home: d.path().into(), data_homes: vec![d.path().join("data")], ..Default::default() };
        let dir = d.path().join("data/shadPS4/game_data/CUSA00900/TrophyFiles/trophy00");
        std::fs::create_dir_all(dir.join("Xml")).unwrap();
        std::fs::write(dir.join("Xml/TROP.XML"), with_unlock("1700000000000000000")).unwrap();
        let t = target(d.path().join("moved/eboot.bin"), "BLOODBORNE™");
        let Fetch::Set(set) = fetch(&env, &[], &t, "en") else { panic!("expected a set") };
        assert_eq!(set.unlocked(), 1);
        assert_eq!(set.achievements[1].unlocked_at.unwrap().timestamp(), 1_700_000_000, "nanoseconds");
        assert_eq!(fetch(&env, &[], &target(d.path().join("x/eboot.bin"), "Other"), "en"), Fetch::None);
    }

    #[test]
    fn a_damaged_copy_fails_and_no_pack_is_none() {
        let d = tempfile::tempdir().unwrap();
        let env = Env { home: d.path().into(), data_homes: vec![d.path().join("data")], ..Default::default() };
        let t = target(game(d.path()), "Bloodborne");
        assert_eq!(fetch(&env, &[], &t, "en"), Fetch::None);
        let user = d.path().join("data/shadPS4");
        std::fs::create_dir_all(user.join("trophy/NPWR10000_00/Xml")).unwrap();
        std::fs::write(user.join("trophy/NPWR10000_00/Xml/TROP.XML"), LIST).unwrap();
        std::fs::create_dir_all(user.join("home/1000/trophy")).unwrap();
        std::fs::write(user.join("home/1000/trophy/NPWR10000_00.xml"), "<trophyconf><trophy id=").unwrap();
        assert!(matches!(fetch(&env, &[], &t, "en"), Fetch::Failed(_)));
        std::fs::remove_file(user.join("home/1000/trophy/NPWR10000_00.xml")).unwrap();
        let Fetch::Set(set) = fetch(&env, &[], &t, "en") else { panic!("expected a set") };
        assert_eq!(set.unlocked(), 0, "nothing unlocked before the user's copy exists");
    }

    #[test]
    fn unlock_times_in_any_unit() {
        for v in [1_700_000_000u64, 1_700_000_000_000, 1_700_000_000_000_000, 1_700_000_000_000_000_000] {
            assert_eq!(unlock_time(v).unwrap().timestamp(), 1_700_000_000, "{v}");
        }
        assert_eq!(unlock_time(5), None);
        assert_eq!(unlock_time(u64::MAX), None);
    }

    #[test]
    fn a_configured_home_folder_is_used() {
        let d = tempfile::tempdir().unwrap();
        let elsewhere = d.path().join("homes");
        let config = serde_json::json!({"General": {"home_dir": elsewhere}});
        std::fs::write(d.path().join("config.json"), config.to_string()).unwrap();
        assert_eq!(home_of(d.path()), elsewhere);
        std::fs::write(d.path().join("config.json"), r#"{"General":{"home_dir":""}}"#).unwrap();
        assert_eq!(home_of(d.path()), d.path().join("home"));
    }
}
