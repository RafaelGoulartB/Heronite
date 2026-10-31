//! What "Getting started" offers: the setup steps of its three cards (your
//! games, your look, while you play), the tips to discover later, what each
//! one leads to, and whether it is done (read from the library and the
//! settings when the dialog opens; the ones without a setting count once
//! done here).

use std::collections::BTreeSet;

use heronite_data::overlay_prefs::OverlayPrefs;
use heronite_data::{AppPaths, Library};
use heronite_i18n::{tr, trn};

use crate::icons::Icon;
use crate::pages::settings::Section;
use crate::pages::Page;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    Steam,
    Stores,
    Emulators,
    Appearance,
    Overlay,
    Screenshots,
    Saves,
    Proton,
    Tray,
    Palette,
    Statuses,
    Journal,
    Stats,
    Console,
}

/// The steps that count in the progress, card by card.
pub const GAMES: [Step; 3] = [Step::Steam, Step::Stores, Step::Emulators];
pub const PLAY: [Step; 5] = [Step::Overlay, Step::Screenshots, Step::Saves, Step::Proton, Step::Tray];
/// The tips under the cards: nothing to set up, only to know about.
pub const TIPS: [Step; 5] = [Step::Palette, Step::Statuses, Step::Journal, Step::Stats, Step::Console];

const ALL: [Step; 14] = [
    Step::Steam,
    Step::Stores,
    Step::Emulators,
    Step::Appearance,
    Step::Overlay,
    Step::Screenshots,
    Step::Saves,
    Step::Proton,
    Step::Tray,
    Step::Palette,
    Step::Statuses,
    Step::Journal,
    Step::Stats,
    Step::Console,
];

/// What the dialog asks the Collection to do (it has the window).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Page(Page),
    /// The store page shown last (Epic, GOG or Amazon).
    Stores,
    AddGame,
    Settings(Section),
    Palette,
    Console,
    /// The appearance changed here: the Collection repaints and applies
    /// "Open the Collection in".
    LookChanged,
    /// A tip read here ("Got it").
    Done(Step),
    /// "Don't show again".
    Hide,
}

impl Step {
    /// Stored in `onboarding.json` (`done`).
    pub fn id(self) -> &'static str {
        match self {
            Step::Steam => "steam",
            Step::Stores => "stores",
            Step::Emulators => "emulators",
            Step::Appearance => "appearance",
            Step::Overlay => "overlay",
            Step::Screenshots => "screenshots",
            Step::Saves => "saves",
            Step::Proton => "proton",
            Step::Tray => "tray",
            Step::Palette => "palette",
            Step::Statuses => "statuses",
            Step::Journal => "journal",
            Step::Stats => "stats",
            Step::Console => "console",
        }
    }

    /// Whether this system has what the step sets up
    /// (`heronite_platform::features`).
    pub fn available(self) -> bool {
        use heronite_platform::{supported, Feature};
        match self {
            Step::Stores => heronite_platform::features::any_store(),
            Step::Emulators => supported(Feature::Emulation),
            Step::Saves => supported(Feature::SaveBackups),
            Step::Overlay => supported(Feature::Overlay),
            Step::Screenshots => supported(Feature::Capture),
            Step::Proton => supported(Feature::Wine),
            Step::Tray => supported(Feature::Tray),
            _ => true,
        }
    }

    pub fn parse(id: &str) -> Option<Self> {
        ALL.into_iter().find(|s| s.id() == id)
    }

    pub fn icon(self) -> Icon {
        match self {
            Step::Steam => Icon::CloudSync,
            Step::Stores => Icon::Storefront,
            Step::Emulators => Icon::SportsEsports,
            Step::Appearance => Icon::Palette,
            Step::Overlay => Icon::SportsEsports,
            Step::Screenshots => Icon::PhotoCamera,
            Step::Saves => Icon::Save,
            Step::Proton => Icon::WineBar,
            Step::Tray => Icon::Inbox,
            Step::Palette => Icon::FaSearch,
            Step::Statuses => Icon::Flag,
            Step::Journal => Icon::Notes,
            Step::Stats => Icon::BarChart,
            Step::Console => Icon::Tv,
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Step::Steam => "Steam",
            Step::Stores => tr!("Epic, GOG and Amazon"),
            Step::Emulators => tr!("Emulators and ROMs"),
            Step::Appearance => tr!("Make it yours"),
            Step::Overlay => tr!("Controller overlay"),
            Step::Screenshots => tr!("Screenshots"),
            Step::Saves => tr!("Save backups"),
            Step::Proton => "Proton",
            Step::Tray => tr!("Keep in the tray"),
            Step::Palette => "Ctrl K",
            Step::Statuses => tr!("Status and rating"),
            Step::Journal => tr!("The game's journal"),
            Step::Stats => tr!("Statistics"),
            Step::Console => tr!("Console mode"),
        }
    }

    /// One line on what it is for (the tips and the source tiles).
    pub fn description(self) -> &'static str {
        match self {
            Step::Steam => tr!("Open Steam and your games come in by themselves, installed or not, with Steam's play time."),
            Step::Stores => tr!("Log in and your whole library shows in the Collection. You install games from there."),
            Step::Emulators => tr!("Heronite finds the emulators on this PC and scans your ROM folders."),
            Step::Palette => tr!("Games, pages and actions in one place."),
            Step::Statuses => tr!("Playing, Beaten, Plan to Play. Group the Collection by them."),
            Step::Journal => tr!("Notes on the days you played, on screenshots, and where you left off."),
            Step::Stats => tr!("Your play time month by month and your Year in review."),
            Step::Console => tr!("From the couch, with just a controller."),
            _ => "",
        }
    }

    /// Where the feature lives, for later.
    pub fn how(self) -> &'static str {
        match self {
            Step::Statuses => tr!("Right-click a game \u{2192} Status"),
            Step::Journal => tr!("A game's page \u{2192} Journal"),
            _ => "",
        }
    }

    /// What clicking the step does.
    pub fn action(self) -> Action {
        match self {
            Step::Steam => Action::Page(Page::Steam),
            Step::Stores => Action::Stores,
            Step::Emulators => Action::Page(Page::Emulators),
            Step::Appearance => Action::Settings(Section::Appearance),
            Step::Overlay => Action::Settings(Section::Overlay),
            Step::Screenshots => Action::Settings(Section::Screenshots),
            Step::Saves => Action::Settings(Section::Saves),
            Step::Proton => Action::Page(Page::WineManager),
            Step::Tray => Action::Settings(Section::General),
            Step::Palette => Action::Palette,
            Step::Statuses | Step::Journal => Action::Done(self),
            Step::Stats => Action::Page(Page::Statistics),
            Step::Console => Action::Console,
        }
    }

    /// The label of a tip's action.
    pub fn tip_label(self) -> &'static str {
        match self {
            Step::Palette => tr!("Try it"),
            Step::Statuses | Step::Journal => tr!("Got it"),
            Step::Console => tr!("Try it"),
            _ => tr!("Open"),
        }
    }
}

/// What the dialog knows about this setup.
#[derive(Clone, Debug, Default)]
pub struct Facts {
    pub steam_games: usize,
    pub store_games: usize,
    pub emulated_games: usize,
    /// The default Wine/Proton, when it can run games.
    pub proton: Option<String>,
    pub overlay: OverlayPrefs,
    /// This desktop can draw the overlay over games, and why not.
    pub overlay_ok: bool,
    pub overlay_why: String,
    pub screenshots: bool,
    pub shot_key: String,
    pub saves: bool,
    /// Whether Ludusavi is installed (`None` while looking).
    pub ludusavi: Option<bool>,
    pub tray: bool,
    /// The Collection shows the Grid view (else the Focus view).
    pub grid_view: bool,
    /// `onboarding.json`'s `done`.
    pub done: BTreeSet<String>,
}

impl Facts {
    /// Reads the library and the settings (small files).
    pub fn read(lib: &Library, data_paths: &heronite_data::DataPaths, app: &AppPaths, done: BTreeSet<String>) -> Self {
        let count = |f: &dyn Fn(&heronite_data::CollectionGame) -> bool| lib.games.iter().filter(|g| f(g)).count();
        let (overlay_why, overlay_ok) = crate::overlay::desktop_status();
        Self {
            steam_games: count(&|g| g.steam_app_id.is_some()),
            store_games: count(&|g| matches!(g.runner.as_str(), "legendary" | "gog" | "nile")),
            emulated_games: count(&|g| g.launch_kind.as_deref() == Some("emulator")),
            proton: heronite_wine::default::current(data_paths).map(|w| w.name),
            overlay: OverlayPrefs::load(app),
            overlay_ok,
            overlay_why,
            screenshots: lib.settings.screenshots.capture_enabled,
            shot_key: Some(lib.settings.screenshots.capture_accelerator.clone()).filter(|k| !k.is_empty()).unwrap_or_else(|| "F10".into()),
            saves: lib.settings.ludusavi.enabled,
            ludusavi: None,
            tray: crate::system::settings::SysSettings::load(data_paths).exit_to_tray,
            grid_view: heronite_data::ui_state::UiState::load(app).grid_view,
            done,
        }
    }

    pub fn games(&self) -> usize {
        self.steam_games + self.store_games + self.emulated_games
    }

    pub fn is_done(&self, step: Step) -> bool {
        self.done.contains(step.id())
            || match step {
                Step::Steam => self.steam_games > 0,
                Step::Stores => self.store_games > 0,
                Step::Emulators => self.emulated_games > 0,
                Step::Overlay => self.overlay.enabled,
                Step::Screenshots => self.screenshots,
                Step::Saves => self.saves,
                Step::Proton => self.proton.is_some(),
                Step::Tray => self.tray,
                _ => false,
            }
    }

    /// The games a source brought in, as "{n} games".
    pub fn count_line(&self, step: Step) -> Option<String> {
        let n = match step {
            Step::Steam => self.steam_games,
            Step::Stores => self.store_games,
            Step::Emulators => self.emulated_games,
            _ => 0,
        };
        (n > 0).then(|| trn!(n, "{n} game", "{n} games"))
    }

    /// The steps the progress counts: the three cards (the overlay only
    /// where the desktop can draw it).
    pub fn counted(&self) -> Vec<Step> {
        GAMES
            .into_iter()
            .chain([Step::Appearance])
            .chain(PLAY.into_iter().filter(|s| *s != Step::Overlay || self.overlay_ok))
            .filter(|s| s.available())
            .collect()
    }

    pub fn done_count(&self) -> usize {
        self.counted().into_iter().filter(|s| self.is_done(*s)).count()
    }
}
