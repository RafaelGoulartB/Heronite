//! What the palette means in the quick launcher (`crate::quick_launch`),
//! the same view in a small window of its own: which items it runs by
//! itself, with no main window, and which open Heronite to run there.

use heronite_data::models::CompletionStatus;
use heronite_i18n::tr;

use super::sources::{game_verbs, Act, Entry, GameBits, Kind};
use crate::collection::game_actions::GameAction;
use crate::icons::Icon;

/// What choosing an item of the quick launcher asks for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Chosen {
    /// Play the installed game under the key.
    Play(String),
    /// Close the running game under the key.
    Stop(String),
    /// Give the focus to the running game's window; its actions when it
    /// cannot be found.
    BackToGame(String),
    OpenFolder(String),
    /// Open the game's store page in the browser.
    StorePage(String),
    /// Set the game's status (`CompletionStatus::id`).
    Status(String, String),
    Favourite(String),
    Hide(String),
    OpenLogs,
    Quit,
    /// Anything else: Heronite opens and runs it as Ctrl K would.
    InApp(Act),
}

/// What `act` asks for in the quick launcher; `bits`: the game it is about.
pub(crate) fn chosen(act: &Act, bits: Option<&GameBits>) -> Chosen {
    let key = |k: &String| k.clone();
    match act {
        Act::Game(k, GameAction::Primary) => match bits {
            Some(b) if b.playing => Chosen::Stop(key(k)),
            Some(b) if b.installed => Chosen::Play(key(k)),
            // Installing asks questions: in the window.
            _ => Chosen::InApp(act.clone()),
        },
        Act::Game(k, GameAction::BackToGame) => Chosen::BackToGame(key(k)),
        Act::Game(k, GameAction::OpenFolder) => Chosen::OpenFolder(key(k)),
        Act::Game(k, GameAction::Details) => Chosen::StorePage(key(k)),
        Act::Game(k, GameAction::Status(id)) => Chosen::Status(key(k), id.clone()),
        Act::Game(k, GameAction::Favourite) => Chosen::Favourite(key(k)),
        Act::Game(k, GameAction::Hide) => Chosen::Hide(key(k)),
        Act::OpenLogs => Chosen::OpenLogs,
        Act::Quit => Chosen::Quit,
        act => Chosen::InApp(act.clone()),
    }
}

/// Whether the running game's window can be looked for here, to go back to
/// it (`heronite_overlay::game_window`).
pub(crate) fn can_focus_games() -> bool {
    heronite_overlay::game_window::supported()
}

/// What Enter on `e` runs in the quick launcher: a game of the list is
/// played; one to install is shown in Heronite. A running game is never
/// stopped by Enter: it goes back to the game where its window can be
/// looked for, else opens its actions (Stop is one of them).
pub(crate) fn enter_act(e: &Entry) -> Act {
    enter_act_with(e, can_focus_games())
}

/// [`enter_act`], `can_focus`: whether the game's window can be looked for.
pub(crate) fn enter_act_with(e: &Entry, can_focus: bool) -> Act {
    match (&e.act, &e.game) {
        (Act::SelectGame(k), Some(b)) if b.playing && can_focus => Act::Game(k.clone(), GameAction::BackToGame),
        (Act::SelectGame(k), Some(b)) if b.playing => Act::GameActions(k.clone()),
        (Act::SelectGame(k), Some(b)) if b.installed => Act::Game(k.clone(), GameAction::Primary),
        (act, _) => act.clone(),
    }
}

/// Whether the quick launcher runs `e` without opening Heronite (the others
/// are marked ↗). A game's action list stays in the launcher.
pub(crate) fn runs_alone(e: &Entry) -> bool {
    let act = enter_act(e);
    matches!(act, Act::GameActions(_)) || !matches!(chosen(&act, e.game.as_ref()), Chosen::InApp(_))
}

/// A game's actions in the quick launcher: its verbs (Play, settings,
/// folder, store page, favourites, hide, log), showing it in Heronite where
/// the Collection's full list is, and its other statuses.
///
/// While it runs, "Back to {game}" comes first where its window can be
/// looked for (`can_focus`), and Stop comes after the other verbs: Enter on
/// a running game opens this list where it cannot, and a second Enter out
/// of habit must not close the game.
pub(crate) fn game_level(title: &str, bits: &GameBits, statuses: &[CompletionStatus], can_focus: bool) -> Vec<Entry> {
    let mk = |act: GameAction, id: &str, text: String, sub: &str, icon: Icon| {
        let mut e = Entry::new(Kind::Game, Act::Game(bits.key.clone(), act), format!("game:{}:{id}", bits.key), &text, sub, icon, "");
        e.game = Some(bits.clone());
        e
    };
    let mut v: Vec<Entry> = game_verbs(title, bits).into_iter().filter(|e| !matches!(e.act, Act::GameActions(_))).collect();
    if bits.store_page {
        let at = v.iter().position(|e| matches!(e.act, Act::Game(_, GameAction::Favourite))).unwrap_or(v.len());
        let page = mk(GameAction::Details, "details", tr!("Open {title} store page", title), tr!("In the browser"), Icon::OpenInNew);
        v.insert(at, page);
    }
    if bits.playing {
        let stop = v.remove(0);
        let log = v.iter().position(|e| matches!(e.act, Act::Game(_, GameAction::Log))).map_or(v.len(), |i| i + 1);
        v.insert(log, stop);
        if can_focus {
            let back =
                mk(GameAction::BackToGame, "back", tr!("Back to {title}", title), tr!("Bring its window to the front"), Icon::ArrowForward);
            v.insert(0, back);
        }
    }
    v.push(super::sources::show_in_app_entry(title, bits));
    // Listed under "Set status": no caption of their own.
    for st in statuses.iter().filter(|st| st.id != bits.status_id) {
        let label = tr!("Status: {name}", name = heronite_data::status::status_label(&st.name));
        v.push(mk(GameAction::Status(st.id.clone()), &format!("st-{}", st.id), label, "", Icon::Flag));
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bits(installed: bool, playing: bool) -> GameBits {
        GameBits { key: "k".into(), installed, has_folder: installed, playing, ..Default::default() }
    }

    fn row(installed: bool, playing: bool) -> Entry {
        let mut e = Entry::new(Kind::Game, Act::SelectGame("k".into()), "game:k", "Control", "", Icon::SportsEsports, "");
        e.game = Some(bits(installed, playing));
        e
    }

    fn titles(v: &[Entry]) -> Vec<String> {
        v.iter().map(|e| e.title.to_string()).collect()
    }

    #[test]
    fn enter_on_a_game_plays_or_opens_heronite_to_install() {
        let run = |e: &Entry| chosen(&enter_act_with(e, true), e.game.as_ref());
        assert_eq!(run(&row(true, false)), Chosen::Play("k".into()));
        assert_eq!(run(&row(false, false)), Chosen::InApp(Act::SelectGame("k".into())));
        assert!(runs_alone(&row(true, false)) && !runs_alone(&row(false, false)));
        let install = Act::Game("k".into(), GameAction::Primary);
        assert_eq!(chosen(&install, Some(&bits(false, false))), Chosen::InApp(install.clone()));
    }

    #[test]
    fn enter_on_a_running_game_goes_back_to_it_or_opens_its_actions_never_stops_it() {
        let running = row(true, true);
        // Where its window can be looked for: back to the game.
        let act = enter_act_with(&running, true);
        assert_eq!(act, Act::Game("k".into(), GameAction::BackToGame));
        assert_eq!(chosen(&act, running.game.as_ref()), Chosen::BackToGame("k".into()));
        // Where it cannot (KDE, GNOME on Wayland): its actions, where Stop is.
        assert_eq!(enter_act_with(&running, false), Act::GameActions("k".into()));
        assert!(runs_alone(&running), "neither opens Heronite");
    }

    #[test]
    fn only_what_needs_no_window_runs_alone() {
        let b = GameBits { store_page: true, ..bits(true, false) };
        let verbs = game_level("Control", &b, &[], true);
        let alone: Vec<_> = verbs.iter().filter(|e| runs_alone(e)).map(|e| e.title.to_string()).collect();
        assert_eq!(alone, ["Play Control", "Open Control folder", "Open Control store page", "Add Control to favourites", "Hide Control"]);
        let opens: Vec<_> = verbs.iter().filter(|e| !runs_alone(e)).map(|e| e.title.to_string()).collect();
        assert_eq!(opens, ["Control settings", "Control log", "Show Control in Heronite"]);
    }

    #[test]
    fn the_store_page_is_offered_only_when_there_is_one_and_opens_alone() {
        assert!(!titles(&game_level("Control", &bits(true, false), &[], true)).contains(&"Open Control store page".to_string()));
        let b = GameBits { store_page: true, ..bits(false, false) };
        let page = game_level("Control", &b, &[], true).into_iter().find(|e| e.title.as_ref() == "Open Control store page").unwrap();
        assert_eq!(chosen(&page.act, Some(&b)), Chosen::StorePage("k".into()));
        assert_eq!(page.id.as_ref(), "game:k:details", "the in-app palette's id, one history");
    }

    #[test]
    fn the_other_statuses_are_offered_and_set_alone() {
        let statuses = heronite_data::status::default_statuses();
        let current = statuses[0].id.clone();
        let b = GameBits { status_id: current.clone(), ..bits(true, false) };
        let level = game_level("Control", &b, &statuses, true);
        let picks: Vec<&Entry> = level.iter().filter(|e| matches!(e.act, Act::Game(_, GameAction::Status(_)))).collect();
        assert_eq!(picks.len(), statuses.len() - 1, "all but the one it has");
        assert!(picks.iter().all(|e| e.title.starts_with("Status: ") && runs_alone(e)));
        let other = &statuses[1].id;
        let pick = picks.iter().find(|e| e.id.as_ref() == format!("game:k:st-{other}")).unwrap();
        assert_eq!(chosen(&pick.act, Some(&b)), Chosen::Status("k".into(), other.clone()));
        // After the verbs and "Show in Heronite".
        let show = level.iter().position(|e| e.title.as_ref() == "Show Control in Heronite").unwrap();
        assert!(level.iter().position(|e| e.id == pick.id).unwrap() > show);
    }

    #[test]
    fn a_running_games_actions_start_with_back_to_it_and_keep_stop_further_down() {
        let level = game_level("Control", &bits(true, true), &[], true);
        assert_eq!(
            titles(&level),
            [
                "Back to Control",
                "Open Control folder",
                "Add Control to favourites",
                "Hide Control",
                "Control log",
                "Stop Control",
                "Show Control in Heronite"
            ]
        );
        assert_eq!(chosen(&level[0].act, Some(&bits(true, true))), Chosen::BackToGame("k".into()));
        // Where its window cannot be looked for, the first one is harmless too.
        let level = game_level("Control", &bits(true, true), &[], false);
        assert_eq!(level[0].title.as_ref(), "Open Control folder");
        assert!(!titles(&level).contains(&"Back to Control".to_string()));
    }

    #[test]
    fn a_game_that_starts_keeps_its_play_row_now_to_stop_it() {
        let primary = |playing| {
            game_level("Control", &bits(true, playing), &[], true).into_iter().find(|e| e.id.as_ref() == "game:k:primary").unwrap()
        };
        let (before, after) = (primary(false), primary(true));
        // The same row (the launcher keeps it selected by its id), now Stop.
        assert_eq!((before.title.as_ref(), after.title.as_ref()), ("Play Control", "Stop Control"));
        assert_eq!(chosen(&after.act, Some(&bits(true, true))), Chosen::Stop("k".into()));
        assert_eq!(chosen(&before.act, Some(&bits(true, false))), Chosen::Play("k".into()));
    }
}
