//! How a Playnite game starts without Playnite: its first play action, a
//! program (`File`) or one of Playnite's emulators (`Emulator`), the way
//! Playnite's `GenericPlayController` starts them. A link (`URL`), a script,
//! an emulator profile started by a script, a program that is not an `.exe`
//! (or `.bat`, `.cmd`) and a game whose play action is an add-on's (none in
//! the library) have no start here: those games keep starting through
//! Playnite.
//!
//! Emulators (`emulators.db`) have custom profiles (a program and its
//! arguments) and built-in ones, whose program and arguments come from
//! Playnite's definition (`Emulation\Emulators\*\emulator.yaml` in its
//! program folder): the first file under the emulator's folder whose name
//! matches the profile's `StartupExecutable` (case-insensitive), with its
//! `StartupArguments` unless the profile overrides them.

use std::cell::{OnceCell, RefCell};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde_json::Value;

use super::{absolute, key, Install};
use crate::litedb::{Bson, Document};
use crate::transfer::{Rom, Start};

/// How deep under an emulator's folder its program is looked for.
const PROGRAM_DEPTH: usize = 5;

/// Playnite's variables of one game (`ExpandableVariables`), but the ROM's
/// (`{ImagePath}`, `{ImageName}`, `{ImageNameNoExt}`, `{ImageDir}`): the
/// launch fills those in.
#[derive(Debug, Clone, Default)]
pub(super) struct Vars(Vec<(&'static str, String)>);

impl Vars {
    /// The variables of the game `d` installed in `folder`; `platform` is
    /// the name of its first platform.
    pub(super) fn of(d: &Document, install: &Install, folder: Option<&Path>, platform: Option<&str>) -> Self {
        let text = |v: Option<&str>| v.unwrap_or_default().to_string();
        Vars(vec![
            ("InstallDir", folder.map(|f| f.to_string_lossy().into_owned()).unwrap_or_default()),
            ("InstallDirName", folder.and_then(Path::file_name).map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()),
            ("PlayniteDir", install.program.to_string_lossy().into_owned()),
            ("Name", text(d.str("Name").map(str::trim))),
            ("Platform", text(platform)),
            ("GameId", text(d.str("GameId"))),
            ("DatabaseId", d.guid("_id").unwrap_or_default()),
            ("PluginId", d.guid("PluginId").unwrap_or_default()),
            ("Version", text(d.str("Version"))),
        ])
    }

    fn with_emulator(&self, dir: &str) -> Self {
        let mut vars = self.clone();
        vars.0.push(("EmulatorDir", dir.to_string()));
        vars
    }

    pub(super) fn expand(&self, text: &str) -> String {
        let mut out = text.trim().to_string();
        for (name, value) in &self.0 {
            out = out.replace(&format!("{{{name}}}"), value);
        }
        out
    }
}

/// One of Playnite's emulators.
#[derive(Debug, Clone, Default)]
pub(super) struct Emulator {
    pub name: String,
    /// Its folder (`InstallDir`).
    pub dir: Option<PathBuf>,
    /// The definition of its built-in profiles (`BuiltInConfigId`).
    config: Option<String>,
    custom: Vec<Custom>,
    builtin: Vec<Builtin>,
}

#[derive(Debug, Clone)]
struct Custom {
    id: String,
    executable: String,
    arguments: String,
    /// Started by a script instead of its program.
    script: bool,
}

#[derive(Debug, Clone)]
struct Builtin {
    id: String,
    /// The definition's profile (`BuiltInProfileName`).
    profile: String,
    /// `Some`: these arguments instead of the definition's.
    arguments: Option<String>,
}

/// A nested document's id (LiteDB names every `Id` `_id`).
fn id_of(d: &Document) -> Option<String> {
    d.str("_id").or_else(|| d.str("Id")).map(str::to_string)
}

fn documents<'a>(d: &'a Document, field: &str) -> impl Iterator<Item = &'a Document> {
    let items = match d.get(field) {
        Some(Bson::Array(items)) => items.as_slice(),
        _ => &[],
    };
    items.iter().filter_map(|b| match b {
        Bson::Document(d) => Some(d),
        _ => None,
    })
}

impl Emulator {
    /// The emulator of a document of `emulators.db`; `program` is Playnite's folder.
    pub(super) fn read(d: &Document, program: &Path) -> Self {
        let text = |d: &Document, k: &str| d.str(k).unwrap_or_default().to_string();
        Emulator {
            name: text(d, "Name"),
            dir: d.str("InstallDir").and_then(|dir| absolute(&dir.trim().replace("{PlayniteDir}", &program.to_string_lossy()))),
            config: d.str("BuiltInConfigId").map(str::to_string),
            custom: documents(d, "CustomProfiles")
                .filter_map(|p| {
                    Some(Custom {
                        id: id_of(p)?,
                        executable: text(p, "Executable"),
                        arguments: text(p, "Arguments"),
                        script: p.str("StartupScript").is_some_and(|s| !s.trim().is_empty()),
                    })
                })
                .collect(),
            builtin: documents(d, "BuiltinProfiles")
                .filter_map(|p| {
                    Some(Builtin {
                        id: id_of(p)?,
                        profile: p.str("BuiltInProfileName")?.to_string(),
                        arguments: (p.bool("OverrideDefaultArgs") == Some(true)).then(|| text(p, "CustomArguments")),
                    })
                })
                .collect(),
        }
    }
}

/// A built-in profile of Playnite's definitions.
#[derive(Debug, Clone, PartialEq)]
struct Definition {
    /// `StartupExecutable`: a pattern of its program's file name.
    executable: String,
    arguments: String,
}

/// Finds the starts of one library's games, keeping what they share.
pub(super) struct Resolver<'a> {
    install: &'a Install,
    pub emulators: HashMap<String, Emulator>,
    /// Playnite's emulator definitions, read when a game needs one.
    definitions: OnceCell<Vec<Value>>,
    /// The program found under an emulator's folder for each pattern.
    programs: RefCell<HashMap<(PathBuf, String), Option<PathBuf>>>,
}

impl<'a> Resolver<'a> {
    pub(super) fn new(install: &'a Install, emulators: HashMap<String, Emulator>) -> Self {
        Resolver { install, emulators, definitions: OnceCell::new(), programs: RefCell::default() }
    }

    /// The built-in profile `profile` of the definition `config`, unless it
    /// is started by a script.
    fn definition(&self, config: &str, profile: &str) -> Option<Definition> {
        let definitions = self.definitions.get_or_init(|| read_definitions(&self.install.program.join("Emulation").join("Emulators")));
        let def = definitions.iter().find(|d| d.get("Id").and_then(Value::as_str) == Some(config))?;
        let p = def.get("Profiles")?.as_array()?.iter().find(|p| p.get("Name").and_then(Value::as_str) == Some(profile))?;
        if p.get("ScriptStartup").and_then(Value::as_bool) == Some(true) {
            return None;
        }
        let text = |k: &str| p.get(k).and_then(Value::as_str).map(str::to_string);
        Some(Definition { executable: text("StartupExecutable")?, arguments: text("StartupArguments").unwrap_or_default() })
    }

    fn program(&self, dir: &Path, pattern: &str) -> Option<PathBuf> {
        let key = (dir.to_path_buf(), pattern.to_string());
        if let Some(found) = self.programs.borrow().get(&key) {
            return found.clone();
        }
        let found = find_program(dir, pattern);
        self.programs.borrow_mut().insert(key, found.clone());
        found
    }

    /// How the game `d` starts without Playnite, and which of its actions
    /// that is (its index in `GameActions`).
    pub(super) fn start(&self, d: &Document, vars: &Vars) -> Option<(usize, Start)> {
        let (index, play) = documents(d, "GameActions").enumerate().find(|(_, a)| a.bool("IsPlayAction") == Some(true))?;
        let start = match play.str("Type")? {
            "File" => program_start(play, vars)?,
            "Emulator" => self.emulator_start(play, d, vars)?,
            _ => return None,
        };
        Some((index, start))
    }

    fn emulator_start(&self, play: &Document, d: &Document, vars: &Vars) -> Option<Start> {
        let emulator = self.emulators.get(&play.guid("EmulatorId")?)?;
        let profile = play.str("EmulatorProfileId")?;
        let dir = emulator.dir.as_deref();
        let vars = vars.with_emulator(&dir.map(|d| d.to_string_lossy().into_owned()).unwrap_or_default());
        let (program, defaults) = if let Some(p) = emulator.custom.iter().find(|p| p.id == profile) {
            if p.script {
                return None;
            }
            (absolute(&vars.expand(&p.executable))?, p.arguments.clone())
        } else {
            let p = emulator.builtin.iter().find(|p| p.id == profile)?;
            let def = self.definition(emulator.config.as_deref()?, &p.profile)?;
            (self.program(dir?, &def.executable)?, p.arguments.clone().unwrap_or(def.arguments))
        };
        let args = if play.bool("OverrideDefaultArgs") == Some(true) {
            play.str("Arguments").unwrap_or_default().to_string()
        } else {
            [defaults.as_str(), play.str("AdditionalArguments").unwrap_or_default()]
                .iter()
                .map(|a| a.trim())
                .filter(|a| !a.is_empty())
                .collect::<Vec<_>>()
                .join(" ")
        };
        // A ROM's relative path is relative to the emulator's program, where
        // Playnite starts it.
        let from = program.parent().unwrap_or(Path::new(""));
        let roms: Vec<Rom> = documents(d, "Roms")
            .filter_map(|r| {
                let path = vars.expand(r.str("Path")?);
                let path = absolute(&path).unwrap_or_else(|| from.join(path.trim()));
                Some(Rom {
                    name: r.str("Name").map(str::trim).filter(|n| !n.is_empty()).map(str::to_string),
                    path: path.to_string_lossy().into_owned(),
                })
            })
            .filter(|r| !r.path.trim().is_empty())
            .collect();
        if roms.is_empty() {
            return None;
        }
        Some(Start::Emulator {
            name: emulator.name.clone(),
            program: program.to_string_lossy().into_owned(),
            args: shlex_literal(&vars.expand(&args)),
            roms,
        })
    }
}

/// `text` with its backslashes kept by a shlex split (the launch splits an
/// emulator's arguments that way, and Windows paths are full of them).
pub(super) fn shlex_literal(text: &str) -> String {
    text.replace('\\', "\\\\")
}

/// A program, from its folder or the one the action names. A relative path
/// is relative to that folder (Playnite's `GetRawExecutablePath`).
fn program_start(play: &Document, vars: &Vars) -> Option<Start> {
    let working_dir = play.str("WorkingDir").map(|w| vars.expand(w)).and_then(|w| absolute(&w));
    let path = vars.expand(play.str("Path")?);
    let path = match absolute(&path) {
        Some(p) => p,
        None => working_dir.as_ref()?.join(path.trim()),
    };
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    if !matches!(ext.as_str(), "exe" | "bat" | "cmd") {
        return None;
    }
    let args = vars.expand(play.str("Arguments").unwrap_or_default());
    let working_dir = working_dir.filter(|w| path.parent().is_none_or(|p| key(p) != key(w))).map(|w| w.to_string_lossy().into_owned());
    Some(Start::Program { path: path.to_string_lossy().into_owned(), args, working_dir })
}

/// The other actions of a game, but the one it starts with (`play`): its
/// programs and links.
pub(super) fn other_actions(d: &Document, play: usize, vars: &Vars) -> Vec<heronite_data::game_actions::Action> {
    use heronite_data::game_actions::{Action, ActionKind};
    let actions = documents(d, "GameActions").enumerate().filter(|(i, _)| *i != play).filter_map(|(_, a)| {
        let name = a.str("Name").map(str::trim).filter(|n| !n.is_empty())?.to_string();
        let (kind, target, args) = match a.str("Type")? {
            "File" => match program_start(a, vars)? {
                Start::Program { path, args, .. } => (ActionKind::Program, path, args),
                Start::Emulator { .. } => return None,
            },
            "URL" => (ActionKind::Open, vars.expand(a.str("Path")?), String::new()),
            _ => return None,
        };
        let counts_as_playing = kind == ActionKind::Program && a.bool("IsPlayAction") == Some(true);
        Some(Action { id: String::new(), name, kind, target, args, counts_as_playing })
    });
    crate::transfer::with_ids(actions.filter(|a| !a.target.trim().is_empty()).collect())
}

/// Playnite's emulator definitions (`<dir>\<emulator>\emulator.yaml`); one
/// that cannot be read is left out.
fn read_definitions(dir: &Path) -> Vec<Value> {
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    entries
        .flatten()
        .filter_map(|e| {
            let file = e.path().join("emulator.yaml");
            let text = std::fs::read_to_string(&file).ok()?;
            serde_saphyr::from_str::<Value>(text.trim_start_matches('\u{feff}'))
                .inspect_err(|err| log::debug!("Reading {}: {err}", file.display()))
                .ok()
        })
        .collect()
}

/// The first file under `dir` (its own files first, then its folders' in
/// name order) whose name matches `pattern`, ignoring case.
fn find_program(dir: &Path, pattern: &str) -> Option<PathBuf> {
    let pattern = regex::RegexBuilder::new(pattern).case_insensitive(true).build().ok()?;
    let mut level = vec![dir.to_path_buf()];
    for _ in 0..PROGRAM_DEPTH {
        let mut next = Vec::new();
        for folder in &level {
            let Ok(entries) = std::fs::read_dir(folder) else { continue };
            let mut entries: Vec<_> = entries.flatten().collect();
            entries.sort_by_key(|e| e.file_name());
            for e in entries {
                let Ok(kind) = e.file_type() else { continue };
                if kind.is_dir() {
                    next.push(e.path());
                } else if pattern.is_match(&e.file_name().to_string_lossy()) {
                    return Some(e.path());
                }
            }
        }
        level = next;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backslashes_survive_a_shlex_split() {
        let args = shlex_literal(r#"-L "C:\RetroArch\cores\snes9x_libretro.dll" -c C:\cfg\a.cfg "{ImagePath}""#);
        assert_eq!(shlex::split(&args).unwrap(), ["-L", r"C:\RetroArch\cores\snes9x_libretro.dll", "-c", r"C:\cfg\a.cfg", "{ImagePath}"]);
    }

    #[test]
    fn the_rom_variables_are_left_for_the_launch() {
        let vars = Vars(vec![("InstallDir", "/games/ff7".into()), ("Name", "Final Fantasy VII".into())]).with_emulator("/emu/duck");
        assert_eq!(
            vars.expand(r#" -batch "{ImagePath}" -d "{InstallDir}" -t "{Name}" -e {EmulatorDir} {ImageNameNoExt} "#),
            r#"-batch "{ImagePath}" -d "/games/ff7" -t "Final Fantasy VII" -e /emu/duck {ImageNameNoExt}"#
        );
    }

    #[test]
    fn an_emulators_program_is_found_by_its_pattern() {
        let d = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(d.path().join("bin/x64")).unwrap();
        std::fs::write(d.path().join("readme.txt"), b"").unwrap();
        std::fs::write(d.path().join("bin/x64/DuckStation-Qt-x64-ReleaseLTCG.exe"), b"").unwrap();
        let found = find_program(d.path(), r"^duckstation-qt-x64-ReleaseLTCG\.exe$");
        assert_eq!(found, Some(d.path().join("bin/x64/DuckStation-Qt-x64-ReleaseLTCG.exe")));
        assert_eq!(find_program(d.path(), r"^retroarch\.exe$"), None);
        assert_eq!(find_program(d.path(), "(unclosed"), None);
    }
}
