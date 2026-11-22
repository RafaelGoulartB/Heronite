//! Actions users add to a game besides Play (`local_library/actions.json`):
//! a program, a command or something to open (a web page, a file, a folder).
//! Play itself never changes; the actions sit next to it.
//!
//! The file is `{ "games": { "<game key>": [ { action } ] }, "askOnPlay":
//! ["<game key>"] }`, keyed by `CollectionGame.key` like the ratings. The
//! games in `askOnPlay` ask on Play which way to start them: Play or one of
//! their actions that count as playing. Other fields in the file are kept on
//! write; an entry that does not read as an action is dropped on read.

use std::collections::{HashMap, HashSet};

use anyhow::Result;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::json_store;
use crate::models::Extra;
use crate::paths::DataPaths;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ActionKind {
    /// A program file: Linux programs run directly, Windows ones with the
    /// game's Wine and prefix.
    #[default]
    Program,
    /// A shell command run in the game's folder.
    Command,
    /// A web page, file or folder opened with the desktop's default app.
    Open,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Action {
    /// Stable within the game (menus and the palette refer to it).
    pub id: String,
    pub name: String,
    pub kind: ActionKind,
    /// The program's path, the command line, or the address / path to open.
    pub target: String,
    /// Arguments of a program.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub args: String,
    /// Runs like Play: with the game's settings (Wine, MangoHud, gamescope...),
    /// as the game's session (playtime, Stop). Programs and commands only.
    #[serde(default)]
    pub counts_as_playing: bool,
}

impl Action {
    /// Whether it starts as the game's session.
    pub fn plays(&self) -> bool {
        self.counts_as_playing && self.kind != ActionKind::Open
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ActionsFile {
    #[serde(default)]
    games: IndexMap<String, Vec<Value>>,
    /// Games whose Play asks how to play.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    ask_on_play: Vec<String>,
    #[serde(flatten)]
    extra: Extra,
}

/// Every game's actions by game key, in the order the user gave them.
pub fn load(paths: &DataPaths) -> HashMap<String, Vec<Action>> {
    let file: ActionsFile = json_store::read(&paths.game_actions()).unwrap_or_default();
    file.games
        .into_iter()
        .map(|(key, list)| {
            let actions: Vec<Action> = list
                .into_iter()
                .filter_map(|v| serde_json::from_value::<Action>(v).ok())
                .filter(|a| !a.id.is_empty() && !a.target.trim().is_empty())
                .collect();
            (key, actions)
        })
        .filter(|(_, list)| !list.is_empty())
        .collect()
}

/// The games whose Play asks how to play (keys).
pub fn load_ask_on_play(paths: &DataPaths) -> HashSet<String> {
    let file: ActionsFile = json_store::read(&paths.game_actions()).unwrap_or_default();
    file.ask_on_play.into_iter().collect()
}

/// Whether Play of the game under `key` asks how to play.
pub fn set_ask_on_play(paths: &DataPaths, key: &str, on: bool) -> Result<()> {
    json_store::update::<ActionsFile, _>(&paths.game_actions(), |file| {
        file.ask_on_play.retain(|k| k != key);
        if on {
            file.ask_on_play.push(key.to_owned());
        }
        Ok(())
    })
}

/// Replaces a game's actions (`[]` removes the game from the file).
pub fn save(paths: &DataPaths, key: &str, actions: &[Action]) -> Result<()> {
    json_store::update::<ActionsFile, _>(&paths.game_actions(), |file| {
        if actions.is_empty() {
            file.games.shift_remove(key);
        } else {
            let list = actions.iter().map(serde_json::to_value).collect::<Result<Vec<_>, _>>()?;
            file.games.insert(key.to_owned(), list);
        }
        Ok(())
    })
}

/// A new id, unique among `taken`.
pub fn new_id(taken: &[Action]) -> String {
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    let mut n = (nanos % 0xffff_ffff) as u64;
    loop {
        let id = format!("{n:08x}");
        if !taken.iter().any(|a| a.id == id) {
            return id;
        }
        n = n.wrapping_add(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths() -> (tempfile::TempDir, DataPaths) {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path());
        (dir, paths)
    }

    fn action(id: &str, kind: ActionKind, target: &str) -> Action {
        Action { id: id.into(), name: format!("A {id}"), kind, target: target.into(), ..Default::default() }
    }

    #[test]
    fn saves_and_reads_back() {
        let (_d, p) = paths();
        assert!(load(&p).is_empty());
        let list = vec![
            Action { args: "--safe".into(), counts_as_playing: true, ..action("1", ActionKind::Program, "/g/mods.exe") },
            action("2", ActionKind::Open, "https://wiki.example"),
        ];
        save(&p, "sideload_x", &list).unwrap();
        assert_eq!(load(&p)["sideload_x"], list);
        save(&p, "sideload_x", &[]).unwrap();
        assert!(load(&p).is_empty());
    }

    #[test]
    fn odd_entries_are_dropped_and_other_fields_kept() {
        let (_d, p) = paths();
        std::fs::create_dir_all(p.game_actions().parent().unwrap()).unwrap();
        std::fs::write(
            p.game_actions(),
            r#"{"games":{"g":[{"id":"1","name":"Ok","kind":"command","target":"./run.sh"},{"id":"2","kind":"nope","target":"x"},{"id":"","name":"x","kind":"open","target":"y"},"junk"]},"note":1}"#,
        )
        .unwrap();
        let read = load(&p);
        assert_eq!(read["g"], [action("1", ActionKind::Command, "./run.sh").with_name("Ok")]);
        save(&p, "h", &[action("9", ActionKind::Open, "/x")]).unwrap();
        let raw: Value = serde_json::from_str(&std::fs::read_to_string(p.game_actions()).unwrap()).unwrap();
        assert_eq!(raw["note"], 1);
    }

    #[test]
    fn asking_on_play_is_kept_per_game() {
        let (_d, p) = paths();
        assert!(load_ask_on_play(&p).is_empty());
        save(&p, "a", &[action("1", ActionKind::Program, "/g/recomp.exe")]).unwrap();
        set_ask_on_play(&p, "a", true).unwrap();
        set_ask_on_play(&p, "a", true).unwrap();
        set_ask_on_play(&p, "b", true).unwrap();
        assert_eq!(load_ask_on_play(&p), HashSet::from(["a".to_string(), "b".to_string()]));
        let raw: Value = serde_json::from_str(&std::fs::read_to_string(p.game_actions()).unwrap()).unwrap();
        assert_eq!(raw["askOnPlay"], serde_json::json!(["a", "b"]));
        set_ask_on_play(&p, "b", false).unwrap();
        assert_eq!(load_ask_on_play(&p), HashSet::from(["a".to_string()]));
    }

    #[test]
    fn links_never_play() {
        let open = Action { counts_as_playing: true, ..action("1", ActionKind::Open, "/x") };
        assert!(!open.plays());
        assert!(Action { counts_as_playing: true, ..action("1", ActionKind::Program, "/x") }.plays());
        let ids = [action("a", ActionKind::Open, "x")];
        assert_ne!(new_id(&ids), "a");
    }

    impl Action {
        fn with_name(mut self, name: &str) -> Self {
            self.name = name.into();
            self
        }
    }
}
