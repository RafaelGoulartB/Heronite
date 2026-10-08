//! Which local games count as installed (port of the pure parts of
//! `install-state.ts`). The caller applies the results to
//! `sideload_apps/library.json`.

use std::collections::HashSet;

use heronite_data::models::{GameInfo, InstallInfo, LocalGameMeta};

use crate::plan::{emulator_executable, PlanContext};
use crate::probe::Probe;
use crate::steam::{steam_cdn_covers, SteamInstalledGame};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallState {
    pub is_installed: bool,
    pub executable: String,
}

fn first_existing(probe: &dyn Probe, paths: &[Option<&str>]) -> String {
    paths
        .iter()
        .flatten()
        .find(|p| !p.is_empty() && probe.exists(p))
        .map(|p| p.to_string())
        .unwrap_or_default()
}

/// `emulatorRomInstalled`.
pub fn emulator_rom_installed(meta: &LocalGameMeta, probe: &dyn Probe) -> bool {
    match meta.roms.as_ref().filter(|r| !r.is_empty()) {
        None => meta
            .remapped_executable
            .as_deref()
            .map(|p| probe.exists(p))
            .unwrap_or(false),
        Some(roms) => roms.iter().any(|r| probe.exists(&r.path)),
    }
}

/// `nextInstallState`: what `is_installed` / `install.executable` should be.
pub fn next_install_state(
    ctx: &PlanContext,
    probe: &dyn Probe,
    game: &GameInfo,
    meta: &LocalGameMeta,
    installed_steam: &HashSet<String>,
    steam_bin: Option<&str>,
) -> InstallState {
    let local_exe = || {
        first_existing(
            probe,
            &[
                meta.remapped_executable.as_deref(),
                game.install.executable.as_deref(),
            ],
        )
    };

    if meta.launch_kind == "steam-uri" {
        let installed = meta
            .steam_app_id
            .as_deref()
            .map(|id| installed_steam.contains(id))
            .unwrap_or(false);
        return InstallState {
            is_installed: installed,
            executable: if installed {
                steam_bin.unwrap_or("steam").to_string()
            } else {
                String::new()
            },
        };
    }

    if meta.launch_kind == "emulator" {
        let executable = emulator_executable(ctx, meta).unwrap_or_else(local_exe);
        let roms_ok = emulator_rom_installed(meta, probe);
        let exe_ok = if meta
            .emulator_id
            .as_deref()
            .map(|s| !s.is_empty())
            .unwrap_or(false)
            && executable == "flatpak"
        {
            true
        } else {
            !executable.is_empty() && probe.exists(&executable)
        };
        return InstallState {
            is_installed: exe_ok && roms_ok,
            executable: if exe_ok { executable } else { String::new() },
        };
    }

    let executable = local_exe();
    if meta.launch_kind == "executable" || !executable.is_empty() {
        return InstallState {
            is_installed: !executable.is_empty(),
            executable,
        };
    }
    InstallState {
        is_installed: false,
        executable: String::new(),
    }
}

/// The sideload entry + Collection metadata created for a Steam game found
/// by scanning manifests (`steamSideloadEntry`). `not_played_status_id` is
/// the id of the "not played" completion status.
pub fn steam_sideload_entry(
    game: &SteamInstalledGame,
    steam_bin: Option<&str>,
    not_played_status_id: &str,
) -> (GameInfo, LocalGameMeta) {
    let app_name = format!("steam_{}", game.app_id);
    let (art_cover, art_square) = steam_cdn_covers(&game.app_id);
    let mut info = GameInfo {
        runner: "sideload".into(),
        app_name: app_name.clone(),
        title: game.name.clone(),
        art_cover: Some(art_cover),
        art_square: Some(art_square),
        is_installed: true,
        is_linux_native: Some(true),
        install: InstallInfo {
            executable: Some(steam_bin.unwrap_or("steam").to_string()),
            platform: Some("linux".into()),
            is_dlc: Some(false),
            ..Default::default()
        },
        ..Default::default()
    };
    info.rest
        .insert("canRunOffline".into(), serde_json::Value::Bool(false));
    let meta = LocalGameMeta {
        app_name,
        runner: "sideload".into(),
        playnite_id: format!("steam_{}", game.app_id),
        source: "steam".into(),
        launch_kind: "steam-uri".into(),
        steam_app_id: Some(game.app_id.clone()),
        store_game_id: Some(game.app_id.clone()),
        title: game.name.clone(),
        launcher_args: Some(format!("steam://rungameid/{}", game.app_id)),
        completion_status_id: Some(not_played_status_id.to_string()),
        ..Default::default()
    };
    (info, meta)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::probe::FakeProbe;
    use heronite_data::models::LocalGameRom;
    use heronite_data::HeroicPaths;

    fn ctx() -> PlanContext {
        let mut c = PlanContext::load(&HeroicPaths::at("/nonexistent-heroic-root"));
        c.emulators.emulators.push(crate::emulators::UserEmulator {
            id: "e1".into(),
            definition_id: "custom".into(),
            executable: "/emu/pcsx2".into(),
            launch_kind: "native".into(),
            ..Default::default()
        });
        c
    }

    fn game(exe: &str) -> GameInfo {
        GameInfo {
            runner: "sideload".into(),
            app_name: "g".into(),
            install: InstallInfo {
                executable: Some(exe.into()),
                ..Default::default()
            },
            ..Default::default()
        }
    }

    #[test]
    fn steam_games_follow_manifests() {
        let meta = LocalGameMeta {
            launch_kind: "steam-uri".into(),
            steam_app_id: Some("400".into()),
            ..Default::default()
        };
        let probe = FakeProbe::new("/h");
        let installed: HashSet<String> = ["400".to_string()].into();
        let on = next_install_state(
            &ctx(),
            &probe,
            &game(""),
            &meta,
            &installed,
            Some("/usr/bin/steam"),
        );
        assert_eq!(
            on,
            InstallState {
                is_installed: true,
                executable: "/usr/bin/steam".into()
            }
        );
        let off = next_install_state(
            &ctx(),
            &probe,
            &game("/usr/bin/steam"),
            &meta,
            &HashSet::new(),
            None,
        );
        assert_eq!(
            off,
            InstallState {
                is_installed: false,
                executable: String::new()
            }
        );
    }

    #[test]
    fn emulator_games_need_emulator_and_rom() {
        let probe = FakeProbe::new("/h")
            .with_path("/emu/pcsx2")
            .with_path("/roms/a.iso");
        let mut meta = LocalGameMeta {
            launch_kind: "emulator".into(),
            emulator_id: Some("e1".into()),
            roms: Some(vec![LocalGameRom {
                name: None,
                path: "/roms/a.iso".into(),
                extra: Default::default(),
            }]),
            ..Default::default()
        };
        let s = next_install_state(&ctx(), &probe, &game(""), &meta, &HashSet::new(), None);
        assert_eq!(
            s,
            InstallState {
                is_installed: true,
                executable: "/emu/pcsx2".into()
            }
        );
        meta.roms.as_mut().unwrap()[0].path = "/roms/missing.iso".into();
        let s = next_install_state(&ctx(), &probe, &game(""), &meta, &HashSet::new(), None);
        assert!(!s.is_installed);
        assert_eq!(
            s.executable, "/emu/pcsx2",
            "emulator itself is still present"
        );
    }

    #[test]
    fn plain_executables_prefer_remapped_path() {
        let probe = FakeProbe::new("/h")
            .with_path("/new/game")
            .with_path("/old/game");
        let meta = LocalGameMeta {
            launch_kind: "executable".into(),
            remapped_executable: Some("/new/game".into()),
            ..Default::default()
        };
        assert_eq!(
            next_install_state(
                &ctx(),
                &probe,
                &game("/old/game"),
                &meta,
                &HashSet::new(),
                None
            )
            .executable,
            "/new/game"
        );
        let gone = LocalGameMeta {
            launch_kind: "executable".into(),
            ..Default::default()
        };
        assert!(
            !next_install_state(&ctx(), &probe, &game("/nope"), &gone, &HashSet::new(), None)
                .is_installed
        );
    }

    #[test]
    fn new_steam_entries_match_heronite_shape() {
        let (info, meta) = steam_sideload_entry(
            &SteamInstalledGame {
                app_id: "400".into(),
                name: "Portal".into(),
            },
            None,
            "not-played",
        );
        assert_eq!(info.app_name, "steam_400");
        assert_eq!(info.install.executable.as_deref(), Some("steam"));
        assert_eq!(info.rest["canRunOffline"], false);
        assert_eq!(meta.launcher_args.as_deref(), Some("steam://rungameid/400"));
        assert_eq!(meta.launch_kind, "steam-uri");
    }
}
