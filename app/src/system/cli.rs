//! Command line and protocol URLs (`src/backend/protocol.ts` of Heroic).
//!
//! Accepted launch forms:
//!
//! * `heronite-rs --launch <runner>_<appName>`, `--launch <runner> <appName>`,
//!   `--launch <appName>` (also `--launch=...`),
//! * `heronite://launch/<appName>`, `heronite://launch/<runner>/<appName>`,
//!   `heronite://launch?appName=..&runner=..[&arg=..][&altExe=..][&gui=false]`
//!   (Heroic's URL forms under this app's own scheme; `heroic://` stays Heroic's).
//! * `heronite://stop?appName=..&runner=..` (and the path forms) closes the
//!   game if the running instance has it running; with no instance it does
//!   nothing (Sunshine runs it when a stream's application ends).
//!   The old `heronite-rs://` scheme is still read: Steam shortcuts made
//!   before the rename keep it.
//! * `heronite://screenshot` (or `--screenshot`) captures the game the
//!   running instance has open, like the screenshot shortcut. A key bound to
//!   it in the desktop's settings stands in for that shortcut where Heronite
//!   cannot register one (a desktop without a global shortcuts portal).
//!
//! Mod links (`nxm://` from Nexus Mods' "Mod manager download", `ror2mm://`
//! from Thunderstore's "Install with mod manager") go to the mods service.
//!
//! `--open-client <id>` opens a store client (`battlenet`, `ea`, `ubisoft`,
//! `rockstar`; the shortcuts of the clients under Wine) and never shows the
//! window.
//!
//! `--no-gui` means "do not show the window". `--console` and `--fullscreen`
//! show it in console mode and/or fullscreen.

use heronite_data::clients::Client;

/// Runners Heroic's protocol accepts; we also know `zoom`.
pub const RUNNERS: [&str; 5] = ["legendary", "gog", "nile", "sideload", "zoom"];

/// The URL scheme we handle.
pub const SCHEME: &str = "heronite";

/// The scheme's earlier name, still accepted.
pub const OLD_SCHEME: &str = "heronite-rs";

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LaunchRequest {
    /// `None`: search every runner (like `findGame` without a runner).
    pub runner: Option<String>,
    pub app_name: String,
    /// `arg=` parameters (extra game arguments).
    pub args: Vec<String>,
    /// `altExe=` parameter.
    pub alt_exe: Option<String>,
    /// `gui=false|0|no` in the URL.
    pub gui_false: bool,
}

/// What a `heronite://` URL asks for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UrlAction {
    Launch(LaunchRequest),
    /// Close this game if it runs (only `app_name` and `runner` are read).
    Stop(LaunchRequest),
    /// Capture the running game.
    Screenshot,
    Ping,
}

/// What the command line asks for.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Cli {
    pub launch: Option<LaunchRequest>,
    /// An `nxm://` or `ror2mm://` link to install a mod from.
    pub mod_link: Option<String>,
    /// `--open-client <id>`.
    pub open_client: Option<Client>,
    /// `heronite://stop?..`: the game to close in the running instance.
    pub stop: Option<LaunchRequest>,
    /// `--screenshot` or `heronite://screenshot`: capture the game the
    /// running instance has open.
    pub screenshot: bool,
    /// `--no-gui`.
    pub no_gui: bool,
    /// `--console`: switch to console mode.
    pub console: bool,
    /// `--fullscreen`: the window fills the screen.
    pub fullscreen: bool,
    /// `--handoff`: with no instance running, start one on its own (outside
    /// the caller's process group or job) and exit at once. Sunshine ends the
    /// whole group of what it started when the stream quits; the instance
    /// must not be part of it ([`HANDOFF`]).
    pub handoff: bool,
    /// `--autostart`: started with the session (the autostart entry); only
    /// then does "Start minimized" apply.
    pub autostart: bool,
    pub help: bool,
    /// Problems worth telling the user (unusable `--launch`).
    pub errors: Vec<String>,
}

pub const HELP: &str = "\
heronite-rs - game Collection

Usage:
  heronite-rs                         open the Collection
  heronite-rs --launch <runner>_<app> launch a game (also: --launch <runner> <app>, --launch <app>)
  heronite-rs [--no-gui] <url>        launch from heronite://launch?appName=..&runner=..
  heronite-rs <nxm://..|ror2mm://..>  install a mod from Nexus Mods or Thunderstore
  heronite-rs --open-client <client>  open battlenet, ea, ubisoft or rockstar (no window)
  heronite-rs --screenshot            capture the running game (for a key bound in the desktop's settings)
  --no-gui                            do not show the window (the process exits when the game ends)
  --console                           open in console mode (it stays on until you leave it)
  --fullscreen                        open the window fullscreen (F11 switches back)
  --autostart                         started with the system (\"Start minimized\" applies)
  --handoff                           start the instance detached and exit at once (Sunshine)
  --display-probe                     print the monitors, their modes and HDR as JSON, and exit
  -h, --help                          this text
";

/// Prints what [`heronite_display::probe`] reads and exits, before anything
/// else starts (no window, no single-instance check, no data folder). Only
/// reads the displays.
pub const DISPLAY_PROBE: &str = "--display-probe";

/// Whether the command line asks for [`DISPLAY_PROBE`].
pub fn wants_display_probe<I: IntoIterator<Item = std::ffi::OsString>>(args: I) -> bool {
    args.into_iter().any(|a| a == DISPLAY_PROBE)
}

/// See [`Cli::handoff`].
pub const HANDOFF: &str = "--handoff";

/// The arguments of this process for the instance it hands off to: the same,
/// without [`HANDOFF`].
pub fn without_handoff<I: IntoIterator<Item = std::ffi::OsString>>(args: I) -> Vec<std::ffi::OsString> {
    args.into_iter().filter(|a| a != HANDOFF).collect()
}

const OPEN_CLIENT_USAGE: &str = "--open-client needs a store client: battlenet, ea, ubisoft or rockstar";

fn is_runner(s: &str) -> bool {
    RUNNERS.contains(&s)
}

/// `legendary_Quail` -> (`legendary`, `Quail`); anything else is just an
/// app name.
pub fn split_key(s: &str) -> (Option<String>, String) {
    for r in RUNNERS {
        if let Some(rest) = s.strip_prefix(r).and_then(|x| x.strip_prefix('_')) {
            if !rest.is_empty() {
                return (Some(r.to_string()), rest.to_string());
            }
        }
    }
    (None, s.to_string())
}

fn percent_decode(s: &str) -> String {
    heronite_data::paths::percent_decode(&s.replace('+', " "))
}

fn gui_false(v: &str) -> bool {
    matches!(v, "false" | "0" | "no")
}

/// Parses `heronite://...` (or `heronite-rs://...`); `None` for other
/// strings, other schemes and unknown hosts.
pub fn parse_url(url: &str) -> Option<UrlAction> {
    let (scheme, rest) = url.split_once("://")?;
    if !scheme.eq_ignore_ascii_case(SCHEME) && !scheme.eq_ignore_ascii_case(OLD_SCHEME) {
        return None;
    }
    let rest = rest.split('#').next().unwrap_or(rest);
    let (before_query, query) = match rest.split_once('?') {
        Some((a, b)) => (a, b),
        None => (rest, ""),
    };
    let (host, path) = match before_query.find('/') {
        Some(i) => (&before_query[..i], &before_query[i..]),
        None => (before_query, ""),
    };
    let params: Vec<(String, String)> = query
        .split('&')
        .filter(|p| !p.is_empty())
        .map(|p| match p.split_once('=') {
            Some((k, v)) => (percent_decode(k), percent_decode(v)),
            None => (percent_decode(p), String::new()),
        })
        .collect();
    let param = |name: &str| params.iter().find(|(k, _)| k == name).map(|(_, v)| v.clone());

    match host.to_ascii_lowercase().as_str() {
        "ping" => Some(UrlAction::Ping),
        "screenshot" => Some(UrlAction::Screenshot),
        host @ ("launch" | "stop") => {
            let mut req = LaunchRequest { gui_false: param("gui").is_some_and(|g| gui_false(&g)), ..Default::default() };
            let (app_name, runner);
            // Windows adds a trailing / to shortcuts: "/" means "no path".
            if !path.is_empty() && path != "/" {
                let mut segs: Vec<String> = path.split('/').filter(|s| !s.is_empty()).map(percent_decode).collect();
                app_name = segs.pop();
                runner = segs.pop();
            } else {
                app_name = param("appName");
                runner = param("runner");
                req.args = params.iter().filter(|(k, _)| k == "arg").map(|(_, v)| v.clone()).collect();
                req.alt_exe = param("altExe").filter(|p| p.starts_with('/'));
            }
            req.app_name = app_name.filter(|a| !a.is_empty())?;
            req.runner = runner.filter(|r| is_runner(r));
            Some(if host == "stop" { UrlAction::Stop(req) } else { UrlAction::Launch(req) })
        }
        _ => None,
    }
}

/// `nxm://…` (Nexus Mods) or `ror2mm://…` (Thunderstore).
pub fn is_mod_link(arg: &str) -> bool {
    arg.split_once("://").is_some_and(|(scheme, _)| scheme.eq_ignore_ascii_case("nxm") || scheme.eq_ignore_ascii_case("ror2mm"))
}

/// Parses the arguments after the program name.
pub fn parse<I: IntoIterator<Item = String>>(args: I) -> Cli {
    let args: Vec<String> = args.into_iter().collect();
    let mut cli = Cli::default();
    let mut i = 0;
    while i < args.len() {
        let a = &args[i];
        match a.as_str() {
            "--no-gui" => cli.no_gui = true,
            "--console" => cli.console = true,
            "--fullscreen" => cli.fullscreen = true,
            HANDOFF => cli.handoff = true,
            "--autostart" => cli.autostart = true,
            "--screenshot" => cli.screenshot = true,
            "-h" | "--help" => cli.help = true,
            "--launch" => {
                let first = args.get(i + 1).filter(|s| !s.starts_with("--"));
                match first {
                    None => cli.errors.push("--launch needs a game: --launch <runner>_<appName>".into()),
                    Some(first) => {
                        let second = args.get(i + 2).filter(|s| !s.starts_with('-') && !s.contains("://"));
                        let req = match second {
                            Some(app) if is_runner(first) => {
                                i += 1;
                                LaunchRequest { runner: Some(first.clone()), app_name: app.clone(), ..Default::default() }
                            }
                            _ => {
                                let (runner, app_name) = split_key(first);
                                LaunchRequest { runner, app_name, ..Default::default() }
                            }
                        };
                        i += 1;
                        cli.launch = Some(req);
                    }
                }
            }
            "--open-client" => {
                let id = args.get(i + 1).filter(|s| !s.starts_with("--"));
                match id.and_then(|id| Client::from_id(id)) {
                    Some(client) => cli.open_client = Some(client),
                    None => cli.errors.push(OPEN_CLIENT_USAGE.into()),
                }
                if id.is_some() {
                    i += 1;
                }
            }
            _ if a.starts_with("--open-client=") => match Client::from_id(&a["--open-client=".len()..]) {
                Some(client) => cli.open_client = Some(client),
                None => cli.errors.push(OPEN_CLIENT_USAGE.into()),
            },
            _ if a.starts_with("--launch=") => {
                let (runner, app_name) = split_key(&a["--launch=".len()..]);
                if app_name.is_empty() {
                    cli.errors.push("--launch needs a game: --launch=<runner>_<appName>".into());
                } else {
                    cli.launch = Some(LaunchRequest { runner, app_name, ..Default::default() });
                }
            }
            _ if is_mod_link(a) => cli.mod_link = Some(a.clone()),
            _ => {
                match parse_url(a) {
                    // `--launch X` already seen wins over nothing; a URL fills in when absent.
                    Some(UrlAction::Launch(req)) if cli.launch.is_none() => cli.launch = Some(req),
                    Some(UrlAction::Stop(req)) => cli.stop = Some(req),
                    Some(UrlAction::Screenshot) => cli.screenshot = true,
                    _ => {}
                }
            }
        }
        i += 1;
    }
    cli
}

impl Cli {
    /// Should this launch keep the window hidden? (`--no-gui`, `gui=false`, or
    /// the `hideWindowOnProtocolLaunch` setting.) Opening a store client
    /// never shows it.
    pub fn hide_window(&self, hide_setting: bool) -> bool {
        if self.open_client.is_some() {
            return true;
        }
        match &self.launch {
            Some(req) => self.no_gui || req.gui_false || hide_setting,
            None => self.no_gui,
        }
    }

    /// `--console` / `--fullscreen` ask for the window (over "start
    /// minimized"); hiding it still wins.
    pub fn wants_window(&self) -> bool {
        self.console || self.fullscreen
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(args: &[&str]) -> Cli {
        parse(args.iter().map(|s| s.to_string()))
    }

    fn req(runner: Option<&str>, app: &str) -> LaunchRequest {
        LaunchRequest { runner: runner.map(str::to_string), app_name: app.into(), ..Default::default() }
    }

    #[test]
    fn launch_flag_forms() {
        assert_eq!(p(&["--launch", "legendary_Quail"]).launch, Some(req(Some("legendary"), "Quail")));
        assert_eq!(p(&["--launch", "legendary", "Quail"]).launch, Some(req(Some("legendary"), "Quail")));
        assert_eq!(p(&["--launch", "Quail"]).launch, Some(req(None, "Quail")));
        assert_eq!(p(&["--launch=sideload_abc_def"]).launch, Some(req(Some("sideload"), "abc_def")));
        // app names with underscores keep them
        assert_eq!(p(&["--launch", "gog_1207658930"]).launch, Some(req(Some("gog"), "1207658930")));
        // unknown prefix is part of the app name
        assert_eq!(p(&["--launch", "epic_x"]).launch, Some(req(None, "epic_x")));
        // a runner alone is an app name candidate, not a pair
        assert_eq!(p(&["--launch", "gog", "--no-gui"]).launch, Some(req(None, "gog")));
        assert!(p(&["--launch"]).launch.is_none());
        assert!(!p(&["--launch"]).errors.is_empty());
        assert!(!p(&["--launch", "--no-gui"]).errors.is_empty());
    }

    #[test]
    fn no_gui_and_help() {
        let c = p(&["--no-gui", "--launch", "gog_1"]);
        assert!(c.no_gui);
        assert!(c.hide_window(false));
        assert!(p(&["--help"]).help && p(&["-h"]).help);
        assert!(!p(&[]).no_gui);
        assert!(!p(&[]).hide_window(true), "the setting only applies to launches");
        assert!(p(&["--launch", "gog_1"]).hide_window(true));
        assert!(!p(&["--launch", "gog_1"]).hide_window(false));
    }

    #[test]
    fn display_probe_is_spotted_anywhere_and_ignored_by_parse() {
        let os = |args: &[&str]| args.iter().map(std::ffi::OsString::from).collect::<Vec<_>>();
        assert!(wants_display_probe(os(&["--display-probe"])));
        assert!(wants_display_probe(os(&["--no-gui", "--display-probe"])));
        assert!(!wants_display_probe(os(&["--launch", "gog_1"])));
        assert!(!wants_display_probe(os(&["--display-probe=x"])));
        // Never mistaken for a launch or an error.
        let c = p(&["--display-probe"]);
        assert!(c.launch.is_none() && c.errors.is_empty());
    }

    #[test]
    fn handoff_is_read_and_left_out_of_the_new_instance() {
        let c = p(&["--no-gui", "--handoff", "heronite://launch?appName=1&runner=gog"]);
        assert!(c.handoff && c.no_gui);
        assert_eq!(c.launch.map(|r| r.app_name).as_deref(), Some("1"));
        assert!(c.errors.is_empty());
        assert!(!p(&["--no-gui"]).handoff);
        let args = ["--no-gui", "--handoff", "heronite://launch?appName=1&runner=gog"].map(std::ffi::OsString::from);
        assert_eq!(without_handoff(args), ["--no-gui", "heronite://launch?appName=1&runner=gog"].map(std::ffi::OsString::from));
    }

    #[test]
    fn console_and_fullscreen() {
        let c = p(&["--console", "--fullscreen"]);
        assert!(c.console && c.fullscreen && c.wants_window());
        assert!(!p(&[]).wants_window());
        assert!(p(&["--autostart"]).autostart && !p(&[]).autostart);
        assert!(p(&["--fullscreen"]).wants_window());
        // they mix with a launch and do not eat its arguments
        let c = p(&["--console", "--launch", "gog", "1"]);
        assert!(c.console);
        assert_eq!(c.launch, Some(req(Some("gog"), "1")));
        let c = p(&["--launch", "gog_1", "--fullscreen"]);
        assert!(c.fullscreen);
        assert_eq!(c.launch, Some(req(Some("gog"), "1")));
    }

    #[test]
    fn url_path_forms() {
        let l = |u: &str| match parse_url(u) {
            Some(UrlAction::Launch(r)) => r,
            other => panic!("{u}: {other:?}"),
        };
        assert_eq!(l("heronite://launch/Quail"), req(None, "Quail"));
        assert_eq!(l("heronite://launch/legendary/Quail"), req(Some("legendary"), "Quail"));
        assert_eq!(l("heronite://launch/gog/123"), req(Some("gog"), "123"));
        assert_eq!(l("HERONITE://launch/gog/123/"), req(Some("gog"), "123"));
        // unknown runner is ignored (findGame searches all runners)
        assert_eq!(l("heronite://launch/steam/123"), req(None, "123"));
        assert_eq!(l("heronite://launch/My%20Game"), req(None, "My Game"));
    }

    #[test]
    fn old_scheme_still_launches() {
        // Steam shortcuts made before the rename
        let c = p(&["--no-gui", "heronite-rs://launch?appName=Quail&runner=legendary"]);
        assert_eq!(c.launch, Some(req(Some("legendary"), "Quail")));
        assert_eq!(parse_url("HERONITE-RS://ping"), Some(UrlAction::Ping));
    }

    #[test]
    fn url_query_forms() {
        let l = |u: &str| match parse_url(u) {
            Some(UrlAction::Launch(r)) => r,
            other => panic!("{u}: {other:?}"),
        };
        assert_eq!(l("heronite://launch?appName=Quail&runner=legendary"), req(Some("legendary"), "Quail"));
        assert_eq!(l("heronite://launch/?appName=Quail&runner=nile"), req(Some("nile"), "Quail"));
        let r = l("heronite://launch?appName=Quail&runner=sideload&arg=--fast&arg=a%20b&altExe=%2Fgames%2Fx.exe&gui=false");
        assert_eq!(r.runner.as_deref(), Some("sideload"));
        assert_eq!(r.args, vec!["--fast", "a b"]);
        assert_eq!(r.alt_exe.as_deref(), Some("/games/x.exe"));
        assert!(r.gui_false);
        // gui=0 / gui=no
        assert!(l("heronite://launch?appName=a&gui=0").gui_false);
        assert!(l("heronite://launch?appName=a&gui=no").gui_false);
        assert!(!l("heronite://launch?appName=a&gui=true").gui_false);
        // relative altExe is refused
        assert!(l("heronite://launch?appName=a&altExe=x.exe").alt_exe.is_none());
        // fragment is ignored
        assert_eq!(l("heronite://launch?appName=a#frag").app_name, "a");
    }

    #[test]
    fn mod_links() {
        let c = p(&["nxm://skyrimspecialedition/mods/12604/files/35407?key=a&expires=1&user_id=2"]);
        assert_eq!(c.mod_link.as_deref(), Some("nxm://skyrimspecialedition/mods/12604/files/35407?key=a&expires=1&user_id=2"));
        assert!(c.launch.is_none());
        assert!(p(&["ROR2MM://v1/install/thunderstore.io/a/b/1.0.0/"]).mod_link.is_some());
        assert!(p(&["heronite://launch/Quail"]).mod_link.is_none());
        assert!(!is_mod_link("nxm:/broken"));
    }

    #[test]
    fn stop_urls() {
        assert_eq!(parse_url("heronite://stop?appName=Quail&runner=legendary"), Some(UrlAction::Stop(req(Some("legendary"), "Quail"))));
        assert_eq!(parse_url("heronite://stop/gog/1"), Some(UrlAction::Stop(req(Some("gog"), "1"))));
        let c = p(&["heronite://stop?appName=My%20Game&runner=sideload"]);
        assert_eq!(c.stop, Some(req(Some("sideload"), "My Game")));
        assert!(c.launch.is_none());
    }

    #[test]
    fn screenshot_requests() {
        assert_eq!(parse_url("heronite://screenshot"), Some(UrlAction::Screenshot));
        assert_eq!(parse_url("heronite://screenshot/"), Some(UrlAction::Screenshot));
        assert!(p(&["heronite://screenshot"]).screenshot);
        let c = p(&["--screenshot"]);
        assert!(c.screenshot && c.launch.is_none() && c.errors.is_empty());
        assert!(!p(&["--no-gui"]).screenshot);
        // It does not eat a launch that comes with it.
        let c = p(&["--screenshot", "--launch", "gog_1"]);
        assert!(c.screenshot);
        assert_eq!(c.launch, Some(req(Some("gog"), "1")));
    }

    #[test]
    fn url_rejections() {
        assert_eq!(parse_url("heronite://ping"), Some(UrlAction::Ping));
        assert_eq!(parse_url("heronite://ping?x=1"), Some(UrlAction::Ping));
        assert_eq!(parse_url("heronite://launch"), None, "no appName");
        assert_eq!(parse_url("heronite://launch?runner=gog"), None);
        assert_eq!(parse_url("heronite://other/thing"), None);
        assert_eq!(parse_url("heronite://stop"), None, "no appName");
        assert_eq!(parse_url("https://launch/x"), None);
        assert_eq!(parse_url("heroic://launch/Quail"), None, "Heroic's own links are not ours");
        assert_eq!(parse_url("heronitex://launch/Quail"), None);
        assert_eq!(parse_url("not a url"), None);
    }

    #[test]
    fn url_as_argument() {
        let c = p(&["--no-gui", "heronite://launch?appName=Quail&runner=legendary"]);
        assert_eq!(c.launch, Some(req(Some("legendary"), "Quail")));
        assert!(c.no_gui);
        // gui=false in the URL hides the window even without the flag
        let c = p(&["heronite://launch/Quail?gui=false"]);
        assert!(c.hide_window(false));
        // Electron-style leftovers are ignored
        let c = p(&["--no-sandbox", "heronite://launch/gog/1"]);
        assert_eq!(c.launch, Some(req(Some("gog"), "1")));
    }

    #[test]
    fn open_client_forms() {
        assert_eq!(p(&["--open-client", "battlenet"]).open_client, Some(Client::BattleNet));
        assert_eq!(p(&["--open-client=ea"]).open_client, Some(Client::Ea));
        let cli = p(&["--open-client", "rockstar", "--console"]);
        assert_eq!((cli.open_client, cli.console), (Some(Client::Rockstar), true));
        // It never shows the window, whatever the setting says.
        assert!(cli.hide_window(false));
        for bad in [&["--open-client"][..], &["--open-client", "steam"], &["--open-client", "--console"], &["--open-client="]] {
            let cli = p(bad);
            assert_eq!(cli.open_client, None, "{bad:?}");
            assert_eq!(cli.errors.len(), 1, "{bad:?}");
        }
        assert!(p(&["--open-client", "--console"]).console, "the next flag is not eaten");
    }
}
