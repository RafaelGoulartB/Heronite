//! Amazon Games. The `fuel.json` of the install folder names the program,
//! its arguments and working folder (`Main`) and the installers to run once
//! (`PostInstall`); the game runs with the variables the Amazon Games SDK
//! reads, which nile's config folder provides: as it is on Windows, through
//! Wine/Proton on Linux ([`crate::host::run_mode`]).

use heronite_data::models::GameInfo;
use heronite_i18n::tr;
use serde_json::Value;

use crate::emulators::node_dirname;
#[cfg(not(windows))]
use crate::env::join;
use crate::host::{run_mode, unavailable_reason, Os, RunMode};
use crate::legendary::{cached_umu_id, known_fixes_env, known_fixes_steps};
use crate::plan::{split_args, windows_plan, FirstLaunch, LaunchOptions, LaunchPlan, PlanContext, SetupStep, WineRuntime};
use crate::probe::Probe;
use crate::settings::GameSettings;
use crate::wine;

/// What `fuel.json` says.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Fuel {
    /// Relative to the install folder, `/`-separated.
    pub command: String,
    pub args: Vec<String>,
    /// Relative to the install folder; `None` = the install folder.
    pub working_dir: Option<String>,
    /// `PostInstall`: program (as written) and arguments.
    pub post_install: Vec<(String, Vec<String>)>,
}

/// `fuel.json` is JSON5: drops `//` and `/* */` comments, then trailing
/// commas (all outside strings), so a JSON parser takes it.
fn relaxed_json(text: &str) -> String {
    trailing_commas(&outside_strings(text, |rest| {
        if rest.starts_with("//") {
            Some(rest.find('\n').unwrap_or(rest.len()))
        } else {
            rest.strip_prefix("/*").map(|body| body.find("*/").map_or(rest.len(), |end| end + 4))
        }
    }))
}

fn trailing_commas(text: &str) -> String {
    outside_strings(text, |rest| {
        let after = rest.strip_prefix(',')?.trim_start();
        (after.starts_with('}') || after.starts_with(']')).then_some(1)
    })
}

/// Copies `text`, letting `skip` drop bytes (it returns how many) wherever a
/// string literal is not open.
fn outside_strings(text: &str, skip: impl Fn(&str) -> Option<usize>) -> String {
    let mut out = String::with_capacity(text.len());
    let (mut i, mut in_str, mut escaped) = (0, false, false);
    while let Some(c) = text[i..].chars().next() {
        if in_str {
            match c {
                _ if escaped => escaped = false,
                '\\' => escaped = true,
                '"' => in_str = false,
                _ => {}
            }
        } else if let Some(n) = skip(&text[i..]) {
            i += n;
            continue;
        } else if c == '"' {
            in_str = true;
        }
        out.push(c);
        i += c.len_utf8();
    }
    out
}

fn strings(v: Option<&Value>) -> Vec<String> {
    match v {
        Some(Value::Array(a)) => a.iter().filter_map(|s| s.as_str().map(str::to_string)).collect(),
        Some(Value::String(s)) => split_args(s),
        _ => Vec::new(),
    }
}

pub fn parse_fuel(text: &str) -> Option<Fuel> {
    let doc: Value = serde_json::from_str(&relaxed_json(text)).ok()?;
    let main = doc.get("Main")?;
    let command = main.get("Command")?.as_str()?.replace('\\', "/");
    if command.is_empty() {
        return None;
    }
    let post_install = doc
        .get("PostInstall")
        .and_then(Value::as_array)
        .map(|steps| {
            steps
                .iter()
                .filter_map(|s| Some((s.get("Command")?.as_str()?.to_string(), strings(s.get("Args")))))
                .filter(|(c, _)| !c.is_empty())
                .collect()
        })
        .unwrap_or_default();
    Some(Fuel {
        command,
        args: strings(main.get("Args")),
        working_dir: main.get("WorkingSubdirOverride").and_then(Value::as_str).filter(|d| !d.is_empty()).map(|d| d.replace('\\', "/")),
        post_install,
    })
}

/// nile's own folder (`$NILE_CONFIG_PATH/nile`).
fn nile_dir(ctx: &PlanContext) -> String {
    ctx.paths.nile_config_dir().join("nile").to_string_lossy().into_owned()
}

/// `rel` (`/`- or `\`-separated, from `fuel.json`) inside the folder
/// `base`. Linux: like Node's `path.join`. Windows: with `\`, `.` dropped and
/// `..` resolved, the drive kept.
fn path_in(base: &str, rel: &str) -> String {
    #[cfg(windows)]
    {
        use std::path::{Component, Path, PathBuf};
        let mut out = PathBuf::new();
        for comp in Path::new(base).join(rel.replace('/', "\\")).components() {
            match comp {
                Component::CurDir => {}
                Component::ParentDir => {
                    if !out.pop() {
                        out.push("..");
                    }
                }
                other => out.push(other.as_os_str()),
            }
        }
        out.to_string_lossy().into_owned()
    }
    #[cfg(not(windows))]
    {
        join(base, &rel.replace('\\', "/"))
    }
}

/// `(entitlement id, product SKU)` of `product_id` in nile's `library.json`.
fn entitlement(probe: &dyn Probe, nile_dir: &str, product_id: &str) -> Option<(String, String)> {
    let bytes = probe.read_head(&path_in(nile_dir, "library.json"), 256 << 20)?;
    let doc: Value = serde_json::from_slice(&bytes).ok()?;
    let entry = doc.as_array()?.iter().find(|e| e.pointer("/product/id").and_then(Value::as_str) == Some(product_id))?;
    let text = |p: &str| entry.pointer(p).and_then(Value::as_str).unwrap_or_default().to_string();
    Some((text("/id"), text("/product/sku")))
}

fn display_name(probe: &dyn Probe, nile_dir: &str) -> String {
    probe
        .read_head(&path_in(nile_dir, "current_user.json"), 1 << 16)
        .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
        .and_then(|d| d.get("name").and_then(Value::as_str).map(str::to_string))
        .unwrap_or_default()
}

/// The variables the Amazon Games SDK (FuelPump) reads, as `nile launch`
/// sets them: the SDK nile downloads into its folder, the entitlement and
/// SKU of the game and the user's name. No sign-in token: `nile launch`
/// passes none either.
fn sdk_env(ctx: &PlanContext, probe: &dyn Probe, app: &str) -> Vec<(String, String)> {
    let dir = nile_dir(ctx);
    let sdk = path_in(&dir, "SDK/Amazon Games Services");
    let (entitlement_id, sku) = entitlement(probe, &dir, app).unwrap_or_default();
    vec![
        ("FUEL_DIR".into(), path_in(&sdk, "Legacy")),
        ("AMAZON_GAMES_SDK_PATH".into(), path_in(&sdk, "AmazonGamesSDK")),
        ("AMAZON_GAMES_FUEL_ENTITLEMENT_ID".into(), entitlement_id),
        ("AMAZON_GAMES_FUEL_PRODUCT_SKU".into(), sku),
        ("AMAZON_GAMES_FUEL_DISPLAY_NAME".into(), display_name(probe, &dir)),
        ("NILE_CONFIG_PATH".into(), ctx.paths.nile_config_dir().to_string_lossy().into_owned()),
    ]
}

/// A `PostInstall` program: relative ones live in the install folder.
fn setup_program(probe: &dyn Probe, install_path: &str, command: &str) -> String {
    let rel = path_in(install_path, command);
    if probe.exists(&rel) {
        rel
    } else {
        command.to_string()
    }
}

/// The `PostInstall` steps, started in the install folder.
fn post_install_steps(probe: &dyn Probe, install_path: &str, fuel: &Fuel) -> Vec<SetupStep> {
    fuel.post_install
        .iter()
        .map(|(command, args)| {
            let program = setup_program(probe, install_path, command);
            // Windows starts an `.msi` package through its installer only.
            let mut parts = if cfg!(windows) && program.to_lowercase().ends_with(".msi") {
                vec!["msiexec".to_string(), "/i".to_string(), program]
            } else {
                vec![program]
            };
            parts.extend(args.iter().cloned());
            SetupStep { parts, cwd: Some(install_path.to_string()), ..Default::default() }
        })
        .collect()
}

pub fn build(ctx: &PlanContext, probe: &dyn Probe, game: &GameInfo, settings: &GameSettings, opts: &LaunchOptions) -> LaunchPlan {
    let app = game.app_name.as_str();
    if !game.is_installed {
        return LaunchPlan::Unavailable(tr!("This Amazon game is not installed").into());
    }
    // Amazon Games has Windows builds only.
    let mode = run_mode(Os::Windows);
    if mode == RunMode::Unavailable {
        return LaunchPlan::Unavailable(unavailable_reason(Os::Windows));
    }
    let install_path = game.install.install_path.as_deref().unwrap_or("");
    if install_path.is_empty() || !probe.exists(install_path) {
        return LaunchPlan::Unavailable(tr!("Install folder not found: {install_path}", install_path));
    }
    let fuel_path = path_in(install_path, "fuel.json");
    let Some(fuel) = probe.read_head(&fuel_path, 4 << 20).and_then(|b| parse_fuel(&String::from_utf8_lossy(&b))) else {
        return LaunchPlan::Unavailable(tr!("The game's launch information was not found: {path}", path = fuel_path));
    };
    let override_exe =
        opts.alt_exe.clone().filter(|e| !e.is_empty()).or_else(|| Some(settings.target_exe.clone()).filter(|e| !e.is_empty()));
    let (exe, mut args, cwd) = match override_exe {
        Some(exe) => {
            let cwd = node_dirname(&exe);
            (exe, Vec::new(), cwd)
        }
        None => {
            let cwd = fuel.working_dir.as_deref().map(|d| path_in(install_path, d)).unwrap_or_else(|| install_path.to_string());
            (path_in(install_path, &fuel.command), fuel.args.clone(), cwd)
        }
    };
    args.extend(split_args(&settings.launcher_args));
    args.extend(opts.extra_args.iter().cloned());

    if mode == RunMode::Native {
        return native_windows(ctx, probe, game, settings, install_path, &fuel, &exe, args, cwd);
    }

    let mut plan = wine::build_wine_plan(ctx, probe, game, settings, &exe, &args);
    if let LaunchPlan::Wine { process, runtime, .. } = &mut plan {
        process.command.cwd = Some(cwd);
        let env = &mut process.command.env;
        env.extend(known_fixes_env(ctx, "amazon", app));
        if *runtime == WineRuntime::Umu {
            if let Some(id) = cached_umu_id(ctx, "nile", app) {
                env.insert("GAMEID".into(), id);
            }
        }
        env.extend(sdk_env(ctx, probe, app));

        // The game's installers (`PostInstall`) and known fixes: once per game and prefix.
        let (winetricks, run_in_prefix) = known_fixes_steps(ctx, "amazon", app);
        process.prep.first_launch = Some(FirstLaunch {
            runner: "nile".into(),
            app_name: app.to_string(),
            install_path: install_path.to_string(),
            platform: "Windows".into(),
            winetricks,
            run_in_prefix,
            post_install: post_install_steps(probe, install_path, &fuel),
            ..Default::default()
        });
    }
    plan
}

/// The game on Windows: `fuel.json`'s program started as it is, from its
/// working folder, with the SDK's variables, like a game added by hand (in
/// the session's job object, which follows every process it starts). That
/// is what `nile launch` does on Windows, without a nile process (and its
/// SDK check online) between the session and the game. The `PostInstall`
/// installers run once on this PC first ([`crate::native_setup`]); Wine's
/// known fixes do not apply.
#[allow(clippy::too_many_arguments)]
fn native_windows(
    ctx: &PlanContext,
    probe: &dyn Probe,
    game: &GameInfo,
    settings: &GameSettings,
    install_path: &str,
    fuel: &Fuel,
    exe: &str,
    args: Vec<String>,
    cwd: String,
) -> LaunchPlan {
    let app = game.app_name.as_str();
    let mut plan = windows_plan(probe, game, settings, exe, args, "nile");
    if let LaunchPlan::Native(process) = &mut plan {
        if probe.exists(&cwd) {
            process.command.cwd = Some(cwd);
        }
        // The game's own variables (`enviromentOptions`) win.
        let own: Vec<String> = settings.env_options.iter().map(|(k, _)| k.clone()).collect();
        for (key, value) in sdk_env(ctx, probe, app) {
            if !own.contains(&key) {
                process.command.env.insert(key, value);
            }
        }
        process.prep.first_launch = Some(FirstLaunch {
            runner: "nile".into(),
            app_name: app.to_string(),
            install_path: install_path.to_string(),
            platform: "Windows".into(),
            post_install: post_install_steps(probe, install_path, fuel),
            ..Default::default()
        });
    }
    plan
}

#[cfg(test)]
mod tests {
    use super::*;

    const FUEL: &str = r#"{
        // Written by Amazon's tools
        "SchemaVersion": "2",
        "Main": {
            "Command": "Binaries\\Win64\\Game.exe",
            "Args": ["-windowed", "-url=http://x/y"],
            "WorkingSubdirOverride": "Binaries\\Win64",
        },
        "PostInstall": [
            {"Command": "_redist\\vcredist_x64.exe", "Args": ["/quiet", "/norestart"], "ValidReturns": [0, 3010]},
            /* empty */
        ],
    }"#;

    #[test]
    fn fuel_json5_is_read() {
        let f = parse_fuel(FUEL).unwrap();
        assert_eq!(f.command, "Binaries/Win64/Game.exe");
        assert_eq!(f.args, ["-windowed", "-url=http://x/y"]);
        assert_eq!(f.working_dir.as_deref(), Some("Binaries/Win64"));
        assert_eq!(f.post_install, [("_redist\\vcredist_x64.exe".to_string(), vec!["/quiet".to_string(), "/norestart".to_string()])]);
    }

    #[test]
    fn plain_fuel_without_extras() {
        let f = parse_fuel(r#"{"Main": {"Command": "Game.exe"}}"#).unwrap();
        assert_eq!((f.command.as_str(), f.args.len(), f.working_dir, f.post_install.len()), ("Game.exe", 0, None, 0));
        assert!(parse_fuel(r#"{"Main": {}}"#).is_none());
        assert!(parse_fuel("nope").is_none());
    }

    #[test]
    fn comments_inside_strings_survive() {
        assert_eq!(relaxed_json(r#"{"a": "http://x//y", "b": [1, 2,],}"#), r#"{"a": "http://x//y", "b": [1, 2]}"#);
    }

    #[cfg(windows)]
    mod windows {
        use super::*;
        use crate::probe::FakeProbe;
        use heronite_data::models::InstallInfo;
        use heronite_data::DataPaths;
        use serde_json::json;

        const DIR: &str = r"D:\Games\Cool Game";

        fn ctx() -> PlanContext {
            PlanContext::load(&DataPaths::at(r"C:\hr"))
        }

        fn game() -> GameInfo {
            GameInfo {
                runner: "nile".into(),
                app_name: "amzn1.p".into(),
                title: "Cool Game".into(),
                is_installed: true,
                install: InstallInfo { install_path: Some(DIR.into()), platform: Some("Windows".into()), ..Default::default() },
                ..Default::default()
            }
        }

        fn settings(v: serde_json::Value) -> GameSettings {
            GameSettings::from_map(v.as_object().unwrap(), r"C:\Users\u")
        }

        fn probe() -> FakeProbe {
            FakeProbe::new(r"C:\Users\u")
                .with_path(DIR)
                .with_path(r"D:\Games\Cool Game\Binaries\Win64")
                .with_path(r"D:\Games\Cool Game\Binaries\Win64\Game.exe")
                .with_path(r"D:\Games\Cool Game\_redist\vcredist_x64.exe")
                .with_file(r"D:\Games\Cool Game\fuel.json", FUEL.as_bytes())
                .with_file(r"C:\hr\nile_config\nile\library.json", br#"[{"id": "ent-1", "product": {"id": "amzn1.p", "sku": "SKU9"}}]"#)
                .with_file(r"C:\hr\nile_config\nile\current_user.json", br#"{"name": "Ana", "user_id": "u"}"#)
        }

        #[test]
        fn paths_inside_the_install_folder_keep_the_drive() {
            assert_eq!(path_in(DIR, "Binaries/Win64/Game.exe"), r"D:\Games\Cool Game\Binaries\Win64\Game.exe");
            assert_eq!(path_in(DIR, r".\bin\..\Game.exe"), r"D:\Games\Cool Game\Game.exe");
            assert_eq!(
                path_in(r"C:\hr\nile_config\nile", "SDK/Amazon Games Services"),
                r"C:\hr\nile_config\nile\SDK\Amazon Games Services"
            );
        }

        #[test]
        fn the_fuel_command_runs_as_it_is_with_the_sdk_variables() {
            let s = settings(
                json!({"launcherArgs": "-skipintro", "enviromentOptions": [{"key": "AMAZON_GAMES_FUEL_DISPLAY_NAME", "value": "Me"}]}),
            );
            let plan = build(&ctx(), &probe(), &game(), &s, &LaunchOptions { extra_args: vec!["-x".into()], ..Default::default() });
            let LaunchPlan::Native(p) = plan else { panic!("{plan:?}") };
            assert_eq!(p.command.program, r"D:\Games\Cool Game\Binaries\Win64\Game.exe");
            assert_eq!(p.command.args, ["-windowed", "-url=http://x/y", "-skipintro", "-x"]);
            assert_eq!(p.command.cwd.as_deref(), Some(r"D:\Games\Cool Game\Binaries\Win64"));
            assert_eq!(p.stop.kill_patterns, ["Game.exe"]);
            let env = &p.command.env;
            assert_eq!(env["FUEL_DIR"], r"C:\hr\nile_config\nile\SDK\Amazon Games Services\Legacy");
            assert_eq!(env["AMAZON_GAMES_SDK_PATH"], r"C:\hr\nile_config\nile\SDK\Amazon Games Services\AmazonGamesSDK");
            assert_eq!(env["AMAZON_GAMES_FUEL_ENTITLEMENT_ID"], "ent-1");
            assert_eq!(env["AMAZON_GAMES_FUEL_PRODUCT_SKU"], "SKU9");
            // The game's own variable wins.
            assert_eq!(env["AMAZON_GAMES_FUEL_DISPLAY_NAME"], "Me");
            assert_eq!(env["NILE_CONFIG_PATH"], r"C:\hr\nile_config");
            assert_eq!(env["HEROIC_APP_SOURCE"], "amazon");
            assert!(!env.contains_key("WINEPREFIX"));
            assert!(p.prep.wine.is_none(), "no Wine on Windows");

            // The installers run once, natively, from the install folder.
            let first = p.prep.first_launch.expect("first launch setup");
            assert_eq!((first.runner.as_str(), first.app_name.as_str(), first.install_path.as_str()), ("nile", "amzn1.p", DIR));
            assert_eq!(first.post_install.len(), 1);
            assert_eq!(first.post_install[0].parts, [r"D:\Games\Cool Game\_redist\vcredist_x64.exe", "/quiet", "/norestart"]);
            assert_eq!(first.post_install[0].cwd.as_deref(), Some(DIR));
            assert!(!first.post_install[0].elevate, "Windows asks for administrator rights when the installer needs them");
            assert!(first.winetricks.is_empty() && first.run_in_prefix.is_empty());
        }

        #[test]
        fn an_msi_package_runs_through_the_windows_installer() {
            let fuel = r#"{"Main": {"Command": "Game.exe"}, "PostInstall": [{"Command": "redist\\PhysX.msi", "Args": ["/qn"]}]}"#;
            let probe = FakeProbe::new(r"C:\Users\u")
                .with_path(DIR)
                .with_path(r"D:\Games\Cool Game\Game.exe")
                .with_path(r"D:\Games\Cool Game\redist\PhysX.msi")
                .with_file(r"D:\Games\Cool Game\fuel.json", fuel.as_bytes());
            let LaunchPlan::Native(p) = build(&ctx(), &probe, &game(), &settings(json!({})), &Default::default()) else { panic!() };
            let first = p.prep.first_launch.unwrap();
            assert_eq!(first.post_install[0].parts, ["msiexec", "/i", r"D:\Games\Cool Game\redist\PhysX.msi", "/qn"]);
        }

        #[test]
        fn a_chosen_program_replaces_the_fuel_command() {
            let s = settings(json!({"targetExe": r"D:\Games\Cool Game\Tools\Editor.exe"}));
            let probe = probe().with_path(r"D:\Games\Cool Game\Tools").with_path(r"D:\Games\Cool Game\Tools\Editor.exe");
            let LaunchPlan::Native(p) = build(&ctx(), &probe, &game(), &s, &Default::default()) else { panic!() };
            assert_eq!(p.command.program, r"D:\Games\Cool Game\Tools\Editor.exe");
            assert!(p.command.args.is_empty());
            assert_eq!(p.command.cwd.as_deref(), Some(r"D:\Games\Cool Game\Tools"));
        }

        #[test]
        fn what_is_missing_is_said() {
            let s = settings(json!({}));
            let no_exe = FakeProbe::new(r"C:\Users\u").with_path(DIR).with_file(r"D:\Games\Cool Game\fuel.json", FUEL.as_bytes());
            let plan = build(&ctx(), &no_exe, &game(), &s, &Default::default());
            assert!(plan.unavailable_reason().unwrap().contains("Game.exe"), "{plan:?}");
            let no_fuel = FakeProbe::new(r"C:\Users\u").with_path(DIR);
            let plan = build(&ctx(), &no_fuel, &game(), &s, &Default::default());
            assert!(plan.unavailable_reason().unwrap().contains(r"D:\Games\Cool Game\fuel.json"), "{plan:?}");
            let plan = build(&ctx(), &FakeProbe::new(r"C:\Users\u"), &game(), &s, &Default::default());
            assert!(plan.unavailable_reason().unwrap().contains("Install folder not found"), "{plan:?}");
        }
    }
}
