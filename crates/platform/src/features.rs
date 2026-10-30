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
    /// Emulators, ROM folders and Ludusavi.
    Emulation,
    /// The in-game overlay and the replay buffer.
    Overlay,
    /// Steam's per-game settings (launch options, compatibility tool) and its
    /// library read from the running client.
    SteamClient,
    /// Running games in a sandbox (bubblewrap).
    Sandbox,
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
        Feature::Wine
        | Feature::StoreAccounts
        | Feature::StoreClients
        | Feature::LauncherImports
        | Feature::Emulation
        | Feature::Overlay
        | Feature::SteamClient
        | Feature::Sandbox
        | Feature::Tray
        | Feature::SelfUpdate
        | Feature::Autostart
        | Feature::DesktopShortcuts
        | Feature::Capture => false,
    }
}
