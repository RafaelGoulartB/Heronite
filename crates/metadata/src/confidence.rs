//! When the games the sources found for a game are safe to save without
//! asking the user ([`judge`]).
//!
//! Each candidate says how its source found the game ([`MatchedBy`]); every
//! way but a fuzzy title is sure. The rule:
//! - sure candidates are used;
//! - a guess (fuzzy title) that is the same game as a sure candidate, or as
//!   a sure match the record already has (an anchor), is used too; one that
//!   is another game is left out of the merge, without asking;
//! - with nothing sure, the guesses are saved when at least
//!   [`AGREEING_GUESSES`] sources found the same game and no other group
//!   as large found another; a lone guess, or guesses that disagree, are
//!   not saved: the game goes to review.
//!
//! "The same game" ([`same_game`]): the names are equal once normalized,
//! or one is the other followed by a subtitle or edition words (not by a
//! number, a sequel, or an add-on word), and the release years, when both
//! are known, are at most [`YEAR_SLACK`] apart.
//!
//! Every automatic path obeys it (bulk jobs, the idle fill); the per-game
//! dialog saves what the user picked.

use crate::covers::EDITION_WORDS;
use crate::review::ReviewReason;
use crate::types::MatchedBy;
use crate::util::normalize_title;

/// How many sources must find the same game by a close name for that
/// game to be saved without asking. The one knob of the rule.
pub const AGREEING_GUESSES: usize = 2;

/// How far apart two sources' release years may be for the same game
/// (platforms and regions release a year apart).
pub const YEAR_SLACK: i32 = 1;

/// A word after a shared name that makes the longer name another game.
fn is_sequel_word(word: &str) -> bool {
    word.chars().all(|c| c.is_ascii_digit())
        || matches!(word, "ii" | "iii" | "iv" | "v" | "vi" | "vii" | "viii" | "ix" | "x" | "xi" | "xii" | "xiii" | "zero")
}

/// Words that make the longer name an add-on of the game.
fn is_addon_word(word: &str) -> bool {
    matches!(
        word,
        "soundtrack"
            | "ost"
            | "dlc"
            | "pack"
            | "bundle"
            | "demo"
            | "toolkit"
            | "bonus"
            | "skin"
            | "skins"
            | "expansion"
            | "season"
            | "update"
            | "artbook"
            | "costume"
            | "mod"
    )
}

/// Whether a name sets a subtitle apart ("Hades: Battle out of Hell",
/// "Castlevania - Symphony of the Night").
fn has_subtitle_mark(title: &str) -> bool {
    title.contains(':') || title.contains(" - ") || title.contains(" – ") || title.contains(" — ") || title.contains('(')
}

/// Whether two names are the same game's: equal once normalized, or the
/// longer is the shorter plus edition words ("Sekiro" and "Sekiro GOTY
/// Edition") or plus a subtitle set apart by a colon or a dash ("The
/// Witcher 3" and "The Witcher 3: Wild Hunt"); never plus a number (a
/// sequel), an add-on word ("Soundtrack") or a bare word ("Portal Knights").
pub fn same_title(a: &str, b: &str) -> bool {
    let (ka, kb) = (normalize_title(a), normalize_title(b));
    if ka.is_empty() || kb.is_empty() {
        return false;
    }
    if ka == kb {
        return true;
    }
    let ((short, _), (long, long_raw)) = if ka.len() <= kb.len() { ((&ka, a), (&kb, b)) } else { ((&kb, b), (&ka, a)) };
    let Some(rest) = long.strip_prefix(short.as_str()).and_then(|r| r.strip_prefix(' ')) else { return false };
    let words = || rest.split(' ');
    if words().all(|w| EDITION_WORDS.contains(&w)) {
        return true;
    }
    has_subtitle_mark(long_raw) && !words().next().is_some_and(is_sequel_word) && !words().any(is_addon_word)
}

/// Whether two found games are the same one (see the module docs).
pub fn same_game(a_title: &str, a_year: Option<i32>, b_title: &str, b_year: Option<i32>) -> bool {
    let years_agree = match (a_year, b_year) {
        (Some(a), Some(b)) => (a - b).abs() <= YEAR_SLACK,
        _ => true,
    };
    years_agree && same_title(a_title, b_title)
}

/// A game a source found, as the rule sees it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Guess<'a> {
    /// `None` (older data): not said, taken as sure.
    pub matched_by: Option<MatchedBy>,
    pub title: Option<&'a str>,
    pub year: Option<i32>,
}

impl Guess<'_> {
    pub fn is_sure(&self) -> bool {
        self.matched_by.is_none_or(MatchedBy::is_sure)
    }

    fn agrees(&self, other: &Guess<'_>) -> bool {
        match (self.title, other.title) {
            (Some(a), Some(b)) => same_game(a, self.year, b, other.year),
            _ => false,
        }
    }
}

/// What to do with a game's candidates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// Save: the indices of the candidates the merge uses, in order (empty
    /// when there were none). The others are left out.
    Save(Vec<usize>),
    /// Save nothing: the user picks the match.
    Review(ReviewReason),
}

/// The rule (see the module docs). `anchors` are sure matches the game
/// already has from sources not among `guesses`.
pub fn judge(guesses: &[Guess<'_>], anchors: &[Guess<'_>]) -> Verdict {
    let known: Vec<&Guess<'_>> = guesses.iter().filter(|g| g.is_sure()).chain(anchors.iter().filter(|a| a.title.is_some())).collect();
    if !known.is_empty() {
        let used = (0..guesses.len()).filter(|&i| guesses[i].is_sure() || known.iter().any(|k| guesses[i].agrees(k))).collect();
        return Verdict::Save(used);
    }
    if guesses.is_empty() {
        return Verdict::Save(Vec::new());
    }
    let mut groups: Vec<Vec<usize>> = Vec::new();
    for (i, g) in guesses.iter().enumerate() {
        match groups.iter_mut().find(|group| guesses[group[0]].agrees(g)) {
            Some(group) => group.push(i),
            None => groups.push(vec![i]),
        }
    }
    // Stable: the first group found wins a tie (and a tie is never saved).
    groups.sort_by_key(|g| std::cmp::Reverse(g.len()));
    let runner_up = groups.get(1).map_or(0, Vec::len);
    match groups.first() {
        Some(best) if best.len() >= AGREEING_GUESSES && best.len() > runner_up => Verdict::Save(best.clone()),
        _ if guesses.len() == 1 => Verdict::Review(ReviewReason::LoneGuess),
        _ => Verdict::Review(ReviewReason::SourcesDisagree),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use MatchedBy::*;

    fn g(by: MatchedBy, title: &'static str, year: Option<i32>) -> Guess<'static> {
        Guess { matched_by: Some(by), title: Some(title), year }
    }

    #[test]
    fn same_title_takes_subtitles_and_editions_but_not_sequels_or_addons() {
        assert!(same_title("Portal 2", "portal-2"));
        assert!(same_title("The Witcher 3", "The Witcher 3: Wild Hunt"));
        assert!(same_title("Baldur's Gate 3", "Baldur's Gate 3: Deluxe Edition"));
        assert!(same_title("Castlevania - Symphony of the Night", "Castlevania: Symphony of the Night"));
        assert!(!same_title("Portal", "Portal 2"));
        assert!(!same_title("Final Fantasy", "Final Fantasy VII"));
        assert!(!same_title("eFootball PES 2021", "eFootball PES 2021 Season Update"));
        assert!(!same_title("Batman Arkham Knight", "Batman: Arkham Knight - Batman Inc. Skin"));
        assert!(!same_title("NieR:Automata", "Untitled NieR:Automata Project"));
        assert!(!same_title("Hades", "Hades Soundtrack"));
        assert!(!same_title("Portal", "Portal Knights"));
        assert!(same_title("Tomb Raider", "Tomb Raider Game of the Year"));
        assert!(!same_title("", ""));
    }

    #[test]
    fn years_more_than_the_slack_apart_are_other_games() {
        assert!(same_game("Tomb Raider", Some(2013), "Tomb Raider", Some(2013)));
        assert!(same_game("Tomb Raider", Some(2013), "Tomb Raider", Some(2013 + YEAR_SLACK)));
        assert!(!same_game("Tomb Raider", Some(1996), "Tomb Raider", Some(2013)));
        assert!(same_game("Tomb Raider", None, "Tomb Raider", Some(2013)));
    }

    #[test]
    fn sure_candidates_are_used_with_the_guesses_that_agree() {
        let found = [g(SteamId, "Portal 2", Some(2011)), g(Game, "Portal 2", None), g(FuzzyTitle, "Portal 2: Wild", Some(2011))];
        assert_eq!(judge(&found, &[]), Verdict::Save(vec![0, 1, 2]));
    }

    #[test]
    fn a_guess_that_is_another_game_is_left_out_without_asking() {
        let found = [g(SteamId, "Portal 2", Some(2011)), g(FuzzyTitle, "Portal", Some(2007))];
        assert_eq!(judge(&found, &[]), Verdict::Save(vec![0]));
        // An exact title is sure, whatever the guesses say.
        let found = [g(FuzzyTitle, "Portal Knights", None), g(ExactTitle, "Portal", Some(2007))];
        assert_eq!(judge(&found, &[]), Verdict::Save(vec![1]));
    }

    #[test]
    fn a_match_the_record_has_anchors_the_guesses() {
        let anchor = [g(SourceId, "The Witcher 3: Wild Hunt", Some(2015))];
        assert_eq!(judge(&[g(FuzzyTitle, "The Witcher 3", Some(2015))], &anchor), Verdict::Save(vec![0]));
        assert_eq!(judge(&[g(FuzzyTitle, "The Witcher 2", Some(2011))], &anchor), Verdict::Save(vec![]));
        // An anchor without a name tells nothing.
        let blind = [Guess { matched_by: Some(SourceId), title: None, year: None }];
        assert_eq!(judge(&[g(FuzzyTitle, "The Witcher 3", None)], &blind), Verdict::Review(ReviewReason::LoneGuess));
    }

    #[test]
    fn guesses_alone_are_saved_only_when_enough_sources_agree() {
        let two = [g(FuzzyTitle, "The Witcher 3: Wild Hunt", Some(2015)), g(FuzzyTitle, "The Witcher 3 Wild Hunt", Some(2015))];
        assert_eq!(judge(&two, &[]), Verdict::Save(vec![0, 1]));
        let majority = [
            g(FuzzyTitle, "Hades II", Some(2024)),
            g(FuzzyTitle, "Hades", Some(2020)),
            g(FuzzyTitle, "Hades: Battle out of Hell", Some(2020)),
        ];
        assert_eq!(judge(&majority, &[]), Verdict::Save(vec![1, 2]));
    }

    #[test]
    fn a_lone_guess_or_guesses_that_disagree_go_to_review() {
        assert_eq!(judge(&[g(FuzzyTitle, "Portal Knights", None)], &[]), Verdict::Review(ReviewReason::LoneGuess));
        let split = [g(FuzzyTitle, "Doom", Some(1993)), g(FuzzyTitle, "Doom", Some(2016))];
        assert_eq!(judge(&split, &[]), Verdict::Review(ReviewReason::SourcesDisagree));
        let tie = [
            g(FuzzyTitle, "Doom", Some(1993)),
            g(FuzzyTitle, "Doom", Some(1993)),
            g(FuzzyTitle, "Doom", Some(2016)),
            g(FuzzyTitle, "Doom", Some(2016)),
        ];
        assert_eq!(judge(&tie, &[]), Verdict::Review(ReviewReason::SourcesDisagree));
    }

    #[test]
    fn nothing_found_and_unsaid_matches_save_as_before() {
        assert_eq!(judge(&[], &[]), Verdict::Save(vec![]));
        let older = [Guess { matched_by: None, title: Some("X"), year: None }];
        assert_eq!(judge(&older, &[]), Verdict::Save(vec![0]));
    }
}
