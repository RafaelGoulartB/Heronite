//! Finding emulators that are not on `$PATH`: AppImages and binaries in the
//! usual places (`~/Applications`, `~/.local/bin`, `~/Games`, `/opt`, the
//! folders of emulators already configured) or in a folder the user picks,
//! matched against the catalog by file name.
//!
//! Nothing here executes a file: matching is by name and permission bits only.

use std::collections::HashSet;
use std::path::Path;
use std::time::{Duration, Instant};

use anyhow::Result;
use heronite_data::DataPaths;
use heronite_launch::emulators::{UserEmulator, UserEmulatorProfile};

use crate::catalog::{emulator_definition, emulator_definitions};
use crate::detect::{detect_emulators, existing_profiles, DetectEnv};
use crate::pathutil::{basename, dirname};
use crate::store::{self, insert_user_emulator};
use crate::types::{DetectedEmulator, DetectedEmulatorProfile};

/// A Switch emulator the catalog has no entry for (yuzu and its forks). It is
/// registered as a `custom` emulator with this profile.
struct CustomProfile {
    id: &'static str,
    name: &'static str,
    platform_id: &'static str,
    platform_name: &'static str,
    arguments: &'static str,
    extensions: &'static [&'static str],
}

const SWITCH: CustomProfile = CustomProfile {
    id: "switch",
    name: "Nintendo Switch",
    platform_id: "nintendo_switch",
    platform_name: "Nintendo Switch",
    arguments: "-f -g {ImagePath}",
    extensions: &["nsp", "xci", "nca", "nro", "nso"],
};

struct Rule {
    /// Catalog definition (`custom` for [`CustomProfile`] rules).
    definition: &'static str,
    name: &'static str,
    /// Lower-case alphanumeric names (a file name's first token).
    aliases: &'static [&'static str],
    /// Aliases that clash with common programs (KDE's `dolphin`): only
    /// accepted for AppImages.
    appimage_only: &'static [&'static str],
    custom: Option<&'static CustomProfile>,
}

const fn rule(definition: &'static str, name: &'static str, aliases: &'static [&'static str]) -> Rule {
    Rule { definition, name, aliases, appimage_only: &[], custom: None }
}

const RULES: &[Rule] = &[
    Rule { appimage_only: &["dolphin"], ..rule("dolphin", "Dolphin", &["dolphinemu"]) },
    rule("pcsx2", "PCSX2", &["pcsx2"]),
    rule("duckstation", "DuckStation", &["duckstation"]),
    rule("ppsspp", "PPSSPP", &["ppsspp"]),
    rule("rpcs3", "RPCS3", &["rpcs3"]),
    rule("cemu", "Cemu", &["cemu"]),
    rule("ryujinx", "Ryujinx", &["ryujinx"]),
    rule("xemu", "Xemu", &["xemu"]),
    rule("melonds", "melonDS", &["melonds"]),
    rule("mgba", "mGBA", &["mgba"]),
    rule("flycast", "Flycast", &["flycast"]),
    rule("mame", "MAME", &["mame"]),
    rule("scummvm", "ScummVM", &["scummvm"]),
    rule("vita3k", "Vita3K", &["vita3k"]),
    rule("shadps4", "shadPS4", &["shadps4"]),
    rule("azahar", "Azahar", &["azahar", "citra"]),
    rule("retroarch", "RetroArch", &["retroarch"]),
    Rule { custom: Some(&SWITCH), ..rule("custom", "Yuzu", &["yuzu"]) },
    Rule { custom: Some(&SWITCH), ..rule("custom", "Suyu", &["suyu"]) },
    Rule { custom: Some(&SWITCH), ..rule("custom", "Sudachi", &["sudachi"]) },
    Rule { custom: Some(&SWITCH), ..rule("custom", "Eden", &["eden"]) },
    Rule { custom: Some(&SWITCH), ..rule("custom", "Citron", &["citron"]) },
];

/// What may follow an alias in a file name token (`PPSSPPSDL`, `pcsx2-qt`,
/// `shadPS4QtLauncher`).
const SUFFIXES: &[&str] = &[
    "qt", "sdl", "sdl2", "nogui", "emu", "gui", "qtlauncher", "launcher", "x64", "x8664", "linux",
    "appimage", "nightly", "mgr",
];

const NOT_EXECUTABLE_EXT: &[&str] = &[
    "so", "txt", "json", "ini", "png", "jpg", "svg", "desktop", "log", "md", "zip", "7z", "gz", "xz",
    "bz2", "tar", "dll", "yml", "yaml", "cfg", "conf", "bak", "sig", "sha256", "zsync", "pdb", "xml",
    "ico", "pkg", "iso", "bin", "chd", "cue",
];

fn matches_alias(token: &str, alias: &str) -> bool {
    token == alias || token.strip_prefix(alias).is_some_and(|rest| SUFFIXES.contains(&rest))
}

fn is_appimage(file_name: &str) -> bool {
    file_name.to_lowercase().ends_with(".appimage")
}

/// The catalog rule a file name belongs to (`pcsx2-v2.8.2.AppImage`,
/// `Ryujinx`, `dolphin-emu`...), or `None`.
fn identify(file_name: &str) -> Option<&'static Rule> {
    let lower = file_name.to_lowercase();
    let tokens: Vec<&str> = lower.split(|c: char| !c.is_ascii_alphanumeric()).filter(|t| !t.is_empty()).collect();
    let first = *tokens.first()?;
    let pair = format!("{first}{}", tokens.get(1).copied().unwrap_or(""));
    let appimage = is_appimage(file_name);
    for rule in RULES {
        for alias in rule.aliases.iter().chain(rule.appimage_only) {
            if rule.appimage_only.contains(alias) && !appimage {
                continue;
            }
            if matches_alias(first, alias) || matches_alias(&pair, alias) {
                return Some(rule);
            }
        }
    }
    None
}

fn ext_of(name: &str) -> String {
    Path::new(name).extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default()
}

#[cfg(unix)]
fn has_exec_bit(meta: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt;
    meta.permissions().mode() & 0o111 != 0
}

#[cfg(not(unix))]
fn has_exec_bit(_: &std::fs::Metadata) -> bool {
    true
}

/// Directory names never entered (ROM, save and cache trees can be huge).
const SKIP_DIRS: &[&str] = &[
    "games", "roms", "rom", "saves", "save", "savestates", "states", "bios", "firmware", "pkg", "media",
    "covers", "art", "screenshots", "shaders", "textures", "cache", "logs", "log", "dump", "dumps",
    "node_modules", "squashfs-root", "mods", "cheats", "themes", "sysdata", "dev_hdd0", "steamapps",
    "compatdata", "backups", "playlists", "thumbnails", "cores", "system", "assets", "icons", "locales",
];

/// Limits of one walk (a folder full of games must not stall the UI worker).
#[derive(Debug, Clone, Copy)]
pub struct WalkLimits {
    pub max_depth: usize,
    /// Directory entries looked at, in total.
    pub max_entries: usize,
    /// Directories with more entries than this are skipped (game libraries).
    pub max_dir_entries: usize,
    pub time: Duration,
}

impl Default for WalkLimits {
    fn default() -> Self {
        Self { max_depth: 4, max_entries: 60_000, max_dir_entries: 3_000, time: Duration::from_secs(6) }
    }
}

/// An executable file whose name matches the catalog.
#[derive(Debug, Clone, PartialEq)]
pub struct Candidate {
    pub path: String,
    pub definition_id: &'static str,
    pub name: &'static str,
    pub wine: bool,
}

struct Walk {
    limits: WalkLimits,
    started: Instant,
    seen: usize,
}

impl Walk {
    fn exhausted(&self) -> bool {
        self.seen >= self.limits.max_entries || self.started.elapsed() > self.limits.time
    }
}

/// Executable files matching the catalog under `root` (root itself may be a
/// file). Never executes anything.
pub fn find_candidates(root: &Path, limits: WalkLimits) -> Vec<Candidate> {
    let mut out = Vec::new();
    let mut walk = Walk { limits, started: Instant::now(), seen: 0 };
    visit(root, 0, true, &mut walk, &mut out);
    out
}

fn visit(dir: &Path, depth: usize, is_root: bool, walk: &mut Walk, out: &mut Vec<Candidate>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        if is_root {
            consider(dir, out);
        }
        return;
    };
    let mut entries: Vec<std::fs::DirEntry> = Vec::new();
    for e in rd.flatten() {
        entries.push(e);
        if entries.len() > walk.limits.max_dir_entries {
            // A game library, not an emulator folder.
            return;
        }
    }
    walk.seen += entries.len();
    entries.sort_by_key(|e| e.file_name());
    let mut subdirs = Vec::new();
    for e in entries {
        if walk.exhausted() {
            return;
        }
        let name = e.file_name().to_string_lossy().into_owned();
        let Ok(ft) = e.file_type() else { continue };
        if ft.is_dir() {
            let lower = name.to_lowercase();
            if depth < walk.limits.max_depth && !name.starts_with('.') && !SKIP_DIRS.contains(&lower.as_str()) {
                subdirs.push(e.path());
            }
            continue;
        }
        // Files and symlinks to files.
        consider(&e.path(), out);
    }
    for d in subdirs {
        if walk.exhausted() {
            return;
        }
        visit(&d, depth + 1, false, walk, out);
    }
}

fn consider(path: &Path, out: &mut Vec<Candidate>) {
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let Some(rule) = identify(&name) else { return };
    let ext = ext_of(&name);
    let appimage = ext == "appimage";
    let exe = ext == "exe";
    if !appimage && !exe && NOT_EXECUTABLE_EXT.contains(&ext.as_str()) {
        return;
    }
    // Follows symlinks.
    let Ok(meta) = std::fs::metadata(path) else { return };
    if !meta.is_file() {
        return;
    }
    if !(appimage || exe || has_exec_bit(&meta)) {
        return;
    }
    out.push(Candidate {
        path: path.to_string_lossy().into_owned(),
        definition_id: rule.definition,
        name: rule.name,
        wine: exe,
    });
}

/// A folder to search with its label (shown as the result's source).
#[derive(Debug, Clone, PartialEq)]
pub struct SearchRoot {
    pub path: String,
    pub depth: usize,
    pub label: String,
}

/// The usual places plus the folders of emulators already configured.
pub fn common_search_roots(home: &str, existing: &[UserEmulator]) -> Vec<SearchRoot> {
    let mut roots: Vec<SearchRoot> = Vec::new();
    let mut add = |path: String, depth: usize, label: &str| {
        if !roots.iter().any(|r| r.path == path) {
            roots.push(SearchRoot { path, depth, label: label.to_owned() });
        }
    };
    for (rel, depth) in [
        ("Applications", 2),
        ("Apps", 3),
        ("AppImages", 2),
        (".local/bin", 1),
        (".local/opt", 2),
        ("bin", 1),
        ("Games", 3),
        ("Emulation", 4),
        ("Emulators", 4),
        ("Downloads", 1),
    ] {
        add(format!("{home}/{rel}"), depth, &format!("~/{rel}"));
    }
    for (abs, depth) in [("/opt", 3), ("/usr/local/bin", 1), ("/usr/games", 1)] {
        add(abs.to_owned(), depth, abs);
    }
    for e in existing {
        if e.launch_kind == "flatpak" || !e.executable.contains('/') {
            continue;
        }
        let dir = dirname(&e.executable);
        let parent = dirname(&dir);
        let grand = dirname(&parent);
        add(dir.clone(), 3, "next to a configured emulator");
        // Emulators are often kept one per folder (`Emulation/PS1/emulator`,
        // `Emulation/PS2/emulator`): look at the siblings too, unless that
        // would mean searching `$HOME` or a top-level folder.
        let deep_enough = |p: &str| p.split('/').filter(|s| !s.is_empty()).count() >= 2 && p != home;
        if deep_enough(&grand) {
            add(grand, 3, "next to a configured emulator");
        } else if parent != dir && deep_enough(&parent) {
            add(parent, 1, "next to a configured emulator");
        }
    }
    roots
}

fn canonical(path: &str) -> String {
    std::fs::canonicalize(path).map(|p| p.to_string_lossy().into_owned()).unwrap_or_else(|_| path.to_owned())
}

/// Whether an emulator with this executable (or flatpak id) is configured.
pub fn is_configured(existing: &[UserEmulator], executable: &str, flatpak_id: Option<&str>) -> bool {
    if let Some(id) = flatpak_id {
        return existing.iter().any(|e| e.launch_kind == "flatpak" && e.flatpak_id.as_deref() == Some(id));
    }
    let key = canonical(executable);
    existing
        .iter()
        .filter(|e| e.launch_kind != "flatpak")
        .any(|e| e.executable == executable || (!e.executable.is_empty() && canonical(&e.executable) == key))
}

fn detected_from(
    env: &dyn DetectEnv,
    c: &Candidate,
    existing: &[UserEmulator],
    source: &str,
) -> Option<DetectedEmulator> {
    let launch_kind = if c.wine { "wine" } else { "native" };
    let profiles: Vec<DetectedEmulatorProfile> = match RULES.iter().find(|r| r.name == c.name).and_then(|r| r.custom) {
        Some(p) => vec![DetectedEmulatorProfile {
            id: p.id.into(),
            name: p.name.into(),
            platform_name: p.platform_name.into(),
            core_path: None,
            platform_id: Some(p.platform_id.into()),
            arguments: Some(p.arguments.into()),
            extensions: Some(p.extensions.iter().map(|s| s.to_string()).collect()),
        }],
        None => existing_profiles(env, c.definition_id, &c.path, "native"),
    };
    if profiles.is_empty() {
        return None;
    }
    Some(DetectedEmulator {
        definition_id: c.definition_id.into(),
        name: c.name.into(),
        executable: c.path.clone(),
        launch_kind: launch_kind.into(),
        flatpak_id: None,
        profiles,
        already_added: is_configured(existing, &c.path, None),
        source: source.into(),
    })
}

fn definition_order(id: &str) -> usize {
    emulator_definitions().iter().position(|d| d.id == id).unwrap_or(usize::MAX)
}

/// Merges `extra` into `found` skipping executables already listed (same
/// file, symlinks resolved) and orders the list by catalog, then name.
fn merge(found: &mut Vec<DetectedEmulator>, extra: Vec<DetectedEmulator>) {
    let mut seen: HashSet<String> =
        found.iter().filter(|d| d.launch_kind != "flatpak").map(|d| canonical(&d.executable)).collect();
    for d in extra {
        if d.launch_kind == "flatpak" || seen.insert(canonical(&d.executable)) {
            found.push(d);
        }
    }
    found.sort_by(|a, b| {
        definition_order(&a.definition_id)
            .cmp(&definition_order(&b.definition_id))
            .then_with(|| a.name.cmp(&b.name))
            // Newer versions (larger names) first.
            .then_with(|| basename(&b.executable).cmp(&basename(&a.executable)))
    });
}

/// Everything detectable without the user choosing a folder: `$PATH`,
/// Flatpak, and the usual places. `already_added` marks configured ones.
pub fn discover_emulators(env: &dyn DetectEnv, existing: &[UserEmulator]) -> Vec<DetectedEmulator> {
    discover_in_roots(env, existing, &common_search_roots(&env.home(), existing))
}

/// [`discover_emulators`] with explicit search folders.
pub fn discover_in_roots(
    env: &dyn DetectEnv,
    existing: &[UserEmulator],
    roots: &[SearchRoot],
) -> Vec<DetectedEmulator> {
    let mut found = detect_emulators(env, existing);
    for d in &mut found {
        d.already_added = is_configured(existing, &d.executable, d.flatpak_id.as_deref());
    }
    let mut extra = Vec::new();
    for root in roots {
        let limits = WalkLimits { max_depth: root.depth, ..Default::default() };
        for c in find_candidates(Path::new(&root.path), limits) {
            extra.extend(detected_from(env, &c, existing, &root.label));
        }
    }
    merge(&mut found, extra);
    found
}

/// "Scan a folder for emulators": searches `folder` recursively (bounded).
pub fn discover_in_folder(env: &dyn DetectEnv, folder: &str, existing: &[UserEmulator]) -> Vec<DetectedEmulator> {
    let limits = WalkLimits { max_depth: 6, ..Default::default() };
    let mut found = Vec::new();
    let extra: Vec<DetectedEmulator> = find_candidates(Path::new(folder), limits)
        .iter()
        .filter_map(|c| detected_from(env, c, existing, "chosen folder"))
        .collect();
    merge(&mut found, extra);
    found
}

/// Registers the chosen detections as new emulators with their detected
/// profiles. Ones already configured (re-checked against the store) are
/// skipped; a repeated name gets the file name appended. Returns the
/// emulators added.
pub fn add_discovered(paths: &DataPaths, items: &[DetectedEmulator]) -> Result<Vec<UserEmulator>> {
    let mut added = Vec::new();
    for item in items {
        let current = store::list_user_emulators(paths)?;
        if is_configured(&current, &item.executable, item.flatpak_id.as_deref()) {
            continue;
        }
        let name = if current.iter().any(|e| e.name == item.name) && item.launch_kind != "flatpak" {
            let file = basename(&item.executable);
            let stem = file.strip_suffix(".AppImage").or_else(|| file.strip_suffix(".appimage")).unwrap_or(&file);
            format!("{} ({stem})", item.name)
        } else {
            item.name.clone()
        };
        let def = emulator_definition(&item.definition_id);
        let profiles: Vec<UserEmulatorProfile> = item
            .profiles
            .iter()
            .map(|p| {
                let cat = def.and_then(|d| d.profiles.iter().find(|c| c.id == p.id));
                UserEmulatorProfile {
                    id: p.id.clone(),
                    definition_profile_id: Some(p.id.clone()),
                    name: p.name.clone(),
                    platform_id: p.platform_id.clone().or_else(|| cat.map(|c| c.platform_id.clone())),
                    platform_name: Some(p.platform_name.clone()),
                    arguments: p
                        .arguments
                        .clone()
                        .or_else(|| cat.map(|c| c.arguments.clone()))
                        .unwrap_or_else(|| "{ImagePath}".into()),
                    extensions: p
                        .extensions
                        .clone()
                        .or_else(|| cat.map(|c| c.extensions.clone()))
                        .unwrap_or_default(),
                    core_path: p.core_path.clone(),
                    folder_indicator: cat.and_then(|c| c.folder_indicator.clone()),
                    playlist_extensions: cat.and_then(|c| c.playlist_extensions.clone()),
                    working_dir: cat.and_then(|c| c.working_dir.clone()),
                    extra: Default::default(),
                }
            })
            .collect();
        added.push(insert_user_emulator(
            paths,
            UserEmulator {
                id: String::new(),
                definition_id: item.definition_id.clone(),
                name,
                executable: item.executable.clone(),
                install_dir: None,
                launch_kind: item.launch_kind.clone(),
                flatpak_id: item.flatpak_id.clone(),
                playnite_id: None,
                profiles,
                extra: Default::default(),
            },
        )?);
    }
    Ok(added)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{HashMap, HashSet};
    use std::fs;

    struct Env {
        home: String,
    }
    impl DetectEnv for Env {
        fn which(&self, _: &str) -> Option<String> {
            None
        }
        fn flatpak_installed(&self, _: &str) -> bool {
            false
        }
        fn exists(&self, path: &str) -> bool {
            Path::new(path).exists()
        }
        fn read_dir(&self, dir: &str) -> Vec<String> {
            fs::read_dir(dir)
                .map(|r| r.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect())
                .unwrap_or_default()
        }
        fn home(&self) -> String {
            self.home.clone()
        }
    }

    fn touch(path: &Path, exec: bool) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, "x").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(path, fs::Permissions::from_mode(if exec { 0o755 } else { 0o644 })).unwrap();
        }
    }

    #[test]
    fn identifies_names() {
        let id = |n: &str| identify(n).map(|r| r.name);
        assert_eq!(id("pcsx2-v2.8.2.AppImage"), Some("PCSX2"));
        assert_eq!(id("pcsx2-qt"), Some("PCSX2"));
        assert_eq!(id("DuckStation.AppImage"), Some("DuckStation"));
        assert_eq!(id("PPSSPPSDL"), Some("PPSSPP"));
        assert_eq!(id("rpcs3-v0.0.42-20069.AppImage"), Some("RPCS3"));
        assert_eq!(id("shadPS4QtLauncher-qt.AppImage"), Some("shadPS4"));
        assert_eq!(id("Shadps4-sdl.AppImage"), Some("shadPS4"));
        assert_eq!(id("RetroArch-Linux-x86_64.AppImage"), Some("RetroArch"));
        assert_eq!(id("dolphin-emu"), Some("Dolphin"));
        assert_eq!(id("Dolphin_Emulator-x86_64.AppImage"), Some("Dolphin"));
        assert_eq!(id("yuzu-mainline.AppImage"), Some("Yuzu"));
        assert_eq!(id("Eden-Linux.AppImage"), Some("Eden"));
        assert_eq!(id("mGBA-0.10.3.AppImage"), Some("mGBA"));
        assert_eq!(id("Cemu-2.0.AppImage"), Some("Cemu"));
        // KDE's file manager and unrelated names are not emulators.
        assert_eq!(id("dolphin"), None);
        assert_eq!(id("edentool"), None);
        assert_eq!(id("mamehub"), None);
        assert_eq!(id("firefox"), None);
    }

    #[test]
    fn walk_finds_executables_and_skips_game_dirs() {
        let d = tempfile::tempdir().unwrap();
        let r = d.path();
        touch(&r.join("PS2/emulator/pcsx2-v2.8.2.AppImage"), false);
        touch(&r.join("PS1/emulator/DuckStation.AppImage"), true);
        touch(&r.join("PS4/emulator/Shadps4-sdl.AppImage"), true);
        touch(&r.join("tools/ryujinx/Ryujinx"), true);
        // Not executable, not an AppImage: ignored.
        touch(&r.join("tools/cemu/Cemu"), false);
        // Documentation next to an emulator.
        touch(&r.join("PS2/emulator/pcsx2.txt"), true);
        // Game folders are never entered.
        touch(&r.join("PS2/games/pcsx2-demo.AppImage"), true);
        // Too deep.
        touch(&r.join("a/b/c/d/e/f/g/xemu-v1.AppImage"), true);
        let mut names: Vec<String> = find_candidates(r, WalkLimits { max_depth: 4, ..Default::default() })
            .iter()
            .map(|c| basename(&c.path))
            .collect();
        names.sort();
        assert_eq!(names, ["DuckStation.AppImage", "Ryujinx", "Shadps4-sdl.AppImage", "pcsx2-v2.8.2.AppImage"]);
    }

    #[test]
    fn huge_directories_are_skipped() {
        let d = tempfile::tempdir().unwrap();
        for i in 0..12 {
            touch(&d.path().join(format!("lib/file{i}.bin")), false);
        }
        touch(&d.path().join("lib/pcsx2.AppImage"), true);
        let found = find_candidates(d.path(), WalkLimits { max_dir_entries: 10, ..Default::default() });
        assert!(found.is_empty());
        let found = find_candidates(d.path(), WalkLimits::default());
        assert_eq!(found.len(), 1);
    }

    #[test]
    fn discovery_marks_configured_and_builds_profiles() {
        let d = tempfile::tempdir().unwrap();
        let home = d.path().to_str().unwrap().to_owned();
        let pcsx2 = d.path().join("Applications/pcsx2-v2.8.2.AppImage");
        touch(&pcsx2, true);
        touch(&d.path().join("Applications/yuzu.AppImage"), true);
        touch(&d.path().join("Games/Emu/PS3/rpcs3.AppImage"), true);
        // RetroArch without cores is not listed.
        touch(&d.path().join("Applications/RetroArch.AppImage"), true);
        let existing = vec![UserEmulator {
            id: "e1".into(),
            definition_id: "custom".into(),
            name: "PCSX2".into(),
            executable: pcsx2.to_str().unwrap().into(),
            launch_kind: "native".into(),
            ..Default::default()
        }];
        let env = Env { home: home.clone() };
        let discover = |existing: &[UserEmulator]| {
            let roots: Vec<SearchRoot> =
                common_search_roots(&home, existing).into_iter().filter(|r| r.path.starts_with(&home)).collect();
            discover_in_roots(&env, existing, &roots)
        };
        let found = discover(&existing);
        let by: HashMap<&str, &DetectedEmulator> = found.iter().map(|d| (d.name.as_str(), d)).collect();
        assert_eq!(found.len(), 3, "{found:#?}");
        // Configured as a custom emulator: still recognised as configured.
        assert!(by["PCSX2"].already_added);
        assert_eq!(by["PCSX2"].profiles[0].id, "ps2");
        assert!(!by["RPCS3"].already_added);
        let yuzu = by["Yuzu"];
        assert_eq!(yuzu.definition_id, "custom");
        assert_eq!(yuzu.profiles[0].arguments.as_deref(), Some("-f -g {ImagePath}"));
        // Catalog order.
        let order: Vec<&str> = found.iter().map(|d| d.name.as_str()).collect();
        assert_eq!(order, ["PCSX2", "RPCS3", "Yuzu"]);
        // A core next to the AppImage makes RetroArch usable.
        touch(&d.path().join("Applications/cores/snes9x_libretro.so"), false);
        let found = discover(&existing);
        assert!(found.iter().any(|d| d.name == "RetroArch" && d.profiles[0].id == "snes"));
    }

    #[test]
    fn configured_folders_are_searched() {
        let d = tempfile::tempdir().unwrap();
        let root = d.path().join("elsewhere/PS2/emulator");
        touch(&root.join("pcsx2.AppImage"), true);
        touch(&d.path().join("elsewhere/PS1/emulator/duckstation-qt.AppImage"), true);
        let existing = vec![UserEmulator {
            id: "e1".into(),
            definition_id: "custom".into(),
            executable: root.join("pcsx2.AppImage").to_str().unwrap().into(),
            launch_kind: "native".into(),
            ..Default::default()
        }];
        let roots = common_search_roots("/nowhere", &existing);
        assert!(roots.iter().any(|r| r.path == root.to_str().unwrap()));
        let roots: Vec<SearchRoot> = common_search_roots("/nowhere", &existing)
            .into_iter()
            .filter(|r| r.path.starts_with(d.path().to_str().unwrap()))
            .collect();
        let found = discover_in_roots(&Env { home: "/nowhere".into() }, &existing, &roots);
        // Sibling emulator folders (`.../PS1/emulator`) are searched too.
        let names: HashSet<_> = found.iter().map(|d| d.name.as_str()).collect();
        assert_eq!(names, HashSet::from(["PCSX2", "DuckStation"]));
        let in_folder = discover_in_folder(&Env { home: "/nowhere".into() }, d.path().join("elsewhere").to_str().unwrap(), &existing);
        let names: HashSet<_> = in_folder.iter().map(|d| d.name.as_str()).collect();
        assert_eq!(names, HashSet::from(["PCSX2", "DuckStation"]));
        assert!(in_folder.iter().all(|d| d.source == "chosen folder"));
    }

    #[test]
    fn add_discovered_registers_several_and_skips_configured() {
        let h = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(h.path());
        let d = tempfile::tempdir().unwrap();
        touch(&d.path().join("a/pcsx2-v2.8.AppImage"), true);
        touch(&d.path().join("b/pcsx2-v2.7.AppImage"), true);
        touch(&d.path().join("yuzu.AppImage"), true);
        let env = Env { home: "/nowhere".into() };
        let found = discover_in_folder(&env, d.path().to_str().unwrap(), &[]);
        assert_eq!(found.len(), 3);
        let added = add_discovered(&paths, &found).unwrap();
        assert_eq!(added.len(), 3);
        let names: Vec<_> = added.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["PCSX2", "PCSX2 (pcsx2-v2.7)", "Yuzu"]);
        assert_eq!(added[0].profiles[0].arguments, "-fullscreen -slowboot -- {ImagePath}");
        assert_eq!(added[2].definition_id, "custom");
        assert_eq!(added[2].profiles[0].extensions, ["nsp", "xci", "nca", "nro", "nso"]);
        // Second run: everything is configured.
        assert!(add_discovered(&paths, &found).unwrap().is_empty());
        assert_eq!(store::list_user_emulators(&paths).unwrap().len(), 3);
        let again = discover_in_folder(&env, d.path().to_str().unwrap(), &store::list_user_emulators(&paths).unwrap());
        assert!(again.iter().all(|d| d.already_added));
    }
}
