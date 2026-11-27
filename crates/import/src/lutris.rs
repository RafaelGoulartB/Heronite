//! Lutris: its library is `pga.db` (SQLite) in its data folder, each game's
//! settings a YAML file under `games/` of its config folder (the Wine
//! prefix is read from there, and the game's folder when `pga.db` has none:
//! a game added by hand in Lutris only has its executable). A game starts with
//! `lutris lutris:rungameid/<id>`; started that way while Lutris is closed,
//! Lutris quits with the game. To start a Wine or Linux game here instead,
//! its settings give its program, arguments, working folder, Wine build and
//! environment variables, falling back to Lutris' settings for all games
//! and for its runner ([`Install::settings`]).

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use heronite_data::launchers::Launcher;
use rusqlite::{Connection, OpenFlags};
use serde_json::Value;

use crate::{Found, Game, Own, Wine};

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
    let games = read(&install, home)?;
    Ok(Some(Found { launcher: Launcher::Lutris, location: install.data, flatpak: install.flatpak, games, heroic: None, in_use: false }))
}

fn command(flatpak: bool, id: i64) -> String {
    if flatpak {
        format!("flatpak run {FLATPAK_ID} lutris:rungameid/{id}")
    } else {
        format!("lutris lutris:rungameid/{id}")
    }
}

/// The installed games, but Steam's (the Steam import brings those).
fn read(install: &Install, home: &Path) -> Result<Vec<Game>> {
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
        let settings = install.settings(&runner, &configpath);
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
            own: own(install, home, &runner, directory.trim(), &settings),
            prefix: settings.prefix,
            runs_from: None,
            transfer: None,
        });
    }
    Ok(games)
}

/// What a game's settings say, laid over Lutris' settings for all games
/// (`system.yml`) and for its runner (`runners/<runner>.yml`), as Lutris
/// merges them.
#[derive(Debug, Default)]
struct Settings {
    prefix: Option<String>,
    exe: Option<String>,
    args: Option<String>,
    working_dir: Option<String>,
    main_file: Option<String>,
    /// `wine: version:`, a folder of `runners/wine` (or a Proton build).
    wine: Option<String>,
    /// `system: env:`, in order; a later file's value wins.
    env: Vec<(String, String)>,
}

impl Settings {
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

    /// Lays `doc` (one of Lutris' settings files) over these.
    fn read(&mut self, doc: &Value) {
        let get = |section: &str, key: &str| doc.get(section)?.get(key).and_then(crate::scalar).filter(|v| !v.trim().is_empty());
        for (slot, key) in [
            (&mut self.prefix, "prefix"),
            (&mut self.exe, "exe"),
            (&mut self.args, "args"),
            (&mut self.working_dir, "working_dir"),
            (&mut self.main_file, "main_file"),
        ] {
            if let Some(v) = get("game", key) {
                *slot = Some(v);
            }
        }
        if let Some(v) = get("wine", "version") {
            self.wine = Some(v);
        }
        let env = doc.get("system").and_then(|s| s.get("env")).and_then(Value::as_object);
        for (k, v) in env.into_iter().flatten() {
            let Some(v) = crate::scalar(v) else { continue };
            self.env.retain(|(key, _)| key != k);
            self.env.push((k.clone(), v));
        }
    }
}

impl Install {
    /// Lutris' settings for all games of `runner`, then the game's own file
    /// (`configpath` under `games/`; none for an empty one).
    fn settings(&self, runner: &str, configpath: &str) -> Settings {
        let mut out = Settings::default();
        let mut files = vec![self.config.join("system.yml")];
        if !runner.is_empty() && !runner.contains('/') {
            files.push(self.config.join("runners").join(format!("{runner}.yml")));
        }
        if !configpath.is_empty() {
            files.push(self.config.join("games").join(format!("{configpath}.yml")));
        }
        for file in files.iter().filter(|f| f.is_file()) {
            match crate::load_yaml(file) {
                Ok(doc) => out.read(&doc),
                Err(e) => log::warn!("Lutris: {e:#}"),
            }
        }
        out
    }

    /// The Wine build named `version`: one Lutris keeps (`runners/wine`, or
    /// Proton in `runners/proton`), else a Proton build Steam has.
    fn wine(&self, home: &Path, version: &str) -> Option<Wine> {
        if version.is_empty() || version.contains('/') {
            return None;
        }
        let wine = self.data.join("runners/wine").join(version).join("bin/wine");
        if wine.is_file() {
            return Some(Wine { bin: wine.to_string_lossy().into_owned(), name: version.to_string(), proton: false });
        }
        let proton = self.data.join("runners/proton").join(version).join("proton");
        if proton.is_file() {
            return Some(Wine { bin: proton.to_string_lossy().into_owned(), name: version.to_string(), proton: true });
        }
        crate::proton_named(home, version)
    }
}

/// How the game starts here: a Wine game (with its prefix) or a Linux one
/// whose program is known. Other runners (DOSBox, emulators...) only
/// Lutris starts.
fn own(install: &Install, home: &Path, runner: &str, directory: &str, settings: &Settings) -> Option<Own> {
    let wine = match runner {
        "wine" => true,
        "linux" => false,
        _ => return None,
    };
    if wine && settings.prefix.is_none() {
        return None;
    }
    // A program set without its folder is in the game's.
    let exe = PathBuf::from(settings.exe.as_deref()?);
    let program = if exe.is_absolute() { exe } else { Path::new(Some(directory).filter(|d| !d.is_empty())?).join(exe) };
    Some(Own {
        program: program.to_string_lossy().into_owned(),
        args: settings.args.clone().unwrap_or_default(),
        working_dir: settings.working_dir.clone().filter(|d| Path::new(d).is_absolute()),
        wine: if wine { settings.wine.as_deref().and_then(|v| install.wine(home, v)) } else { None },
        env: settings.env.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
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

    // Linux launchers' data and paths; imports are off on Windows (`Feature::LauncherImports`).
    #[cfg(unix)]
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

    /// How its Wine and Linux games start here: their settings over
    /// Lutris' own for all games and for the runner.
    #[cfg(unix)]
    #[test]
    fn how_its_games_start_here() {
        let h = tempfile::tempdir().unwrap();
        let home = h.path();
        let data = home.join(".local/share/lutris");
        let config = home.join(".config/lutris");
        library(&data);
        let conn = Connection::open(data.join("pga.db")).unwrap();
        conn.execute_batch(
            "INSERT INTO games (id, name, slug, platform, runner, directory, installed, configpath) VALUES
               (8, 'Doom', 'doom', 'Linux', 'dosbox', '/games/doom', 1, 'doom-1');",
        )
        .unwrap();
        let write = |path: PathBuf, text: &str| {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        };
        write(config.join("system.yml"), "system:\n  env:\n    DXVK_HUD: fps\n    MANGOHUD: 1\n");
        write(config.join("runners/wine.yml"), "wine:\n  version: wine-ge-8-26\nsystem:\n  env:\n    WINEDEBUG: -all\n");
        write(data.join("runners/wine/wine-ge-8-26/bin/wine"), "");
        write(
            config.join("games/celeste-1700.yml"),
            "game:\n  exe: /games/celeste/Celeste.exe\n  args: -windowed\n  prefix: /pfx/celeste\nsystem:\n  env:\n    DXVK_HUD: full\n",
        );
        // Its own Wine build, gone from this PC: this app's default one.
        write(config.join("games/hollow-knight-1.yml"), "game:\n  exe: hk.exe\n  prefix: /pfx/hk\nwine:\n  version: removed-build\n");
        write(config.join("games/supertux-1.yml"), "game:\n  exe: bin/supertux2\n  working_dir: /games/supertux/data\n");
        write(config.join("games/doom-1.yml"), "game:\n  main_file: /games/doom/DOOM.EXE\n");

        let found = detect(home).unwrap().unwrap();
        let by = |t: &str| found.games.iter().find(|g| g.title == t).unwrap();
        let celeste = by("Celeste").own.clone().unwrap();
        assert_eq!((celeste.program.as_str(), celeste.args.as_str()), ("/games/celeste/Celeste.exe", "-windowed"));
        let wine = celeste.wine.unwrap();
        assert_eq!((wine.name.as_str(), wine.proton), ("wine-ge-8-26", false));
        assert!(wine.bin.ends_with("runners/wine/wine-ge-8-26/bin/wine"));
        let env: Vec<_> = celeste.env.iter().map(|(k, v)| format!("{k}={v}")).collect();
        assert_eq!(env, ["MANGOHUD=1", "WINEDEBUG=-all", "DXVK_HUD=full"]);
        assert_eq!(by("Celeste").prefix.as_deref(), Some("/pfx/celeste"));
        // Hollow Knight has no folder in Lutris: a program without one is left to Lutris.
        assert!(by("Hollow Knight").own.is_none());
        let tux = by("SuperTux").own.clone().unwrap();
        assert_eq!(tux.program, "/games/supertux/bin/supertux2");
        assert_eq!((tux.working_dir.as_deref(), tux.wine), (Some("/games/supertux/data"), None));
        assert!(by("Doom").own.is_none());
    }

    #[cfg(unix)]
    #[test]
    fn a_wine_game_without_its_prefix_or_build_here() {
        let h = tempfile::tempdir().unwrap();
        let home = h.path();
        let data = home.join(".local/share/lutris");
        library(&data);
        let games = home.join(".config/lutris/games");
        std::fs::create_dir_all(&games).unwrap();
        std::fs::write(games.join("celeste-1700.yml"), "game:\n  exe: Celeste.exe\n").unwrap();
        std::fs::write(games.join("hollow-knight-1.yml"), "game:\n  exe: /g/hk.exe\n  prefix: /pfx\nwine:\n  version: gone\n").unwrap();
        let found = detect(home).unwrap().unwrap();
        // No prefix: only Lutris knows where it runs.
        assert!(found.games[0].own.is_none());
        let hk = found.games[2].own.clone().unwrap();
        assert_eq!((hk.program.as_str(), hk.wine), ("/g/hk.exe", None));
    }

    #[test]
    fn nothing_without_a_library() {
        let h = tempfile::tempdir().unwrap();
        assert!(detect(h.path()).unwrap().is_none());
    }
}
