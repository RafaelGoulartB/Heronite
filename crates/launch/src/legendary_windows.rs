//! Epic games on Windows, natively (no Wine).
//!
//! `legendary launch <app> --json` signs in, checks for an update and prints
//! how the game starts: its program, its arguments (with the sign-in of this
//! session, `-AUTH_PASSWORD=<exchange code>`), its folder and environment.
//! The launch then starts that program itself instead of letting legendary do
//! it (legendary would start it through `cmd.exe` and exit at once). So the
//! game is the launch's own process: its job object, its pid and its log, and
//! no console window shows (legendary runs as a hidden helper).
//!
//! Games of the EA app and Ubisoft Connect (`--origin`, `--ubisoft`) get a
//! link instead (`link2ea://...`, `uplay://...`), which the store's app opens
//! when it is installed.

use std::path::Path;
use std::time::Duration;

use heronite_i18n::tr;
use serde_json::Value;

use crate::env::Env;
use crate::plan::Command;

/// The argument that makes `legendary launch` print the plan.
pub const JSON: &str = "--json";

/// How long legendary may take to sign in and check for an update.
const TIMEOUT: Duration = Duration::from_secs(180);

/// What legendary says starts the game.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resolved {
    /// The game's program, to start as the launch's process.
    Game(Command),
    /// A link the EA app or Ubisoft Connect opens.
    Link(String),
}

/// Why the game cannot start.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Failure {
    Cancelled,
    /// For the user, in the interface language.
    Message(String),
}

/// Runs `cmd` (`legendary launch ... --json`) and reads what it printed.
/// Blocking: legendary signs in over the network.
pub fn resolve(cmd: &Command, cancel: &dyn Fn() -> bool) -> Result<Resolved, Failure> {
    let mut c = std::process::Command::new(&cmd.program);
    c.args(&cmd.args).envs(&cmd.env);
    if let Some(dir) = cmd.cwd.as_deref().filter(|d| !d.is_empty() && Path::new(d).is_dir()) {
        c.current_dir(dir);
    }
    let run = heronite_data::proc::Run::new().poll(cancel).timeout(TIMEOUT).grace(Duration::from_secs(5)).line_capture();
    let out = run
        .run(&mut c)
        .map_err(|e| Failure::Message(tr!("could not start {program}: {error}", program = cmd.program.as_str(), error = e.to_string())))?;
    if out.aborted {
        return Err(Failure::Cancelled);
    }
    if out.timed_out {
        return Err(Failure::Message(tr!("legendary did not answer in time. Check the internet connection and try again.").into()));
    }
    let (stdout, stderr) = (out.stdout_text(), out.stderr_text());
    if !out.success() {
        return Err(Failure::Message(failure_message(&stderr, &stdout, out.code)));
    }
    parse(&stdout, &cmd.env).map_err(|why| {
        log::warn!("legendary launch --json printed {}: {why}", stdout.trim());
        Failure::Message(failure_message(&stderr, &stdout, out.code))
    })
}

/// The user's reason from legendary's `ERROR:` lines.
fn failure_message(stderr: &str, stdout: &str, code: Option<i32>) -> String {
    let all = format!("{stderr}\n{stdout}");
    if all.contains("out of date") {
        return tr!("The game has an update: update it, or choose \"Play anyway\".").into();
    }
    let lines: Vec<&str> = all
        .lines()
        .filter(|l| l.contains("ERROR") || l.contains("CRITICAL"))
        .map(|l| l.split_once(": ").map_or(l, |(_, rest)| rest).trim())
        .filter(|l| !l.is_empty())
        .collect();
    let why = if lines.is_empty() { tr!("legendary exited with {code}", code = format!("{code:?}")) } else { lines.join("\n") };
    tr!("legendary could not start the game: {error}", error = why)
}

fn strings(doc: &Value, key: &str) -> Vec<String> {
    doc.get(key).and_then(Value::as_array).map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect()).unwrap_or_default()
}

fn text<'a>(doc: &'a Value, key: &str) -> &'a str {
    doc.get(key).and_then(Value::as_str).unwrap_or_default()
}

/// A path in the system's own form (legendary's executables may use `/`).
fn native_path(p: &str) -> String {
    if cfg!(windows) {
        p.replace('/', "\\")
    } else {
        p.to_string()
    }
}

/// Reads what `legendary launch --json` printed (log lines may come first).
/// `env` is the plan's environment: the game gets it, with legendary's own
/// variables on top.
pub fn parse(stdout: &str, env: &Env) -> Result<Resolved, String> {
    let start = stdout.find('{').ok_or("no JSON")?;
    let doc: Value = serde_json::from_str(stdout[start..].trim()).map_err(|e| e.to_string())?;
    if let Some(uri) = doc.get("uri").and_then(Value::as_str).filter(|u| !u.is_empty()) {
        return Ok(Resolved::Link(uri.to_string()));
    }
    let exe = text(&doc, "game_executable").trim_start_matches(['/', '\\']);
    if exe.is_empty() {
        return Err("no game_executable".into());
    }
    let dir = text(&doc, "game_directory");
    let game = native_path(&Path::new(dir).join(exe).to_string_lossy());
    let mut parts = strings(&doc, "launch_command");
    parts.push(game);
    parts.extend(strings(&doc, "game_parameters"));
    parts.extend(strings(&doc, "user_parameters"));
    parts.extend(strings(&doc, "egl_parameters"));
    let program = parts.remove(0);
    let mut game_env = env.clone();
    game_env.remove("LEGENDARY_CONFIG_PATH");
    for (k, v) in doc.get("environment").and_then(Value::as_object).into_iter().flatten() {
        if let Some(v) = v.as_str() {
            game_env.insert(k.clone(), v.to_string());
        }
    }
    let cwd = Some(text(&doc, "working_directory")).filter(|d| !d.is_empty()).unwrap_or(dir);
    Ok(Resolved::Game(Command { program, args: parts, env: game_env, cwd: Some(native_path(cwd)).filter(|d| !d.is_empty()) }))
}

/// The store app a link of [`Resolved::Link`] opens, for messages.
pub fn link_client(uri: &str) -> &'static str {
    if uri.starts_with("uplay:") {
        "Ubisoft Connect"
    } else {
        "EA app"
    }
}

/// Whether a program is registered for the link's scheme (`link2ea`,
/// `uplay`): the store's app is installed.
pub fn link_handler_installed(uri: &str) -> bool {
    let Some((scheme, _)) = uri.split_once(':') else { return false };
    handler_installed(scheme)
}

#[cfg(windows)]
fn handler_installed(scheme: &str) -> bool {
    use heronite_platform::registry::{string, Hive};
    let key = format!("Software\\Classes\\{scheme}\\shell\\open\\command");
    [Hive::CurrentUser, Hive::LocalMachine].into_iter().any(|hive| string(hive, &key, "").is_some_and(|c| !c.trim().is_empty()))
}

#[cfg(not(windows))]
fn handler_installed(_scheme: &str) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use heronite_data::testing::{FakeTool, Reply};

    const GAME: &str = r#"[cli] INFO: Logging in...
{"game_parameters": ["-nolauncher"], "game_executable": "Binaries/Win64/Control.exe", "game_directory": "C:\\Games\\Control",
 "egl_parameters": ["-AUTH_LOGIN=unused", "-AUTH_PASSWORD=abc", "-AUTH_TYPE=exchangecode", "-epicapp=Calluna"],
 "launch_command": [], "working_directory": "", "user_parameters": ["-dx11"], "environment": {"EOS": "1"},
 "pre_launch_command": "", "pre_launch_wait": false}"#;

    fn env() -> Env {
        let mut env = Env::new();
        env.insert("LEGENDARY_CONFIG_PATH".into(), "C:\\hr\\legendaryConfig\\legendary".into());
        env.insert("HEROIC_APP_NAME".into(), "Calluna".into());
        env
    }

    #[test]
    fn the_game_starts_with_its_arguments_and_the_sign_in() {
        let Resolved::Game(c) = parse(GAME, &env()).unwrap() else { panic!() };
        let exe =
            if cfg!(windows) { r"C:\Games\Control\Binaries\Win64\Control.exe" } else { "C:\\Games\\Control/Binaries/Win64/Control.exe" };
        assert_eq!(c.program, exe);
        assert_eq!(
            c.args,
            ["-nolauncher", "-dx11", "-AUTH_LOGIN=unused", "-AUTH_PASSWORD=abc", "-AUTH_TYPE=exchangecode", "-epicapp=Calluna"]
        );
        assert_eq!(c.cwd.as_deref(), Some(r"C:\Games\Control"), "the game's folder when legendary names none");
        assert_eq!(c.env["EOS"], "1");
        assert_eq!(c.env["HEROIC_APP_NAME"], "Calluna");
        assert!(!c.env.contains_key("LEGENDARY_CONFIG_PATH"));
        // The sign-in never reaches the game's log.
        let shown = crate::log_writer::command_line_for_log(&c);
        assert!(shown.contains("-AUTH_PASSWORD=<hidden>") && !shown.contains("abc"), "{shown}");
    }

    #[test]
    fn a_wrapper_of_legendarys_config_comes_first() {
        let out = r#"{"game_executable": "g.exe", "game_directory": "D:\\G", "launch_command": ["wrap.exe", "-x"], "working_directory": "D:\\G\\bin"}"#;
        let Resolved::Game(c) = parse(out, &Env::new()).unwrap() else { panic!() };
        assert_eq!(c.program, "wrap.exe");
        assert_eq!(c.args[0], "-x");
        assert!(c.args[1].ends_with("g.exe"));
        assert_eq!(c.cwd.as_deref(), Some(r"D:\G\bin"));
    }

    #[test]
    fn store_app_games_give_a_link() {
        let out = r#"{"uri": "link2ea://launchgame/Origin.OFR.1?AUTH_PASSWORD=x"}"#;
        assert_eq!(parse(out, &Env::new()).unwrap(), Resolved::Link("link2ea://launchgame/Origin.OFR.1?AUTH_PASSWORD=x".into()));
        assert_eq!(link_client("uplay://launch/1"), "Ubisoft Connect");
        assert_eq!(link_client("link2ea://launchgame/x"), "EA app");
        assert!(!link_handler_installed("heronite-no-such-scheme://x"));
        assert!(!link_handler_installed("no scheme"));
    }

    #[test]
    fn nothing_to_start_is_an_error() {
        assert!(parse("[cli] ERROR: nope", &Env::new()).is_err());
        assert!(parse(r#"{"game_executable": ""}"#, &Env::new()).is_err());
    }

    fn command(tool: &FakeTool) -> Command {
        Command {
            program: tool.bin().to_string_lossy().into_owned(),
            args: vec!["launch".into(), "Calluna".into(), JSON.into()],
            env: env(),
            cwd: None,
        }
    }

    #[test]
    fn legendary_runs_as_a_helper_and_its_answer_is_read() {
        let dir = tempfile::tempdir().unwrap();
        let tool = FakeTool::new(dir.path(), "legendary").record_env("LEGENDARY_CONFIG_PATH").otherwise(Reply::new().out(GAME));
        let Resolved::Game(c) = resolve(&command(&tool), &|| false).unwrap() else { panic!() };
        assert!(c.program.ends_with("Control.exe"));
        assert_eq!(tool.calls(), ["launch Calluna --json"]);
        assert_eq!(tool.env_seen("LEGENDARY_CONFIG_PATH"), [r"C:\hr\legendaryConfig\legendary"]);
    }

    #[test]
    fn legendarys_errors_are_explained() {
        let dir = tempfile::tempdir().unwrap();
        let tool = FakeTool::new(dir.path(), "legendary")
            .on(&["Old"], Reply::new().err("[cli] ERROR: Game is out of date, please update or launch with update check skipping!").exit(1))
            .otherwise(Reply::new().err("[cli] ERROR: Login failed, cannot continue!").exit(1));
        let Err(Failure::Message(why)) = resolve(&command(&tool), &|| false) else { panic!() };
        assert!(why.contains("Login failed, cannot continue!") && !why.contains("[cli]"), "{why}");
        let mut old = command(&tool);
        old.args[1] = "Old".into();
        let Err(Failure::Message(why)) = resolve(&old, &|| false) else { panic!() };
        assert!(why.contains("Play anyway"), "{why}");
    }

    #[test]
    fn a_stop_while_legendary_signs_in_cancels() {
        let dir = tempfile::tempdir().unwrap();
        let tool = FakeTool::new(dir.path(), "legendary").otherwise(Reply::new().sleep(Duration::from_secs(30)));
        let started = std::time::Instant::now();
        assert_eq!(resolve(&command(&tool), &|| started.elapsed() > Duration::from_millis(300)), Err(Failure::Cancelled));
        assert!(started.elapsed() < Duration::from_secs(15));
    }
}
