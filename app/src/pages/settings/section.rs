//! The sections of the Settings page: labels, navigation groups, palette
//! keywords and the `HERONITE_DEBUG_PAGE` names.

use crate::dialogs::settings_panel::GlobalView;
use crate::icons::Icon;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Section {
    General,
    Appearance,
    Advanced,
    GameDefaults,
    Downloads,
    Library,
    Metadata,
    Screenshots,
    Backups,
    Logs,
    About,
}

impl Section {
    /// Left navigation: caption + sections.
    pub const GROUPS: [(&'static str, &'static [Section]); 4] = [
        ("App", &[Section::General, Section::Appearance, Section::Advanced]),
        ("Games", &[Section::GameDefaults, Section::Downloads, Section::Library]),
        ("Collection", &[Section::Metadata, Section::Screenshots, Section::Backups]),
        ("Help", &[Section::Logs, Section::About]),
    ];

    /// Same order as [`Section::GROUPS`].
    pub const ALL: [Section; 11] = [
        Section::General,
        Section::Appearance,
        Section::Advanced,
        Section::GameDefaults,
        Section::Downloads,
        Section::Library,
        Section::Metadata,
        Section::Screenshots,
        Section::Backups,
        Section::Logs,
        Section::About,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Section::General => "General",
            Section::Appearance => "Appearance",
            Section::Advanced => "Advanced",
            Section::GameDefaults => "Game defaults",
            Section::Downloads => "Downloads",
            Section::Library => "Library",
            Section::Metadata => "Metadata",
            Section::Screenshots => "Screenshots",
            Section::Backups => "Backups",
            Section::Logs => "Logs",
            Section::About => "About",
        }
    }

    pub(super) fn icon(self) -> Icon {
        match self {
            Section::General => Icon::Settings,
            Section::Appearance => Icon::Visibility,
            Section::Advanced => Icon::Build,
            Section::GameDefaults => Icon::Tune,
            Section::Downloads => Icon::Download,
            Section::Library => Icon::GridView,
            Section::Metadata => Icon::TravelExplore,
            Section::Screenshots => Icon::Fullscreen,
            Section::Backups => Icon::Save,
            Section::Logs => Icon::Description,
            Section::About => Icon::Memory,
        }
    }

    /// Short description under the section title.
    pub(super) fn blurb(self) -> &'static str {
        match self {
            Section::General => "Folders, tray and shortcuts, Wine and Proton lookup. Saved automatically.",
            Section::Appearance => "Two independent looks: the artwork behind the Collection, and the background of menus, dialogs and pages (glass or solid).",
            Section::Advanced => "Logs, folders and cache.",
            Section::GameDefaults => "Values new games start with. Settings of an installed game are changed from its own Settings.",
            Section::Downloads => "Epic downloads and updates.",
            Section::Library => "What the Collection shows: hidden games, favourites and recent games.",
            Section::Metadata => "Collection uses IGDB first, then Steam, store pages and Lutris to fill covers, descriptions and tags.",
            Section::Screenshots => "Where your captures live, the in-game shortcut and the preview cache of the gallery.",
            Section::Backups => "Save backups with Ludusavi after you close a game, and a scheduled copy of the data folder.",
            Section::Logs => "The logs of this app, the runners and every game you launched. Switch logging off in General (Disable logs) or per game (Enable verbose logs).",
            Section::About => "Updates, this app's data folders and the system it runs on.",
        }
    }

    /// Extra words the command palette matches.
    pub fn keywords(self) -> &'static str {
        match self {
            Section::General => "folders tray shortcuts wine proton language theme",
            Section::Appearance => "background blur dim darkening saturation cover aspect grey uninstalled artwork glass opacity transparent transparency solid interface panels menus dialogs colour tint collection",
            Section::Advanced => "cache folders copy settings",
            Section::GameDefaults => "defaults wine prefix gamescope launch options new games",
            Section::Downloads => "epic updates install",
            Section::Library => "hidden favourites favorites recent",
            Section::Metadata => "igdb twitch steam lutris covers fill download images priority",
            Section::Screenshots => "capture shortcut hotkey folder thumbnails cache gallery",
            Section::Backups => "ludusavi saves backup config copy schedule zip compression",
            Section::Logs => "log viewer debug",
            Section::About => "version system info update upgrade new release changelog",
        }
    }

    pub(super) fn global_view(self) -> Option<GlobalView> {
        match self {
            Section::General => Some(GlobalView::General),
            Section::Downloads => Some(GlobalView::Downloads),
            Section::Advanced => Some(GlobalView::Advanced),
            _ => None,
        }
    }

    pub fn parse(s: &str) -> Option<Section> {
        match s {
            "general" => Some(Section::General),
            "appearance" => Some(Section::Appearance),
            "advanced" => Some(Section::Advanced),
            "defaults" => Some(Section::GameDefaults),
            "downloads" => Some(Section::Downloads),
            "library" => Some(Section::Library),
            "metadata" => Some(Section::Metadata),
            "screenshots" => Some(Section::Screenshots),
            "backups" | "backup" | "saves" => Some(Section::Backups),
            "logs" => Some(Section::Logs),
            "about" => Some(Section::About),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Section;

    #[test]
    fn sections_parse_for_debug_and_settings_page_is_a_debug_page() {
        for s in Section::ALL {
            assert!(!s.label().is_empty());
        }
        // The nav groups list every section once, in `ALL` order.
        let grouped: Vec<Section> = Section::GROUPS.iter().flat_map(|(_, s)| s.iter().copied()).collect();
        assert_eq!(grouped, Section::ALL);
        assert_eq!(Section::parse("backups"), Some(Section::Backups));
        assert_eq!(Section::parse("saves"), Some(Section::Backups));
        assert_eq!(Section::parse("metadata"), Some(Section::Metadata));
        assert_eq!(Section::parse("library"), Some(Section::Library));
        assert_eq!(Section::parse("defaults"), Some(Section::GameDefaults));
        assert_eq!(Section::parse("nope"), None);
        assert_eq!(crate::pages::Page::from_debug("settings"), Some(crate::pages::Page::Settings));
        assert_eq!(crate::pages::Page::from_debug("settings:about"), Some(crate::pages::Page::Settings));
        assert_eq!(crate::pages::Page::from_debug("settingsx"), None);
    }
}
