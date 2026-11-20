//! The file names libretro-thumbnails keys its pictures by, and the names
//! tried for a game.
//!
//! The collection names a picture after the game as RetroArch's playlists
//! do: the dump's name (No-Intro for cartridges, Redump for discs) with
//! `/&*:\`<>?\\|"` replaced by `_`, so `Sonic & Knuckles (World).md` is
//! `Sonic _ Knuckles (World).png`. A dump name is a title, then groups in
//! parentheses: region, languages, then revision and other tags.

use heronite_i18n::Lang;

use crate::rom_names::{dump_title, strip_extension};

/// The characters RetroArch's `gfx_thumbnail_fill_content_img` scrubs; a
/// `/` cannot be in a file name at all.
const SCRUBBED: &[char] = &['&', '*', '/', ':', '`', '"', '<', '>', '?', '\\', '|'];

/// The file name (without `.png`) the collection uses for the name `name`.
pub fn file_name(name: &str) -> String {
    name.chars().map(|c| if SCRUBBED.contains(&c) { '_' } else { c }).collect()
}

/// Regions a No-Intro or Redump name lists, as they write them.
const REGIONS: &[&str] = &[
    "Argentina",
    "Asia",
    "Australia",
    "Austria",
    "Belgium",
    "Brazil",
    "Canada",
    "China",
    "Croatia",
    "Czech",
    "Denmark",
    "Europe",
    "Finland",
    "France",
    "Germany",
    "Greece",
    "Hong Kong",
    "Hungary",
    "India",
    "Ireland",
    "Israel",
    "Italy",
    "Japan",
    "Korea",
    "Latin America",
    "Mexico",
    "Netherlands",
    "New Zealand",
    "Norway",
    "Poland",
    "Portugal",
    "Russia",
    "Scandinavia",
    "Singapore",
    "South Africa",
    "Spain",
    "Sweden",
    "Switzerland",
    "Taiwan",
    "Turkey",
    "UK",
    "Ukraine",
    "United Arab Emirates",
    "Unknown",
    "USA",
    "World",
];

/// The regions to try for a game whose own are not known, best first, for
/// the interface language: its own country, the wide releases, then Japan
/// (a third of a console's collection is Japanese-only). The names are
/// the ones the collection's files carry (`USA, Europe` is one tag).
pub fn regions_for(lang: Lang) -> &'static [&'static str] {
    match lang {
        Lang::En => &["USA", "Europe", "USA, Europe", "World", "Japan"],
        Lang::De => &["Germany", "Europe", "USA", "USA, Europe", "World", "Japan"],
        Lang::Fr => &["France", "Europe", "USA", "USA, Europe", "World", "Japan"],
        Lang::Es => &["Spain", "Europe", "USA", "USA, Europe", "World", "Japan"],
        Lang::PtBr => &["Brazil", "USA", "Europe", "USA, Europe", "World", "Japan"],
    }
}

fn is_region_group(group: &str) -> bool {
    group.split(',').map(str::trim).all(|part| REGIONS.contains(&part))
}

/// `En,Fr,De` and `Zh-Hant`.
fn is_language_group(group: &str) -> bool {
    group.split(',').map(str::trim).all(|part| {
        let (code, script) = part.split_once('-').map_or((part, None), |(c, s)| (c, Some(s)));
        let mut chars = code.chars();
        matches!((chars.next(), chars.next(), chars.next()), (Some(a), Some(b), None) if a.is_ascii_uppercase() && b.is_ascii_lowercase())
            && script.is_none_or(|s| !s.is_empty() && s.chars().all(|c| c.is_ascii_alphabetic()))
    })
}

/// A name's title and its `(…)` groups, without the brackets (`[!]`, `[b]`
/// are dump flags and never part of a picture's name).
struct Split<'a> {
    title: &'a str,
    groups: Vec<&'a str>,
}

fn split(name: &str) -> Split<'_> {
    let name = name.trim();
    let at = name.find(['(', '[']).unwrap_or(name.len());
    let (title, mut rest) = (name[..at].trim_end(), &name[at..]);
    let mut groups = Vec::new();
    while let Some(open) = rest.find(['(', '[']) {
        let (opener, closer) = if rest.as_bytes()[open] == b'(' { ('(', ')') } else { ('[', ']') };
        let mut depth = 0u32;
        let mut end = None;
        for (i, c) in rest[open..].char_indices() {
            if c == opener {
                depth += 1;
            } else if c == closer {
                depth -= 1;
                if depth == 0 {
                    end = Some(open + i);
                    break;
                }
            }
        }
        let Some(end) = end else { break };
        if opener == '(' {
            groups.push(&rest[open + 1..end]);
        }
        rest = &rest[end + 1..];
    }
    Split { title, groups }
}

/// `The Legend of Zelda - A Link to the Past` as the collection writes it:
/// `Legend of Zelda, The - A Link to the Past`. Only English articles: a
/// leading `Die` or `La` is as often a word of the title.
fn article_last(title: &str) -> String {
    for article in ["The", "An", "A"] {
        let Some(rest) = title.strip_prefix(article).and_then(|r| r.strip_prefix(' ')) else { continue };
        return match rest.find(" - ") {
            Some(at) => format!("{}, {article}{}", &rest[..at], &rest[at..]),
            None => format!("{rest}, {article}"),
        };
    }
    title.to_owned()
}

/// A title as No-Intro writes it: `Zelda: Link's Awakening` is
/// `Zelda - Link's Awakening`, and the article is last.
fn no_intro_title(title: &str) -> String {
    article_last(&title.trim().replace(": ", " - "))
}

fn tagged(title: &str, groups: &[&str]) -> String {
    groups.iter().fold(title.to_owned(), |name, group| format!("{name} ({group})"))
}

/// One name to ask for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attempt {
    /// The file name, without `.png`, as the collection spells it.
    pub name: String,
    /// The game's title as people write it (article first, no tags).
    pub title: String,
    /// The dump's own name: a hit is that game on its platform. Only the
    /// first attempt, and only when it is a file's name.
    pub exact: bool,
}

/// The names to ask for, best first, for a game whose ROM is `file` (its
/// title when it has none):
///
/// 1. the file's own name, without its extension (the only sure one;
///    without a file, the title as it is, a guess);
/// 2. without the revision and other tags after the region and the
///    languages (`Super Metroid (Europe) (En,Fr,De) (Rev 1)` →
///    `Super Metroid (Europe) (En,Fr,De)`);
/// 3. with the region alone;
/// 4. the title with each of the regions `regions` names, as No-Intro
///    writes the title (article last, `: ` as ` - `);
/// 5. the title alone, as RetroArch's own last try does.
///
/// The title of 2 to 5 is the file's own when it is a dump name (it has
/// tags); a file without any (`sonic.md`) says nothing of the game, so it
/// is the game's `title`. Names that repeat an earlier one are left out.
pub fn attempts(file: Option<&str>, title: &str, regions: &[&str]) -> Vec<Attempt> {
    let file = file.map(str::trim).filter(|f| !f.is_empty());
    let stem = strip_extension(file.unwrap_or(title.trim()));
    let parts = split(stem);
    let mut out: Vec<Attempt> = Vec::new();
    let mut add = |name: String, exact: bool| {
        let title = dump_title(&name);
        let name = file_name(&name);
        if !name.trim().is_empty() && !out.iter().any(|a| a.name == name) {
            out.push(Attempt { name, title, exact });
        }
    };
    // Only a real file's name is the dump's own: a title is a guess.
    add(stem.to_owned(), file.is_some());
    let kept = parts.groups.iter().take_while(|g| is_region_group(g) || is_language_group(g)).count();
    if kept > 0 {
        add(tagged(parts.title, &parts.groups[..kept]), false);
    }
    if let Some(region) = parts.groups.iter().find(|g| is_region_group(g)) {
        add(tagged(parts.title, &[region]), false);
    }
    let own = split(title).title;
    let title = no_intro_title(if parts.groups.is_empty() && !own.is_empty() { own } else { parts.title });
    for region in regions {
        add(tagged(&title, &[region]), false);
    }
    add(title, false);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(file: &str) -> Vec<String> {
        attempts(Some(file), "", regions_for(Lang::En)).into_iter().map(|a| a.name).collect()
    }

    fn names_of(file: Option<&str>, title: &str) -> Vec<String> {
        attempts(file, title, regions_for(Lang::En)).into_iter().map(|a| a.name).collect()
    }

    #[test]
    fn the_names_the_collection_scrubs() {
        // Real names of the collection (each exists on the server).
        for (dump, file) in [
            ("Sonic & Knuckles (World)", "Sonic _ Knuckles (World)"),
            ("Joe & Mac (USA)", "Joe _ Mac (USA)"),
            ("Puzzle & Action - Tanto-R (Japan)", "Puzzle _ Action - Tanto-R (Japan)"),
            ("Q*bert's Qubes (USA)", "Q_bert's Qubes (USA)"),
            ("Zork Zero: The Revenge of Megaboz", "Zork Zero_ The Revenge of Megaboz"),
            ("Legend of Zelda, The - A Link to the Past (USA)", "Legend of Zelda, The - A Link to the Past (USA)"),
            ("Super Mario World 2 - Yoshi's Island (USA)", "Super Mario World 2 - Yoshi's Island (USA)"),
            ("Is it a/b? <yes>|\"no\" `x` \\", "Is it a_b_ _yes___no_ _x_ _"),
        ] {
            assert_eq!(file_name(dump), file, "{dump}");
        }
    }

    #[test]
    fn groups_and_regions() {
        let s = split("Super Metroid (Japan, USA) (En,Ja) (Rev A) [!]");
        assert_eq!((s.title, s.groups), ("Super Metroid", vec!["Japan, USA", "En,Ja", "Rev A"]));
        let s = split("Pokemon (Europe) (Fr) (Beta (2001-01-01))");
        assert_eq!(s.groups, ["Europe", "Fr", "Beta (2001-01-01)"]);
        let s = split("Burning Heroes [English]");
        assert_eq!((s.title, s.groups.len()), ("Burning Heroes", 0));
        assert_eq!(split("Tetris").title, "Tetris");
        assert!(is_region_group("Japan, USA") && is_region_group("USA, Europe, Korea") && !is_region_group("Rev 1"));
        assert!(is_language_group("En,Fr,De") && is_language_group("Zh-Hant") && is_language_group("Ja"));
        assert!(!is_language_group("Rev A") && !is_language_group("USA") && !is_language_group("Beta"));
    }

    #[test]
    fn a_dump_name_tries_itself_then_shorter_forms() {
        assert_eq!(
            names("Super Metroid (Europe) (En,Fr,De) (Rev 1).sfc")[..4],
            [
                "Super Metroid (Europe) (En,Fr,De) (Rev 1)",
                "Super Metroid (Europe) (En,Fr,De)",
                "Super Metroid (Europe)",
                "Super Metroid (USA)",
            ]
        );
        let list = attempts(Some("Super Metroid (Europe) (En,Fr,De) (Rev 1).sfc"), "", &["USA"]);
        assert_eq!(list.iter().filter(|a| a.exact).count(), 1);
        assert!(list[0].exact);
        // Nothing but a region: no shorter forms, then the regions.
        assert_eq!(
            names("Sonic The Hedgehog (USA).md")[..3],
            ["Sonic The Hedgehog (USA)", "Sonic The Hedgehog (Europe)", "Sonic The Hedgehog (USA, Europe)"]
        );
        // A disc's track and a dump flag go.
        assert_eq!(names("Castlevania - Symphony of the Night (USA) (Track 1).bin")[1], "Castlevania - Symphony of the Night (USA)");
        assert_eq!(names("Game (Japan) [!].zip")[..2], ["Game (Japan) [!]", "Game (Japan)"]);
    }

    #[test]
    fn a_plain_title_is_written_as_the_collection_writes_it() {
        assert_eq!(
            names_of(None, "The Legend of Zelda: A Link to the Past")[..3],
            [
                "The Legend of Zelda_ A Link to the Past",
                "Legend of Zelda, The - A Link to the Past (USA)",
                "Legend of Zelda, The - A Link to the Past (Europe)"
            ]
        );
        assert_eq!(*names_of(None, "The Legend of Zelda: A Link to the Past").last().unwrap(), "Legend of Zelda, The - A Link to the Past");
        assert_eq!(article_last("The Sims"), "Sims, The");
        assert_eq!(article_last("A Bug's Life"), "Bug's Life, A");
        assert_eq!(article_last("The Addams Family - Pugsley's Scavenger Hunt"), "Addams Family, The - Pugsley's Scavenger Hunt");
        assert_eq!(article_last("Theme Park"), "Theme Park");
        assert_eq!(article_last("Anna's Quest"), "Anna's Quest");
        // Plain `Tetris` is asked once for itself and once as the title.
        let tetris = names("Tetris.gb");
        assert_eq!(tetris[0], "Tetris");
        assert_eq!(tetris.iter().filter(|n| *n == "Tetris").count(), 1);
        // A file that says nothing of the game: its title, after the file's own name.
        assert_eq!(
            names_of(Some("sonic.md"), "Sonic The Hedgehog")[..3],
            ["sonic", "Sonic The Hedgehog (USA)", "Sonic The Hedgehog (Europe)"]
        );
        assert_eq!(names_of(Some("Super_Mario_World.smc"), "Super Mario World")[1], "Super Mario World (USA)");
        // A dump name keeps its own title whatever the game is called.
        assert_eq!(names_of(Some("Super Metroid (Europe).sfc"), "Metroid 3")[1], "Super Metroid (USA)");
    }

    #[test]
    fn regions_follow_the_language() {
        let first = |lang| regions_for(lang)[0];
        assert_eq!([first(Lang::En), first(Lang::De), first(Lang::Es), first(Lang::PtBr)], ["USA", "Germany", "Spain", "Brazil"]);
        for lang in Lang::ALL {
            let list = regions_for(lang);
            assert!(list.iter().all(|r| is_region_group(r)), "{lang:?}");
            assert_eq!(list.last(), Some(&"Japan"), "{lang:?}");
        }
    }
}
