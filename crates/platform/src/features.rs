//! The parts of the app each system has.
//!
//! Linux has everything, except what a Flatpak sandbox cannot do
//! ([`crate::flatpak`]). On Windows the port is under way
//! (`docs/WINDOWS-MIGRATION.md`): a part turns on here once it works there.
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
    /// Launcher under Wine.
    StoreClients,
    /// Games of Heroic, Lutris, Bottles and Faugus.
    LauncherImports,
    /// Copying an existing Heroic setup in at the first start
    /// (`heronite_data::migrate`): Heroic's Windows folder holds stores
    /// this system has no runner for yet.
    HeroicMigration,
    /// Emulators, ROM folders and save states.
    Emulation,
    /// Save backups through Ludusavi: after play, by hand and restoring
    /// them (into Wine prefixes on Linux).
    SaveBackups,
    /// The in-game overlay and the replay buffer.
    Overlay,
    /// Steam's per-game settings: the launch options line and the
    /// compatibility tool (Proton) forced for a game.
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
}

impl Feature {
    pub const ALL: [Feature; 20] = [
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
/// namespaces of a sandbox inside it, and Flatpak installs the updates.
const fn off_in_flatpak(feature: Feature) -> bool {
    matches!(feature, Feature::Sandbox | Feature::SelfUpdate)
}

/// Whether any of the stores Heronite signs in to is on: what they share
/// (the Downloads page, Settings → Downloads, the download queue) shows then.
pub fn any_store() -> bool {
    [Feature::EpicStore, Feature::GogStore, Feature::AmazonStore].into_iter().any(supported)
}

/// What this system has, without `HERONITE_FEATURES`.
const fn built_in(feature: Feature) -> bool {
    if cfg!(target_os = "linux") {
        return true;
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
        | Feature::LaunchExtras => true,
        Feature::Wine
        | Feature::StoreClients
        | Feature::LauncherImports
        | Feature::HeroicMigration
        | Feature::Overlay
        | Feature::SteamGameSettings
        | Feature::Sandbox
        | Feature::Wrappers => false,
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
    fn linux_has_everything() {
        if cfg!(target_os = "linux") {
            assert!(Feature::ALL.into_iter().all(built_in));
        }
    }
}
