//! What the palette means in the quick launcher (`crate::quick_launch`),
//! the same view in a small window of its own: which items it runs by
//! itself, with no main window, and which open Heronite to run there.

use super::sources::{game_verbs, Act, Entry, GameBits};
use crate::collection::game_actions::GameAction;

/// What choosing an item of the quick launcher asks for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Chosen {
    /// Play the installed game under the key.
    Play(String),
    /// Close the running game under the key.
    Stop(String),
    OpenFolder(String),
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
        Act::Game(k, GameAction::OpenFolder) => Chosen::OpenFolder(key(k)),
        Act::Game(k, GameAction::Favourite) => Chosen::Favourite(key(k)),
        Act::Game(k, GameAction::Hide) => Chosen::Hide(key(k)),
        Act::OpenLogs => Chosen::OpenLogs,
        Act::Quit => Chosen::Quit,
        act => Chosen::InApp(act.clone()),
    }
}

/// What Enter on `e` runs in the quick launcher: a game of the list is
/// played (stopped while it runs); one to install is shown in Heronite.
pub(crate) fn enter_act(e: &Entry) -> Act {
    match (&e.act, &e.game) {
        (Act::SelectGame(k), Some(b)) if b.installed || b.playing => Act::Game(k.clone(), GameAction::Primary),
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
/// folder, favourites, hide, log), and showing it in Heronite where the
/// Collection's full list is.
pub(crate) fn game_level(title: &str, bits: &GameBits) -> Vec<Entry> {
    let mut v: Vec<Entry> = game_verbs(title, bits).into_iter().filter(|e| !matches!(e.act, Act::GameActions(_))).collect();
    v.push(super::sources::show_in_app_entry(title, bits));
    v
}

#[cfg(test)]
mod tests {
    use super::super::sources::Kind;
    use super::*;
    use crate::icons::Icon;

    fn bits(installed: bool, playing: bool) -> GameBits {
        GameBits { key: "k".into(), installed, has_folder: installed, playing, ..Default::default() }
    }

    fn row(installed: bool, playing: bool) -> Entry {
        let mut e = Entry::new(Kind::Game, Act::SelectGame("k".into()), "game:k", "Control", "", Icon::SportsEsports, "");
        e.game = Some(bits(installed, playing));
        e
    }

    #[test]
    fn enter_on_a_game_plays_stops_or_opens_heronite_to_install() {
        let run = |e: &Entry| chosen(&enter_act(e), e.game.as_ref());
        assert_eq!(run(&row(true, false)), Chosen::Play("k".into()));
        assert_eq!(run(&row(true, true)), Chosen::Stop("k".into()));
        assert_eq!(run(&row(false, false)), Chosen::InApp(Act::SelectGame("k".into())));
        assert!(runs_alone(&row(true, false)) && !runs_alone(&row(false, false)));
        let install = Act::Game("k".into(), GameAction::Primary);
        assert_eq!(chosen(&install, Some(&bits(false, false))), Chosen::InApp(install.clone()));
    }

    #[test]
    fn only_what_needs_no_window_runs_alone() {
        let b = bits(true, false);
        let verbs = game_level("Control", &b);
        let alone: Vec<_> = verbs.iter().filter(|e| runs_alone(e)).map(|e| e.title.to_string()).collect();
        assert_eq!(alone, ["Play Control", "Open Control folder", "Add Control to favourites", "Hide Control"]);
        let opens: Vec<_> = verbs.iter().filter(|e| !runs_alone(e)).map(|e| e.title.to_string()).collect();
        assert_eq!(opens, ["Control settings", "Control log", "Show Control in Heronite"]);
    }
}
