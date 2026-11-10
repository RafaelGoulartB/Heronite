//! Runs an extension against a game and prints what it gives, for authors.
//!
//! ```text
//! cargo run -p heronite-metadata-ext --example run -- <folder|zip> inspect
//! cargo run -p heronite-metadata-ext --example run -- <folder|zip> find "Portal 2" [--steam 620] [--platform snes] [--file NAME]
//! cargo run -p heronite-metadata-ext --example run -- <folder|zip> search "Portal"
//! cargo run -p heronite-metadata-ext --example run -- <folder|zip> fetch <id>
//! cargo run -p heronite-metadata-ext --example run -- <folder|zip> test
//! ```
//!
//! `--set key=value` sets one of its settings, `--language pt-BR` the
//! interface language, `--platform` (an id, name or alias) makes the game
//! an emulated one and `--file` gives its file name. It runs with a fresh, temporary data folder and the
//! real network; set `RUST_LOG=info` to see the script's `ctx.log` lines.

use std::path::PathBuf;

use heronite_data::paths::DataPaths;
use heronite_metadata::sources::options::set_option_value;
use heronite_metadata::sources::OptionValue;
use heronite_metadata::{Http, Lookup, MetadataSource, SourceContext, SourceGame};
use heronite_metadata_ext::package;
use heronite_metadata_ext::source::ScriptSource;
use heronite_metadata_ext::Limits;

fn usage() -> ! {
    eprintln!("usage: run <folder|zip> (inspect | find <title> | search <query> | fetch <id> | test) [--steam ID] [--platform ID] [--file NAME] [--set key=value] [--language CODE]");
    std::process::exit(2)
}

fn print<T: serde::Serialize>(lookup: Lookup<T>) {
    match lookup {
        Lookup::Found(v) => println!("{}", serde_json::to_string_pretty(&v).unwrap_or_default()),
        Lookup::NotFound => println!("not found (or the script failed: see the log)"),
        Lookup::Unreachable => println!("unreachable"),
        Lookup::Skipped => println!("skipped (a required setting is empty?)"),
    }
}

fn main() {
    let _ = env_logger_lite();
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let mut flag = |name: &str| -> Vec<String> {
        let mut out = Vec::new();
        while let Some(i) = args.iter().position(|a| a == name) {
            if i + 1 >= args.len() {
                usage();
            }
            out.push(args.remove(i + 1));
            args.remove(i);
        }
        out
    };
    let steam = flag("--steam").pop();
    // As the app passes it: its platform id (`snes` -> `nintendo_super_nes`).
    let platform = flag("--platform").pop().map(|p| heronite_metadata::platforms::by_name(&p).map_or(p, |p| p.id.to_owned()));
    let file = flag("--file").pop();
    let language = flag("--language").pop();
    let sets = flag("--set");
    let (Some(path), Some(command)) = (args.first(), args.get(1)) else { usage() };
    let pkg = match package::read(&PathBuf::from(path)) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1)
        }
    };
    for w in &pkg.manifest.warnings {
        eprintln!("warning: {w}");
    }
    let source = match ScriptSource::new(pkg.manifest.clone(), &pkg.script, Limits::default(), None) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1)
        }
    };
    let data = std::env::temp_dir().join(format!("heronite-extension-run-{}", std::process::id()));
    let paths = DataPaths::at(&data);
    for set in &sets {
        let Some((key, value)) = set.split_once('=') else { usage() };
        let value = match value {
            "true" => OptionValue::Switch(true),
            "false" => OptionValue::Switch(false),
            v => OptionValue::Text(v.to_owned()),
        };
        if let Err(e) = set_option_value(&paths, source.descriptor(), key, value) {
            eprintln!("{e:#}");
            std::process::exit(1)
        }
    }
    let http = Http::default();
    let ctx = SourceContext::new(&paths, &http);
    let arg = args.get(2).cloned().unwrap_or_default();
    let emulated = platform.is_some();
    let game = SourceGame {
        app_name: "run".into(),
        runner: "sideload".into(),
        title: arg.clone(),
        steam_app_id: steam,
        language,
        emulated,
        platform,
        rom_file: file.clone().filter(|_| emulated),
        file_name: file,
        ..Default::default()
    };
    match command.as_str() {
        "inspect" => println!("{}", serde_json::to_string_pretty(source.descriptor()).unwrap_or_default()),
        "find" => print(source.lookup(&ctx, &game)),
        "search" => print(source.search(&ctx, &arg)),
        "fetch" => print(source.lookup_id(&ctx, &game, &arg)),
        "test" => match source.test(&ctx) {
            Some(t) => println!("{}", serde_json::to_string_pretty(&t).unwrap_or_default()),
            None => println!("it has no test"),
        },
        _ => usage(),
    }
    let _ = std::fs::remove_dir_all(&data);
}

/// Log lines (the script's `ctx.log`, failures) to stderr when `RUST_LOG`
/// is set, without a logger crate.
fn env_logger_lite() -> Result<(), log::SetLoggerError> {
    struct Stderr;
    impl log::Log for Stderr {
        fn enabled(&self, _: &log::Metadata<'_>) -> bool {
            true
        }
        fn log(&self, r: &log::Record<'_>) {
            eprintln!("[{}] {}", r.level(), r.args());
        }
        fn flush(&self) {}
    }
    if std::env::var_os("RUST_LOG").is_none() {
        return Ok(());
    }
    log::set_max_level(log::LevelFilter::Info);
    static LOGGER: Stderr = Stderr;
    log::set_logger(&LOGGER)
}
