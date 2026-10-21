//! A real game in the sandbox, through real Proton: Wine's Notepad plays the
//! game and Wine's Task Manager the program alongside it. Checks that the game
//! runs in its own namespaces, without the network and without the real home,
//! that the program alongside joins its sandbox, and that stopping the game
//! ends everything, holding nothing big in memory; with umu wanted or not,
//! Proton runs directly there. Ignored by default (opens windows, needs
//! bubblewrap and a Proton build). Run with a Proton folder:
//!   HERONITE_REAL_PROTON=~/.local/share/Steam/compatibilitytools.d/UMU-Proton-10.0-4 \
//!   cargo test -p heronite-launch --test real_sandbox -- --ignored --nocapture --test-threads=1

use std::sync::mpsc::channel;
use std::sync::Arc;
use std::time::{Duration, Instant};

use heronite_data::models::GameInfo;
use heronite_data::DataPaths;
use heronite_launch::procs::{Procs, SystemProcs};
use heronite_launch::settings::GameSettings;
use heronite_launch::steam::SteamLocator;
use heronite_launch::{
    build_plan_with_settings, LaunchEvent, LaunchOptions, LaunchRequest, Launcher, LauncherConfig, PlanContext, SystemSpawner,
};
use serde_json::json;

fn home() -> std::path::PathBuf {
    std::path::PathBuf::from(std::env::var("HOME").expect("HOME"))
}

fn proton() -> Option<String> {
    std::env::var("HERONITE_REAL_PROTON").ok().filter(|p| std::path::Path::new(p).join("proton").exists())
}

/// Pid of the Windows program `name` running in `prefix`.
fn windows_pid(prefix: &str, name: &str) -> Option<u32> {
    std::fs::read_dir("/proc").ok()?.flatten().find_map(|e| {
        let pid: u32 = e.file_name().to_str()?.parse().ok()?;
        let cmd = std::fs::read(format!("/proc/{pid}/cmdline")).ok()?;
        let argv0 = String::from_utf8_lossy(cmd.split(|b| *b == 0).next()?).to_lowercase();
        let env = std::fs::read(format!("/proc/{pid}/environ")).ok()?;
        let in_prefix = env.split(|b| *b == 0).any(|kv| kv.starts_with(format!("WINEPREFIX={prefix}").as_bytes()));
        (argv0.ends_with(&format!("\\{name}")) && in_prefix).then_some(pid)
    })
}

/// How many processes run in `prefix` (Wine's own included).
fn in_prefix(prefix: &str) -> usize {
    let Ok(dir) = std::fs::read_dir("/proc") else { return 0 };
    dir.flatten()
        .filter_map(|e| e.file_name().to_str()?.parse::<u32>().ok())
        .filter(|pid| {
            let env = std::fs::read(format!("/proc/{pid}/environ")).unwrap_or_default();
            env.split(|b| *b == 0).any(|kv| kv.starts_with(format!("WINEPREFIX={prefix}").as_bytes()))
        })
        .count()
}

/// Waits for `f`; the game's log is shown when it does not happen.
fn wait_for(what: &str, secs: u64, log: &std::path::Path, mut f: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(secs);
    while Instant::now() < deadline {
        if f() {
            return;
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    panic!("timed out waiting for {what}:\n{}", std::fs::read_to_string(log).unwrap_or_default());
}

/// Shared memory in use on the machine (`Shmem`, KiB): what tmpfs layers hold.
fn shmem_kib() -> u64 {
    let info = std::fs::read_to_string("/proc/meminfo").unwrap_or_default();
    info.lines().find_map(|l| l.strip_prefix("Shmem:")).and_then(|v| v.split_whitespace().next()?.parse().ok()).unwrap_or(0)
}

/// Whether `path` exists for the process `pid` (through its root).
fn sees(pid: u32, path: &std::path::Path) -> bool {
    std::path::Path::new(&format!("/proc/{pid}/root")).join(path.strip_prefix("/").unwrap()).exists()
}

fn run(proton: &str, umu: bool) {
    // On disk: in a `/tmp` held in memory the new prefix (Proton copies its
    // default one, hundreds of MiB) would count as the sandbox's memory.
    let cache = home().join(".cache");
    std::fs::create_dir_all(&cache).unwrap();
    let dir = tempfile::Builder::new().prefix("heronite-sandbox-test").tempdir_in(&cache).unwrap();
    let paths = DataPaths::at(dir.path().join("data"));
    // The umu this app downloads, without downloading it again.
    let umu_dir = paths.tools_dir().join("runtimes");
    std::fs::create_dir_all(&umu_dir).unwrap();
    std::os::unix::fs::symlink(home().join(".local/share/heronite/tools/runtimes/umu"), umu_dir.join("umu")).unwrap();
    // Something of the real home the game must not see.
    let secret = home().join(".cache/heronite-sandbox-test-secret");
    std::fs::write(&secret, "secret").unwrap();

    let prefix = dir.path().join("pfx").to_string_lossy().into_owned();
    let trainer = format!("{proton}/files/lib/wine/x86_64-windows/taskmgr.exe");
    let settings = json!({
        "wineVersion": {"bin": format!("{proton}/proton"), "name": "Proton", "type": "proton"},
        "winePrefix": prefix, "disableUMU": !umu,
        "heroniteCompanions": [{"path": trainer, "delay": 3}],
        "heroniteSandbox": {"enabled": true}
    });
    let settings = GameSettings::from_map(settings.as_object().unwrap(), &home().to_string_lossy());
    let game: GameInfo = serde_json::from_value(json!({
        "runner": "sideload", "app_name": "notepad", "title": "Notepad", "is_installed": true,
        "install": {"executable": "notepad", "platform": "Windows"}
    }))
    .unwrap();
    let ctx = PlanContext::load(&paths);
    let plan = build_plan_with_settings(&ctx, &heronite_launch::SystemProbe, &game, None, &settings, &LaunchOptions::default());
    let process = plan.process().unwrap_or_else(|| panic!("{plan:?}"));
    assert!(process.sandboxed, "{plan:?}");
    // umu wanted or not, Proton runs directly in the sandbox.
    assert!(matches!(plan, heronite_launch::LaunchPlan::Wine { runtime: heronite_launch::WineRuntime::Proton, .. }), "{plan:?}");
    eprintln!("[umu={umu}] notes: {:?}", process.notes);

    let (tx, rx) = channel();
    let procs: Arc<dyn Procs> = Arc::new(SystemProcs);
    let steam = SteamLocator { home: dir.path().into(), steam_path: dir.path().join("steam"), system_roots: vec![] };
    let launcher =
        Launcher::new(paths.clone(), steam, Arc::new(SystemSpawner::new(procs.clone())), procs.clone(), LauncherConfig::default(), tx);
    let shmem_before = shmem_kib();
    let started = Instant::now();
    launcher.launch(LaunchRequest { app_name: "notepad".into(), runner: "sideload".into(), title: "Notepad".into(), plan }).unwrap();

    let log_file = paths.root.join("logs/games/notepad_sideload/launch.log");
    wait_for("the game", 240, &log_file, || windows_pid(&prefix, "notepad.exe").is_some());
    eprintln!("[umu={umu}] game up after {:?}", started.elapsed());
    // Nothing big is held in memory: umu's container in a throwaway layer
    // held a copy of its Steam runtime there (about 700 MiB per container).
    let grown = shmem_kib().saturating_sub(shmem_before) / 1024;
    eprintln!("[umu={umu}] shared memory grew by {grown} MiB");
    if std::env::var("HERONITE_TMP_INSPECT").is_ok() {
        eprintln!("INSPECT");
        std::thread::sleep(Duration::from_secs(90));
    }
    assert!(grown < 200, "the sandbox holds {grown} MiB in memory");
    let game_pid = windows_pid(&prefix, "notepad.exe").unwrap();
    let own = procs.user_ns(std::process::id()).unwrap();
    assert_ne!(procs.user_ns(game_pid).unwrap(), own, "the game runs in the sandbox");
    assert!(!sees(game_pid, &secret), "the real home is hidden");
    assert!(sees(game_pid, std::path::Path::new(&prefix)), "the prefix is there");
    let net = std::fs::read_to_string(format!("/proc/{game_pid}/net/dev")).unwrap();
    let interfaces: Vec<&str> = net.lines().skip(2).filter_map(|l| l.split(':').next()).map(str::trim).collect();
    assert_eq!(interfaces, ["lo"], "no network");

    wait_for("the program alongside", 60, &log_file, || windows_pid(&prefix, "taskmgr.exe").is_some());
    eprintln!("[umu={umu}] program alongside up after {:?}", started.elapsed());
    let trainer_pid = windows_pid(&prefix, "taskmgr.exe").unwrap();
    // Its own sandbox would have its own `/tmp` and wineserver: same mounts.
    let mnt = |pid: u32| std::fs::read_link(format!("/proc/{pid}/ns/net")).unwrap();
    assert_eq!(mnt(trainer_pid), mnt(game_pid), "it joined the game's sandbox");
    // A Wine tool of the prefix while the game plays joins its sandbox.
    let mut tool = run_tool(&ctx, &settings, &prefix);
    wait_for("winecfg beside the game", 60, &log_file, || windows_pid(&prefix, "winecfg.exe").is_some());
    assert_eq!(mnt(windows_pid(&prefix, "winecfg.exe").unwrap()), mnt(game_pid), "winecfg joined the game's sandbox");
    let _ = tool.kill();
    let _ = tool.wait();
    eprintln!("[umu={umu}] winecfg joined the game's sandbox");
    std::thread::sleep(Duration::from_secs(2));
    assert!(launcher.is_running("notepad"));

    // Stop: everything in the sandbox ends with the session.
    let stopped = Instant::now();
    assert!(launcher.stop("notepad"));
    loop {
        match rx.recv_timeout(Duration::from_secs(60)).expect("an event") {
            LaunchEvent::Exited { .. } => break,
            LaunchEvent::Error { message, .. } => panic!("{message}"),
            _ => {}
        }
    }
    eprintln!("[umu={umu}] stopped after {:?}", stopped.elapsed());
    wait_for("the sandbox to be empty", 20, &log_file, || in_prefix(&prefix) == 0);
    let log = std::fs::read_to_string(&log_file).unwrap_or_default();
    assert!(log.contains("heronite-sandbox"), "{log}");

    // Alone, a tool gets a sandbox of its own.
    let mut tool = run_tool(&ctx, &settings, &prefix);
    wait_for("winecfg alone", 120, &log_file, || windows_pid(&prefix, "winecfg.exe").is_some());
    let alone = windows_pid(&prefix, "winecfg.exe").unwrap();
    assert_ne!(procs.user_ns(alone).unwrap(), own, "winecfg runs in a sandbox");
    assert!(!sees(alone, &secret), "the real home is hidden from it too");
    eprintln!("[umu={umu}] winecfg alone in its own sandbox");
    unsafe { libc::kill(alone as i32, libc::SIGTERM) };
    let _ = tool.wait();
    wait_for("the tool's sandbox to be empty", 30, &log_file, || in_prefix(&prefix) == 0);
    let _ = std::fs::remove_file(&secret);
}

/// Winecfg of the prefix, started as the game settings start it.
fn run_tool(ctx: &PlanContext, settings: &GameSettings, prefix: &str) -> std::process::Child {
    use heronite_launch::wine_tools::{build_tool_command, WineTool};
    let built = build_tool_command(ctx, &heronite_launch::SystemProbe, settings, &WineTool::Winecfg).unwrap();
    let c = heronite_launch::sandbox::into_running(built.command, prefix, None).unwrap();
    std::process::Command::new(&c.program)
        .args(&c.args)
        .envs(&c.env)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap()
}

#[test]
#[ignore]
fn a_game_in_the_sandbox_sees_only_what_it_is_given() {
    let Some(proton) = proton() else { panic!("set HERONITE_REAL_PROTON to a Proton folder") };
    let only = std::env::var("HERONITE_REAL_UMU").ok();
    for umu in [true, false] {
        if only.as_deref().is_none_or(|o| o == umu.to_string()) {
            run(&proton, umu);
        }
    }
}
