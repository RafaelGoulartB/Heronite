//! Where RetroArch on Windows keeps its config: beside each registered or
//! PATH executable, then `%APPDATA%` (its documented search order), or what a
//! profile's `-c` names.

use std::path::{Path, PathBuf};

use heronite_launch::emulators::{expand_argv, resolve_profile_template, EmulatorsFile, LaunchVars};
use heronite_launch::probe::Probe;

use super::{same_file, ConfigPaths};
use crate::emulators::Emulator;

impl ConfigPaths {
    pub(super) fn windows(registered: &EmulatorsFile, probe: &dyn Probe) -> Self {
        let appdata = probe.env_var("APPDATA").map(PathBuf::from).filter(|p| p.is_absolute()).map(|p| p.join("retroarch.cfg"));
        let mut configs = Vec::new();
        let mut programs: Vec<PathBuf> = Vec::new();
        let mut skipped = Vec::new();
        let known = |programs: &[PathBuf], program: &Path| programs.iter().any(|p| same_file(p) == same_file(program));
        for emulator in &registered.emulators {
            let program = PathBuf::from(&emulator.executable);
            let named = program.file_name().is_some_and(|n| n.to_string_lossy().eq_ignore_ascii_case("retroarch.exe"));
            if emulator.launch_kind == "flatpak" || (emulator.definition_id != "retroarch" && !named) {
                continue;
            }
            if !program.is_absolute() || !probe.exists(&emulator.executable) {
                continue;
            }
            // A root (`C:\`) exists but is no program.
            let Some(directory) = program.parent() else { continue };
            // The launcher's rule (`heronite_launch::plan::emulator`), so a
            // relative `-c` is the file RetroArch opens when a game starts.
            let emulator_dir = PathBuf::from(emulator.folder());
            let vars = LaunchVars {
                emulator_dir: Some(emulator_dir.to_string_lossy().into_owned()),
                install_dir: Some(emulator_dir.to_string_lossy().into_owned()),
                ..Default::default()
            };
            // "Open" on the Emulators page starts it without arguments, so
            // the default config is used whatever the profiles pass.
            configs.extend(default_config(directory, appdata.as_deref(), probe));
            for profile in &emulator.profiles {
                let template = resolve_profile_template(emulator, profile);
                let args = expand_argv(&template.arguments, &vars);
                let Some(value) = config_arg(&args) else { continue };
                let working_directory = (!template.in_rom_dir()).then_some(emulator_dir.as_path());
                match value.map_err(str::to_string).and_then(|value| config_override(value, working_directory)) {
                    Ok(path) => configs.push(path),
                    Err(why) => {
                        let name = if profile.name.is_empty() { &profile.id } else { &profile.name };
                        skipped.push((Emulator::RetroArch, format!("{} (profile {name}): {why}", program.display())));
                    }
                }
            }
            if !known(&programs, &program) {
                programs.push(program);
            }
        }
        // `which` tries `retroarch.exe` too on Windows.
        if let Some(program) = probe.which("retroarch").map(PathBuf::from) {
            if let Some(program) = path_program(program, probe).filter(|p| !known(&programs, p)) {
                if let Some(directory) = program.parent() {
                    configs.extend(default_config(directory, appdata.as_deref(), probe));
                    programs.push(program);
                }
            }
        }
        if programs.is_empty() {
            configs.extend(appdata);
        }
        Self { paths: vec![(Emulator::RetroArch, configs)], retroarch_programs: programs, skipped }
    }
}

/// The value of RetroArch's `-c` / `--config` in `argv` (the last one wins,
/// as with `getopt_long`), `Err` when the option ends the line without one.
fn config_arg(argv: &[String]) -> Option<Result<&str, &'static str>> {
    let mut value = None;
    let mut args = argv.iter();
    while let Some(arg) = args.next() {
        if arg == "--" {
            break;
        } else if arg == "-c" || arg == "--config" {
            value = Some(args.next().map(String::as_str).ok_or("RetroArch's config option has no path"));
        } else if let Some(path) = arg.strip_prefix("--config=") {
            value = Some(Ok(path));
        } else if let Some(path) = arg.strip_prefix("-c").filter(|p| !p.is_empty()) {
            value = Some(Ok(path));
        }
    }
    value
}

/// The RetroArch a PATH hit starts. A Scoop shim (`<root>\shims\retroarch.exe`)
/// names its program in the `.shim` file beside it, failing that the app's
/// `current` folder; RetroArch reads the config beside that program, not the
/// shim. A Chocolatey shim (`<root>\bin`) keeps its target inside the
/// executable, so it is skipped rather than guessed.
fn path_program(hit: PathBuf, probe: &dyn Probe) -> Option<PathBuf> {
    let absolute = |p: PathBuf| (p.is_absolute() && probe.exists(&p.to_string_lossy())).then_some(p);
    let hit = absolute(hit)?;
    let dir = hit.parent()?;
    let named = |p: &Path, name: &str| p.file_name().is_some_and(|n| n.to_string_lossy().eq_ignore_ascii_case(name));
    if named(dir, "shims") {
        let shim = hit.with_extension("shim");
        let listed = probe
            .read_head(&shim.to_string_lossy(), 64 * 1024)
            .and_then(|b| heronite_platform::scoop::shim_target(&String::from_utf8_lossy(&b)));
        let current = || Some(dir.parent()?.join("apps").join(hit.file_stem()?).join("current").join(hit.file_name()?));
        return listed.and_then(absolute).or_else(|| current().and_then(absolute));
    }
    let chocolatey_root = probe.env_var("ChocolateyInstall").map(|root| Path::new(&root).join("bin"));
    let chocolatey = chocolatey_root.is_some_and(|bin| same_file(&bin) == same_file(dir))
        || (named(dir, "bin") && dir.parent().is_some_and(|root| named(root, "chocolatey")));
    (!chocolatey).then_some(hit)
}

fn default_config(directory: &Path, appdata: Option<&Path>, probe: &dyn Probe) -> Option<PathBuf> {
    let local = directory.join("retroarch.cfg");
    if probe.exists(&local.to_string_lossy()) {
        Some(local)
    } else {
        appdata.map(Path::to_path_buf)
    }
}

/// The file a profile's `-c` value names. RetroArch opens it as it is
/// (`retroarch.c` copies the option, `config_file.c` reads that path), so
/// unlike values inside the config there is no `:\` or `~` expansion; a
/// relative path is relative to the folder the launcher starts it in.
fn config_override(value: &str, working_dir: Option<&Path>) -> Result<PathBuf, String> {
    if value.is_empty() || value.contains(['{', '}']) {
        return Err("RetroArch's config path must be a fixed path, not a game-dependent template".into());
    }
    let path = Path::new(value);
    let path = if path.is_absolute() {
        path.to_path_buf()
    } else if let Some(working_dir) = working_dir.filter(|p| p.is_absolute()) {
        working_dir.join(path)
    } else {
        return Err("RetroArch's relative config path needs a fixed absolute working folder; use an absolute config path".into());
    };
    // On Windows a drive without a root (`C:x.cfg`, what an unquoted
    // `C:\x.cfg` becomes once the arguments are split) replaces the working
    // folder in `join` and is relative to that drive's current folder: the
    // result must have both a drive and a root, which `is_absolute` checks.
    if path.is_absolute() {
        Ok(path)
    } else {
        Err(format!("RetroArch's config path {value} is relative to a drive's current folder; quote the full path"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use heronite_launch::emulators::{UserEmulator, UserEmulatorProfile};
    use heronite_launch::probe::FakeProbe;
    use std::fs;

    fn emulator(program: &Path, arguments: &str) -> UserEmulator {
        UserEmulator {
            id: program.to_string_lossy().into_owned(),
            definition_id: "retroarch".into(),
            executable: program.to_string_lossy().into_owned(),
            launch_kind: "native".into(),
            profiles: if arguments.is_empty() {
                Vec::new()
            } else {
                vec![UserEmulatorProfile { id: "profile".into(), arguments: arguments.into(), ..Default::default() }]
            },
            ..Default::default()
        }
    }

    fn put(probe: &mut FakeProbe, path: &Path, text: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
        probe.paths.insert(path.to_string_lossy().into_owned());
    }

    #[test]
    fn installed_and_portable_copies_choose_local_config_before_appdata() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path();
        let appdata = home.join("Roaming");
        let mut probe = FakeProbe::new(home.to_str().unwrap()).with_env("APPDATA", appdata.to_str().unwrap());
        let installed = home.join("Program Files/RetroArch/retroarch.exe");
        let portable = home.join("Portable \u{e9}/retroarch.exe");
        let shared = appdata.join("retroarch.cfg");
        let local = portable.with_file_name("retroarch.cfg");
        for path in [&installed, &portable, &shared, &local] {
            put(&mut probe, path, "");
        }
        let registered = EmulatorsFile { emulators: vec![emulator(&portable, ""), emulator(&installed, "")], ..Default::default() };
        let configs = ConfigPaths::windows(&registered, &probe);
        assert_eq!(configs.candidates(Emulator::RetroArch), [local, shared]);
        assert_eq!(configs.retroarch_programs, [portable, installed]);
        assert!(configs.candidates(Emulator::DuckStation).is_empty());
    }

    #[test]
    fn path_and_appdata_discovery_do_not_reach_the_host() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path();
        let program = home.join("RetroArch/retroarch.exe");
        let local = program.with_file_name("retroarch.cfg");
        let mut probe = FakeProbe::new(home.to_str().unwrap()).with_bin("retroarch", program.to_str().unwrap());
        put(&mut probe, &local, "");
        let registered = EmulatorsFile::default();
        assert_eq!(ConfigPaths::windows(&registered, &probe).candidates(Emulator::RetroArch), [local]);
        let empty = FakeProbe::new(home.to_str().unwrap());
        assert!(ConfigPaths::windows(&registered, &empty).candidates(Emulator::RetroArch).is_empty());
        let fallback = empty.with_env("APPDATA", home.to_str().unwrap());
        assert_eq!(ConfigPaths::windows(&registered, &fallback).candidates(Emulator::RetroArch), [home.join("retroarch.cfg")]);
    }

    #[test]
    fn profile_config_options_add_to_default_discovery() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path();
        let program = home.join("portable/retroarch.exe");
        let local = program.with_file_name("retroarch.cfg");
        let custom = home.join("My configs/custom.cfg");
        let mut probe = FakeProbe::new(home.to_str().unwrap());
        put(&mut probe, &program, "");
        put(&mut probe, &local, "");
        for arguments in [
            format!("-c \"{}\"", custom.display()),
            format!("--config=\"{}\"", custom.display()),
            format!("-c '{}'", custom.display()),
            format!("-c {}", custom.to_string_lossy().replace('\\', "\\\\").replace(' ', "\\ ")),
        ] {
            let registered = EmulatorsFile { emulators: vec![emulator(&program, &arguments)], ..Default::default() };
            // "Open" starts it without `-c`: its default config stays.
            assert_eq!(ConfigPaths::windows(&registered, &probe).candidates(Emulator::RetroArch), [local.clone(), custom.clone()]);
        }
        assert_eq!(config_override("relative.cfg", program.parent()).unwrap(), program.with_file_name("relative.cfg"));
        let template = EmulatorsFile { emulators: vec![emulator(&program, r#"-c "{EmulatorDir}/custom.cfg""#)], ..Default::default() };
        assert_eq!(ConfigPaths::windows(&template, &probe).candidates(Emulator::RetroArch), [local, program.with_file_name("custom.cfg")]);
        assert!(config_override("relative.cfg", None).is_err());
        assert!(config_override("{ImageDir}/retroarch.cfg", Some(home)).is_err());
        assert!(config_override("", Some(home)).is_err());
    }

    #[test]
    fn config_options_are_read_like_retroarch() {
        let arg = |line: &[&str]| config_arg(&line.iter().map(|a| a.to_string()).collect::<Vec<_>>()).map(|r| r.map(str::to_string));
        assert_eq!(arg(&["-L", "core.dll", "-c", "a.cfg", "game.sfc"]), Some(Ok("a.cfg".into())));
        assert_eq!(arg(&["--config", "a.cfg"]), Some(Ok("a.cfg".into())));
        assert_eq!(arg(&["--config=a.cfg"]), Some(Ok("a.cfg".into())));
        assert_eq!(arg(&["-ca.cfg"]), Some(Ok("a.cfg".into())));
        assert_eq!(arg(&["-c", "a.cfg", "--config=b.cfg"]), Some(Ok("b.cfg".into())));
        assert_eq!(arg(&["--", "-c", "a.cfg"]), None);
        assert_eq!(arg(&["-L", "core.dll", "game.sfc"]), None);
        assert!(arg(&["-c"]).unwrap().is_err());
    }

    #[test]
    fn a_bad_profile_skips_only_its_own_config() {
        use crate::emulators::{run, Login, State};
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path();
        let (first, second) = (home.join("A/retroarch.exe"), home.join("B/retroarch.exe"));
        let mut probe = FakeProbe::new(home.to_str().unwrap());
        for program in [&first, &second] {
            put(&mut probe, program, "");
            put(&mut probe, &program.with_file_name("retroarch.cfg"), "cheevos_token = \"\"\n");
        }
        let mut broken = emulator(&first, "-c");
        broken.profiles.push(UserEmulatorProfile { id: "rom".into(), arguments: "-c {ImageDir}/x.cfg".into(), ..Default::default() });
        let registered = EmulatorsFile { emulators: vec![broken, emulator(&second, "")], ..Default::default() };
        let configs = ConfigPaths::windows(&registered, &probe);
        assert_eq!(
            configs.candidates(Emulator::RetroArch),
            [first.with_file_name("retroarch.cfg"), second.with_file_name("retroarch.cfg")]
        );
        assert_eq!(configs.skipped(Emulator::RetroArch).count(), 2);
        let login = Login { username: "FixtureUser", token: "fixture-token", hardcore: false };
        let statuses = run(&configs, &[Emulator::RetroArch], Some(&login), &|_| false, 0, &|| None, None);
        assert_eq!(statuses.iter().filter(|s| s.state == State::SignedIn).count(), 2);
        assert_eq!(statuses.iter().filter(|s| s.config.is_none() && matches!(s.state, State::Failed(_))).count(), 2);
        for program in [&first, &second] {
            assert!(fs::read_to_string(program.with_file_name("retroarch.cfg")).unwrap().contains("fixture-token"));
        }
    }

    #[test]
    fn relative_configs_use_the_install_directory() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path();
        let program = home.join("bin/retroarch.exe");
        let working = home.join("settings");
        let mut probe = FakeProbe::new(home.to_str().unwrap());
        put(&mut probe, &program, "");
        let mut entry = emulator(&program, "-c custom.cfg");
        entry.install_dir = Some(working.to_string_lossy().into_owned());
        let registered = EmulatorsFile { emulators: vec![entry], ..Default::default() };
        assert_eq!(ConfigPaths::windows(&registered, &probe).candidates(Emulator::RetroArch), [working.join("custom.cfg")]);
    }

    /// Windows path rules: only Windows has drives.
    #[cfg(windows)]
    #[test]
    fn drive_relative_configs_are_rejected() {
        let working = Path::new(r"D:\Emu");
        assert!(config_override("C:Emux.cfg", Some(working)).is_err());
        assert!(config_override("D:x.cfg", Some(working)).is_err());
        assert!(config_override("C:Emux.cfg", None).is_err());
        assert_eq!(config_override(r"\Configs\x.cfg", Some(working)).unwrap(), Path::new(r"D:\Configs\x.cfg"));
        assert_eq!(config_override(r"C:\Emu\x.cfg", None).unwrap(), Path::new(r"C:\Emu\x.cfg"));
    }

    /// `rel` under `base`, one component at a time: the fake probe compares
    /// paths as text, and on Windows `join("a/b")` keeps the `/` while the
    /// code under test builds `a\b`.
    fn under(base: &Path, rel: &str) -> PathBuf {
        rel.split('/').fold(base.to_path_buf(), |path, part| path.join(part))
    }

    #[test]
    fn scoop_shims_resolve_and_chocolatey_shims_are_skipped() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path();
        let scoop = home.join("scoop");
        let shim = under(&scoop, "shims/retroarch.exe");
        let program = under(&scoop, "apps/retroarch/current/retroarch.exe");
        let local = program.with_file_name("retroarch.cfg");
        let appdata = home.join("Roaming");
        let mut probe = FakeProbe::new(home.to_str().unwrap())
            .with_env("APPDATA", appdata.to_str().unwrap())
            .with_bin("retroarch", shim.to_str().unwrap());
        put(&mut probe, &program, "");
        put(&mut probe, &local, "");
        let listed = probe
            .clone()
            .with_file(shim.with_extension("shim").to_str().unwrap(), format!("path = \"{}\"\r\n", program.display()).as_bytes());
        let none = EmulatorsFile::default();
        for probe in [&listed, &probe] {
            let configs = ConfigPaths::windows(&none, probe);
            assert_eq!(configs.candidates(Emulator::RetroArch), std::slice::from_ref(&local));
            assert_eq!(configs.retroarch_programs, std::slice::from_ref(&program));
        }
        // Registered too: one installation.
        let registered = EmulatorsFile { emulators: vec![emulator(&program, "")], ..Default::default() };
        assert_eq!(ConfigPaths::windows(&registered, &listed).retroarch_programs, [program]);
        // A shim whose program is gone is not RetroArch's folder either.
        let gone = FakeProbe::new(home.to_str().unwrap())
            .with_env("APPDATA", appdata.to_str().unwrap())
            .with_bin("retroarch", shim.to_str().unwrap());
        assert_eq!(ConfigPaths::windows(&none, &gone).candidates(Emulator::RetroArch), [appdata.join("retroarch.cfg")]);

        let chocolatey = under(home, "ProgramData/chocolatey/bin/retroarch.exe");
        let mut choco = FakeProbe::new(home.to_str().unwrap())
            .with_env("APPDATA", appdata.to_str().unwrap())
            .with_bin("retroarch", chocolatey.to_str().unwrap());
        put(&mut choco, &chocolatey.with_file_name("retroarch.cfg"), "");
        let configs = ConfigPaths::windows(&none, &choco);
        assert!(configs.retroarch_programs.is_empty());
        assert_eq!(configs.candidates(Emulator::RetroArch), [appdata.join("retroarch.cfg")]);
        let custom = under(home, "Choco/bin/retroarch.exe");
        let elsewhere = FakeProbe::new(home.to_str().unwrap())
            .with_env("ChocolateyInstall", home.join("Choco").to_str().unwrap())
            .with_bin("retroarch", custom.to_str().unwrap());
        assert!(ConfigPaths::windows(&none, &elsewhere).retroarch_programs.is_empty());
    }

    #[test]
    fn windows_configs_round_trip_and_sign_out_preserves_other_accounts() {
        use crate::emulators::{check, run, Login, State};
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path();
        let mut probe = FakeProbe::new(home.to_str().unwrap());
        let first = home.join("Installed/retroarch.exe");
        let second = home.join("Portable/renamed.exe");
        let sample = "# keep\r\naudio_volume = \"-3.5\"\r\ncheevos_enable = \"false\"\r\n";
        for program in [&first, &second] {
            put(&mut probe, program, "");
            put(&mut probe, &program.with_file_name("retroarch.cfg"), sample);
        }
        let registered =
            EmulatorsFile { emulators: vec![emulator(&first, ""), emulator(&first, ""), emulator(&second, "")], ..Default::default() };
        let configs = ConfigPaths::windows(&registered, &probe);
        let login = Login { username: "FixtureUser", token: "fixture-token", hardcore: true };
        let statuses = run(&configs, &[Emulator::RetroArch], Some(&login), &|_| false, 0, &|| None, None);
        assert_eq!(statuses.len(), 2);
        assert!(statuses.iter().all(|s| s.state == State::SignedIn));
        let first_config = first.with_file_name("retroarch.cfg");
        let second_config = second.with_file_name("retroarch.cfg");
        let ours = fs::read_to_string(&first_config).unwrap();
        assert!(ours.starts_with("# keep\r\naudio_volume = \"-3.5\"\r\n"));
        assert!(ours.contains("cheevos_token = \"fixture-token\"\r\n"));
        assert!(ours.contains("cheevos_hardcore_mode_enable = \"true\"\r\n"));
        let other = ours.replace("FixtureUser", "OtherUser");
        fs::write(&second_config, &other).unwrap();
        let signed_out = run(&configs, &[Emulator::RetroArch], None, &|_| false, 0, &|| None, Some("fixtureuser"));
        assert!(signed_out.iter().all(|s| s.state == State::SignedOut));
        assert!(!fs::read_to_string(&first_config).unwrap().contains("fixture-token"));
        assert_eq!(fs::read_to_string(&second_config).unwrap(), other);
        assert!(check(&configs, Some("OtherUser"), &|| None).iter().any(|s| s.state == State::SignedIn));
    }

    #[test]
    fn missing_and_running_windows_configs_are_never_written() {
        use crate::emulators::{run, Login, State};
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path();
        let program = home.join("RetroArch/retroarch.exe");
        let config = program.with_file_name("retroarch.cfg");
        let mut probe = FakeProbe::new(home.to_str().unwrap());
        put(&mut probe, &program, "");
        let registered = EmulatorsFile { emulators: vec![emulator(&program, "")], ..Default::default() };
        let login = Login { username: "FixtureUser", token: "fixture-token", hardcore: false };
        let configs = ConfigPaths::windows(&registered, &probe);
        let states = run(&configs, &[Emulator::RetroArch], Some(&login), &|_| false, 0, &|| None, None);
        assert_eq!(states[0].state, State::NotSetUp);
        assert!(!config.exists());
        put(&mut probe, &config, "# unchanged\r\n");
        let configs = ConfigPaths::windows(&registered, &probe);
        let states = run(&configs, &[Emulator::RetroArch], Some(&login), &|_| true, 0, &|| None, None);
        assert_eq!(states[0].state, State::Running);
        assert_eq!(fs::read_to_string(&config).unwrap(), "# unchanged\r\n");
    }
}
