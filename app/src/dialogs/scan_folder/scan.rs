//! Finding the games in a folder: walks it on a background thread (a few
//! levels deep, links to folders not followed), keeps the programs that may
//! start a game and groups them by the game's folder, best program first.
//! No GPUI: the dialog calls [`scan`] off the UI thread.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs::{self, Metadata};
use std::io::Read;
use std::path::{Path, PathBuf};

use crate::dialogs::add_game::dropped::{title_from_executable, title_from_name, GENERIC_DIRS, GENERIC_STEMS};

/// Folder levels below the chosen one that are searched (Unreal games keep
/// their program in `Game/Name/Binaries/Win64`).
const MAX_DEPTH: usize = 6;
/// Entries read before the scan stops: a whole drive picked by mistake.
const MAX_ENTRIES: usize = 150_000;
/// A folder with programs this many levels inside another one belongs to
/// that game (`Game/Name/Binaries/Win64`, `Game/Tools/Editor`).
const NEST_LEVELS: usize = 2;
/// A folder that would take in more folders with programs than this is a
/// folder of games (one with a stray program), not one game.
const MAX_NESTED: usize = 2;
/// Programs offered per game.
const MAX_PROGRAMS: usize = 8;

/// Folders never searched (lower case): redistributables, anti-cheat
/// installers, runtimes bundled with games, Windows' own folders.
const SKIP_DIRS: [&str; 44] = [
    "$recycle.bin",
    "system volume information",
    "dosdevices",
    "node_modules",
    "__pycache__",
    "_commonredist",
    "commonredist",
    "redist",
    "_redist",
    "redistributable",
    "redistributables",
    "directx",
    "vcredist",
    "dotnet",
    "dotnetfx",
    "prerequisites",
    "_prerequisites",
    "__installer",
    "installer",
    "installers",
    "__support",
    "easyanticheat",
    "battleye",
    "monobleedingedge",
    "jre",
    "jdk",
    "steam-runtime",
    "steamworks shared",
    "engine",
    "crashreporter",
    "common files",
    "windows nt",
    "internet explorer",
    "windows media player",
    "windows defender",
    "windows mail",
    "windows photo viewer",
    "windows sidebar",
    "windowspowershell",
    "microsoft.net",
    "appdata",
    "lib",
    "lib32",
    "lib64",
];

/// Starts of program names that never start a game (lower case).
const NOT_GAME_PREFIXES: [&str; 47] = [
    "unins",
    "setup",
    "install",
    "vc_redist",
    "vcredist",
    "dxsetup",
    "dxwebsetup",
    "dotnet",
    "ndp4",
    "directx",
    "oalinst",
    "physx",
    "ue4prereq",
    "ueprereq",
    "crashreport",
    "crashhandler",
    "unitycrashhandler",
    "crashpad",
    "crash_reporter",
    "crs-handler",
    "bssndrpt",
    "easyanticheat",
    "beservice",
    "battleye",
    "redist",
    "notification_helper",
    "elevate",
    "7z",
    "unrar",
    "update",
    "patcher",
    "cefsharp",
    "qtwebengine",
    "steamerrorreporter",
    "gameoverlayui",
    "steamwebhelper",
    "chrome-sandbox",
    "chrome_crashpad",
    "xdg-",
    "ffmpeg",
    "ffprobe",
    "python",
    "java",
    "dxdiag",
    "vulkaninfo",
    "winecfg",
    "msiexec",
];

/// Parts of program names that never start a game (lower case).
const NOT_GAME_PARTS: [&str; 7] = ["crashhandler", "crashreport", "crash_report", "uninstall", "redist", "subprocess", "webhelper"];

/// Parts of names that make a program less likely to be the game, and by
/// how much.
const LESS_LIKELY: [(&[&str], i32); 4] = [
    (&["config", "settings", "setup", "options"], 25),
    (&["editor", "server", "dedicated", "benchmark", "tool", "sdk", "debug", "modmanager"], 20),
    (&["dosbox", "scummvm"], 15),
    (&["launcher"], 10),
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// An `.exe`: runs through Wine / Proton on Linux.
    Windows,
    /// An ELF program, an AppImage or a start script.
    Linux,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Program {
    pub path: PathBuf,
    pub kind: Kind,
    /// Bytes.
    size: u64,
}

/// Why a found game is already in the Collection.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Known {
    #[default]
    No,
    /// One of its programs, or its folder, is a game's.
    Path,
    /// A game with the same name (from another store, maybe).
    Title,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FoundGame {
    pub folder: PathBuf,
    pub title: String,
    /// Best first.
    pub programs: Vec<Program>,
    /// Index in `programs` of the one that starts it.
    pub chosen: usize,
    pub import: bool,
    pub known: Known,
    /// A program right in the chosen folder, next to other games: the
    /// folder is not its own.
    loose: bool,
}

impl FoundGame {
    pub fn program(&self) -> &Program {
        &self.programs[self.chosen.min(self.programs.len() - 1)]
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Scan {
    pub games: Vec<FoundGame>,
    /// The folder was too big to read whole.
    pub truncated: bool,
}

/// What the Collection already has, to tell the games found that it has.
#[derive(Clone, Debug, Default)]
pub struct KnownGames {
    executables: HashSet<String>,
    folders: HashSet<String>,
    titles: HashSet<String>,
}

impl KnownGames {
    pub fn add(&mut self, executables: &[&str], folders: &[&str], title: &str) {
        let keys =
            |list: &[&str]| -> Vec<String> { list.iter().map(|p| Path::new(p.trim())).filter(|p| p.is_absolute()).map(path_key).collect() };
        self.executables.extend(keys(executables));
        self.folders.extend(keys(folders));
        let t = simple(title);
        if !t.is_empty() {
            self.titles.insert(t);
        }
    }
}

/// Searches `root` for games. `linux_programs`: native Linux programs count
/// (not on Windows, where they cannot run).
pub fn scan(root: &Path, linux_programs: bool) -> Scan {
    let (programs, truncated) = walk(root, linux_programs);
    Scan { games: group(root, programs), truncated }
}

/// Marks the games the Collection has and leaves them unselected.
pub fn mark_known(games: &mut [FoundGame], known: &KnownGames) {
    for g in games {
        let by_path = g.programs.iter().any(|p| known.executables.contains(&path_key(&p.path)))
            || (!g.loose && known.folders.contains(&path_key(&g.folder)));
        g.known = if by_path {
            Known::Path
        } else if known.titles.contains(&simple(&g.title)) {
            Known::Title
        } else {
            Known::No
        };
        g.import = g.known == Known::No;
    }
}

fn walk(root: &Path, linux_programs: bool) -> (Vec<Program>, bool) {
    let mut programs = Vec::new();
    let mut stack = vec![(root.to_path_buf(), 0usize)];
    let mut entries = 0usize;
    while let Some((dir, depth)) = stack.pop() {
        let Ok(read) = fs::read_dir(&dir) else { continue };
        for entry in read.flatten() {
            entries += 1;
            if entries > MAX_ENTRIES {
                return (programs, true);
            }
            let Ok(file_type) = entry.file_type() else { continue };
            let name = entry.file_name().to_string_lossy().into_owned();
            let path = entry.path();
            if file_type.is_dir() {
                if depth < MAX_DEPTH && !skip_dir(&name, &dir) {
                    stack.push((path, depth + 1));
                }
                continue;
            }
            // Links to files count (a game linked into a games folder); links
            // to folders are not followed (a prefix's `dosdevices/z:` is `/`).
            let meta = if file_type.is_symlink() { fs::metadata(&path) } else { entry.metadata() };
            let Ok(meta) = meta else { continue };
            if !meta.is_file() {
                continue;
            }
            if let Some(kind) = program_kind(&path, &name, &meta, linux_programs) {
                programs.push(Program { path, kind, size: meta.len() });
            }
        }
    }
    (programs, false)
}

fn skip_dir(name: &str, parent: &Path) -> bool {
    let lower = name.to_lowercase();
    if name.starts_with('.') || SKIP_DIRS.contains(&lower.as_str()) {
        return true;
    }
    // Windows' own folders: `drive_c/windows` of a Wine prefix, `C:\Windows`.
    let drive_root = parent.parent().is_none() || parent.file_name().is_some_and(|n| n.eq_ignore_ascii_case("drive_c"));
    drive_root && matches!(lower.as_str(), "windows" | "users" | "programdata")
}

/// `(stem, extension)` of a lower-case file name.
fn split_name(lower: &str) -> (&str, &str) {
    match lower.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() => (stem, ext),
        _ => (lower, ""),
    }
}

fn program_kind(path: &Path, name: &str, meta: &Metadata, linux_programs: bool) -> Option<Kind> {
    let lower = name.to_lowercase();
    let (stem, ext) = split_name(&lower);
    if !may_be_game(stem) {
        return None;
    }
    match ext {
        "exe" => Some(Kind::Windows),
        _ if !linux_programs => None,
        "appimage" | "x86_64" | "x86" => Some(Kind::Linux),
        "sh" => starts_game(stem, path).then_some(Kind::Linux),
        // Only files without an extension are read: on a FAT or NTFS drive
        // every file looks executable.
        "" | "bin" | "run" => (executable(meta) && is_elf(path)).then_some(Kind::Linux),
        _ => None,
    }
}

fn may_be_game(stem: &str) -> bool {
    !NOT_GAME_PREFIXES.iter().any(|p| stem.starts_with(p)) && !NOT_GAME_PARTS.iter().any(|p| stem.contains(p)) && !stem.contains(".so")
}

/// A shell script that starts a game ("start.sh", "run_game.sh", one named
/// after its folder), not one of its helpers.
fn starts_game(stem: &str, path: &Path) -> bool {
    let folder = path.parent().and_then(Path::file_name).map(|n| simple(&n.to_string_lossy())).unwrap_or_default();
    ["start", "run", "launch", "play", "game"].iter().any(|p| stem.starts_with(p)) || (!folder.is_empty() && simple(stem) == folder)
}

#[cfg(unix)]
fn executable(meta: &Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt;
    meta.permissions().mode() & 0o111 != 0
}

#[cfg(not(unix))]
fn executable(_: &Metadata) -> bool {
    false
}

fn is_elf(path: &Path) -> bool {
    let mut magic = [0u8; 4];
    fs::File::open(path).and_then(|mut f| f.read_exact(&mut magic)).is_ok() && &magic == b"\x7fELF"
}

/// Letters and digits only, lower case: "Hollow_Knight" and "Hollow Knight"
/// compare equal.
fn simple(s: &str) -> String {
    s.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect()
}

/// A path as compared with the Collection's (Windows paths ignore case).
fn path_key(p: &Path) -> String {
    let s = p.to_string_lossy();
    let s = s.trim_end_matches(['/', '\\']);
    if cfg!(windows) {
        s.replace('/', "\\").to_lowercase()
    } else {
        s.to_string()
    }
}

fn is_generic_dir(dir: &Path) -> bool {
    dir.file_name().is_some_and(|n| GENERIC_DIRS.contains(&n.to_string_lossy().to_lowercase().as_str()))
}

/// The folder a program's game is in: its own, past `bin` / `Win64` ones,
/// never above `root`.
fn game_dir(program: &Path, root: &Path) -> PathBuf {
    let mut dir = program.parent().unwrap_or(root);
    while dir != root && is_generic_dir(dir) {
        match dir.parent() {
            Some(p) if p.starts_with(root) => dir = p,
            _ => break,
        }
    }
    dir.to_path_buf()
}

fn levels(inner: &Path, outer: &Path) -> usize {
    inner.strip_prefix(outer).map_or(usize::MAX, |r| r.components().count())
}

fn group(root: &Path, programs: Vec<Program>) -> Vec<FoundGame> {
    let mut by_dir: BTreeMap<PathBuf, Vec<Program>> = BTreeMap::new();
    for p in programs {
        by_dir.entry(game_dir(&p.path, root)).or_default().push(p);
    }
    let in_root = by_dir.remove(root).unwrap_or_default();

    // A folder takes in the folders with programs close inside it, unless
    // that is many of them (a folder of games).
    let dirs: Vec<PathBuf> = by_dir.keys().cloned().collect();
    let nested = |outer: &Path| dirs.iter().filter(|d| *d != outer && levels(d, outer) <= NEST_LEVELS).count();
    let mut owner: HashMap<PathBuf, PathBuf> = HashMap::new();
    for d in &dirs {
        let outer = dirs
            .iter()
            .filter(|o| *o != d && levels(d, o) <= NEST_LEVELS && nested(o) <= MAX_NESTED)
            .min_by_key(|o| o.components().count());
        owner.insert(d.clone(), outer.unwrap_or(d).clone());
    }
    let resolve = |d: &PathBuf| {
        let mut cur = d.clone();
        while let Some(next) = owner.get(&cur).filter(|n| **n != cur) {
            cur = next.clone();
        }
        cur
    };
    let mut merged: BTreeMap<PathBuf, Vec<Program>> = BTreeMap::new();
    for (d, list) in by_dir {
        merged.entry(resolve(&d)).or_default().extend(list);
    }

    let mut games: Vec<FoundGame> = Vec::new();
    let found = |folder: PathBuf, title: String, programs: Vec<Program>, loose: bool| {
        let programs = ranked(programs, &folder);
        FoundGame { folder, title, programs, chosen: 0, import: true, known: Known::No, loose }
    };
    if merged.is_empty() {
        // The folder of one game.
        if !in_root.is_empty() {
            games.push(found(root.to_path_buf(), folder_title(root), in_root, false));
        }
    } else {
        for p in in_root {
            let title = title_from_executable(&p.path);
            games.push(found(root.to_path_buf(), title, vec![p], true));
        }
    }
    for (folder, list) in merged {
        let title = folder_title(&folder);
        games.push(found(folder, title, list, false));
    }
    games.retain(|g| !g.programs.is_empty() && !g.title.is_empty());
    games.sort_by(|a, b| a.title.to_lowercase().cmp(&b.title.to_lowercase()).then_with(|| a.folder.cmp(&b.folder)));
    games
}

fn ranked(mut programs: Vec<Program>, folder: &Path) -> Vec<Program> {
    programs.sort_by_cached_key(|p| (-score(p, folder), p.path.clone()));
    programs.truncate(MAX_PROGRAMS);
    programs
}

/// How likely `p` is the program that starts the game in `folder`: named
/// after the folder, near its top, big, native.
fn score(p: &Program, folder: &Path) -> i32 {
    let lower = p.path.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
    let (stem, ext) = split_name(&lower);
    let (name, own) = (simple(stem), folder.file_name().map(|n| simple(&n.to_string_lossy())).unwrap_or_default());
    let mut s = 0;
    if !name.is_empty() && !own.is_empty() {
        if name == own {
            s += 40;
        } else if name.len() >= 3 && own.len() >= 3 && (name.contains(&own) || own.contains(&name)) {
            s += 25;
        }
    }
    if GENERIC_STEMS.contains(&stem) {
        s += 5;
    }
    for (parts, points) in LESS_LIKELY {
        if parts.iter().any(|w| stem.contains(w)) {
            s -= points;
        }
    }
    let depth = p.path.parent().map_or(0, |d| levels(d, folder).min(10)) as i32;
    s -= 6 * depth;
    // 64 KB or less adds nothing, 64 MB or more 20.
    s += ((p.size / 1024).max(1).ilog2() as i32 - 6).clamp(0, 10) * 2;
    if p.kind == Kind::Linux {
        s += 12;
        if matches!(ext, "appimage" | "x86_64") {
            s += 8;
        }
        // GOG's `start.sh` sets the game up before starting its program.
        if ext == "sh" && depth == 0 {
            s += 35;
        }
    }
    s
}

/// The title a game's folder gives, without release tags ("[GOG]",
/// "(2015)") and a version at its end ("Game v1.5.78").
fn folder_title(dir: &Path) -> String {
    let name = dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let mut bare = String::new();
    let mut inside = 0usize;
    for c in name.chars() {
        match c {
            '[' | '(' => inside += 1,
            ']' | ')' => inside = inside.saturating_sub(1),
            c if inside == 0 => bare.push(c),
            _ => {}
        }
    }
    let words: Vec<&str> = bare.split(|c: char| c.is_whitespace() || c == '_' || c == '-').filter(|w| !w.is_empty()).collect();
    let end = words.iter().position(|w| looks_like_version(w)).filter(|&i| i > 0).unwrap_or(words.len());
    let title = title_from_name(Path::new(&words[..end].join(" ")), true);
    if title.is_empty() {
        title_from_name(dir, true)
    } else {
        title
    }
}

/// "v2", "v1.5", "1.0.3" (a lone "2" is part of a name: "Half-Life 2").
fn looks_like_version(word: &str) -> bool {
    let digits = |s: &str| !s.is_empty() && s.chars().all(|c| c.is_ascii_digit());
    match word.strip_prefix(['v', 'V']) {
        Some(rest) => rest.split('.').all(digits),
        None => word.contains('.') && word.split('.').all(digits),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Tree(PathBuf);

    impl Tree {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("hr-scan-{name}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).unwrap();
            Tree(dir)
        }

        fn file(&self, rel: &str, bytes: &[u8]) -> PathBuf {
            let p = self.0.join(rel);
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            fs::write(&p, bytes).unwrap();
            p
        }

        #[cfg(unix)]
        fn program(&self, rel: &str, bytes: &[u8]) -> PathBuf {
            use std::os::unix::fs::PermissionsExt;
            let p = self.file(rel, bytes);
            fs::set_permissions(&p, fs::Permissions::from_mode(0o755)).unwrap();
            p
        }
    }

    impl Drop for Tree {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn big() -> Vec<u8> {
        let mut b = b"MZ".to_vec();
        b.resize(4 << 20, 0);
        b
    }

    fn titles(scan: &Scan) -> Vec<&str> {
        scan.games.iter().map(|g| g.title.as_str()).collect()
    }

    #[test]
    fn a_folder_of_games_gives_one_game_per_folder() {
        let t = Tree::new("library");
        t.file("Hollow Knight/hollow_knight.exe", &big());
        t.file("Hollow Knight/unins000.exe", b"MZ");
        t.file("Hades/x64/Hades.exe", &big());
        t.file("Hades/x64/UnityCrashHandler64.exe", b"MZ");
        t.file("Hades/_CommonRedist/vcredist/vc_redist.x64.exe", b"MZ");
        t.file("Celeste [GOG] v1.4.0.0/Celeste.exe", b"MZ");
        t.file("Docs/readme.txt", b"");
        let scan = scan(&t.0, false);
        assert_eq!(titles(&scan), ["Celeste", "Hades", "Hollow Knight"]);
        let hades = &scan.games[1];
        assert_eq!(hades.folder, t.0.join("Hades"));
        assert_eq!(hades.program().path, t.0.join("Hades/x64/Hades.exe"));
        assert_eq!(hades.programs.len(), 1);
        assert!(scan.games.iter().all(|g| g.import && g.known == Known::No));
    }

    #[test]
    fn an_unreal_game_is_one_game_started_by_its_top_program() {
        let t = Tree::new("unreal");
        t.file("Stray/Stray.exe", b"MZ");
        t.file("Stray/Hk_project/Binaries/Win64/Stray-Win64-Shipping.exe", &big());
        t.file("Stray/Engine/Binaries/Win64/CrashReportClient.exe", b"MZ");
        t.file("Other/Other.exe", b"MZ");
        let scan = scan(&t.0, false);
        assert_eq!(titles(&scan), ["Other", "Stray"]);
        let stray = &scan.games[1];
        assert_eq!(stray.programs.len(), 2);
        assert_eq!(stray.program().path, t.0.join("Stray/Stray.exe"));
    }

    #[test]
    fn the_folder_of_one_game_is_that_game() {
        let t = Tree::new("single");
        let root = t.0.join("Outer Wilds");
        t.file("Outer Wilds/OuterWilds.exe", &big());
        t.file("Outer Wilds/OuterWilds_Config.exe", b"MZ");
        let scan = scan(&root, false);
        assert_eq!(titles(&scan), ["Outer Wilds"]);
        assert_eq!(scan.games[0].program().path, root.join("OuterWilds.exe"));
    }

    #[test]
    fn loose_programs_next_to_game_folders_are_games_of_their_own() {
        let t = Tree::new("loose");
        t.file("DOOM.exe", b"MZ");
        t.file("setup_quake.exe", b"MZ");
        t.file("Portal/portal.exe", b"MZ");
        let scan = scan(&t.0, false);
        assert_eq!(titles(&scan), ["DOOM", "Portal"]);
        assert_eq!(scan.games[0].folder, t.0);
    }

    #[test]
    fn a_folder_of_games_with_a_stray_program_does_not_swallow_them() {
        let t = Tree::new("stray");
        t.file("Games/tool.exe", b"MZ");
        for g in ["A", "B", "C"] {
            t.file(&format!("Games/{g}/{g}.exe"), b"MZ");
        }
        let scan = scan(&t.0, false);
        assert_eq!(titles(&scan), ["A", "B", "C", "Games"]);
    }

    #[test]
    fn windows_folders_of_a_prefix_are_skipped() {
        let t = Tree::new("prefix");
        t.file("drive_c/windows/system32/notepad.exe", b"MZ");
        t.file("drive_c/Program Files/Common Files/x.exe", b"MZ");
        t.file("drive_c/Program Files/Ubisoft/Rayman/Rayman.exe", b"MZ");
        t.file("drive_c/users/me/AppData/Local/x/x.exe", b"MZ");
        let scan = scan(&t.0, false);
        assert_eq!(titles(&scan), ["Rayman"]);
    }

    #[cfg(unix)]
    #[test]
    fn linux_programs_count_only_when_asked_and_native_ones_come_first() {
        let t = Tree::new("linux");
        t.program("Celeste/Celeste.x86_64", b"\x7fELF");
        t.file("Celeste/Celeste.exe", b"MZ");
        t.program("Celeste/lib64/libSDL2.so.0", b"\x7fELF");
        t.program("Stardew Valley/start.sh", b"#!/bin/sh");
        t.program("Stardew Valley/game/StardewValley", b"\x7fELF");
        t.program("Stardew Valley/support/postinst.sh", b"#!/bin/sh");
        t.program("Notes/readme", b"just text");
        t.file("Tool/tool", b"\x7fELF");
        let scan = scan(&t.0, true);
        assert_eq!(titles(&scan), ["Celeste", "Stardew Valley"]);
        let celeste = &scan.games[0];
        assert_eq!(celeste.program().path, t.0.join("Celeste/Celeste.x86_64"));
        assert_eq!(celeste.program().kind, Kind::Linux);
        assert_eq!(scan.games[1].program().path, t.0.join("Stardew Valley/start.sh"));
        assert_eq!(scan.games[1].programs.len(), 2);

        let windows_only = super::scan(&t.0, false);
        assert_eq!(titles(&windows_only), ["Celeste"]);
        assert_eq!(windows_only.games[0].program().kind, Kind::Windows);
    }

    #[cfg(unix)]
    #[test]
    fn links_to_folders_are_not_followed() {
        let t = Tree::new("links");
        t.file("Portal/Portal.exe", b"MZ");
        std::os::unix::fs::symlink(&t.0, t.0.join("Portal/loop")).unwrap();
        let scan = scan(&t.0, false);
        assert_eq!(titles(&scan), ["Portal"]);
        assert_eq!(scan.games[0].programs.len(), 1);
    }

    #[test]
    fn games_the_collection_has_are_marked_and_left_out() {
        let t = Tree::new("known");
        let a = t.file("A/A.exe", b"MZ");
        t.file("B Game/B.exe", b"MZ");
        t.file("C/C.exe", b"MZ");
        let mut scan = scan(&t.0, false);
        let mut known = KnownGames::default();
        known.add(&[&a.to_string_lossy()], &[], "Something");
        known.add(&[], &[&format!("{}/", t.0.join("C").display())], "");
        known.add(&["relative.exe"], &[], "b game");
        mark_known(&mut scan.games, &known);
        let marks: Vec<(Known, bool)> = scan.games.iter().map(|g| (g.known, g.import)).collect();
        assert_eq!(marks, [(Known::Path, false), (Known::Title, false), (Known::Path, false)]);
    }

    #[test]
    fn folder_titles_lose_tags_and_versions() {
        let t = |n: &str| folder_title(Path::new(&format!("/g/{n}")));
        assert_eq!(t("Celeste [GOG] v1.4.0.0"), "Celeste");
        assert_eq!(t("The_Witcher_3 (2015) 4.04"), "The Witcher 3");
        assert_eq!(t("Half-Life 2"), "Half Life 2");
        assert_eq!(t("v1.0"), "v1 0");
        assert!(looks_like_version("v2") && looks_like_version("1.0.3") && !looks_like_version("2") && !looks_like_version("v"));
    }

    #[test]
    fn helpers_and_installers_are_not_games() {
        for name in ["unins000", "setup", "vc_redist.x64", "unitycrashhandler64", "crashreportclient", "dxsetup", "easyanticheat_setup"] {
            assert!(!may_be_game(name), "{name}");
        }
        for name in ["hades", "game", "start_protected_game", "launcher"] {
            assert!(may_be_game(name), "{name}");
        }
    }
}
