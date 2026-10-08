//! Finding and reading log files for the log viewer: which logs exist (the
//! launcher's, Heroic's, runners', every game's) and the last part of a file
//! without reading all of it (logs of a chatty game reach hundreds of MB).

use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use heronite_data::HeroicPaths;

use crate::log_writer::{old_path, GameLogType, LogPaths};

/// What a log belongs to.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum LogSource {
    /// This app (`heronite-rs-<date>.log`).
    Launcher,
    /// `heroic.log`, written by Heroic / Heronite.
    Heroic,
    /// `runners/<runner>.log`.
    Runner(String),
    /// `games/<appName>_<runner>/<type>.log`.
    Game { app_name: String, runner: String, ty: GameLogType },
    /// `GamesConfig/<appName>-lastPlay.log` of older Heroic versions.
    Legacy { app_name: String },
}

#[derive(Clone, Debug)]
pub struct LogEntry {
    pub source: LogSource,
    pub path: PathBuf,
    /// The previous run (`<file>.old`), when there is one.
    pub previous: Option<PathBuf>,
    pub size: u64,
    pub modified: Option<SystemTime>,
}

impl LogEntry {
    pub fn app_name(&self) -> Option<&str> {
        match &self.source {
            LogSource::Game { app_name, .. } | LogSource::Legacy { app_name } => Some(app_name),
            _ => None,
        }
    }
}

fn meta(path: &Path) -> Option<(u64, Option<SystemTime>)> {
    let m = std::fs::metadata(path).ok()?;
    m.is_file().then(|| (m.len(), m.modified().ok()))
}

fn entry(source: LogSource, path: PathBuf) -> Option<LogEntry> {
    let previous = Some(old_path(&path)).filter(|p| p.is_file());
    match meta(&path) {
        Some((size, modified)) => Some(LogEntry { source, path, previous, size, modified }),
        // Only the previous run is left (the current one was rotated away).
        None => {
            let old = previous?;
            let (size, modified) = meta(&old)?;
            Some(LogEntry { source, path: old, previous: None, size, modified })
        }
    }
}

/// `<appName>_<runner>` -> (`appName`, `runner`). App names may contain `_`.
pub fn split_game_dir(name: &str) -> Option<(&str, &str)> {
    let (app, runner) = name.rsplit_once('_')?;
    (!app.is_empty() && !runner.is_empty()).then_some((app, runner))
}

/// Every log that exists: launcher logs newest first, `heroic.log`, runners
/// by name, then games (and legacy per-game logs) newest first.
pub fn list_logs(paths: &LogPaths, heroic: &HeroicPaths) -> Vec<LogEntry> {
    let mut out = Vec::new();

    // Launcher: one file per day.
    let mut launcher: Vec<LogEntry> = read_dir_files(&paths.launcher_dir)
        .into_iter()
        .filter(|p| {
            let n = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
            n.starts_with("heronite-rs-") && n.ends_with(".log")
        })
        .filter_map(|p| entry(LogSource::Launcher, p))
        .collect();
    launcher.sort_by(|a, b| b.path.cmp(&a.path));
    out.extend(launcher);

    out.extend(entry(LogSource::Heroic, paths.heroic_log()));

    let mut runners: Vec<LogEntry> = read_dir_files(&paths.base.join("runners"))
        .into_iter()
        .filter_map(|p| {
            let name = p.file_name()?.to_string_lossy().to_string();
            let runner = name.strip_suffix(".log")?.to_string();
            entry(LogSource::Runner(runner), p)
        })
        .collect();
    runners.sort_by(|a, b| a.path.cmp(&b.path));
    out.extend(runners);

    let mut games: Vec<LogEntry> = Vec::new();
    if let Ok(rd) = std::fs::read_dir(paths.base.join("games")) {
        for dir in rd.flatten() {
            let name = dir.file_name().to_string_lossy().to_string();
            let Some((app, runner)) = split_game_dir(&name) else { continue };
            let mut seen = std::collections::BTreeSet::new();
            for file in read_dir_files(&dir.path()) {
                let fname = file.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                // `launch.log` or, when only the rotated one is left, `launch.log.old`.
                let Some(stem) = fname.strip_suffix(".log").or_else(|| fname.strip_suffix(".log.old")) else { continue };
                let Some(ty) = GameLogType::from_stem(stem) else { continue };
                if seen.insert(stem.to_string()) {
                    let current = dir.path().join(format!("{stem}.log"));
                    games.extend(entry(LogSource::Game { app_name: app.into(), runner: runner.into(), ty }, current));
                }
            }
        }
    }
    for file in read_dir_files(&heroic.games_config_dir()) {
        let fname = file.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        let Some(app) = fname.strip_suffix("-lastPlay.log") else { continue };
        games.extend(entry(LogSource::Legacy { app_name: app.into() }, file));
    }
    games.sort_by(|a, b| b.modified.cmp(&a.modified).then_with(|| a.path.cmp(&b.path)));
    out.extend(games);
    out
}

/// The logs of one game (any runner, any type).
pub fn game_logs(paths: &LogPaths, heroic: &HeroicPaths, app_name: &str) -> Vec<LogEntry> {
    list_logs(paths, heroic).into_iter().filter(|e| e.app_name() == Some(app_name)).collect()
}

fn read_dir_files(dir: &Path) -> Vec<PathBuf> {
    match std::fs::read_dir(dir) {
        Ok(rd) => rd.flatten().map(|e| e.path()).filter(|p| p.is_file()).collect(),
        Err(_) => Vec::new(),
    }
}

// ---------------------------------------------------------------------------
// Reading
// ---------------------------------------------------------------------------

/// The end of a file as text.
#[derive(Clone, Debug, Default)]
pub struct Tail {
    pub text: String,
    /// File size when read.
    pub total: u64,
    /// Bytes of the file not included (before `text`).
    pub skipped: u64,
}

impl Tail {
    pub fn is_complete(&self) -> bool {
        self.skipped == 0
    }
}

/// The last `max_bytes` of `path`, starting at a line start when the file is
/// cut. Invalid UTF-8 is replaced.
pub fn read_tail(path: &Path, max_bytes: u64) -> io::Result<Tail> {
    let mut f = File::open(path)?;
    let total = f.metadata()?.len();
    let start = total.saturating_sub(max_bytes);
    f.seek(SeekFrom::Start(start))?;
    let mut buf = Vec::with_capacity((total - start) as usize);
    f.take(total - start).read_to_end(&mut buf)?;
    let mut skip = 0usize;
    if start > 0 {
        // Begin after the first newline so no line is cut in half.
        if let Some(i) = buf.iter().position(|b| *b == b'\n') {
            if i + 1 < buf.len() {
                skip = i + 1;
            }
        }
    }
    Ok(Tail { text: String::from_utf8_lossy(&buf[skip..]).into_owned(), total, skipped: start + skip as u64 })
}

/// Text lines for display: ANSI colour codes removed and carriage-return
/// progress output reduced to its last state.
pub fn display_lines(text: &str) -> Vec<String> {
    text.lines().map(clean_line).collect()
}

fn clean_line(line: &str) -> String {
    let last = line.rsplit('\r').find(|s| !s.is_empty()).unwrap_or("");
    strip_ansi(last)
}

fn strip_ansi(s: &str) -> String {
    if !s.contains('\x1b') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\x1b' {
            out.push(c);
            continue;
        }
        if chars.peek() == Some(&'[') {
            chars.next();
            // Parameters then a final byte in @..~.
            for n in chars.by_ref() {
                if ('@'..='~').contains(&n) {
                    break;
                }
            }
        } else {
            chars.next();
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(p: &Path, text: &str) {
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }

    #[test]
    fn tail_starts_on_a_line_boundary() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("a.log");
        write(&p, "line one\nline two\nline three\n");
        let all = read_tail(&p, 1000).unwrap();
        assert!(all.is_complete());
        assert_eq!(all.text, "line one\nline two\nline three\n");
        let part = read_tail(&p, 15).unwrap();
        assert_eq!(part.text, "line three\n");
        assert!(!part.is_complete());
        assert_eq!(part.total, 29);
        assert_eq!(part.skipped, 18);
        // Cut exactly at a line start keeps that line... after the previous newline.
        let exact = read_tail(&p, 11).unwrap();
        assert!(exact.text.ends_with("line three\n"));
    }

    #[test]
    fn tail_of_a_big_file_reads_only_the_end() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("big.log");
        let mut text = String::new();
        for i in 0..200_000 {
            text.push_str(&format!("line {i}\n"));
        }
        write(&p, &text);
        let t = read_tail(&p, 64 * 1024).unwrap();
        assert!(t.text.len() <= 64 * 1024 && t.text.ends_with("line 199999\n"));
        assert!(t.text.starts_with("line "));
        assert!(t.skipped > 0 && t.total as usize == text.len());
    }

    #[test]
    fn tail_survives_invalid_utf8() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("b.log");
        std::fs::write(&p, b"ok\n\xff\xfe broken\nend\n").unwrap();
        let t = read_tail(&p, 100).unwrap();
        assert!(t.text.contains("broken") && t.text.ends_with("end\n"));
    }

    #[test]
    fn display_cleans_ansi_and_progress_lines() {
        assert_eq!(display_lines("\x1b[31mred\x1b[0m text"), ["red text"]);
        assert_eq!(display_lines("10%\r50%\r100%"), ["100%"]);
        assert_eq!(display_lines("a\n\nb"), ["a", "", "b"]);
    }

    #[test]
    fn lists_launcher_runner_game_and_legacy_logs() {
        let dir = tempfile::tempdir().unwrap();
        let heroic = HeroicPaths::at(dir.path().join("heroic"));
        let paths = LogPaths { base: dir.path().join("logs"), launcher_dir: dir.path().join("launcher") };
        write(&paths.launcher_dir.join("heronite-rs-2026-10-07.log"), "old day");
        write(&paths.launcher_dir.join("heronite-rs-2026-10-08.log"), "today");
        write(&paths.launcher_dir.join("notes.txt"), "ignored");
        write(&paths.heroic_log(), "heroic");
        write(&paths.runner("legendary"), "lg");
        write(&paths.game("playnite_ab-cd", "sideload", GameLogType::Launch), "launch");
        write(&old_path(&paths.game("playnite_ab-cd", "sideload", GameLogType::Launch)), "previous");
        write(&paths.game("Calluna", "legendary", GameLogType::Install), "install");
        // Only the rotated file is left.
        write(&old_path(&paths.game("Lonely", "gog", GameLogType::Launch)), "only old");
        write(&paths.base.join("games/weird/launch.log"), "no runner");
        write(&heroic.games_config_dir().join("emu_1-lastPlay.log"), "legacy");
        write(&heroic.games_config_dir().join("emu_1.json"), "{}");

        let all = list_logs(&paths, &heroic);
        let kinds: Vec<String> = all
            .iter()
            .map(|e| match &e.source {
                LogSource::Launcher => format!("launcher:{}", e.path.file_name().unwrap().to_string_lossy()),
                LogSource::Heroic => "heroic".into(),
                LogSource::Runner(r) => format!("runner:{r}"),
                LogSource::Game { app_name, runner, ty } => format!("game:{app_name}:{runner}:{}", ty.file_stem()),
                LogSource::Legacy { app_name } => format!("legacy:{app_name}"),
            })
            .collect();
        assert_eq!(&kinds[..4], ["launcher:heronite-rs-2026-10-08.log", "launcher:heronite-rs-2026-10-07.log", "heroic", "runner:legendary"]);
        for want in ["game:playnite_ab-cd:sideload:launch", "game:Calluna:legendary:install", "game:Lonely:gog:launch", "legacy:emu_1"] {
            assert!(kinds.iter().any(|k| k == want), "{want} in {kinds:?}");
        }
        assert_eq!(kinds.len(), 8, "{kinds:?}");
        let pl = all.iter().find(|e| e.app_name() == Some("playnite_ab-cd")).unwrap();
        assert!(pl.previous.is_some());
        let lonely = all.iter().find(|e| e.app_name() == Some("Lonely")).unwrap();
        assert!(lonely.path.to_string_lossy().ends_with(".old") && lonely.previous.is_none());
        assert_eq!(game_logs(&paths, &heroic, "Calluna").len(), 1);
        assert_eq!(split_game_dir("steam_1245620_sideload"), Some(("steam_1245620", "sideload")));
        assert_eq!(split_game_dir("nounderscore"), None);
    }
}
