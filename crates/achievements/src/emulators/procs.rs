//! Which emulators are running now: Toolhelp on Windows, `/proc` on Linux.
//! An emulator that runs rewrites its config when it
//! closes, so we do not write while it is open.

use super::Emulator;

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
    arg.rsplit(['/', '\\']).next().unwrap_or(arg)
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
#[cfg(any(not(windows), test))]
pub fn list(proc_root: &std::path::Path) -> Vec<Proc> {
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
#[cfg(not(windows))]
fn running_from_proc() -> impl Fn(Emulator) -> bool {
    let procs = std::cell::OnceCell::new();
    move |emulator| procs.get_or_init(|| list(std::path::Path::new("/proc"))).iter().any(|p| matches(emulator, p))
}

pub fn running(retroarch_programs: &[std::path::PathBuf]) -> impl Fn(Emulator) -> bool {
    #[cfg(windows)]
    {
        let names: Vec<_> = retroarch_programs.iter().filter_map(|p| p.file_name()).map(|n| n.to_string_lossy().into_owned()).collect();
        let procs = std::cell::OnceCell::new();
        move |emulator| {
            let processes = procs.get_or_init(|| {
                let entries = heronite_platform::process::snapshot();
                if entries.is_empty() {
                    log::warn!("could not inspect running emulators; leaving their configurations unchanged");
                }
                entries.into_iter().map(|p| Proc { comm: p.name, args: Vec::new() }).collect::<Vec<_>>()
            });
            windows_running(emulator, processes, &names)
        }
    }
    #[cfg(not(windows))]
    {
        let _ = retroarch_programs;
        running_from_proc()
    }
}

#[cfg(any(windows, test))]
fn windows_running(emulator: Emulator, processes: &[Proc], retroarch_names: &[String]) -> bool {
    processes.is_empty()
        || processes.iter().any(|p| {
            matches(emulator, p)
                || (emulator == Emulator::RetroArch && retroarch_names.iter().any(|name| name.eq_ignore_ascii_case(&p.comm)))
        })
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
        assert!(matches(Emulator::RetroArch, &proc("RETROARCH.EXE", &[])));
        assert!(matches(Emulator::RetroArch, &proc("", &[r"C:\Emulators\RetroArch\retroarch.exe"])));
    }

    #[test]
    fn windows_guard_covers_renamed_programs_and_fails_closed() {
        let emulator = Emulator::RetroArch;
        assert!(windows_running(emulator, &[], &[]));
        assert!(!windows_running(emulator, &[proc("explorer.exe", &[])], &[]));
        assert!(windows_running(emulator, &[proc("RETROARCH.EXE", &[])], &[]));
        assert!(windows_running(emulator, &[proc("My-Emulator.exe", &[])], &["my-emulator.exe".into()]));
    }

    #[cfg(windows)]
    #[test]
    fn windows_guard_sees_a_real_renamed_process() {
        use std::process::{Child, Command, Stdio};
        /// Ends the fixture process even when an assertion fails.
        struct Reap(Child);
        impl Drop for Reap {
            fn drop(&mut self) {
                let _ = self.0.kill();
                let _ = self.0.wait();
            }
        }
        let temp = tempfile::tempdir().unwrap();
        let program = temp.path().join("fixture-emulator.exe");
        std::fs::copy(std::env::var_os("ComSpec").expect("Windows command interpreter"), &program).unwrap();
        let mut command = Command::new(&program);
        command.args(["/d", "/c", "set /p value="]).stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::null());
        heronite_platform::process::new_helper_group(&mut command);
        let child = Reap(command.spawn().unwrap());
        // An empty snapshot also counts as running (fail closed), so check
        // that the guard really sees the fixture.
        let snapshot = heronite_platform::process::snapshot();
        let pid = child.0.id();
        assert!(
            snapshot.iter().any(|p| p.pid == pid && p.name.eq_ignore_ascii_case("fixture-emulator.exe")),
            "the snapshot lists the fixture"
        );
        let programs = vec![program];
        assert!(running(&programs)(Emulator::RetroArch));
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
