mod assets;
mod collection;
mod dialogs;
mod icons;
mod images;
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
    env_logger_init();

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
        SingleInstance::Primary { server, messages } => (server, messages),
    };

    gpui_kit::application()
        .with_assets(assets::Assets)
        // The windows come and go (closed to the tray); `system` decides when to quit.
        .with_quit_mode(QuitMode::Explicit)
        .run(move |cx| {
            gpui_kit::init(cx);
            assets::load_fonts(cx);
            theme::apply_component_theme(cx);
            system::init(cx, cli, started, messages);
        });

    // Removes the socket of the single-instance server.
    drop(server);
    let code = system::exit_code();
    if code != 0 {
        std::process::exit(code);
    }
}

enum SingleInstance {
    /// Another instance took the request: nothing left to do.
    Forwarded,
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
    if std::env::var("HERONITE_NO_SINGLE_INSTANCE").is_ok_and(|v| !v.is_empty() && v != "0") {
        return primary(None, None);
    }
    let socket = socket_path(&heronite_data::HeroicPaths::detect().root);
    let msg = Message { launch: cli.launch.clone(), no_gui: cli.no_gui };
    for _ in 0..2 {
        match forward(&socket, &msg) {
            Ok(Forward::Delivered) => {
                log::info!("handed the request to the running instance");
                return SingleInstance::Forwarded;
            }
            Ok(Forward::NoInstance) => {}
            Err(err) => {
                log::warn!("could not reach the running instance: {err}");
                return primary(None, None);
            }
        }
        let (tx, rx) = futures::channel::mpsc::unbounded();
        match Server::bind(&socket, move |m| {
            let _ = tx.unbounded_send(m);
        }) {
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

/// Startup parses several MB of JSON on worker threads; glibc keeps the
/// freed pages in per-thread arenas. Give them back once loading settled.
pub fn trim_heap_later() {
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    std::thread::spawn(|| {
        std::thread::sleep(std::time::Duration::from_secs(3));
        // SAFETY: malloc_trim only releases free memory.
        unsafe { libc::malloc_trim(0) };
    });
}

fn env_logger_init() {
    // Minimal stderr logger; RUST_LOG=debug for more.
    struct L(log::LevelFilter);
    impl log::Log for L {
        fn enabled(&self, m: &log::Metadata) -> bool {
            // Our crates at the chosen level, third-party crates at warn
            // (zbus/wgpu/naga are very chatty at info/debug).
            let ours = m.target().starts_with("heronite");
            m.level() <= if ours { self.0 } else { self.0.min(log::LevelFilter::Warn) }
        }
        fn log(&self, r: &log::Record) {
            if self.enabled(r.metadata()) {
                eprintln!("[{} {}] {}", r.level(), r.target(), r.args());
            }
        }
        fn flush(&self) {}
    }
    let level = match std::env::var("RUST_LOG").as_deref() {
        Ok("debug") => log::LevelFilter::Debug,
        Ok("trace") => log::LevelFilter::Trace,
        _ => log::LevelFilter::Info,
    };
    let _ = log::set_boxed_logger(Box::new(L(level))).map(|_| log::set_max_level(level));
}
