mod assets;
mod collection;
mod console;
mod dialogs;
mod env;
mod icons;
mod images;
mod language;
mod pages;
mod runtime;
mod services;
mod system;
mod theme;
mod ui;

use std::time::Instant;

use gpui_kit::*;

fn main() {
    let started = Instant::now();
    tune_allocator();
    // First, so even the migration's error is in the user's language (only
    // reads a file).
    language::init(&heronite_data::AppPaths::for_data(&heronite_data::DataPaths::detect()));
    // Before anything (the logger included) creates the data folder.
    let migration = migrate_from_heroic();
    env_logger_init();
    log::info!("interface language: {}", heronite_i18n::current().code());
    match migration {
        heronite_data::migrate::Outcome::Migrated(r) => log::info!(
            "copied the Heroic setup into the data folder: {} files, {} MB, {} files with rewritten paths, legendary from {:?}",
            r.files,
            r.bytes / 1_000_000,
            r.rewritten,
            r.legendary
        ),
        heronite_data::migrate::Outcome::Fresh => log::info!("new data folder (no Heroic setup to copy)"),
        heronite_data::migrate::Outcome::NotNeeded => {}
    }

    let cli = system::cli::parse(std::env::args().skip(1));
    if cli.help {
        print!("{}", system::cli::HELP);
        return;
    }
    for err in &cli.errors {
        eprintln!("heronite-rs: {err}");
    }
    if !cli.errors.is_empty() && cli.launch.is_none() {
        std::process::exit(2);
    }

    // One instance per data folder: a second start hands its request to the
    // first and exits (Heroic's `requestSingleInstanceLock`).
    let (server, messages) = match single_instance(&cli) {
        SingleInstance::Forwarded => return,
        SingleInstance::Unresponsive(reason) => {
            // Starting a second, unlocked instance on the same data would
            // double-run the Epic queue, the tray and every game record.
            let text = format!("Heronite is already running but is not responding ({reason})");
            eprintln!("heronite-rs: {text}");
            system::notify::send_and_wait("Heronite", text, std::time::Duration::from_secs(3));
            std::process::exit(1);
        }
        SingleInstance::Primary { server, messages } => (server, messages),
    };
    // SIGTERM / SIGINT / SIGHUP are a quit (see `system::signals`).
    let signals = system::signals::spawn();
    // Temp files of writes cut short by a crash or kill (only ours, by name).
    std::thread::spawn(|| {
        let data_paths = heronite_data::DataPaths::detect();
        let app = heronite_data::AppPaths::for_data(&data_paths);
        heronite_shortcuts::sweep_stale_temps_at_startup(&data_paths, &app);
    });

    gpui_kit::application()
        .with_assets(assets::Assets)
        // The windows come and go (closed to the tray); `system` decides when to quit.
        .with_quit_mode(QuitMode::Explicit)
        .run(move |cx| {
            gpui_kit::init(cx);
            assets::load_fonts(cx);
            theme::apply_component_theme(cx);
            system::init(cx, cli, started, messages, signals);
        });

    // Removes the socket of the single-instance server.
    drop(server);
    // "Restart now" after an update: the new version takes over the socket.
    system::relaunch_if_requested();
    let code = system::exit_code();
    if code != 0 {
        std::process::exit(code);
    }
}

/// First start: copies an existing Heroic setup into the data folder (see
/// `heronite_data::migrate`). A failure leaves no data folder behind, so
/// the app stops instead of starting empty and retries on the next start.
fn migrate_from_heroic() -> heronite_data::migrate::Outcome {
    let data_paths = heronite_data::DataPaths::detect();
    let app = heronite_data::AppPaths::for_data(&data_paths);
    match heronite_data::migrate::run_if_needed(&data_paths, &app) {
        Ok(outcome) => outcome,
        Err(err) => {
            let text = format!("Could not copy the Heroic data into {}: {err:#}", data_paths.root.display());
            eprintln!("heronite-rs: {text}");
            system::notify::send_and_wait("Heronite", text, std::time::Duration::from_secs(5));
            std::process::exit(1);
        }
    }
}

enum SingleInstance {
    /// Another instance took the request: nothing left to do.
    Forwarded,
    /// A running instance holds the socket but did not take the request.
    Unresponsive(String),
    Primary {
        server: Option<system::single_instance::Server>,
        messages: Option<futures::channel::mpsc::UnboundedReceiver<system::single_instance::Message>>,
    },
}

/// `HERONITE_NO_SINGLE_INSTANCE=1` skips all of this (screenshot runs and
/// tests never talk to the user's running instance).
fn single_instance(cli: &system::cli::Cli) -> SingleInstance {
    use system::single_instance::{forward, socket_path, Forward, Message, Server};
    let primary = |server, messages| SingleInstance::Primary { server, messages };
    if env::flag("HERONITE_NO_SINGLE_INSTANCE") {
        return primary(None, None);
    }
    let socket = socket_path(&heronite_data::DataPaths::detect().root);
    let msg = Message { launch: cli.launch.clone(), no_gui: cli.no_gui };
    for _ in 0..2 {
        match forward(&socket, &msg) {
            Ok(Forward::Delivered) => {
                log::info!("handed the request to the running instance");
                return SingleInstance::Forwarded;
            }
            Ok(Forward::NoInstance) => {}
            Ok(Forward::Unresponsive(reason)) => {
                log::error!("the running instance did not take the request: {reason}");
                return SingleInstance::Unresponsive(reason);
            }
            Err(err) => {
                log::warn!("could not reach the running instance: {err}");
                return primary(None, None);
            }
        }
        let (tx, rx) = futures::channel::mpsc::unbounded();
        // `ok` goes back to the other process only if the request was queued
        // for the instance (the receiver is gone while it shuts down).
        match Server::bind(&socket, move |m| tx.unbounded_send(m).is_ok()) {
            Ok(server) => return primary(Some(server), Some(rx)),
            // Another instance won the race: forward to it on the next round.
            Err(err) if err.kind() == std::io::ErrorKind::AddrInUse => continue,
            Err(err) => {
                log::warn!("single instance disabled ({}): {err}", socket.display());
                return primary(None, None);
            }
        }
    }
    primary(None, None)
}

/// The app stays open for days and decodes large images on worker threads.
/// glibc's defaults suit short programs: big buffers move into the heap once
/// a few were freed (dynamic mmap threshold) and every thread gets its own
/// arena, so freed pixels stay in the process. Fixed thresholds send every
/// buffer above 128 KiB to `mmap` (back to the OS on free), and two arenas
/// bound the fragmentation.
fn tune_allocator() {
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    // SAFETY: mallopt only changes allocator tuning; called before any thread starts.
    unsafe {
        libc::mallopt(libc::M_MMAP_THRESHOLD, 128 * 1024);
        libc::mallopt(libc::M_TRIM_THRESHOLD, 512 * 1024);
        libc::mallopt(libc::M_ARENA_MAX, 2);
    }
}

/// Gives freed heap pages back to the OS a moment after a burst of frees
/// (startup parsing, the window closing to the tray); glibc keeps them
/// otherwise.
pub fn trim_heap_later() {
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    std::thread::spawn(|| {
        std::thread::sleep(std::time::Duration::from_secs(3));
        // SAFETY: malloc_trim only releases free memory.
        unsafe { libc::malloc_trim(0) };
    });
}

fn env_logger_init() {
    // Records go to stderr and, unless `disableLogs` is on, to
    // `heronite-rs-<date>.log` (see `heronite_launch::log_writer`).
    struct L {
        level: log::LevelFilter,
        file: Option<heronite_launch::log_writer::LauncherLogger>,
    }
    impl log::Log for L {
        fn enabled(&self, m: &log::Metadata) -> bool {
            // Our crates at the chosen level, third-party crates at warn
            // (zbus/wgpu/naga are very chatty at info/debug).
            let ours = m.target().starts_with("heronite");
            m.level() <= if ours { self.level } else { self.level.min(log::LevelFilter::Warn) }
        }
        fn log(&self, r: &log::Record) {
            if !self.enabled(r.metadata()) {
                return;
            }
            eprintln!("[{} {}] {}", r.level(), r.target(), r.args());
            if let Some(file) = &self.file {
                use heronite_launch::log_writer::Level;
                let level = match r.level() {
                    log::Level::Error => Level::Error,
                    log::Level::Warn => Level::Warning,
                    log::Level::Info => Level::Info,
                    log::Level::Debug | log::Level::Trace => Level::Debug,
                };
                file.record(level, r.target(), &r.args().to_string());
            }
        }
        fn flush(&self) {}
    }
    let level = match std::env::var("RUST_LOG").as_deref() {
        Ok("debug") => log::LevelFilter::Debug,
        Ok("trace") => log::LevelFilter::Trace,
        _ => log::LevelFilter::Info,
    };
    let data_paths = heronite_data::DataPaths::detect();
    let disabled = heronite_launch::settings::GlobalSettings::load(&data_paths)
        .defaults
        .get("disableLogs")
        .is_some_and(|v| v.as_bool().unwrap_or(false));
    let file = heronite_launch::log_writer::LauncherLogger::open(heronite_launch::log_writer::LogPaths::for_data(&data_paths), disabled);
    if !disabled {
        file.note(&format!("============= Heronite {} started (pid {}) =============", env!("CARGO_PKG_VERSION"), std::process::id()));
    }
    let _ = log::set_boxed_logger(Box::new(L { level, file: Some(file) })).map(|_| log::set_max_level(level));
    if !disabled {
        // The system information of Heroic's log, gathered off the startup path.
        let _ = std::thread::Builder::new().name("log-sysinfo".into()).spawn(|| {
            log::info!(target: "heronite_app", "System Information:\n{}", pages::settings::logs::system_info_text());
        });
    }
    // Panics end up in the log file too.
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        log::error!(target: "heronite_app", "panic: {info}");
        default_hook(info);
    }));
}
