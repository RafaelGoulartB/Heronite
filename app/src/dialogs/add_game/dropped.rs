//! A file or folder dropped on the Collection pre-fills the Add game form.
//! The disk is read on a background thread, never while rendering.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::sideload::SideloadPlatform;

use super::logic::clean_title;
use super::{AddGameDialog, Mode};

/// What a dropped path is.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Kind {
    /// `.exe`, `.msi`, `.bat`: runs through Wine/Proton.
    Windows,
    /// `.AppImage`, `.sh` or an executable file.
    Linux,
    /// A file an emulator of the catalog opens.
    Rom,
    Folder,
    Other,
}

/// Extensions the catalog lists for emulators that are programs or archives
/// as well, so they never mean "ROM" here.
const NOT_ROM: [&str; 9] = ["exe", "msi", "bat", "cmd", "sh", "appimage", "zip", "7z", "rar"];

/// Folders with more entries than this are not searched for a program.
const MAX_ENTRIES: usize = 500;

fn lower_ext(path: &Path) -> String {
    path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default()
}

/// Every extension of the emulator catalog (the one ROM scanning uses).
fn rom_extensions() -> &'static HashSet<String> {
    static SET: OnceLock<HashSet<String>> = OnceLock::new();
    SET.get_or_init(|| {
        heronite_emulation::emulator_definitions()
            .iter()
            .flat_map(|d| &d.profiles)
            .flat_map(|p| &p.extensions)
            .map(|e| e.trim_start_matches('.').to_lowercase())
            .filter(|e| !NOT_ROM.contains(&e.as_str()))
            .collect()
    })
}

/// The program kind a file name says it is (`None`: not a program by name).
fn program_kind(path: &Path) -> Option<Kind> {
    match lower_ext(path).as_str() {
        "exe" | "msi" | "bat" | "cmd" => Some(Kind::Windows),
        "appimage" | "sh" => Some(Kind::Linux),
        _ => None,
    }
}

/// `executable` is the Unix execute bit; it only counts for files that are
/// not ROMs (ROMs on FAT or NTFS drives look executable).
pub(super) fn classify(path: &Path, is_dir: bool, executable: bool) -> Kind {
    if is_dir {
        return Kind::Folder;
    }
    if let Some(kind) = program_kind(path) {
        return kind;
    }
    if rom_extensions().contains(&lower_ext(path)) {
        return Kind::Rom;
    }
    if executable {
        Kind::Linux
    } else {
        Kind::Other
    }
}

/// A title from a file or folder name: `_`, `-` and `.` become spaces and the
/// spaces are tidied. A file loses its extension first.
pub(crate) fn title_from_name(path: &Path, is_dir: bool) -> String {
    let name = if is_dir { path.file_name() } else { path.file_stem() };
    let name = name.map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let spaced: String = name.chars().map(|c| if matches!(c, '_' | '-' | '.') { ' ' } else { c }).collect();
    clean_title(&spaced.split_whitespace().collect::<Vec<_>>().join(" "))
}

/// Program names that do not name the game ("start.exe", "Launcher.exe"):
/// its folder does.
pub(crate) const GENERIC_STEMS: [&str; 14] =
    ["game", "launcher", "launch", "start", "play", "run", "main", "app", "client", "bin", "start game", "play game", "win64", "x64"];

/// Folders between a game's own folder and its program.
pub(crate) const GENERIC_DIRS: [&str; 16] = [
    "bin", "bin32", "bin64", "binaries", "win32", "win64", "x86", "x64", "x86_64", "game", "data", "program", "windows", "linux",
    "release", "shipping",
];

/// Endings build tools add to a program's name ("Hades-Win64-Shipping.exe").
const BUILD_SUFFIXES: [&str; 7] = ["-win64-shipping", "-wingdk-shipping", "-linux-shipping", "-shipping", ".x86_64", "_x64", "-x64"];

/// The title an executable gives: its name without what build tools add,
/// or, when that name is generic ("start.exe"), the nearest folder that is
/// not a `bin` / `Win64` one.
pub(crate) fn title_from_executable(path: &Path) -> String {
    let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let lower = stem.to_lowercase();
    let trimmed = BUILD_SUFFIXES.iter().find(|s| lower.ends_with(*s)).map_or(stem.as_str(), |s| &stem[..stem.len() - s.len()]);
    let own = title_from_name(Path::new(&format!("{trimmed}.x")), false);
    if !own.is_empty() && !GENERIC_STEMS.contains(&own.to_lowercase().as_str()) {
        return own;
    }
    path.ancestors()
        .skip(1)
        .filter_map(|dir| dir.file_name().map(|n| (dir, n.to_string_lossy().to_lowercase())))
        .find(|(_, name)| !GENERIC_DIRS.contains(&name.as_str()))
        .map(|(dir, _)| title_from_name(dir, true))
        .filter(|t| !t.is_empty())
        .unwrap_or(own)
}

/// The one program at the top level of `dir`, when there is exactly one (an
/// uninstaller does not count).
fn single_program(dir: &Path) -> Option<(PathBuf, Kind)> {
    let mut found = None;
    for entry in std::fs::read_dir(dir).ok()?.take(MAX_ENTRIES).flatten() {
        let path = entry.path();
        let Some(kind) = program_kind(&path) else { continue };
        let uninstaller = path.file_name().is_some_and(|n| n.to_string_lossy().to_lowercase().starts_with("unins"));
        if uninstaller || !entry.file_type().is_ok_and(|t| !t.is_dir()) {
            continue;
        }
        if found.replace((path, kind)).is_some() {
            return None;
        }
    }
    found
}

#[cfg(unix)]
fn has_execute_bit(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path).is_ok_and(|m| m.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
fn has_execute_bit(_: &Path) -> bool {
    false
}

/// What the background thread found out about a dropped path.
struct Inspected {
    path: PathBuf,
    kind: Kind,
    /// Folder: its only program.
    program: Option<(PathBuf, Kind)>,
}

fn inspect(path: PathBuf) -> Inspected {
    let is_dir = path.is_dir();
    let kind = classify(&path, is_dir, !is_dir && has_execute_bit(&path));
    let program = (kind == Kind::Folder).then(|| single_program(&path)).flatten();
    Inspected { path, kind, program }
}

impl AddGameDialog {
    /// Fills the new-game form from a dropped file or folder. Does nothing
    /// when editing or when a game is being installed.
    pub fn prefill_from_path(&mut self, path: &Path, window: &mut Window, cx: &mut Context<Self>) {
        if !matches!(self.mode, Mode::Add) {
            return;
        }
        let path = path.to_path_buf();
        cx.spawn_in(window, async move |this, cx| {
            let found = cx.background_spawn(async move { inspect(path) }).await;
            this.update_in(cx, |this, window, cx| this.apply_dropped(found, window, cx)).ok();
        })
        .detach();
    }

    fn apply_dropped(&mut self, found: Inspected, window: &mut Window, cx: &mut Context<Self>) {
        let Inspected { path, kind, program } = found;
        let set = |input: &Entity<_>, v: String, window: &mut Window, cx: &mut Context<Self>| {
            input.update(cx, |s: &mut gpui_kit::component::input::InputState, cx| s.set_value(v, window, cx));
        };
        let is_dir = kind == Kind::Folder;
        let title = if is_dir { title_from_name(&path, true) } else { title_from_executable(&path) };
        self.last_auto_title = title.clone();
        set(&self.title, title, window, cx);
        // The program to run: the file itself, or the folder's only program.
        let (exe, exe_kind) = match (is_dir, program) {
            (false, _) => (Some(path.clone()), kind),
            (true, Some((exe, k))) => (Some(exe), k),
            (true, None) => (None, kind),
        };
        match exe {
            Some(exe) => {
                self.by_command = false;
                let exe = exe.to_string_lossy().into_owned();
                set(&self.exe, exe.clone(), window, cx);
                match exe_kind {
                    Kind::Windows => self.set_platform(SideloadPlatform::Windows, cx),
                    Kind::Linux => self.set_platform(SideloadPlatform::Linux, cx),
                    Kind::Rom | Kind::Folder | Kind::Other => self.executable_changed(&exe),
                }
                self.fill_folder_from(&exe, window, cx);
            }
            None => {
                // A folder without a clear program: start it with a command in it.
                self.by_command = true;
                set(&self.folder, path.to_string_lossy().into_owned(), window, cx);
                self.set_platform(SideloadPlatform::Linux, cx);
            }
        }
        self.title_changed(window, cx);
        self.auto_find_art(window, cx);
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    // Not `super::*`: it would bring in gpui's `test` macro.
    use super::{classify, single_program, title_from_executable, title_from_name, Kind};
    use std::path::Path;

    fn kind(name: &str) -> Kind {
        classify(Path::new(name), false, false)
    }

    #[test]
    fn programs_are_told_by_extension() {
        assert_eq!(kind("/g/Setup.EXE"), Kind::Windows);
        assert_eq!(kind("/g/x.msi"), Kind::Windows);
        assert_eq!(kind("/g/Game.AppImage"), Kind::Linux);
        assert_eq!(kind("/g/start.sh"), Kind::Linux);
        assert_eq!(kind("/g/notes.txt"), Kind::Other);
    }

    #[test]
    fn roms_come_from_the_emulator_catalog() {
        assert_eq!(kind("/r/Super Metroid.sfc"), Kind::Rom);
        assert_eq!(kind("/r/Sonic.GBA"), Kind::Rom);
        // Archives and programs are never ROMs by themselves.
        assert_eq!(kind("/r/game.zip"), Kind::Other);
        assert_eq!(kind("/r/game.exe"), Kind::Windows);
    }

    #[test]
    fn folders_and_executable_files() {
        assert_eq!(classify(Path::new("/g/My Game"), true, false), Kind::Folder);
        assert_eq!(classify(Path::new("/g/game"), false, true), Kind::Linux);
        assert_eq!(classify(Path::new("/g/game"), false, false), Kind::Other);
        // A ROM that looks executable stays a ROM.
        assert_eq!(classify(Path::new("/r/a.sfc"), false, true), Kind::Rom);
    }

    #[test]
    fn titles_are_tidied() {
        let t = |n: &str, dir| title_from_name(Path::new(n), dir);
        assert_eq!(t("/g/Super_Game-Deluxe.v2.exe", false), "Super Game Deluxe v2");
        assert_eq!(t("/g/  my__game .AppImage", false), "my game");
        assert_eq!(t("/g/Half-Life_2.v1.5", true), "Half Life 2 v1 5");
        assert_eq!(t("/", true), "");
    }

    #[test]
    fn an_executable_names_the_game_or_its_folder_does() {
        let t = |p: &str| title_from_executable(Path::new(p));
        assert_eq!(t("/games/Hollow Knight/hollow_knight.exe"), "hollow knight");
        // Build tools' endings go.
        assert_eq!(t("/games/Hades/Hades-Win64-Shipping.exe"), "Hades");
        assert_eq!(t("/games/Celeste/Celeste.x86_64"), "Celeste");
        // A generic name: the game's folder, past bin / Win64 folders.
        assert_eq!(t("/games/Outer Wilds/Launcher.exe"), "Outer Wilds");
        assert_eq!(t("/games/Some_Game/bin/x64/start.exe"), "Some Game");
        assert_eq!(t("/games/Some Game/Game.sh"), "Some Game");
    }

    #[test]
    fn a_folder_with_one_program_gives_it() {
        let dir = std::env::temp_dir().join(format!("hr-drop-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(single_program(&dir), None);
        std::fs::write(dir.join("game.exe"), b"MZ").unwrap();
        std::fs::write(dir.join("unins000.exe"), b"MZ").unwrap();
        std::fs::write(dir.join("readme.txt"), b"").unwrap();
        assert_eq!(single_program(&dir), Some((dir.join("game.exe"), Kind::Windows)));
        std::fs::write(dir.join("other.exe"), b"MZ").unwrap();
        assert_eq!(single_program(&dir), None);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
