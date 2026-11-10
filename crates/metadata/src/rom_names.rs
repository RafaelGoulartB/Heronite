//! ROM dump names as titles. No-Intro and Redump name a dump
//! `Legend of Zelda, The - A Link to the Past (USA) (Rev 1) [!].sfc`: the
//! title with its article moved to the end, then tags in parentheses
//! (region, languages, revision, disc) and brackets (dump flags), then the
//! extension. TOSEC adds `(1988)(Publisher)`.
//!
//! [`dump_title`] gives the title as people write it; [`match_key`] the form
//! two spellings of one title share (the dump's and a database's), which a
//! source compares for an exact match.

use std::path::Path;

use heronite_data::text::title_key;

use crate::util::re;

/// Articles a dump name moves to the end (`Legend of Zelda, The`), in the
/// languages dumps are named in.
const ARTICLES: &[&str] = &["The", "A", "An", "Die", "Der", "Das", "Le", "La", "Les", "L'", "El", "Los", "Las", "Il", "Lo", "Gli"];

/// The name without its extension: a last `.xyz` of 1 to 5 letters and
/// digits with at least one letter (`Super Mario Bros. 3` keeps its
/// `. 3`, `Vol. 2` its `. 2`).
pub fn strip_extension(name: &str) -> &str {
    let Some(ext) = Path::new(name).extension().and_then(|e| e.to_str()) else { return name };
    let ok = (1..=5).contains(&ext.len()) && ext.chars().all(|c| c.is_ascii_alphanumeric()) && ext.chars().any(|c| c.is_ascii_alphabetic());
    if ok {
        &name[..name.len() - ext.len() - 1]
    } else {
        name
    }
}

/// Everything outside `(…)` and `[…]`.
fn untagged(name: &str) -> String {
    let mut depth = 0u32;
    let out: String = name
        .chars()
        .filter(|c| match c {
            '(' | '[' => {
                depth += 1;
                false
            }
            ')' | ']' => {
                depth = depth.saturating_sub(1);
                false
            }
            _ => depth == 0,
        })
        .collect();
    re!(r"\s+").replace_all(out.trim(), " ").into_owned()
}

/// `Legend of Zelda, The - A Link to the Past` → `The Legend of Zelda - A
/// Link to the Past`; also at the very end (`Addams Family, The`).
fn article_first(title: &str) -> String {
    for article in ARTICLES {
        let tail = format!(", {article}");
        if let Some(head) = title.strip_suffix(tail.as_str()) {
            return join_article(article, head);
        }
        for sep in [" - ", ": "] {
            let marker = format!("{tail}{sep}");
            if let Some(at) = title.find(marker.as_str()) {
                let rest = &title[at + marker.len()..];
                return format!("{}{sep}{rest}", join_article(article, &title[..at]));
            }
        }
    }
    title.to_owned()
}

fn join_article(article: &str, head: &str) -> String {
    if article.ends_with('\'') {
        format!("{article}{head}")
    } else {
        format!("{article} {head}")
    }
}

/// Words after the dump's title that only say which disc or side it is.
fn without_disc(title: &str) -> String {
    re!(r"(?i)[\s,:-]+(?:disc|disk|cd|side)\s*(?:\d+|[a-d])(?:\s*of\s*\d+)?\s*$").replace(title, "").trim().to_owned()
}

/// The title a dump file is of: no extension, no tags, the article back
/// in front, no disc number. `""` when nothing is left.
pub fn dump_title(file_name: &str) -> String {
    let stem = strip_extension(file_name.trim());
    without_disc(&article_first(&untagged(&stem.replace('_', " "))))
}

/// Roman numerals a key turns into digits (`Final Fantasy VII` =
/// `Final Fantasy 7`). Not `I` (a word in English) and not `X`
/// (`Mega Man X` is not `Mega Man 10`), and never a title's first word
/// (`XIII`, `V-Rally`).
const ROMAN: &[(&str, &str)] = &[
    ("ii", "2"),
    ("iii", "3"),
    ("iv", "4"),
    ("v", "5"),
    ("vi", "6"),
    ("vii", "7"),
    ("viii", "8"),
    ("ix", "9"),
    ("xi", "11"),
    ("xii", "12"),
    ("xiii", "13"),
    ("xiv", "14"),
    ("xv", "15"),
    ("xvi", "16"),
];

/// The form two spellings of a title share: [`dump_title`]'s cleaning,
/// `&` read as `and`, accents folded and punctuation dropped
/// ([`title_key`]), safe roman numerals as digits, a leading article
/// dropped, the words joined (`Super Mario Bros.` = `Super Mario Bros`).
/// `""` when nothing is left: never match it.
pub fn match_key(name: &str) -> String {
    let title = dump_title(name).replace('&', " and ");
    let key = title_key(&title);
    let mut words: Vec<&str> = key.split(' ').filter(|w| !w.is_empty()).collect();
    if words.len() > 1 && matches!(words[0], "the" | "a" | "an") {
        words.remove(0);
    }
    words
        .iter()
        .enumerate()
        .map(|(i, w)| if i == 0 { *w } else { ROMAN.iter().find(|(r, _)| r == w).map_or(*w, |(_, n)| *n) })
        .collect::<Vec<_>>()
        .concat()
}

/// [`match_key`] without its last words, one by one, while at least two
/// remain: `Gran Turismo 4 Spec II` → `granturismo4spec`, `granturismo4`,
/// `granturismo`. A database name found this way is a loose match.
pub fn shorter_keys(name: &str) -> Vec<String> {
    let title = dump_title(name);
    let mut words: Vec<&str> = title.split_whitespace().collect();
    let mut out = Vec::new();
    while words.len() > 2 {
        words.pop();
        let key = match_key(&words.join(" "));
        if !key.is_empty() && !out.contains(&key) {
            out.push(key);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Real dump names (No-Intro, Redump, TOSEC, WHDLoad) and the title each is of.
    #[test]
    fn dump_names_as_titles() {
        let cases = [
            ("Super Metroid (Japan, USA) (En,Ja).sfc", "Super Metroid"),
            ("Legend of Zelda, The - A Link to the Past (USA).sfc", "The Legend of Zelda - A Link to the Past"),
            ("Final Fantasy III (USA) (Rev 1).sfc", "Final Fantasy III"),
            ("Sonic The Hedgehog 2 (World) (Rev A).md", "Sonic The Hedgehog 2"),
            ("Castlevania - Symphony of the Night (USA).cue", "Castlevania - Symphony of the Night"),
            ("Metal Gear Solid (USA) (Disc 1) (Rev 1).chd", "Metal Gear Solid"),
            ("Final Fantasy VII (Europe) (Disc 2).chd", "Final Fantasy VII"),
            ("Pokemon - FireRed Version (USA, Europe) (Rev 1).gba", "Pokemon - FireRed Version"),
            ("Bare Knuckle II - Shitou e no Requiem (Japan).md", "Bare Knuckle II - Shitou e no Requiem"),
            ("Addams Family, The (Europe).sfc", "The Addams Family"),
            ("Super Mario Bros. 3 (USA) [!].nes", "Super Mario Bros. 3"),
            ("Progolf (1988)(Atlantis Software).uef", "Progolf"),
            ("Tomb Raider - Disc 1.cue", "Tomb Raider"),
            ("Shadow of the Colossus (USA).iso", "Shadow of the Colossus"),
            ("Mega Man X (USA).sfc", "Mega Man X"),
            ("Some_Homebrew_Game.gba", "Some Homebrew Game"),
            ("(USA).sfc", ""),
        ];
        for (file, title) in cases {
            assert_eq!(dump_title(file), title, "{file}");
        }
    }

    #[test]
    fn keys_two_spellings_share() {
        let same = [
            ("Legend of Zelda, The - A Link to the Past (USA).sfc", "The Legend of Zelda: A Link to the Past"),
            ("Castlevania - Symphony of the Night (USA).cue", "Castlevania: Symphony of the Night"),
            ("Pokemon - FireRed Version (USA, Europe) (Rev 1).gba", "Pokémon FireRed Version"),
            ("Final Fantasy VII (USA) (Disc 1).chd", "Final Fantasy 7"),
            ("Sonic The Hedgehog 2 (World) (Rev A).md", "Sonic the Hedgehog 2"),
            ("Super Mario Bros. 3 (USA).nes", "Super Mario Bros 3"),
            ("Spy vs Spy (USA).nes", "Spy vs. Spy"),
            ("Ren & Stimpy Show, The - Veediots! (USA).sfc", "The Ren and Stimpy Show: Veediots!"),
        ];
        for (a, b) in same {
            assert_eq!(match_key(a), match_key(b), "{a} = {b}");
            assert!(!match_key(a).is_empty());
        }
        let different = [
            ("Mega Man X (USA).sfc", "Mega Man 10"),
            ("Sonic the Hedgehog (USA).md", "Sonic the Hedgehog 2"),
            ("Final Fantasy (USA).nes", "Final Fantasy II"),
            ("Street Fighter II (USA).sfc", "Street Fighter III"),
        ];
        for (a, b) in different {
            assert_ne!(match_key(a), match_key(b), "{a} ≠ {b}");
        }
        assert_eq!(match_key("XIII"), "xiii", "a first word stays");
        assert_eq!(match_key("V-Rally 3"), "vrally3");
        assert_eq!(match_key("The"), "the", "an article alone stays");
        assert_eq!(match_key("[BIOS] (Japan).bin"), "");
    }

    #[test]
    fn shorter_keys_drop_the_last_words() {
        assert_eq!(shorter_keys("Gran Turismo 4 Spec II (1.09).iso"), ["granturismo4spec", "granturismo4", "granturismo"]);
        assert!(shorter_keys("Super Metroid (USA).sfc").is_empty());
    }

    #[test]
    fn extensions() {
        assert_eq!(strip_extension("Super Mario Bros. 3"), "Super Mario Bros. 3");
        assert_eq!(strip_extension("Game.7z"), "Game");
        assert_eq!(strip_extension("Vol. 2"), "Vol. 2");
        assert_eq!(strip_extension("Doom v1.666"), "Doom v1.666");
    }
}
