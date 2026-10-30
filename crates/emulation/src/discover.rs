//! Finding emulators that are not on `$PATH`: AppImages and binaries in the
//! usual places (`~/Applications`, `~/.local/bin`, `~/Games`, `/opt`, the
//! folders of emulators already configured) or in a folder the user picks,
//! matched against the catalog by file name. On Windows, `.exe` files in
//! the catalog's install folders (Program Files, `%LOCALAPPDATA%\Programs`,
//! `%APPDATA%`, the system drive), Scoop's and EmuDeck's apps and the usual
//! folders of the home.
//!
//! Nothing here executes a file: matching is by name and permission bits only.

use std::collections::HashSet;
use std::path::{Component, Path};
use std::time::{Duration, Instant};

use anyhow::Result;
use heronite_data::DataPaths;
use heronite_launch::emulators::{UserEmulator, UserEmulatorProfile};

use crate::catalog::{emulator_definition, emulator_definitions};
use crate::detect::{detect_emulators, drives, existing_profiles, scoop_apps, windows_install_bases, DetectEnv};
use crate::pathutil::{basename, dirname, has_dir, join, join_all};
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
/// `shadPS4QtLauncher`, `PPSSPPWindows64`).
const SUFFIXES: &[&str] = &[
    "qt",
    "sdl",
    "sdl2",
    "nogui",
    "emu",
    "gui",
    "qtlauncher",
    "launcher",
    "x64",
    "x8664",
    "linux",
    "appimage",
    "nightly",
    "mgr",
    "windows64",
];

/// Words of the other programs Windows emulators ship next to themselves
/// (`retroarch_debug.exe`, `azahar-room.exe`, `Ryujinx.Headless.SDL2.exe`).
const HELPERS: &[&str] = &["debug", "headless", "room", "updater", "uninstall", "setup", "installer", "crashpad", "crashhandler"];

const NOT_EXECUTABLE_EXT: &[&str] = &[
    "so", "txt", "json", "ini", "png", "jpg", "svg", "desktop", "log", "md", "zip", "7z", "gz", "xz", "bz2", "tar", "dll", "yml", "yaml",
    "cfg", "conf", "bak", "sig", "sha256", "zsync", "pdb", "xml", "ico", "pkg", "iso", "bin", "chd", "cue",
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
    // Windows has no KDE: its `Dolphin.exe` is the emulator.
    let unambiguous = is_appimage(file_name) || (cfg!(windows) && lower.ends_with(".exe"));
    if lower.ends_with(".exe") && tokens.iter().any(|t| HELPERS.contains(t)) {
        return None;
    }
    for rule in RULES {
        for alias in rule.aliases.iter().chain(rule.appimage_only) {
            if rule.appimage_only.contains(alias) && !unambiguous {
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
    "games",
    "roms",
    "rom",
    "saves",
    "save",
    "savestates",
    "states",
    "bios",
    "firmware",
    "pkg",
    "media",
    "covers",
    "art",
    "screenshots",
    "shaders",
    "textures",
    "cache",
    "logs",
    "log",
    "dump",
    "dumps",
    "node_modules",
    "squashfs-root",
    "mods",
    "cheats",
    "themes",
    "sysdata",
    "dev_hdd0",
    "steamapps",
    "compatdata",
    "backups",
    "playlists",
    "thumbnails",
    "cores",
    "system",
    "assets",
    "icons",
    "locales",
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
    // Windows runs `.exe` files only.
    if cfg!(windows) && !exe {
        return;
    }
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
    // A Windows program needs Wine on Linux; Windows runs it as it is.
    let wine = exe && !cfg!(windows);
    out.push(Candidate { path: path.to_string_lossy().into_owned(), definition_id: rule.definition, name: rule.name, wine });
}

/// A folder to search with its label (shown as the result's source).
#[derive(Debug, Clone, PartialEq)]
pub struct SearchRoot {
    pub path: String,
    pub depth: usize,
    pub label: String,
}

/// The usual places plus the folders of emulators already configured.
pub fn common_search_roots(env: &dyn DetectEnv, existing: &[UserEmulator]) -> Vec<SearchRoot> {
    let home = env.home();
    let mut roots: Vec<SearchRoot> = Vec::new();
    let mut add = |path: String, depth: usize, label: &str| {
        if !roots.iter().any(|r| r.path == path) {
            roots.push(SearchRoot { path, depth, label: label.to_owned() });
        }
    };
    if cfg!(windows) {
        windows_roots(env, &home, &mut add);
    } else {
        linux_roots(&home, &mut add);
    }
    for e in existing {
        if e.launch_kind == "flatpak" || !has_dir(&e.executable) {
            continue;
        }
        let dir = dirname(&e.executable);
        let parent = dirname(&dir);
        let grand = dirname(&parent);
        add(dir.clone(), 3, "next to a configured emulator");
        // Scoop keeps an app's versions side by side (`apps\<app>\<version>`);
        // `current` is the one in use, its siblings are old copies.
        if cfg!(windows) && basename(&dir).eq_ignore_ascii_case("current") {
            continue;
        }
        // Emulators are often kept one per folder (`Emulation/PS1/emulator`,
        // `Emulation/PS2/emulator`): look at the siblings too, unless that
        // would mean searching `$HOME` or a top-level folder.
        let deep_enough = |p: &str| Path::new(p).components().filter(|c| matches!(c, Component::Normal(_))).count() >= 2 && p != home;
        if deep_enough(&grand) {
            add(grand, 3, "next to a configured emulator");
        } else if parent != dir && deep_enough(&parent) {
            add(parent, 1, "next to a configured emulator");
        }
    }
    roots
}

fn linux_roots(home: &str, add: &mut dyn FnMut(String, usize, &str)) {
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
        add(join(home, rel), depth, &format!("~/{rel}"));
    }
    for (abs, depth) in [("/opt", 3), ("/usr/local/bin", 1), ("/usr/games", 1)] {
        add(abs.to_owned(), depth, abs);
    }
}

/// Windows: the catalog's install folders (`Program Files\PCSX2`...), the
/// apps of Scoop (`<app>\current`, the version it keeps up to date) and
/// EmuDeck, and folders of the home and of each drive.
fn windows_roots(env: &dyn DetectEnv, home: &str, add: &mut dyn FnMut(String, usize, &str)) {
    let folders: Vec<&str> = emulator_definitions().iter().flat_map(|d| d.install_folders.iter().map(String::as_str)).collect();
    for (base, label) in windows_install_bases(env) {
        for folder in &folders {
            let dir = join(&base, folder);
            if env.exists(&dir) {
                add(dir, 2, &label);
            }
        }
    }
    for apps in scoop_apps(env) {
        let mut names = env.read_dir(&apps);
        names.sort();
        for app in names.iter().filter(|a| folders.iter().any(|f| f.eq_ignore_ascii_case(a)) || identify(a).is_some()) {
            add(join_all(&apps, &[app, "current"]), 1, "Scoop");
        }
    }
    if let Some(appdata) = env.var("APPDATA") {
        add(join_all(&appdata, &["EmuDeck", "Emulators"]), 3, "EmuDeck");
    }
    for (rel, depth) in [("Emulation", 4), ("Emulators", 4), ("Games", 3), ("Apps", 3), ("Downloads", 1)] {
        add(join(home, rel), depth, &format!(r"~\{rel}"));
    }
    for drive in drives(env) {
        for (rel, depth) in [("Emulation", 4), ("Emulators", 4), ("Games", 3)] {
            let dir = join(&drive, rel);
            add(dir.clone(), depth, &dir);
        }
    }
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

fn detected_from(env: &dyn DetectEnv, c: &Candidate, existing: &[UserEmulator], source: &str) -> Option<DetectedEmulator> {
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
    let mut seen: HashSet<String> = found.iter().filter(|d| d.launch_kind != "flatpak").map(|d| canonical(&d.executable)).collect();
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
    discover_in_roots(env, existing, &common_search_roots(env, existing))
}

/// [`discover_emulators`] with explicit search folders.
pub fn discover_in_roots(env: &dyn DetectEnv, existing: &[UserEmulator], roots: &[SearchRoot]) -> Vec<DetectedEmulator> {
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
    let extra: Vec<DetectedEmulator> =
        find_candidates(Path::new(folder), limits).iter().filter_map(|c| detected_from(env, c, existing, "chosen folder")).collect();
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
            let stem = file
                .strip_suffix(".AppImage")
                .or_else(|| file.strip_suffix(".appimage"))
                .or_else(|| file.strip_suffix(".exe").filter(|_| cfg!(windows)))
                .unwrap_or(&file);
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
                    arguments: p.arguments.clone().or_else(|| cat.map(|c| c.arguments.clone())).unwrap_or_else(|| "{ImagePath}".into()),
                    extensions: p.extensions.clone().or_else(|| cat.map(|c| c.extensions.clone())).unwrap_or_default(),
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

    #[derive(Default)]
    struct Env {
        home: String,
        vars: HashMap<String, String>,
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
            fs::read_dir(dir).map(|r| r.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect()).unwrap_or_default()
        }
        fn home(&self) -> String {
            self.home.clone()
        }
        fn var(&self, key: &str) -> Option<String> {
            self.vars.get(key).cloned()
        }
    }

    fn env(home: &str) -> Env {
        Env { home: home.into(), ..Default::default() }
    }

    fn touch(path: &Path, exec: bool) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, "x").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(path, fs::Permissions::from_mode(if exec { 0o755 } else { 0o644 })).unwrap();
        }
        #[cfg(not(unix))]
        let _ = exec;
    }

    /// An emulator file found whatever its permission bits: an AppImage,
    /// on Windows an `.exe`.
    fn program(stem: &str) -> String {
        if cfg!(windows) {
            format!("{stem}.exe")
        } else {
            format!("{stem}.AppImage")
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
        // Windows builds.
        assert_eq!(id("pcsx2-qt.exe"), Some("PCSX2"));
        assert_eq!(id("pcsx2-qtx64-avx2.exe"), Some("PCSX2"));
        assert_eq!(id("duckstation-qt-x64-ReleaseLTCG.exe"), Some("DuckStation"));
        assert_eq!(id("PPSSPPWindows64.exe"), Some("PPSSPP"));
        assert_eq!(id("Ryujinx.exe"), Some("Ryujinx"));
        assert_eq!(id("Vita3K.exe"), Some("Vita3K"));
        assert_eq!(id("citra-qt.exe"), Some("Azahar"));
        // Only Windows' `Dolphin.exe` is sure to be the emulator.
        assert_eq!(id("Dolphin.exe"), cfg!(windows).then_some("Dolphin"));
        // The other programs shipped next to an emulator.
        assert_eq!(id("retroarch_debug.exe"), None);
        assert_eq!(id("azahar-room.exe"), None);
        assert_eq!(id("Ryujinx.Headless.SDL2.exe"), None);
        assert_eq!(id("DolphinTool.exe"), None);
    }

    // Execute bits are Linux's: on Windows only `.exe` files run.
    #[cfg(unix)]
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
        let mut names: Vec<String> =
            find_candidates(r, WalkLimits { max_depth: 4, ..Default::default() }).iter().map(|c| basename(&c.path)).collect();
        names.sort();
        assert_eq!(names, ["DuckStation.AppImage", "Ryujinx", "Shadps4-sdl.AppImage", "pcsx2-v2.8.2.AppImage"]);
    }

    #[cfg(windows)]
    #[test]
    fn walk_finds_windows_programs_and_skips_game_dirs() {
        let d = tempfile::tempdir().unwrap();
        let r = d.path();
        touch(&r.join(r"PS2\PCSX2\pcsx2-qt.exe"), false);
        touch(&r.join(r"PS1\duckstation\duckstation-qt-x64-ReleaseLTCG.exe"), false);
        touch(&r.join(r"GC\Dolphin-x64\Dolphin.exe"), false);
        // Its tools and files: ignored.
        touch(&r.join(r"GC\Dolphin-x64\DolphinTool.exe"), false);
        touch(&r.join(r"PS2\PCSX2\pcsx2-qt.pdb"), false);
        touch(&r.join(r"tools\ryujinx\Ryujinx"), false);
        touch(&r.join(r"tools\RetroArch-Win64\retroarch_debug.exe"), false);
        // Game folders are never entered.
        touch(&r.join(r"PS2\games\pcsx2-demo.exe"), false);
        let found = find_candidates(r, WalkLimits { max_depth: 4, ..Default::default() });
        let mut names: Vec<String> = found.iter().map(|c| basename(&c.path)).collect();
        names.sort();
        assert_eq!(names, ["Dolphin.exe", "duckstation-qt-x64-ReleaseLTCG.exe", "pcsx2-qt.exe"]);
        assert!(found.iter().all(|c| !c.wine), "Windows runs them as they are");
    }

    #[test]
    fn huge_directories_are_skipped() {
        let d = tempfile::tempdir().unwrap();
        for i in 0..12 {
            touch(&d.path().join(format!("lib/file{i}.bin")), false);
        }
        touch(&d.path().join("lib").join(program("pcsx2")), true);
        let found = find_candidates(d.path(), WalkLimits { max_dir_entries: 10, ..Default::default() });
        assert!(found.is_empty());
        let found = find_candidates(d.path(), WalkLimits::default());
        assert_eq!(found.len(), 1);
    }

    #[test]
    fn discovery_marks_configured_and_builds_profiles() {
        let d = tempfile::tempdir().unwrap();
        let home = d.path().to_str().unwrap().to_owned();
        // One of the usual folders of the home.
        let apps = d.path().join(if cfg!(windows) { "Emulators" } else { "Applications" });
        let pcsx2 = apps.join(program("pcsx2-v2.8.2"));
        touch(&pcsx2, true);
        touch(&apps.join(program("yuzu")), true);
        touch(&d.path().join("Games/Emu/PS3").join(program("rpcs3")), true);
        // RetroArch without cores is not listed.
        touch(&apps.join(program("RetroArch")), true);
        let existing = vec![UserEmulator {
            id: "e1".into(),
            definition_id: "custom".into(),
            name: "PCSX2".into(),
            executable: pcsx2.to_str().unwrap().into(),
            launch_kind: "native".into(),
            ..Default::default()
        }];
        let env = env(&home);
        let discover = |existing: &[UserEmulator]| {
            let roots: Vec<SearchRoot> = common_search_roots(&env, existing).into_iter().filter(|r| r.path.starts_with(&home)).collect();
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
        // A core next to the emulator makes RetroArch usable.
        touch(&apps.join("cores").join(if cfg!(windows) { "snes9x_libretro.dll" } else { "snes9x_libretro.so" }), false);
        let found = discover(&existing);
        assert!(found.iter().any(|d| d.name == "RetroArch" && d.profiles[0].id == "snes"));
    }

    #[test]
    fn configured_folders_are_searched() {
        let d = tempfile::tempdir().unwrap();
        let root = d.path().join("elsewhere/PS2/emulator");
        touch(&root.join(program("pcsx2")), true);
        touch(&d.path().join("elsewhere/PS1/emulator").join(program("duckstation-qt")), true);
        let existing = vec![UserEmulator {
            id: "e1".into(),
            definition_id: "custom".into(),
            executable: root.join(program("pcsx2")).to_str().unwrap().into(),
            launch_kind: "native".into(),
            ..Default::default()
        }];
        let nowhere = env("/nowhere");
        let roots = common_search_roots(&nowhere, &existing);
        assert!(roots.iter().any(|r| r.path == root.to_str().unwrap()));
        let roots: Vec<SearchRoot> =
            common_search_roots(&nowhere, &existing).into_iter().filter(|r| r.path.starts_with(d.path().to_str().unwrap())).collect();
        let found = discover_in_roots(&nowhere, &existing, &roots);
        // Sibling emulator folders (`.../PS1/emulator`) are searched too.
        let names: HashSet<_> = found.iter().map(|d| d.name.as_str()).collect();
        assert_eq!(names, HashSet::from(["PCSX2", "DuckStation"]));
        let in_folder = discover_in_folder(&nowhere, d.path().join("elsewhere").to_str().unwrap(), &existing);
        let names: HashSet<_> = in_folder.iter().map(|d| d.name.as_str()).collect();
        assert_eq!(names, HashSet::from(["PCSX2", "DuckStation"]));
        assert!(in_folder.iter().all(|d| d.source == "chosen folder"));
    }

    #[test]
    fn add_discovered_registers_several_and_skips_configured() {
        let h = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(h.path());
        let d = tempfile::tempdir().unwrap();
        touch(&d.path().join("a").join(program("pcsx2-v2.8")), true);
        touch(&d.path().join("b").join(program("pcsx2-v2.7")), true);
        touch(&d.path().join(program("yuzu")), true);
        let env = env("/nowhere");
        let found = discover_in_folder(&env, d.path().to_str().unwrap(), &[]);
        assert_eq!(found.len(), 3);
        assert!(found.iter().all(|d| d.launch_kind == "native"));
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

    #[cfg(windows)]
    #[test]
    fn windows_install_folders_scoop_and_emudeck_are_searched() {
        let d = tempfile::tempdir().unwrap();
        let at = |rest: &str| d.path().join(rest);
        let var = |k: &str, rest: &str| (k.to_string(), at(rest).to_string_lossy().into_owned());
        let mut env = env(at("home").to_str().unwrap());
        env.vars = HashMap::from([
            var("ProgramFiles", "Program Files"),
            var("ProgramFiles(x86)", "Program Files (x86)"),
            var("LOCALAPPDATA", r"home\AppData\Local"),
            var("APPDATA", r"home\AppData\Roaming"),
            var("SystemDrive", "drive"),
        ]);
        touch(&at(r"Program Files\PCSX2\pcsx2-qt.exe"), false);
        touch(&at(r"Program Files (x86)\PPSSPP\PPSSPPWindows64.exe"), false);
        touch(&at(r"home\AppData\Local\Programs\Ryujinx\publish\Ryujinx.exe"), false);
        touch(&at(r"drive\RetroArch-Win64\retroarch.exe"), false);
        touch(&at(r"drive\RetroArch-Win64\cores\snes9x_libretro.dll"), false);
        // Scoop: only the version it keeps up to date (`current`).
        touch(&at(r"home\scoop\apps\duckstation\current\duckstation-qt-x64-ReleaseLTCG.exe"), false);
        touch(&at(r"home\scoop\apps\duckstation\0.1-7\duckstation-qt-x64-ReleaseLTCG.exe"), false);
        touch(&at(r"home\scoop\apps\firefox\current\firefox.exe"), false);
        touch(&at(r"home\AppData\Roaming\EmuDeck\Emulators\Dolphin-x64\Dolphin.exe"), false);
        touch(&at(r"home\Downloads\melonDS-windows\melonDS.exe"), false);
        // Somewhere nobody looks.
        touch(&at(r"elsewhere\xemu\xemu.exe"), false);
        let found = discover_emulators(&env, &[]);
        let by: HashMap<&str, (&str, &str)> = found.iter().map(|d| (d.name.as_str(), (d.executable.as_str(), d.source.as_str()))).collect();
        let s = |rest: &str| at(rest).to_string_lossy().into_owned();
        assert_eq!(by["PCSX2"], (s(r"Program Files\PCSX2\pcsx2-qt.exe").as_str(), "Program Files"));
        assert_eq!(by["PPSSPP"].1, "Program Files (x86)");
        assert_eq!(by["Ryujinx"].1, r"AppData\Local\Programs");
        assert_eq!(by["RetroArch"].1, s(r"drive\").as_str());
        assert_eq!(by["DuckStation"], (s(r"home\scoop\apps\duckstation\current\duckstation-qt-x64-ReleaseLTCG.exe").as_str(), "Scoop"));
        assert_eq!(by["Dolphin"].1, "EmuDeck");
        assert_eq!(by["melonDS"].1, r"~\Downloads");
        assert_eq!(found.len(), 7, "{found:#?}");
        assert!(found.iter().all(|d| d.launch_kind == "native" && d.flatpak_id.is_none()));
        let ra = found.iter().find(|d| d.name == "RetroArch").unwrap();
        assert_eq!(ra.profiles[0].core_path.as_deref(), Some(s(r"drive\RetroArch-Win64\cores\snes9x_libretro.dll").as_str()));
        // Once configured, the old versions next to Scoop's `current` stay out.
        let duck = found.iter().find(|d| d.name == "DuckStation").unwrap();
        let configured = vec![UserEmulator {
            id: "d".into(),
            definition_id: "duckstation".into(),
            executable: duck.executable.clone(),
            launch_kind: "native".into(),
            ..Default::default()
        }];
        let again = discover_emulators(&env, &configured);
        assert!(again.iter().all(|d| !d.executable.contains("0.1-7")), "{again:#?}");
    }
}
