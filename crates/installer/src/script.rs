//! Reading a script file. A file holds one script, a list of them, or an
//! API dump (`{ "results": [ .. ] }`). A script is its identity (`name`,
//! `game_slug`, `version`, `runner`...) plus a `script` section:
//!
//! * `files`: what to fetch, by id ([`crate::files`]);
//! * `installer`: the steps, in order ([`crate::steps`]);
//! * `game`: how the installed game starts (`exe`, `args`, `prefix`,
//!   `working_dir`, `launch_configs`, `main_file`...);
//! * `system` (`env`...) and a section named after the runner (`wine:`
//!   with `dxvk`, `overrides`...): settings for the game;
//! * `variables`, `require-binaries`.
//!
//! A file with only the `script` section's keys at its root is read too.

use std::path::{Path, PathBuf};

use heronite_i18n::tr;
use serde_json::{Map, Value};

use crate::files::{self, FileSpec};
use crate::steps::{self, Step};
use crate::text::{get_nonempty, get_str, scalar};

/// Keys that name a game's entry point, in priority order. Old scripts put
/// them at the root; they belong in `game`.
pub const ENTRY_POINT_KEYS: [&str; 6] = ["exe", "main_file", "iso", "rom", "disk-a", "path"];

#[derive(Debug, Clone, Default)]
pub struct Script {
    pub name: String,
    pub game_slug: String,
    /// The script's own slug (`<game>-<variant>`).
    pub slug: String,
    /// The variant the script installs ("Windows", "GOG", "Setup"...).
    pub version: String,
    pub runner: String,
    pub year: Option<String>,
    pub description: String,
    pub notes: String,
    pub files: Vec<FileSpec>,
    pub steps: Vec<Step>,
    pub game: Map<String, Value>,
    pub system: Map<String, Value>,
    /// The section named after the runner (`wine:`).
    pub runner_config: Map<String, Value>,
    pub variables: Vec<(String, String)>,
    /// `require-binaries`: each entry is satisfied by any of its names
    /// (`"7z | 7za, unzip"` -> `[["7z", "7za"], ["unzip"]]`).
    pub require_binaries: Vec<Vec<String>>,
    /// `requires` / `extends`: the script adds to another installed game
    /// (a mod, a patch). Not supported, reported by [`crate::check`].
    pub requires: Option<String>,
    pub extends: Option<String>,
    /// Folder of the script file (`$SCRIPTDIR`).
    pub script_dir: Option<PathBuf>,
}

impl Script {
    /// The title the game gets in the Collection.
    pub fn title(&self) -> &str {
        if self.name.trim().is_empty() {
            &self.game_slug
        } else {
            self.name.trim()
        }
    }

    /// A folder name for the game: the game's slug, else one made from the
    /// title.
    pub fn folder_name(&self) -> String {
        let base = if self.game_slug.trim().is_empty() { slugify(self.title()) } else { self.game_slug.trim().to_string() };
        let clean: String = base.chars().filter(|c| *c != '/' && *c != '\0').collect();
        if clean.is_empty() || clean == "." || clean == ".." {
            "game".into()
        } else {
            clean
        }
    }

    /// The script's own variables (and `$VERSION`), for showing its texts
    /// before it runs (`SPFL${VERSION}_setup.exe` -> `SPFL26_setup.exe`).
    pub fn preview(&self, text: &str) -> String {
        let mut v = crate::vars::Vars::new();
        v.set("VERSION", self.version.clone());
        for (k, val) in &self.variables {
            v.set(k.clone(), val.clone());
        }
        v.substitute(text)
    }

    /// The label shown next to the title ("Windows · wine").
    pub fn variant(&self) -> String {
        let v = self.version.trim();
        if v.is_empty() {
            self.runner.clone()
        } else {
            format!("{v} \u{b7} {}", self.runner)
        }
    }
}

/// Lowercase ASCII letters and digits joined by `-`.
pub fn slugify(text: &str) -> String {
    let mut out = String::new();
    for c in text.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
    }
    out.trim_end_matches('-').to_string()
}

/// Reads the scripts of the file at `path`.
pub fn read_file(path: &Path) -> Result<Vec<Script>, String> {
    let bytes = std::fs::read(path).map_err(|e| tr!("Could not read {path}: {error}", path = path.display(), error = e))?;
    let text = String::from_utf8_lossy(&bytes);
    let fallback = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    parse(&text, path.parent(), &fallback)
}

/// Parses the text of a script file. `script_dir` is its folder,
/// `fallback_name` names a script that has no `name`.
pub fn parse(text: &str, script_dir: Option<&Path>, fallback_name: &str) -> Result<Vec<Script>, String> {
    let doc = load_yaml(text)?;
    let docs: Vec<Value> = match doc {
        Value::Array(list) => list,
        Value::Object(mut m) if m.get("results").is_some_and(Value::is_array) => match m.remove("results") {
            Some(Value::Array(list)) => list,
            _ => Vec::new(),
        },
        Value::Object(m) => vec![Value::Object(m)],
        _ => return Err(tr!("This file is not an install script").into()),
    };
    let mut out = Vec::new();
    for d in docs {
        let Value::Object(m) = d else { continue };
        out.push(from_map(m, script_dir, fallback_name)?);
    }
    if out.is_empty() {
        return Err(tr!("This file has no install script").into());
    }
    Ok(out)
}

/// YAML 1.2 (JSON included). A key written twice keeps the last value, as
/// the scripts' authors saw it.
fn load_yaml(text: &str) -> Result<Value, String> {
    let options = serde_saphyr::options! { duplicate_keys: serde_saphyr::DuplicateKeyPolicy::LastWins };
    serde_saphyr::from_str_with_options::<Value>(text, options)
        .map_err(|e| tr!("The script could not be read: {error}", error = first_line(&e.to_string())))
}

fn first_line(s: &str) -> String {
    s.lines().find(|l| !l.trim().is_empty()).unwrap_or(s).trim().to_string()
}

fn from_map(mut root: Map<String, Value>, script_dir: Option<&Path>, fallback_name: &str) -> Result<Script, String> {
    let body = match root.remove("script") {
        Some(Value::Object(m)) => m,
        // A JSON dump may carry the script as text.
        Some(Value::String(s)) => match load_yaml(&s)? {
            Value::Object(m) => m,
            _ => return Err(tr!("The script section is not a map").into()),
        },
        Some(_) => return Err(tr!("The script section is not a map").into()),
        None if root.contains_key("installer") || root.contains_key("game") || root.contains_key("files") => std::mem::take(&mut root),
        None => return Err(tr!("This file has no script section").into()),
    };
    let ident = |k: &str| get_nonempty(&root, k).or_else(|| get_nonempty(&body, k));
    let runner = ident("runner").unwrap_or_default().trim().to_ascii_lowercase();
    if runner.is_empty() {
        return Err(tr!("The script does not say which runner it uses").into());
    }
    let name = ident("name").or_else(|| ident("custom-name")).unwrap_or_else(|| fallback_name.to_string());
    let game_slug = ident("game_slug").unwrap_or_else(|| slugify(&name));
    let mut game = match body.get("game") {
        Some(Value::Object(m)) => m.clone(),
        _ => Map::new(),
    };
    for key in ENTRY_POINT_KEYS {
        if !game.contains_key(key) {
            if let Some(v) = body.get(key).filter(|v| v.is_string()) {
                game.insert(key.to_string(), v.clone());
            }
        }
    }
    let steps = match body.get("installer") {
        Some(Value::Array(items)) => items.iter().map(|i| steps::parse(i, &runner)).collect(),
        _ => Vec::new(),
    };
    let files = match body.get("files") {
        Some(Value::Array(items)) => items.iter().flat_map(files::parse_entry).collect(),
        _ => Vec::new(),
    };
    let section = |k: &str| match body.get(k) {
        Some(Value::Object(m)) => m.clone(),
        _ => Map::new(),
    };
    let variables = match body.get("variables") {
        Some(Value::Object(m)) => m.iter().filter_map(|(k, v)| scalar(v).map(|s| (k.clone(), s))).collect(),
        _ => Vec::new(),
    };
    let require_binaries = get_str(&body, "require-binaries").map(|s| parse_binaries(&s)).unwrap_or_default();
    Ok(Script {
        name: name.trim().to_string(),
        game_slug: game_slug.trim().to_string(),
        slug: ident("slug").unwrap_or_default(),
        version: ident("version").unwrap_or_default(),
        year: ident("year"),
        description: ident("description").unwrap_or_default().trim().to_string(),
        notes: ident("notes").unwrap_or_default().trim().to_string(),
        files,
        steps,
        system: section("system"),
        runner_config: section(&runner),
        runner,
        game,
        variables,
        require_binaries,
        requires: get_nonempty(&body, "requires").or_else(|| get_nonempty(&root, "requires")),
        extends: get_nonempty(&body, "extends"),
        script_dir: script_dir.map(Path::to_path_buf),
    })
}

/// `"a | b, c"` -> `[["a", "b"], ["c"]]`.
fn parse_binaries(s: &str) -> Vec<Vec<String>> {
    s.split(',')
        .map(|group| group.split('|').map(|b| b.trim().to_string()).filter(|b| !b.is_empty()).collect::<Vec<_>>())
        .filter(|g| !g.is_empty())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::steps::Action;

    const SCRIPT: &str = r#"
game_slug: some-game
name: "Some Game"
year: 2025
runner: wine
slug: some-game-win
version: Windows
description: Installer for Some Game.
script:
  variables:
    VERSION: 26
  require-binaries: 7z | 7za, cabextract
  files:
  - setup: N/A:Select the setup file
  - patch:
      url: https://example.com/patch.zip
      filename: patch.zip
  game:
    exe: $GAMEDIR/drive_c/Game/game.exe
    prefix: $GAMEDIR
    prefix: $GAMEDIR
  installer:
  - task:
      name: create_prefix
      prefix: $GAMEDIR
  - task:
      description: Running the setup...
      name: wineexec
      executable: setup
  - extract:
      file: patch
      dst: $GAMEDIR/drive_c/Game
  wine:
    dxvk: true
    overrides:
      ddraw: n,b
"#;

    #[test]
    fn reads_a_full_script() {
        let s = parse(SCRIPT, Some(Path::new("/scripts")), "x").unwrap().remove(0);
        assert_eq!(s.name, "Some Game");
        assert_eq!(s.game_slug, "some-game");
        assert_eq!(s.runner, "wine");
        assert_eq!(s.version, "Windows");
        assert_eq!(s.year.as_deref(), Some("2025"));
        assert_eq!(s.variables, [("VERSION".to_string(), "26".to_string())]);
        assert_eq!(s.require_binaries, vec![vec!["7z".to_string(), "7za".into()], vec!["cabextract".to_string()]]);
        assert_eq!(s.files.len(), 2);
        assert_eq!(s.steps.len(), 3);
        assert_eq!(s.steps[1].description.as_deref(), Some("Running the setup..."));
        assert!(matches!(s.steps[2].action, Action::Extract(_)));
        assert_eq!(s.game.get("prefix").and_then(Value::as_str), Some("$GAMEDIR"), "a repeated key keeps the last value");
        assert_eq!(s.runner_config.get("dxvk"), Some(&Value::Bool(true)));
        assert_eq!(s.script_dir.as_deref(), Some(Path::new("/scripts")));
        assert_eq!(s.variant(), "Windows \u{b7} wine");
    }

    #[test]
    fn lists_dumps_and_bare_scripts() {
        let list = format!("- {}", SCRIPT.trim().replace('\n', "\n  "));
        assert_eq!(parse(&list, None, "x").unwrap().len(), 1);
        let dump = r#"{"count": 1, "results": [{"name": "A", "runner": "linux", "game_slug": "a", "script": {"game": {"exe": "a.sh"}}}]}"#;
        let s = parse(dump, None, "x").unwrap().remove(0);
        assert_eq!((s.name.as_str(), s.runner.as_str()), ("A", "linux"));
        let bare = "runner: linux\nexe: start.sh\ninstaller:\n- chmodx: $GAMEDIR/start.sh\n";
        let s = parse(bare, None, "my-file").unwrap().remove(0);
        assert_eq!(s.name, "my-file");
        assert_eq!(s.game_slug, "my-file");
        assert_eq!(s.game.get("exe").and_then(Value::as_str), Some("start.sh"), "root entry points move into game");
        assert_eq!(s.steps.len(), 1);
    }

    #[test]
    fn bad_files_explain_themselves() {
        assert!(parse("just text", None, "x").is_err());
        assert!(parse("name: A\nscript:\n  files: []\n", None, "x").unwrap_err().contains("runner"));
        assert!(parse("a: [1, 2\n", None, "x").is_err());
    }

    #[test]
    fn folder_names_are_safe() {
        let s = Script { name: "Half-Life: Source".into(), ..Default::default() };
        assert_eq!(s.folder_name(), "half-life-source");
        let s = Script { game_slug: "../..".into(), ..Default::default() };
        assert_eq!(s.folder_name(), "....");
        let s = Script { game_slug: "..".into(), name: "x".into(), ..Default::default() };
        assert_eq!(s.folder_name(), "game");
    }
}
