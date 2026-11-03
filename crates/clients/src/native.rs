//! The clients on Windows, where they run as they are: setting one up with
//! its official installer (a client already installed is used as it is),
//! opening and closing it, and telling whether it or one of its games runs.
//!
//! This PC's processes are told apart by name and by where their program
//! is: the client's programs run from its folder (`launcher.exe` or
//! `agent.exe` could be any program's), its games from theirs.

use std::path::{Path, PathBuf};
use std::process::{Child, Stdio};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, Result};
use heronite_data::proc::Cancel;
use heronite_data::DataPaths;
use heronite_i18n::tr;
use heronite_launch::procs::{Procs, Sig, WinProgram};
use heronite_platform::shell::ElevatedError;

use crate::setup::Step;
use crate::spawn::Waits;
use crate::{find_exe, Client, Place};

/// `ERROR_ELEVATION_REQUIRED`: the installer's manifest asks for
/// administrator rights.
const ELEVATION_REQUIRED: i32 = 740;

/// The installers' programs, as Windows names their processes. While one
/// runs the setup is not done. Not `msiexec.exe`: Windows' installer
/// service keeps it running for minutes after any install.
const INSTALLERS: [&str; 4] = ["battle.net-setup.exe", "eaappinstaller.exe", "ubisoftconnectinstaller.exe", "rockstar-games-launcher.exe"];

/// How often closing looks whether the client is gone.
const CLOSE_POLL: Duration = Duration::from_millis(200);

/// An installer that was started.
pub(crate) trait Running {
    /// Whether it ended; an error when it never ran (the administrator
    /// prompt was declined, or the program could not start).
    fn ended(&mut self) -> Result<bool, ElevatedError>;
    /// Ends it, when this app can (not one running as administrator).
    fn stop(&mut self);
}

/// Starts an installer (a trait so tests start nothing).
pub(crate) trait Start {
    fn start(&self, program: &Path, args: &[String]) -> Result<Box<dyn Running>>;
}

/// Starts the installer as it is, or through Windows' administrator prompt
/// when Windows refuses to start it without (its manifest asks for it).
pub(crate) struct SystemStart;

impl Start for SystemStart {
    fn start(&self, program: &Path, args: &[String]) -> Result<Box<dyn Running>> {
        let mut cmd = std::process::Command::new(program);
        cmd.args(args).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
        if let Some(dir) = program.parent() {
            cmd.current_dir(dir);
        }
        // Not tied to the app: the installer may start the client, which outlives it.
        heronite_platform::process::detach(&mut cmd);
        match cmd.spawn() {
            Ok(child) => Ok(Box::new(child)),
            Err(e) if e.raw_os_error() == Some(ELEVATION_REQUIRED) => Ok(Box::new(Elevated::start(program, args))),
            Err(e) => Err(anyhow!(tr!("Could not start {program}: {error}", program = program.display(), error = e))),
        }
    }
}

impl Running for Child {
    fn ended(&mut self) -> Result<bool, ElevatedError> {
        self.try_wait().map(|s| s.is_some()).map_err(ElevatedError::Failed)
    }

    fn stop(&mut self) {
        let _ = self.kill();
        let _ = self.wait();
    }
}

/// An installer run through the administrator prompt: a thread waits for
/// the prompt and the installer (both block), and nothing here can end it.
struct Elevated(Receiver<Result<u32, ElevatedError>>);

impl Elevated {
    fn start(program: &Path, args: &[String]) -> Self {
        let (tx, rx) = mpsc::channel();
        let (program, args) = (program.to_path_buf(), args.to_vec());
        let dir = program.parent().map(Path::to_path_buf);
        std::thread::spawn(move || {
            let _ = tx.send(heronite_platform::shell::run_elevated(&program, &args, dir.as_deref()));
        });
        Self(rx)
    }
}

impl Running for Elevated {
    fn ended(&mut self) -> Result<bool, ElevatedError> {
        match self.0.try_recv() {
            Ok(Ok(code)) => {
                log::info!("the installer ended with code {code}");
                Ok(true)
            }
            Ok(Err(e)) => Err(e),
            Err(TryRecvError::Empty) => Ok(false),
            // Its answer was taken already.
            Err(TryRecvError::Disconnected) => Ok(true),
        }
    }

    fn stop(&mut self) {}
}

/// What the installer is given on Windows: under Wine's arguments (silent,
/// the client not started afterwards), but for Battle.net's setup, which
/// has no silent mode: its window asks the language and the folder, which
/// the user picks there.
fn installer_args(client: Client) -> Vec<String> {
    match client {
        Client::BattleNet => Vec::new(),
        other => crate::setup::installer_args(other),
    }
}

/// Sets `client` up on this PC (blocking; minutes): nothing to do when it
/// is installed already, else its official installer is downloaded and
/// run. Fails when cancelled, when the administrator prompt is declined, or
/// when the client is not there once the installer is done.
pub(crate) fn install(
    paths: &DataPaths,
    client: Client,
    place: &Place,
    procs: &dyn Procs,
    start: &dyn Start,
    cancel: &Cancel,
    on_step: &mut dyn FnMut(Step),
    waits: &Waits,
) -> Result<()> {
    let installed = || find_exe(client, place).is_some();
    if installed() {
        return Ok(());
    }
    let installer = crate::setup::get_installer(paths, client, cancel, on_step)?;
    if cancel.is_requested() {
        bail!(tr!("Cancelled"));
    }
    on_step(Step::Installing);
    let args = installer_args(client);
    log::info!("{}: running {} {}", client.name(), installer.display(), args.join(" "));
    let mut running = start.start(&installer, &args)?;
    let installers = || procs.prefix_programs("", None).iter().any(|p| INSTALLERS.contains(&p.name.as_str()));
    wait(&mut *running, &installers, &installed, cancel, waits, client, &installer)?;
    if !installed() {
        bail!(tr!("The installer of {client} closed before {client} was installed.", client = client.name()));
    }
    Ok(())
}

/// Waits for an installer, the way a setup under Wine does: done when it
/// ended and no installer program runs, or when the client is installed
/// and none has run for `waits.quiet` (an installer that starts the client
/// may stay). Cancelling or the timeout ends it, when it can be ended.
fn wait(
    running: &mut dyn Running,
    installers: &dyn Fn() -> bool,
    installed: &dyn Fn() -> bool,
    cancel: &Cancel,
    waits: &Waits,
    client: Client,
    program: &Path,
) -> Result<()> {
    let started = Instant::now();
    let mut last_seen = Instant::now();
    loop {
        if cancel.is_requested() {
            running.stop();
            bail!(tr!("Cancelled"));
        }
        let ended = match running.ended() {
            Ok(ended) => ended,
            Err(ElevatedError::Declined) => {
                bail!(tr!("{client} was not installed: the administrator prompt was declined.", client = client.name()))
            }
            Err(ElevatedError::Failed(e)) => bail!(tr!("Could not start {program}: {error}", program = program.display(), error = e)),
        };
        let busy = installers();
        if busy {
            last_seen = Instant::now();
        }
        if !busy && (ended || (installed() && last_seen.elapsed() >= waits.quiet)) {
            return Ok(());
        }
        if started.elapsed() > waits.timeout {
            running.stop();
            bail!(tr!("{name} took too long to install", name = client.name()));
        }
        std::thread::sleep(waits.poll);
    }
}

fn not_installed(client: Client) -> String {
    tr!("{client} is not installed on this PC. Set it up in Stores → {client}.", client = client.name())
}

/// Starts the client's program on its own: it outlives the app, and a
/// client already open takes it as a request to show its window. The
/// child is the caller's to wait for.
pub(crate) fn open(client: Client, place: &Place) -> Result<Child, String> {
    let exe = find_exe(client, place).ok_or_else(|| not_installed(client))?;
    let mut cmd = std::process::Command::new(&exe);
    cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    if let Some(dir) = exe.parent() {
        cmd.current_dir(dir);
    }
    heronite_platform::process::detach(&mut cmd);
    cmd.spawn().map_err(|e| tr!("Could not start {client}: {error}", client = client.name(), error = e))
}

/// Whether the program at `path` (a Windows path) is inside `folder`.
/// Windows paths ignore case.
fn under(path: &str, folder: &Path) -> bool {
    let normal = |s: &str| s.replace('/', "\\").trim_end_matches('\\').to_lowercase();
    let folder = normal(&folder.to_string_lossy());
    !folder.is_empty() && normal(path).starts_with(&format!("{folder}\\"))
}

/// The client's programs running from the folder of its program `exe`.
fn own(procs: &dyn Procs, client: Client, exe: &Path) -> Vec<WinProgram> {
    let Some(folder) = exe.parent() else { return Vec::new() };
    procs.prefix_programs("", None).into_iter().filter(|p| client.programs().contains(&p.name.as_str()) && under(&p.path, folder)).collect()
}

/// `(the client's main program runs, one of its games runs)`: the client
/// from the folder of its program `exe`, a game from one of `games` (their
/// folders), from one look at the processes.
pub(crate) fn running(procs: &dyn Procs, client: Client, exe: &Path, games: &[PathBuf]) -> (bool, bool) {
    let programs = procs.prefix_programs("", None);
    let folder = exe.parent();
    let main = programs.iter().any(|p| p.name == client.main_program() && folder.is_some_and(|f| under(&p.path, f)));
    let game = programs.iter().any(|p| !client.programs().contains(&p.name.as_str()) && games.iter().any(|g| under(&p.path, g)));
    (main, game)
}

/// Ends the client's programs that run from its folder and waits, up to
/// `timeout`, for its main program to be gone. Its services (they run as
/// the system) stay, as Windows has them.
pub(crate) fn close(procs: &dyn Procs, client: Client, exe: &Path, timeout: Duration) -> Result<(), String> {
    for p in own(procs, client, exe) {
        procs.signal(p.pid, Sig::Term);
    }
    let deadline = Instant::now() + timeout;
    while own(procs, client, exe).iter().any(|p| p.name == client.main_program()) {
        if Instant::now() > deadline {
            return Err(tr!("{client} did not close in time", client = client.name()));
        }
        std::thread::sleep(CLOSE_POLL);
    }
    Ok(())
}

/// Closes the client installed on this PC.
pub(crate) fn close_here(procs: &dyn Procs, client: Client, timeout: Duration) -> Result<(), String> {
    let exe = find_exe(client, &Place::machine()).ok_or_else(|| not_installed(client))?;
    close(procs, client, &exe, timeout)
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;

    use heronite_launch::procs::FakeProcs;

    use super::*;

    fn program(pid: u32, path: &str) -> WinProgram {
        let name = path.rsplit(['\\', '/']).next().unwrap().to_lowercase();
        WinProgram { pid, name, path: path.into(), from_folder: false }
    }

    fn procs_with(programs: Vec<WinProgram>) -> FakeProcs {
        let procs = FakeProcs::new();
        for p in &programs {
            procs.start(p.pid, &p.name, None);
        }
        procs.set_win_programs(programs);
        procs
    }

    const ROCKSTAR: &str = r"C:\Program Files\Rockstar Games\Launcher";

    #[test]
    fn the_client_and_its_games_are_told_by_their_folders() {
        let exe = Path::new(ROCKSTAR).join("Launcher.exe");
        let gta = [PathBuf::from(r"C:\Program Files\Rockstar Games\Grand Theft Auto V")];
        let seen = |programs: Vec<WinProgram>| running(&procs_with(programs), Client::Rockstar, &exe, &gta);
        let other = program(5, r"D:\Other Game\Launcher.exe");
        assert_eq!(seen(vec![other.clone()]), (false, false), "another program's launcher");
        let main = program(6, r"c:\program files\rockstar games\launcher\LAUNCHER.EXE");
        assert_eq!(seen(vec![other, main.clone()]), (true, false));
        // A helper of the client in the game's folder is not the game.
        let helper = program(7, r"C:\Program Files\Rockstar Games\Grand Theft Auto V\RockstarErrorHandler.exe");
        assert_eq!(seen(vec![main.clone(), helper]), (true, false));
        let game = program(8, r"C:/Program Files/Rockstar Games/Grand Theft Auto V/GTA5.exe");
        assert_eq!(seen(vec![game.clone()]), (false, true));
        assert_eq!(seen(vec![main, game]), (true, true));
        assert!(!under(r"C:\Games\Hades II\Hades.exe", Path::new(r"C:\Games\Hades")));
        assert!(!under(r"C:\Games\Hades.exe", Path::new("")));
    }

    #[test]
    fn closing_ends_only_the_clients_programs_from_its_folder() {
        let exe = Path::new(ROCKSTAR).join("Launcher.exe");
        let main = program(10, &format!(r"{ROCKSTAR}\Launcher.exe"));
        let service = program(11, &format!(r"{ROCKSTAR}\RockstarService.exe"));
        let elsewhere = program(12, r"D:\Other Game\Launcher.exe");
        let game = program(13, r"C:\Program Files\Rockstar Games\Grand Theft Auto V\GTA5.exe");
        let procs = procs_with(vec![main.clone(), service, elsewhere, game]);
        // The fake's processes stay: the main program is still there at the end.
        let err = close(&procs, Client::Rockstar, &exe, Duration::from_millis(50)).unwrap_err();
        assert_eq!(err, "Rockstar Games Launcher did not close in time");
        let mut asked: Vec<u32> = procs.signals().iter().map(|(pid, _)| *pid).collect();
        asked.sort_unstable();
        assert_eq!(asked, [10, 11]);

        let procs = procs_with(vec![main]);
        std::thread::scope(|s| {
            s.spawn(|| {
                std::thread::sleep(Duration::from_millis(100));
                procs.exit(10);
            });
            close(&procs, Client::Rockstar, &exe, Duration::from_secs(10)).unwrap();
        });
        assert!(close(&FakeProcs::new(), Client::Rockstar, &exe, Duration::ZERO).is_ok(), "nothing to close");
    }

    /// An installer of the tests: it ends after `polls` looks, or never.
    struct FakeRunning {
        polls: Rc<Cell<u32>>,
        ends_after: Option<u32>,
        error: RefCell<Option<ElevatedError>>,
        stopped: Rc<Cell<bool>>,
    }

    impl Running for FakeRunning {
        fn ended(&mut self) -> Result<bool, ElevatedError> {
            if let Some(e) = self.error.borrow_mut().take() {
                return Err(e);
            }
            self.polls.set(self.polls.get() + 1);
            Ok(self.ends_after.is_some_and(|n| self.polls.get() >= n))
        }

        fn stop(&mut self) {
            self.stopped.set(true);
        }
    }

    fn quick() -> Waits {
        Waits { poll: Duration::from_millis(5), quiet: Duration::from_millis(40), timeout: Duration::from_secs(10) }
    }

    fn fake(ends_after: Option<u32>) -> (FakeRunning, Rc<Cell<u32>>, Rc<Cell<bool>>) {
        let (polls, stopped) = (Rc::new(Cell::new(0)), Rc::new(Cell::new(false)));
        (FakeRunning { polls: polls.clone(), ends_after, error: RefCell::new(None), stopped: stopped.clone() }, polls, stopped)
    }

    fn wait_for(
        running: &mut FakeRunning,
        busy: &dyn Fn() -> bool,
        installed: &dyn Fn() -> bool,
        cancel: &Cancel,
        waits: &Waits,
    ) -> Result<()> {
        wait(running, busy, installed, cancel, waits, Client::Ubisoft, Path::new("UbisoftConnectInstaller.exe"))
    }

    #[test]
    fn an_installer_is_done_when_it_ended_or_the_client_is_there() {
        let (mut ended, polls, _) = fake(Some(3));
        wait_for(&mut ended, &|| false, &|| false, &Cancel::new(), &quick()).unwrap();
        assert_eq!(polls.get(), 3);

        // Still running, but the client is there and no installer runs.
        let (mut stays, _, stopped) = fake(None);
        let started = Instant::now();
        wait_for(&mut stays, &|| false, &|| true, &Cancel::new(), &quick()).unwrap();
        assert!(started.elapsed() >= Duration::from_millis(40));
        assert!(!stopped.get(), "what may be the client is left running");

        let (mut never, _, stopped) = fake(None);
        let waits = Waits { timeout: Duration::from_millis(100), ..quick() };
        let err = wait_for(&mut never, &|| true, &|| true, &Cancel::new(), &waits).unwrap_err();
        assert_eq!(err.to_string(), "Ubisoft Connect took too long to install");
        assert!(stopped.get());
    }

    #[test]
    fn a_declined_prompt_or_a_cancel_ends_the_setup() {
        let (mut declined, ..) = fake(None);
        *declined.error.borrow_mut() = Some(ElevatedError::Declined);
        let err = wait_for(&mut declined, &|| false, &|| false, &Cancel::new(), &quick()).unwrap_err();
        assert_eq!(err.to_string(), "Ubisoft Connect was not installed: the administrator prompt was declined.");

        let (mut cancelled, _, stopped) = fake(None);
        let cancel = Cancel::new();
        cancel.request();
        assert_eq!(wait_for(&mut cancelled, &|| true, &|| false, &cancel, &quick()).unwrap_err().to_string(), "Cancelled");
        assert!(stopped.get());
    }

    /// Starts nothing: the installer "installs" the client into the fake PC.
    struct FakeStart {
        exe: PathBuf,
        args: RefCell<Vec<String>>,
    }

    impl Start for FakeStart {
        fn start(&self, program: &Path, args: &[String]) -> Result<Box<dyn Running>> {
            assert!(program.ends_with("UbisoftConnectInstaller.exe") || program.ends_with("Rockstar-Games-Launcher.exe"), "{program:?}");
            *self.args.borrow_mut() = args.to_vec();
            std::fs::create_dir_all(self.exe.parent().unwrap()).unwrap();
            std::fs::write(&self.exe, "MZ").unwrap();
            Ok(Box::new(fake(Some(1)).0))
        }
    }

    #[test]
    fn a_setup_runs_the_installer_once_and_a_client_already_there_needs_none() {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path().join("data"));
        let (pc, drive, _) = crate::place::tests::machine_in(dir.path(), r"Software\HeroniteClientsTest\none");
        // The installer was downloaded already.
        let installer = paths.tools_dir().join("redist/clients/Rockstar-Games-Launcher.exe");
        std::fs::create_dir_all(installer.parent().unwrap()).unwrap();
        std::fs::write(&installer, "MZ").unwrap();
        let exe = drive.join("Program_Files/Rockstar Games/Launcher/Launcher.exe");
        let start = FakeStart { exe: exe.clone(), args: RefCell::new(Vec::new()) };

        let mut steps = Vec::new();
        install(&paths, Client::Rockstar, &pc, &FakeProcs::new(), &start, &Cancel::new(), &mut |s| steps.push(s), &quick()).unwrap();
        assert_eq!(steps, [Step::Installing]);
        assert_eq!(*start.args.borrow(), ["/s", "/f"]);
        assert_eq!(find_exe(Client::Rockstar, &pc), Some(exe));

        // Installed: nothing runs, nothing is downloaded.
        let mut again = Vec::new();
        let nothing = FakeStart { exe: dir.path().join("unused.exe"), args: RefCell::new(Vec::new()) };
        install(&paths, Client::Rockstar, &pc, &FakeProcs::new(), &nothing, &Cancel::new(), &mut |s| again.push(s), &quick()).unwrap();
        assert!(again.is_empty());
        assert!(!dir.path().join("unused.exe").exists());
    }

    #[test]
    fn an_installer_that_leaves_no_client_says_so() {
        struct Leaves;
        impl Start for Leaves {
            fn start(&self, _program: &Path, _args: &[String]) -> Result<Box<dyn Running>> {
                Ok(Box::new(fake(Some(1)).0))
            }
        }
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path().join("data"));
        let (pc, ..) = crate::place::tests::machine_in(dir.path(), r"Software\HeroniteClientsTest\none");
        let installer = paths.tools_dir().join("redist/clients/Battle.net-Setup.exe");
        std::fs::create_dir_all(installer.parent().unwrap()).unwrap();
        std::fs::write(&installer, "MZ").unwrap();
        let err = install(&paths, Client::BattleNet, &pc, &FakeProcs::new(), &Leaves, &Cancel::new(), &mut |_| {}, &quick()).unwrap_err();
        assert_eq!(err.to_string(), "The installer of Battle.net closed before Battle.net was installed.");
        assert!(installer_args(Client::BattleNet).is_empty(), "its window asks the user");
        assert_eq!(installer_args(Client::Ubisoft), ["/S"]);
    }

    #[test]
    fn a_client_that_is_not_installed_does_not_open() {
        let dir = tempfile::tempdir().unwrap();
        let (pc, ..) = crate::place::tests::machine_in(dir.path(), r"Software\HeroniteClientsTest\none");
        assert_eq!(open(Client::Ea, &pc).unwrap_err(), "EA app is not installed on this PC. Set it up in Stores → EA app.");
    }
}
