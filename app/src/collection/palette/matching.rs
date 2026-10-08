//! Case- and diacritic-insensitive matching for the command palette.
//!
//! Every searchable text is folded once when the palette opens (see
//! [`fold`]); a keystroke only folds the query and scores the entries without
//! allocating per entry. Scores come in tiers so the ranking reads naturally:
//! exact > prefix > word start > substring > fuzzy (letters in order).

/// Whole text equals the token.
const TIER_EXACT: i32 = 1000;
/// Text starts with the token.
const TIER_PREFIX: i32 = 900;
/// A word of the text starts with the token.
pub(super) const TIER_WORD: i32 = 800;
/// The token appears inside a word.
const TIER_SUBSTR: i32 = 600;
/// The token's letters appear in order (e.g. "wnmg" -> "wine manager").
const TIER_FUZZY: i32 = 100;

/// Lower-cases `c` and strips the diacritic of the common Latin letters.
fn fold_char(c: char) -> char {
    if c.is_ascii() {
        return c.to_ascii_lowercase();
    }
    let c = c.to_lowercase().next().unwrap_or(c);
    match c {
        'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' | 'ā' | 'ă' | 'ą' => 'a',
        'ç' | 'ć' | 'ĉ' | 'ċ' | 'č' => 'c',
        'ď' | 'đ' => 'd',
        'è' | 'é' | 'ê' | 'ë' | 'ē' | 'ĕ' | 'ė' | 'ę' | 'ě' => 'e',
        'ĝ' | 'ğ' | 'ġ' | 'ģ' => 'g',
        'ĥ' | 'ħ' => 'h',
        'ì' | 'í' | 'î' | 'ï' | 'ĩ' | 'ī' | 'ĭ' | 'į' | 'ı' => 'i',
        'ĵ' => 'j',
        'ķ' => 'k',
        'ĺ' | 'ļ' | 'ľ' | 'ŀ' | 'ł' => 'l',
        'ñ' | 'ń' | 'ņ' | 'ň' => 'n',
        'ò' | 'ó' | 'ô' | 'õ' | 'ö' | 'ø' | 'ō' | 'ŏ' | 'ő' => 'o',
        'ŕ' | 'ŗ' | 'ř' => 'r',
        'ś' | 'ŝ' | 'ş' | 'š' => 's',
        'ţ' | 'ť' | 'ŧ' => 't',
        'ù' | 'ú' | 'û' | 'ü' | 'ũ' | 'ū' | 'ŭ' | 'ů' | 'ű' | 'ų' => 'u',
        'ŵ' => 'w',
        'ý' | 'ÿ' | 'ŷ' => 'y',
        'ź' | 'ż' | 'ž' => 'z',
        c => c,
    }
}

/// Appends the folded form of `s` to `out`. Multi-letter ligatures expand
/// (`ß` -> `ss`, `æ` -> `ae`, `œ` -> `oe`).
fn fold_into(s: &str, out: &mut String) {
    for c in s.chars() {
        match c {
            'ß' => out.push_str("ss"),
            'æ' | 'Æ' => out.push_str("ae"),
            'œ' | 'Œ' => out.push_str("oe"),
            c => out.push(fold_char(c)),
        }
    }
}

/// Folded copy of `s`.
pub(super) fn fold(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    fold_into(s, &mut out);
    out
}

fn is_word_start(text: &str, at: usize) -> bool {
    at == 0 || text[..at].chars().next_back().is_some_and(|c| !c.is_alphanumeric())
}

/// How well one folded `token` matches one folded `text`, `None` when it does
/// not. Shorter texts and earlier matches rank first inside a tier.
fn score_token(token: &str, text: &str) -> Option<i32> {
    if token.is_empty() || text.is_empty() {
        return None;
    }
    if text == token {
        return Some(TIER_EXACT);
    }
    let extra = ((text.len() - token.len().min(text.len())).min(99)) as i32;
    if text.starts_with(token) {
        return Some(TIER_PREFIX - extra / 3);
    }
    let mut first = None;
    for (at, _) in text.match_indices(token) {
        if is_word_start(text, at) {
            return Some(TIER_WORD - (at.min(99) as i32) / 3);
        }
        first.get_or_insert(at);
    }
    if let Some(at) = first {
        return Some(TIER_SUBSTR - (at.min(99) as i32) / 3);
    }
    fuzzy(token, text)
}

/// Letters of `token` in order inside `text` (two or more letters), starting
/// at a word start and not spread wider than about twice the token. Word
/// starts and consecutive letters add to the score; gaps subtract.
fn fuzzy(token: &str, text: &str) -> Option<i32> {
    let len = token.chars().count();
    if len < 2 {
        return None;
    }
    let first = token.chars().next()?;
    let mut best: Option<i32> = None;
    for (start, c) in text.char_indices() {
        if c != first || !is_word_start(text, start) {
            continue;
        }
        let mut score = 0i32;
        let mut from = start;
        let mut prev_end: Option<usize> = None;
        let mut ok = true;
        for q in token.chars() {
            let Some((rel, _)) = text[from..].char_indices().find(|(_, c)| *c == q) else {
                ok = false;
                break;
            };
            let at = from + rel;
            if is_word_start(text, at) {
                score += 15;
            }
            match prev_end {
                Some(p) if p == at => score += 10,
                Some(p) => score -= (at - p).min(20) as i32 / 4,
                None => {}
            }
            prev_end = Some(at + q.len_utf8());
            from = at + q.len_utf8();
        }
        if ok && from - start <= 2 * len + 4 {
            best = best.max(Some(score));
        }
    }
    best.map(|s| (TIER_FUZZY + s).clamp(1, TIER_SUBSTR - 101))
}

/// Splits a folded query into tokens (no allocation besides the vector).
pub fn tokens(folded: &str) -> Vec<&str> {
    folded.split_whitespace().collect()
}

/// Searchable fields of an entry, all folded.
pub(super) struct Haystack<'a> {
    pub title: &'a str,
    /// Other names (a game's store title).
    pub alt: &'a str,
    /// Extra search words (a page's synonyms).
    pub keywords: &'a str,
}

/// Score of an entry for all `tokens` (each must match somewhere): the
/// average of the best score of every token. The title counts fully, other
/// names slightly less, keywords much less.
pub(super) fn score_entry(tokens: &[&str], h: &Haystack) -> Option<i32> {
    if tokens.is_empty() {
        return None;
    }
    let mut sum = 0;
    for t in tokens {
        let mut best = score_token(t, h.title);
        if !h.alt.is_empty() {
            if let Some(s) = score_token(t, h.alt) {
                best = best.max(Some(s - 60));
            }
        }
        if !h.keywords.is_empty() {
            if let Some(s) = score_token(t, h.keywords) {
                best = best.max(Some(s.min(TIER_WORD) - 250));
            }
        }
        sum += best?;
    }
    Some(sum / tokens.len() as i32)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(q: &str, t: &str) -> Option<i32> {
        score_token(&fold(q), &fold(t))
    }

    #[test]
    fn folds_case_and_diacritics() {
        assert_eq!(fold("Pokémon Légendes: Arceus"), "pokemon legendes: arceus");
        assert_eq!(fold("ÅNGSTRÖM Straße"), "angstrom strasse");
        assert_eq!(fold("Bōkura no Sōsaku"), "bokura no sosaku");
    }

    #[test]
    fn accents_in_the_query_or_the_title_match_either_way() {
        assert!(s("pokemon", "Pokémon Sword").is_some());
        assert!(s("pokémon", "Pokemon Sword").is_some());
        assert!(s("CAFE", "Café del Mar").is_some());
    }

    #[test]
    fn tiers_are_ordered() {
        let exact = s("wine", "Wine").unwrap();
        let prefix = s("wine", "Wine Manager").unwrap();
        let word = s("man", "Wine Manager").unwrap();
        let sub = s("ana", "Wine Manager").unwrap();
        let fuzzy = s("wmgr", "Wine Manager").unwrap();
        assert!(exact > prefix, "{exact} {prefix}");
        assert!(prefix > word, "{prefix} {word}");
        assert!(word > sub, "{word} {sub}");
        assert!(sub > fuzzy, "{sub} {fuzzy}");
        assert!(fuzzy > 0);
    }

    #[test]
    fn shorter_title_wins_inside_a_tier() {
        assert!(s("control", "Control") > s("control", "Control: Ultimate Edition"));
        assert!(s("hal", "Halo") > s("hal", "Halo Infinite Campaign"));
    }

    #[test]
    fn word_start_beats_substring() {
        assert!(s("ring", "Elden Ring").unwrap() > s("ring", "Spring Break").unwrap());
    }

    #[test]
    fn missing_letters_do_not_match() {
        assert_eq!(s("xyz", "Wine Manager"), None);
        assert_eq!(s("wq", "Wine Manager"), None);
        // One letter only matches as a substring, never fuzzy.
        assert_eq!(s("z", "Wine Manager"), None);
        assert!(s("w", "Wine Manager").is_some());
    }

    #[test]
    fn fuzzy_needs_letters_in_order() {
        assert!(s("wmg", "Wine Manager").is_some());
        assert_eq!(s("gmw", "Wine Manager"), None);
    }

    #[test]
    fn fuzzy_does_not_match_letters_scattered_over_a_long_title() {
        assert_eq!(s("control", "Bridge Constructor: The Walking Dead"), None);
        assert!(s("ctrl", "Control").is_some());
    }

    #[test]
    fn every_token_must_match() {
        let h = Haystack { title: "wine manager", alt: "", keywords: "proton" };
        assert!(score_entry(&["wine", "man"], &h).is_some());
        assert!(score_entry(&["wine", "zzz"], &h).is_none());
        assert!(score_entry(&[], &h).is_none());
    }

    #[test]
    fn keywords_and_alt_names_rank_below_the_title() {
        let by_title = Haystack { title: "proton tools", alt: "", keywords: "" };
        let by_kw = Haystack { title: "wine manager", alt: "", keywords: "proton" };
        let by_alt = Haystack { title: "other", alt: "proton game", keywords: "" };
        let a = score_entry(&["proton"], &by_title).unwrap();
        let b = score_entry(&["proton"], &by_kw).unwrap();
        let c = score_entry(&["proton"], &by_alt).unwrap();
        assert!(a > c && c > b, "{a} {b} {c}");
    }

    #[test]
    fn is_fast_enough_for_two_thousand_titles() {
        let titles: Vec<String> = (0..2000).map(|i| fold(&format!("Some Rather Long Game Title Number {i} Edition"))).collect();
        let t = std::time::Instant::now();
        let mut hits = 0;
        for title in &titles {
            let h = Haystack { title, alt: "", keywords: "" };
            if score_entry(&["gt", "ed"], &h).is_some() {
                hits += 1;
            }
        }
        assert_eq!(hits, 2000);
        // Debug build, generous: the point is "no pathological slowdown".
        assert!(t.elapsed() < std::time::Duration::from_millis(250), "{:?}", t.elapsed());
    }
}
