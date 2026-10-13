//! The steps of the getting-started checklist: their text, what they lead
//! to, and whether each one is done (read from the library and the settings
//! when the checklist opens; the ones without a setting count once done here).

use std::collections::BTreeSet;

use heronite_data::Library;
use heronite_i18n::{tr, trn};

use crate::icons::Icon;
use crate::pages::Page;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    Steam,
    Stores,
    Emulators,
    Proton,
    Screenshots,
    Saves,
    Tray,
    Appearance,
    Palette,
    Statuses,
    Stats,
    Console,
}

pub const STEPS: [Step; 12] = [
    Step::Steam,
    Step::Stores,
    Step::Emulators,
    Step::Proton,
    Step::Screenshots,
    Step::Saves,
    Step::Tray,
    Step::Appearance,
    Step::Palette,
    Step::Statuses,
    Step::Stats,
    Step::Console,
];

/// What the checklist asks the Collection to do (it has the window).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Page(Page),
    Palette,
    Console,
    /// Settings → Appearance.
    Appearance,
    /// A step done inside the checklist ("Got it").
    Done(Step),
    /// "Don't show again".
    Hide,
}

/// How a step is done from the checklist.
pub enum Kind {
    /// A button that leads to the feature.
    Go { label: &'static str, action: Action },
    /// A switch that turns the feature on right here.
    Switch { label: String },
}

pub fn group_title(group: usize) -> &'static str {
    match group {
        0 => tr!("Bring in your games"),
        1 => tr!("Ready to play"),
        _ => tr!("Discover Heronite"),
    }
}

impl Step {
    /// Stored in `onboarding.json` (`done`).
    pub fn id(self) -> &'static str {
        match self {
            Step::Steam => "steam",
            Step::Stores => "stores",
            Step::Emulators => "emulators",
            Step::Proton => "proton",
            Step::Screenshots => "screenshots",
            Step::Saves => "saves",
            Step::Tray => "tray",
            Step::Palette => "palette",
            Step::Statuses => "statuses",
            Step::Appearance => "appearance",
            Step::Stats => "stats",
            Step::Console => "console",
        }
    }

    pub fn parse(id: &str) -> Option<Self> {
        STEPS.into_iter().find(|s| s.id() == id)
    }

    pub fn group(self) -> usize {
        match self {
            Step::Steam | Step::Stores | Step::Emulators => 0,
            Step::Proton | Step::Screenshots | Step::Saves | Step::Tray => 1,
            _ => 2,
        }
    }

    pub fn icon(self) -> Icon {
        match self {
            Step::Steam => Icon::CloudSync,
            Step::Stores => Icon::Storefront,
            Step::Emulators => Icon::SportsEsports,
            Step::Proton => Icon::WineBar,
            Step::Screenshots => Icon::PhotoCamera,
            Step::Saves => Icon::Save,
            Step::Tray => Icon::Inbox,
            Step::Palette => Icon::FaSearch,
            Step::Statuses => Icon::Flag,
            Step::Appearance => Icon::Palette,
            Step::Stats => Icon::BarChart,
            Step::Console => Icon::Tv,
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Step::Steam => tr!("Steam library"),
            Step::Stores => tr!("Epic, GOG and Amazon"),
            Step::Emulators => tr!("Emulators and ROMs"),
            Step::Proton => tr!("Proton for Windows games"),
            Step::Screenshots => tr!("Screenshots"),
            Step::Saves => tr!("Save backups"),
            Step::Tray => tr!("Always at hand"),
            Step::Palette => tr!("Ctrl K finds everything"),
            Step::Statuses => tr!("Organise with statuses"),
            Step::Appearance => tr!("Make it yours"),
            Step::Stats => tr!("Statistics and Year in review"),
            Step::Console => tr!("Console mode"),
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Step::Steam => tr!("Your Steam games come in by themselves while Steam is open, with the play time Steam counted."),
            Step::Stores => tr!("Log in and your whole library shows in the Collection. You install games from there."),
            Step::Emulators => {
                tr!("Heronite finds the emulators on this PC and scans your ROM folders. Each game becomes a cover in the Collection.")
            }
            Step::Proton => tr!("Download it once and it becomes the default: Windows games start without any setup."),
            Step::Screenshots => {
                tr!("Press the capture key while playing. The pictures go to the game's timeline and to your Year in review.")
            }
            Step::Saves => tr!("When you close a game, Ludusavi keeps a copy of its saves. If something goes wrong, you restore it."),
            Step::Tray => tr!("Closing the window keeps Heronite in the tray: downloads go on and notifications still arrive."),
            Step::Palette => tr!("Games, pages and actions in one place. Type part of a name and press Enter."),
            Step::Statuses => {
                tr!("Mark games as Playing, Beaten or Plan to Play and rate them. Then group the Collection by status.")
            }
            Step::Appearance => tr!(
                "Change how Heronite looks: the game's art behind the Collection with the blur, darkening and colour you like, glass or solid panels in several colours, and the shape of the covers."
            ),
            Step::Stats => tr!("Your play time month by month and, at the end of the year, the story of your year in games."),
            Step::Console => tr!("Play from the couch with just a controller: big screens and button navigation."),
        }
    }

    /// Where the feature lives, for later.
    pub fn how(self) -> &'static str {
        match self {
            Step::Steam => tr!("Automatic while Steam is open"),
            Step::Stores => tr!("\u{2630} \u{2192} Stores"),
            Step::Emulators => tr!("\u{2630} \u{2192} Emulators"),
            Step::Proton => tr!("\u{2630} \u{2192} Wine Manager"),
            Step::Screenshots => tr!("Settings \u{2192} Screenshots"),
            Step::Saves => tr!("Settings \u{2192} Backups"),
            Step::Tray => tr!("Settings \u{2192} General"),
            Step::Palette => tr!("Ctrl K on any screen"),
            Step::Statuses => tr!("Right-click a game \u{2192} Status"),
            Step::Appearance => tr!("Settings \u{2192} Appearance"),
            Step::Stats => tr!("\u{2630} \u{2192} Statistics"),
            Step::Console => tr!("\u{2630} \u{2192} Console mode"),
        }
    }

    pub fn kind(self, facts: &Facts) -> Kind {
        let go = |label, action| Kind::Go { label, action };
        match self {
            Step::Steam => go(tr!("Open the Steam page"), Action::Page(Page::Steam)),
            Step::Stores => go(tr!("Log in to a store"), Action::Page(Page::Epic)),
            Step::Emulators => go(tr!("Set up emulators"), Action::Page(Page::Emulators)),
            Step::Proton => go(tr!("Open the Wine Manager"), Action::Page(Page::WineManager)),
            Step::Screenshots => Kind::Switch { label: tr!("Capture with {key}", key = facts.shot_key) },
            Step::Saves => Kind::Switch { label: tr!("Back up after every session").into() },
            Step::Tray => Kind::Switch { label: tr!("Keep Heronite in the tray when closed").into() },
            Step::Palette => go(tr!("Try it now"), Action::Palette),
            Step::Statuses => go(tr!("Got it"), Action::Done(Step::Statuses)),
            Step::Appearance => go(tr!("Customise the look"), Action::Appearance),
            Step::Stats => go(tr!("Open statistics"), Action::Page(Page::Statistics)),
            Step::Console => go(tr!("Try console mode"), Action::Console),
        }
    }
}

/// What the checklist knows about this setup.
#[derive(Clone, Debug, Default)]
pub struct Facts {
    pub steam_games: usize,
    pub store_games: usize,
    pub emulated_games: usize,
    /// The default Wine/Proton, when it can run games.
    pub proton: Option<String>,
    pub screenshots: bool,
    pub shot_key: String,
    pub saves: bool,
    /// Whether Ludusavi is installed (`None` while looking).
    pub ludusavi: Option<bool>,
    pub tray: bool,
    /// `onboarding.json`'s `done`.
    pub done: BTreeSet<String>,
}

impl Facts {
    /// Reads the library and the settings (small files).
    pub fn read(lib: &Library, data_paths: &heronite_data::DataPaths, done: BTreeSet<String>) -> Self {
        let count = |f: &dyn Fn(&heronite_data::CollectionGame) -> bool| lib.games.iter().filter(|g| f(g)).count();
        Self {
            steam_games: count(&|g| g.steam_app_id.is_some()),
            store_games: count(&|g| matches!(g.runner.as_str(), "legendary" | "gog" | "nile")),
            emulated_games: count(&|g| g.launch_kind.as_deref() == Some("emulator")),
            proton: heronite_wine::default::current(data_paths).map(|w| w.name),
            screenshots: lib.settings.screenshots.capture_enabled,
            shot_key: Some(lib.settings.screenshots.capture_accelerator.clone()).filter(|k| !k.is_empty()).unwrap_or_else(|| "F10".into()),
            saves: lib.settings.ludusavi.enabled,
            ludusavi: None,
            tray: crate::system::settings::SysSettings::load(data_paths).exit_to_tray,
            done,
        }
    }

    pub fn is_done(&self, step: Step) -> bool {
        self.done.contains(step.id())
            || match step {
                Step::Steam => self.steam_games > 0,
                Step::Stores => self.store_games > 0,
                Step::Emulators => self.emulated_games > 0,
                Step::Proton => self.proton.is_some(),
                Step::Screenshots => self.screenshots,
                Step::Saves => self.saves,
                Step::Tray => self.tray,
                _ => false,
            }
    }

    /// The line under a step's title in the list.
    pub fn status_line(&self, step: Step) -> String {
        if !self.is_done(step) {
            return step.how().into();
        }
        match step {
            Step::Steam if self.steam_games > 0 => trn!(self.steam_games, "{n} game from Steam", "{n} games from Steam"),
            Step::Stores if self.store_games > 0 => trn!(self.store_games, "{n} game from the stores", "{n} games from the stores"),
            Step::Emulators if self.emulated_games > 0 => trn!(self.emulated_games, "{n} emulated game", "{n} emulated games"),
            Step::Proton => match &self.proton {
                Some(name) => tr!("{name} is the default", name),
                None => step.how().into(),
            },
            _ => step.how().into(),
        }
    }

    pub fn done_count(&self) -> usize {
        STEPS.iter().filter(|s| self.is_done(**s)).count()
    }
}
