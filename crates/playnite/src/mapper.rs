//! Playnite game -> Heronite overlay entry: port of `playnite/mapper.ts`.

use std::collections::HashMap;

use chrono::{DateTime, FixedOffset, Local, NaiveDate, NaiveDateTime, SecondsFormat, TimeZone, Utc};
use heronite_data::models::{GameInfo, InstallInfo, LocalGameMeta, LocalGameRom, LocalGameSession};
use heronite_data::JsNum;
use heronite_launch::probe::Probe;
use serde_json::Value;

use crate::reader::*;
use crate::remap::{expand_playnite_variables, path_exists, remap_windows_path, DriveRemap};
use crate::types::Destination;

#[derive(Debug, Clone)]
pub struct MappedPlayniteGame {
    pub meta: LocalGameMeta,
    pub game_info: Option<GameInfo>,
    pub sessions: Vec<LocalGameSession>,
    pub destination: Destination,
    pub playtime_minutes: f64,
    pub first_played: Option<String>,
    pub last_played: Option<String>,
}

/// `pluginSource`.
fn plugin_source(plugin_id: &str, is_emulator: bool) -> &'static str {
    if is_emulator {
        return "emulator";
    }
    match plugin_id {
        STEAM_PLUGIN_ID => "steam",
        EPIC_PLUGIN_ID => "epic",
        GOG_PLUGIN_ID => "gog",
        AMAZON_PLUGIN_ID => "amazon",
        XBOX_PLUGIN_ID => "xbox",
        EMPTY_GUID => "manual",
        _ => "other",
    }
}

/// `normalizeTitle`: lower-case, non `[a-z0-9]` runs collapsed to one space.
pub fn normalize_title(title: &str) -> String {
    let lowered = title.to_lowercase();
    lowered
        .split(|c: char| !c.is_ascii_lowercase() && !c.is_ascii_digit())
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

fn play_action(game: &PlayniteGame) -> Option<&PlayniteGameAction> {
    game.game_actions.iter().find(|a| a.is_play_action).or_else(|| game.game_actions.first())
}

fn is_emulated(game: &PlayniteGame) -> bool {
    !game.roms.is_empty() || game.game_actions.iter().any(|a| a.action_type == 2)
}

fn stable_app_name(game: &PlayniteGame, source: &str) -> String {
    if source == "steam" {
        if let Some(game_id) = game.game_id.as_deref().filter(|g| !g.is_empty()) {
            return format!("steam_{game_id}");
        }
    }
    format!("playnite_{}", game.id)
}

fn pick_platform(launch_kind: &str, executable: Option<&str>) -> &'static str {
    if launch_kind == "steam-uri" {
        return "linux";
    }
    if launch_kind == "unavailable" {
        return "Windows";
    }
    match executable {
        Some(e) if !e.is_empty() && !e.to_lowercase().ends_with(".exe") => "linux",
        _ => "Windows",
    }
}

#[derive(Default)]
struct EmulatorLaunch {
    executable: Option<String>,
    args: Option<String>,
    emulator_name: Option<String>,
    emulator_id: Option<String>,
    emulator_profile_id: Option<String>,
}

fn resolve_emulator_launch(game: &PlayniteGame, emulators: &[PlayniteEmulator], drive_map: &[DriveRemap]) -> EmulatorLaunch {
    let action = play_action(game);
    let emulator = action
        .and_then(|a| a.emulator_id.as_deref())
        .and_then(|id| emulators.iter().find(|e| e.id == id));
    let wanted_profile = action.and_then(|a| a.emulator_profile_id.as_deref());
    let profile = emulator.and_then(|e| {
        e.profiles.iter().find(|p| p.id.as_deref() == wanted_profile).or_else(|| e.profiles.first())
    });

    let install_dir = remap_windows_path(emulator.and_then(|e| e.install_dir.as_deref()), drive_map);
    let raw_exe = profile
        .and_then(|p| p.executable.clone().or_else(|| p.startup_executable.clone()))
        .or_else(|| emulator.and_then(|e| e.install_dir.clone()));
    let expanded_exe = expand_playnite_variables(
        raw_exe.as_deref(),
        &[("InstallDir", install_dir.as_deref()), ("EmulatorDir", install_dir.as_deref())],
    );
    let executable = remap_windows_path(expanded_exe.as_deref(), drive_map).or(expanded_exe);

    let raw_args = if action.is_some_and(|a| a.override_default_args) {
        action.and_then(|a| a.arguments.clone())
    } else {
        let base = profile
            .and_then(|p| p.arguments.clone().or_else(|| p.custom_arguments.clone()))
            .unwrap_or_else(|| "{ImagePath}".to_owned());
        let parts: Vec<String> = [Some(base), action.and_then(|a| a.additional_arguments.clone())]
            .into_iter()
            .flatten()
            .filter(|s| !s.is_empty())
            .collect();
        Some(parts.join(" "))
    };

    let game_install = remap_windows_path(game.install_directory.as_deref(), drive_map);
    let args = expand_playnite_variables(
        raw_args.as_deref(),
        &[
            ("InstallDir", game_install.as_deref()),
            ("EmulatorDir", install_dir.as_deref()),
            ("Name", Some(game.name.as_str())),
        ],
    );

    EmulatorLaunch {
        executable,
        args,
        emulator_name: emulator.and_then(|e| e.name.clone()),
        emulator_id: emulator.map(|e| e.id.clone()),
        emulator_profile_id: profile.and_then(|p| p.id.clone()),
    }
}

fn resolve_executable_launch(game: &PlayniteGame, drive_map: &[DriveRemap]) -> (Option<String>, Option<String>) {
    let action = play_action(game);
    let install_dir = remap_windows_path(game.install_directory.as_deref(), drive_map);
    let vars = [("InstallDir", install_dir.as_deref()), ("Name", Some(game.name.as_str()))];
    let raw_path = expand_playnite_variables(action.and_then(|a| a.path.as_deref()), &vars);
    let executable = remap_windows_path(raw_path.as_deref(), drive_map)
        .or_else(|| raw_path.as_ref().filter(|p| p.starts_with('/')).cloned());
    let args = expand_playnite_variables(action.and_then(|a| a.arguments.as_deref()), &vars);
    (executable, args)
}

/// `mapPlayniteGame`. `probe` answers the `existsSync` checks.
pub fn map_playnite_game(
    game: &PlayniteGame,
    emulators: &[PlayniteEmulator],
    sessions: Vec<LocalGameSession>,
    drive_map: &[DriveRemap],
    probe: &dyn Probe,
) -> MappedPlayniteGame {
    let emulated = is_emulated(game);
    let source = plugin_source(&game.plugin_id, emulated);
    let app_name = stable_app_name(game, source);
    let playtime_minutes = (game.playtime_seconds / 60.0).floor();
    let first_played = sessions
        .first()
        .map(|s| s.started_at.clone())
        .or_else(|| game.added.clone())
        .or_else(|| game.last_activity.clone());
    let last_played = game.last_activity.clone().or_else(|| sessions.last().map(|s| s.started_at.clone()));

    let mut destination = Destination::Sideload;
    let mut launch_kind = "executable";
    let mut executable: Option<String> = None;
    let mut launcher_args: Option<String> = None;
    let mut emulator = EmulatorLaunch::default();
    let has_game_id = game.game_id.as_deref().is_some_and(|g| !g.is_empty());

    if source == "epic" {
        destination = Destination::Legendary;
        launch_kind = "unavailable";
    } else if source == "gog" {
        destination = Destination::Gog;
        launch_kind = "unavailable";
    } else if source == "steam" && has_game_id {
        launch_kind = "steam-uri";
        launcher_args = Some(format!("steam://rungameid/{}", game.game_id.as_deref().unwrap_or_default()));
    } else if source == "xbox" {
        launch_kind = "unavailable";
    } else if emulated {
        launch_kind = "emulator";
        let resolved = resolve_emulator_launch(game, emulators, drive_map);
        executable = resolved.executable.clone();
        launcher_args = resolved.args.clone();
        emulator = resolved;
    } else {
        let (exe, args) = resolve_executable_launch(game, drive_map);
        executable = exe;
        launcher_args = args;
    }

    if (launch_kind == "executable" || launch_kind == "emulator") && !path_exists(probe, executable.as_deref()) {
        executable = None;
    }

    if launch_kind == "emulator" && !game.roms.is_empty() {
        let rom_on_disk = game
            .roms
            .iter()
            .any(|rom| path_exists(probe, remap_windows_path(Some(&rom.path), drive_map).as_deref()));
        if !rom_on_disk {
            executable = None;
        }
    }

    let platform = pick_platform(launch_kind, executable.as_deref());
    let is_installed = (launch_kind == "executable" || launch_kind == "emulator") && executable.is_some();

    let runner = match destination {
        Destination::Sideload => "sideload",
        other => other.as_str(),
    };

    let meta = LocalGameMeta {
        app_name: app_name.clone(),
        runner: runner.to_owned(),
        playnite_id: game.id.clone(),
        plugin_id: Some(game.plugin_id.clone()),
        source: source.to_owned(),
        launch_kind: launch_kind.to_owned(),
        steam_app_id: if source == "steam" { game.game_id.clone() } else { None },
        store_game_id: game.game_id.clone(),
        title: game.name.clone(),
        windows_install_directory: game.install_directory.clone(),
        windows_executable: play_action(game).and_then(|a| a.path.clone()),
        remapped_executable: executable.clone(),
        launcher_args,
        roms: Some(
            game.roms
                .iter()
                .map(|rom| LocalGameRom {
                    name: rom.name.clone(),
                    path: remap_windows_path(Some(&rom.path), drive_map).unwrap_or_else(|| rom.path.clone()),
                    extra: Default::default(),
                })
                .filter(|rom| !rom.path.is_empty())
                .collect(),
        ),
        emulator_name: emulator.emulator_name,
        emulator_id: emulator.emulator_id,
        emulator_profile_id: emulator.emulator_profile_id,
        notes: game.notes.clone(),
        playnite_completion_status_id: game.completion_status_id.clone(),
        ..Default::default()
    };

    let game_info = (destination == Destination::Sideload).then(|| {
        let mut info = GameInfo {
            runner: "sideload".into(),
            app_name,
            title: game.name.clone(),
            art_cover: Some(String::new()),
            art_square: Some(String::new()),
            developer: game.developers.first().cloned(),
            description: game.notes.clone(),
            is_installed,
            is_linux_native: Some(platform == "linux"),
            folder_name: executable.as_deref().map(dirname_like_ts),
            install: InstallInfo {
                executable: Some(executable.clone().unwrap_or_default()),
                platform: Some(platform.to_owned()),
                is_dlc: Some(false),
                ..Default::default()
            },
            ..Default::default()
        };
        info.rest.insert("canRunOffline".into(), Value::Bool(launch_kind != "steam-uri"));
        info
    });

    MappedPlayniteGame {
        meta,
        game_info,
        sessions,
        destination,
        playtime_minutes,
        first_played,
        last_played,
    }
}

/// `executable.replace(/\/[^/]+$/, '')`.
fn dirname_like_ts(path: &str) -> String {
    match path.rfind('/') {
        Some(i) if i + 1 < path.len() => path[..i].to_owned(),
        _ => path.to_owned(),
    }
}

// ---------------------------------------------------------------------------
// Sessions
// ---------------------------------------------------------------------------

/// `new Date(text)` for the formats GameActivity writes: RFC 3339 with any
/// offset, or a date-time without offset (local time, like JS), or a bare
/// date (UTC). Milliseconds since the epoch.
pub fn parse_js_date_ms(text: &str) -> Option<i64> {
    let text = text.trim();
    if let Ok(dt) = DateTime::<FixedOffset>::parse_from_rfc3339(text) {
        return Some(dt.timestamp_millis());
    }
    for fmt in ["%Y-%m-%dT%H:%M:%S%.f", "%Y-%m-%d %H:%M:%S%.f", "%Y-%m-%dT%H:%M", "%Y-%m-%d %H:%M"] {
        if let Ok(naive) = NaiveDateTime::parse_from_str(text, fmt) {
            return Local.from_local_datetime(&naive).earliest().map(|d| d.timestamp_millis());
        }
    }
    // "2024-01-05 10:00:00Z" / "+0100" style offsets
    for fmt in ["%Y-%m-%d %H:%M:%S%.f%#z", "%Y-%m-%dT%H:%M:%S%.f%#z", "%Y-%m-%d %H:%M:%S%.f%:z"] {
        if let Ok(dt) = DateTime::parse_from_str(text, fmt) {
            return Some(dt.timestamp_millis());
        }
    }
    if let Ok(date) = NaiveDate::parse_from_str(text, "%Y-%m-%d") {
        return Some(date.and_hms_opt(0, 0, 0)?.and_utc().timestamp_millis());
    }
    None
}

pub fn iso_from_ms(ms: i64) -> Option<String> {
    DateTime::<Utc>::from_timestamp_millis(ms).map(|d| d.to_rfc3339_opts(SecondsFormat::Millis, true))
}

/// `playniteSessionsToLocal`: sessions of `playnite_id`, ascending by start.
/// Items with an unparsable date are dropped.
pub fn playnite_sessions_to_local(
    playnite_id: &str,
    dump_sessions: &HashMap<String, Vec<PlayniteActivitySession>>,
) -> Vec<LocalGameSession> {
    let Some(items) = dump_sessions.get(playnite_id) else { return Vec::new() };
    let mut out: Vec<LocalGameSession> = items
        .iter()
        .filter_map(|item| {
            let started = parse_js_date_ms(&item.date_session)?;
            let ended = (started as f64 + item.elapsed_seconds * 1000.0).trunc() as i64;
            Some(LocalGameSession {
                started_at: iso_from_ms(started)?,
                ended_at: iso_from_ms(ended)?,
                elapsed_seconds: JsNum(item.elapsed_seconds),
                source: "playnite".into(),
                game_action_name: item.game_action_name.clone(),
                extra: Default::default(),
            })
        })
        .collect();
    out.sort_by(|a, b| a.started_at.cmp(&b.started_at));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use heronite_launch::probe::FakeProbe;

    fn game(plugin: &str, id: &str) -> PlayniteGame {
        PlayniteGame {
            id: "11111111-2222-3333-4444-555555555555".into(),
            name: "Game".into(),
            game_id: Some(id.into()),
            plugin_id: plugin.into(),
            playtime_seconds: 3725.0,
            ..Default::default()
        }
    }

    #[test]
    fn normalizes_titles() {
        assert_eq!(normalize_title("The Witcher® 3: Wild Hunt!"), "the witcher 3 wild hunt");
        assert_eq!(normalize_title("日本語"), "");
    }

    #[test]
    fn steam_epic_gog_destinations() {
        let probe = FakeProbe::new("/h");
        let steam = map_playnite_game(&game(STEAM_PLUGIN_ID, "440"), &[], vec![], &[], &probe);
        assert_eq!(steam.meta.app_name, "steam_440");
        assert_eq!(steam.meta.launch_kind, "steam-uri");
        assert_eq!(steam.meta.launcher_args.as_deref(), Some("steam://rungameid/440"));
        assert_eq!(steam.meta.steam_app_id.as_deref(), Some("440"));
        assert_eq!(steam.destination, Destination::Sideload);
        assert_eq!(steam.playtime_minutes, 62.0);
        let info = steam.game_info.unwrap();
        assert!(info.is_linux_native == Some(true));
        assert_eq!(info.rest["canRunOffline"], false);

        let epic = map_playnite_game(&game(EPIC_PLUGIN_ID, "abc"), &[], vec![], &[], &probe);
        assert_eq!(epic.destination, Destination::Legendary);
        assert_eq!(epic.meta.runner, "legendary");
        assert_eq!(epic.meta.launch_kind, "unavailable");
        assert!(epic.game_info.is_none());
        assert_eq!(epic.meta.app_name, "playnite_11111111-2222-3333-4444-555555555555");

        let gog = map_playnite_game(&game(GOG_PLUGIN_ID, "1"), &[], vec![], &[], &probe);
        assert_eq!(gog.destination, Destination::Gog);
    }

    #[test]
    fn executable_games_need_the_file_on_disk() {
        let mut g = game(EMPTY_GUID, "");
        g.game_id = None;
        g.install_directory = Some("D:\\Games\\X".into());
        g.game_actions = vec![PlayniteGameAction {
            action_type: 0,
            is_play_action: true,
            path: Some("{InstallDir}/x.exe".into()),
            arguments: Some("-w {Name}".into()),
            ..Default::default()
        }];
        let map = [DriveRemap { windows_root: "D:\\".into(), linux_path: "/mnt/d".into() }];
        let missing = map_playnite_game(&g, &[], vec![], &map, &FakeProbe::new("/h"));
        assert_eq!(missing.meta.source, "manual");
        assert!(missing.meta.remapped_executable.is_none());
        assert!(!missing.game_info.as_ref().unwrap().is_installed);
        assert_eq!(missing.meta.launcher_args.as_deref(), Some("-w Game"));
        assert_eq!(missing.meta.windows_executable.as_deref(), Some("{InstallDir}/x.exe"));

        let probe = FakeProbe::new("/h").with_path("/mnt/d/Games/X/x.exe");
        let found = map_playnite_game(&g, &[], vec![], &map, &probe);
        assert_eq!(found.meta.remapped_executable.as_deref(), Some("/mnt/d/Games/X/x.exe"));
        let info = found.game_info.unwrap();
        assert!(info.is_installed);
        assert_eq!(info.install.platform.as_deref(), Some("Windows"));
        assert_eq!(info.folder_name.as_deref(), Some("/mnt/d/Games/X"));
    }

    #[test]
    fn emulator_games_resolve_profile_and_roms() {
        let mut g = game(EMPTY_GUID, "");
        g.game_id = None;
        g.roms = vec![PlayniteRom { name: Some("r".into()), path: "E:\\roms\\a.iso".into() }];
        g.game_actions = vec![PlayniteGameAction {
            action_type: 2,
            is_play_action: true,
            emulator_id: Some("emu-1".into()),
            emulator_profile_id: Some("p2".into()),
            additional_arguments: Some("--fs".into()),
            ..Default::default()
        }];
        let emu = PlayniteEmulator {
            id: "emu-1".into(),
            name: Some("PCSX2".into()),
            install_dir: Some("E:\\emu".into()),
            profiles: vec![
                PlayniteEmulatorProfile { id: Some("p1".into()), executable: Some("{EmulatorDir}/a.exe".into()), ..Default::default() },
                PlayniteEmulatorProfile {
                    id: Some("p2".into()),
                    executable: Some("{EmulatorDir}/pcsx2.exe".into()),
                    arguments: Some("-batch {ImagePath}".into()),
                    ..Default::default()
                },
            ],
        };
        let map = [DriveRemap { windows_root: "E:".into(), linux_path: "/mnt/e".into() }];
        let probe = FakeProbe::new("/h").with_path("/mnt/e/emu/pcsx2.exe").with_path("/mnt/e/roms/a.iso");
        let m = map_playnite_game(&g, &[emu.clone()], vec![], &map, &probe);
        assert_eq!(m.meta.source, "emulator");
        assert_eq!(m.meta.launch_kind, "emulator");
        assert_eq!(m.meta.emulator_profile_id.as_deref(), Some("p2"));
        assert_eq!(m.meta.launcher_args.as_deref(), Some("-batch {ImagePath} --fs"));
        assert_eq!(m.meta.remapped_executable.as_deref(), Some("/mnt/e/emu/pcsx2.exe"));
        assert_eq!(m.meta.roms.as_ref().unwrap()[0].path, "/mnt/e/roms/a.iso");
        assert!(m.game_info.unwrap().is_installed);

        // ROM missing: not installed even if the emulator exists.
        let probe = FakeProbe::new("/h").with_path("/mnt/e/emu/pcsx2.exe");
        let m = map_playnite_game(&g, &[emu], vec![], &map, &probe);
        assert!(m.meta.remapped_executable.is_none());
        assert!(!m.game_info.unwrap().is_installed);
    }

    #[test]
    fn sessions_are_sorted_and_use_iso_millis() {
        let mut map = HashMap::new();
        map.insert(
            "g".to_string(),
            vec![
                PlayniteActivitySession { date_session: "2024-03-02T10:00:00.1234567Z".into(), elapsed_seconds: 90.0, game_action_name: Some("Play".into()) },
                PlayniteActivitySession { date_session: "2024-03-01T23:59:59+02:00".into(), elapsed_seconds: 10.0, game_action_name: None },
                PlayniteActivitySession { date_session: "garbage".into(), elapsed_seconds: 10.0, game_action_name: None },
            ],
        );
        let s = playnite_sessions_to_local("g", &map);
        assert_eq!(s.len(), 2);
        assert_eq!(s[0].started_at, "2024-03-01T21:59:59.000Z");
        assert_eq!(s[0].ended_at, "2024-03-01T22:00:09.000Z");
        assert_eq!(s[1].started_at, "2024-03-02T10:00:00.123Z");
        assert_eq!(s[1].ended_at, "2024-03-02T10:01:30.123Z");
        assert_eq!(s[1].source, "playnite");
        assert!(playnite_sessions_to_local("other", &map).is_empty());
    }
}
