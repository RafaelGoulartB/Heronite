//! Executable kind detection and Linux/Windows choice (port of the pure
//! parts of `native-launch.ts`).

use heronite_data::models::{GameInfo, LocalGameMeta};

use crate::probe::Probe;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecutableKind {
    Linux,
    Windows,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    Linux,
    Windows,
}

const WINDOWS_EXTENSIONS: &[&str] = &[".exe", ".bat", ".cmd", ".com", ".msi", ".lnk"];
const LINUX_EXTENSIONS: &[&str] = &[".sh", ".appimage", ".x86_64", ".x86", ".run"];

/// Windows extension wins; then the file header (ELF or `#!` -> Linux,
/// `MZ` -> Windows); then Linux-looking extensions.
pub fn detect_executable_kind(path: &str, probe: &dyn Probe) -> ExecutableKind {
    if path.is_empty() {
        return ExecutableKind::Unknown;
    }
    let lower = path.to_lowercase();
    if WINDOWS_EXTENSIONS.iter().any(|e| lower.ends_with(e)) {
        return ExecutableKind::Windows;
    }
    if let Some(head) = probe.read_head(path, 4) {
        if head.len() >= 2 {
            if head.len() == 4 && head == b"\x7fELF" {
                return ExecutableKind::Linux;
            }
            if head[0] == 0x23 && head[1] == 0x21 {
                return ExecutableKind::Linux;
            }
            if head[0] == 0x4d && head[1] == 0x5a {
                return ExecutableKind::Windows;
            }
        }
    }
    if LINUX_EXTENSIONS.iter().any(|e| lower.ends_with(e)) {
        return ExecutableKind::Linux;
    }
    ExecutableKind::Unknown
}

/// The program a game's command starts: the first word that is not `env`
/// or a `NAME=value` assignment.
pub fn command_program(command: &str) -> Option<String> {
    let words = shlex::split(command).unwrap_or_else(|| command.split_whitespace().map(str::to_string).collect());
    words.into_iter().find(|w| w != "env" && !w.contains('='))
}

/// Checks a command before it is saved or run in `folder`: the folder exists
/// and so does the program it starts. A program with a `/` is a file (`./x`
/// inside the folder, or a full path); a bare name must be installed. Returns
/// the program's file when it is one (it may need the executable bit).
pub fn check_command(command: &str, folder: &str, probe: &dyn Probe) -> Result<Option<String>, String> {
    let folder = folder.trim().trim_end_matches('/');
    if folder.is_empty() || !probe.exists(folder) {
        return Err(heronite_i18n::tr!("Game folder not found: {folder}", folder));
    }
    let Some(program) = command_program(command) else {
        return Err(heronite_i18n::tr!("Type the command that starts the game").into());
    };
    if program.contains('/') {
        let path = if program.starts_with('/') { program.clone() } else { format!("{folder}/{}", program.trim_start_matches("./")) };
        if probe.exists(&path) {
            return Ok(Some(path));
        }
        // "./" + a full path: the user meant the full path.
        if let Some(full) = program.strip_prefix('.').filter(|rest| rest.starts_with('/') && probe.exists(rest)) {
            return Err(heronite_i18n::tr!(
                "{program} is not in the game folder. Did you mean {full}? (./ means \"inside the game folder\")",
                program,
                full
            ));
        }
        return Err(heronite_i18n::tr!("{program} is not in the game folder", program));
    }
    if probe.which(&program).is_some() {
        return Ok(None);
    }
    if probe.exists(&format!("{folder}/{program}")) {
        return Err(heronite_i18n::tr!("To run {program} from the game folder, write ./{program}", program));
    }
    Err(heronite_i18n::tr!("{program} was not found: it is not installed", program))
}

/// Local executables only: emulators and Steam URIs launch their own way.
pub fn is_executable_game(game: &GameInfo, meta: Option<&LocalGameMeta>) -> bool {
    if game.runner != "sideload" {
        return false;
    }
    !matches!(meta.map(|m| m.launch_kind.as_str()), Some("emulator") | Some("steam-uri"))
}

/// `runAs` from the Collection, else detection, else the stored platform.
pub fn effective_platform(game: &GameInfo, meta: Option<&LocalGameMeta>, executable: &str, probe: &dyn Probe) -> Platform {
    match meta.and_then(|m| m.run_as.as_deref()) {
        Some("linux") => return Platform::Linux,
        Some("windows") => return Platform::Windows,
        _ => {}
    }
    match detect_executable_kind(executable, probe) {
        ExecutableKind::Linux => Platform::Linux,
        ExecutableKind::Windows => Platform::Windows,
        ExecutableKind::Unknown => {
            if game.install.platform.as_deref().map(|p| p.eq_ignore_ascii_case("linux")).unwrap_or(false) {
                Platform::Linux
            } else {
                Platform::Windows
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::probe::FakeProbe;

    #[test]
    fn detects_by_extension_then_header_then_extension() {
        let probe = FakeProbe::new("/h")
            .with_file("/g/elf", b"\x7fELF\x02\x01")
            .with_file("/g/script", b"#!/bin/sh\n")
            .with_file("/g/pe", b"MZ\x90\x00")
            .with_file("/g/blob", b"\x00\x01\x02\x03")
            .with_file("/g/Game.AppImage", b"\x00\x00\x00\x00");
        assert_eq!(detect_executable_kind("/g/elf", &probe), ExecutableKind::Linux);
        assert_eq!(detect_executable_kind("/g/script", &probe), ExecutableKind::Linux);
        assert_eq!(detect_executable_kind("/g/pe", &probe), ExecutableKind::Windows);
        assert_eq!(detect_executable_kind("/g/blob", &probe), ExecutableKind::Unknown);
        assert_eq!(detect_executable_kind("/g/Game.AppImage", &probe), ExecutableKind::Linux);
        // A Windows extension wins even over an ELF header.
        let probe2 = FakeProbe::new("/h").with_file("/g/x.exe", b"\x7fELF");
        assert_eq!(detect_executable_kind("/g/x.exe", &probe2), ExecutableKind::Windows);
        assert_eq!(detect_executable_kind("", &probe), ExecutableKind::Unknown);
    }

    #[test]
    fn run_as_overrides_detection() {
        let probe = FakeProbe::new("/h").with_file("/g/elf", b"\x7fELF");
        let mut game = GameInfo { runner: "sideload".into(), app_name: "a".into(), ..Default::default() };
        let mut meta = LocalGameMeta { app_name: "a".into(), ..Default::default() };
        assert_eq!(effective_platform(&game, Some(&meta), "/g/elf", &probe), Platform::Linux);
        meta.run_as = Some("windows".into());
        assert_eq!(effective_platform(&game, Some(&meta), "/g/elf", &probe), Platform::Windows);
        meta.run_as = Some("auto".into());
        game.install.platform = Some("linux".into());
        assert_eq!(effective_platform(&game, Some(&meta), "/unknown", &probe), Platform::Linux);
        game.install.platform = Some("Windows".into());
        assert_eq!(effective_platform(&game, None, "/unknown", &probe), Platform::Windows);
    }

    #[test]
    fn commands_are_checked_against_the_folder() {
        use crate::probe::FakeProbe;
        let probe = FakeProbe::new("/h")
            .with_file("/g/gow3", b"")
            .with_file("/g/gow3/game.AppImage", b"\x7fELF")
            .with_bin("python3", "/usr/bin/python3");
        assert_eq!(check_command("./game.AppImage --play", "/g/gow3", &probe), Ok(Some("/g/gow3/game.AppImage".into())));
        assert_eq!(check_command("env A=1 /g/gow3/game.AppImage", "/g/gow3/", &probe), Ok(Some("/g/gow3/game.AppImage".into())));
        assert_eq!(check_command("python3 run.py", "/g/gow3", &probe), Ok(None));
        let err = check_command("./g/gow3/game.AppImage --play", "/g/gow3", &probe).unwrap_err();
        assert!(err.contains("Did you mean /g/gow3/game.AppImage"), "{err}");
        assert!(check_command("./missing", "/g/gow3", &probe).unwrap_err().contains("not in the game folder"));
        assert!(check_command("game.AppImage", "/g/gow3", &probe).unwrap_err().contains("./game.AppImage"));
        assert!(check_command("nope --x", "/g/gow3", &probe).unwrap_err().contains("not installed"));
        assert!(check_command("./game.AppImage", "/g/none", &probe).unwrap_err().contains("Game folder not found"));
        assert!(check_command("  ", "/g/gow3", &probe).is_err());
    }
}
