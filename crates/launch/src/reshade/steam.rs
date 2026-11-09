//! Steam games Steam starts: their folder (the Collection does not keep it)
//! and, on Linux, which build Steam installed. Steam installs one: the
//! Windows build runs under Proton and takes ReShade like any Wine game (the
//! override then goes in the game's launch options); the Linux build is a
//! Linux game, where vkBasalt is the way.

use std::io::Read;
use std::path::{Path, PathBuf};

use crate::steam::SteamLocator;
use crate::vdf::Vdf;

/// Where ReShade goes for a Steam game.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SteamInstall {
    /// The Windows build in `dir`; `wine`: Proton runs it.
    Windows { dir: PathBuf, wine: bool },
    /// Steam installed the game's Linux build.
    Linux,
    /// No installed copy in Steam's libraries.
    Missing,
}

/// What Steam installed of `app_id`.
pub fn install(locator: &SteamLocator, app_id: &str) -> SteamInstall {
    let Some(dir) = locator.install_dir(app_id) else { return SteamInstall::Missing };
    if cfg!(windows) {
        return SteamInstall::Windows { dir, wine: false };
    }
    let manifest = locator
        .library_roots()
        .into_iter()
        .find_map(|root| std::fs::read_to_string(root.join("steamapps").join(format!("appmanifest_{app_id}.acf"))).ok());
    match windows_build(&dir, manifest.as_deref()) {
        true => SteamInstall::Windows { dir, wine: true },
        false => SteamInstall::Linux,
    }
}

/// Whether the build in `dir` is the Windows one. A game with a Linux build
/// that the user put on Proton says so in its app manifest
/// (`platform_override_source` `windows`); otherwise the files decide: a
/// Windows program that draws (it imports a graphics API) and no Linux
/// program at the top of the folder.
pub fn windows_build(dir: &Path, manifest: Option<&str>) -> bool {
    if manifest.is_some_and(overridden_to_windows) {
        return true;
    }
    !super::detect::candidates(dir).is_empty() && !has_linux_program(dir)
}

/// `AppState` → `UserConfig` / `MountedConfig` → `platform_override_source`.
fn overridden_to_windows(text: &str) -> bool {
    let Ok(doc) = Vdf::parse(text) else { return false };
    let Some(state) = doc.get("AppState") else { return false };
    ["UserConfig", "MountedConfig"]
        .iter()
        .filter_map(|k| state.get(k)?.str_of("platform_override_source"))
        .any(|v| v.eq_ignore_ascii_case("windows"))
}

/// An ELF program (not a library) directly in `dir` or its `bin` folder:
/// where Linux builds keep what starts them.
fn has_linux_program(dir: &Path) -> bool {
    [dir.to_path_buf(), dir.join("bin")].iter().any(|d| {
        let Ok(entries) = std::fs::read_dir(d) else { return false };
        entries.flatten().any(|e| {
            let name = e.file_name().to_string_lossy().to_ascii_lowercase();
            let library = name.ends_with(".so") || name.contains(".so.");
            !library && e.file_type().is_ok_and(|t| t.is_file()) && is_elf(&e.path())
        })
    })
}

fn is_elf(path: &Path) -> bool {
    let mut magic = [0u8; 4];
    std::fs::File::open(path).and_then(|mut f| f.read_exact(&mut magic)).is_ok() && magic == *b"\x7fELF"
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diagnose::pe::tests::pe_with as pe;

    fn pe_with(dlls: &[&str]) -> Vec<u8> {
        pe(crate::diagnose::pe::MACHINE_X64, dlls, None, 0x10_0000)
    }

    fn dir() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    #[test]
    fn a_windows_program_that_draws_is_the_windows_build() {
        let d = dir();
        std::fs::write(d.path().join("Game.exe"), pe_with(&["d3d11.dll"])).unwrap();
        assert!(windows_build(d.path(), None));
    }

    #[test]
    fn a_linux_program_next_to_it_is_the_linux_build() {
        let d = dir();
        std::fs::write(d.path().join("Game.exe"), pe_with(&["d3d11.dll"])).unwrap();
        std::fs::write(d.path().join("Game.x86_64"), b"\x7fELF\x02\x01\x01").unwrap();
        assert!(!windows_build(d.path(), None));
        // Libraries are not what starts a game.
        let w = dir();
        std::fs::write(w.path().join("Game.exe"), pe_with(&["d3d11.dll"])).unwrap();
        std::fs::write(w.path().join("libsteam_api.so"), b"\x7fELF\x02\x01\x01").unwrap();
        assert!(windows_build(w.path(), None));
    }

    #[test]
    fn a_linux_build_alone_is_not_the_windows_one() {
        let d = dir();
        std::fs::write(d.path().join("game.sh"), b"#!/bin/sh\n").unwrap();
        std::fs::create_dir_all(d.path().join("bin")).unwrap();
        std::fs::write(d.path().join("bin").join("game"), b"\x7fELF\x02\x01\x01").unwrap();
        assert!(!windows_build(d.path(), None));
    }

    #[test]
    fn proton_forced_on_a_linux_game_is_the_windows_build() {
        let d = dir();
        let acf = "\"AppState\"\n{\n\t\"appid\"\t\t\"400\"\n\t\"UserConfig\"\n\t{\n\t\t\"platform_override_dest\"\t\t\"linux\"\n\t\t\"platform_override_source\"\t\t\"windows\"\n\t}\n}\n";
        assert!(windows_build(d.path(), Some(acf)));
        assert!(!windows_build(d.path(), Some("\"AppState\"\n{\n\t\"appid\"\t\t\"400\"\n}\n")));
    }
}
