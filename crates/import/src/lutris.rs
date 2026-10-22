//! Lutris: its library is `pga.db` (SQLite) in its data folder, each game's
//! settings a YAML file under `games/` of its config folder (the Wine
//! prefix is read from there, and the game's folder when `pga.db` has none:
//! a game added by hand in Lutris only has its executable). A game starts with
//! `lutris lutris:rungameid/<id>`; started that way while Lutris is closed,
//! Lutris quits with the game.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use heronite_data::launchers::Launcher;
use rusqlite::{Connection, OpenFlags};

use crate::{Found, Game};

const FLATPAK_ID: &str = "net.lutris.Lutris";

/// Where one Lutris install keeps its files.
struct Install {
    data: PathBuf,
    config: PathBuf,
    flatpak: bool,
}

fn installs(home: &Path) -> [Install; 2] {
    let data = crate::xdg(home, "XDG_DATA_HOME", ".local/share").join("lutris");
    let config = crate::xdg(home, "XDG_CONFIG_HOME", ".config").join("lutris");
    let flatpak = home.join(".var/app").join(FLATPAK_ID);
    [
        // Lutris keeps its config in the data folder when `~/.config/lutris` is missing.
        Install { config: if config.is_dir() { config } else { data.clone() }, data, flatpak: false },
        Install { data: flatpak.join("data/lutris"), config: flatpak.join("config/lutris"), flatpak: true },
    ]
}

pub fn detect(home: &Path) -> Result<Option<Found>> {
    let Some(install) = installs(home).into_iter().find(|i| i.data.join("pga.db").is_file()) else { return Ok(None) };
    let games = read(&install)?;
    Ok(Some(Found { launcher: Launcher::Lutris, location: install.data, flatpak: install.flatpak, games, heroic: None }))
}

fn command(flatpak: bool, id: i64) -> String {
    if flatpak {
        format!("flatpak run {FLATPAK_ID} lutris:rungameid/{id}")
    } else {
        format!("lutris lutris:rungameid/{id}")
    }
}

/// The installed games, but Steam's (the Steam import brings those).
fn read(install: &Install) -> Result<Vec<Game>> {
    let db = install.data.join("pga.db");
    let conn = Connection::open_with_flags(&db, OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX)
        .with_context(|| format!("opening {}", db.display()))?;
    conn.busy_timeout(std::time::Duration::from_secs(2))?;
    let mut stmt = conn.prepare(
        "SELECT id, name, slug, runner, platform, directory, playtime, lastplayed, configpath
         FROM games WHERE installed = 1 AND COALESCE(runner, '') <> 'steam' ORDER BY id",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            r.get::<_, Option<String>>(1)?.unwrap_or_default(),
            r.get::<_, Option<String>>(2)?.unwrap_or_default(),
            r.get::<_, Option<String>>(3)?.unwrap_or_default(),
            r.get::<_, Option<String>>(4)?.unwrap_or_default(),
            r.get::<_, Option<String>>(5)?.unwrap_or_default(),
            r.get::<_, Option<f64>>(6)?.unwrap_or(0.0),
            r.get::<_, Option<i64>>(7)?.unwrap_or(0),
            r.get::<_, Option<String>>(8)?.unwrap_or_default(),
        ))
    })?;
    let mut games = Vec::new();
    for row in rows {
        let (id, name, slug, runner, platform, directory, hours, last, configpath) = row?;
        if name.trim().is_empty() {
            continue;
        }
        let art = |dir: &str| ["jpg", "png"].iter().map(|ext| install.data.join(dir).join(format!("{slug}.{ext}"))).find(|p| p.is_file());
        let settings = if configpath.is_empty() {
            GameSection::default()
        } else {
            game_section(&install.config.join("games").join(format!("{configpath}.yml")))
        };
        let folder = Some(directory.trim().to_string()).filter(|d| !d.is_empty()).or_else(|| settings.folder());
        games.push(Game {
            id: id.to_string(),
            title: name.trim().to_string(),
            folder,
            windows: runner == "wine" || platform.to_ascii_lowercase().contains("windows"),
            minutes: (hours.max(0.0) * 60.0).round() as u64,
            last_played: (last > 0).then(|| heronite_data::steam_owned::unix_to_iso(last)).flatten(),
            cover: art("coverart").map(|p| p.to_string_lossy().into_owned()),
            banner: art("banners").map(|p| p.to_string_lossy().into_owned()),
            command: command(install.flatpak, id),
            prefix: settings.prefix,
        });
    }
    Ok(games)
}

/// The keys of a game's YAML settings under `game:` this reads.
#[derive(Debug, Default)]
struct GameSection {
    prefix: Option<String>,
    exe: Option<String>,
    working_dir: Option<String>,
    main_file: Option<String>,
}

impl GameSection {
    /// The game's folder from its settings: its working folder, else the
    /// folder of its executable (or main file). Only full paths.
    fn folder(&self) -> Option<String> {
        let full = |p: &Option<String>| p.as_deref().map(Path::new).filter(|p| p.is_absolute()).map(Path::to_path_buf);
        let file = full(&self.exe).or_else(|| full(&self.main_file)).and_then(|f| f.parent().map(Path::to_path_buf));
        full(&self.working_dir)
            .or(file)
            .filter(|d| d.components().count() > 2)
            .map(|d| d.to_string_lossy().trim_end_matches('/').to_string())
    }
}

/// `game:` of a game's YAML settings. Only a few plain keys are needed, so
/// the lines are read as Lutris writes them: `game:` at the start of a line,
/// its keys indented under it.
fn game_section(file: &Path) -> GameSection {
    let mut out = GameSection::default();
    let Ok(text) = std::fs::read_to_string(file) else { return out };
    let mut in_game = false;
    for line in text.lines() {
        if !line.starts_with([' ', '\t']) {
            in_game = line.trim_end() == "game:";
            continue;
        }
        if !in_game {
            continue;
        }
        let Some((key, value)) = line.trim_start().split_once(':') else { continue };
        let value = Some(value.trim().trim_matches(['\'', '"']).to_string()).filter(|v| !v.is_empty());
        match key {
            "prefix" => out.prefix = value,
            "exe" => out.exe = value,
            "working_dir" => out.working_dir = value,
            "main_file" => out.main_file = value,
            _ => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn library(data: &Path) {
        std::fs::create_dir_all(data).unwrap();
        let conn = Connection::open(data.join("pga.db")).unwrap();
        conn.execute_batch(
            "CREATE TABLE games (id INTEGER PRIMARY KEY, name TEXT, sortname TEXT, slug TEXT, installer_slug TEXT,
               parent_slug TEXT, platform TEXT, runner TEXT, executable TEXT, directory TEXT, updated DATETIME,
               lastplayed INTEGER, installed INTEGER, installed_at INTEGER, year INTEGER, configpath TEXT,
               has_custom_banner INTEGER, has_custom_icon INTEGER, has_custom_coverart_big INTEGER, playtime REAL,
               service TEXT, service_id TEXT, discord_id TEXT);
             INSERT INTO games (id, name, slug, platform, runner, directory, installed, playtime, lastplayed, configpath) VALUES
               (3, 'Celeste', 'celeste', 'Windows', 'wine', '/games/celeste', 1, 1.5, 1700000000, 'celeste-1700'),
               (4, 'SuperTux', 'supertux', 'Linux', 'linux', '/games/supertux', 1, NULL, NULL, 'supertux-1'),
               (5, 'Portal', 'portal', 'Linux', 'steam', '', 1, 2.0, NULL, 'portal-1'),
               (6, 'Hades', 'hades', 'Windows', 'wine', '', 0, 0, NULL, 'hades-1'),
               (7, 'Hollow Knight', 'hollow-knight', NULL, 'wine', '', 1, 0, NULL, 'hollow-knight-1');",
        )
        .unwrap();
    }

    #[test]
    fn reads_the_installed_games_but_steams() {
        let h = tempfile::tempdir().unwrap();
        let home = h.path();
        let data = home.join(".var/app/net.lutris.Lutris/data/lutris");
        library(&data);
        std::fs::create_dir_all(data.join("coverart")).unwrap();
        std::fs::write(data.join("coverart/celeste.jpg"), b"x").unwrap();
        let config = home.join(".var/app/net.lutris.Lutris/config/lutris/games");
        std::fs::create_dir_all(&config).unwrap();
        std::fs::write(
            config.join("celeste-1700.yml"),
            "game:\n  exe: /games/celeste/Celeste.exe\n  prefix: '/home/u/Games/celeste'\nsystem:\n  prefix: /wrong\n",
        )
        .unwrap();
        // Added by hand in Lutris: no directory, only the executable.
        std::fs::write(
            config.join("hollow-knight-1.yml"),
            "game:\n  exe: /data/Games/Hollow Knight/hollow_knight.exe\n  prefix: /data/Games/Prex\n",
        )
        .unwrap();

        let found = detect(home).unwrap().unwrap();
        assert!(found.flatpak);
        let names: Vec<_> = found.games.iter().map(|g| g.title.as_str()).collect();
        assert_eq!(names, ["Celeste", "SuperTux", "Hollow Knight"]);
        let hk = &found.games[2];
        assert_eq!((hk.folder.as_deref(), hk.prefix.as_deref()), (Some("/data/Games/Hollow Knight"), Some("/data/Games/Prex")));
        assert!(hk.windows);
        let celeste = &found.games[0];
        assert_eq!(celeste.id, "3");
        assert!(celeste.windows);
        assert_eq!(celeste.minutes, 90);
        assert_eq!(celeste.last_played.as_deref(), Some("2023-11-14T22:13:20.000Z"));
        assert_eq!(celeste.prefix.as_deref(), Some("/home/u/Games/celeste"));
        assert_eq!(celeste.command, "flatpak run net.lutris.Lutris lutris:rungameid/3");
        assert!(celeste.cover.as_deref().is_some_and(|c| c.ends_with("coverart/celeste.jpg")));
        let tux = &found.games[1];
        assert!(!tux.windows);
        assert_eq!((tux.minutes, tux.prefix.as_deref(), tux.cover.as_deref()), (0, None, None));
    }

    #[test]
    fn nothing_without_a_library() {
        let h = tempfile::tempdir().unwrap();
        assert!(detect(h.path()).unwrap().is_none());
    }
}
