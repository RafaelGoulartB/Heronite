//! The parts of the app each system has.
//!
//! Linux has everything. On Windows the port is under way
//! (`docs/WINDOWS-MIGRATION.md`): a part turns on here once it works there.
//! The UI asks [`supported`] before it shows a page, a menu entry or a
//! setting, and services that would only fail do not start.

/// A part of the app that not every system has.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Feature {
    /// Wine and Proton: the Wine Manager, Wine settings, prefixes, winetricks.
    Wine,
    /// Epic, GOG and Amazon Games through legendary, gogdl and nile, and the
    /// download queue.
    StoreAccounts,
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
    /// What Linux's launch adds around a game: wrappers (GameMode,
    /// MangoHud, gamescope), scripts before and after it, programs alongside
    /// it, launch presets, the Steam runtime, and the Performance and GPU
    /// settings built on them.
    LaunchExtras,
    /// Settings → Storage: what games, Wine prefixes, Proton builds and
    /// caches take on disk, and cleaning them up.
    Storage,
    /// The icon in the system tray.
    Tray,
    /// Updating the app from inside it.
    SelfUpdate,
    /// Starting with the session.
    Autostart,
    /// Menu and desktop shortcuts of games.
    DesktopShortcuts,
    /// Taking screenshots of a running game.
    Capture,
}

/// Whether this system has `feature`.
pub const fn supported(feature: Feature) -> bool {
    if cfg!(target_os = "linux") {
        return true;
    }
    match feature {
        // Ported to Windows.
        Feature::Autostart | Feature::Tray | Feature::Emulation => true,
        Feature::Wine
        | Feature::StoreAccounts
        | Feature::StoreClients
        | Feature::LauncherImports
        | Feature::HeroicMigration
        | Feature::SaveBackups
        | Feature::Overlay
        | Feature::SteamGameSettings
        | Feature::Sandbox
        | Feature::LaunchExtras
        | Feature::Storage
        | Feature::SelfUpdate
        | Feature::DesktopShortcuts
        | Feature::Capture => false,
    }
}
