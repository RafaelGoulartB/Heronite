//! Where the files of an archive go when nobody tells us ([`auto`]): wrappers
//! are unwrapped, Bethesda mods go into `Data`, Thunderstore packages follow
//! r2modman's layouts. A pure function of the file tree (and of the game
//! folder, which is only read).

mod bepinex;
mod bethesda;
mod generic;
#[cfg(test)]
mod tests;

use std::collections::HashSet;
use std::path::Path;

use crate::fsutil::{entries, extension, find_ci};
use crate::games::{Kind, Profile};
use crate::install::{exe_dirs, is_proxy_dll};
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
/// Folders of a MelonLoader install (the loader's and its mods').
const MELON_FOLDERS: &[&str] = &["melonloader", "mods", "userlibs"];
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

/// `Mod-1.2.0` as `Mod`, `No Intro Screen-12-1-0-1576850747` as `No Intro
/// Screen`: the name up to a tail that is only a version.
fn without_version(name: &str) -> String {
    for (i, c) in name.char_indices().filter(|(_, c)| matches!(c, '-' | '_' | ' ' | '.')) {
        let tail = &name[i + c.len_utf8()..];
        let tail = tail.strip_prefix(['v', 'V']).unwrap_or(tail);
        let is_version = tail.starts_with(|c: char| c.is_ascii_digit())
            && tail.chars().all(|c| c.is_ascii_digit() || matches!(c, '.' | '-' | '_' | ' '));
        let head = name[..i].trim_end();
        if is_version && head.chars().any(char::is_alphabetic) {
            return head.to_string();
        }
    }
    name.to_string()
}

/// `LICENSE`, `license.txt`, `Licence-MIT.md`; not `LicenseManager.dll`.
fn is_license(lower: &str) -> bool {
    let stem = lower.strip_suffix(".txt").or_else(|| lower.strip_suffix(".md")).unwrap_or(lower);
    let rest = stem.strip_prefix("license").or_else(|| stem.strip_prefix("licence"));
    rest.is_some_and(|r| r.is_empty() || r.starts_with(['-', '_', ' ']))
}

fn is_doc(name: &str) -> bool {
    DOC_EXTENSIONS.contains(&extension(name).as_str())
}

/// A file a game loads from the folder of its executable: a proxy DLL, an
/// ASI plugin or a launcher.
fn is_loader_file(name: &str) -> bool {
    is_proxy_dll(name) || matches!(extension(name).as_str(), "asi" | "exe")
}

/// The copy rules of an archive unpacked at `content`. `destination` is
/// relative to the game folder, `source` to `content`; a folder rule copies
/// the folder's contents to its destination (`""` = the game folder).
/// `game_dir` (when known) lets the game's own folders tell what is a
/// wrapper and what is a part of the game's tree.
pub fn auto(content: &Path, profile: &Profile, game_dir: Option<&Path>, source: &ModSource) -> Vec<CopyRule> {
    let thunderstore = matches!(source, ModSource::Thunderstore { .. });
    let game_root: HashSet<String> = game_dir.map(|d| entries(d).into_iter().map(|(n, _)| n.to_lowercase()).collect()).unwrap_or_default();
    let ctx = Ctx { profile, game_dir, game_root, thunderstore, package_files: thunderstore || profile.kind == Kind::BepInEx };

    let mut cur = content.to_path_buf();
    let mut prefix = String::new();
    let mut wrapper: Option<String> = None;
    let mut top = ctx.visible(&cur);
    // Docs next to a lone folder do not make it two things.
    while let [(name, true)] = ctx.core(&top).as_slice() {
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
        ctx.package(&top, &place, source, wrapper.as_deref())
    } else {
        ctx.generic(&top, &place)
    }
}

struct Ctx<'a> {
    profile: &'a Profile,
    game_dir: Option<&'a Path>,
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

impl Ctx<'_> {
    /// Entries that are part of the mod: the installer folder and a
    /// Thunderstore package's own files are left out.
    fn visible(&self, dir: &Path) -> Vec<Entry> {
        entries(dir)
            .into_iter()
            .filter(|(name, is_dir)| {
                let lower = name.to_lowercase();
                let installer = *is_dir && lower == "fomod";
                let package = self.package_files && !is_dir && (PACKAGE_FILES.contains(&lower.as_str()) || is_license(&lower));
                !installer && !package
            })
            .collect()
    }

    /// `top` without docs and images.
    fn core(&self, top: &[Entry]) -> Vec<Entry> {
        top.iter().filter(|(n, is_dir)| *is_dir || !is_doc(n)).cloned().collect()
    }

    fn first_segment(&self) -> String {
        self.profile.mod_root.split('/').next().unwrap_or("").to_lowercase()
    }

    fn is_marker(&self, name: &str) -> bool {
        self.profile.markers.iter().any(|m| m.eq_ignore_ascii_case(name))
    }

    /// Whether `name` is at the root of the game folder, case aside.
    fn in_game_root(&self, name: &str) -> bool {
        self.game_root.contains(&name.to_lowercase())
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
        if lower == "bepinex" || lower == "~mods" || self.in_game_root(&lower) || self.is_marker(&lower) {
            return true;
        }
        let first = self.first_segment();
        if !first.is_empty() && lower == first {
            return true;
        }
        if self.package_files && MELON_FOLDERS.contains(&lower.as_str()) {
            return true;
        }
        match self.profile.kind {
            Kind::Bethesda => DATA_FOLDERS.contains(&lower.as_str()),
            Kind::BepInEx => BEPINEX_SPLIT.contains(&lower.as_str()) || lower == "config" || lower.starts_with("bepinexpack"),
            Kind::Generic => self.thunderstore && (lower.starts_with("bepinexpack") || bepinex::ROM_FOLDERS.contains(&lower.as_str())),
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

    /// The folder (relative to the game folder) of the only executable, when
    /// the game folder itself has none (Unreal's `<Game>/Binaries/Win64`):
    /// where proxy DLLs and ASI loaders take effect.
    fn exe_dir(&self) -> Option<String> {
        match exe_dirs(self.game_dir?).as_slice() {
            [only] if !only.is_empty() => Some(only.clone()),
            _ => None,
        }
    }

    /// `<X>/Content/Paks` of an Unreal game, spelled as the game has it.
    fn paks_dir(&self) -> Option<String> {
        let game = self.game_dir?;
        entries(game).into_iter().filter(|(_, d)| *d).find_map(|(x, _)| {
            let (content, _) = find_ci(&game.join(&x), "Content").filter(|(_, d)| *d)?;
            let (paks, _) = find_ci(&game.join(&x).join(&content), "Paks").filter(|(_, d)| *d)?;
            Some(format!("{x}/{content}/{paks}"))
        })
    }
}
