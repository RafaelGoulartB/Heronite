//! Where the files of an archive go when nobody tells us ([`auto`]): wrappers
//! are unwrapped, Bethesda mods go into `Data`, Thunderstore packages follow
//! r2modman's BepInEx layout. A pure function of the file tree.

use std::collections::HashSet;
use std::path::Path;

use crate::fsutil::{entries, same_name};
use crate::games::{Kind, Profile};
use crate::model::{CopyRule, ModSource};

/// Folders and files that tell a Bethesda mod's content from a wrapper.
const DATA_FOLDERS: &[&str] = &[
    "meshes",
    "textures",
    "scripts",
    "interface",
    "sound",
    "music",
    "seq",
    "strings",
    "video",
    "shadersfx",
    "lodsettings",
    "grass",
    "skse",
    "f4se",
    "nvse",
    "obse",
    "sfse",
    "fose",
    "mcm",
    "source",
    "calientetools",
    "dyndolod",
    "netscriptframework",
    "platform",
    "tools",
];
const DATA_FILES: &[&str] = &["esp", "esm", "esl", "bsa", "ba2"];
/// BepInEx folders r2modman files under `BepInEx/<folder>/<Namespace-Name>`.
const BEPINEX_SPLIT: &[&str] = &["plugins", "patchers", "monomod", "core"];
/// Files of a Thunderstore package that are not part of the mod.
const PACKAGE_FILES: &[&str] = &["manifest.json", "icon.png", "readme.md", "changelog.md"];
/// Files that say nothing about where a mod goes.
const DOC_EXTENSIONS: &[&str] = &["txt", "md", "pdf", "rtf", "html", "htm", "url", "jpg", "jpeg", "png"];

/// Files that are a mod by themselves wherever they sit.
const MOD_FILES: &[&str] = &["dll", "exe", "asi", "esp", "esm", "esl", "bsa", "ba2", "pak", "archive", "ucas", "utoc"];

type Entry = (String, bool);

/// Archive and download names carry a version: a digit next to `-`, `_`,
/// `.`, ` ` or `v` (`Mod-1.2`, `SkyUI_5_2`, `Mod v3`).
fn named_like_a_download(name: &str) -> bool {
    let chars: Vec<char> = name.chars().collect();
    chars.windows(2).any(|w| w[1].is_ascii_digit() && matches!(w[0], '-' | '_' | '.' | ' ' | 'v' | 'V'))
}

/// The copy rules of an archive unpacked at `content`. `destination` is
/// relative to the game folder, `source` to `content`; a folder rule copies
/// the folder's contents to its destination (`""` = the game folder).
/// `game_dir` (when known) lets the game's own folders tell what is a
/// wrapper and what is a part of the game's tree.
pub fn auto(content: &Path, profile: &Profile, game_dir: Option<&Path>, source: &ModSource) -> Vec<CopyRule> {
    let thunderstore = matches!(source, ModSource::Thunderstore { .. });
    let game_root: HashSet<String> = game_dir.map(|d| entries(d).into_iter().map(|(n, _)| n.to_lowercase()).collect()).unwrap_or_default();
    let ctx = Ctx { profile, game_root, thunderstore, package_files: thunderstore || profile.kind == Kind::BepInEx };

    let mut cur = content.to_path_buf();
    let mut prefix = String::new();
    let mut wrapper: Option<String> = None;
    let mut top = ctx.visible(&cur);
    while let [(name, true)] = top.as_slice() {
        let child = cur.join(name);
        if ctx.meaningful(name) || !ctx.may_unwrap(&child) || !ctx.is_wrapper(name, &child, wrapper.is_none()) {
            break;
        }
        prefix = format!("{prefix}{name}/");
        wrapper = Some(name.clone());
        cur = child;
        top = ctx.visible(&cur);
    }
    if top.is_empty() {
        return Vec::new();
    }

    let place = Placer { prefix: &prefix, dir: &cur };
    if profile.kind == Kind::Bethesda {
        ctx.bethesda(&top, &place)
    } else if profile.kind == Kind::BepInEx || thunderstore {
        ctx.bepinex(&top, &place, source, wrapper.as_deref())
    } else {
        ctx.generic(&top, &place)
    }
}

struct Ctx<'a> {
    profile: &'a Profile,
    /// Lower-cased names at the game folder's root.
    game_root: HashSet<String>,
    thunderstore: bool,
    /// `manifest.json` and friends are a Thunderstore package's, not the mod's.
    package_files: bool,
}

struct Placer<'a> {
    /// Path from `content` to the folder holding the files, `""` or `a/b/`.
    prefix: &'a str,
    dir: &'a Path,
}

impl Placer<'_> {
    fn rule(&self, name: &str, is_dir: bool, destination: String) -> CopyRule {
        CopyRule { source: format!("{}{name}", self.prefix), destination, is_folder: is_dir, priority: 0 }
    }
}

fn join(dir: &str, name: &str) -> String {
    if dir.is_empty() {
        name.to_string()
    } else {
        format!("{dir}/{name}")
    }
}

fn extension(name: &str) -> String {
    name.rsplit_once('.').map(|(_, e)| e.to_lowercase()).unwrap_or_default()
}

impl Ctx<'_> {
    /// Entries that are part of the mod: the installer folder and a
    /// Thunderstore package's own files are left out.
    fn visible(&self, dir: &Path) -> Vec<Entry> {
        entries(dir)
            .into_iter()
            .filter(|(name, is_dir)| {
                let lower = name.to_lowercase();
                let installer = *is_dir && lower == "fomod";
                let package = self.package_files && !is_dir && (PACKAGE_FILES.contains(&lower.as_str()) || lower.starts_with("license"));
                !installer && !package
            })
            .collect()
    }

    fn first_segment(&self) -> String {
        self.profile.mod_root.split('/').next().unwrap_or("").to_lowercase()
    }

    fn is_data_marker(&self, name: &str, is_dir: bool) -> bool {
        if is_dir {
            DATA_FOLDERS.contains(&name.to_lowercase().as_str())
        } else {
            DATA_FILES.contains(&extension(name).as_str())
        }
    }

    /// A folder name that is part of the game's or the loader's tree, so not
    /// a wrapper around the mod.
    fn meaningful(&self, name: &str) -> bool {
        let lower = name.to_lowercase();
        if lower == "bepinex" || self.game_root.contains(&lower) {
            return true;
        }
        let first = self.first_segment();
        if !first.is_empty() && lower == first {
            return true;
        }
        match self.profile.kind {
            Kind::Bethesda => DATA_FOLDERS.contains(&lower.as_str()),
            Kind::BepInEx => BEPINEX_SPLIT.contains(&lower.as_str()) || lower == "config" || lower.starts_with("bepinexpack"),
            Kind::Generic => self.thunderstore && lower.starts_with("bepinexpack"),
        }
    }

    /// A lone folder is a wrapper when what is inside it is recognisably the
    /// mod (a folder of the game, a loader's, mod files), or, at the top, when
    /// it is named like a download (`No Intro Screen-12-1-0-1576850747`,
    /// `SkyUI_5_2`). Anything else is the start of the mod's own tree: a
    /// folder the game reads but does not ship (`data_pc` of Control) must
    /// not be peeled down to its files.
    fn is_wrapper(&self, name: &str, dir: &Path, top: bool) -> bool {
        let inside = self.visible(dir);
        let recognised = inside.iter().any(|(n, is_dir)| {
            self.meaningful(n)
                || (self.profile.kind == Kind::Bethesda && self.is_data_marker(n, *is_dir))
                || (!is_dir && MOD_FILES.contains(&extension(n).as_str()))
        });
        recognised || (top && named_like_a_download(name))
    }

    /// Games whose mods are folders of their own (`Mods/<mod>`) keep a single
    /// folder as the mod; it is a wrapper only when the mods folder is in it.
    /// (The game's own names do not tell: `content` is a game folder and the
    /// mod's too.)
    fn may_unwrap(&self, dir: &Path) -> bool {
        if self.profile.kind != Kind::Generic || self.profile.mod_root.is_empty() || self.thunderstore {
            return true;
        }
        let first = self.first_segment();
        entries(dir).iter().any(|(n, _)| n.to_lowercase() == first)
    }

    fn bethesda(&self, top: &[Entry], place: &Placer) -> Vec<CopyRule> {
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

    fn bepinex(&self, top: &[Entry], place: &Placer, source: &ModSource, wrapper: Option<&str>) -> Vec<CopyRule> {
        let ns_name = match source {
            ModSource::Thunderstore { namespace, name, .. } => format!("{namespace}-{name}"),
            _ => wrapper.unwrap_or("mod").to_string(),
        };
        let named_pack = matches!(source, ModSource::Thunderstore { name, .. } if name.to_lowercase().starts_with("bepinexpack"));
        let has = |name: &str, dir: bool| top.iter().any(|(n, d)| *d == dir && same_name(n, name));
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

    fn generic(&self, top: &[Entry], place: &Placer) -> Vec<CopyRule> {
        let root = self.profile.mod_root.as_str();
        let to = |dir: &str| top.iter().map(|(n, d)| place.rule(n, *d, join(dir, n))).collect();
        if root.is_empty() {
            return to("");
        }
        let first = self.first_segment();
        let has_first = top.iter().any(|(n, _)| n.to_lowercase() == first);
        let known = |(n, d): &Entry| self.game_root.contains(&n.to_lowercase()) || (!d && DOC_EXTENSIONS.contains(&extension(n).as_str()));
        let in_game =
            !self.game_root.is_empty() && top.iter().all(known) && top.iter().any(|(n, _)| self.game_root.contains(&n.to_lowercase()));
        if has_first || in_game {
            to("")
        } else {
            to(root)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::games::{find, KNOWN};
    use crate::model::GameLinks;

    fn tree(paths: &[&str]) -> tempfile::TempDir {
        let t = tempfile::tempdir().unwrap();
        for p in paths {
            let full = t.path().join(p);
            if p.ends_with('/') {
                std::fs::create_dir_all(&full).unwrap();
            } else {
                std::fs::create_dir_all(full.parent().unwrap()).unwrap();
                std::fs::write(&full, b"x").unwrap();
            }
        }
        t
    }

    fn profile(steam_id: &str) -> Profile {
        Profile::of(&GameLinks::default(), find(Some(steam_id), ""))
    }

    fn map(rules: &[CopyRule]) -> Vec<String> {
        let mut v: Vec<String> = rules.iter().map(|r| format!("{} -> {}", r.source, r.destination)).collect();
        v.sort();
        v
    }

    const SKYRIM: &str = "489830";

    #[test]
    fn skyrim_texture_mod_in_a_wrapper_goes_under_data() {
        let c = tree(&["Cool Textures-123-1/textures/a.dds", "Cool Textures-123-1/meshes/b.nif"]);
        let rules = auto(c.path(), &profile(SKYRIM), None, &ModSource::File);
        assert_eq!(map(&rules), ["Cool Textures-123-1/meshes -> Data/meshes", "Cool Textures-123-1/textures -> Data/textures"]);
        assert!(rules.iter().all(|r| r.is_folder));
    }

    #[test]
    fn script_extender_goes_to_the_game_folder() {
        let c = tree(&["skse64_loader.exe", "skse64_1_6_1170.dll", "Data/Scripts/x.pex", "src/readme.txt"]);
        let rules = auto(c.path(), &profile(SKYRIM), None, &ModSource::File);
        assert_eq!(
            map(&rules),
            ["Data -> Data", "skse64_1_6_1170.dll -> skse64_1_6_1170.dll", "skse64_loader.exe -> skse64_loader.exe", "src -> src"]
        );
    }

    #[test]
    fn a_lone_plugin_goes_under_data_and_a_data_folder_in_a_wrapper_to_the_root() {
        let c = tree(&["MyMod.esp"]);
        assert_eq!(map(&auto(c.path(), &profile(SKYRIM), None, &ModSource::File)), ["MyMod.esp -> Data/MyMod.esp"]);
        let c = tree(&["Some Mod v2/data/MyMod.esp", "Some Mod v2/data/textures/a.dds"]);
        assert_eq!(map(&auto(c.path(), &profile(SKYRIM), None, &ModSource::File)), ["Some Mod v2/data -> Data"]);
    }

    #[test]
    fn executables_alone_go_to_the_root_and_odd_files_under_data() {
        let c = tree(&["ENBSeries.dll", "d3d11.dll"]);
        assert_eq!(
            map(&auto(c.path(), &profile(SKYRIM), None, &ModSource::File)),
            ["ENBSeries.dll -> ENBSeries.dll", "d3d11.dll -> d3d11.dll"]
        );
        let c = tree(&["config.ini"]);
        assert_eq!(map(&auto(c.path(), &profile(SKYRIM), None, &ModSource::File)), ["config.ini -> Data/config.ini"]);
    }

    #[test]
    fn the_installer_folder_is_ignored() {
        let c = tree(&["fomod/ModuleConfig.xml", "Textures/a.dds"]);
        assert_eq!(map(&auto(c.path(), &profile(SKYRIM), None, &ModSource::File)), ["Textures -> Data/Textures"]);
        let c = tree(&["Wrapper/fomod/info.xml", "Wrapper/meshes/a.nif"]);
        assert_eq!(map(&auto(c.path(), &profile(SKYRIM), None, &ModSource::File)), ["Wrapper/meshes -> Data/meshes"]);
    }

    fn ts(namespace: &str, name: &str) -> ModSource {
        ModSource::Thunderstore { community: "valheim".into(), namespace: namespace.into(), name: name.into() }
    }

    fn valheim() -> Profile {
        profile("892970")
    }

    #[test]
    fn bepinex_pack_goes_to_the_game_folder() {
        let c =
            tree(&["manifest.json", "icon.png", "README.md", "BepInExPack_Valheim/BepInEx/core/a.dll", "BepInExPack_Valheim/winhttp.dll"]);
        let rules = auto(c.path(), &valheim(), None, &ts("denikson", "BepInExPack_Valheim"));
        assert_eq!(map(&rules), ["BepInExPack_Valheim -> "]);
        let c = tree(&["manifest.json", "BepInEx/core/a.dll", "winhttp.dll", "doorstop_config.ini"]);
        let rules = auto(c.path(), &valheim(), None, &ModSource::File);
        assert_eq!(map(&rules), ["BepInEx -> BepInEx", "doorstop_config.ini -> doorstop_config.ini", "winhttp.dll -> winhttp.dll"]);
    }

    #[test]
    fn thunderstore_plugin_with_a_loose_dll() {
        let c = tree(&["manifest.json", "icon.png", "README.md", "LICENSE", "CHANGELOG.md", "MyPlugin.dll"]);
        let rules = auto(c.path(), &valheim(), None, &ts("Author", "MyPlugin"));
        assert_eq!(map(&rules), ["MyPlugin.dll -> BepInEx/plugins/Author-MyPlugin/MyPlugin.dll"]);
    }

    #[test]
    fn thunderstore_plugin_with_plugins_and_config_folders() {
        let c = tree(&["manifest.json", "plugins/a.dll", "config/a.cfg", "patchers/p.dll", "assets/x.bundle"]);
        let rules = auto(c.path(), &valheim(), None, &ts("Author", "Thing"));
        assert_eq!(
            map(&rules),
            [
                "assets -> BepInEx/plugins/Author-Thing/assets",
                "config -> BepInEx/config",
                "patchers -> BepInEx/patchers/Author-Thing",
                "plugins -> BepInEx/plugins/Author-Thing"
            ]
        );
        let c = tree(&["BepInEx/plugins/x/a.dll", "BepInEx/config/a.cfg"]);
        assert_eq!(map(&auto(c.path(), &valheim(), None, &ts("A", "B"))), ["BepInEx -> BepInEx"]);
    }

    #[test]
    fn a_wrapper_around_a_plugin_is_unwrapped_and_names_the_folder() {
        let c = tree(&["CoolMod-1.2/CoolMod.dll"]);
        let rules = auto(c.path(), &valheim(), None, &ModSource::File);
        assert_eq!(map(&rules), ["CoolMod-1.2/CoolMod.dll -> BepInEx/plugins/CoolMod-1.2/CoolMod.dll"]);
    }

    fn cyberpunk() -> Profile {
        profile("1091500")
    }

    #[test]
    fn cyberpunk_archive_with_r6_goes_to_the_root() {
        let game = tree(&["archive/pc/content/a.archive", "bin/x64/Cyberpunk2077.exe", "r6/config/a.xml"]);
        let c = tree(&["archive/pc/mod/m.archive", "r6/scripts/m.reds"]);
        let rules = auto(c.path(), &cyberpunk(), Some(game.path()), &ModSource::File);
        assert_eq!(map(&rules), ["archive -> archive", "r6 -> r6"]);
        let c = tree(&["My Mod 2.0/archive/pc/mod/m.archive"]);
        assert_eq!(map(&auto(c.path(), &cyberpunk(), Some(game.path()), &ModSource::File)), ["My Mod 2.0/archive -> archive"]);
    }

    #[test]
    fn witcher_mod_folder_goes_under_mods() {
        let game = tree(&["bin/x64/witcher3.exe", "content/content0/a.bundle", "dlc/dlc1/a.w3strings", "Mods/"]);
        let witcher = profile("292030");
        let c = tree(&["modFoo/content/scripts/a.ws"]);
        assert_eq!(map(&auto(c.path(), &witcher, Some(game.path()), &ModSource::File)), ["modFoo -> Mods/modFoo"]);
        let c = tree(&["mods/modFoo/content/scripts/a.ws", "readme.txt"]);
        assert_eq!(map(&auto(c.path(), &witcher, Some(game.path()), &ModSource::File)), ["mods -> mods", "readme.txt -> readme.txt"]);
        let c = tree(&["dlc/dlcFoo/content/a.bundle"]);
        assert_eq!(map(&auto(c.path(), &witcher, Some(game.path()), &ModSource::File)), ["dlc -> dlc"]);
        let c = tree(&["Wrapper v1/mods/modFoo/content/a.bundle"]);
        assert_eq!(map(&auto(c.path(), &witcher, Some(game.path()), &ModSource::File)), ["Wrapper v1/mods -> mods"]);
    }

    #[test]
    fn a_user_chosen_folder_is_used_for_unknown_games() {
        let links = GameLinks { mod_root: Some("Plugins".into()), ..Default::default() };
        let p = Profile::of(&links, None);
        let c = tree(&["foo.dll", "bar.txt"]);
        assert_eq!(map(&auto(c.path(), &p, None, &ModSource::File)), ["bar.txt -> Plugins/bar.txt", "foo.dll -> Plugins/foo.dll"]);
        assert!(auto(tree(&[]).path(), &p, None, &ModSource::File).is_empty());
    }

    #[test]
    fn a_game_folder_the_game_does_not_ship_is_not_peeled() {
        // Control's loose files go in `data_pc`, which the game folder lacks.
        let game = tree(&["data/x.rmdp", "Control_DX12.exe"]);
        let t = tree(&["data_pc/textures/videos/intro/startvideos.tex"]);
        let rules = auto(t.path(), &Profile::of(&Default::default(), None), Some(game.path()), &ModSource::File);
        assert_eq!(map(&rules), ["data_pc -> data_pc"]);
        // The same in a download-named wrapper: the wrapper goes, the tree stays.
        let t = tree(&["No Intro Screen-12-1-0-1576850747/data_pc/textures/videos/intro/startvideos.tex"]);
        let rules = auto(t.path(), &Profile::of(&Default::default(), None), Some(game.path()), &ModSource::File);
        assert_eq!(map(&rules), ["No Intro Screen-12-1-0-1576850747/data_pc -> data_pc"]);
    }

    #[test]
    fn download_names() {
        assert!(named_like_a_download("No Intro Screen-12-1-0-1576850747"));
        assert!(named_like_a_download("SkyUI_5_2"));
        assert!(named_like_a_download("Mod v3"));
        assert!(!named_like_a_download("data_pc"));
        assert!(!named_like_a_download("textures"));
    }

    #[test]
    fn symlinks_are_not_entries() {
        let c = tree(&["textures/a.dds"]);
        std::os::unix::fs::symlink("/etc", c.path().join("evil")).unwrap();
        assert_eq!(map(&auto(c.path(), &profile(SKYRIM), None, &ModSource::File)), ["textures -> Data/textures"]);
    }

    #[test]
    fn every_known_game_has_a_layout() {
        let c = tree(&["a.txt"]);
        for k in KNOWN {
            let rules = auto(c.path(), &Profile::of(&GameLinks::default(), Some(k)), None, &ModSource::File);
            assert_eq!(rules.len(), 1, "{}", k.name);
        }
    }
}
