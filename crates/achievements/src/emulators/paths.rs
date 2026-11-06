//! Config discovery is separate from editing: portable Windows emulators
//! live beside their registered executable, not necessarily under the home.

use std::path::{Path, PathBuf};

use heronite_launch::emulators::EmulatorsFile;
use heronite_launch::probe::Probe;

#[cfg(any(not(windows), test))]
use super::spec;
use super::Emulator;

#[derive(Clone, Default)]
pub struct ConfigPaths {
    paths: Vec<(Emulator, Vec<PathBuf>)>,
    pub(super) retroarch_programs: Vec<PathBuf>,
}

impl ConfigPaths {
    /// Reload registered installations so adding a portable emulator does not
    /// require restarting Heronite. Malformed registrations are not ignored.
    pub fn load(home: &Path, paths: &heronite_data::DataPaths) -> Result<Self, String> {
        let registered = if cfg!(windows) {
            heronite_data::json_store::read(&paths.emulators()).map_err(|e| e.to_string())?
        } else {
            EmulatorsFile::default()
        };
        Self::detect(home, &registered, &heronite_launch::probe::SystemProbe)
    }

    /// Uses native Linux locations, or on Windows the registered RetroArch
    /// executables (including portable copies), PATH and APPDATA.
    pub fn detect(home: &Path, registered: &EmulatorsFile, probe: &dyn Probe) -> Result<Self, String> {
        #[cfg(windows)]
        {
            let _ = home;
            Self::windows(registered, probe)
        }
        #[cfg(not(windows))]
        {
            let _ = (registered, probe);
            Ok(Self::linux(home))
        }
    }

    #[cfg(any(not(windows), test))]
    pub(super) fn linux(home: &Path) -> Self {
        Self { paths: Emulator::ALL.into_iter().map(|emulator| (emulator, spec::candidates(home, emulator))).collect(), ..Self::default() }
    }

    pub(super) fn candidates(&self, emulator: Emulator) -> Vec<PathBuf> {
        self.paths.iter().find(|(kind, _)| *kind == emulator).map(|(_, paths)| paths.clone()).unwrap_or_default()
    }

    #[cfg(any(windows, test))]
    fn windows(registered: &EmulatorsFile, probe: &dyn Probe) -> Result<Self, String> {
        use heronite_launch::emulators::{expand_argv, resolve_profile_template, LaunchVars};
        let appdata = probe.env_var("APPDATA").map(PathBuf::from).filter(|p| p.is_absolute()).map(|p| p.join("retroarch.cfg"));
        let mut configs = Vec::new();
        let mut programs = Vec::new();
        for emulator in &registered.emulators {
            let program = PathBuf::from(&emulator.executable);
            let named = program.file_name().is_some_and(|n| n.to_string_lossy().eq_ignore_ascii_case("retroarch.exe"));
            if emulator.launch_kind == "flatpak" || (emulator.definition_id != "retroarch" && !named) {
                continue;
            }
            if !program.is_absolute() || !probe.exists(&emulator.executable) {
                continue;
            }
            let directory = program.parent().expect("an absolute executable has a parent");
            let emulator_dir =
                emulator.install_dir.as_deref().filter(|p| !p.is_empty()).map(PathBuf::from).unwrap_or_else(|| directory.to_path_buf());
            let vars = LaunchVars {
                emulator_dir: Some(emulator_dir.to_string_lossy().into_owned()),
                install_dir: Some(emulator_dir.to_string_lossy().into_owned()),
                ..Default::default()
            };
            let default = || default_config(directory, appdata.as_deref(), probe);
            if emulator.profiles.is_empty() {
                configs.extend(default());
            }
            for profile in &emulator.profiles {
                let template = resolve_profile_template(emulator, profile);
                let args = expand_argv(&template.arguments, &vars);
                let mut override_path = None;
                let mut args = args.iter();
                while let Some(arg) = args.next() {
                    if arg == "--" {
                        break;
                    } else if arg == "-c" || arg == "--config" {
                        override_path = Some(args.next().ok_or("RetroArch's config option has no path")?.as_str());
                    } else if let Some(path) = arg.strip_prefix("--config=") {
                        override_path = Some(path);
                    } else if let Some(path) = arg.strip_prefix("-c").filter(|p| !p.is_empty()) {
                        override_path = Some(path);
                    }
                }
                match override_path {
                    Some(path) => {
                        let working_directory = (template.working_dir.as_deref() != Some("rom")).then_some(emulator_dir.as_path());
                        let path = config_override(path, directory, working_directory)?;
                        configs.push(path);
                    }
                    None => configs.extend(default()),
                }
            }
            programs.push(program);
        }
        if let Some(path) = probe.which("retroarch").or_else(|| probe.which("retroarch.exe")) {
            let program = PathBuf::from(path);
            if program.is_absolute() && !programs.contains(&program) {
                configs.extend(default_config(program.parent().expect("absolute executable"), appdata.as_deref(), probe));
                programs.push(program);
            }
        }
        if programs.is_empty() {
            configs.extend(appdata);
        }
        Ok(Self { paths: vec![(Emulator::RetroArch, configs)], retroarch_programs: programs })
    }
}

#[cfg(any(windows, test))]
fn default_config(directory: &Path, appdata: Option<&Path>, probe: &dyn Probe) -> Option<PathBuf> {
    let local = directory.join("retroarch.cfg");
    if probe.exists(&local.to_string_lossy()) {
        Some(local)
    } else {
        appdata.map(Path::to_path_buf)
    }
}

#[cfg(any(windows, test))]
fn config_override(value: &str, directory: &Path, working_dir: Option<&Path>) -> Result<PathBuf, String> {
    if value.is_empty() || value.contains(['{', '}']) {
        return Err("RetroArch's config path must be a fixed path, not a game-dependent template".into());
    }
    if let Some(relative) = value.strip_prefix(":\\").or_else(|| value.strip_prefix(":/")) {
        return Ok(directory.join(relative));
    }
    let path = PathBuf::from(value);
    if path.is_absolute() {
        Ok(path)
    } else if let Some(working_dir) = working_dir.filter(|p| p.is_absolute()) {
        Ok(working_dir.join(path))
    } else {
        Err("RetroArch's relative config path needs a fixed absolute working folder; use an absolute config path".into())
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
        let configs = ConfigPaths::windows(&registered, &probe).unwrap();
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
        assert_eq!(ConfigPaths::windows(&registered, &probe).unwrap().candidates(Emulator::RetroArch), [local]);
        let empty = FakeProbe::new(home.to_str().unwrap());
        assert!(ConfigPaths::windows(&registered, &empty).unwrap().candidates(Emulator::RetroArch).is_empty());
        let fallback = empty.with_env("APPDATA", home.to_str().unwrap());
        assert_eq!(ConfigPaths::windows(&registered, &fallback).unwrap().candidates(Emulator::RetroArch), [home.join("retroarch.cfg")]);
    }

    #[test]
    fn profile_config_options_override_default_discovery() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path();
        let program = home.join("portable/retroarch.exe");
        let custom = home.join("My configs/custom.cfg");
        let mut probe = FakeProbe::new(home.to_str().unwrap());
        put(&mut probe, &program, "");
        put(&mut probe, &program.with_file_name("retroarch.cfg"), "");
        for arguments in [
            format!("-c \"{}\"", custom.display()),
            format!("--config=\"{}\"", custom.display()),
            format!("-c '{}'", custom.display()),
            format!("-c {}", custom.to_string_lossy().replace('\\', "\\\\").replace(' ', "\\ ")),
        ] {
            let registered = EmulatorsFile { emulators: vec![emulator(&program, &arguments)], ..Default::default() };
            assert_eq!(ConfigPaths::windows(&registered, &probe).unwrap().candidates(Emulator::RetroArch), std::slice::from_ref(&custom));
        }
        assert_eq!(
            config_override("relative.cfg", program.parent().unwrap(), program.parent()).unwrap(),
            program.with_file_name("relative.cfg")
        );
        let template = EmulatorsFile { emulators: vec![emulator(&program, r#"-c "{EmulatorDir}/custom.cfg""#)], ..Default::default() };
        assert_eq!(
            ConfigPaths::windows(&template, &probe).unwrap().candidates(Emulator::RetroArch),
            [program.with_file_name("custom.cfg")]
        );
        assert_eq!(config_override(r":\relative.cfg", home, None).unwrap(), home.join("relative.cfg"));
        assert!(config_override("relative.cfg", home, None).is_err());
        assert!(config_override("{ImageDir}/retroarch.cfg", home, None).is_err());
        assert!(config_override("", home, None).is_err());
        let invalid = EmulatorsFile { emulators: vec![emulator(&program, "-c")], ..Default::default() };
        assert!(ConfigPaths::windows(&invalid, &probe).is_err());
    }

    #[test]
    fn relative_configs_use_install_directory_but_colon_prefix_uses_executable() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path();
        let program = home.join("bin/retroarch.exe");
        let working = home.join("settings");
        let mut probe = FakeProbe::new(home.to_str().unwrap());
        put(&mut probe, &program, "");
        for (argument, expected) in
            [("-c custom.cfg", working.join("custom.cfg")), ("-c ':/custom.cfg'", program.with_file_name("custom.cfg"))]
        {
            let mut entry = emulator(&program, argument);
            entry.install_dir = Some(working.to_string_lossy().into_owned());
            let registered = EmulatorsFile { emulators: vec![entry], ..Default::default() };
            assert_eq!(ConfigPaths::windows(&registered, &probe).unwrap().candidates(Emulator::RetroArch), [expected]);
        }
    }

    #[test]
    fn windows_configs_round_trip_and_sign_out_preserves_other_accounts() {
        use super::super::{check, run, Login, State};
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
        let configs = ConfigPaths::windows(&registered, &probe).unwrap();
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
        use super::super::{run, Login, State};
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path();
        let program = home.join("RetroArch/retroarch.exe");
        let config = program.with_file_name("retroarch.cfg");
        let mut probe = FakeProbe::new(home.to_str().unwrap());
        put(&mut probe, &program, "");
        let registered = EmulatorsFile { emulators: vec![emulator(&program, "")], ..Default::default() };
        let login = Login { username: "FixtureUser", token: "fixture-token", hardcore: false };
        let configs = ConfigPaths::windows(&registered, &probe).unwrap();
        let states = run(&configs, &[Emulator::RetroArch], Some(&login), &|_| false, 0, &|| None, None);
        assert_eq!(states[0].state, State::NotSetUp);
        assert!(!config.exists());
        put(&mut probe, &config, "# unchanged\r\n");
        let configs = ConfigPaths::windows(&registered, &probe).unwrap();
        let states = run(&configs, &[Emulator::RetroArch], Some(&login), &|_| true, 0, &|| None, None);
        assert_eq!(states[0].state, State::Running);
        assert_eq!(fs::read_to_string(&config).unwrap(), "# unchanged\r\n");
    }
}
