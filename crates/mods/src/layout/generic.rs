//! Games without a plugin list: the game folder, a mods folder, or the
//! folders of a known tree (Cyberpunk's `archive`, Unreal's `Content/Paks`).

use crate::fsutil::{extension, same_name};
use crate::model::CopyRule;

use super::{is_loader_file, join, Ctx, Entry, Placer};

/// Unreal's packaged content; a `~mods` folder of `Paks` is loaded last.
const PAK_EXTENSIONS: &[&str] = &["pak", "ucas", "utoc", "sig"];

impl Ctx<'_> {
    /// Each top entry is placed by itself: what the game has at its root or
    /// the game's tree stays, loose game files go where the game reads them,
    /// the rest goes to the mod folder (`Mods/<mod>`).
    pub(super) fn generic(&self, top: &[Entry], place: &Placer) -> Vec<CopyRule> {
        let is_pak = |(n, d): &Entry| !d && PAK_EXTENSIONS.contains(&extension(n).as_str());
        let is_mods_folder = |(n, d): &Entry| *d && same_name(n, "~mods");
        let loaders = top.iter().any(|(n, d)| !d && is_loader_file(n));
        let exe_dir = if loaders { self.exe_dir() } else { None };
        let paks = if top.iter().any(|e| is_pak(e) || is_mods_folder(e)) { self.paks_dir() } else { None };
        let root = self.profile.mod_root.as_str();
        let first = self.first_segment();

        top.iter()
            .map(|entry @ (n, d)| {
                let lower = n.to_lowercase();
                let destination = if let Some(folder) = self.profile.loose.iter().find(|(e, _)| !d && *e == extension(n)).map(|(_, f)| *f) {
                    join(folder, n)
                } else if let Some(paks) = paks.as_deref().filter(|_| is_pak(entry)) {
                    format!("{paks}/~mods/{n}")
                } else if let Some(paks) = paks.as_deref().filter(|_| is_mods_folder(entry)) {
                    format!("{paks}/~mods")
                } else if let Some(dir) = exe_dir.as_deref().filter(|_| !d && (is_loader_file(n) || extension(n) == "ini")) {
                    join(dir, n)
                } else if root.is_empty() || self.in_game_root(&lower) || self.is_marker(&lower) || lower == first || lower == "~mods" {
                    n.clone()
                } else {
                    join(root, n)
                };
                place.rule(n, *d, destination)
            })
            .collect()
    }
}
