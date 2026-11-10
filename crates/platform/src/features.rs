//! The parts of the app each system has.
//!
//! Linux has everything but Windows' own store (Xbox) and the first-start
//! copy of Heroic ([`Feature::HeroicMigration`], off on every system),
//! except what a Flatpak sandbox cannot do ([`crate::flatpak`]). On Windows the port is
//! under way (`docs/WINDOWS-MIGRATION.md`): a part turns on here once it
//! works there.
//! The UI asks [`supported`] before it shows a page, a menu entry or a
//! setting, and services that would only fail do not start.
//!
//! Debug builds read `HERONITE_FEATURES` once, to try a part before it is
//! turned on for good: `HERONITE_FEATURES=+epic,+gog` turns Epic and GOG on,
//! `-capture` turns screenshots off (names: [`Feature::name`]). Release
//! builds ignore it.

use std::sync::OnceLock;

/// A part of the app that not every system has.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Feature {
    /// Wine and Proton: the Wine Manager, Wine settings, prefixes, winetricks.
    Wine,
    /// Epic Games through legendary: its page, login, library, installs,
    /// cloud saves and the EOS overlay.
    EpicStore,
    /// GOG through gogdl: its page, login, library, installs and cloud saves.
    GogStore,
    /// Amazon Games through nile: its page, login, library and installs.
    AmazonStore,
    /// Battle.net, the EA app, Ubisoft Connect and the Rockstar Games
    /// Launcher (under Wine on Linux, as they are on Windows).
    StoreClients,
    /// Games of the other launchers: Heroic, Lutris, Bottles and Faugus on
    /// Linux, Heroic and Playnite on Windows.
    LauncherImports,
    /// Copying an existing Heroic setup in at the first start
    /// (`heronite_data::migrate`): `~/.config/heroic` on Linux,
    /// `%APPDATA%\heroic` on Windows. Off on every system: Heroic's games
    /// come in only when the user turns its import on in Settings → Other
    /// launchers ([`Feature::LauncherImports`]).
    HeroicMigration,
    /// Emulators, ROM folders and save states.
    Emulation,
    /// Save backups through Ludusavi: after play, by hand and restoring
    /// them (into Wine prefixes on Linux).
    SaveBackups,
    /// The in-game overlay.
    Overlay,
    /// Keeping and saving the last seconds of a game (Linux's replay buffer).
    Replay,
    /// Steam's per-game settings: the launch options line and, on Linux,
    /// the compatibility tool (Proton) forced for a game. Windows has the
    /// line alone: Steam starts Windows games as they are.
    SteamGameSettings,
    /// Running games in a sandbox (bubblewrap).
    Sandbox,
    /// What the launch adds around a game: scripts before and after it,
    /// programs alongside it and launch presets.
    LaunchExtras,
    /// What Linux's launch wraps a game in: wrapper commands, GameMode,
    /// MangoHud, gamescope, the Steam runtime, and the Performance and GPU
    /// settings built on them.
    Wrappers,
    /// Settings → Storage: what games, Wine prefixes, Proton builds and
    /// caches take on disk, and cleaning them up (on Windows: games, store
    /// tools, caches and old downloads).
    Storage,
    /// The icon in the system tray.
    Tray,
    /// Updating the app from inside it.
    SelfUpdate,
    /// Starting with the session.
    Autostart,
    /// Shortcuts of games: on the Desktop and in the menu of programs
    /// (`.desktop` files, `.lnk` files in the Start menu on Windows), and in
    /// Steam as non-Steam games.
    DesktopShortcuts,
    /// Taking screenshots of a running game.
    Capture,
    /// The Microsoft Store and PC Game Pass games the Xbox app installs
    /// (Windows' own packages): its page and the games it brings in.
    XboxStore,
    /// The mods of a game (`docs/MODS.md`): its manager, Settings → Mods,
    /// the mod sites' links and applying the mods when the game starts.
    Mods,
}

impl Feature {
    pub const ALL: [Feature; 23] = [
        Feature::Wine,
        Feature::EpicStore,
        Feature::GogStore,
        Feature::AmazonStore,
        Feature::StoreClients,
        Feature::LauncherImports,
        Feature::HeroicMigration,
        Feature::Emulation,
        Feature::SaveBackups,
        Feature::Overlay,
        Feature::Replay,
        Feature::SteamGameSettings,
        Feature::Sandbox,
        Feature::LaunchExtras,
        Feature::Wrappers,
        Feature::Storage,
        Feature::Tray,
        Feature::SelfUpdate,
        Feature::Autostart,
        Feature::DesktopShortcuts,
        Feature::Capture,
        Feature::XboxStore,
        Feature::Mods,
    ];

    /// Its name in `HERONITE_FEATURES`.
    pub const fn name(self) -> &'static str {
        match self {
            Feature::Wine => "wine",
            Feature::EpicStore => "epic",
            Feature::GogStore => "gog",
            Feature::AmazonStore => "amazon",
            Feature::StoreClients => "store-clients",
            Feature::LauncherImports => "launcher-imports",
            Feature::HeroicMigration => "heroic-migration",
            Feature::Emulation => "emulation",
            Feature::SaveBackups => "save-backups",
            Feature::Overlay => "overlay",
            Feature::Replay => "replay",
            Feature::SteamGameSettings => "steam-game-settings",
            Feature::Sandbox => "sandbox",
            Feature::LaunchExtras => "launch-extras",
            Feature::Wrappers => "wrappers",
            Feature::Storage => "storage",
            Feature::Tray => "tray",
            Feature::SelfUpdate => "self-update",
            Feature::Autostart => "autostart",
            Feature::DesktopShortcuts => "desktop-shortcuts",
            Feature::Capture => "capture",
            Feature::XboxStore => "xbox",
            Feature::Mods => "mods",
        }
    }
}

/// Whether this system has `feature` (in a debug build, after
/// `HERONITE_FEATURES`).
pub fn supported(feature: Feature) -> bool {
    if cfg!(debug_assertions) {
        if let Some(&(_, on)) = overrides().iter().find(|(f, _)| *f == feature) {
            return on;
        }
    }
    built_in(feature) && !(crate::flatpak::active() && off_in_flatpak(feature))
}

/// What the Flatpak's sandbox rules out: bubblewrap cannot make the
/// namespaces of a sandbox inside it, Flatpak installs the updates, and
/// nothing can be mounted over a game's folder (`fusermount3` needs to be
/// setuid, which a Flatpak never is), so mods cannot be applied.
const fn off_in_flatpak(feature: Feature) -> bool {
    matches!(feature, Feature::Sandbox | Feature::SelfUpdate | Feature::Mods)
}

/// Whether any of the stores Heronite signs in to is on: what they share
/// (the Downloads page, Settings → Downloads, the download queue) shows then.
pub fn any_store() -> bool {
    [Feature::EpicStore, Feature::GogStore, Feature::AmazonStore].into_iter().any(supported)
}

/// What this system has, without `HERONITE_FEATURES`.
const fn built_in(feature: Feature) -> bool {
    if cfg!(target_os = "linux") {
        // Packages of the Microsoft Store exist on Windows only.
        return !matches!(feature, Feature::XboxStore | Feature::HeroicMigration);
    }
    match feature {
        // Ported to Windows.
        Feature::Autostart
        | Feature::Tray
        | Feature::Emulation
        | Feature::SelfUpdate
        | Feature::DesktopShortcuts
        | Feature::SaveBackups
        | Feature::Capture
        | Feature::EpicStore
        | Feature::GogStore
        | Feature::AmazonStore
        | Feature::Storage
        | Feature::LaunchExtras
        | Feature::StoreClients
        | Feature::LauncherImports
        | Feature::SteamGameSettings
        | Feature::XboxStore => true,
        Feature::Overlay => cfg!(windows),
        // Mods on Windows wait until mod archives open without 7-Zip or
        // bsdtar on the PATH, and the game folder is put back reliably.
        Feature::Mods | Feature::Wine | Feature::HeroicMigration | Feature::Replay | Feature::Sandbox | Feature::Wrappers => false,
    }
}

/// `HERONITE_FEATURES`, read once.
fn overrides() -> &'static [(Feature, bool)] {
    static OVERRIDES: OnceLock<Vec<(Feature, bool)>> = OnceLock::new();
    OVERRIDES.get_or_init(|| {
        let Some(value) = std::env::var("HERONITE_FEATURES").ok().filter(|v| !v.trim().is_empty()) else { return Vec::new() };
        let (list, unknown) = parse_overrides(&value);
        // Read before the logger may start: on the console of the debug build.
        for name in unknown {
            eprintln!("HERONITE_FEATURES: no feature is called {name:?}");
        }
        for (feature, on) in &list {
            eprintln!("HERONITE_FEATURES: {} is {}", feature.name(), if *on { "on" } else { "off" });
        }
        list
    })
}

/// `+epic,-capture,gog` (a bare name turns it on; commas or spaces between
/// them): the features named, and the names that are none.
fn parse_overrides(value: &str) -> (Vec<(Feature, bool)>, Vec<String>) {
    let (mut list, mut unknown) = (Vec::new(), Vec::new());
    for item in value.split([',', ' ']).map(str::trim).filter(|s| !s.is_empty()) {
        let (on, name) = match item.strip_prefix('-') {
            Some(name) => (false, name),
            None => (true, item.strip_prefix('+').unwrap_or(item)),
        };
        match Feature::ALL.into_iter().find(|f| f.name().eq_ignore_ascii_case(name)) {
            Some(feature) => {
                list.retain(|(f, _)| *f != feature);
                list.push((feature, on));
            }
            None => unknown.push(name.to_string()),
        }
    }
    (list, unknown)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overrides_name_features_on_and_off() {
        let (list, unknown) = parse_overrides("+epic, -capture gog,,+EPIC, nope");
        assert_eq!(list, [(Feature::Capture, false), (Feature::GogStore, true), (Feature::EpicStore, true)]);
        assert_eq!(unknown, ["nope"]);
        assert_eq!(parse_overrides("-epic,+epic").0, [(Feature::EpicStore, true)], "the last one wins");
        assert!(parse_overrides("").0.is_empty());
    }

    #[test]
    fn every_feature_has_its_own_name() {
        let mut names: Vec<&str> = Feature::ALL.iter().map(|f| f.name()).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), Feature::ALL.len());
    }

    #[test]
    fn overlay_and_replay_are_independent_capabilities() {
        assert_eq!(built_in(Feature::Overlay), cfg!(any(target_os = "linux", windows)));
        assert_eq!(built_in(Feature::Replay), cfg!(target_os = "linux"));
        assert_eq!(parse_overrides("+overlay,-replay").0, [(Feature::Overlay, true), (Feature::Replay, false)]);
        assert_eq!(parse_overrides("-overlay,+replay").0, [(Feature::Overlay, false), (Feature::Replay, true)]);
    }

    #[test]
    fn linux_has_everything_but_windows_own_store_and_the_heroic_copy() {
        if cfg!(target_os = "linux") {
            assert!(Feature::ALL.into_iter().filter(|f| !matches!(f, Feature::XboxStore | Feature::HeroicMigration)).all(built_in));
            assert!(!built_in(Feature::XboxStore));
        }
        if cfg!(windows) {
            assert!(built_in(Feature::XboxStore));
            assert!(built_in(Feature::SteamGameSettings), "the launch options line");
            assert!(!built_in(Feature::Mods), "the mod manager is Linux only for now");
        }
    }

    /// Heroic's games come in only from Settings → Other launchers.
    #[test]
    fn heroic_is_never_copied_at_the_first_start() {
        assert!(!built_in(Feature::HeroicMigration));
    }
}
