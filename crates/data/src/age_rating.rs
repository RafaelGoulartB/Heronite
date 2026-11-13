//! Age ratings: the ESRB, PEGI, USK, ClassInd, CERO, ACB and GRAC boards
//! (plus Steam's own age check), read from IGDB and the Steam store into a
//! game's metadata (`ageRatings`), and the Collection's "ages up to" filter.
//!
//! Each board has its own codes; [`AgeRating::new`] keeps one spelling per
//! code and [`AgeRating::min_age`] puts them on one scale, so a game rated by
//! different boards in different places filters the same way.

use heronite_i18n::{tr, Lang};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RatingSystem {
    Esrb,
    Pegi,
    Usk,
    ClassInd,
    Cero,
    Acb,
    Grac,
    /// The age a Steam store page asks for (`required_age`), for games no
    /// board rated.
    Steam,
}

impl RatingSystem {
    /// The boards a user can choose, in the order the setting lists them.
    pub const BOARDS: [RatingSystem; 7] = [
        RatingSystem::Esrb,
        RatingSystem::Pegi,
        RatingSystem::Usk,
        RatingSystem::ClassInd,
        RatingSystem::Acb,
        RatingSystem::Cero,
        RatingSystem::Grac,
    ];

    /// What `ageRatings[].system` and the setting store.
    pub fn id(self) -> &'static str {
        match self {
            Self::Esrb => "esrb",
            Self::Pegi => "pegi",
            Self::Usk => "usk",
            Self::ClassInd => "classind",
            Self::Cero => "cero",
            Self::Acb => "acb",
            Self::Grac => "grac",
            Self::Steam => "steam",
        }
    }

    pub fn parse(id: &str) -> Option<Self> {
        Self::BOARDS.into_iter().chain([Self::Steam]).find(|s| s.id() == id)
    }

    /// The board's own name (not translated).
    pub fn name(self) -> &'static str {
        match self {
            Self::Esrb => "ESRB",
            Self::Pegi => "PEGI",
            Self::Usk => "USK",
            Self::ClassInd => "ClassInd",
            Self::Cero => "CERO",
            Self::Acb => "ACB",
            Self::Grac => "GRAC",
            Self::Steam => "Steam",
        }
    }

    /// Where the board rates games.
    pub fn region(self) -> &'static str {
        match self {
            Self::Esrb => tr!("United States and Canada"),
            Self::Pegi => tr!("Europe"),
            Self::Usk => tr!("Germany"),
            Self::ClassInd => tr!("Brazil"),
            Self::Cero => tr!("Japan"),
            Self::Acb => tr!("Australia"),
            Self::Grac => tr!("South Korea"),
            Self::Steam => tr!("Steam store"),
        }
    }

    /// The board IGDB names `name` (`age_rating_organizations.name`).
    pub fn from_igdb(name: &str) -> Option<Self> {
        Some(match name.trim().to_ascii_uppercase().as_str() {
            "ESRB" => Self::Esrb,
            "PEGI" => Self::Pegi,
            "USK" => Self::Usk,
            "CLASS_IND" | "CLASSIND" => Self::ClassInd,
            "CERO" => Self::Cero,
            "ACB" => Self::Acb,
            "GRAC" => Self::Grac,
            _ => return None,
        })
    }

    /// The board behind a key of the Steam store's `ratings` object. Steam's
    /// own German and Australian ratings stand in for the board's when the
    /// board has none ([`merge`] keeps the first of a board).
    pub fn from_steam(key: &str) -> Option<Self> {
        Some(match key {
            "esrb" => Self::Esrb,
            "pegi" => Self::Pegi,
            "usk" | "steam_germany" => Self::Usk,
            "dejus" => Self::ClassInd,
            "cero" => Self::Cero,
            "oflc" | "steam_australia" => Self::Acb,
            "kgrb" => Self::Grac,
            _ => return None,
        })
    }
}

/// One board's rating of a game, as stored in `ageRatings`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgeRating {
    /// [`RatingSystem::id`].
    pub system: String,
    /// The board's code in one spelling: `EC` `E` `E10+` `T` `M` `AO` `RP`
    /// (ESRB), `L` (ClassInd), `A`-`D` `Z` (CERO), `G` `PG` `M` `MA15+`
    /// `R18+` `X18+` `RC` (ACB), `ALL` (GRAC), otherwise the age.
    pub rating: String,
    /// Why it got the rating ("Blood", "Mild Violence"...), as the source
    /// wrote it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub descriptors: Vec<String>,
}

impl AgeRating {
    /// `None` for a code the board does not have (or IGDB's `TESTING`,
    /// Steam's `BANNED`).
    pub fn new(system: RatingSystem, raw: &str, descriptors: Vec<String>) -> Option<Self> {
        let rating = normalize(system, raw)?;
        let descriptors = descriptors.into_iter().map(|d| d.trim().to_owned()).filter(|d| !d.is_empty()).collect();
        Some(Self { system: system.id().to_owned(), rating, descriptors })
    }

    pub fn board(&self) -> Option<RatingSystem> {
        RatingSystem::parse(&self.system)
    }

    /// The youngest age the rating is for, `None` while it is pending (ESRB
    /// `RP`) or unknown.
    pub fn min_age(&self) -> Option<u8> {
        let r = self.rating.as_str();
        let number = || r.parse::<u8>().ok();
        match self.board()? {
            RatingSystem::Esrb => match r {
                "EC" => Some(3),
                "E" => Some(6),
                "E10+" => Some(10),
                "T" => Some(13),
                "M" => Some(17),
                "AO" => Some(18),
                _ => None,
            },
            RatingSystem::ClassInd if r == "L" => Some(0),
            RatingSystem::Cero => match r {
                "A" => Some(0),
                "B" => Some(12),
                "C" => Some(15),
                "D" => Some(17),
                "Z" => Some(18),
                _ => None,
            },
            // PG is advice for under-15s; M is "recommended for 15 and up".
            RatingSystem::Acb => match r {
                "G" => Some(0),
                "PG" => Some(8),
                "M" | "MA15+" => Some(15),
                "R18+" | "X18+" | "RC" => Some(18),
                _ => None,
            },
            RatingSystem::Grac if r == "ALL" => Some(0),
            _ => number(),
        }
    }

    /// The short code on the badge: `T`, `12`, `MA 15+`, `18+`.
    pub fn badge(&self) -> String {
        match (self.board(), self.rating.as_str()) {
            (Some(RatingSystem::Acb), r) if r.ends_with('+') && r.len() > 2 => {
                let (head, age) = r.split_at(r.find(|c: char| c.is_ascii_digit()).unwrap_or(0));
                format!("{head} {age}")
            }
            (Some(RatingSystem::Steam), r) => format!("{r}+"),
            (_, r) => r.to_owned(),
        }
    }

    /// What the rating means: the ESRB's own category names (proper names,
    /// left in English), "All ages" or "Ages 12 and up" for the others.
    pub fn title(&self) -> String {
        if self.board() == Some(RatingSystem::Esrb) {
            let name = match self.rating.as_str() {
                "EC" => "Early Childhood",
                "E" => "Everyone",
                "E10+" => "Everyone 10+",
                "T" => "Teen",
                "M" => "Mature 17+",
                "AO" => "Adults Only 18+",
                _ => "Rating Pending",
            };
            return name.to_owned();
        }
        match self.min_age() {
            Some(0) => tr!("All ages").to_owned(),
            Some(age) => tr!("Ages {age} and up", age),
            None => tr!("Rating pending").to_owned(),
        }
    }
}

/// The board's code of `raw` in [`AgeRating::rating`]'s spelling. Steam
/// writes them in lower case (`e10`, `ma15`, `r18`), IGDB as printed on the
/// box (`E10+`, `MA 15+`, `15+`).
fn normalize(system: RatingSystem, raw: &str) -> Option<String> {
    let code: String = raw.chars().filter(|c| !c.is_whitespace()).collect::<String>().to_ascii_uppercase();
    let bare = code.trim_end_matches('+');
    let age_in = |allowed: &[u8]| bare.parse::<u8>().ok().filter(|a| allowed.contains(a)).map(|a| a.to_string());
    match system {
        RatingSystem::Esrb => match bare {
            "EC" | "E" | "T" | "M" | "AO" | "RP" => Some(bare.to_owned()),
            "E10" => Some("E10+".to_owned()),
            _ => None,
        },
        RatingSystem::Pegi => age_in(&[3, 7, 12, 16, 18]),
        RatingSystem::Usk => age_in(&[0, 6, 12, 16, 18]),
        RatingSystem::ClassInd => match bare {
            "L" | "AL" => Some("L".to_owned()),
            _ => age_in(&[10, 12, 14, 16, 18]),
        },
        RatingSystem::Cero => matches!(bare, "A" | "B" | "C" | "D" | "Z").then(|| bare.to_owned()),
        RatingSystem::Acb => match bare {
            "G" | "PG" | "M" | "RC" => Some(bare.to_owned()),
            "MA15" | "R18" | "X18" => Some(format!("{bare}+")),
            _ => None,
        },
        RatingSystem::Grac => match bare {
            "ALL" => Some("ALL".to_owned()),
            _ => age_in(&[12, 15, 18, 19]),
        },
        RatingSystem::Steam => bare.parse::<u8>().ok().filter(|a| (1..=21).contains(a)).map(|a| a.to_string()),
    }
}

/// One rating per board, the first list's first: later lists only add the
/// boards the earlier ones lack, and the reasons for a rating that came
/// without them.
pub fn merge<'a>(lists: impl IntoIterator<Item = &'a [AgeRating]>) -> Vec<AgeRating> {
    let mut out: Vec<AgeRating> = Vec::new();
    for rating in lists.into_iter().flatten() {
        match out.iter_mut().find(|r| r.system == rating.system) {
            Some(kept) => {
                if kept.descriptors.is_empty() && kept.rating == rating.rating {
                    kept.descriptors = rating.descriptors.clone();
                }
            }
            None => out.push(rating.clone()),
        }
    }
    out
}

/// `metadata.ageRatingSystem`: which board the Collection shows first.
pub const AUTO: &str = "auto";

/// The boards in the order the Collection looks for a game's rating:
/// `preference` ([`RatingSystem::id`], or [`AUTO`] for the board of the
/// interface language's main country), then the others.
pub fn board_order(preference: &str, lang: Lang) -> Vec<RatingSystem> {
    let first = RatingSystem::parse(preference).unwrap_or(match lang {
        Lang::En => RatingSystem::Esrb,
        Lang::PtBr => RatingSystem::ClassInd,
        Lang::Es | Lang::Fr => RatingSystem::Pegi,
        Lang::De => RatingSystem::Usk,
    });
    let mut order = vec![first];
    order.extend(RatingSystem::BOARDS.into_iter().chain([RatingSystem::Steam]).filter(|s| *s != first));
    order
}

/// The rating the Collection shows and filters by: the first board of
/// `order` that rated the game (a pending rating only when there is nothing
/// else).
pub fn shown<'a>(ratings: &'a [AgeRating], order: &[RatingSystem]) -> Option<&'a AgeRating> {
    let rated = order.iter().find_map(|s| ratings.iter().find(|r| r.board() == Some(*s) && r.min_age().is_some()));
    rated.or_else(|| ratings.iter().find(|r| r.board().is_some()))
}

/// The View menu's "Ages up to" filter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct AgeFilter {
    /// Hides games rated for older players; `None` = off.
    pub max: Option<u8>,
    /// While on, also shows the games with no rating.
    pub unrated: bool,
}

impl AgeFilter {
    /// The ages the menu offers.
    pub const STEPS: [u8; 4] = [7, 10, 12, 16];

    pub fn is_active(&self) -> bool {
        self.max.is_some()
    }

    /// Whether a game whose [`shown`] rating is for `min_age` passes.
    pub fn allows(&self, min_age: Option<u8>) -> bool {
        match (self.max, min_age) {
            (None, _) => true,
            (Some(max), Some(age)) => age <= max,
            (Some(_), None) => self.unrated,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(system: RatingSystem, raw: &str) -> AgeRating {
        AgeRating::new(system, raw, Vec::new()).unwrap()
    }

    #[test]
    fn steam_and_igdb_spellings_meet() {
        assert_eq!(r(RatingSystem::Esrb, "e10").rating, "E10+");
        assert_eq!(r(RatingSystem::Esrb, "E10+").rating, "E10+");
        assert_eq!(r(RatingSystem::Acb, "ma15").rating, "MA15+");
        assert_eq!(r(RatingSystem::Acb, "MA 15+").rating, "MA15+");
        assert_eq!(r(RatingSystem::Grac, "15+").rating, "15");
        assert_eq!(r(RatingSystem::ClassInd, "l").rating, "L");
        assert!(AgeRating::new(RatingSystem::Grac, "TESTING", vec![]).is_none());
        assert!(AgeRating::new(RatingSystem::Pegi, "13", vec![]).is_none());
        assert!(AgeRating::new(RatingSystem::Steam, "0", vec![]).is_none());
    }

    #[test]
    fn ages_and_badges() {
        assert_eq!(r(RatingSystem::Esrb, "t").min_age(), Some(13));
        assert_eq!(r(RatingSystem::Esrb, "RP").min_age(), None);
        assert_eq!(r(RatingSystem::ClassInd, "L").min_age(), Some(0));
        assert_eq!(r(RatingSystem::Cero, "Z").min_age(), Some(18));
        assert_eq!(r(RatingSystem::Acb, "r18").badge(), "R 18+");
        assert_eq!(r(RatingSystem::Steam, "18").badge(), "18+");
        assert_eq!(r(RatingSystem::Esrb, "M").title(), "Mature 17+");
        assert_eq!(r(RatingSystem::Pegi, "12").title(), "Ages 12 and up");
        assert_eq!(r(RatingSystem::Usk, "0").title(), "All ages");
    }

    #[test]
    fn first_list_wins_per_board_and_lends_its_reasons() {
        let igdb = vec![r(RatingSystem::Esrb, "T"), r(RatingSystem::Pegi, "12")];
        let steam = vec![
            AgeRating::new(RatingSystem::Pegi, "12", vec!["Violence".into()]).unwrap(),
            r(RatingSystem::Pegi, "16"),
            r(RatingSystem::Usk, "12"),
        ];
        let merged = merge([igdb.as_slice(), steam.as_slice()]);
        assert_eq!(merged.len(), 3);
        assert_eq!(merged[1].descriptors, ["Violence"]);
        assert_eq!(merged[2].system, "usk");
    }

    #[test]
    fn shows_the_preferred_board_then_the_next_one() {
        let ratings = vec![r(RatingSystem::Esrb, "RP"), r(RatingSystem::Pegi, "16"), r(RatingSystem::ClassInd, "14")];
        assert_eq!(board_order(AUTO, Lang::Fr)[0], RatingSystem::Pegi);
        let en = board_order(AUTO, Lang::En);
        assert_eq!(shown(&ratings, &en).unwrap().system, "pegi", "pending ESRB is skipped");
        assert_eq!(shown(&ratings, &board_order(AUTO, Lang::PtBr)).unwrap().system, "classind");
        assert_eq!(shown(&ratings, &board_order("pegi", Lang::PtBr)).unwrap().system, "pegi");
        let pending = vec![r(RatingSystem::Esrb, "RP")];
        assert_eq!(shown(&pending, &en).unwrap().rating, "RP");
        assert!(shown(&[], &en).is_none());
    }

    #[test]
    fn filter_hides_older_and_maybe_unrated() {
        let up_to_12 = AgeFilter { max: Some(12), unrated: false };
        assert!(up_to_12.allows(Some(12)));
        assert!(!up_to_12.allows(Some(13)));
        assert!(!up_to_12.allows(None));
        assert!(AgeFilter { unrated: true, ..up_to_12 }.allows(None));
        assert!(AgeFilter::default().allows(Some(18)));
    }
}
