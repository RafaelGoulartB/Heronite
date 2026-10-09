//! Turns a query into the grouped result list the palette shows. Pure (no
//! GPUI), so it is unit tested.

use super::matching::{fold, score_entry, tokens, TIER_WORD};
use super::sources::{game_verbs, Act, Entry, Kind};

/// Tab cycles through these.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Filter {
    All,
    Pages,
    Actions,
    Games,
}

impl Filter {
    pub fn next(self) -> Filter {
        match self {
            Filter::All => Filter::Pages,
            Filter::Pages => Filter::Actions,
            Filter::Actions => Filter::Games,
            Filter::Games => Filter::All,
        }
    }

    pub fn prev(self) -> Filter {
        match self {
            Filter::All => Filter::Games,
            Filter::Pages => Filter::All,
            Filter::Actions => Filter::Pages,
            Filter::Games => Filter::Actions,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Filter::All => "All",
            Filter::Pages => "Pages",
            Filter::Actions => "Actions",
            Filter::Games => "Games",
        }
    }

    fn admits(self, k: Kind) -> bool {
        matches!((self, k), (Filter::All, _) | (Filter::Pages, Kind::Page) | (Filter::Actions, Kind::Action) | (Filter::Games, Kind::Game))
    }
}

/// An entry of the list handed to [`search`], or one of the verb entries made
/// for the query.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum ItemRef {
    Entry(usize),
    Verb(usize),
}

#[derive(Clone, Debug)]
pub struct Group {
    pub caption: String,
    pub items: Vec<ItemRef>,
}

#[derive(Default, Debug)]
pub(super) struct Results {
    pub groups: Vec<Group>,
    /// "Play X", "X settings", "Open X folder" made for this query.
    pub verbs: Vec<Entry>,
}

impl Results {
    /// Selectable items in display order.
    pub fn items(&self) -> impl Iterator<Item = ItemRef> + '_ {
        self.groups.iter().flat_map(|g| g.items.iter().copied())
    }

    pub fn count(&self) -> usize {
        self.groups.iter().map(|g| g.items.len()).sum()
    }
}

const ALL_PAGES: usize = 6;
const ALL_ACTIONS: usize = 5;
const ALL_GAMES: usize = 8;
const GAMES_ONLY: usize = 50;
const RECENT: usize = 5;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Verb {
    Play,
    Settings,
    Folder,
}

fn verb_of(token: &str) -> Option<Verb> {
    match token {
        "play" | "launch" | "run" => Some(Verb::Play),
        "settings" | "config" | "configure" | "options" => Some(Verb::Settings),
        "open" | "folder" => Some(Verb::Folder),
        _ => None,
    }
}

/// Ranks `entries` for `query` under `filter`.
pub fn search(entries: &[Entry], query: &str, filter: Filter) -> Results {
    let folded = fold(query);
    let toks = tokens(&folded);
    if toks.is_empty() {
        return browse(entries, filter);
    }

    // (score, index) per kind, best first; stable on ties.
    let mut pages: Vec<(i32, usize)> = Vec::new();
    let mut actions: Vec<(i32, usize)> = Vec::new();
    let mut games: Vec<(i32, usize)> = Vec::new();
    for (i, e) in entries.iter().enumerate() {
        if !filter.admits(e.kind) {
            continue;
        }
        if let Some(s) = score_entry(&toks, &e.haystack()) {
            match e.kind {
                Kind::Page => pages.push((s, i)),
                Kind::Action => actions.push((s, i)),
                Kind::Game => games.push((s, i)),
            }
        }
    }
    for v in [&mut pages, &mut actions, &mut games] {
        v.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    }

    // "play <game>", "<game> settings", "open <game> folder": only when the
    // text itself is not already a good title match.
    if matches!(filter, Filter::All | Filter::Games) && games.first().is_none_or(|(s, _)| *s < TIER_WORD) {
        if let Some(r) = verb_query(entries, &toks) {
            return r;
        }
    }

    let only = filter != Filter::All;
    let cap = |n_all: usize, n_only: usize| if only { n_only } else { n_all };
    pages.truncate(cap(ALL_PAGES, usize::MAX));
    actions.truncate(cap(ALL_ACTIONS, usize::MAX));
    games.truncate(cap(ALL_GAMES, GAMES_ONLY));

    let mut ranked: Vec<(i32, u8, &str, Vec<ItemRef>)> = Vec::new();
    let refs = |v: &[(i32, usize)]| v.iter().map(|(_, i)| ItemRef::Entry(*i)).collect::<Vec<_>>();
    if let Some((s, _)) = pages.first() {
        ranked.push((*s, 0, "Go to", refs(&pages)));
    }
    if let Some((s, _)) = actions.first() {
        ranked.push((*s, 1, "Actions", refs(&actions)));
    }
    if let Some((s, _)) = games.first() {
        ranked.push((*s, 2, "Games", refs(&games)));
    }
    // The group with the best hit first; ties keep Go to, Actions, Games.
    ranked.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));

    let mut out = Results::default();
    let games_first = ranked.first().is_some_and(|g| g.1 == 2);
    for (_, kind, caption, items) in ranked {
        out.groups.push(Group { caption: caption.to_string(), items });
        // The best game match comes with what can be done to it.
        if kind == 2 && games_first && games.first().is_some_and(|(s, _)| *s >= TIER_WORD) {
            let e = &entries[games[0].1];
            if let Some(bits) = &e.game {
                let verbs = game_verbs(&e.title, bits);
                if !verbs.is_empty() {
                    let base = out.verbs.len();
                    let items = (0..verbs.len()).map(|i| ItemRef::Verb(base + i)).collect();
                    out.verbs.extend(verbs);
                    out.groups.push(Group { caption: e.title.to_string(), items });
                }
            }
        }
    }
    out
}

/// `play <rest>`, `settings <rest>`, `open <rest>`, or the verb last.
fn verb_query(entries: &[Entry], toks: &[&str]) -> Option<Results> {
    if toks.len() < 2 {
        return None;
    }
    let (verb, rest) = match (verb_of(toks[0]), verb_of(toks[toks.len() - 1])) {
        (Some(v), _) => (v, &toks[1..]),
        (None, Some(v)) => (v, &toks[..toks.len() - 1]),
        _ => return None,
    };
    let mut hits: Vec<(i32, usize)> = entries
        .iter()
        .enumerate()
        .filter(|(_, e)| {
            e.game.as_ref().is_some_and(|g| match verb {
                Verb::Play | Verb::Settings => g.installed,
                Verb::Folder => g.has_folder,
            })
        })
        .filter_map(|(i, e)| score_entry(rest, &e.haystack()).map(|s| (s, i)))
        .collect();
    if hits.is_empty() {
        return None;
    }
    hits.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    hits.truncate(ALL_GAMES);
    let mut out = Results::default();
    let mut items = Vec::new();
    for (_, i) in hits {
        let e = &entries[i];
        let bits = e.game.as_ref()?;
        let want = |a: &Act| match verb {
            Verb::Play => matches!(a, Act::PlayGame(_)),
            Verb::Settings => matches!(a, Act::GameSettings(_)),
            Verb::Folder => matches!(a, Act::OpenFolder(_)),
        };
        if let Some(v) = game_verbs(&e.title, bits).into_iter().find(|v| want(&v.act)) {
            items.push(ItemRef::Verb(out.verbs.len()));
            out.verbs.push(v);
        }
    }
    let caption = match verb {
        Verb::Play => "Play",
        Verb::Settings => "Game settings",
        Verb::Folder => "Open folder",
    };
    out.groups.push(Group { caption: caption.to_string(), items });
    Some(out)
}

/// Nothing typed: recent games and the Go to list (or what the filter asks).
fn browse(entries: &[Entry], filter: Filter) -> Results {
    let idx = |kind: Kind, secondary: bool| -> Vec<ItemRef> {
        entries
            .iter()
            .enumerate()
            .filter(|(_, e)| e.kind == kind && (secondary || !e.secondary))
            .map(|(i, _)| ItemRef::Entry(i))
            .collect()
    };
    let games: Vec<usize> = entries.iter().enumerate().filter(|(_, e)| e.kind == Kind::Game).map(|(i, _)| i).collect();
    let recent_games: Vec<usize> = {
        let mut r: Vec<usize> = games.iter().copied().filter(|i| entries[*i].game.as_ref().is_some_and(|g| g.last_played.is_some())).collect();
        r.sort_by(|a, b| {
            let lp = |i: &usize| entries[*i].game.as_ref().and_then(|g| g.last_played.clone());
            lp(b).cmp(&lp(a)).then(a.cmp(b))
        });
        r.truncate(RECENT);
        r
    };
    let mut out = Results::default();
    let mut push = |caption: &str, items: Vec<ItemRef>| {
        if !items.is_empty() {
            out.groups.push(Group { caption: caption.to_string(), items });
        }
    };
    let recent_items: Vec<ItemRef> = recent_games.iter().map(|i| ItemRef::Entry(*i)).collect();
    match filter {
        Filter::All => {
            push("Recent", recent_items);
            push("Go to", idx(Kind::Page, false));
        }
        Filter::Pages => push("Go to", idx(Kind::Page, true)),
        Filter::Actions => push("Actions", idx(Kind::Action, true)),
        Filter::Games => {
            let rest: Vec<ItemRef> = games
                .iter()
                .filter(|i| !recent_games.contains(i))
                .take(GAMES_ONLY)
                .map(|i| ItemRef::Entry(*i))
                .collect();
            push("Recent", recent_items);
            push("Games", rest);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::super::sources::{game_entry, static_entries, GameRow};
    use super::*;

    fn game(key: &str, title: &str, last: Option<&str>, installed: bool) -> Entry {
        game_entry(&GameRow { key, title, store_title: "", source: "Steam", playtime: "1h", installed, has_folder: installed, last_played: last })
    }

    fn corpus() -> Vec<Entry> {
        let mut v = static_entries();
        v.push(game("s_control", "Control", Some("2026-05-01"), true));
        v.push(game("s_cyber", "Cyberpunk 2077", Some("2026-03-01"), true));
        v.push(game("s_eldenring", "Elden Ring", None, true));
        v.push(game("s_wine", "Wine Quest", Some("2026-04-01"), true));
        v.push(game("s_hades", "Hades", None, false));
        v
    }

    fn titles(e: &[Entry], r: &Results) -> Vec<String> {
        r.items()
            .map(|i| match i {
                ItemRef::Entry(i) => e[i].title.to_string(),
                ItemRef::Verb(i) => r.verbs[i].title.to_string(),
            })
            .collect()
    }

    #[test]
    fn empty_query_shows_recent_games_then_go_to() {
        let e = corpus();
        let r = search(&e, "", Filter::All);
        assert_eq!(r.groups[0].caption, "Recent");
        assert_eq!(titles(&e, &r)[..4], ["Control", "Wine Quest", "Cyberpunk 2077", "Collection"]);
        assert_eq!(r.groups[1].caption, "Go to");
        // Settings sections are not listed unless asked for.
        assert_eq!(r.groups[1].items.len(), 7);
    }

    #[test]
    fn empty_query_filters() {
        let e = corpus();
        let pages = search(&e, "", Filter::Pages);
        assert_eq!(pages.count(), 7 + crate::pages::settings::Section::ALL.len());
        let actions = search(&e, "", Filter::Actions);
        assert_eq!(actions.count(), 7);
        let games = search(&e, "", Filter::Games);
        assert_eq!(games.groups[0].caption, "Recent");
        assert_eq!(games.groups[1].caption, "Games");
        assert_eq!(games.count(), 5);
    }

    #[test]
    fn settings_sections_are_found_by_their_keywords() {
        let e = corpus();
        for (q, section) in [
            ("blur", "Settings \u{203a} Appearance"),
            ("igdb", "Settings \u{203a} Metadata"),
            ("ludusavi", "Settings \u{203a} Backups"),
            ("screenshots", "Settings \u{203a} Screenshots"),
        ] {
            let r = search(&e, q, Filter::All);
            let t = titles(&e, &r);
            assert!(t.contains(&section.to_string()), "{q}: {t:?}");
        }
    }

    #[test]
    fn wi_finds_the_wine_manager_and_a_game() {
        let e = corpus();
        let r = search(&e, "wi", Filter::All);
        let t = titles(&e, &r);
        assert!(t.contains(&"Wine Manager".to_string()), "{t:?}");
        assert!(t.contains(&"Wine Quest".to_string()), "{t:?}");
    }

    #[test]
    fn page_shortcuts_and_sections_are_found() {
        let e = corpus();
        let r = search(&e, "settings gen", Filter::All);
        assert_eq!(titles(&e, &r)[0], "Settings \u{203a} General");
        let r = search(&e, "dwn", Filter::All);
        assert_eq!(titles(&e, &r)[0], "Downloads");
    }

    #[test]
    fn the_group_with_the_best_hit_comes_first() {
        let e = corpus();
        let r = search(&e, "control", Filter::All);
        assert_eq!(r.groups[0].caption, "Games");
        let r = search(&e, "quit", Filter::All);
        assert_eq!(r.groups[0].caption, "Actions");
    }

    #[test]
    fn best_game_comes_with_play_settings_and_folder() {
        let e = corpus();
        let r = search(&e, "control", Filter::All);
        assert_eq!(r.groups[1].caption, "Control");
        let t = titles(&e, &r);
        assert!(t.contains(&"Play Control".to_string()));
        assert!(t.contains(&"Control settings".to_string()));
        assert!(t.contains(&"Open Control folder".to_string()));
    }

    #[test]
    fn a_game_that_is_not_installed_has_no_verbs() {
        let e = corpus();
        let r = search(&e, "hades", Filter::All);
        assert_eq!(r.groups.len(), 1);
        assert!(r.verbs.is_empty());
    }

    #[test]
    fn leading_and_trailing_verbs() {
        let e = corpus();
        let r = search(&e, "play elden", Filter::All);
        assert_eq!(r.groups.len(), 1);
        assert_eq!(titles(&e, &r), ["Play Elden Ring"]);
        assert_eq!(r.verbs[0].act, Act::PlayGame("s_eldenring".into()));
        let r = search(&e, "cyber settings", Filter::All);
        assert_eq!(titles(&e, &r), ["Cyberpunk 2077 settings"]);
        let r = search(&e, "open control", Filter::All);
        assert_eq!(titles(&e, &r), ["Open Control folder"]);
        // Not installed: nothing to play.
        let r = search(&e, "play hades", Filter::All);
        assert!(!titles(&e, &r).iter().any(|t| t.starts_with("Play ")));
    }

    #[test]
    fn a_title_that_starts_with_a_verb_still_matches_as_a_title() {
        let mut e = corpus();
        e.push(game("s_ow", "Open World Racer", None, true));
        let r = search(&e, "open world", Filter::All);
        assert_eq!(titles(&e, &r)[0], "Open World Racer");
    }

    #[test]
    fn filters_restrict_the_kinds() {
        let e = corpus();
        let r = search(&e, "e", Filter::Games);
        assert!(r.groups.iter().all(|g| g.caption != "Go to" && g.caption != "Actions"));
        let r = search(&e, "e", Filter::Pages);
        assert!(r.items().all(|i| matches!(i, ItemRef::Entry(i) if e[i].kind == Kind::Page)));
    }

    #[test]
    fn ranking_is_stable() {
        let e = corpus();
        let a = titles(&e, &search(&e, "e", Filter::All));
        let b = titles(&e, &search(&e, "e", Filter::All));
        assert_eq!(a, b);
    }

    #[test]
    fn nothing_matches_nothing() {
        let e = corpus();
        let r = search(&e, "zzzzqq", Filter::All);
        assert_eq!(r.count(), 0);
    }

    #[test]
    fn filter_cycles_through_all_four() {
        let mut f = Filter::All;
        let mut seen = vec![f];
        for _ in 0..4 {
            f = f.next();
            seen.push(f);
        }
        assert_eq!(seen, [Filter::All, Filter::Pages, Filter::Actions, Filter::Games, Filter::All]);
        assert_eq!(Filter::All.prev(), Filter::Games);
    }
}
