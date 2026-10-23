//! Faugus Launcher: its games are `games.json` in its data folder (older
//! versions kept it in the config folder), covers `covers/<gameid>.png`.
//! A game starts with `faugus-launcher --game <gameid>`, which runs it
//! through umu and returns when it exits. Play time is in seconds and the
//! last played date is a local time without a zone.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use heronite_data::launchers::Launcher;
use serde_json::Value;

use crate::{Found, Game};

const FLATPAK_ID: &str = "io.github.Faugus.faugus-launcher";

pub fn detect(home: &Path) -> Result<Option<Found>> {
    let flatpak = home.join(".var/app").join(FLATPAK_ID);
    let candidates = [
        (crate::xdg(home, "XDG_DATA_HOME", ".local/share").join("faugus-launcher"), false),
        (crate::xdg(home, "XDG_CONFIG_HOME", ".config").join("faugus-launcher"), false),
        (flatpak.join("data/faugus-launcher"), true),
        (flatpak.join("config/faugus-launcher"), true),
    ];
    let Some((dir, flatpak)) = candidates.into_iter().find(|(d, _)| d.join("games.json").is_file()) else { return Ok(None) };
    let file = dir.join("games.json");
    let text = std::fs::read_to_string(&file).with_context(|| format!("reading {}", file.display()))?;
    let list: Value = serde_json::from_str(&text).with_context(|| format!("{} is not JSON", file.display()))?;
    // Covers live in the data folder even when `games.json` is in the config one.
    let data = if flatpak { home.join(".var/app").join(FLATPAK_ID).join("data/faugus-launcher") } else { dir.clone() };
    let games = list.as_array().map(|l| l.iter().filter_map(|g| game(g, &data, flatpak, home)).collect()).unwrap_or_default();
    Ok(Some(Found { launcher: Launcher::Faugus, location: dir, flatpak, games, heroic: None }))
}

fn command(flatpak: bool, id: &str) -> String {
    if flatpak {
        format!("flatpak run --command=/app/bin/faugus-launcher {FLATPAK_ID} --game {id}")
    } else {
        format!("faugus-launcher --game {id}")
    }
}

/// `~` at the start of a path, as Faugus writes some.
fn expand(path: &str, home: &Path) -> String {
    match path.strip_prefix("~/") {
        Some(rest) => home.join(rest).to_string_lossy().into_owned(),
        None => path.to_string(),
    }
}

/// Faugus' `last_played` (`2026-01-02T03:04:05.123456`, local time).
fn last_played(value: &str) -> Option<String> {
    use chrono::{Local, NaiveDateTime, SecondsFormat, TimeZone, Utc};
    let naive = NaiveDateTime::parse_from_str(value, "%Y-%m-%dT%H:%M:%S%.f").ok()?;
    let local = Local.from_local_datetime(&naive).earliest()?;
    Some(local.with_timezone(&Utc).to_rfc3339_opts(SecondsFormat::Millis, true))
}

/// One entry of `games.json`, unless it is hidden in Faugus, a Steam game
/// (the Steam import brings those) or has no id or title.
fn game(g: &Value, data: &Path, flatpak: bool, home: &Path) -> Option<Game> {
    let text = |k: &str| g.get(k).and_then(Value::as_str).map(str::trim).unwrap_or_default().to_string();
    let id = text("gameid");
    let title = text("title");
    let runner = text("runner");
    let hidden = g.get("hidden").and_then(Value::as_bool).unwrap_or(false);
    // Faugus starts the game by its id: only one it can name.
    let usable_id = !id.is_empty() && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_');
    if !usable_id || title.is_empty() || hidden || runner == "Steam" {
        return None;
    }
    let path = expand(&text("path"), home);
    let folder = Path::new(&path).parent().map(|p| p.to_string_lossy().into_owned()).filter(|p| !p.is_empty() && p != "/");
    let file = |p: PathBuf| p.is_file().then(|| p.to_string_lossy().into_owned());
    let cover = Some(text("cover"))
        .filter(|c| !c.is_empty())
        .and_then(|c| file(PathBuf::from(expand(&c, home))))
        .or_else(|| file(data.join("covers").join(format!("{id}.png"))));
    let seconds = g.get("playtime").and_then(|v| v.as_f64().or_else(|| v.as_str()?.parse().ok())).unwrap_or(0.0);
    let windows = runner != "Linux-Native";
    Some(Game {
        command: command(flatpak, &id),
        title,
        folder,
        windows,
        minutes: (seconds.max(0.0) / 60.0).round() as u64,
        last_played: last_played(&text("last_played")),
        cover,
        banner: file(data.join("banners").join(format!("{id}.png"))),
        prefix: Some(expand(&text("prefix"), home)).filter(|p| windows && !p.is_empty()),
        id,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_its_games_but_hidden_and_steam_ones() {
        let h = tempfile::tempdir().unwrap();
        let home = h.path();
        let data = home.join(".local/share/faugus-launcher");
        std::fs::create_dir_all(data.join("covers")).unwrap();
        std::fs::write(data.join("covers/hollow-knight.png"), b"x").unwrap();
        std::fs::write(
            data.join("games.json"),
            r#"[
              {"gameid":"hollow-knight","title":"Hollow Knight","path":"~/Games/HK/hollow_knight.exe","prefix":"~/Faugus/hollow-knight",
               "runner":"GE-Proton","playtime":5430,"last_played":"2026-01-02T03:04:05.123456","hidden":false},
              {"gameid":"tux","title":"SuperTux","path":"/opt/tux/supertux","runner":"Linux-Native","playtime":0,"prefix":"/x"},
              {"gameid":"secret","title":"Secret","path":"/g/s.exe","hidden":true},
              {"gameid":"620","title":"Portal 2","path":"620","runner":"Steam"},
              {"gameid":"bad id","title":"Odd","path":"/g/o.exe"}
            ]"#,
        )
        .unwrap();
        let found = detect(home).unwrap().unwrap();
        assert!(!found.flatpak);
        let names: Vec<_> = found.games.iter().map(|g| g.title.as_str()).collect();
        assert_eq!(names, ["Hollow Knight", "SuperTux"]);
        let hk = &found.games[0];
        assert_eq!(hk.command, "faugus-launcher --game hollow-knight");
        assert_eq!(hk.folder.as_deref(), Some(home.join("Games/HK").to_str().unwrap()));
        assert_eq!(hk.prefix.as_deref(), Some(home.join("Faugus/hollow-knight").to_str().unwrap()));
        assert_eq!(hk.minutes, 91);
        assert!(hk.windows && hk.last_played.is_some() && hk.cover.is_some());
        let tux = &found.games[1];
        assert!(!tux.windows);
        assert_eq!((tux.folder.as_deref(), tux.prefix.as_deref()), (Some("/opt/tux"), None));
    }
}
