//! Bottles: each bottle is a Wine prefix, a folder of `bottles/` in its data
//! folder (or wherever `data.yml`'s `custom_bottles_path` says) with its
//! settings in `bottle.yml`; a bottle made elsewhere leaves a
//! `placeholder.yml` there pointing at it. The programs the user added to a
//! bottle are its `External_Programs` (the hidden ones say `removed`), and
//! `library.yml` gives the ones in Bottles' Library a cover
//! (`grid:<file>`, in the bottle's `grids/`). A program starts with
//! `bottles-cli run -b <bottle> -p <program>`; Bottles keeps no play time.
//!
//! Inside the Flatpak, Bottles' data folder is `/var/data`: paths it wrote
//! there are read from `~/.var/app/com.usebottles.bottles/data`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use heronite_data::launchers::Launcher;
use serde_json::Value;

use crate::{Found, Game};

const FLATPAK_ID: &str = "com.usebottles.bottles";

/// Where one Bottles install keeps its files.
struct Install {
    /// `.../bottles`: `data.yml`, `library.yml` and `bottles/`.
    root: PathBuf,
    flatpak: bool,
    /// What `/var/data` is outside the Flatpak.
    var_data: Option<PathBuf>,
}

fn installs(home: &Path) -> [Install; 2] {
    let flatpak = home.join(".var/app").join(FLATPAK_ID).join("data");
    [
        Install { root: crate::xdg(home, "XDG_DATA_HOME", ".local/share").join("bottles"), flatpak: false, var_data: None },
        Install { root: flatpak.join("bottles"), flatpak: true, var_data: Some(flatpak) },
    ]
}

pub fn detect(home: &Path) -> Result<Option<Found>> {
    let Some(install) = installs(home).into_iter().find(|i| i.root.join("bottles").is_dir()) else { return Ok(None) };
    let games = read(&install)?;
    Ok(Some(Found { launcher: Launcher::Bottles, location: install.root, flatpak: install.flatpak, games, heroic: None }))
}

fn load_yaml(file: &Path) -> Result<Value> {
    let text = std::fs::read_to_string(file).with_context(|| format!("reading {}", file.display()))?;
    let options = serde_saphyr::options! { duplicate_keys: serde_saphyr::DuplicateKeyPolicy::LastWins };
    serde_saphyr::from_str_with_options::<Value>(&text, options).with_context(|| format!("{} is not YAML", file.display()))
}

fn text<'a>(v: &'a Value, key: &str) -> &'a str {
    v.get(key).and_then(Value::as_str).map(str::trim).unwrap_or_default()
}

impl Install {
    /// A path Bottles wrote, as seen from outside its Flatpak.
    fn host(&self, path: &str) -> PathBuf {
        match (&self.var_data, path.strip_prefix("/var/data")) {
            (Some(outside), Some(rest)) if rest.is_empty() || rest.starts_with('/') => outside.join(rest.trim_start_matches('/')),
            _ => PathBuf::from(path),
        }
    }

    /// `bottles/`, or the folder chosen in Bottles' preferences.
    fn bottles_dir(&self) -> PathBuf {
        let custom = load_yaml(&self.root.join("data.yml"))
            .ok()
            .map(|d| text(&d, "custom_bottles_path").to_string())
            .filter(|p| !p.is_empty())
            .map(|p| self.host(&p))
            .filter(|p| p.is_dir());
        custom.unwrap_or_else(|| self.root.join("bottles"))
    }

    fn command(&self, bottle: &str, program: &str) -> String {
        let run = format!("run -b {} -p {}", quote(bottle), quote(program));
        if self.flatpak {
            format!("flatpak run --command=bottles-cli {FLATPAK_ID} {run}")
        } else {
            format!("bottles-cli {run}")
        }
    }
}

/// One shell word (the command is split with shell rules).
fn quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

/// The bottle in `folder`: its settings and the folder it really is.
fn bottle(install: &Install, folder: &Path) -> Option<(Value, PathBuf)> {
    let placeholder = folder.join("placeholder.yml");
    let dir = match load_yaml(&placeholder) {
        Ok(p) if !text(&p, "Path").is_empty() => install.host(text(&p, "Path")),
        _ => folder.to_path_buf(),
    };
    let config = load_yaml(&dir.join("bottle.yml")).map_err(|e| log::warn!("Bottles: {e:#}")).ok()?;
    let custom = config.get("Custom_Path").and_then(Value::as_bool).unwrap_or(false);
    let path = text(&config, "Path");
    let dir = if custom && !path.is_empty() { install.host(path) } else { dir };
    Some((config, dir))
}

/// The covers of Bottles' Library, by program id.
fn covers(install: &Install, bottles: &HashMap<String, PathBuf>) -> HashMap<String, String> {
    let Ok(library) = load_yaml(&install.root.join("library.yml")) else { return HashMap::new() };
    let mut out = HashMap::new();
    for entry in library.as_object().into_iter().flat_map(|m| m.values()) {
        let (id, thumbnail) = (text(entry, "id"), text(entry, "thumbnail"));
        let Some(file) = thumbnail.strip_prefix("grid:") else { continue };
        let bottle = entry.get("bottle").map(|b| text(b, "name")).unwrap_or_default();
        if let Some(dir) = bottles.get(bottle) {
            let cover = dir.join("grids").join(file);
            if !id.is_empty() && cover.is_file() {
                out.insert(id.to_string(), cover.to_string_lossy().into_owned());
            }
        }
    }
    out
}

/// The programs of every bottle, but hidden ones and the bottles of Steam's
/// Proton (the Steam import brings those).
fn read(install: &Install) -> Result<Vec<Game>> {
    let dir = install.bottles_dir();
    let mut found = Vec::new();
    let mut folders: Vec<PathBuf> = std::fs::read_dir(&dir)
        .with_context(|| format!("reading {}", dir.display()))?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    folders.sort();
    for folder in folders {
        let Some((config, path)) = bottle(install, &folder) else { continue };
        let name = text(&config, "Name").to_string();
        if name.is_empty() || text(&config, "Environment").eq_ignore_ascii_case("steam") {
            continue;
        }
        found.push((name, path, config));
    }
    let paths: HashMap<String, PathBuf> = found.iter().map(|(n, p, _)| (n.clone(), p.clone())).collect();
    let covers = covers(install, &paths);

    let mut games = Vec::new();
    for (bottle, prefix, config) in &found {
        let Some(programs) = config.get("External_Programs").and_then(Value::as_object) else { continue };
        let mut list: Vec<&Value> = programs.values().filter(|p| !p.get("removed").and_then(Value::as_bool).unwrap_or(false)).collect();
        list.sort_by_key(|p| text(p, "name").to_lowercase());
        for p in list {
            let title = text(p, "name");
            if title.is_empty() {
                continue;
            }
            let id = match text(p, "id") {
                "" => format!("{bottle}-{title}"),
                id => id.to_string(),
            };
            let exe = Some(text(p, "path")).filter(|e| !e.is_empty()).map(|e| install.host(e));
            let folder = Some(text(p, "folder"))
                .filter(|f| !f.is_empty())
                .map(|f| install.host(f))
                .or_else(|| exe.as_deref().and_then(Path::parent).map(Path::to_path_buf))
                .filter(|f| f.components().count() > 2)
                .map(|f| f.to_string_lossy().into_owned());
            games.push(Game {
                cover: covers.get(&id).cloned(),
                id,
                title: title.to_string(),
                folder,
                windows: true,
                command: install.command(bottle, title),
                prefix: Some(prefix.to_string_lossy().into_owned()),
                ..Default::default()
            });
        }
    }
    Ok(games)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn write(path: &Path, text: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    #[test]
    fn reads_the_programs_of_every_bottle() {
        let h = tempfile::tempdir().unwrap();
        let home = h.path();
        let root = home.join(".var/app/com.usebottles.bottles/data/bottles");
        write(
            &root.join("bottles/Games/bottle.yml"),
            "Name: Games\nPath: Games\nCustom_Path: false\nEnvironment: Gaming\nRunner: soda-9.0-1\n\
             External_Programs:\n\
             \x20 a1:\n    executable: HD_Launcher.exe\n    name: HoMM3 HD\n    path: /var/data/bottles/bottles/Games/drive_c/HoMM3/HD_Launcher.exe\n    id: a1\n    folder: /var/data/bottles/bottles/Games/drive_c/HoMM3\n\
             \x20 b2:\n    executable: setup.exe\n    name: Old setup\n    path: /x/setup.exe\n    id: b2\n    removed: true\n\
             \x20 c3:\n    executable: Diablo.exe\n    name: Diablo's Hellfire\n    path: /var/data/bottles/bottles/Games/drive_c/Diablo/Diablo.exe\n    id: c3\n",
        );
        fs::create_dir_all(root.join("bottles/Games/grids")).unwrap();
        fs::write(root.join("bottles/Games/grids/cover-1.png"), b"png").unwrap();
        write(
            &root.join("library.yml"),
            "u1:\n  bottle:\n    name: Games\n    path: Games\n  id: a1\n  name: HoMM3 HD\n  thumbnail: 'grid:cover-1.png'\n",
        );
        // Proton's prefixes of Steam games: left to the Steam import.
        write(
            &root.join("bottles/Steam/bottle.yml"),
            "Name: Steam\nEnvironment: Steam\nExternal_Programs:\n  s:\n    name: X\n    id: s\n",
        );
        // A bottle made elsewhere.
        let elsewhere = home.join("Bottles/Office");
        write(&root.join("bottles/Office/placeholder.yml"), &format!("Path: {}\n", elsewhere.display()));
        write(
            &elsewhere.join("bottle.yml"),
            &format!(
                "Name: Office\nPath: {}\nCustom_Path: true\nExternal_Programs:\n  d4:\n    name: Notepad++\n    path: {}/drive_c/npp/notepad++.exe\n    id: d4\n",
                elsewhere.display(),
                elsewhere.display()
            ),
        );

        let found = detect(home).unwrap().unwrap();
        assert!(found.flatpak);
        let games = &found.games;
        let names: Vec<_> = games.iter().map(|g| g.title.as_str()).collect();
        assert_eq!(names, ["Diablo's Hellfire", "HoMM3 HD", "Notepad++"]);
        let homm3 = &games[1];
        let bottle = root.join("bottles/Games");
        assert_eq!(homm3.id, "a1");
        assert_eq!(homm3.prefix.as_deref(), Some(bottle.to_str().unwrap()));
        assert_eq!(homm3.folder.as_deref(), Some(bottle.join("drive_c/HoMM3").to_str().unwrap()));
        assert!(homm3.cover.as_deref().is_some_and(|c| c.ends_with("grids/cover-1.png")));
        assert_eq!(homm3.command, "flatpak run --command=bottles-cli com.usebottles.bottles run -b 'Games' -p 'HoMM3 HD'");
        assert_eq!(games[0].command.split_once("-p ").unwrap().1, r"'Diablo'\''s Hellfire'");
        assert_eq!(games[2].prefix.as_deref(), Some(elsewhere.to_str().unwrap()));
        assert!(games.iter().all(|g| g.windows && g.minutes == 0));
    }

    #[test]
    fn a_custom_bottles_folder_and_the_native_command() {
        let h = tempfile::tempdir().unwrap();
        let home = h.path();
        let root = home.join(".local/share/bottles");
        let custom = home.join("Games/Bottles");
        fs::create_dir_all(root.join("bottles")).unwrap();
        write(&root.join("data.yml"), &format!("custom_bottles_path: {}\n", custom.display()));
        write(
            &custom.join("Hades/bottle.yml"),
            "Name: Hades\nPath: Hades\nExternal_Programs:\n  h:\n    name: Hades\n    path: /g/Hades.exe\n    id: h\n",
        );
        let found = detect(home).unwrap().unwrap();
        assert!(!found.flatpak);
        assert_eq!(found.games.len(), 1);
        assert_eq!(found.games[0].command, "bottles-cli run -b 'Hades' -p 'Hades'");
        assert_eq!(found.games[0].prefix.as_deref(), Some(custom.join("Hades").to_str().unwrap()));
    }

    #[test]
    fn nothing_without_bottles() {
        let h = tempfile::tempdir().unwrap();
        assert!(detect(h.path()).unwrap().is_none());
    }
}
