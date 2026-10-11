//! What the palette can find: pages, actions (global, Collection view,
//! page commands, the selected game's) and games. Entries are built once
//! when the palette opens (titles folded for matching); a keystroke only
//! ranks them (see [`super::view`]).

use gpui_kit::SharedString;
use heronite_data::collection::{FacetOptions, FACET_KINDS};
use heronite_data::ui_state::UiState;
use heronite_data::{FacetKind, InstallFilter, SortMode};

use super::matching::{fold, Haystack};
use crate::collection::game_actions::{GameAction, GameActionItem};
use crate::icons::Icon;
use crate::pages::commands::PageCommand;
use crate::pages::settings::Section;
use crate::pages::Page;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Page,
    Action,
    Game,
}

/// What choosing an entry does (the Collection runs it, see `palette/mod.rs`).
#[derive(Clone, PartialEq, Eq, Debug)]
pub(super) enum Act {
    Go(Page),
    /// Settings, opened on this section.
    Section(Section),
    AddGame,
    Refresh,
    ScanRoms,
    EpicUpdates,
    OpenLogs,
    /// Console mode (controller, TV layout).
    Console,
    /// Statistics, Year in review of this year.
    YearReview,
    ManageStatuses,
    Quit,
    /// Sort, filters and grouping of the Collection list.
    View(ViewCmd),
    /// A command of a page (`pages/commands.rs`).
    PageCmd {
        page: Page,
        id: String,
        navigate: bool,
    },
    SelectGame(String),
    /// An action of the game under the key (`game_actions.rs`).
    Game(String, GameAction),
    /// Lists every action of the game (the palette's second level).
    GameActions(String),
}

/// The Collection's View menu, as commands.
#[derive(Clone, PartialEq, Eq, Debug)]
pub(super) enum ViewCmd {
    Sort(SortMode),
    Show(InstallFilter),
    GroupByStatus,
    ShowHidden,
    ClearFilters,
    /// Selects (or, when it is the active one, clears) a facet value.
    Facet(FacetKind, String),
}

/// What a game entry can do besides being selected.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct GameBits {
    pub key: String,
    pub installed: bool,
    pub has_folder: bool,
    /// Running now (its main action is Stop).
    pub playing: bool,
    pub favourite: bool,
    pub hidden: bool,
    /// ISO timestamp (sorts as text); `None` = never played.
    pub last_played: Option<String>,
}

#[derive(Clone, Debug)]
pub(super) struct Entry {
    pub kind: Kind,
    pub act: Act,
    /// Stable id, kept in the palette's history.
    pub id: SharedString,
    pub title: SharedString,
    pub subtitle: SharedString,
    pub icon: Icon,
    pub shortcut: Option<&'static str>,
    /// Only listed when something is typed or the Pages filter is on
    /// (the Settings sections).
    pub secondary: bool,
    /// Never listed without a query (the facet values: there can be
    /// hundreds).
    pub search_only: bool,
    /// Why it cannot run now (listed dimmed with this text).
    pub disabled: Option<SharedString>,
    /// An action of the game selected in the Collection.
    pub context: bool,
    pub game: Option<GameBits>,
    // Folded search texts.
    pub f_title: String,
    pub f_alt: String,
    pub f_keywords: String,
}

impl Entry {
    pub(super) fn haystack(&self) -> Haystack<'_> {
        Haystack { title: &self.f_title, alt: &self.f_alt, keywords: &self.f_keywords }
    }

    fn new(kind: Kind, act: Act, id: impl Into<SharedString>, title: &str, subtitle: &str, icon: Icon, keywords: &str) -> Entry {
        Entry {
            kind,
            act,
            id: id.into(),
            title: title.to_string().into(),
            subtitle: subtitle.to_string().into(),
            icon,
            shortcut: None,
            secondary: false,
            search_only: false,
            disabled: None,
            context: false,
            game: None,
            f_title: fold(title),
            f_alt: String::new(),
            f_keywords: fold(keywords),
        }
    }

    fn shortcut(mut self, s: &'static str) -> Entry {
        self.shortcut = Some(s);
        self
    }

    pub(super) fn disable(&mut self, reason: impl Into<SharedString>) {
        self.disabled = Some(reason.into());
    }

    /// The game this entry is about (a game or one of its actions).
    pub(super) fn game_key(&self) -> Option<&str> {
        match &self.act {
            Act::SelectGame(k) | Act::Game(k, _) | Act::GameActions(k) => Some(k),
            _ => None,
        }
    }
}

/// Settings sections the palette opens directly: (section, one-line help).
fn section_help(s: Section) -> &'static str {
    match s {
        Section::General => "Folders, tray, Wine and Proton lookup",
        Section::Appearance => "Page background, blur, covers",
        Section::Advanced => "Folders, cache, copy settings",
        Section::GameDefaults => "Values new games start with",
        Section::Downloads => "Epic downloads and updates",
        Section::Library => "Hidden games, favourites, recent games",
        Section::Metadata => "IGDB keys, covers, field sources",
        Section::Screenshots => "Folder, capture shortcut, preview cache",
        Section::Backups => "Ludusavi saves and data folder backup",
        Section::Logs => "Logs of this app and your games",
        Section::About => "Version, data folders, system",
    }
}

/// Pages and global actions: the same list on every open.
pub(super) fn static_entries() -> Vec<Entry> {
    let mut v = Vec::new();
    let page = |p: Page, sub: &str, shortcut: &'static str, kw: &str| {
        Entry::new(Kind::Page, Act::Go(p), format!("go:{p:?}"), p.title(), sub, p.icon(), kw).shortcut(shortcut)
    };
    v.push(page(Page::Collection, "Your games", "Ctrl 1", "library home games list"));
    v.push(page(Page::Downloads, "Epic installs and updates", "Ctrl 2", "queue install update epic"));
    v.push(page(Page::WineManager, "Download and manage Proton / Wine", "Ctrl 3", "proton wine runner compatibility"));
    v.push(page(Page::Epic, "Account and library", "Ctrl 4", "epic games store account login legendary"));
    v.push(page(Page::Emulators, "Emulators and ROM folders", "Ctrl 5", "emulator rom retro console"));
    v.push(page(Page::Statistics, "Play time and habits", "Ctrl 6", "stats playtime hours activity chart most played"));
    v.push(page(Page::Settings, "Global settings", "Ctrl ,", "preferences options config"));
    for s in Section::ALL {
        let mut e = Entry::new(
            Kind::Page,
            Act::Section(s),
            format!("section:{s:?}"),
            &format!("Settings \u{203a} {}", s.label()),
            section_help(s),
            Page::Settings.icon(),
            &format!("preferences options config {}", s.keywords()),
        );
        e.secondary = true;
        v.push(e);
    }
    let action = |a: Act, id: &str, title: &str, sub: &str, icon: Icon, kw: &str| {
        Entry::new(Kind::Action, a, format!("act:{id}"), title, sub, icon, kw)
    };
    v.push(
        action(Act::AddGame, "add-game", "Add game\u{2026}", "Sideload a program, browser game or ROM", Icon::Add, "new sideload import")
            .shortcut("Ctrl N"),
    );
    v.push(
        action(
            Act::Refresh,
            "refresh",
            "Refresh library and scan ROMs",
            "Reload the library and import new ROMs",
            Icon::Refresh,
            "reload update rescan",
        )
        .shortcut("F5"),
    );
    v.push(action(
        Act::ScanRoms,
        "scan-roms",
        "Scan ROMs\u{2026}",
        "Add games from a ROM folder",
        Icon::FolderOpen,
        "emulation import folder",
    ));
    v.push(action(
        Act::EpicUpdates,
        "epic-updates",
        "Check for Epic updates",
        "Refresh the Epic library and look for updates",
        Icon::Update,
        "epic games legendary upgrade",
    ));
    v.push(action(Act::OpenLogs, "logs", "Open logs folder", "App, runner and game logs", Icon::Description, "log folder directory debug"));
    v.push(action(
        Act::YearReview,
        "year-review",
        "Year in review",
        "Your year in games, as stories",
        Icon::BarChart,
        "wrapped recap retrospective statistics stats year summary",
    ));
    v.push(action(
        Act::ManageStatuses,
        "statuses",
        "Manage statuses\u{2026}",
        "Add, rename and order the statuses",
        Icon::Flag,
        "status playing beaten completed custom",
    ));
    v.push(action(
        Act::Console,
        "console",
        "Console mode",
        "Controller-first TV layout",
        Icon::Tv,
        "controller gamepad tv big picture couch",
    ));
    v.push(action(Act::Quit, "quit", "Quit", "Close Heronite", Icon::Logout, "exit close"));
    v
}

fn facet_label(k: FacetKind) -> &'static str {
    match k {
        FacetKind::Series => "Series",
        FacetKind::Developer => "Developer",
        FacetKind::Publisher => "Publisher",
        FacetKind::Genre => "Genre",
        FacetKind::Platform => "Platform",
    }
}

/// The Collection's View menu (sort, show, grouping, hidden games, facet
/// filters) as commands, for the state in `ui`.
pub(super) fn view_entries(ui: &UiState, facets: &FacetOptions) -> Vec<Entry> {
    let mut v = Vec::new();
    let cmd = |c: ViewCmd, id: String, title: &str, sub: &str, icon: Icon, kw: &str| {
        Entry::new(Kind::Action, Act::View(c), id, title, sub, icon, kw)
    };
    for (mode, id, label) in [
        (SortMode::Title, "title", "title"),
        (SortMode::LastPlayed, "last-played", "last played"),
        (SortMode::Playtime, "playtime", "playtime"),
    ] {
        let mut e = cmd(
            ViewCmd::Sort(mode),
            format!("view:sort:{id}"),
            &format!("Sort by {label}"),
            "Order of the Collection list",
            Icon::Tune,
            "order view list",
        );
        if ui.sort == mode {
            e.disable("Current order");
        }
        v.push(e);
    }
    for (f, id, title) in [
        (InstallFilter::All, "all", "Show all games"),
        (InstallFilter::Installed, "installed", "Show installed games only"),
        (InstallFilter::Uninstalled, "uninstalled", "Show games not installed only"),
    ] {
        let mut e = cmd(ViewCmd::Show(f), format!("view:show:{id}"), title, "Collection list", Icon::Tune, "filter view installed");
        if ui.install_filter == f {
            e.disable("Shown now");
        }
        v.push(e);
    }
    let group = if ui.group_by_status { "Stop grouping by status" } else { "Group by status" };
    v.push(cmd(ViewCmd::GroupByStatus, "view:group".into(), group, "Collection list", Icon::Flag, "view status groups sections"));
    let hidden = if ui.show_hidden { "Stop showing hidden games" } else { "Show hidden games" };
    v.push(cmd(ViewCmd::ShowHidden, "view:hidden".into(), hidden, "Collection list", Icon::Visibility, "view hidden unhide"));
    let mut clear = cmd(
        ViewCmd::ClearFilters,
        "view:clear".into(),
        "Clear filters",
        "Series, developer, publisher, genre, platform",
        Icon::Close,
        "reset filters facets",
    );
    if !ui.facet_filters.is_active() {
        clear.disable("No filter is active");
    }
    v.push(clear);
    for kind in FACET_KINDS {
        let active = ui.facet_filters.get(kind).map(fold);
        let label = facet_label(kind);
        for o in facets.get(kind) {
            let on = active.as_deref() == Some(fold(&o.value).as_str());
            let title = if on { format!("Remove filter {label}: {}", o.value) } else { format!("Filter {label}: {}", o.value) };
            let sub = format!("{} game{}", o.count, if o.count == 1 { "" } else { "s" });
            let id = format!("view:facet:{}:{}", kind.as_str(), o.value);
            let mut e = cmd(ViewCmd::Facet(kind, o.value.clone()), id, &title, &sub, Icon::Tune, &format!("filter {label}"));
            e.search_only = true;
            v.push(e);
        }
    }
    v
}

/// A page's palette command.
pub(super) fn page_entry(page: Page, c: &PageCommand) -> Entry {
    let act = Act::PageCmd { page, id: c.id.clone(), navigate: c.navigate };
    let mut e = Entry::new(
        Kind::Action,
        act,
        format!("page:{page:?}:{}", c.id),
        &c.title,
        &c.subtitle,
        c.icon,
        &format!("{} {}", page.title(), c.keywords),
    );
    if let Some(why) = &c.disabled {
        e.disable(why.clone());
    }
    e
}

/// Library facts the palette needs of one game.
pub(super) struct GameRow<'a> {
    pub key: &'a str,
    pub title: &'a str,
    /// The store's own title when it differs (alternative name).
    pub store_title: &'a str,
    pub source: &'a str,
    pub playtime: &'a str,
    pub installed: bool,
    pub has_folder: bool,
    pub playing: bool,
    pub favourite: bool,
    pub hidden: bool,
    pub last_played: Option<&'a str>,
}

pub(super) fn game_entry(g: &GameRow) -> Entry {
    let sub = if g.playing { format!("{} \u{b7} Playing now", g.source) } else { format!("{} \u{b7} {}", g.source, g.playtime) };
    let mut e =
        Entry::new(Kind::Game, Act::SelectGame(g.key.to_string()), format!("game:{}", g.key), g.title, &sub, Icon::SportsEsports, "");
    if !g.store_title.is_empty() && g.store_title != g.title {
        e.f_alt = fold(g.store_title);
    }
    e.game = Some(GameBits {
        key: g.key.to_string(),
        installed: g.installed,
        has_folder: g.has_folder,
        playing: g.playing,
        favourite: g.favourite,
        hidden: g.hidden,
        last_played: g.last_played.map(str::to_string),
    });
    e
}

/// Verb entries of a game: "Play X" (or "Stop X" / "Install X"),
/// "X settings", "Open X folder", favourites, hide, log and "All actions".
/// Only the ones that make sense for its state.
pub(super) fn game_verbs(title: &str, bits: &GameBits) -> Vec<Entry> {
    let mut v = Vec::new();
    let mk = |act: GameAction, id: &str, text: String, sub: &str, icon: Icon| {
        let mut e = Entry::new(Kind::Game, Act::Game(bits.key.clone(), act), format!("game:{}:{id}", bits.key), &text, sub, icon, "");
        e.game = Some(bits.clone());
        e
    };
    v.push(if bits.playing {
        mk(GameAction::Primary, "primary", format!("Stop {title}"), "Close the running game", Icon::Stop)
    } else if bits.installed {
        mk(GameAction::Primary, "primary", format!("Play {title}"), "Launch the game", Icon::PlayArrow)
    } else {
        mk(GameAction::Primary, "primary", format!("Install {title}"), "Install the game", Icon::Download)
    });
    if bits.installed && !bits.playing {
        v.push(mk(GameAction::Settings, "settings", format!("{title} settings"), "Wine, launch options, art\u{2026}", Icon::Tune));
    }
    if bits.has_folder {
        v.push(mk(
            GameAction::OpenFolder,
            "folder",
            format!("Open {title} folder"),
            "Install folder in the file manager",
            Icon::FolderOpen,
        ));
    }
    v.push(if bits.favourite {
        mk(GameAction::Favourite, "fav", format!("Remove {title} from favourites"), "Favourites", Icon::Favorite)
    } else {
        mk(GameAction::Favourite, "fav", format!("Add {title} to favourites"), "Favourites", Icon::FavoriteBorder)
    });
    v.push(if bits.hidden {
        mk(GameAction::Hide, "hide", format!("Unhide {title}"), "Show it in the Collection again", Icon::Visibility)
    } else {
        mk(GameAction::Hide, "hide", format!("Hide {title}"), "Leave it out of the Collection", Icon::VisibilityOff)
    });
    v.push(mk(GameAction::Log, "log", format!("{title} log"), "Last launch log", Icon::Description));
    let mut all = Entry::new(
        Kind::Game,
        Act::GameActions(bits.key.clone()),
        format!("game:{}:actions", bits.key),
        &format!("All actions for {title}\u{2026}"),
        "Status, uninstall, shortcuts\u{2026}",
        Icon::Menu,
        "",
    );
    all.game = Some(bits.clone());
    v.push(all);
    v
}

/// The verbs listed under the best game match (the rest are reached by
/// typing them, or through "All actions").
pub(super) fn is_main_verb(e: &Entry) -> bool {
    matches!(e.act, Act::Game(_, GameAction::Primary | GameAction::Settings | GameAction::OpenFolder) | Act::GameActions(_))
}

/// Entries of a game's action list. `context` marks the actions of the game
/// selected in the Collection, listed among the other actions (the title
/// says the action, the subtitle the game).
pub(super) fn action_entries(key: &str, game_title: &str, items: &[GameActionItem], context: bool) -> Vec<Entry> {
    let mut v: Vec<Entry> = items
        .iter()
        .map(|a| {
            // On the game's own list its name is the caption already.
            let sub = if context { game_title } else { "" };
            let mut e = Entry::new(
                Kind::Action,
                Act::Game(key.to_string(), a.action.clone()),
                format!("game:{key}:{}", a.id),
                &a.label,
                sub,
                a.icon,
                a.keywords,
            );
            e.context = context;
            e
        })
        .collect();
    if context {
        let mut all = Entry::new(
            Kind::Action,
            Act::GameActions(key.to_string()),
            format!("game:{key}:actions"),
            "All actions\u{2026}",
            game_title,
            Icon::Menu,
            "menu more",
        );
        all.context = true;
        v.push(all);
    }
    v
}

/// A status entry of a game's action list.
pub(super) fn is_status(e: &Entry) -> bool {
    matches!(e.act, Act::Game(_, GameAction::Status(_)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row<'a>(key: &'a str, title: &'a str, last: Option<&'a str>, installed: bool) -> GameRow<'a> {
        GameRow {
            key,
            title,
            store_title: "",
            source: "Steam",
            playtime: "1h",
            installed,
            has_folder: installed,
            playing: false,
            favourite: false,
            hidden: false,
            last_played: last,
        }
    }

    #[test]
    fn static_entries_list_every_page_section_and_action() {
        let v = static_entries();
        let pages: Vec<_> = v.iter().filter(|e| e.kind == Kind::Page && !e.secondary).map(|e| e.title.to_string()).collect();
        assert_eq!(pages, ["Collection", "Downloads", "Wine Manager", "Epic Games", "Emulators", "Statistics", "Settings"]);
        let sections = v.iter().filter(|e| matches!(e.act, Act::Section(_))).count();
        assert_eq!(sections, Section::ALL.len());
        assert!(v.iter().filter(|e| matches!(e.act, Act::Section(_))).all(|e| e.secondary));
        for a in [
            Act::AddGame,
            Act::Refresh,
            Act::ScanRoms,
            Act::EpicUpdates,
            Act::OpenLogs,
            Act::YearReview,
            Act::ManageStatuses,
            Act::Console,
            Act::Quit,
        ] {
            assert!(v.iter().any(|e| e.act == a && e.kind == Kind::Action), "{a:?}");
        }
    }

    #[test]
    fn ids_are_unique_and_stable() {
        let v = static_entries();
        let mut ids: Vec<_> = v.iter().map(|e| e.id.to_string()).collect();
        assert!(ids.contains(&"go:Downloads".to_string()));
        assert!(ids.contains(&"section:Appearance".to_string()));
        assert!(ids.contains(&"act:quit".to_string()));
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), v.len());
    }

    #[test]
    fn shortcuts_shown_are_the_ones_that_exist() {
        let v = static_entries();
        let sc = |a: Act| v.iter().find(|e| e.act == a).and_then(|e| e.shortcut);
        assert_eq!(sc(Act::Go(Page::Collection)), Some("Ctrl 1"));
        assert_eq!(sc(Act::Go(Page::Emulators)), Some("Ctrl 5"));
        assert_eq!(sc(Act::Go(Page::Statistics)), Some("Ctrl 6"));
        assert_eq!(sc(Act::Go(Page::Settings)), Some("Ctrl ,"));
        assert_eq!(sc(Act::AddGame), Some("Ctrl N"));
        assert_eq!(sc(Act::Refresh), Some("F5"));
    }

    #[test]
    fn game_entries_fold_titles_and_keep_the_store_title_as_alt() {
        let mut r = row("steam_1", "Pok\u{e9}mon Legends", None, true);
        r.store_title = "POKEMON LEGENDS: ARCEUS";
        let e = game_entry(&r);
        assert_eq!(e.f_title, "pokemon legends");
        assert_eq!(e.f_alt, "pokemon legends: arceus");
        assert_eq!(e.act, Act::SelectGame("steam_1".into()));
        assert_eq!(e.id.as_ref(), "game:steam_1");
        assert_eq!(e.subtitle.as_ref(), "Steam \u{b7} 1h");
    }

    fn titles(v: &[Entry]) -> Vec<String> {
        v.iter().map(|e| e.title.to_string()).collect()
    }

    #[test]
    fn verbs_depend_on_the_game_state() {
        let installed = GameBits { key: "k".into(), installed: true, has_folder: true, ..Default::default() };
        let v = game_verbs("Control", &installed);
        assert_eq!(
            titles(&v),
            [
                "Play Control",
                "Control settings",
                "Open Control folder",
                "Add Control to favourites",
                "Hide Control",
                "Control log",
                "All actions for Control\u{2026}"
            ]
        );
        assert_eq!(v[0].act, Act::Game("k".into(), GameAction::Primary));
        assert_eq!(v[1].act, Act::Game("k".into(), GameAction::Settings));
        assert_eq!(v[2].act, Act::Game("k".into(), GameAction::OpenFolder));
        assert_eq!(v[6].act, Act::GameActions("k".into()));

        let not = GameBits { key: "k".into(), ..Default::default() };
        let v = game_verbs("Control", &not);
        assert_eq!(v[0].title.as_ref(), "Install Control");
        assert!(!titles(&v).iter().any(|t| t.contains("settings") || t.contains("folder")));
    }

    #[test]
    fn a_running_game_offers_stop_not_play() {
        let running = GameBits { key: "k".into(), installed: true, has_folder: true, playing: true, ..Default::default() };
        let v = game_verbs("Control", &running);
        assert_eq!(v[0].title.as_ref(), "Stop Control");
        assert!(!titles(&v).iter().any(|t| t.starts_with("Play ") || t.ends_with(" settings")));
        let e = game_entry(&GameRow { playing: true, ..row("k", "Control", None, true) });
        assert!(e.subtitle.contains("Playing now"));
    }

    #[test]
    fn view_entries_follow_the_ui_state() {
        let ui = UiState { sort: SortMode::Playtime, group_by_status: true, ..Default::default() };
        let v = view_entries(&ui, &FacetOptions::default());
        let get = |id: &str| v.iter().find(|e| e.id.as_ref() == id).unwrap();
        assert!(get("view:sort:playtime").disabled.is_some());
        assert!(get("view:sort:title").disabled.is_none());
        assert!(get("view:show:all").disabled.is_some());
        assert_eq!(get("view:group").title.as_ref(), "Stop grouping by status");
        assert_eq!(get("view:hidden").title.as_ref(), "Show hidden games");
        assert!(get("view:clear").disabled.is_some());
    }

    #[test]
    fn page_commands_carry_their_page_and_reason() {
        let c = PageCommand::new("pause", "Pause downloads", "Control", Icon::Pause)
            .keywords("hold")
            .disabled(Some("Nothing is downloading"))
            .stay();
        let e = page_entry(Page::Downloads, &c);
        assert_eq!(e.act, Act::PageCmd { page: Page::Downloads, id: "pause".into(), navigate: false });
        assert_eq!(e.id.as_ref(), "page:Downloads:pause");
        assert_eq!(e.disabled.as_deref(), Some("Nothing is downloading"));
        assert!(e.f_keywords.contains("downloads") && e.f_keywords.contains("hold"));
    }

    #[test]
    fn context_actions_end_with_all_actions() {
        let items = [
            GameActionItem::new(GameAction::Primary, "primary", "Play", Icon::PlayArrow, "launch"),
            GameActionItem::new(GameAction::Favourite, "fav", "Add to favourites", Icon::FavoriteBorder, ""),
        ];
        let v = action_entries("k", "Control", &items, true);
        assert_eq!(titles(&v), ["Play", "Add to favourites", "All actions\u{2026}"]);
        assert!(v.iter().all(|e| e.context && e.subtitle.as_ref() == "Control"));
        assert_eq!(v[1].id.as_ref(), "game:k:fav");
        assert_eq!(action_entries("k", "Control", &items, false).len(), 2);
    }
}
