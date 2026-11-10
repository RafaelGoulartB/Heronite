//! The search above the Settings sections: it finds sections (label, group
//! caption, palette keywords, blurb) and the settings inside them (the labels
//! of their rows, fields and captions, listed in [`INDEX`] and
//! [`DEFAULTS_INDEX`], plus the metadata sources and their options from the
//! source registry). Picking a setting opens its section and flashes it
//! (`ui::find`).
//!
//! The labels are the English texts the sections draw (`tr!` keys): a renamed
//! label must be renamed here too (`every_indexed_label_is_still_drawn`).
//! The overlay's chord labels are this system's (`overlay::labels`).
//! `HERONITE_DEBUG_FIND_LABELS=1` logs what a section draws.

use heronite_data::fuzzy;
use heronite_i18n::tr;
use heronite_metadata::sources::builtin_descriptors;

use super::Section;
use crate::dialogs::settings_panel::logic::Section as Tab;
use crate::overlay::labels;

/// Settings listed under a section in the results, at most.
const SHOWN: usize = 3;

/// The settings of each section (Game defaults: [`DEFAULTS_INDEX`]).
const INDEX: &[(Section, &[&str])] = &[
    (
        Section::General,
        &[
            "Language",
            "Interface language",
            "Folders",
            "Default install folder",
            "Folder for new Wine prefixes",
            "Steam folder",
            "Startup",
            "Start with the system",
            "Start minimized",
            "System tray",
            "Keep in the tray when closed",
            "Dark tray icon",
            "Hide the tray icon",
            "When a game starts and ends",
            "Minimize when a game starts",
            "Stay hidden when a shortcut starts a game",
            "Close Steam when the game ends",
            "Game shortcuts",
            "Add desktop shortcuts automatically",
            "Add games to start menu automatically",
            "Add games to Steam automatically",
            "Wine and Proton",
            "Offer Valve's Proton builds",
            "Download GE-Proton into Steam",
            "Other Wine and Proton builds",
        ],
    ),
    (
        Section::Appearance,
        &[
            "Profiles",
            "Colour",
            "Theme",
            "Panels",
            "Accent colour",
            "High contrast",
            "Behind the Collection",
            "Use the game artwork as page background",
            "Picture",
            "Covers",
            "Cover aspect",
            "Show uninstalled games in grey",
            "Corners",
            "Under the cover",
            "Favourite star",
            "Store icon",
            "Show every store a game is in",
            "Hide while grouped by store",
            "Under the pointer",
            "Categories on covers",
            "Show categories on covers",
            "Text and motion",
            "Zoom",
            "Follow the system",
            "Font",
            "Density",
            "Animations",
            "Layout",
            "Quick toolbar controls",
            "Open the Collection in",
            "Switch with the window's width",
            "Panel widths",
            "Game sections",
        ],
    ),
    (
        Section::ConsoleMode,
        &["The theme in use", "Startup", "Open the app", "Open console mode in fullscreen", "When a controller connects"],
    ),
    (
        Section::Downloads,
        &[
            "Download workers",
            "Legendary binary",
            "gogdl binary",
            "nile binary",
            "Updates",
            "Download games without HTTPS",
            "Check for game updates at startup",
            "Automatically update games",
        ],
    ),
    (Section::Launchers, if cfg!(windows) { &["Heroic", "Playnite"] } else { &["Heroic", "Lutris", "Bottles", "Faugus"] }),
    (
        Section::Integrations,
        &[
            "Keep my games in Sunshine",
            "Address of Sunshine's web page",
            "Games in Sunshine",
            "Quitting in Moonlight closes the game",
            "Add Heronite console mode",
        ],
    ),
    (Section::Library, &["Show hidden games", "Recent games", "Recent games to show", "Clear recent games"]),
    (
        Section::Metadata,
        &[
            "Game details",
            "IGDB",
            "Client ID",
            "Client secret",
            "Steam",
            "Store",
            "Lutris",
            "Pictures",
            "SteamGridDB",
            "SteamGridDB API key",
            "From the web",
            "Extras",
            "HowLongToBeat",
            "Steam tags",
            "Steam tags per game",
            "ProtonDB",
            "Get more sources",
            "Cover shapes",
            "Download a shape as soon as a view starts using it",
            "When details arrive",
            "Fill missing metadata in the background",
            "Copy covers to disk",
            "Allow adult covers",
            "Show age ratings",
            "Rating board shown first",
            "Order for one field (advanced)",
        ],
    ),
    (
        Section::Achievements,
        &[
            "Turn on achievements",
            "RetroAchievements",
            "Sign the emulators in too",
            "Hardcore mode in the emulators",
            "Web API key",
            "Steam",
            "Read from this PC's Steam",
            "Steam Web API key (optional)",
            "Check Steam games while they run",
            "GOG",
            "Unlock GOG achievements while you play",
            "How they show",
            "Show new achievements over the game",
            "Show the description",
            "Show how rare it is",
            "Show points or XP",
            "Show the achievement set",
            "Show the RetroAchievements type",
            "Show hardcore unlocks",
            "Notify new achievements",
            "Try the notification",
            "Show achievements in the Timeline",
            "Hide secret achievements",
            "Update every game now",
        ],
    ),
    (
        Section::Overlay,
        &[
            "Turn on the overlay",
            "Open it without a game too",
            "Keyboard",
            "Open it with a keyboard shortcut",
            "Keyboard shortcut",
            "Shortcuts without opening it",
            labels::BUTTON_HELD,
            labels::VOLUME,
            "Volume step",
            labels::MOMENT,
            "Guide + X: save the replay",
            "Position",
            "Side of the screen",
            "Status",
            "Desktop",
            "Controllers",
            "Good to know",
        ],
    ),
    (
        Section::Screenshots,
        &[
            "Screenshots folder",
            "Space used",
            "In-game capture",
            "Capture screenshots while a game is running",
            "Screenshot shortcut",
            "With the controller",
            labels::BUTTON_HELD,
            "Lossless compression",
            "Compress screenshots without quality loss",
            "Preview cache",
            "Preview cache limit (MB)",
            "Clear preview cache",
        ],
    ),
    (
        Section::Replay,
        &[
            "Keep a replay of the game",
            "Length saved",
            "Saving it",
            "Guide + X: save the replay",
            "Keyboard shortcut",
            "Picture and sound",
            "Quality",
            "Frame rate",
            "Resolution",
            "Format",
            "Monitor",
            "Video encoder",
            "Encoder",
            "Buffer",
            "Room for the panel",
            "Sound",
            "Microphone too",
            "Saved in",
            "Status",
        ],
    ),
    (Section::Storage, &["Free up space"]),
    (
        Section::Saves,
        &[
            "Save backups (Ludusavi)",
            "Back up saves when a game closes",
            "Use installed Ludusavi settings",
            "Ludusavi binary",
            "Save backup folder",
            "Backup format",
            "Zip compression",
            "Full backups kept per game",
            "Differential backups",
            "Restoring saves",
            "Backups kept",
        ],
    ),
    (
        Section::Backups,
        &[
            "Data folder backup",
            "Backup folder",
            "How often",
            "Backups kept",
            "What goes in a backup",
            "Collection",
            "App settings",
            "Your artwork",
            "Store sign-ins",
        ],
    ),
    (
        Section::Cloud,
        &[
            "Service",
            "Sync folder",
            "Advanced options",
            "Folder in the account",
            "Your own Google app",
            "What goes to the cloud",
            "Library between PCs",
            "This PC only sends play time and sessions",
            "Heronite backups",
            "Restore from the cloud",
            "What is in the cloud",
            "Game saves (Ludusavi)",
            "Screenshots",
            "Include replays (videos)",
            "Your PCs",
            "This PC's name",
        ],
    ),
    (
        Section::Advanced,
        &[
            "Logging",
            "Turn off logs",
            "Folders",
            "Data folder",
            "Logs folder",
            "Settings folder",
            "Cache",
            "Clear cache",
            "Image cache limit",
            "Copy all settings",
        ],
    ),
    (
        Section::About,
        &[
            "Updates",
            "Check for updates automatically",
            "Install updates automatically",
            "Help and feedback",
            "Report a problem",
            "Suggest an idea",
            "Ask a question",
            "Like Heronite?",
            "Version",
            "Legendary",
            "Wine / Proton found",
            "Data folder",
            "Settings folder",
            "Cache folder",
            "System",
            "OS",
            "Kernel",
            "CPU",
            "Memory",
            "Session",
            "Copy system information",
        ],
    ),
    (Section::Logs, &["This app", "Runners", "Games"]),
];

/// The settings of each Game defaults tab.
const DEFAULTS_INDEX: &[(Tab, &[&str])] = &[
    (
        Tab::Compatibility,
        &[
            "Wine / Proton",
            "Version",
            "Prefix",
            "Graphics and libraries",
            "DXVK-NVAPI",
            "Sync",
            "Experimental",
            "Wayland driver",
            "WoW64",
            "Folders visible to Proton",
        ],
    ),
    (
        Tab::Performance,
        &[
            "On screen",
            "FPS counter",
            "MangoHud",
            "FPS limit",
            "Maximum FPS",
            "Frame generation",
            "Lossless Scaling frame generation",
            "Wine FSR",
            "gamescope",
            "Image",
            "System",
            "GameMode",
            "Keep the computer awake",
            "Read controllers through SDL",
        ],
    ),
    (
        Tab::Gpu,
        &[
            "Graphics card",
            "Draws the game",
            "Smooth Motion",
            "Reflex in Vulkan games",
            "Bigger shader cache",
            "Upscalers",
            "Newest DLSS",
            "Newest XeSS",
            "All cards",
            "VSync",
            "Card maker the game sees",
            "Video memory the game sees",
        ],
    ),
    (
        Tab::Launch,
        &[
            "Game language",
            "Starting",
            "Before the game starts",
            "After the game exits",
            "Scripts",
            "Presets",
            "Wrapper command",
            "Environment variables",
            "Advanced",
            "Detailed logs",
        ],
    ),
];

/// A setting found inside a section.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Setting {
    /// The English label (translated with `tr` when drawn).
    pub label: &'static str,
    /// The Game defaults tab it is on.
    pub tab: Option<Tab>,
}

/// What Enter opens for a section of the results.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Pick {
    Section(Section),
    Setting(Section, Setting),
}

/// A section of the results and its matching settings, best first.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Hit {
    pub section: Section,
    pub settings: Vec<Setting>,
    /// The section itself matched better than any of its settings.
    pub section_first: bool,
}

impl Hit {
    /// What Enter opens: the section, or its best setting.
    pub fn pick(&self) -> Pick {
        match self.settings.first() {
            Some(&s) if !self.section_first => Pick::Setting(self.section, s),
            _ => Pick::Section(self.section),
        }
    }
}

/// Every indexed setting of `section`.
fn settings_of(section: Section) -> Vec<Setting> {
    if section == Section::GameDefaults {
        return DEFAULTS_INDEX.iter().flat_map(|(tab, labels)| labels.iter().map(|&label| Setting { label, tab: Some(*tab) })).collect();
    }
    let mut settings: Vec<Setting> = INDEX
        .iter()
        .filter(|(s, _)| *s == section)
        .flat_map(|(_, labels)| labels.iter().copied())
        .filter(|label| *label != "Guide + X: save the replay" || Section::Replay.available())
        .map(|label| Setting { label, tab: None })
        .collect();
    if section == Section::Metadata {
        // The sources and their options come from the registry, so a new
        // source is found without a line here.
        let sources = builtin_descriptors().iter().filter(|d| !d.hidden);
        for label in sources.flat_map(|d| std::iter::once(d.name.as_str()).chain(d.options.iter().map(|o| o.label.as_str()))) {
            if !settings.iter().any(|s| s.label == label) {
                settings.push(Setting { label, tab: None });
            }
        }
    }
    settings
}

/// How well `section` itself matches.
fn section_score(tokens: &[&str], s: Section) -> Option<i32> {
    let group = Section::GROUPS.iter().find(|(_, list)| list.contains(&s)).map_or("", |(g, _)| tr(g));
    let title = fuzzy::fold(s.label());
    let group = fuzzy::fold(group);
    let keywords = fuzzy::fold(&format!("{} {}", s.keywords(), s.blurb()));
    fuzzy::score_entry(tokens, &fuzzy::Haystack { title: &title, alt: &group, keywords: &keywords })
}

/// How well a setting's label matches (the English one counts a bit less).
fn setting_score(tokens: &[&str], label: &str) -> Option<i32> {
    let title = fuzzy::fold(tr(label));
    let english = fuzzy::fold(label);
    let alt = if english == title { "" } else { english.as_str() };
    fuzzy::score_entry(tokens, &fuzzy::Haystack { title: &title, alt, keywords: "" })
}

/// The sections and settings `query` finds, best first; nothing when it is
/// empty. A setting named like its section (the Downloads caption of
/// Downloads) is left to the section.
pub fn search(query: &str) -> Vec<Hit> {
    let folded = fuzzy::fold(query);
    let tokens = fuzzy::tokens(&folded);
    if tokens.is_empty() {
        return Vec::new();
    }
    let mut hits: Vec<(i32, usize, Hit)> = Vec::new();
    for (ix, &section) in Section::ALL.iter().enumerate().filter(|(_, s)| s.available()) {
        let own = section_score(&tokens, section);
        let name = fuzzy::fold(section.label());
        let mut found: Vec<(i32, usize, Setting)> = settings_of(section)
            .into_iter()
            .enumerate()
            .filter(|(_, s)| fuzzy::fold(tr(s.label)) != name)
            .filter_map(|(at, s)| setting_score(&tokens, s.label).map(|score| (score, at, s)))
            .collect();
        found.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
        // The same label on two tabs (System): the first one.
        found.dedup_by(|a, b| a.2.label == b.2.label);
        let best_setting = found.first().map(|f| f.0);
        let Some(score) = own.max(best_setting) else { continue };
        let settings = found.into_iter().take(SHOWN).map(|(_, _, s)| s).collect();
        hits.push((score, ix, Hit { section, settings, section_first: own.is_some() && own >= best_setting }));
    }
    hits.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    hits.into_iter().map(|(_, _, h)| h).collect()
}

#[cfg(test)]
mod tests {
    use super::{search, settings_of, Section, DEFAULTS_INDEX, INDEX};
    #[cfg(target_os = "linux")]
    use super::{Pick, Setting, Tab, SHOWN};

    #[test]
    fn replay_search_follows_its_own_capability() {
        let replay = Section::Replay.available();
        assert_eq!(search("replay").iter().any(|hit| hit.section == Section::Replay), replay);
        assert_eq!(settings_of(Section::Overlay).iter().any(|setting| setting.label == "Guide + X: save the replay"), replay);
    }

    #[cfg(windows)]
    #[test]
    fn windows_controller_settings_use_back_start() {
        let settings = settings_of(Section::Overlay);
        assert!(settings.iter().any(|s| s.label == "Back + Start + Y: mark a moment"));
        assert!(settings.iter().all(|s| !s.label.contains("Guide")));
        assert!(search("Back Start").iter().any(|hit| hit.section == Section::Overlay));
    }

    #[cfg(target_os = "linux")]
    fn sections(query: &str) -> Vec<Section> {
        search(query).into_iter().map(|h| h.section).collect()
    }

    #[cfg(target_os = "linux")]
    fn labels(query: &str, section: Section) -> Vec<&'static str> {
        search(query).into_iter().find(|h| h.section == section).map(|h| h.settings.iter().map(|s| s.label).collect()).unwrap_or_default()
    }

    // The Linux page and section list: other systems hide what they lack (`Feature`).
    #[cfg(target_os = "linux")]
    #[test]
    fn finds_sections_by_label_keyword_and_group() {
        assert!(search("").is_empty());
        assert!(search("  ").is_empty());
        assert_eq!(sections("saves").first(), Some(&Section::Saves));
        assert_eq!(sections("Aparência").first(), Some(&Section::Appearance));
        assert_eq!(sections("ludusavi").first(), Some(&Section::Saves));
        assert_eq!(sections("apearance").first(), Some(&Section::Appearance));
        assert!(sections("proton").contains(&Section::GameDefaults));
        // The group caption brings its sections, ahead of a blurb that says the word.
        assert_eq!(sections("while")[..3], [Section::Overlay, Section::Screenshots, Section::Replay]);
        assert!(search("zzqx").is_empty());
    }

    // The Linux page and section list: other systems hide what they lack (`Feature`).
    #[cfg(target_os = "linux")]
    #[test]
    fn finds_the_settings_inside_a_section() {
        let language = search("language");
        assert_eq!(language[0].section, Section::General);
        assert_eq!(
            language[0].settings[..2],
            [Setting { label: "Language", tab: None }, Setting { label: "Interface language", tab: None }]
        );
        assert_eq!(language[0].pick(), Pick::Setting(Section::General, Setting { label: "Language", tab: None }));
        // A Game defaults setting knows its tab.
        assert_eq!(labels("language", Section::GameDefaults), ["Game language"]);
        let wayland = search("wayland");
        let defaults = wayland.iter().find(|h| h.section == Section::GameDefaults).unwrap();
        assert_eq!(defaults.settings[0], Setting { label: "Wayland driver", tab: Some(Tab::Compatibility) });
        // The section's own name wins over its settings.
        assert_eq!(search("downloads")[0].pick(), Pick::Section(Section::Downloads));
        // At most a few settings per section.
        assert!(search("a").iter().all(|h| h.settings.len() <= SHOWN));
    }

    #[test]
    fn finds_the_live_achievement_options() {
        let hits = search("achievements over the game");
        let hit = hits.iter().find(|h| h.section == Section::Achievements).expect("Achievements is found");
        assert!(hit.settings.iter().any(|s| s.label == "Show new achievements over the game"));
        let points = search("points or xp");
        assert!(points.iter().any(|h| h.section == Section::Achievements && h.settings.iter().any(|s| s.label == "Show points or XP")));
        let steam = search("steam games while they run");
        assert!(steam
            .iter()
            .any(|h| h.section == Section::Achievements && h.settings.iter().any(|s| s.label == "Check Steam games while they run")));
    }

    /// Each label must still be a text the app draws, else the search lists
    /// a setting the page cannot flash.
    #[test]
    fn every_indexed_label_is_still_drawn() {
        fn sources(dir: &std::path::Path, out: &mut String) {
            for e in std::fs::read_dir(dir).unwrap().flatten() {
                let p = e.path();
                if p.is_dir() {
                    sources(&p, out);
                } else if p.extension().is_some_and(|e| e == "rs") && !p.ends_with("pages/settings/search.rs") {
                    out.push_str(&std::fs::read_to_string(&p).unwrap());
                }
            }
        }
        // The app and the workspace crates (the launchers' names are `heronite_data`'s).
        let app = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let mut all = String::new();
        sources(&app.join("src"), &mut all);
        for c in std::fs::read_dir(app.join("../crates")).unwrap().flatten() {
            if c.path().join("src").is_dir() {
                sources(&c.path().join("src"), &mut all);
            }
        }
        let labels = INDEX.iter().flat_map(|(_, l)| l.iter()).chain(DEFAULTS_INDEX.iter().flat_map(|(_, l)| l.iter()));
        let missing: Vec<&str> = labels.filter(|l| !all.contains(&format!("{l:?}"))).copied().collect();
        assert!(missing.is_empty(), "labels no longer drawn: {missing:?}");
    }
}
