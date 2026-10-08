//! What the palette can find: pages, actions and games. Entries are built
//! once when the palette opens (titles folded for matching); a keystroke only
//! ranks them (see [`super::view`]).

use gpui_kit::SharedString;

use super::matching::{fold, Haystack};
use crate::icons::Icon;
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
pub enum Act {
    Go(Page),
    /// Settings, opened on this section.
    Section(Section),
    AddGame,
    Refresh,
    ScanRoms,
    EpicUpdates,
    OpenLogs,
    Quit,
    SelectGame(String),
    PlayGame(String),
    GameSettings(String),
    OpenFolder(String),
}

/// What a game entry can do besides being selected.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GameBits {
    pub key: String,
    pub installed: bool,
    pub has_folder: bool,
    /// ISO timestamp (sorts as text); `None` = never played.
    pub last_played: Option<String>,
}

#[derive(Clone, Debug)]
pub struct Entry {
    pub kind: Kind,
    pub act: Act,
    pub title: SharedString,
    pub subtitle: SharedString,
    pub icon: Icon,
    pub shortcut: Option<&'static str>,
    /// Only listed when something is typed or the Pages filter is on
    /// (the Settings sections).
    pub secondary: bool,
    pub game: Option<GameBits>,
    // Folded search texts.
    pub f_title: String,
    pub f_alt: String,
    pub f_keywords: String,
}

impl Entry {
    pub fn haystack(&self) -> Haystack<'_> {
        Haystack { title: &self.f_title, alt: &self.f_alt, keywords: &self.f_keywords }
    }

    fn new(kind: Kind, act: Act, title: &str, subtitle: &str, icon: Icon, shortcut: Option<&'static str>, keywords: &str) -> Entry {
        Entry {
            kind,
            act,
            title: title.to_string().into(),
            subtitle: subtitle.to_string().into(),
            icon,
            shortcut,
            secondary: false,
            game: None,
            f_title: fold(title),
            f_alt: String::new(),
            f_keywords: fold(keywords),
        }
    }
}

/// Settings sections the palette opens directly: (section, one-line help).
fn section_help(s: Section) -> &'static str {
    match s {
        Section::General => "Folders, tray, Wine and Proton lookup",
        Section::GameDefaults => "Values new games start with",
        Section::Library => "Hidden games, favourites, recent games",
        Section::Downloads => "Epic downloads and updates",
        Section::Advanced => "Folders, cache, copy settings",
        Section::Logs => "Logs of this app and your games",
        Section::About => "Version, data folders, system",
    }
}

/// Pages and actions (no games): the same list on every open.
pub fn static_entries() -> Vec<Entry> {
    let mut v = Vec::new();
    let page = |p: Page, sub: &str, shortcut: &'static str, kw: &str| Entry::new(Kind::Page, Act::Go(p), p.title(), sub, p.icon(), Some(shortcut), kw);
    v.push(page(Page::Collection, "Your games", "Ctrl 1", "library home games list"));
    v.push(page(Page::Downloads, "Epic installs and updates", "Ctrl 2", "queue install update epic"));
    v.push(page(Page::WineManager, "Download and manage Proton / Wine", "Ctrl 3", "proton wine runner compatibility"));
    v.push(page(Page::Epic, "Account and library", "Ctrl 4", "epic games store account login legendary"));
    v.push(page(Page::Emulators, "Emulators and ROM folders", "Ctrl 5", "emulator rom retro console"));
    v.push(page(Page::Settings, "Global settings", "Ctrl ,", "preferences options config"));
    for s in Section::ALL {
        let mut e = Entry::new(
            Kind::Page,
            Act::Section(s),
            &format!("Settings \u{203a} {}", s.label()),
            section_help(s),
            Page::Settings.icon(),
            None,
            "preferences options config",
        );
        e.secondary = true;
        v.push(e);
    }
    let action = |a: Act, title: &str, sub: &str, icon: Icon, shortcut: Option<&'static str>, kw: &str| Entry::new(Kind::Action, a, title, sub, icon, shortcut, kw);
    v.push(action(Act::AddGame, "Add game\u{2026}", "Sideload a program, browser game or ROM", Icon::Add, Some("Ctrl N"), "new sideload import"));
    v.push(action(Act::Refresh, "Refresh library and scan ROMs", "Reload the library and import new ROMs", Icon::Refresh, Some("F5"), "reload update rescan"));
    v.push(action(Act::ScanRoms, "Scan ROMs\u{2026}", "Add games from a ROM folder", Icon::FolderOpen, None, "emulation import folder"));
    v.push(action(Act::EpicUpdates, "Check for Epic updates", "Refresh the Epic library and look for updates", Icon::Update, None, "epic games legendary upgrade"));
    v.push(action(Act::OpenLogs, "Open logs folder", "Heroic and game logs", Icon::Description, None, "log folder directory debug"));
    v.push(action(Act::Quit, "Quit", "Close Heronite", Icon::Logout, None, "exit close"));
    v
}

/// Library facts the palette needs of one game.
pub struct GameRow<'a> {
    pub key: &'a str,
    pub title: &'a str,
    /// The store's own title when it differs (alternative name).
    pub store_title: &'a str,
    pub source: &'a str,
    pub playtime: &'a str,
    pub installed: bool,
    pub has_folder: bool,
    pub last_played: Option<&'a str>,
}

pub fn game_entry(g: &GameRow) -> Entry {
    let mut e = Entry::new(
        Kind::Game,
        Act::SelectGame(g.key.to_string()),
        g.title,
        &format!("{} \u{b7} {}", g.source, g.playtime),
        Icon::SportsEsports,
        None,
        "",
    );
    if !g.store_title.is_empty() && g.store_title != g.title {
        e.f_alt = fold(g.store_title);
    }
    e.game = Some(GameBits {
        key: g.key.to_string(),
        installed: g.installed,
        has_folder: g.has_folder,
        last_played: g.last_played.map(str::to_string),
    });
    e
}

/// The verbs offered for a game: "Play X", "X settings", "Open X folder".
/// Only the ones that make sense for its state.
pub fn game_verbs(title: &str, bits: &GameBits) -> Vec<Entry> {
    let mut v = Vec::new();
    let mk = |act: Act, text: String, sub: &str, icon: Icon| {
        let mut e = Entry::new(Kind::Game, act, &text, sub, icon, None, "");
        e.game = Some(bits.clone());
        e
    };
    if bits.installed {
        v.push(mk(Act::PlayGame(bits.key.clone()), format!("Play {title}"), "Launch the game", Icon::PlayArrow));
        v.push(mk(Act::GameSettings(bits.key.clone()), format!("{title} settings"), "Wine, launch options, art\u{2026}", Icon::Tune));
    }
    if bits.has_folder {
        v.push(mk(Act::OpenFolder(bits.key.clone()), format!("Open {title} folder"), "Install folder in the file manager", Icon::FolderOpen));
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row<'a>(key: &'a str, title: &'a str, last: Option<&'a str>, installed: bool) -> GameRow<'a> {
        GameRow { key, title, store_title: "", source: "Steam", playtime: "1h", installed, has_folder: installed, last_played: last }
    }

    #[test]
    fn static_entries_list_every_page_section_and_action() {
        let v = static_entries();
        let pages: Vec<_> = v.iter().filter(|e| e.kind == Kind::Page && !e.secondary).map(|e| e.title.to_string()).collect();
        assert_eq!(pages, ["Collection", "Downloads", "Wine Manager", "Epic Games", "Emulators", "Settings"]);
        let sections = v.iter().filter(|e| matches!(e.act, Act::Section(_))).count();
        assert_eq!(sections, Section::ALL.len());
        assert!(v.iter().filter(|e| matches!(e.act, Act::Section(_))).all(|e| e.secondary));
        for a in [Act::AddGame, Act::Refresh, Act::ScanRoms, Act::EpicUpdates, Act::OpenLogs, Act::Quit] {
            assert!(v.iter().any(|e| e.act == a && e.kind == Kind::Action), "{a:?}");
        }
    }

    #[test]
    fn shortcuts_shown_are_the_ones_that_exist() {
        let v = static_entries();
        let sc = |a: Act| v.iter().find(|e| e.act == a).and_then(|e| e.shortcut);
        assert_eq!(sc(Act::Go(Page::Collection)), Some("Ctrl 1"));
        assert_eq!(sc(Act::Go(Page::Emulators)), Some("Ctrl 5"));
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
        assert_eq!(e.subtitle.as_ref(), "Steam \u{b7} 1h");
    }

    #[test]
    fn verbs_depend_on_the_game_state() {
        let installed = GameBits { key: "k".into(), installed: true, has_folder: true, last_played: None };
        let v = game_verbs("Control", &installed);
        let t: Vec<_> = v.iter().map(|e| e.title.to_string()).collect();
        assert_eq!(t, ["Play Control", "Control settings", "Open Control folder"]);
        assert_eq!(v[0].act, Act::PlayGame("k".into()));
        assert_eq!(v[1].act, Act::GameSettings("k".into()));
        assert_eq!(v[2].act, Act::OpenFolder("k".into()));
        let not = GameBits { key: "k".into(), installed: false, has_folder: false, last_played: None };
        assert!(game_verbs("Control", &not).is_empty());
    }
}
