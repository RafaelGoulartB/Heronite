//! Amazon Games. The `fuel.json` of the install folder names the program,
//! its arguments and working folder (`Main`) and the installers to run once
//! (`PostInstall`); the game runs through Wine/Proton with the variables the
//! Amazon Games SDK reads, which nile's config folder provides.

use heronite_data::models::GameInfo;
use heronite_i18n::tr;
use serde_json::Value;

use crate::emulators::node_dirname;
use crate::env::join;
use crate::legendary::{cached_umu_id, known_fixes_env, known_fixes_steps};
use crate::plan::{split_args, FirstLaunch, LaunchOptions, LaunchPlan, PlanContext, SetupStep, WineRuntime};
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

/// `(entitlement id, product SKU)` of `product_id` in nile's `library.json`.
fn entitlement(probe: &dyn Probe, nile_dir: &str, product_id: &str) -> Option<(String, String)> {
    let bytes = probe.read_head(&join(nile_dir, "library.json"), 256 << 20)?;
    let doc: Value = serde_json::from_slice(&bytes).ok()?;
    let entry = doc.as_array()?.iter().find(|e| e.pointer("/product/id").and_then(Value::as_str) == Some(product_id))?;
    let text = |p: &str| entry.pointer(p).and_then(Value::as_str).unwrap_or_default().to_string();
    Some((text("/id"), text("/product/sku")))
}

fn display_name(probe: &dyn Probe, nile_dir: &str) -> String {
    probe
        .read_head(&join(nile_dir, "current_user.json"), 1 << 16)
        .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
        .and_then(|d| d.get("name").and_then(Value::as_str).map(str::to_string))
        .unwrap_or_default()
}

/// A `PostInstall` program: relative ones live in the install folder.
fn setup_program(probe: &dyn Probe, install_path: &str, command: &str) -> String {
    let rel = join(install_path, &command.replace('\\', "/"));
    if probe.exists(&rel) {
        rel
    } else {
        command.to_string()
    }
}

pub fn build(ctx: &PlanContext, probe: &dyn Probe, game: &GameInfo, settings: &GameSettings, opts: &LaunchOptions) -> LaunchPlan {
    let app = game.app_name.as_str();
    if !game.is_installed {
        return LaunchPlan::Unavailable(tr!("This Amazon game is not installed").into());
    }
    let install_path = game.install.install_path.as_deref().unwrap_or("");
    if install_path.is_empty() || !probe.exists(install_path) {
        return LaunchPlan::Unavailable(tr!("Install folder not found: {install_path}", install_path));
    }
    let fuel_path = join(install_path, "fuel.json");
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
            let cwd = fuel.working_dir.as_deref().map(|d| join(install_path, d)).unwrap_or_else(|| install_path.to_string());
            (join(install_path, &fuel.command), fuel.args.clone(), cwd)
        }
    };
    args.extend(split_args(&settings.launcher_args));
    args.extend(opts.extra_args.iter().cloned());

    let mut plan = wine::build_wine_plan(ctx, probe, game, settings, &exe, &args);
    if let LaunchPlan::Wine { process, runtime, .. } = &mut plan {
        process.command.cwd = Some(cwd);
        let dir = nile_dir(ctx);
        let sdk = join(&dir, "SDK/Amazon Games Services");
        let env = &mut process.command.env;
        env.extend(known_fixes_env(ctx, "amazon", app));
        if *runtime == WineRuntime::Umu {
            if let Some(id) = cached_umu_id(ctx, "nile", app) {
                env.insert("GAMEID".into(), id);
            }
        }
        let (entitlement_id, sku) = entitlement(probe, &dir, app).unwrap_or_default();
        env.insert("FUEL_DIR".into(), join(&sdk, "Legacy"));
        env.insert("AMAZON_GAMES_SDK_PATH".into(), join(&sdk, "AmazonGamesSDK"));
        env.insert("AMAZON_GAMES_FUEL_ENTITLEMENT_ID".into(), entitlement_id);
        env.insert("AMAZON_GAMES_FUEL_PRODUCT_SKU".into(), sku);
        env.insert("AMAZON_GAMES_FUEL_DISPLAY_NAME".into(), display_name(probe, &dir));
        env.insert("NILE_CONFIG_PATH".into(), ctx.paths.nile_config_dir().to_string_lossy().into_owned());

        // The game's installers (`PostInstall`) and known fixes: once per game and prefix.
        let (winetricks, run_in_prefix) = known_fixes_steps(ctx, "amazon", app);
        let post_install = fuel
            .post_install
            .iter()
            .map(|(command, args)| {
                let mut parts = vec![setup_program(probe, install_path, command)];
                parts.extend(args.iter().cloned());
                SetupStep { parts, cwd: Some(install_path.to_string()) }
            })
            .collect();
        process.prep.first_launch = Some(FirstLaunch {
            runner: "nile".into(),
            app_name: app.to_string(),
            install_path: install_path.to_string(),
            platform: "Windows".into(),
            winetricks,
            run_in_prefix,
            post_install,
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
}
