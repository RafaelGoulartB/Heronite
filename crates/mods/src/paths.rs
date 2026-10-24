//! Where the mods of each game live (see `docs/MODS.md`, "Storage").

use std::path::{Path, PathBuf};

use heronite_data::DataPaths;

/// The mods root: `<data>/mods` unless the user picked another folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModsRoot(PathBuf);

impl ModsRoot {
    pub fn at(path: impl Into<PathBuf>) -> Self {
        Self(path.into())
    }

    /// `<data>/mods`.
    pub fn default_for(paths: &DataPaths) -> Self {
        Self(paths.root.join("mods"))
    }

    pub fn path(&self) -> &Path {
        &self.0
    }

    /// The Nexus account file (`0600`).
    pub fn nexus_account(&self) -> PathBuf {
        self.0.join("nexus.json")
    }

    /// Cached lists of the sites (Nexus games, Thunderstore communities).
    pub fn cache_dir(&self) -> PathBuf {
        self.0.join("links-cache")
    }

    pub fn game(&self, game_key: &str) -> GameDirs {
        GameDirs(self.0.join(safe_name(game_key)))
    }
}

/// The folders of one game under the mods root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GameDirs(PathBuf);

impl GameDirs {
    pub fn root(&self) -> &Path {
        &self.0
    }
    pub fn list_file(&self) -> PathBuf {
        self.0.join("mods.json")
    }
    pub fn mods_dir(&self) -> PathBuf {
        self.0.join("mods")
    }
    pub fn mod_dir(&self, id: &str) -> PathBuf {
        self.mods_dir().join(safe_name(id))
    }
    pub fn downloads(&self) -> PathBuf {
        self.0.join("downloads")
    }
    pub fn overwrite(&self) -> PathBuf {
        self.0.join("overwrite")
    }
    pub fn work(&self) -> PathBuf {
        self.0.join(".work")
    }
    pub fn layers(&self) -> PathBuf {
        self.0.join(".layers")
    }
    /// Scratch space of an install (extracted archive), removed after it.
    pub fn staging(&self) -> PathBuf {
        self.0.join(".staging")
    }
}

/// A file name made of `name`: no `/`, no leading dot, no `..`.
pub fn safe_name(name: &str) -> String {
    let s: String = name.chars().map(|c| if c == '/' || c == '\\' || c == '\0' { '_' } else { c }).collect();
    let s = s.trim_start_matches('.').trim();
    if s.is_empty() {
        "_".to_string()
    } else {
        s.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_stay_inside_their_folder() {
        assert_eq!(safe_name("../etc"), "_etc");
        assert_eq!(safe_name("legendary_Quail"), "legendary_Quail");
        assert_eq!(safe_name(".."), "_");
        assert_eq!(safe_name(""), "_");
        let root = ModsRoot::at("/m");
        assert_eq!(root.game("a/b").mod_dir("x/../y"), PathBuf::from("/m/a_b/mods/x_.._y"));
    }
}
