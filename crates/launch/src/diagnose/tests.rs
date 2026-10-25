use serde_json::{json, Value};

use super::*;

const HEADER: &str = "(21:04:41) [INFO]:    Launching \"Calluna\" (sideload)\n(21:04:41) [INFO]:    Native? false\n";

fn log(output: &str) -> String {
    format!("{HEADER}(21:04:42) [INFO]:    Launching Calluna: umu-run game.exe\n(21:04:42) [INFO]:    Game Output:\n{output}\n(21:04:44) [INFO]:    Game process exited with code 53\n")
}

fn settings(v: Value) -> GameSettings {
    GameSettings::from_map(v.as_object().unwrap(), "/home/u")
}

fn proton() -> GameSettings {
    settings(json!({"wineVersion": {"bin": "/t/GE-Proton10/proton", "name": "GE-Proton10", "type": "proton"}}))
}

fn wine() -> GameSettings {
    settings(json!({"wineVersion": {"bin": "/usr/bin/wine", "name": "Wine", "type": "wine"}}))
}

struct Case {
    log: String,
    kind: RunKind,
    settings: GameSettings,
    imports: Vec<String>,
    winetricks: Vec<String>,
    anticheat: Option<AntiCheatInfo>,
    system: System,
    failed: bool,
    can_repair: bool,
}

impl Case {
    fn new(output: &str) -> Self {
        Case {
            log: log(output),
            kind: RunKind::Proton,
            settings: proton(),
            imports: Vec::new(),
            winetricks: Vec::new(),
            anticheat: None,
            system: System { max_map_count: Some(MAX_MAP_COUNT), ..Default::default() },
            failed: true,
            can_repair: false,
        }
    }

    fn run(&self) -> Vec<Finding> {
        diagnose(&Inputs {
            log: Some(&self.log),
            kind: self.kind,
            settings: &self.settings,
            imports: &self.imports,
            winetricks: &self.winetricks,
            anticheat: self.anticheat.as_ref(),
            system: &self.system,
            failed: self.failed,
            can_repair: self.can_repair,
        })
    }
}

fn problems(f: &[Finding]) -> Vec<&Problem> {
    f.iter().map(|f| &f.problem).collect()
}

#[test]
fn missing_visual_cpp_offers_its_redistributable() {
    let c = Case::new(
        r#"0024:err:module:import_dll Library MSVCP140.dll (which is needed by L"C:\\Games\\Calluna\\Calluna.exe") not found
0024:err:module:import_dll Library VCRUNTIME140_1.dll (which is needed by L"C:\\Games\\Calluna\\Calluna.exe") not found
0024:err:module:loader_init Importing dlls for L"C:\\Games\\Calluna\\Calluna.exe" failed, status c0000135"#,
    );
    let f = c.run();
    assert_eq!(f.len(), 1);
    assert_eq!(
        f[0].problem,
        Problem::MissingDlls { dlls: vec!["MSVCP140.dll".into(), "VCRUNTIME140_1.dll".into()], verbs: vec!["vcrun2022".into()] }
    );
    assert_eq!(f[0].fixes, [Fix::Winetricks(vec!["vcrun2022".into()])]);
    assert_eq!(f[0].evidence.len(), 2);
    assert!(f[0].actionable());
    assert!(opens_by_itself(&f));
}

#[test]
fn a_component_already_installed_is_not_offered_again() {
    let mut c = Case::new(r#"0024:err:module:import_dll Library d3dx9_43.dll (which is needed by L"C:\\g\\g.exe") not found"#);
    c.winetricks = vec!["d3dx9".into()];
    let f = c.run();
    assert!(matches!(&f[0].problem, Problem::MissingDlls { verbs, .. } if verbs.is_empty()));
    assert!(f[0].fixes.is_empty());
}

#[test]
fn a_dll_of_the_game_itself_asks_for_a_repair() {
    let mut c = Case::new(r#"0024:err:module:import_dll Library GameCore.dll (which is needed by L"C:\\g\\g.exe") not found"#);
    c.can_repair = true;
    assert_eq!(c.run()[0].fixes, [Fix::Repair]);
}

#[test]
fn the_header_and_settings_are_not_read_as_game_output() {
    // The settings JSON of the header names the runtimes and variables.
    let mut c = Case::new("ready");
    c.log = format!("{HEADER}\"eacRuntime\": false, \"Unhandled exception\"\n(21:04:42) [INFO]:    Game Output:\nok\n");
    c.failed = false;
    assert!(c.run().is_empty());
}

#[test]
fn a_clean_exit_with_nothing_known_says_nothing() {
    let mut c = Case::new("Hello from the launcher\nstarting the game");
    c.failed = false;
    let f = c.run();
    assert!(f.is_empty());
    assert!(!opens_by_itself(&f));
}

#[test]
fn a_crash_without_a_known_cause_offers_another_proton() {
    let f = Case::new("wine: Unhandled page fault on read access to 0000000000000000 at address 0000000140001000").run();
    assert_eq!(problems(&f), [&Problem::Crash]);
    assert_eq!(f[0].fixes, [Fix::OtherProton]);
    // Told when asked, never by itself: the cause is unknown.
    assert!(!f[0].sure);
    assert!(!opens_by_itself(&f));
    let mut c = Case::new("wine: Unhandled page fault");
    c.kind = RunKind::Wine;
    c.settings = wine();
    assert_eq!(c.run()[0].fixes, [Fix::UseProton(Vec::new())]);
}

#[test]
fn a_failed_run_with_a_quiet_log_is_a_likely_crash() {
    let f = Case::new("").run();
    assert_eq!(problems(&f), [&Problem::Crash]);
    assert!(!f[0].sure);
    assert!(!opens_by_itself(&f));
}

#[test]
fn wine_mono_missing_offers_dotnet() {
    let f = Case::new("0110:err:mscoree:CLRRuntimeInfo_GetRuntimeHost Wine Mono is not installed").run();
    assert_eq!(problems(&f), [&Problem::DotNet]);
    assert_eq!(f[0].fixes, [Fix::Winetricks(vec!["dotnet48".into()])]);
}

#[test]
fn dx12_under_plain_wine_needs_vkd3d() {
    let mut c = Case::new("0124:err:d3d12:d3d12_create_device Failed to create device");
    c.kind = RunKind::Wine;
    c.settings = wine();
    c.imports = vec!["kernel32.dll".into(), "d3d12.dll".into()];
    let f = c.run();
    assert_eq!(problems(&f)[0], &Problem::Dx12WithoutVkd3d);
    assert_eq!(f[0].fixes[0], Fix::Settings(vec![("autoInstallVkd3d", Value::Bool(true))]));
    assert!(f[0].actionable());
    // The import alone is only a likely cause: Wine has its own Direct3D 12.
    c.log = log("");
    let f = c.run();
    let dx12 = f.iter().find(|f| f.problem == Problem::Dx12WithoutVkd3d).unwrap();
    assert!(!dx12.sure);
    assert!(!opens_by_itself(&f));
    // CrossOver has no VKD3D-Proton switch.
    c.log = log("0124:err:d3d12:d3d12_create_device Failed to create device");
    c.settings = settings(json!({"wineVersion": {"bin": "/opt/cxoffice/bin/wine", "name": "CrossOver", "type": "crossover"}}));
    assert!(!problems(&c.run()).contains(&&Problem::Dx12WithoutVkd3d));
    // Proton has VKD3D-Proton already.
    let mut c = Case::new("");
    c.imports = vec!["d3d12.dll".into()];
    assert!(!problems(&c.run()).contains(&&Problem::Dx12WithoutVkd3d));
    // So does a prefix with it on.
    let mut c = Case::new("");
    c.kind = RunKind::Wine;
    c.settings = settings(json!({"wineVersion": {"bin": "/usr/bin/wine", "type": "wine"}, "autoInstallVkd3d": true}));
    c.imports = vec!["d3d12.dll".into()];
    assert!(!problems(&c.run()).contains(&&Problem::Dx12WithoutVkd3d));
}

#[test]
fn wined3d_failing_without_dxvk_offers_dxvk() {
    let mut c = Case::new("0024:err:d3d:wined3d_adapter_create No GPU found");
    c.kind = RunKind::Wine;
    c.settings = wine();
    let f = c.run();
    assert_eq!(problems(&f), [&Problem::Direct3dWithoutDxvk]);
    assert_eq!(f[0].fixes, [Fix::Settings(vec![("autoInstallDxvk", Value::Bool(true))])]);
    // DXVK is not installed into a CrossOver prefix.
    c.settings = settings(json!({"wineVersion": {"bin": "/opt/cxoffice/bin/wine", "name": "CrossOver", "type": "crossover"}}));
    assert!(!problems(&c.run()).contains(&&Problem::Direct3dWithoutDxvk));
}

fn anticheat(status: &str, names: &[&str]) -> Option<AntiCheatInfo> {
    Some(AntiCheatInfo { status: status.into(), anticheats: names.iter().map(|s| s.to_string()).collect() })
}

#[test]
fn a_supported_anticheat_turns_its_runtime_on() {
    let mut c = Case::new("");
    c.failed = false;
    c.anticheat = anticheat("Supported", &["Easy Anti-Cheat"]);
    let f = c.run();
    assert_eq!(f[0].problem, Problem::AntiCheat { names: vec!["Easy Anti-Cheat".into()], status: AntiCheatStatus::Works });
    assert_eq!(f[0].fixes, [Fix::Settings(vec![("eacRuntime", Value::Bool(true))])]);
    assert!(opens_by_itself(&f));
    // Plain Wine cannot load it: Proton first.
    c.kind = RunKind::Wine;
    c.settings = wine();
    assert_eq!(c.run()[0].fixes, [Fix::UseProton(vec![("eacRuntime", Value::Bool(true))])]);
    // With the runtime on there is nothing left to say.
    let mut c = Case::new("");
    c.failed = false;
    c.settings = settings(json!({"wineVersion": {"bin": "/t/p/proton", "type": "proton"}, "eacRuntime": true}));
    c.anticheat = anticheat("Running", &["Easy Anti-Cheat"]);
    assert!(c.run().is_empty());
}

#[test]
fn a_denied_anticheat_is_explained_without_a_fix() {
    let mut c = Case::new("");
    c.anticheat = anticheat("Denied", &["Easy Anti-Cheat", "BattlEye"]);
    let f = c.run();
    assert_eq!(
        f[0].problem,
        Problem::AntiCheat { names: vec!["Easy Anti-Cheat".into(), "BattlEye".into()], status: AntiCheatStatus::Denied }
    );
    assert!(f[0].fixes.is_empty());
    // A known cause: no guessing about crashes. Nothing to fix: only told
    // when asked.
    assert_eq!(f.len(), 1);
    assert!(!opens_by_itself(&f));
}

#[test]
fn vulkan_failures_offer_the_drivers_of_the_distribution() {
    let mut c = Case::new("err:   DxvkInstance: Failed to create instance\nerr:   vkCreateInstance failed: VK_ERROR_INCOMPATIBLE_DRIVER");
    c.system.distro = Distro::Arch;
    c.system.gpu = Some(crate::gpu::Vendor::Amd);
    let f = c.run();
    assert_eq!(problems(&f), [&Problem::Vulkan]);
    assert_eq!(
        f[0].fixes,
        [Fix::Terminal("sudo pacman -S --needed vulkan-icd-loader lib32-vulkan-icd-loader vulkan-radeon lib32-vulkan-radeon".into())]
    );
    assert_eq!(f[0].evidence.len(), 2);
}

#[test]
fn too_many_open_files_with_esync_switches_to_fsync() {
    let mut c = Case::new("eventfd: Too many open files");
    c.settings = settings(json!({"wineVersion": {"bin": "/t/p/proton", "type": "proton"}, "enableEsync": true}));
    let f = c.run();
    assert_eq!(problems(&f), [&Problem::TooManyOpenFiles]);
    assert_eq!(f[0].fixes, [Fix::Settings(vec![("enableEsync", Value::Bool(false)), ("enableFsync", Value::Bool(true))])]);
}

#[test]
fn a_low_max_map_count_is_offered_for_unexplained_crashes() {
    let mut c = Case::new("");
    c.system.max_map_count = Some(65530);
    let f = c.run();
    assert_eq!(problems(&f), [&Problem::MaxMapCount { current: 65530 }, &Problem::Crash]);
    assert_eq!(f[0].fixes, [Fix::RaiseMaxMapCount]);
    assert!(!f[0].sure);
    // Not when the log explains the crash, nor after a clean exit.
    c.log = log(r#"err:module:import_dll Library MSVCR120.dll (which is needed by L"C:\\g.exe") not found"#);
    assert_eq!(problems(&c.run()).len(), 1);
    c.log = log("");
    c.failed = false;
    assert!(c.run().is_empty());
    // Proton's own warning (PROTON_LOG) is proof.
    c.log = log("WARNING: Low /proc/sys/vm/max_map_count: 65530 will prevent some games from working");
    let f = c.run();
    assert!(f[0].sure);
}

#[test]
fn missing_linux_libraries_name_the_package_search() {
    let mut c = Case::new("./Game.x86: error while loading shared libraries: libGL.so.1: wrong ELF class: ELFCLASS64");
    c.kind = RunKind::Native;
    c.system.distro = Distro::Fedora;
    let f = c.run();
    assert_eq!(f[0].problem, Problem::MissingLibs { libs: vec!["libGL.so.1".into()], bits32: true });
    assert_eq!(f[0].fixes, [Fix::Terminal("dnf provides '*/libGL.so.1'".into())]);
}

#[test]
fn wine_without_its_32_bit_half_switches_to_proton() {
    let mut c = Case::new("it looks like wine32 is missing, you should install it.");
    c.kind = RunKind::Wine;
    c.settings = wine();
    c.system.distro = Distro::Debian;
    let f = c.run();
    assert_eq!(f[0].problem, Problem::MissingLibs { libs: Vec::new(), bits32: true });
    assert_eq!(f[0].fixes[0], Fix::UseProton(Vec::new()));
    assert!(matches!(&f[0].fixes[1], Fix::Terminal(cmd) if cmd.ends_with("apt install wine32")));
}

#[test]
fn a_missing_executable_asks_for_a_repair() {
    let mut c = Case::new(r#"wine: cannot find L"C:\\Games\\Calluna\\Calluna.exe""#);
    c.can_repair = true;
    let f = c.run();
    assert_eq!(problems(&f), [&Problem::ExecutableMissing]);
    assert_eq!(f[0].fixes, [Fix::Repair]);
}

#[test]
fn a_wineserver_of_another_version_is_stopped() {
    let f = Case::new("wine client error:0: version mismatch 798/801.\nYour wine client and server are not the same version.").run();
    assert_eq!(problems(&f), [&Problem::Wineserver]);
    assert_eq!(f[0].fixes, [Fix::StopWineserver]);
}

#[test]
fn logs_turned_off_ask_for_them() {
    let mut c = Case::new("");
    c.log = format!("{HEADER}(21:04:41) [WARNING]: IMPORTANT: Logs are disabled\n");
    let f = c.run();
    assert_eq!(problems(&f), [&Problem::Crash, &Problem::LogsOff]);
    assert_eq!(f[1].fixes, [Fix::Settings(vec![("verboseLogs", Value::Bool(true))])]);
}

#[test]
fn run_kind_comes_from_the_header() {
    assert_eq!(run_kind(HEADER, &proton()), Some(RunKind::Proton));
    assert_eq!(run_kind(HEADER, &wine()), Some(RunKind::Wine));
    assert_eq!(run_kind("[INFO]:    Native? true", &proton()), Some(RunKind::Native));
    assert_eq!(run_kind("Native? false\nSteam game, opened through steam://rungameid/1", &proton()), None);
    assert_eq!(run_kind("no header", &proton()), None);
}

#[test]
fn long_lines_are_shortened() {
    let long = format!("wine: Unhandled page fault {}", "x".repeat(500));
    let f = Case::new(&long).run();
    assert!(f[0].evidence[0].chars().count() <= 241);
    assert!(f[0].evidence[0].ends_with('\u{2026}'));
}

#[test]
fn distributions_are_told_apart() {
    assert_eq!(Distro::parse("NAME=\"Arch Linux\"\nID=arch\n", false), Distro::Arch);
    assert_eq!(Distro::parse("ID=cachyos\nID_LIKE=arch\n", false), Distro::Arch);
    assert_eq!(Distro::parse("ID=linuxmint\nID_LIKE=\"ubuntu debian\"\n", false), Distro::Debian);
    assert_eq!(Distro::parse("ID=nobara\nID_LIKE=\"rhel centos fedora\"\n", false), Distro::Fedora);
    assert_eq!(Distro::parse("ID=\"opensuse-tumbleweed\"\nID_LIKE=\"opensuse suse\"\n", false), Distro::Suse);
    assert_eq!(Distro::parse("ID=bazzite\nID_LIKE=\"fedora\"\n", true), Distro::Other);
    assert_eq!(Distro::parse("ID=steamos\nID_LIKE=arch\n", false), Distro::Other);
}

#[test]
fn the_max_map_count_script_writes_the_file_and_applies_it() {
    let s = max_map_count_script();
    assert!(s.contains(SYSCTL_FILE));
    assert!(s.contains("sysctl -q -w vm.max_map_count=1048576"));
}

#[test]
fn what_can_be_fixed_comes_first() {
    let mut c = Case::new("0124:err:d3d12:d3d12_create_device Failed to create device");
    c.kind = RunKind::Wine;
    c.settings = wine();
    c.anticheat = anticheat("Denied", &["BattlEye"]);
    c.imports = vec!["d3d12.dll".into()];
    let f = c.run();
    assert_eq!(f[0].problem, Problem::Dx12WithoutVkd3d);
    assert!(matches!(f[1].problem, Problem::AntiCheat { .. }));
    assert!(opens_by_itself(&f));
}
