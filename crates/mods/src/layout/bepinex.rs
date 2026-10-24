//! Thunderstore packages (and archives of BepInEx games): r2modman's layouts
//! for BepInEx, MelonLoader and ReturnOfModding.

use crate::fsutil::extension;
use crate::model::{CopyRule, ModSource};

use super::{is_loader_file, join, without_version, Ctx, Entry, Placer, BEPINEX_SPLIT, MELON_FOLDERS};

/// Folders of a ReturnOfModding plugin that have a place of their own.
pub(super) const ROM_FOLDERS: &[&str] = &["plugins", "plugins_data", "config", "returnofmodding"];

impl Ctx<'_> {
    pub(super) fn package(&self, top: &[Entry], place: &Placer, source: &ModSource, wrapper: Option<&str>) -> Vec<CopyRule> {
        let ns_name = match source {
            ModSource::Thunderstore { namespace, name, .. } => format!("{namespace}-{name}"),
            _ => wrapper.map_or_else(|| "mod".to_string(), without_version),
        };
        if self.is_melon(top, source) {
            // MelonLoader and its mods are laid out relative to the game folder.
            return top.iter().map(|(n, d)| place.rule(n, *d, n.clone())).collect();
        }
        if self.profile.kind == crate::games::Kind::Generic {
            return self.return_of_modding(top, place, &ns_name);
        }
        self.bepinex(top, place, source, &ns_name)
    }

    /// MelonLoader's own package, or content laid out like its install
    /// (`MelonLoader/`, `Mods/`, `UserLibs/`, `version.dll` with `dobby.dll`).
    fn is_melon(&self, top: &[Entry], source: &ModSource) -> bool {
        let loader_package = matches!(source, ModSource::Thunderstore { namespace, name, .. }
            if namespace.eq_ignore_ascii_case("LavaGang") && name.eq_ignore_ascii_case("MelonLoader"));
        let has_file = |file: &str| top.iter().any(|(n, d)| !d && n.eq_ignore_ascii_case(file));
        loader_package
            || top.iter().any(|(n, d)| *d && MELON_FOLDERS.contains(&n.to_lowercase().as_str()))
            || (has_file("version.dll") && has_file("dobby.dll"))
    }

    fn bepinex(&self, top: &[Entry], place: &Placer, source: &ModSource, ns_name: &str) -> Vec<CopyRule> {
        let named_pack = matches!(source, ModSource::Thunderstore { name, .. } if name.to_lowercase().starts_with("bepinexpack"));
        let has = |name: &str, dir: bool| top.iter().any(|(n, d)| *d == dir && n.eq_ignore_ascii_case(name));
        let pack_dir = top.iter().find(|(n, d)| *d && n.to_lowercase().starts_with("bepinexpack"));
        let pack = named_pack
            || pack_dir.is_some_and(|(n, _)| place.dir.join(n).join("BepInEx").is_dir())
            || (has("winhttp.dll", false) && has("BepInEx", true));
        if pack {
            return top
                .iter()
                .map(|(n, d)| place.rule(n, *d, if pack_dir.is_some_and(|(p, _)| p == n) { String::new() } else { n.clone() }))
                .collect();
        }
        top.iter()
            .map(|(n, d)| {
                let lower = n.to_lowercase();
                let dest = if *d && lower == "bepinex" {
                    "BepInEx".to_string()
                } else if *d && BEPINEX_SPLIT.contains(&lower.as_str()) {
                    format!("BepInEx/{lower}/{ns_name}")
                } else if *d && lower == "config" {
                    "BepInEx/config".to_string()
                } else {
                    format!("BepInEx/plugins/{ns_name}/{n}")
                };
                place.rule(n, *d, dest)
            })
            .collect()
    }

    /// Hades II's Hell2Modding (r2modman's ReturnOfModding installers): the
    /// loader's `d3d12.dll` sits next to the executable and so does the
    /// `ReturnOfModding` folder, a plugin lives in
    /// `ReturnOfModding/plugins/<Namespace-Name>`.
    fn return_of_modding(&self, top: &[Entry], place: &Placer, ns_name: &str) -> Vec<CopyRule> {
        let base = self.exe_dir().unwrap_or_default();
        top.iter()
            .map(|(n, d)| {
                let lower = n.to_lowercase();
                let dest = if !d && (is_loader_file(n) || extension(n) == "ini") {
                    join(&base, n)
                } else if *d && lower == "returnofmodding" {
                    join(&base, "ReturnOfModding")
                } else if *d && ROM_FOLDERS.contains(&lower.as_str()) {
                    join(&base, &format!("ReturnOfModding/{lower}/{ns_name}"))
                } else {
                    join(&base, &format!("ReturnOfModding/plugins/{ns_name}/{n}"))
                };
                place.rule(n, *d, dest)
            })
            .collect()
    }
}
