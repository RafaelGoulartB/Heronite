//! Which emulators are running now: the names in `/proc/*/comm` and
//! `/proc/*/cmdline`. An emulator that runs rewrites its config when it
//! closes, so we do not write while it is open.

use super::Emulator;
use std::path::Path;

/// One process: its `comm` and its command line.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Proc {
    pub comm: String,
    pub args: Vec<String>,
}

/// Lowercase file-name prefixes of the binaries, AppImages included
/// (`pcsx2-qt`, `pcsx2-v2.x-linux-appimage.AppImage`, `PPSSPPSDL`, ...).
/// `dolphin-emu` and not `dolphin`: the KDE file manager is `dolphin`.
fn prefixes(emulator: Emulator) -> &'static [&'static str] {
    match emulator {
        Emulator::RetroArch => &["retroarch"],
        Emulator::DuckStation => &["duckstation"],
        Emulator::Pcsx2 => &["pcsx2"],
        Emulator::Ppsspp => &["ppsspp"],
        Emulator::Dolphin => &["dolphin-emu"],
        Emulator::Flycast => &["flycast"],
    }
}

pub fn flatpak_id(emulator: Emulator) -> &'static str {
    match emulator {
        Emulator::RetroArch => "org.libretro.RetroArch",
        Emulator::DuckStation => "org.duckstation.DuckStation",
        Emulator::Pcsx2 => "net.pcsx2.PCSX2",
        Emulator::Ppsspp => "org.ppsspp.PPSSPP",
        Emulator::Dolphin => "org.DolphinEmu.dolphin-emu",
        Emulator::Flycast => "org.flycast.Flycast",
    }
}

fn file_name(arg: &str) -> &str {
    arg.rsplit('/').next().unwrap_or(arg)
}

/// Whether `process` is `emulator`: its `comm` or the file name of its
/// program starts with one of the binary names, or the Flatpak id is an
/// argument (`flatpak run <id>`).
pub fn matches(emulator: Emulator, process: &Proc) -> bool {
    let named = |name: &str| {
        let name = name.to_ascii_lowercase();
        prefixes(emulator).iter().any(|p| name.starts_with(p))
    };
    named(&process.comm)
        || process.args.first().is_some_and(|a| named(file_name(a)))
        || process.args.iter().any(|a| a == flatpak_id(emulator))
}

/// Every process under `proc_root` (`/proc`).
pub fn list(proc_root: &Path) -> Vec<Proc> {
    let Ok(entries) = std::fs::read_dir(proc_root) else { return Vec::new() };
    entries
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().bytes().all(|b| b.is_ascii_digit()))
        .filter_map(|e| {
            let dir = e.path();
            let comm = std::fs::read_to_string(dir.join("comm")).ok()?.trim().to_string();
            let cmdline = std::fs::read(dir.join("cmdline")).unwrap_or_default();
            let args = cmdline.split(|b| *b == 0).filter(|a| !a.is_empty()).map(|a| String::from_utf8_lossy(a).into_owned()).collect();
            Some(Proc { comm, args })
        })
        .collect()
}

/// Which of the emulators run now, read from `/proc` on the first question.
pub fn running_from_proc() -> impl Fn(Emulator) -> bool {
    let procs = std::cell::OnceCell::new();
    move |emulator| procs.get_or_init(|| list(Path::new("/proc"))).iter().any(|p| matches(emulator, p))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn proc(comm: &str, args: &[&str]) -> Proc {
        Proc { comm: comm.into(), args: args.iter().map(|a| a.to_string()).collect() }
    }

    #[test]
    fn binaries_and_appimages_match() {
        assert!(matches(Emulator::Pcsx2, &proc("pcsx2-qt", &[])));
        assert!(matches(Emulator::Pcsx2, &proc("AppRun.wrapped", &["/opt/pcsx2-v2.4.0-linux-appimage-x64-Qt.AppImage"])));
        assert!(matches(Emulator::Ppsspp, &proc("PPSSPPSDL", &[])));
        assert!(matches(Emulator::DuckStation, &proc("duckstation-qt", &["duckstation-qt", "game.chd"])));
        assert!(matches(Emulator::Dolphin, &proc("dolphin-emu", &[])));
        assert!(matches(Emulator::Flycast, &proc("flycast", &[])));
        assert!(matches(Emulator::RetroArch, &proc("retroarch", &[])));
    }

    #[test]
    fn flatpak_runs_match_by_id() {
        assert!(matches(Emulator::RetroArch, &proc("flatpak", &["flatpak", "run", "org.libretro.RetroArch"])));
        assert!(!matches(Emulator::Dolphin, &proc("flatpak", &["flatpak", "run", "org.libretro.RetroArch"])));
    }

    #[test]
    fn other_programs_do_not_match() {
        assert!(!matches(Emulator::Dolphin, &proc("dolphin", &["/usr/bin/dolphin"])));
        assert!(!matches(Emulator::Pcsx2, &proc("zsh", &["zsh", "-c", "echo pcsx2"])));
        assert!(!matches(Emulator::Flycast, &proc("heronite", &["heronite"])));
    }

    #[test]
    fn lists_a_proc_tree() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("123");
        std::fs::create_dir(&p).unwrap();
        std::fs::write(p.join("comm"), "retroarch\n").unwrap();
        std::fs::write(p.join("cmdline"), b"/usr/bin/retroarch\0-v\0").unwrap();
        std::fs::create_dir(dir.path().join("self")).unwrap();
        assert_eq!(list(dir.path()), vec![proc("retroarch", &["/usr/bin/retroarch", "-v"])]);
    }
}
