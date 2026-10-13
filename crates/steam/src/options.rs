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
//! `gamescope`, `enviromentOptions`...), plus [`KEY_NVAPI`] and
//! [`KEY_WINED3D`].

use heronite_launch::env::gamescope_args;
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
const SWITCHES: [(&str, &str); 6] = [
    ("enableWineWayland", "PROTON_ENABLE_WAYLAND"),
    ("enableHDR", "PROTON_ENABLE_HDR"),
    ("enableWoW64", "PROTON_USE_WOW64"),
    (KEY_NVAPI, "PROTON_ENABLE_NVAPI"),
    (KEY_WINED3D, "PROTON_USE_WINED3D"),
    ("verboseLogs", "PROTON_LOG"),
];
/// Switches that are on unless `VAR=1`.
const INVERTED: [(&str, &str); 2] = [("enableEsync", "PROTON_NO_ESYNC"), ("enableFsync", "PROTON_NO_FSYNC")];
/// What "use the dedicated GPU" sets (`setupEnvVars` with `nvidiaPrime`).
const PRIME: [(&str, &str); 3] = [("DRI_PRIME", "1"), ("__NV_PRIME_RENDER_OFFLOAD", "1"), ("__GLX_VENDOR_LIBRARY_NAME", "nvidia")];
const DXVK_HUD: &str = "DXVK_HUD";
const DXVK_FRAME_RATE: &str = "DXVK_FRAME_RATE";
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
    for key in ["showFps", "enableDXVKFpsLimit", "nvidiaPrime", "showMangohud", "useGameMode"] {
        s.insert(key.into(), Value::Bool(false));
    }
    for key in ["DXVKFpsCap", "heroniteMangohudFpsLimit", "launcherArgs", "beforeLaunchScriptPath", "afterLaunchScriptPath"] {
        s.insert(key.into(), Value::String(String::new()));
    }
    s.insert("enviromentOptions".into(), json!([]));
    s.insert("wrapperOptions".into(), json!([]));
    s.insert("gamescope".into(), gamescope_value(&Gamescope::default()));
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
                let (g, mangoapp) = read_gamescope(&tokens[i + 1..end]);
                if mangoapp {
                    s.insert("showMangohud".into(), Value::Bool(true));
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
fn read_env(env: Vec<(String, String)>, gamescope_on: bool, s: &mut Settings) {
    let has = |k: &str, v: &str| env.iter().any(|(key, value)| key == k && value == v);
    let prime = PRIME.iter().all(|(k, v)| has(k, v));
    let mangohud_on = s.get("showMangohud") == Some(&Value::Bool(true));
    let mut rest: Vec<Value> = Vec::new();
    for (key, value) in env {
        let one = value == "1";
        if let Some((name, _)) = SWITCHES.iter().find(|(_, var)| *var == key).filter(|_| one) {
            s.insert((*name).into(), Value::Bool(true));
        } else if let Some((name, _)) = INVERTED.iter().find(|(_, var)| *var == key).filter(|_| one) {
            s.insert((*name).into(), Value::Bool(false));
        } else if key == DXVK_HUD && value == "fps" {
            s.insert("showFps".into(), Value::Bool(true));
        } else if key == DXVK_FRAME_RATE && is_number(&value) {
            s.insert("enableDXVKFpsLimit".into(), Value::Bool(true));
            s.insert("DXVKFpsCap".into(), Value::String(value));
        } else if let Some(limit) = (key == MANGOHUD_CONFIG && mangohud_on).then(|| mangohud_limit(&value)).flatten() {
            s.insert("heroniteMangohudFpsLimit".into(), Value::String(limit));
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

fn is_number(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit())
}

/// The limit of a `MANGOHUD_CONFIG` this module wrote.
fn mangohud_limit(config: &str) -> Option<String> {
    let limit = config.strip_prefix("read_cfg,").unwrap_or(config).strip_prefix("fps_limit=")?;
    is_number(limit).then(|| limit.to_string())
}

/// The gamescope part of the settings (Heroic's `gamescope` object).
#[derive(Debug, Clone, PartialEq)]
struct Gamescope {
    upscaling: bool,
    limiter: bool,
    grab_cursor: bool,
    window_type: &'static str,
    game_width: String,
    game_height: String,
    upscale_width: String,
    upscale_height: String,
    method: String,
    fps: String,
    fps_no_focus: String,
    additional: String,
}

impl Default for Gamescope {
    fn default() -> Self {
        Self {
            upscaling: false,
            limiter: false,
            grab_cursor: false,
            window_type: "fullscreen",
            game_width: String::new(),
            game_height: String::new(),
            upscale_width: String::new(),
            upscale_height: String::new(),
            method: "fsr".into(),
            fps: String::new(),
            fps_no_focus: String::new(),
            additional: String::new(),
        }
    }
}

fn gamescope_value(g: &Gamescope) -> Value {
    json!({
        "enableUpscaling": g.upscaling,
        "enableLimiter": g.limiter,
        "enableForceGrabCursor": g.grab_cursor,
        "windowType": g.window_type,
        "gameWidth": g.game_width,
        "gameHeight": g.game_height,
        "upscaleWidth": g.upscale_width,
        "upscaleHeight": g.upscale_height,
        "upscaleMethod": g.method,
        "fpsLimiter": g.fps,
        "fpsLimiterNoFocus": g.fps_no_focus,
        "additionalOptions": g.additional,
    })
}

/// Gamescope's options (between `gamescope` and `--`); `.1`: `--mangoapp`.
fn read_gamescope(args: &[String]) -> (Gamescope, bool) {
    let mut g = Gamescope { window_type: "windowed", method: String::new(), ..Default::default() };
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
            "-r" => Some(&mut g.fps),
            "-o" => Some(&mut g.fps_no_focus),
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
    g.limiter = !g.fps.is_empty() || !g.fps_no_focus.is_empty();
    g.upscaling = sized || !g.method.is_empty() || g.window_type != "windowed" || !g.limiter;
    g.additional = join(&extra);
    (g, mangoapp)
}

/// The line for `settings`; `wrap` is the path of [`crate::wrap`]'s script.
/// Empty when everything is at Steam's defaults.
pub fn build(settings: &Settings, wrap: &str) -> String {
    let g = GameSettings::from_map(settings, "");
    let flag = |key: &str| settings.get(key) == Some(&Value::Bool(true));
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
    if g.enable_dxvk_fps_limit && is_number(g.dxvk_fps_cap.trim()) {
        set(DXVK_FRAME_RATE, g.dxvk_fps_cap.trim());
    }
    if g.nvidia_prime {
        for (k, v) in PRIME {
            set(k, v);
        }
    }
    let user_mangohud_config = g.env_options.iter().any(|(k, _)| k == MANGOHUD_CONFIG);
    if g.show_mangohud && !user_mangohud_config && g.mangohud_fps_limit.parse::<u32>().is_ok_and(|n| n > 0) {
        set(MANGOHUD_CONFIG, &format!("read_cfg,fps_limit={}", g.mangohud_fps_limit));
    }
    for (k, v) in &g.env_options {
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
    let gamescope_on = g.gamescope.enable_upscaling || g.gamescope.enable_limiter;
    if gamescope_on {
        words.push("gamescope".into());
        words.extend(gamescope_args(&g.gamescope, false, g.show_mangohud));
    }
    for w in &g.wrapper_options {
        words.push(w.exe.clone());
        words.extend(split(&w.args));
    }
    if g.show_mangohud && !gamescope_on {
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
        assert_eq!(s["enableDXVKFpsLimit"], true);
        assert_eq!(s["DXVKFpsCap"], "60");
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
        assert_eq!(g["enableLimiter"], true);
        assert_eq!(g["windowType"], "borderless");
        assert_eq!(g["upscaleWidth"], "1920");
        assert_eq!(g["fpsLimiterNoFocus"], "30");
        assert_eq!(g["upscaleMethod"], "");
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
        assert_eq!(p.settings["heroniteMangohudFpsLimit"], "");
        assert_eq!(env(&p).len(), 4);
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
