//! The Epic part of starting an Epic game, done right before it starts.
//!
//! The plan ([`crate::epic`]) starts the game's own program, through Wine /
//! Proton / umu on Linux and as it is on Windows, with the user's arguments.
//! What only Epic can give is asked for at launch time, never while the plan
//! is built: [`resolve`] signs in, checks for an update (unless "Play
//! anyway"), and completes the command with this session's arguments (the
//! build's own, then the Epic sign-in with a one-time exchange code and,
//! for games whose DRM checks one, the ownership token), the way legendary
//! put them together. Offline, no network is used and the game gets an
//! empty sign-in.
//!
//! Games of the EA app and Ubisoft Connect get a link instead
//! (`link2ea://...`, `uplay://...`): on Windows the store's app opens it
//! when it is installed; under Wine the plan's `start` opens it in the
//! game's prefix, where the store's app lives.

use std::path::Path;
use std::sync::mpsc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use heronite_epic_api::launch::{self as epic, LaunchParams};
use heronite_epic_api::{is_sign_in_refused, legendary_config, Client, Session};
use heronite_i18n::tr;

use crate::plan::Command;

/// How long Epic may take to sign in and check for an update.
const TIMEOUT: Duration = Duration::from_secs(180);

/// The store app that starts a game of the Epic library.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StoreLink {
    /// The EA app (`link2ea://`).
    EaApp,
    /// Ubisoft Connect (`uplay://`).
    Ubisoft,
}

/// What [`resolve`] completes in a plan's command.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct EpicLaunch {
    /// What starts: the game, or a launchable DLC of it.
    pub app_name: String,
    /// The installed game (whose update is checked).
    pub game: String,
    /// Where the game's program is in the command's arguments; `None`: it is
    /// the command's program (Windows). For a store link, the `start` that
    /// opens it.
    pub exe_index: Option<usize>,
    /// No network: no sign-in, no update check.
    pub offline: bool,
    /// Ask whether an update is out first (off for "Play anyway").
    pub check_update: bool,
    /// `-epiclocale`; Epic's default when `None`.
    pub language: Option<String>,
    /// Another program of the install (the game's `targetExe`, an
    /// alternative program of the launch options).
    pub override_exe: Option<String>,
    pub store_link: Option<StoreLink>,
}

/// What Epic answers for a launch. [`SystemEpic`] asks the real services;
/// tests give their own.
pub trait EpicService: Send + Sync {
    /// Whether the installed build of `game` is older than Epic's.
    fn update_available(&self, game: &str) -> Result<bool, Failure>;
    /// The program, its folder and the arguments of this session.
    fn launch_params(&self, launch: &EpicLaunch) -> Result<LaunchParams, Failure>;
    /// The link the store app opens for `launch.app_name`.
    fn store_link(&self, launch: &EpicLaunch, link: StoreLink) -> Result<String, Failure>;
}

/// What starts the game.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resolved {
    /// The command to spawn.
    Game(Command),
    /// A link the EA app or Ubisoft Connect opens (Windows).
    Link(String),
}

/// Why the game cannot start.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Failure {
    Cancelled,
    /// For the user, in the interface language.
    Message(String),
}

impl Failure {
    fn from_error(err: anyhow::Error) -> Self {
        if is_sign_in_refused(&err) {
            return Failure::Message(tr!("The sign-in expired: log in again").into());
        }
        Failure::Message(tr!("Epic Games could not start the game: {error}", error = format!("{err:#}")))
    }
}

/// A path in the system's own form (installed.json may use `/`).
fn native_path(p: &str) -> String {
    if cfg!(windows) {
        p.replace('/', "\\")
    } else {
        p.to_string()
    }
}

/// `command` completed with what Epic says for `launch` (see the module
/// doc). Blocking: it signs in over the network unless offline.
pub fn resolve(command: &Command, launch: &EpicLaunch, epic: &dyn EpicService) -> Result<Resolved, Failure> {
    if launch.check_update && !launch.offline {
        match epic.update_available(&launch.game) {
            Ok(true) => return Err(Failure::Message(tr!("The game has an update: update it, or choose \"Play anyway\".").into())),
            Ok(false) => {}
            // legendary's check needed the network too; a failed one does not
            // keep the game from starting.
            Err(Failure::Message(why)) => log::warn!("epic: the update check of {} failed: {why}", launch.game),
            Err(Failure::Cancelled) => return Err(Failure::Cancelled),
        }
    }
    if let Some(link) = launch.store_link {
        let uri = epic.store_link(launch, link)?;
        let Some(i) = launch.exe_index else { return Ok(Resolved::Link(uri)) };
        // Under Wine `start` (in the prefix of the store's app) opens it.
        let mut game = command.clone();
        game.args.insert((i + 1).min(game.args.len()), uri);
        return Ok(Resolved::Game(game));
    }
    let params = epic.launch_params(launch)?;
    let exe = native_path(&params.executable_path().to_string_lossy());
    let mut game = command.clone();
    let after_exe = match launch.exe_index {
        None => {
            game.program = exe;
            0
        }
        Some(i) if i < game.args.len() => {
            game.args[i] = exe;
            i + 1
        }
        Some(_) => return Err(Failure::Message(tr!("Epic Games could not start the game: {error}", error = "no program in the plan"))),
    };
    // The build's arguments, the user's (already in the plan), then Epic's.
    game.args.splice(after_exe..after_exe, params.game_parameters.iter().cloned());
    game.args.extend(params.egl_parameters.iter().cloned());
    let cwd = Some(params.working_directory.as_str()).filter(|d| !d.is_empty()).unwrap_or(&params.game_directory);
    if !cwd.is_empty() {
        game.cwd = Some(native_path(cwd));
    }
    Ok(Resolved::Game(game))
}

/// [`resolve`] on a helper thread, so a stop of the launch is not held up
/// by a slow sign-in (the answer is then dropped).
pub fn resolve_cancellable(
    command: &Command,
    launch: &EpicLaunch,
    epic: Arc<dyn EpicService>,
    cancelled: &dyn Fn() -> bool,
) -> Result<Resolved, Failure> {
    let (tx, rx) = mpsc::channel();
    let (command, launch) = (command.clone(), launch.clone());
    std::thread::Builder::new()
        .name("epic-launch".into())
        .spawn(move || {
            let _ = tx.send(resolve(&command, &launch, &*epic));
        })
        .map_err(|e| Failure::Message(e.to_string()))?;
    let started = Instant::now();
    loop {
        if cancelled() {
            return Err(Failure::Cancelled);
        }
        match rx.recv_timeout(Duration::from_millis(100)) {
            Ok(result) => return result,
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                return Err(Failure::Message(tr!("Epic Games could not start the game: {error}", error = "the sign-in stopped")))
            }
            Err(mpsc::RecvTimeoutError::Timeout) if started.elapsed() > TIMEOUT => {
                return Err(Failure::Message(tr!("Epic Games did not answer in time. Check the internet connection and try again.").into()))
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
    }
}

/// Epic's real services, on legendary's config folder of the data folder.
pub struct SystemEpic {
    config_dir: std::path::PathBuf,
    client: Client,
}

impl SystemEpic {
    pub fn new(config_dir: &Path) -> Self {
        Self { config_dir: config_dir.to_path_buf(), client: legendary_config::client_for(config_dir) }
    }

    /// The account's sign-in (renewed when it runs out soon); offline, the
    /// one saved, if any, for the account's name and id.
    fn session(&self, offline: bool) -> Result<Option<Session>, Failure> {
        if offline {
            return Ok(Session::load(&self.config_dir).ok().flatten());
        }
        match self.client.ensure_session(&self.config_dir) {
            Ok(Some(s)) => Ok(Some(s)),
            Ok(None) => Err(Failure::Message(tr!("Log in to your Epic account first").into())),
            Err(err) => Err(Failure::from_error(err)),
        }
    }
}

impl EpicService for SystemEpic {
    fn update_available(&self, game: &str) -> Result<bool, Failure> {
        let session = self.session(false)?.expect("online sessions are signed in");
        epic::update_available(&self.client, &self.config_dir, &session, game).map_err(Failure::from_error)
    }

    fn launch_params(&self, launch: &EpicLaunch) -> Result<LaunchParams, Failure> {
        let session = self.session(launch.offline)?;
        let opts = epic::LaunchOptions {
            offline: launch.offline,
            language: launch.language.as_deref(),
            override_exe: launch.override_exe.as_deref(),
            user_args: &[],
        };
        epic::launch_params(&self.client, &self.config_dir, session.as_ref(), &launch.app_name, &opts).map_err(Failure::from_error)
    }

    fn store_link(&self, launch: &EpicLaunch, link: StoreLink) -> Result<String, Failure> {
        let session = self.session(launch.offline)?;
        let uri = match link {
            StoreLink::EaApp => epic::origin_uri(&self.client, &self.config_dir, session.as_ref(), &launch.app_name, launch.offline),
            StoreLink::Ubisoft => epic::ubisoft_uri(&self.client, &self.config_dir, session.as_ref(), &launch.app_name, launch.offline),
        };
        uri.map_err(Failure::from_error)
    }
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

/// A stand-in for Epic's services (tests).
#[derive(Default)]
pub struct FakeEpic {
    pub update: bool,
    pub params: LaunchParams,
    pub link: String,
    /// Answer every request with this failure.
    pub fail: Option<String>,
    /// Take this long to answer.
    pub delay: Duration,
    /// The launches asked for.
    pub asked: parking_lot::Mutex<Vec<EpicLaunch>>,
}

impl FakeEpic {
    fn answer<T>(&self, launch: Option<&EpicLaunch>, value: T) -> Result<T, Failure> {
        std::thread::sleep(self.delay);
        if let Some(l) = launch {
            self.asked.lock().push(l.clone());
        }
        match &self.fail {
            Some(why) => Err(Failure::Message(why.clone())),
            None => Ok(value),
        }
    }
}

impl EpicService for FakeEpic {
    fn update_available(&self, _game: &str) -> Result<bool, Failure> {
        self.answer(None, self.update)
    }
    fn launch_params(&self, launch: &EpicLaunch) -> Result<LaunchParams, Failure> {
        self.answer(Some(launch), self.params.clone())
    }
    fn store_link(&self, launch: &EpicLaunch, _link: StoreLink) -> Result<String, Failure> {
        self.answer(Some(launch), self.link.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params() -> LaunchParams {
        LaunchParams {
            game_executable: "Binaries/Win64/Control.exe".into(),
            game_directory: "/games/Control".into(),
            working_directory: "/games/Control/Binaries/Win64".into(),
            game_parameters: vec!["-nolauncher".into()],
            egl_parameters: vec!["-AUTH_LOGIN=unused".into(), "-AUTH_PASSWORD=abc".into(), "-epicapp=Calluna".into()],
            user_parameters: vec![],
        }
    }

    fn wine_command() -> Command {
        let args = ["/umu/umu_run.py", "/games/Control/Control.exe", "-dx11"].map(String::from).to_vec();
        Command { program: "/usr/bin/gamemoderun".into(), args, ..Default::default() }
    }

    fn launch() -> EpicLaunch {
        EpicLaunch { app_name: "Calluna".into(), game: "Calluna".into(), exe_index: Some(1), check_update: true, ..Default::default() }
    }

    #[test]
    fn the_program_gets_the_builds_then_the_users_then_epics_arguments() {
        let epic = FakeEpic { params: params(), ..Default::default() };
        let Resolved::Game(c) = resolve(&wine_command(), &launch(), &epic).unwrap() else { panic!() };
        assert_eq!(c.program, "/usr/bin/gamemoderun");
        let exe = native_path("/games/Control/Binaries/Win64/Control.exe");
        assert_eq!(
            c.args,
            ["/umu/umu_run.py", &exe, "-nolauncher", "-dx11", "-AUTH_LOGIN=unused", "-AUTH_PASSWORD=abc", "-epicapp=Calluna"]
        );
        assert_eq!(c.cwd.as_deref(), Some(native_path("/games/Control/Binaries/Win64").as_str()));
        // The sign-in never reaches the game's log.
        let shown = crate::log_writer::command_line_for_log(&c);
        assert!(shown.contains("-AUTH_PASSWORD=<hidden>") && !shown.contains("abc"), "{shown}");
    }

    #[test]
    fn on_windows_the_program_is_the_command() {
        let epic = FakeEpic { params: params(), ..Default::default() };
        let cmd = Command { program: r"C:\Games\Control\Control.exe".into(), args: vec!["-dx11".into()], ..Default::default() };
        let l = EpicLaunch { exe_index: None, ..launch() };
        let Resolved::Game(c) = resolve(&cmd, &l, &epic).unwrap() else { panic!() };
        assert!(c.program.ends_with("Control.exe") && c.program.contains("Win64"), "{}", c.program);
        assert_eq!(c.args, ["-nolauncher", "-dx11", "-AUTH_LOGIN=unused", "-AUTH_PASSWORD=abc", "-epicapp=Calluna"]);
    }

    #[test]
    fn an_update_stops_the_launch_unless_play_anyway() {
        let epic = FakeEpic { params: params(), update: true, ..Default::default() };
        let Err(Failure::Message(why)) = resolve(&wine_command(), &launch(), &epic) else { panic!() };
        assert!(why.contains("Play anyway"), "{why}");
        let anyway = EpicLaunch { check_update: false, ..launch() };
        assert!(matches!(resolve(&wine_command(), &anyway, &epic), Ok(Resolved::Game(_))));
        // Offline nothing is asked of Epic but the arguments.
        let offline = EpicLaunch { offline: true, ..launch() };
        assert!(matches!(resolve(&wine_command(), &offline, &epic), Ok(Resolved::Game(_))));
        assert!(epic.asked.lock().iter().any(|l| l.offline));
    }

    #[test]
    fn store_app_games_get_their_link() {
        let epic = FakeEpic { link: "link2ea://launchgame/Ori?AUTH_PASSWORD=x".into(), ..Default::default() };
        let windows = EpicLaunch { exe_index: None, store_link: Some(StoreLink::EaApp), check_update: false, ..launch() };
        let cmd = Command::default();
        assert_eq!(resolve(&cmd, &windows, &epic).unwrap(), Resolved::Link("link2ea://launchgame/Ori?AUTH_PASSWORD=x".into()));
        // Under Wine, `start` opens it in the prefix.
        let wine = Command { program: "/umu/umu_run.py".into(), args: vec!["start".into()], ..Default::default() };
        let l = EpicLaunch { exe_index: Some(0), ..windows };
        let Resolved::Game(c) = resolve(&wine, &l, &epic).unwrap() else { panic!() };
        assert_eq!(c.args, ["start", "link2ea://launchgame/Ori?AUTH_PASSWORD=x"]);
        assert_eq!(link_client("uplay://launch/1"), "Ubisoft Connect");
        assert_eq!(link_client("link2ea://launchgame/x"), "EA app");
        assert!(!link_handler_installed("heronite-no-such-scheme://x"));
        assert!(!link_handler_installed("no scheme"));
    }

    #[test]
    fn epics_refusal_is_explained() {
        let epic = FakeEpic { fail: Some("Log in to your Epic account first".into()), ..Default::default() };
        let l = EpicLaunch { check_update: false, ..launch() };
        assert_eq!(resolve(&wine_command(), &l, &epic), Err(Failure::Message("Log in to your Epic account first".into())));
        let refused =
            Failure::from_error(anyhow::anyhow!(heronite_epic_api::SignInRefused { code: "invalid_grant".into(), continuation_url: None }));
        assert_eq!(refused, Failure::Message("The sign-in expired: log in again".into()));
    }

    #[test]
    fn a_stop_while_epic_signs_in_cancels() {
        let epic = Arc::new(FakeEpic { params: params(), delay: Duration::from_secs(30), ..Default::default() });
        let started = Instant::now();
        let got = resolve_cancellable(&wine_command(), &launch(), epic, &|| started.elapsed() > Duration::from_millis(300));
        assert_eq!(got, Err(Failure::Cancelled));
        assert!(started.elapsed() < Duration::from_secs(5));
    }
}
