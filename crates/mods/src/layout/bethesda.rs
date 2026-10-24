//! Bethesda games: mods go into `Data`, script extenders and graphics
//! injectors next to the executable.

use crate::fsutil::{extension, same_name};
use crate::install::is_proxy_dll;
use crate::model::CopyRule;

use super::{join, Ctx, Entry, Placer};

/// ENB and ReShade files that are not proxy DLLs.
const GRAPHICS_FILES: &[&str] = &["d3dcompiler_46e.dll", "enbseries.ini", "enblocal.ini", "reshade.ini"];
const GRAPHICS_FOLDERS: &[&str] = &["enbseries", "reshade-shaders", "reshade-presets"];

impl Ctx<'_> {
    pub(super) fn bethesda(&self, top: &[Entry], place: &Placer) -> Vec<CopyRule> {
        let (graphics, rest): (Vec<Entry>, Vec<Entry>) = top.iter().cloned().partition(|e| self.is_graphics(top, e));
        let mut rules: Vec<CopyRule> = graphics.iter().map(|(n, d)| place.rule(n, *d, n.clone())).collect();
        rules.extend(self.data_or_root(&rest, place));
        rules
    }

    /// ENB and ReShade archives (`d3d11.dll`, `enbseries/`, their `.ini`) are
    /// installed in the game folder, never in `Data`.
    fn is_graphics(&self, top: &[Entry], (name, is_dir): &Entry) -> bool {
        let lower = name.to_lowercase();
        if *is_dir {
            return GRAPHICS_FOLDERS.contains(&lower.as_str());
        }
        is_proxy_dll(name)
            || GRAPHICS_FILES.contains(&lower.as_str())
            || (extension(name) == "ini" && top.iter().any(|(n, d)| !d && is_proxy_dll(n)))
    }

    fn data_or_root(&self, top: &[Entry], place: &Placer) -> Vec<CopyRule> {
        let data = if self.profile.mod_root.is_empty() { "Data" } else { self.profile.mod_root.as_str() };
        let data_first = data.split('/').next().unwrap_or(data);
        let to_root = |top: &[Entry]| top.iter().map(|(n, d)| place.rule(n, *d, n.clone())).collect();
        if top.iter().any(|(n, d)| *d && same_name(n, data_first)) {
            // Script extenders ship `Data` next to their loader: the game folder.
            return top
                .iter()
                .map(|(n, d)| place.rule(n, *d, if *d && same_name(n, data_first) { data.to_string() } else { n.clone() }))
                .collect();
        }
        let under_data = |top: &[Entry]| -> Vec<CopyRule> { top.iter().map(|(n, d)| place.rule(n, *d, join(data, n))).collect() };
        if top.iter().any(|(n, d)| self.is_data_marker(n, *d)) {
            return under_data(top);
        }
        if top.iter().all(|(n, d)| !d && matches!(extension(n).as_str(), "exe" | "dll")) {
            return to_root(top);
        }
        under_data(top)
    }
}
