//! A Steam game's launch options (`LaunchOptions` in `localconfig.vdf`) as
//! the game settings the panel edits, and back.
//!
//! The line is where these settings live: it is read back from Steam every
//! time, so a change made in Steam shows up here, and nothing is kept on our
//! side. The line this module writes has the shape
//!
//! ```text
//! [VAR=value ...] [heronite-steam-wrap --before a --after b --] [gamescope ... --] [wrapper ...] [mangohud] [gamemoderun] %command% [arguments]
//! ```
//!
//! and any line of that shape (in any wrapper order) reads back. Variables
//! and wrappers we do not know are kept as environment variables and
//! wrappers. A line using shell syntax (`;`, `$VAR`, redirections...) is not
//! [`Parsed::editable`]: rebuilding it from fields would change what it does.
//!
//! Setting keys are Heroic's where the meaning is the same (`showMangohud`,
//! `gamescope`, `enviromentOptions`...), plus [`KEY_NVAPI`],
//! [`KEY_WINED3D`], the frame generation keys of [`lsfg`] (written as
//! lsfg-vk 2's variables) and the FPS limit of [`fps_limit`] (written as the
//! variables or gamescope options of its limiter; the line only says which
//! limiter, so it reads back as that one, never as "automatic"), and the
//! graphics card settings of [`gpu`] (their variables).

use heronite_launch::env::{gamescope_args, DXVK_FRAME_RATE};
use heronite_launch::fps_limit::{self, FpsLimit, Method, Tools};
use heronite_launch::gpu;
use heronite_launch::lsfg::{self, Layer};
use heronite_launch::settings::GameSettings;
use serde_json::{json, Map, Value};

pub type Settings = Map<String, Value>;

/// Steam's placeholder for the game's own command.
pub const COMMAND: &str = "%command%";
/// File name of the script that runs the before/after scripts ([`crate::wrap`]).
pub const WRAP_NAME: &str = "heronite-steam-wrap";

/// `PROTON_ENABLE_NVAPI=1`.
pub const KEY_NVAPI: &str = "steamEnableNvapi";
/// `PROTON_USE_WINED3D=1` (OpenGL instead of DXVK/VKD3D).
pub const KEY_WINED3D: &str = "steamUseWined3d";

/// Switches set by `VAR=1`.
const SWITCHES: [(&str, &str); 7] = [
    ("enableWineWayland", "PROTON_ENABLE_WAYLAND"),
    ("enableHDR", "PROTON_ENABLE_HDR"),
    ("enableWoW64", "PROTON_USE_WOW64"),
    (KEY_NVAPI, "PROTON_ENABLE_NVAPI"),
    (KEY_WINED3D, "PROTON_USE_WINED3D"),
    ("verboseLogs", "PROTON_LOG"),
    (heronite_launch::env::KEY_PREFER_SDL, "PROTON_PREFER_SDL"),
];
/// Switches that are on unless `VAR=1`.
const INVERTED: [(&str, &str); 2] = [("enableEsync", "PROTON_NO_ESYNC"), ("enableFsync", "PROTON_NO_FSYNC")];
/// What "use the dedicated GPU" sets (`setupEnvVars` with `nvidiaPrime`).
const PRIME: [(&str, &str); 3] = gpu::PRIME;
const DXVK_HUD: &str = "DXVK_HUD";
const MANGOHUD_CONFIG: &str = "MANGOHUD_CONFIG";
const KNOWN_WRAPPERS: [&str; 4] = [WRAP_NAME, "gamescope", "mangohud", "gamemoderun"];

#[derive(Debug, Clone, PartialEq)]
pub struct Parsed {
    /// Every key the Steam tabs show, set (off when the line does not say).
    pub settings: Settings,
    /// `false`: the line uses shell syntax and must only be edited as text.
    pub editable: bool,
}

/// The settings of an empty line: Steam's defaults.
pub fn defaults() -> Settings {
    let mut s = Settings::new();
    for (key, _) in SWITCHES {
        s.insert(key.into(), Value::Bool(false));
    }
    for (key, _) in INVERTED {
        s.insert(key.into(), Value::Bool(true));
    }
    for key in ["showFps", "nvidiaPrime", "showMangohud", "useGameMode", fps_limit::KEY_SMOOTH] {
        s.insert(key.into(), Value::Bool(false));
    }
    for key in [fps_limit::KEY_LIMIT, fps_limit::KEY_NO_FOCUS, "launcherArgs", "beforeLaunchScriptPath", "afterLaunchScriptPath"] {
        s.insert(key.into(), Value::String(String::new()));
    }
    s.insert(fps_limit::KEY_METHOD.into(), Value::String(Method::Auto.id().into()));
    s.insert("enviromentOptions".into(), json!([]));
    s.insert("wrapperOptions".into(), json!([]));
    s.insert("gamescope".into(), gamescope_value(&Gamescope::default()));
    for key in [lsfg::KEY_ENABLED, lsfg::KEY_PERFORMANCE, lsfg::KEY_HDR] {
        s.insert(key.into(), Value::Bool(false));
    }
    s.insert(lsfg::KEY_MULTIPLIER.into(), json!(lsfg::DEFAULT_MULTIPLIER));
    s.insert(lsfg::KEY_FLOW_SCALE.into(), json!(lsfg::DEFAULT_FLOW_SCALE));
    for key in [lsfg::KEY_PROFILE, lsfg::KEY_DLL] {
        s.insert(key.into(), Value::String(String::new()));
    }
    gpu::launch_options_defaults(&mut s);
    s
}

/// Reads a launch options line.
pub fn parse(line: &str) -> Parsed {
    let mut s = defaults();
    let line = line.trim();
    if line.is_empty() {
        return Parsed { settings: s, editable: true };
    }
    let not_editable = |s| Parsed { settings: s, editable: false };
    if uses_shell_syntax(line) {
        return not_editable(s);
    }
    let Some(tokens) = shlex::split(line) else { return not_editable(s) };
    let commands: Vec<usize> = tokens.iter().enumerate().filter(|(_, t)| t.contains(COMMAND)).map(|(i, _)| i).collect();
    let at = match commands.as_slice() {
        // Without `%command%` Steam appends the line to the game's command.
        [] => {
            s.insert("launcherArgs".into(), Value::String(join(&tokens)));
            return Parsed { settings: s, editable: true };
        }
        [i] if tokens[*i] == COMMAND => *i,
        _ => return not_editable(s),
    };
    s.insert("launcherArgs".into(), Value::String(join(&tokens[at + 1..])));
    let pre = &tokens[..at];
    let env_end = pre.iter().position(|t| assignment(t).is_none()).unwrap_or(pre.len());
    let env: Vec<(String, String)> = pre[..env_end].iter().filter_map(|t| assignment(t)).collect();
    let Some(gamescope_on) = read_wrappers(&pre[env_end..], &mut s) else { return not_editable(s) };
    read_env(env, gamescope_on, &mut s);
    Parsed { settings: s, editable: true }
}

/// `KEY=value` with a valid variable name.
fn assignment(token: &str) -> Option<(String, String)> {
    let (key, value) = token.split_once('=')?;
    let mut chars = key.chars();
    let first = chars.next()?;
    ((first.is_ascii_alphabetic() || first == '_') && chars.all(|c| c.is_ascii_alphanumeric() || c == '_'))
        .then(|| (key.to_string(), value.to_string()))
}

fn base_name(token: &str) -> &str {
    token.rsplit('/').next().unwrap_or(token)
}

/// Reads the wrappers before `%command%` into `s`. `Some(gamescope is on)`,
/// `None` when they do not read back (our script or gamescope without `--`).
fn read_wrappers(tokens: &[String], s: &mut Settings) -> Option<bool> {
    let mut wrappers: Vec<Value> = Vec::new();
    let mut gamescope_on = false;
    let mut i = 0;
    while i < tokens.len() {
        match base_name(&tokens[i]) {
            WRAP_NAME => {
                i += 1;
                loop {
                    match tokens.get(i).map(String::as_str) {
                        Some("--") => break,
                        Some(flag @ ("--before" | "--after")) => {
                            let key = if flag == "--before" { "beforeLaunchScriptPath" } else { "afterLaunchScriptPath" };
                            s.insert(key.into(), Value::String(tokens.get(i + 1)?.clone()));
                            i += 2;
                        }
                        _ => return None,
                    }
                }
                i += 1;
            }
            "gamescope" => {
                let end = i + 1 + tokens[i + 1..].iter().position(|t| t == "--")?;
                let (g, mangoapp, limit) = read_gamescope(&tokens[i + 1..end]);
                if mangoapp {
                    s.insert("showMangohud".into(), Value::Bool(true));
                }
                if let Some((fps, no_focus)) = limit {
                    set_limit(s, Method::Gamescope, fps, false);
                    s.insert(fps_limit::KEY_NO_FOCUS.into(), Value::String(no_focus));
                }
                s.insert("gamescope".into(), gamescope_value(&g));
                gamescope_on = true;
                i = end + 1;
            }
            "mangohud" => {
                s.insert("showMangohud".into(), Value::Bool(true));
                i += 1;
                if tokens.get(i).is_some_and(|t| t == "--dlsym") {
                    i += 1;
                }
            }
            "gamemoderun" => {
                s.insert("useGameMode".into(), Value::Bool(true));
                i += 1;
            }
            // `VAR=x` after a wrapper is a word of that wrapper; written
            // first it would become a variable of the whole line.
            _ if assignment(&tokens[i]).is_some() => return None,
            _ => {
                let end = tokens[i + 1..].iter().position(|t| KNOWN_WRAPPERS.contains(&base_name(t))).map_or(tokens.len(), |p| i + 1 + p);
                wrappers.push(json!({"exe": tokens[i], "args": join(&tokens[i + 1..end])}));
                i = end;
            }
        }
    }
    s.insert("wrapperOptions".into(), Value::Array(wrappers));
    Some(gamescope_on)
}

/// Sorts the leading variables into switches and plain variables.
fn read_env(mut env: Vec<(String, String)>, gamescope_on: bool, s: &mut Settings) {
    read_lsfg(&mut env, s);
    gpu::read_launch_options(&mut env, s);
    let has = |k: &str, v: &str| env.iter().any(|(key, value)| key == k && value == v);
    let prime = PRIME.iter().all(|(k, v)| has(k, v));
    let mangohud_on = s.get("showMangohud") == Some(&Value::Bool(true));
    // `DXVK_FRAME_RATE` and `VKD3D_FRAME_RATE` with the same value are one limit.
    let mut dxvk: Option<String> = None;
    let mut rest: Vec<Value> = Vec::new();
    for (key, value) in env {
        let one = value == "1";
        if let Some((name, _)) = SWITCHES.iter().find(|(_, var)| *var == key).filter(|_| one) {
            s.insert((*name).into(), Value::Bool(true));
        } else if let Some((name, _)) = INVERTED.iter().find(|(_, var)| *var == key).filter(|_| one) {
            s.insert((*name).into(), Value::Bool(false));
        } else if key == DXVK_HUD && value == "fps" {
            s.insert("showFps".into(), Value::Bool(true));
        } else if DXVK_FRAME_RATE.contains(&key.as_str()) && is_number(&value) && (dxvk.as_ref() == Some(&value) || !limited(s)) {
            set_limit(s, Method::Dxvk, value.clone(), false);
            dxvk = Some(value);
        } else if let Some((limit, smooth, hud)) =
            (key == MANGOHUD_CONFIG && mangohud_on && !limited(s)).then(|| fps_limit::read_mangohud_config(&value)).flatten()
        {
            set_limit(s, Method::MangoHud, limit.to_string(), smooth);
            s.insert("showMangohud".into(), Value::Bool(hud));
        } else if prime && PRIME.iter().any(|(k, v)| *k == key && *v == value) {
            s.insert("nvidiaPrime".into(), Value::Bool(true));
        } else {
            rest.push(json!({"key": key, "value": value}));
        }
    }
    s.insert("enviromentOptions".into(), Value::Array(rest));
    if !gamescope_on {
        s.insert("gamescope".into(), gamescope_value(&Gamescope::default()));
    }
}

/// Takes the frame generation variables this module writes out of `env`:
/// `LSFGVK_ENV=1` with the values, or `LSFGVK_PROFILE`. A value the panel
/// cannot show stays a plain variable.
fn read_lsfg(env: &mut Vec<(String, String)>, s: &mut Settings) {
    let get = |k: &str| env.iter().find(|(key, _)| key == k).map(|(_, v)| v.clone());
    let profile = get("LSFGVK_PROFILE").filter(|p| !p.is_empty());
    let by_env = get("LSFGVK_ENV").as_deref() == Some("1");
    if !by_env && profile.is_none() {
        return;
    }
    let mut taken: Vec<&str> = vec!["LSFGVK_ENV", "LSFGVK_PROFILE"];
    s.insert(lsfg::KEY_ENABLED.into(), Value::Bool(true));
    if let Some(dll) = get("LSFGVK_DLL_PATH").filter(|d| !d.is_empty()) {
        s.insert(lsfg::KEY_DLL.into(), Value::String(dll));
        taken.push("LSFGVK_DLL_PATH");
    }
    match profile {
        // With both, lsfg-vk uses the variables.
        Some(p) if !by_env => {
            s.insert(lsfg::KEY_PROFILE.into(), Value::String(p));
        }
        _ => {
            if let Some(n) = get("LSFGVK_MULTIPLIER").and_then(|v| v.parse::<u32>().ok()).filter(|n| lsfg::MULTIPLIERS.contains(n)) {
                s.insert(lsfg::KEY_MULTIPLIER.into(), json!(n));
                taken.push("LSFGVK_MULTIPLIER");
            }
            let flow = get("LSFGVK_FLOW_SCALE").and_then(|v| v.parse::<f32>().ok()).map(|f| (f * 100.0).round() as u32);
            if let Some(pct) = flow.filter(|p| (25..=100).contains(p)) {
                s.insert(lsfg::KEY_FLOW_SCALE.into(), json!(pct));
                taken.push("LSFGVK_FLOW_SCALE");
            }
            if let Some(v @ ("0" | "1")) = get("LSFGVK_PERFORMANCE_MODE").as_deref() {
                s.insert(lsfg::KEY_PERFORMANCE.into(), Value::Bool(v == "1"));
                taken.push("LSFGVK_PERFORMANCE_MODE");
            }
            if get("LSFGVK_COLOR_SPACE").as_deref() == Some("hdr10") && get("LSFGVK_TRANSFER_FUNCTION").as_deref() == Some("pq") {
                s.insert(lsfg::KEY_HDR.into(), Value::Bool(true));
                taken.extend(["LSFGVK_COLOR_SPACE", "LSFGVK_TRANSFER_FUNCTION"]);
            }
        }
    }
    env.retain(|(k, _)| !taken.contains(&k.as_str()));
}

fn is_number(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit())
}

/// Whether the line already gave the FPS limit (a second limiter stays a
/// plain variable: the panel shows one).
fn limited(s: &Settings) -> bool {
    FpsLimit::from_map(s).is_on()
}

fn set_limit(s: &mut Settings, method: Method, fps: String, smooth: bool) {
    s.insert(fps_limit::KEY_LIMIT.into(), Value::String(fps));
    s.insert(fps_limit::KEY_METHOD.into(), Value::String(method.id().into()));
    s.insert(fps_limit::KEY_SMOOTH.into(), Value::Bool(smooth));
}

/// The gamescope part of the settings (Heroic's `gamescope` object).
#[derive(Debug, Clone, PartialEq)]
struct Gamescope {
    upscaling: bool,
    grab_cursor: bool,
    window_type: &'static str,
    game_width: String,
    game_height: String,
    upscale_width: String,
    upscale_height: String,
    method: String,
    additional: String,
}

impl Default for Gamescope {
    fn default() -> Self {
        Self {
            upscaling: false,
            grab_cursor: false,
            window_type: "fullscreen",
            game_width: String::new(),
            game_height: String::new(),
            upscale_width: String::new(),
            upscale_height: String::new(),
            method: "fsr".into(),
            additional: String::new(),
        }
    }
}

fn gamescope_value(g: &Gamescope) -> Value {
    json!({
        "enableUpscaling": g.upscaling,
        "enableLimiter": false,
        "enableForceGrabCursor": g.grab_cursor,
        "windowType": g.window_type,
        "gameWidth": g.game_width,
        "gameHeight": g.game_height,
        "upscaleWidth": g.upscale_width,
        "upscaleHeight": g.upscale_height,
        "upscaleMethod": g.method,
        "fpsLimiter": "",
        "fpsLimiterNoFocus": "",
        "additionalOptions": g.additional,
    })
}

/// Gamescope's options (between `gamescope` and `--`); `.1`: `--mangoapp`;
/// `.2`: its FPS limit and the one without focus (`-o` alone stays an
/// option of its own).
fn read_gamescope(args: &[String]) -> (Gamescope, bool, Option<(String, String)>) {
    let mut g = Gamescope { window_type: "windowed", method: String::new(), ..Default::default() };
    let (mut fps, mut fps_no_focus) = (String::new(), String::new());
    let mut mangoapp = false;
    let mut extra: Vec<String> = Vec::new();
    let mut i = 0;
    while i < args.len() {
        let value = args.get(i + 1).cloned();
        let field = match args[i].as_str() {
            "-w" => Some(&mut g.game_width),
            "-h" => Some(&mut g.game_height),
            "-W" => Some(&mut g.upscale_width),
            "-H" => Some(&mut g.upscale_height),
            "-r" => Some(&mut fps),
            "-o" => Some(&mut fps_no_focus),
            _ => None,
        };
        match (field, value.as_deref()) {
            (Some(field), Some(v)) if is_number(v) => {
                *field = v.to_string();
                i += 2;
                continue;
            }
            _ => {}
        }
        match (args[i].as_str(), value.as_deref()) {
            ("-F", Some(m @ ("fsr" | "nis"))) | ("-S", Some(m @ ("integer" | "stretch"))) => {
                g.method = m.to_string();
                i += 2;
                continue;
            }
            ("-f", _) => g.window_type = "fullscreen",
            ("-b", _) => g.window_type = "borderless",
            ("--force-grab-cursor", _) => g.grab_cursor = true,
            ("--mangoapp", _) => mangoapp = true,
            (other, _) => extra.push(other.to_string()),
        }
        i += 1;
    }
    let sized = [&g.game_width, &g.game_height, &g.upscale_width, &g.upscale_height].iter().any(|v| !v.is_empty());
    let limit = fps.parse::<u32>().is_ok_and(|n| n > 0);
    if !limit && !fps_no_focus.is_empty() {
        extra.extend(["-o".into(), fps_no_focus.clone()]);
    }
    if !limit && !fps.is_empty() {
        extra.extend(["-r".into(), fps.clone()]);
    }
    // gamescope runs for something else than the limit: keep it running.
    g.upscaling = sized || !g.method.is_empty() || g.window_type != "windowed" || !limit;
    g.additional = join(&extra);
    (g, mangoapp, limit.then_some((fps, fps_no_focus)))
}

/// What can limit the frame rate from a launch options line: the pick is
/// written as it is (Steam may be a Flatpak with tools of its own), except
/// MangoHud inside gamescope, which cannot limit.
fn steam_tools(g: &GameSettings) -> Tools {
    Tools { gamescope_runs: g.gamescope.enable_upscaling, gamescope: true, mangohud: true, dxvk: true }
}

/// The line for `settings`; `wrap` is the path of [`crate::wrap`]'s script.
/// Empty when everything is at Steam's defaults.
pub fn build(settings: &Settings, wrap: &str) -> String {
    build_with(settings, wrap, &heronite_launch::SystemProbe)
}

/// [`build`] on `probe`'s machine (the picked graphics card is read from it).
pub fn build_with(settings: &Settings, wrap: &str, probe: &dyn heronite_launch::Probe) -> String {
    let g = GameSettings::from_map(settings, "");
    let flag = |key: &str| settings.get(key) == Some(&Value::Bool(true));
    let limit = g.fps_limit.applied(&steam_tools(&g));
    let limit_by = |m: Method| limit.filter(|a| a.by(m));
    let mut out: Vec<String> = Vec::new();
    let mut set = |key: &str, value: &str| out.push(format!("{key}={}", quote(value)));
    for (key, var) in INVERTED {
        if settings.get(key) == Some(&Value::Bool(false)) {
            set(var, "1");
        }
    }
    for (key, var) in SWITCHES {
        if flag(key) {
            set(var, "1");
        }
    }
    if g.show_fps {
        set(DXVK_HUD, "fps");
    }
    if let Some(a) = limit_by(Method::Dxvk) {
        for var in DXVK_FRAME_RATE {
            set(var, &a.limit.to_string());
        }
    }
    let user_dxvk_config = g.env_options.iter().rev().find(|(k, _)| k == gpu::DXVK_CONFIG).map(|(_, v)| v.as_str());
    let gpu_vars = gpu::launch_options_vars(&g.gpu, probe, user_dxvk_config);
    for (k, v) in &gpu_vars {
        set(k, v);
    }
    let user_mangohud_config = g.env_options.iter().any(|(k, _)| k == MANGOHUD_CONFIG);
    let mangohud_limit = limit_by(Method::MangoHud);
    if let Some(a) = mangohud_limit.filter(|_| !user_mangohud_config) {
        set(MANGOHUD_CONFIG, &fps_limit::mangohud_config(&a, g.show_mangohud, None));
    }
    if g.lsfg.enabled {
        let dll = settings.get(lsfg::KEY_DLL).and_then(Value::as_str).filter(|d| !d.is_empty());
        let vars = lsfg::env_for(&g.lsfg, Layer::V2, dll, !g.lsfg.profile.is_empty());
        // `LSFGVK_ENV` / `LSFGVK_PROFILE` first: they say what the others are.
        let (lead, rest): (Vec<_>, Vec<_>) = vars.iter().partition(|(k, _)| matches!(k.as_str(), "LSFGVK_ENV" | "LSFGVK_PROFILE"));
        for (k, v) in lead.into_iter().chain(rest) {
            set(k, v);
        }
    }
    for (k, v) in &g.env_options {
        // Merged into the graphics card's own.
        if k == gpu::DXVK_CONFIG && gpu_vars.iter().any(|(var, _)| var == gpu::DXVK_CONFIG) {
            continue;
        }
        set(k, v);
    }
    let mut words: Vec<String> = Vec::new();
    if !g.before_launch_script.is_empty() || !g.after_launch_script.is_empty() {
        words.push(wrap.to_string());
        if !g.before_launch_script.is_empty() {
            words.extend(["--before".into(), g.before_launch_script.clone()]);
        }
        if !g.after_launch_script.is_empty() {
            words.extend(["--after".into(), g.after_launch_script.clone()]);
        }
        words.push("--".into());
    }
    let gamescope_limit = limit_by(Method::Gamescope);
    let gamescope_on = g.gamescope.enable_upscaling || gamescope_limit.is_some();
    if gamescope_on {
        words.push("gamescope".into());
        words.extend(gamescope_args(&g.gamescope, false, g.show_mangohud, gamescope_limit.as_ref()));
    }
    for w in &g.wrapper_options {
        words.push(w.exe.clone());
        words.extend(split(&w.args));
    }
    if (g.show_mangohud || mangohud_limit.is_some()) && !gamescope_on {
        words.push("mangohud".into());
    }
    if g.use_game_mode {
        words.push("gamemoderun".into());
    }
    let args = split(&g.launcher_args);
    if out.is_empty() && words.is_empty() && args.is_empty() {
        return String::new();
    }
    out.extend(words.iter().map(|w| quote(w)));
    out.push(COMMAND.into());
    out.extend(args.iter().map(|a| quote(a)));
    out.join(" ")
}

/// Words of a field the user typed; unbalanced quotes keep the text whole.
fn split(text: &str) -> Vec<String> {
    let text = text.trim();
    if text.is_empty() {
        return Vec::new();
    }
    shlex::split(text).unwrap_or_else(|| vec![text.to_string()])
}

/// The word as the shell reads it back; bare when it has nothing special
/// (`WINEDLLOVERRIDES=winmm=n,b` stays readable in Steam's field).
fn quote(word: &str) -> String {
    let plain = |c: char| c.is_ascii_alphanumeric() || "_@%+=:,./-".contains(c);
    if !word.is_empty() && word.chars().all(plain) {
        return word.to_string();
    }
    shlex::try_quote(word).map(|q| q.into_owned()).unwrap_or_else(|_| word.to_string())
}

fn join(words: &[String]) -> String {
    words.iter().map(|w| quote(w)).collect::<Vec<_>>().join(" ")
}

/// Whether rebuilding the line from its words would change what the shell
/// does with it: separators, redirections, substitutions, globs, `~`,
/// comments or an unbalanced quote.
fn uses_shell_syntax(line: &str) -> bool {
    let (mut single, mut double, mut escaped) = (false, false, false);
    let mut word_start = true;
    for c in line.chars() {
        if escaped {
            escaped = false;
            word_start = false;
            continue;
        }
        if single {
            single = c != '\'';
            continue;
        }
        if double {
            match c {
                '"' => double = false,
                '\\' => escaped = true,
                '$' | '`' => return true,
                _ => {}
            }
            continue;
        }
        match c {
            '\\' => escaped = true,
            '\'' => single = true,
            '"' => double = true,
            ';' | '&' | '|' | '<' | '>' | '(' | ')' | '$' | '`' | '*' | '?' | '~' => return true,
            '#' if word_start => return true,
            _ => {}
        }
        word_start = c.is_whitespace();
    }
    single || double || escaped
}

#[cfg(test)]
mod tests {
    use super::*;

    const WRAP: &str = "/data/tools/bin/heronite-steam-wrap";

    fn roundtrip(line: &str) -> String {
        let p = parse(line);
        assert!(p.editable, "{line}");
        build(&p.settings, WRAP)
    }

    fn env(p: &Parsed) -> Vec<(String, String)> {
        heronite_launch::settings::GameSettings::from_map(&p.settings, "").env_options
    }

    #[test]
    fn empty_line_is_steams_defaults() {
        let p = parse("  ");
        assert!(p.editable);
        assert_eq!(p.settings, defaults());
        assert_eq!(build(&defaults(), WRAP), "");
    }

    #[test]
    fn graphics_card_settings_read_back() {
        let line = "NVPRESENT_ENABLE_SMOOTH_MOTION=1 PROTON_DLSS_UPGRADE=1 \
             DXVK_CONFIG='dxgi.customVendorId=10de;d3d11.maxFeatureLevel=11_0' %command%";
        let p = parse(line);
        assert_eq!(p.settings[gpu::KEY_SMOOTH_MOTION], true);
        assert_eq!(p.settings[gpu::KEY_DLSS_UPGRADE], true);
        assert_eq!(p.settings[gpu::KEY_VENDOR], "nvidia");
        assert_eq!(env(&p), [("DXVK_CONFIG".to_string(), "d3d11.maxFeatureLevel=11_0".to_string())]);
        // Rebuilt by name; the user's DXVK options join ours in one variable.
        // A machine without cards: every option applies.
        assert_eq!(
            build_with(&p.settings, WRAP, &heronite_launch::probe::FakeProbe::new("/h")),
            "DXVK_CONFIG='dxgi.customVendorId=10de;d3d11.maxFeatureLevel=11_0' NVPRESENT_ENABLE_SMOOTH_MOTION=1 PROTON_DLSS_UPGRADE=1 %command%"
        );
    }

    #[test]
    fn reads_switches_wrappers_and_arguments() {
        let p = parse(
            "PROTON_NO_ESYNC=1 PROTON_ENABLE_WAYLAND=1 PROTON_LOG=1 DXVK_HUD=fps DXVK_FRAME_RATE=60 MANGOHUD=1 \
             WINEDLLOVERRIDES=\"winmm=n,b\" strangle 60 mangohud gamemoderun %command% --skip-to-main-menu -x 'a b'",
        );
        assert!(p.editable);
        let s = &p.settings;
        assert_eq!(s["enableEsync"], false);
        assert_eq!(s["enableFsync"], true);
        assert_eq!(s["enableWineWayland"], true);
        assert_eq!(s["verboseLogs"], true);
        assert_eq!(s["showFps"], true);
        assert_eq!(s[fps_limit::KEY_LIMIT], "60");
        assert_eq!(s[fps_limit::KEY_METHOD], "dxvk");
        assert_eq!(s["showMangohud"], true);
        assert_eq!(s["useGameMode"], true);
        assert_eq!(env(&p), [("MANGOHUD".into(), "1".into()), ("WINEDLLOVERRIDES".into(), "winmm=n,b".into())]);
        assert_eq!(s["wrapperOptions"], json!([{"exe": "strangle", "args": "60"}]));
        assert_eq!(s["launcherArgs"], "--skip-to-main-menu -x 'a b'");
    }

    #[test]
    fn rebuilds_in_a_fixed_order_with_the_same_meaning() {
        assert_eq!(
            roundtrip("WINEDLLOVERRIDES=\"winmm=n,b\" PROTON_NO_FSYNC=1 gamemoderun mangohud %command% -novid"),
            "PROTON_NO_FSYNC=1 WINEDLLOVERRIDES=winmm=n,b mangohud gamemoderun %command% -novid"
        );
        let ours = "PROTON_NO_ESYNC=1 PROTON_ENABLE_HDR=1 PROTON_ENABLE_NVAPI=1 DXVK_HUD=fps DRI_PRIME=1 __NV_PRIME_RENDER_OFFLOAD=1 \
                    __GLX_VENDOR_LIBRARY_NAME=nvidia MANGOHUD_CONFIG=read_cfg,fps_limit=90 A='x y' \
                    /data/tools/bin/heronite-steam-wrap --before /s/b.sh --after '/s/a b.sh' -- \
                    gamescope -w 1280 -h 720 -W 2560 -H 1440 -F fsr -f -r 60 --force-grab-cursor --mangoapp --rt -- \
                    /usr/bin/prime-run --flag gamemoderun %command% -dx12";
        assert_eq!(roundtrip(ours), ours);
    }

    #[test]
    fn reads_our_script_and_gamescope() {
        let p = parse("heronite-steam-wrap --after /s/a -- gamescope -W 1920 -b -o 30 -- %command%");
        let s = &p.settings;
        assert_eq!(s["beforeLaunchScriptPath"], "");
        assert_eq!(s["afterLaunchScriptPath"], "/s/a");
        let g = &s["gamescope"];
        assert_eq!(g["enableUpscaling"], true);
        assert_eq!(g["windowType"], "borderless");
        assert_eq!(g["upscaleWidth"], "1920");
        assert_eq!(g["upscaleMethod"], "");
        // A limit without focus alone is not an FPS limit: it stays an option.
        assert_eq!(g["additionalOptions"], "-o 30");
        assert_eq!(s[fps_limit::KEY_LIMIT], "");
        assert_eq!(s["showMangohud"], false);
        // Bare gamescope stays on, with its unknown options kept.
        let p = parse("gamescope --hdr-enabled -- %command%");
        assert_eq!(p.settings["gamescope"]["enableUpscaling"], true);
        assert_eq!(p.settings["gamescope"]["windowType"], "windowed");
        assert_eq!(build(&p.settings, WRAP), "gamescope --hdr-enabled -- %command%");
    }

    #[test]
    fn a_partial_set_of_known_variables_stays_plain() {
        let p = parse("DRI_PRIME=1 PROTON_NO_ESYNC=0 DXVK_HUD=full MANGOHUD_CONFIG=fps_limit=60 %command%");
        assert_eq!(p.settings["nvidiaPrime"], false);
        assert_eq!(p.settings["enableEsync"], true);
        assert_eq!(p.settings["showFps"], false);
        // Without MangoHud on, its config is a plain variable.
        assert_eq!(p.settings[fps_limit::KEY_LIMIT], "");
        assert_eq!(env(&p).len(), 4);
    }

    #[test]
    fn fps_limit_reads_back_as_its_limiter() {
        for line in [
            "DXVK_FRAME_RATE=60 VKD3D_FRAME_RATE=60 %command%",
            "MANGOHUD_CONFIG=read_cfg,no_display,fps_limit=60,fps_limit_method=early mangohud %command%",
            "MANGOHUD_CONFIG=read_cfg,fps_limit=60 mangohud %command%",
            "gamescope -r 60 -o 15 -- %command%",
            "gamescope -W 2560 -H 1440 -F fsr -f -r 60 -- %command%",
        ] {
            assert_eq!(roundtrip(line), line);
        }
        let hidden = parse("MANGOHUD_CONFIG=read_cfg,no_display,fps_limit=60,fps_limit_method=early mangohud %command%").settings;
        assert_eq!(
            (&hidden[fps_limit::KEY_METHOD], &hidden["showMangohud"], &hidden[fps_limit::KEY_SMOOTH]),
            (&json!("mangohud"), &json!(false), &json!(true))
        );
        let gs = parse("gamescope -r 60 -o 15 -- %command%").settings;
        assert_eq!((&gs[fps_limit::KEY_METHOD], &gs[fps_limit::KEY_NO_FOCUS]), (&json!("gamescope"), &json!("15")));
        assert_eq!(gs["gamescope"]["enableUpscaling"], false);
        // Older lines: DXVK's variable alone gains VKD3D-Proton's.
        assert_eq!(roundtrip("DXVK_FRAME_RATE=60 %command%"), "DXVK_FRAME_RATE=60 VKD3D_FRAME_RATE=60 %command%");
        // A second limiter is a plain variable.
        let two = parse("MANGOHUD_CONFIG=read_cfg,fps_limit=90 DXVK_FRAME_RATE=60 mangohud %command%");
        assert_eq!(two.settings[fps_limit::KEY_LIMIT], "90");
        assert_eq!(env(&two), [("DXVK_FRAME_RATE".into(), "60".into())]);
        // Automatic writes the limiter it picks: MangoHud, hidden.
        let mut auto = defaults();
        auto.insert(fps_limit::KEY_LIMIT.into(), json!("60"));
        assert_eq!(build(&auto, WRAP), "MANGOHUD_CONFIG=read_cfg,no_display,fps_limit=60 mangohud %command%");
        // MangoHud cannot limit inside gamescope: gamescope does.
        auto.insert(fps_limit::KEY_METHOD.into(), json!("mangohud"));
        auto.insert("gamescope".into(), json!({"enableUpscaling": true, "windowType": "fullscreen"}));
        assert_eq!(build(&auto, WRAP), "gamescope -f -r 60 -- %command%");
    }

    #[test]
    fn frame_generation_reads_back() {
        let ours = "LSFGVK_ENV=1 LSFGVK_FLOW_SCALE=0.75 LSFGVK_MULTIPLIER=3 LSFGVK_PERFORMANCE_MODE=1 %command%";
        let p = parse(ours);
        assert_eq!(p.settings["heroniteLsfg"], true);
        assert_eq!(p.settings["heroniteLsfgMultiplier"], 3);
        assert_eq!(p.settings["heroniteLsfgFlowScale"], 75);
        assert_eq!(p.settings["heroniteLsfgPerformance"], true);
        assert!(env(&p).is_empty());
        assert_eq!(roundtrip(ours), ours);
        let hdr = "LSFGVK_ENV=1 LSFGVK_COLOR_SPACE=hdr10 LSFGVK_DLL_PATH=/d/lsfg-vk.dll LSFGVK_FLOW_SCALE=1.00 \
                   LSFGVK_MULTIPLIER=2 LSFGVK_PERFORMANCE_MODE=0 LSFGVK_TRANSFER_FUNCTION=pq %command%";
        assert_eq!(roundtrip(hdr), hdr);
        let profile = "LSFGVK_PROFILE='4x FG' %command%";
        assert_eq!(parse(profile).settings["heroniteLsfgProfile"], "4x FG");
        assert_eq!(roundtrip(profile), profile);
        // Values the panel cannot show stay plain variables.
        let odd = parse("LSFGVK_ENV=1 LSFGVK_MULTIPLIER=12 LSFGVK_COLOR_SPACE=scrgb %command%");
        assert_eq!(odd.settings["heroniteLsfgMultiplier"], 2);
        assert_eq!(env(&odd), [("LSFGVK_MULTIPLIER".into(), "12".into()), ("LSFGVK_COLOR_SPACE".into(), "scrgb".into())]);
        // Without LSFGVK_ENV the variables are someone else's.
        assert_eq!(parse("LSFGVK_MULTIPLIER=3 %command%").settings["heroniteLsfg"], false);
    }

    #[test]
    fn a_line_without_command_is_arguments() {
        let p = parse("-novid -console");
        assert_eq!(p.settings["launcherArgs"], "-novid -console");
        assert_eq!(build(&p.settings, WRAP), "%command% -novid -console");
    }

    #[test]
    fn shell_syntax_is_not_editable() {
        for line in [
            "%command%; echo done",
            "FOO=$HOME/x %command%",
            "\"$X\" %command%",
            "~/bin/wrap %command%",
            "%command% > /tmp/log 2>&1",
            "bash -c 'exec \"$@\"' -- %command% | tee",
            "# %command%",
            "bash -c \"%command% -x\"",
            "%command% %command%",
            "unbalanced 'quote %command%",
            "gamescope -f %command%",
            "heronite-steam-wrap --what -- %command%",
            "gamemoderun MANGOHUD_CONFIG=x %command%",
        ] {
            assert!(!parse(line).editable, "{line}");
        }
        assert!(parse("bash -c 'exec \"${@/x/y}\"' -- %command%").editable, "single quotes are literal");
        assert!(parse("A=b#c %command% -x#1").editable);
    }

    #[test]
    fn settings_turned_on_build_the_line() {
        let mut s = defaults();
        s.insert("useGameMode".into(), json!(true));
        s.insert("showMangohud".into(), json!(true));
        s.insert("heroniteMangohudFpsLimit".into(), json!("72"));
        s.insert("enableFsync".into(), json!(false));
        s.insert("enviromentOptions".into(), json!([{"key": "X", "value": "a b"}]));
        s.insert("launcherArgs".into(), json!("-windowed \"two words\""));
        assert_eq!(
            build(&s, WRAP),
            "PROTON_NO_FSYNC=1 MANGOHUD_CONFIG=read_cfg,fps_limit=72 X='a b' mangohud gamemoderun %command% -windowed 'two words'"
        );
        // MangoHud moves inside gamescope.
        s.insert("gamescope".into(), json!({"enableLimiter": true, "fpsLimiter": "30"}));
        assert!(build(&s, WRAP).contains("gamescope -r 30 --mangoapp -- gamemoderun %command%"));
        assert_eq!(parse(&build(&s, WRAP)).settings["showMangohud"], true);
    }
}
