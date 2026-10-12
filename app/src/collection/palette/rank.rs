//! Turns a query into the grouped result list the palette shows. Pure (no
//! GPUI), so it is unit tested.
//!
//! * [`search`]: the main list (pages, actions, the selected game's actions,
//!   games, and "play x" style verbs).
//! * [`actions_list`]: the second level, every action of one game.
//!
//! Both take the palette's history (ids, most recent first): items run
//! lately come first inside their tier and fill the "Recent" group.

use heronite_i18n::tr;

use super::matching::{fold, score_entry, tokens, TIER_SUBSTR, TIER_WORD};
use super::sources::{game_verbs, is_main_verb, is_status, Act, Entry, Kind};

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
            Filter::All => tr!("All"),
            Filter::Pages => tr!("Pages"),
            Filter::Actions => tr!("Actions"),
            Filter::Games => tr!("Games"),
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
    /// "Play X", "X settings", "Open X folder"... made for this query.
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

    fn push(&mut self, caption: &str, items: Vec<ItemRef>) {
        if !items.is_empty() {
            self.groups.push(Group { caption: caption.to_string(), items });
        }
    }
}

const ALL_PAGES: usize = 6;
const ALL_ACTIONS: usize = 5;
const ALL_CONTEXT: usize = 4;
const ALL_GAMES: usize = 8;
const GAMES_ONLY: usize = 50;
const RECENT: usize = 5;
/// Actions of the selected game listed when nothing is typed (then "All
/// actions").
const CONTEXT_BROWSE: usize = 4;
/// Boost of the last item run (less for older ones). It orders items inside
/// a tier and never lifts one over a tier.
const RECENT_BOOST: i32 = 40;
/// What can't run now comes after what can.
const DISABLED_PENALTY: i32 = 30;

fn boost(e: &Entry, recent: &[String]) -> i32 {
    let mut b = 0;
    if let Some(pos) = recent.iter().position(|r| r.as_str() == e.id.as_ref()) {
        b += (RECENT_BOOST - 4 * pos as i32).max(0);
    }
    if e.disabled.is_some() {
        b -= DISABLED_PENALTY;
    }
    b
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Verb {
    Play,
    Stop,
    Install,
    Settings,
    Folder,
    Favourite,
    Hide,
    Log,
    Actions,
}

impl Verb {
    fn of(token: &str) -> Option<Verb> {
        Some(match token {
            "play" | "launch" | "run" | "jogar" | "iniciar" | "executar" => Verb::Play,
            "stop" | "kill" | "close" | "parar" | "encerrar" | "fechar" => Verb::Stop,
            "install" | "download" | "instalar" | "baixar" => Verb::Install,
            "settings" | "config" | "configure" | "options" | "configuracoes" | "configuracao" | "configurar" | "opcoes" => Verb::Settings,
            "open" | "folder" | "abrir" | "pasta" => Verb::Folder,
            "fav" | "favourite" | "favorite" | "unfavourite" | "unfavorite" | "favorito" | "favoritar" | "desfavoritar" => Verb::Favourite,
            "hide" | "unhide" | "ocultar" | "mostrar" | "esconder" => Verb::Hide,
            "log" | "logs" | "registro" => Verb::Log,
            "actions" | "menu" | "acoes" => Verb::Actions,
            _ => return None,
        })
    }

    /// The verb entry (`game_verbs`) it picks, by the end of its id.
    fn id(self) -> &'static str {
        match self {
            Verb::Play | Verb::Stop | Verb::Install => "primary",
            Verb::Settings => "settings",
            Verb::Folder => "folder",
            Verb::Favourite => "fav",
            Verb::Hide => "hide",
            Verb::Log => "log",
            Verb::Actions => "actions",
        }
    }

    fn caption(self) -> &'static str {
        match self {
            Verb::Play => tr!("Play"),
            Verb::Stop => tr!("Stop"),
            Verb::Install => tr!("Install"),
            Verb::Settings => tr!("Game settings"),
            Verb::Folder => tr!("Open folder"),
            Verb::Favourite => tr!("Favourites"),
            Verb::Hide => tr!("Hide"),
            Verb::Log => tr!("Logs"),
            Verb::Actions => tr!("Actions"),
        }
    }

    fn admits(self, g: &super::sources::GameBits) -> bool {
        match self {
            Verb::Play => g.installed && !g.playing,
            Verb::Stop => g.playing,
            Verb::Install => !g.installed && !g.playing,
            Verb::Settings => g.installed && !g.playing,
            Verb::Folder => g.has_folder,
            Verb::Favourite | Verb::Hide | Verb::Log | Verb::Actions => true,
        }
    }
}

/// Ranks `entries` for `query` under `filter`; `recent` is the history.
pub fn search(entries: &[Entry], query: &str, filter: Filter, recent: &[String]) -> Results {
    let folded = fold(query);
    let toks = tokens(&folded);
    if toks.is_empty() {
        return browse(entries, filter, recent);
    }

    // (score, index) per group, best first; stable on ties.
    let mut pages: Vec<(i32, usize)> = Vec::new();
    let mut actions: Vec<(i32, usize)> = Vec::new();
    let mut context: Vec<(i32, usize)> = Vec::new();
    let mut games: Vec<(i32, usize)> = Vec::new();
    for (i, e) in entries.iter().enumerate() {
        if !filter.admits(e.kind) {
            continue;
        }
        if let Some(s) = score_entry(&toks, &e.haystack()) {
            // Facet values (hundreds of names) only by their text, never by
            // scattered letters.
            if e.search_only && s < TIER_SUBSTR - 100 {
                continue;
            }
            let s = s + boost(e, recent);
            match e.kind {
                Kind::Page => pages.push((s, i)),
                Kind::Action if e.context => context.push((s, i)),
                Kind::Action => actions.push((s, i)),
                Kind::Game => games.push((s, i)),
            }
        }
    }
    for v in [&mut pages, &mut actions, &mut context, &mut games] {
        v.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    }

    // "play <game>", "<game> settings", "hide <game>"...: only when the text
    // itself is not already a good title match.
    if matches!(filter, Filter::All | Filter::Games) && games.first().is_none_or(|(s, _)| *s < TIER_WORD) {
        if let Some(r) = verb_query(entries, &toks) {
            return r;
        }
    }

    let only = filter != Filter::All;
    let cap = |n_all: usize, n_only: usize| if only { n_only } else { n_all };
    pages.truncate(cap(ALL_PAGES, usize::MAX));
    actions.truncate(cap(ALL_ACTIONS, usize::MAX));
    context.truncate(cap(ALL_CONTEXT, usize::MAX));
    games.truncate(cap(ALL_GAMES, GAMES_ONLY));

    let context_caption = context.first().map(|(_, i)| entries[*i].subtitle.to_string()).unwrap_or_default();
    let mut ranked: Vec<(i32, u8, &str, Vec<ItemRef>)> = Vec::new();
    let refs = |v: &[(i32, usize)]| v.iter().map(|(_, i)| ItemRef::Entry(*i)).collect::<Vec<_>>();
    if let Some((s, _)) = pages.first() {
        ranked.push((*s, 0, tr!("Go to"), refs(&pages)));
    }
    if let Some((s, _)) = context.first() {
        ranked.push((*s, 1, &context_caption, refs(&context)));
    }
    if let Some((s, _)) = actions.first() {
        ranked.push((*s, 2, tr!("Actions"), refs(&actions)));
    }
    if let Some((s, _)) = games.first() {
        ranked.push((*s, 3, tr!("Games"), refs(&games)));
    }
    // The group with the best hit first; ties keep Go to, the game's, Actions, Games.
    ranked.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));

    let mut out = Results::default();
    let games_first = ranked.first().is_some_and(|g| g.1 == 3);
    for (_, kind, caption, items) in ranked {
        out.push(caption, items);
        // The best game match comes with what can be done to it.
        if kind == 3 && games_first && games.first().is_some_and(|(s, _)| *s >= TIER_WORD) {
            let e = &entries[games[0].1];
            if let Some(bits) = &e.game {
                let verbs: Vec<Entry> = game_verbs(&e.title, bits).into_iter().filter(is_main_verb).collect();
                let base = out.verbs.len();
                let items = (0..verbs.len()).map(|i| ItemRef::Verb(base + i)).collect();
                out.verbs.extend(verbs);
                out.push(&e.title, items);
            }
        }
    }
    out
}

/// `<verb> <game>` or `<game> <verb>`.
fn verb_query(entries: &[Entry], toks: &[&str]) -> Option<Results> {
    if toks.len() < 2 {
        return None;
    }
    let (verb, rest) = match (Verb::of(toks[0]), Verb::of(toks[toks.len() - 1])) {
        (Some(v), _) => (v, &toks[1..]),
        (None, Some(v)) => (v, &toks[..toks.len() - 1]),
        _ => return None,
    };
    let mut hits: Vec<(i32, usize)> = entries
        .iter()
        .enumerate()
        .filter(|(_, e)| e.kind == Kind::Game && matches!(e.act, Act::SelectGame(_)) && e.game.as_ref().is_some_and(|g| verb.admits(g)))
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
        let id = format!("game:{}:{}", bits.key, verb.id());
        if let Some(v) = game_verbs(&e.title, bits).into_iter().find(|v| v.id.as_ref() == id) {
            items.push(ItemRef::Verb(out.verbs.len()));
            out.verbs.push(v);
        }
    }
    out.push(verb.caption(), items);
    Some(out)
}

/// The history as items: entries still offered, and game verbs made again
/// from their game's entry. Gone ones are skipped.
fn resolve_recent(entries: &[Entry], recent: &[String], out: &mut Results) -> Vec<ItemRef> {
    let mut items = Vec::new();
    for id in recent {
        if items.len() == RECENT {
            break;
        }
        if let Some(i) = entries.iter().position(|e| e.id.as_ref() == id) {
            items.push(ItemRef::Entry(i));
            continue;
        }
        let Some((key, _)) = id.strip_prefix("game:").and_then(|rest| rest.rsplit_once(':')) else { continue };
        let Some(e) = entries.iter().find(|e| matches!(&e.act, Act::SelectGame(k) if k == key)) else { continue };
        let Some(bits) = &e.game else { continue };
        if let Some(v) = game_verbs(&e.title, bits).into_iter().find(|v| v.id.as_ref() == id) {
            items.push(ItemRef::Verb(out.verbs.len()));
            out.verbs.push(v);
        }
    }
    items
}

/// Nothing typed: the selected game, recent items, recently played games and
/// the Go to list (or what the filter asks).
fn browse(entries: &[Entry], filter: Filter, recent: &[String]) -> Results {
    let idx = |pick: &dyn Fn(&Entry) -> bool| -> Vec<ItemRef> {
        entries.iter().enumerate().filter(|(_, e)| pick(e)).map(|(i, _)| ItemRef::Entry(i)).collect()
    };
    let games: Vec<usize> = entries.iter().enumerate().filter(|(_, e)| e.kind == Kind::Game).map(|(i, _)| i).collect();
    let recent_games: Vec<usize> = {
        let mut r: Vec<usize> =
            games.iter().copied().filter(|i| entries[*i].game.as_ref().is_some_and(|g| g.last_played.is_some())).collect();
        r.sort_by(|a, b| {
            let lp = |i: &usize| entries[*i].game.as_ref().and_then(|g| g.last_played.clone());
            lp(b).cmp(&lp(a)).then(a.cmp(b))
        });
        r
    };
    let context_caption = entries.iter().find(|e| e.context).map(|e| e.subtitle.to_string()).unwrap_or_default();
    let mut out = Results::default();
    match filter {
        Filter::All => {
            // The selected game: its first actions, then "All actions".
            let mut ctx: Vec<ItemRef> = entries
                .iter()
                .enumerate()
                .filter(|(_, e)| e.context && !is_status(e) && !matches!(e.act, Act::GameActions(_)))
                .take(CONTEXT_BROWSE)
                .map(|(i, _)| ItemRef::Entry(i))
                .collect();
            ctx.extend(idx(&|e| e.context && matches!(e.act, Act::GameActions(_))));
            let rec: Vec<ItemRef> = resolve_recent(entries, recent, &mut out).into_iter().filter(|r| !ctx.contains(r)).collect();
            let played: Vec<ItemRef> = recent_games.iter().map(|i| ItemRef::Entry(*i)).filter(|r| !rec.contains(r)).take(RECENT).collect();
            out.push(&context_caption, ctx);
            out.push(tr!("Recent"), rec);
            out.push(tr!("Recently played"), played);
            out.push(tr!("Go to"), idx(&|e| e.kind == Kind::Page && !e.secondary));
        }
        Filter::Pages => out.push(tr!("Go to"), idx(&|e| e.kind == Kind::Page)),
        Filter::Actions => {
            out.push(&context_caption, idx(&|e| e.context));
            out.push(tr!("Actions"), idx(&|e| e.kind == Kind::Action && !e.context && !e.search_only));
        }
        Filter::Games => {
            let shown: Vec<usize> = recent_games.iter().copied().take(RECENT).collect();
            let rest: Vec<ItemRef> = games.iter().filter(|i| !shown.contains(i)).take(GAMES_ONLY).map(|i| ItemRef::Entry(*i)).collect();
            out.push(tr!("Recently played"), shown.into_iter().map(ItemRef::Entry).collect());
            out.push(tr!("Games"), rest);
        }
    }
    out
}

/// Every action of one game (`entries`, from `action_entries`): the actions
/// then the statuses when nothing is typed, the best matches otherwise.
pub fn actions_list(entries: &[Entry], query: &str, recent: &[String], caption: &str) -> Results {
    let folded = fold(query);
    let toks = tokens(&folded);
    let mut out = Results::default();
    if toks.is_empty() {
        let all =
            |status: bool| entries.iter().enumerate().filter(|(_, e)| is_status(e) == status).map(|(i, _)| ItemRef::Entry(i)).collect();
        out.push(caption, all(false));
        out.push(tr!("Set status"), all(true));
        return out;
    }
    let mut hits: Vec<(i32, usize)> =
        entries.iter().enumerate().filter_map(|(i, e)| score_entry(&toks, &e.haystack()).map(|s| (s + boost(e, recent), i))).collect();
    hits.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    out.push(caption, hits.into_iter().map(|(_, i)| ItemRef::Entry(i)).collect());
    out
}

#[cfg(test)]
mod tests {
    use super::super::sources::{action_entries, game_entry, page_entry, static_entries, view_entries, GameRow};
    use super::*;
    use crate::collection::game_actions::{GameAction, GameActionItem};
    use crate::icons::Icon;
    use crate::pages::commands::PageCommand;
    use crate::pages::Page;

    fn game(key: &str, title: &str, last: Option<&str>, installed: bool) -> Entry {
        game_entry(&GameRow {
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
        })
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

    /// The corpus with Control selected in the Collection.
    fn with_selection() -> Vec<Entry> {
        let mut v = corpus();
        let items = [
            GameActionItem::new(GameAction::Primary, "primary", "Play", Icon::PlayArrow, "launch run start"),
            GameActionItem::new(GameAction::Favourite, "fav", "Add to favourites", Icon::FavoriteBorder, "favourite star"),
            GameActionItem::new(GameAction::OpenFolder, "folder", "Open install folder", Icon::FolderOpen, ""),
            GameActionItem::new(GameAction::Settings, "settings", "Settings", Icon::Tune, ""),
            GameActionItem::new(GameAction::LocalUninstall, "local-uninstall", "Uninstall", Icon::DeleteForever, "remove delete"),
            GameActionItem::new(GameAction::Status("beaten".into()), "st-beaten", "Status: Beaten", Icon::Flag, "status").palette_only(),
        ];
        v.extend(action_entries("s_control", "Control", &items, true));
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

    fn captions(r: &Results) -> Vec<&str> {
        r.groups.iter().map(|g| g.caption.as_str()).collect()
    }

    #[test]
    fn empty_query_shows_recently_played_games_then_go_to() {
        let e = corpus();
        let r = search(&e, "", Filter::All, &[]);
        assert_eq!(captions(&r), ["Recently played", "Go to"]);
        assert_eq!(titles(&e, &r)[..4], ["Control", "Wine Quest", "Cyberpunk 2077", "Collection"]);
        // Settings sections are not listed unless asked for.
        assert_eq!(r.groups[1].items.len(), 7);
    }

    #[test]
    fn empty_query_filters() {
        let e = corpus();
        let pages = search(&e, "", Filter::Pages, &[]);
        assert_eq!(pages.count(), 7 + crate::pages::settings::Section::ALL.len());
        let actions = search(&e, "", Filter::Actions, &[]);
        assert_eq!(actions.count(), 9);
        let games = search(&e, "", Filter::Games, &[]);
        assert_eq!(captions(&games), ["Recently played", "Games"]);
        assert_eq!(games.count(), 5);
    }

    #[test]
    fn recent_items_come_first_when_nothing_is_typed() {
        let e = corpus();
        let recent = ["act:quit".to_string(), "game:s_control:primary".into(), "game:s_gone:primary".into(), "go:Downloads".into()];
        let r = search(&e, "", Filter::All, &recent);
        assert_eq!(captions(&r), ["Recent", "Recently played", "Go to"]);
        // The verb is made again from its game; the gone game is skipped.
        assert_eq!(titles(&e, &r)[..3], ["Quit", "Play Control", "Downloads"]);
    }

    #[test]
    fn recent_items_rank_first_inside_their_tier() {
        let e = corpus();
        // "Collection" and "Console mode" both start with "co"; the shorter
        // title wins until Console mode was used.
        let pos = |t: &[String], s: &str| t.iter().position(|x| x == s).unwrap();
        let plain = titles(&e, &search(&e, "co", Filter::All, &[]));
        assert!(pos(&plain, "Collection") < pos(&plain, "Console mode"), "{plain:?}");
        let boosted = titles(&e, &search(&e, "co", Filter::All, &["act:console".into()]));
        assert!(pos(&boosted, "Console mode") < pos(&boosted, "Collection"), "{boosted:?}");
    }

    #[test]
    fn disabled_items_rank_after_enabled_ones() {
        let mut e = corpus();
        e.push(page_entry(
            Page::Downloads,
            &PageCommand::new("pause", "Pause downloads", "", Icon::Pause).disabled(Some("Nothing is downloading")),
        ));
        e.push(page_entry(Page::Downloads, &PageCommand::new("pause2", "Pause downloads now", "", Icon::Pause)));
        let r = search(&e, "pause", Filter::Actions, &[]);
        assert_eq!(titles(&e, &r)[0], "Pause downloads now");
    }

    #[test]
    fn selected_game_actions_lead_the_empty_list() {
        let e = with_selection();
        let r = search(&e, "", Filter::All, &[]);
        assert_eq!(r.groups[0].caption, "Control");
        assert_eq!(titles(&e, &r)[..5], ["Play", "Add to favourites", "Open install folder", "Settings", "All actions\u{2026}"]);
        // Statuses are left for "All actions".
        assert!(!titles(&e, &r).contains(&"Status: Beaten".to_string()));
    }

    #[test]
    fn selected_game_actions_are_found_by_their_name() {
        let e = with_selection();
        let r = search(&e, "uninstall", Filter::All, &[]);
        assert_eq!(r.groups[0].caption, "Control");
        assert_eq!(titles(&e, &r)[0], "Uninstall");
        // The game's own title does not drag its whole list in.
        let r = search(&e, "control", Filter::All, &[]);
        assert_eq!(r.groups[0].caption, "Games");
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
            let r = search(&e, q, Filter::All, &[]);
            let t = titles(&e, &r);
            assert!(t.contains(&section.to_string()), "{q}: {t:?}");
        }
    }

    #[test]
    fn wi_finds_the_wine_manager_and_a_game() {
        let e = corpus();
        let r = search(&e, "wi", Filter::All, &[]);
        let t = titles(&e, &r);
        assert!(t.contains(&"Wine Manager".to_string()), "{t:?}");
        assert!(t.contains(&"Wine Quest".to_string()), "{t:?}");
    }

    #[test]
    fn page_shortcuts_and_sections_are_found() {
        let e = corpus();
        let r = search(&e, "settings gen", Filter::All, &[]);
        assert_eq!(titles(&e, &r)[0], "Settings \u{203a} General");
        let r = search(&e, "dwn", Filter::All, &[]);
        assert_eq!(titles(&e, &r)[0], "Downloads");
    }

    #[test]
    fn the_group_with_the_best_hit_comes_first() {
        let e = corpus();
        let r = search(&e, "control", Filter::All, &[]);
        assert_eq!(r.groups[0].caption, "Games");
        let r = search(&e, "quit", Filter::All, &[]);
        assert_eq!(r.groups[0].caption, "Actions");
    }

    #[test]
    fn best_game_comes_with_play_settings_folder_and_all_actions() {
        let e = corpus();
        let r = search(&e, "control", Filter::All, &[]);
        assert_eq!(r.groups[1].caption, "Control");
        let t = titles(&e, &r);
        for want in ["Play Control", "Control settings", "Open Control folder", "All actions for Control\u{2026}"] {
            assert!(t.contains(&want.to_string()), "{want}: {t:?}");
        }
        // The minor verbs are typed (or found in All actions).
        assert!(!t.contains(&"Hide Control".to_string()));
    }

    #[test]
    fn a_game_that_is_not_installed_offers_install() {
        let e = corpus();
        let r = search(&e, "hades", Filter::All, &[]);
        assert_eq!(r.groups[1].caption, "Hades");
        assert_eq!(titles(&e, &r)[1], "Install Hades");
        let r = search(&e, "install hades", Filter::All, &[]);
        assert_eq!(titles(&e, &r), ["Install Hades"]);
    }

    #[test]
    fn leading_and_trailing_verbs() {
        let e = corpus();
        let r = search(&e, "play elden", Filter::All, &[]);
        assert_eq!(r.groups.len(), 1);
        assert_eq!(titles(&e, &r), ["Play Elden Ring"]);
        assert_eq!(r.verbs[0].act, Act::Game("s_eldenring".into(), GameAction::Primary));
        let r = search(&e, "cyber settings", Filter::All, &[]);
        assert_eq!(titles(&e, &r), ["Cyberpunk 2077 settings"]);
        let r = search(&e, "open control", Filter::All, &[]);
        assert_eq!(titles(&e, &r), ["Open Control folder"]);
        let r = search(&e, "hide cyber", Filter::All, &[]);
        assert_eq!(titles(&e, &r), ["Hide Cyberpunk 2077"]);
        let r = search(&e, "fav elden", Filter::All, &[]);
        assert_eq!(titles(&e, &r), ["Add Elden Ring to favourites"]);
        // Not installed: nothing to play.
        let r = search(&e, "play hades", Filter::All, &[]);
        assert!(!titles(&e, &r).iter().any(|t| t.starts_with("Play ")));
    }

    #[test]
    fn a_running_game_is_stopped_not_played() {
        let mut e = corpus();
        e.push(game_entry(&GameRow {
            key: "s_run",
            title: "Runner Game",
            store_title: "",
            source: "Steam",
            playtime: "2h",
            installed: true,
            has_folder: true,
            playing: true,
            favourite: false,
            hidden: false,
            last_played: None,
        }));
        let r = search(&e, "play runner", Filter::All, &[]);
        assert!(!titles(&e, &r).iter().any(|t| t == "Play Runner Game"));
        let r = search(&e, "stop runner", Filter::All, &[]);
        assert_eq!(titles(&e, &r), ["Stop Runner Game"]);
        let r = search(&e, "runner game", Filter::All, &[]);
        assert!(titles(&e, &r).contains(&"Stop Runner Game".to_string()));
    }

    #[test]
    fn a_title_that_starts_with_a_verb_still_matches_as_a_title() {
        let mut e = corpus();
        e.push(game("s_ow", "Open World Racer", None, true));
        let r = search(&e, "open world", Filter::All, &[]);
        assert_eq!(titles(&e, &r)[0], "Open World Racer");
    }

    #[test]
    fn facet_filters_are_only_found_by_typing() {
        use heronite_data::collection::{FacetOption, FacetOptions};
        let mut e = corpus();
        let facets = FacetOptions { genre: vec![FacetOption { value: "Roguelike".into(), count: 3 }], ..Default::default() };
        e.extend(view_entries(&Default::default(), &facets));
        let r = search(&e, "", Filter::Actions, &[]);
        assert!(!titles(&e, &r).iter().any(|t| t.contains("Roguelike")));
        let r = search(&e, "roguelike", Filter::All, &[]);
        assert_eq!(titles(&e, &r)[0], "Filter Genre: Roguelike");
        // Not by scattered letters ("rglk").
        let r = search(&e, "rglk", Filter::All, &[]);
        assert!(!titles(&e, &r).iter().any(|t| t.contains("Roguelike")));
        let r = search(&e, "sort playtime", Filter::All, &[]);
        assert_eq!(titles(&e, &r)[0], "Sort by playtime");
    }

    #[test]
    fn actions_list_puts_statuses_last_and_searches() {
        let e = with_selection();
        let only: Vec<Entry> = e.into_iter().filter(|e| e.context && !matches!(e.act, Act::GameActions(_))).collect();
        let r = actions_list(&only, "", &[], "Control");
        assert_eq!(captions(&r), ["Control", "Set status"]);
        assert_eq!(titles(&only, &r).last().unwrap(), "Status: Beaten");
        let r = actions_list(&only, "beat", &[], "Control");
        assert_eq!(titles(&only, &r), ["Status: Beaten"]);
        let r = actions_list(&only, "delete", &[], "Control");
        assert_eq!(titles(&only, &r), ["Uninstall"]);
    }

    #[test]
    fn filters_restrict_the_kinds() {
        let e = with_selection();
        let r = search(&e, "e", Filter::Games, &[]);
        assert!(r.groups.iter().all(|g| g.caption != "Go to" && g.caption != "Actions" && g.caption != "Control"));
        let r = search(&e, "e", Filter::Pages, &[]);
        assert!(r.items().all(|i| matches!(i, ItemRef::Entry(i) if e[i].kind == Kind::Page)));
    }

    #[test]
    fn ranking_is_stable() {
        let e = with_selection();
        let a = titles(&e, &search(&e, "e", Filter::All, &[]));
        let b = titles(&e, &search(&e, "e", Filter::All, &[]));
        assert_eq!(a, b);
    }

    #[test]
    fn nothing_matches_nothing() {
        let e = corpus();
        let r = search(&e, "zzzzqq", Filter::All, &[]);
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
