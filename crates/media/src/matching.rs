//! Game title -> captures folder matching. Exact port of
//! `src/local-library/screenshots/match.ts` (same rules, same scores).
//!
//! Rules, from strongest to weakest: identical normalized name (100), same
//! name without spaces (97), same "core" name (editions such as Director's
//! Cut / GOTY / Remastered and leading articles removed) (94 / 91), then a
//! token overlap score. Sequel numbers must agree (`Game 2` never matches
//! `Game 3` or `Game`). A Steam app id folder (`<id>` or `<id> ...`) wins
//! outright.

use regex::Regex;
use std::collections::HashSet;
use std::sync::OnceLock;

/// Minimum score for a folder to be accepted.
pub const MIN_SCORE: i32 = 64;

const STOPWORDS: [&str; 7] = ["a", "an", "and", "of", "the", "to", "for"];

fn edition_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        // JS `\b` is ASCII-only, so the Unicode-aware `\b` is switched off.
        Regex::new(
            r#"(?i)(?-u:\b)((director['’`]?s|game of the year|goty|definitive|ultimate|complete|deluxe|gold|standard|enhanced|anniversary|special|limited|windows)\s*(edition|cut)?|director s cut|remastered|remaster|goty|edition)(?-u:\b)"#,
        )
        .expect("edition regex")
    })
}

fn article_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^(the|a|an)\s+").expect("article regex"))
}

fn ws_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"\s+").expect("ws regex"))
}

fn non_alnum_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"[^a-z0-9]+").expect("non-alnum regex"))
}

/// JS `String.prototype.trim()` strips Unicode whitespace; Rust's `trim` the
/// same `White_Space` set (plus BOM in JS, irrelevant after normalization).
fn trim(s: &str) -> &str {
    s.trim()
}

/// `normalizeScreenshotName`: lowercase, `&` -> `and`, Greek delta -> `delta`,
/// quotes and marks dropped, everything outside `a-z0-9` is a separator.
pub fn normalize_screenshot_name(value: &str) -> String {
    let lower = value.to_lowercase();
    let mut s = String::with_capacity(lower.len() + 8);
    for c in lower.chars() {
        match c {
            'Δ' | 'δ' => s.push_str(" delta "),
            '&' => s.push_str(" and "),
            // Normalized to `'` and then turned into a separator like any other
            // non-alphanumeric character.
            '’' | '`' | '´' => s.push('\''),
            '™' | '®' | '©' => {}
            c => s.push(c),
        }
    }
    let s = non_alnum_re().replace_all(&s, " ");
    let s = ws_re().replace_all(&s, " ");
    trim(&s).to_owned()
}

fn compact(value: &str) -> String {
    ws_re().replace_all(value, "").into_owned()
}

/// `coreScreenshotName`: [`normalize_screenshot_name`] with editions and a
/// leading article removed.
pub fn core_screenshot_name(value: &str) -> String {
    let stripped = edition_re().replace_all(value, " ");
    let normalized = normalize_screenshot_name(&stripped);
    let stripped = edition_re().replace_all(&normalized, " ");
    let collapsed = ws_re().replace_all(&stripped, " ");
    let no_article = article_re().replace(&collapsed, "");
    trim(&no_article).to_owned()
}

fn tokens(value: &str) -> Vec<&str> {
    value.split(' ').filter(|s| !s.is_empty()).collect()
}

fn significant_tokens(value: &str) -> Vec<&str> {
    tokens(value).into_iter().filter(|t| !STOPWORDS.contains(t)).collect()
}

fn is_number(token: &str) -> bool {
    !token.is_empty() && token.bytes().all(|b| b.is_ascii_digit())
}

fn numbers<'a>(items: &[&'a str]) -> Vec<&'a str> {
    items.iter().copied().filter(|t| is_number(t)).collect()
}

/// `sameSet`: equal length (duplicates count) and every right item is in left.
fn same_set(left: &[&str], right: &[&str]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    let wanted: HashSet<&str> = left.iter().copied().collect();
    right.iter().all(|t| wanted.contains(t))
}

fn is_prefix(query: &[&str], folder: &[&str]) -> bool {
    if query.is_empty() || query.len() > folder.len() {
        return false;
    }
    query.iter().zip(folder).all(|(q, f)| q == f)
}

/// JS `Math.round` (ties toward +infinity).
fn js_round(x: f64) -> i32 {
    (x + 0.5).floor() as i32
}

/// `scoreScreenshotFolder`: 0..=100 similarity between a game name and a
/// folder name.
pub fn score_screenshot_folder(query: &str, folder: &str) -> i32 {
    let qn = normalize_screenshot_name(query);
    let fn_ = normalize_screenshot_name(folder);
    if qn.is_empty() || fn_.is_empty() {
        return 0;
    }
    if qn == fn_ {
        return 100;
    }
    if compact(&qn) == compact(&fn_) {
        return 97;
    }

    let qc = core_screenshot_name(query);
    let fc = core_screenshot_name(folder);
    if !qc.is_empty() && !fc.is_empty() && qc == fc {
        return 94;
    }
    if !qc.is_empty() && !fc.is_empty() && compact(&qc) == compact(&fc) {
        return 91;
    }

    let q_src = if qc.is_empty() { &qn } else { &qc };
    let f_src = if fc.is_empty() { &fn_ } else { &fc };
    let q_tokens = significant_tokens(q_src);
    let f_tokens = significant_tokens(f_src);
    if q_tokens.is_empty() || f_tokens.is_empty() {
        return 0;
    }

    let q_nums = numbers(&q_tokens);
    let f_nums = numbers(&f_tokens);
    if !q_nums.is_empty() && !same_set(&q_nums, &f_nums) {
        return 0;
    }

    let set_q: HashSet<&str> = q_tokens.iter().copied().collect();
    let set_f: HashSet<&str> = f_tokens.iter().copied().collect();
    let inter = set_q.iter().filter(|t| set_f.contains(*t)).count();
    if inter == 0 {
        return 0;
    }

    let union = set_q.union(&set_f).count();
    let jaccard = inter as f64 / union as f64;
    let coverage = inter as f64 / set_q.len() as f64;
    let extra = set_f.iter().filter(|t| !set_q.contains(*t)).count();
    let mut penalty = 0;
    if q_nums.is_empty() && !f_nums.is_empty() {
        penalty += 30;
    }
    if extra >= 2 {
        penalty += 22;
    } else if extra == 1 {
        penalty += 10;
    }
    if extra > 0 && !is_prefix(&q_tokens, &f_tokens) && q_nums.is_empty() {
        penalty += 18;
    }

    let mut score = js_round(jaccard * 50.0 + coverage * 45.0);
    if coverage == 1.0 && !q_nums.is_empty() && same_set(&q_nums, &f_nums) {
        score = score.max(80);
    }
    if is_prefix(&q_tokens, &f_tokens) && coverage == 1.0 {
        score = score.max(74);
    }
    let q_compact = compact(q_src);
    let f_compact = compact(f_src);
    if !q_compact.is_empty()
        && !f_compact.is_empty()
        && (f_compact.starts_with(&q_compact) || q_compact.starts_with(&f_compact))
    {
        let (a, b) = (q_compact.len().min(f_compact.len()), q_compact.len().max(f_compact.len()));
        let ratio = a as f64 / b as f64;
        score = score.max(js_round(70.0 * ratio + 20.0));
    }

    (score - penalty).max(0)
}

/// Result of [`pick_screenshot_folder`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FolderMatch {
    pub folder: String,
    pub score: i32,
}

/// `pickScreenshotFolder`: best folder for any of `queries`, or `None` below
/// `min_score`. A folder named `<steam_app_id>` / `<steam_app_id> ...` is
/// returned at once, in folder order, like the original.
pub fn pick_screenshot_folder<Q: AsRef<str>, F: AsRef<str>>(
    queries: &[Q],
    folders: &[F],
    steam_app_id: Option<&str>,
    min_score: i32,
) -> Option<FolderMatch> {
    let steam = steam_app_id.filter(|s| !s.is_empty());
    let mut best: Option<FolderMatch> = None;
    for folder in folders {
        let folder = folder.as_ref();
        if let Some(id) = steam {
            if folder == id || folder.strip_prefix(id).is_some_and(|rest| rest.starts_with(' ')) {
                return Some(FolderMatch { folder: folder.to_owned(), score: 100 });
            }
        }
        let mut score = 0;
        for query in queries {
            let query = query.as_ref();
            let mut next = score_screenshot_folder(query, folder);
            let q_core = core_screenshot_name(query);
            let f_core = core_screenshot_name(folder);
            let q_tokens = significant_tokens(&q_core);
            let f_tokens = significant_tokens(&f_core);
            let q_nums = numbers(&q_tokens);
            let f_nums = numbers(&f_tokens);
            let q_words: Vec<&str> = q_tokens.iter().copied().filter(|t| !is_number(t)).collect();
            let word_hits = q_words.iter().filter(|w| f_tokens.contains(w)).count();
            if !q_nums.is_empty()
                && same_set(&q_nums, &f_nums)
                && !q_words.is_empty()
                && word_hits as f64 / q_words.len() as f64 >= 0.5
            {
                next = next.max(82);
            }
            score = score.max(next);
        }
        if score < min_score {
            continue;
        }
        if best.as_ref().is_none_or(|b| score > b.score) {
            best = Some(FolderMatch { folder: folder.to_owned(), score });
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pick(queries: &[&str], folders: &[&str], steam: Option<&str>) -> Option<String> {
        pick_screenshot_folder(queries, folders, steam, MIN_SCORE).map(|m| m.folder)
    }

    #[test]
    fn normalize() {
        assert_eq!(normalize_screenshot_name("  Baldur's  Gate 3 "), "baldur s gate 3");
        assert_eq!(normalize_screenshot_name("Tom & Jerry"), "tom and jerry");
        assert_eq!(normalize_screenshot_name("Metal Gear Solid Δ"), "metal gear solid delta");
        assert_eq!(normalize_screenshot_name("Baldur’s Gate"), "baldur s gate");
        assert_eq!(normalize_screenshot_name("Game™ 2®"), "game 2");
        // Non a-z0-9 letters are separators, as in the original.
        assert_eq!(normalize_screenshot_name("Pokémon"), "pok mon");
        assert_eq!(normalize_screenshot_name("A: B - C"), "a b c");
        assert_eq!(normalize_screenshot_name(""), "");
    }

    #[test]
    fn core_removes_editions_and_articles() {
        assert_eq!(core_screenshot_name("Death Stranding - Director's Cut"), "death stranding");
        assert_eq!(core_screenshot_name("Death Stranding Director’s Cut"), "death stranding");
        assert_eq!(core_screenshot_name("The Witcher 3: Game of the Year Edition"), "witcher 3");
        assert_eq!(core_screenshot_name("Fallout 4 GOTY"), "fallout 4");
        assert_eq!(core_screenshot_name("Resident Evil 4 Remastered"), "resident evil 4");
        assert_eq!(core_screenshot_name("Game Definitive Edition"), "game");
        assert_eq!(core_screenshot_name("A Plague Tale"), "plague tale");
        // `director s cut` (after normalization of a different apostrophe).
        assert_eq!(core_screenshot_name("Ghost Director s Cut"), "ghost");
        // Not a whole word: untouched.
        assert_eq!(core_screenshot_name("Goldeneye"), "goldeneye");
        assert_eq!(core_screenshot_name("Standardization"), "standardization");
    }

    #[test]
    fn score_exact_tiers() {
        assert_eq!(score_screenshot_folder("Elden Ring", "Elden Ring"), 100);
        assert_eq!(score_screenshot_folder("Elden Ring", "elden-ring"), 100);
        // Punctuation vs space.
        assert_eq!(score_screenshot_folder("Black Myth: Wukong", "Black Myth - Wukong"), 100);
        // Compact names.
        assert_eq!(score_screenshot_folder("Half Life", "Halflife"), 97);
        assert_eq!(score_screenshot_folder("NieR:Automata", "NieR Automata"), 100);
        assert_eq!(score_screenshot_folder("Nier Automata", "NierAutomata"), 97);
        // Core names.
        assert_eq!(score_screenshot_folder("Death Stranding", "Death Stranding - Director's Cut"), 94);
        assert_eq!(score_screenshot_folder("The Division", "Division"), 94);
        assert_eq!(score_screenshot_folder("The Division 2", "Division2"), 91);
        assert_eq!(score_screenshot_folder("", "x"), 0);
        assert_eq!(score_screenshot_folder("x", ""), 0);
    }

    #[test]
    fn sequels_stay_apart() {
        assert_eq!(score_screenshot_folder("Death Stranding 2", "Death Stranding"), 0);
        assert_eq!(score_screenshot_folder("Mortal Shell 2", "Mortal Shell"), 0);
        assert_eq!(score_screenshot_folder("Game 2", "Game 3"), 0);
        assert!(pick(&["Death Stranding"], &["Death Stranding 2 - On the Beach"], None).is_none());
        assert_eq!(
            pick(
                &["Death Stranding 2: On the Beach"],
                &["Death Stranding - Director's Cut", "Death Stranding 2 - On the Beach"],
                None
            )
            .as_deref(),
            Some("Death Stranding 2 - On the Beach")
        );
        // Query without numbers against a folder with: penalized below the minimum.
        assert!(pick(&["Mortal Shell"], &["Mortal Shell 2"], None).is_none());
    }

    #[test]
    fn token_overlap_and_prefix() {
        // Subtitle missing in the folder name: prefix of the folder tokens.
        assert_eq!(
            pick(&["Metal Gear Solid Delta: Snake Eater"], &["Metal Gear Solid Delta"], None).as_deref(),
            Some("Metal Gear Solid Delta")
        );
        // Folder is the longer one with a subtitle.
        assert!(pick(&["Kingdom Come Deliverance"], &["Kingdom Come - Deliverance"], None).is_some());
        // Unrelated.
        assert!(pick(&["Elden Ring"], &["Dark Souls", "Brotato"], None).is_none());
        assert_eq!(score_screenshot_folder("Alpha", "Beta"), 0);
        // Only stopwords.
        assert_eq!(score_screenshot_folder("The", "A"), 0);
    }

    #[test]
    fn exact_score_numbers() {
        // Values produced by the original match.ts.
        // jaccard 3/4 + coverage 1, one extra token (prefix), compact prefix 0.5.
        assert_eq!(score_screenshot_folder("God of War", "God of War Ragnarok"), 68);
        assert_eq!(score_screenshot_folder("God of War", "God of War (2018)"), 38);
        assert_eq!(score_screenshot_folder("Mortal Shell", "Mortal Shell 2"), 44);
        assert_eq!(score_screenshot_folder("Resident Evil 4 Remake", "Resident Evil 4"), 71);
        assert_eq!(score_screenshot_folder("Resident Evil 4 Remake", "Fallout 4"), 11);
    }

    #[test]
    fn number_word_boost() {
        // Same numbers and at least half of the words in common: boosted to 82
        // by pickScreenshotFolder (the plain score is only 71).
        assert_eq!(
            pick_screenshot_folder(&["Resident Evil 4 Remake"], &["Resident Evil 4"], None, MIN_SCORE),
            Some(FolderMatch { folder: "Resident Evil 4".into(), score: 82 })
        );
        // Different number: never.
        assert!(pick(&["Resident Evil 4 Remake"], &["Resident Evil 5"], None).is_none());
    }

    #[test]
    fn steam_id_wins() {
        let folders = ["Elden Ring", "1245620", "1245620 Elden Ring"];
        assert_eq!(pick(&["Elden Ring"], &folders, Some("1245620")).as_deref(), Some("1245620"));
        // Returns the first id folder, even if an earlier folder scores 100.
        let folders = ["Elden Ring", "1245620 whatever"];
        assert_eq!(pick(&["Elden Ring"], &folders, Some("1245620")).as_deref(), Some("1245620 whatever"));
        // Prefix of another id does not count.
        assert!(pick(&["Zzz"], &["12456201"], Some("1245620")).is_none());
        // Empty id is ignored.
        assert_eq!(pick(&["Elden Ring"], &["Elden Ring"], Some("")).as_deref(), Some("Elden Ring"));
    }

    #[test]
    fn aliases_and_ties() {
        // Any query may match; first folder wins a tie (strict `>`).
        assert_eq!(
            pick(&["Wrong", "Elden Ring"], &["elden ring", "Elden-Ring"], None).as_deref(),
            Some("elden ring")
        );
        // Best score wins over order.
        assert_eq!(
            pick(&["Death Stranding"], &["Death Stranding - Director's Cut", "Death Stranding"], None)
                .as_deref(),
            Some("Death Stranding")
        );
        let no_queries: [&str; 0] = [];
        assert!(pick(&no_queries, &["a"], None).is_none());
    }

    #[test]
    fn js_round_ties_up() {
        assert_eq!(js_round(2.5), 3);
        assert_eq!(js_round(2.4999), 2);
    }
}
