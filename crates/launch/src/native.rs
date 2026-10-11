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
}
