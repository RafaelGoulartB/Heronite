//! Per-field merge of provider candidates: exact port of `metadata/merge.ts`.

use heronite_data::age_rating::{self, AgeRating};
use heronite_data::models::CollectionGameMetadata;
use heronite_data::num::JsNum;
use heronite_data::settings::{ALL_FIELDS, METADATA_FIELDS};
use indexmap::IndexMap;

use crate::util::{blank_metadata, now_iso};

/// [`METADATA_FIELDS`] entry of `ageRatings`.
pub const AGE_RATINGS: &str = "ageRatings";
/// Key in a record's `extra`: when IGDB or Steam were last asked for its
/// age ratings (records stored before ratings were read have none).
pub const AGE_CHECKED: &str = "ageRatingsCheckedAt";

/// A field value as read from a metadata record.
#[derive(Debug, Clone, PartialEq)]
pub enum FieldValue {
    Text(String),
    Number(f64),
    List(Vec<String>),
    AgeRatings(Vec<AgeRating>),
}

fn list_of<'a>(md: &'a CollectionGameMetadata, field: &str) -> Option<&'a Vec<String>> {
    Some(match field {
        "developers" => &md.developers,
        "publishers" => &md.publishers,
        "genres" => &md.genres,
        "themes" => &md.themes,
        "gameModes" => &md.game_modes,
        "platforms" => &md.platforms,
        "features" => &md.features,
        _ => return None,
    })
}

fn list_of_mut<'a>(md: &'a mut CollectionGameMetadata, field: &str) -> Option<&'a mut Vec<String>> {
    Some(match field {
        "developers" => &mut md.developers,
        "publishers" => &mut md.publishers,
        "genres" => &mut md.genres,
        "themes" => &mut md.themes,
        "gameModes" => &mut md.game_modes,
        "platforms" => &mut md.platforms,
        "features" => &mut md.features,
        _ => return None,
    })
}

fn text_of<'a>(md: &'a CollectionGameMetadata, field: &str) -> Option<&'a Option<String>> {
    Some(match field {
        "description" => &md.description,
        "releaseDate" => &md.release_date,
        "series" => &md.series,
        "cover" => &md.cover_url,
        "hero" => &md.hero_url,
        "coverSquare" => &md.cover_square_url,
        "coverWide" => &md.cover_wide_url,
        _ => return None,
    })
}

/// `hasFieldValue`.
pub fn has_field_value(metadata: Option<&CollectionGameMetadata>, field: &str) -> bool {
    let Some(md) = metadata else { return false };
    if field == "criticScore" {
        return md.critic_score.is_some();
    }
    if field == AGE_RATINGS {
        return !md.age_ratings.is_empty();
    }
    if let Some(text) = text_of(md, field) {
        return text.as_deref().is_some_and(|s| !s.is_empty());
    }
    list_of(md, field).is_some_and(|l| !l.is_empty())
}

/// `readField`.
pub fn read_field(metadata: Option<&CollectionGameMetadata>, field: &str) -> Option<FieldValue> {
    let md = metadata?;
    if field == "criticScore" {
        return md.critic_score.map(|n| FieldValue::Number(n.get()));
    }
    if field == AGE_RATINGS {
        return Some(FieldValue::AgeRatings(md.age_ratings.clone()));
    }
    if let Some(text) = text_of(md, field) {
        return text.clone().map(FieldValue::Text);
    }
    list_of(md, field).map(|l| FieldValue::List(l.clone()))
}

/// Sets the value of `field`; `false` (nothing set) for a mismatched value
/// type, ignored like the TS.
fn set_field(target: &mut CollectionGameMetadata, field: &str, value: FieldValue) -> bool {
    match (field, value) {
        ("description", FieldValue::Text(v)) => target.description = Some(v),
        ("releaseDate", FieldValue::Text(v)) => target.release_date = Some(v),
        ("series", FieldValue::Text(v)) => target.series = Some(v),
        ("criticScore", FieldValue::Number(v)) => target.critic_score = Some(JsNum(v)),
        ("cover", FieldValue::Text(v)) => target.cover_url = Some(v),
        ("hero", FieldValue::Text(v)) => target.hero_url = Some(v),
        ("coverSquare", FieldValue::Text(v)) => target.cover_square_url = Some(v),
        ("coverWide", FieldValue::Text(v)) => target.cover_wide_url = Some(v),
        (AGE_RATINGS, FieldValue::AgeRatings(v)) => target.age_ratings = v,
        (f, FieldValue::List(v)) if list_of(target, f).is_some() => {
            *list_of_mut(target, f).expect("checked") = v;
        }
        _ => return false,
    }
    true
}

/// `writeField`: sets the value and records its source.
fn write_field(target: &mut CollectionGameMetadata, field: &str, value: FieldValue, source: &str) {
    if set_field(target, field, value) {
        target.field_sources.insert(field.to_owned(), source.to_owned());
    }
}

/// Makes `field` of `target` what it is in `from` (empty when `from` is
/// `None`), its recorded source included.
pub fn copy_field(from: Option<&CollectionGameMetadata>, target: &mut CollectionGameMetadata, field: &str) {
    clear_field(target, field);
    if let Some(value) = read_field(from, field) {
        set_field(target, field, value);
    }
    match from.and_then(|f| f.field_sources.get(field)) {
        Some(source) => {
            target.field_sources.insert(field.to_owned(), source.clone());
        }
        None => {
            target.field_sources.shift_remove(field);
        }
    }
}

/// Empties `field` of `md` (a `METADATA_FIELDS` entry; others are ignored).
pub fn clear_field(md: &mut CollectionGameMetadata, field: &str) {
    match field {
        "criticScore" => md.critic_score = None,
        AGE_RATINGS => md.age_ratings.clear(),
        f => {
            if let Some(list) = list_of_mut(md, f) {
                list.clear();
            } else if let Some(text) = text_of_mut(md, f) {
                *text = None;
            }
        }
    }
}

fn text_of_mut<'a>(md: &'a mut CollectionGameMetadata, field: &str) -> Option<&'a mut Option<String>> {
    Some(match field {
        "description" => &mut md.description,
        "releaseDate" => &mut md.release_date,
        "series" => &mut md.series,
        "cover" => &mut md.cover_url,
        "hero" => &mut md.hero_url,
        "coverSquare" => &mut md.cover_square_url,
        "coverWide" => &mut md.cover_wide_url,
        _ => return None,
    })
}

/// A provider result handed to [`merge_metadata`].
pub struct MergeCandidate<'a> {
    pub source: &'a str,
    pub metadata: &'a CollectionGameMetadata,
}

pub struct MergeArgs<'a> {
    pub app_name: &'a str,
    pub runner: &'a str,
    pub current: Option<&'a CollectionGameMetadata>,
    pub candidates: &'a [MergeCandidate<'a>],
    /// field -> sources in priority order.
    pub priority: &'a IndexMap<String, Vec<String>>,
    pub overwrite_existing: bool,
    pub preserve_manual: bool,
    /// field -> source | `"keep"` | `"manual"`.
    pub field_picks: Option<&'a IndexMap<String, String>>,
    /// Only these fields are merged; the others stay as they are, their
    /// source too. `None`: every `METADATA_FIELDS` entry (the square and
    /// wide covers only when named).
    pub fields: Option<&'a [String]>,
}

/// `mergeMetadata`.
pub fn merge_metadata(args: MergeArgs<'_>) -> CollectionGameMetadata {
    let mut next = args.current.cloned().unwrap_or_else(|| blank_metadata(args.app_name, args.runner));

    let by_source: IndexMap<&str, &CollectionGameMetadata> = args.candidates.iter().map(|c| (c.source, c.metadata)).collect();

    let chosen = |field: &str| match args.fields {
        Some(f) => f.iter().any(|c| c == field),
        None => METADATA_FIELDS.contains(&field),
    };
    for field in ALL_FIELDS.into_iter().filter(|f| chosen(f)) {
        let pick = args.field_picks.and_then(|p| p.get(field)).map(String::as_str);
        if matches!(pick, Some("keep") | Some("manual")) {
            let kept = args.current.and_then(|c| c.field_sources.get(field)).cloned().unwrap_or_else(|| "manual".to_owned());
            next.field_sources.insert(field.to_owned(), kept);
            continue;
        }
        let locked_manual = pick.is_none()
            && args.preserve_manual
            && args.current.and_then(|c| c.field_sources.get(field)).map(String::as_str) == Some("manual");
        if locked_manual {
            continue;
        }
        if !args.overwrite_existing && has_field_value(args.current, field) && pick.is_none() {
            continue;
        }

        let configured: &[String] = args.priority.get(field).map(Vec::as_slice).unwrap_or(&[]);
        let order: Vec<&str> = match pick {
            Some(p) => std::iter::once(p).chain(configured.iter().map(String::as_str).filter(|s| *s != p)).collect(),
            None => configured.iter().map(String::as_str).collect(),
        };

        // Each source rates for the boards it knows: all of them add up,
        // the first in the order winning a board both have.
        if field == AGE_RATINGS {
            let rated: Vec<(&str, &CollectionGameMetadata)> = order
                .iter()
                .filter(|s| **s != "manual")
                .filter_map(|s| by_source.get(s).map(|md| (*s, *md)))
                .filter(|(_, md)| !md.age_ratings.is_empty())
                .collect();
            if let Some((first, _)) = rated.first() {
                let merged = age_rating::merge(rated.iter().map(|(_, md)| md.age_ratings.as_slice()));
                write_field(&mut next, field, FieldValue::AgeRatings(merged), first);
            }
            continue;
        }
        for source in order {
            if source == "manual" {
                continue;
            }
            let candidate = by_source.get(source).copied();
            if !has_field_value(candidate, field) {
                continue;
            }
            if let Some(value) = read_field(candidate, field) {
                write_field(&mut next, field, value, source);
            }
            break;
        }
    }

    for candidate in args.candidates {
        let md = candidate.metadata;
        if md.igdb_id.is_some_and(|i| i.get() != 0.0) && !next.igdb_id.is_some_and(|i| i.get() != 0.0) {
            next.igdb_id = md.igdb_id;
            next.igdb_slug = md.igdb_slug.clone();
        }
        if md.lutris_slug.as_deref().is_some_and(|s| !s.is_empty()) && !next.lutris_slug.as_deref().is_some_and(|s| !s.is_empty()) {
            next.lutris_slug = md.lutris_slug.clone();
        }
        if md.steam_app_id.as_deref().is_some_and(|s| !s.is_empty()) && !next.steam_app_id.as_deref().is_some_and(|s| !s.is_empty()) {
            next.steam_app_id = md.steam_app_id.clone();
        }
        if md.title.as_deref().is_some_and(|s| !s.is_empty()) && !next.title.as_deref().is_some_and(|s| !s.is_empty()) {
            next.title = md.title.clone();
        }
        // The Steam store header follows the newest fetch (its URL changes).
        if let Some(header) = md.extra.get(crate::steam::HEADER_URL).filter(|v| v.as_str().is_some_and(|s| !s.is_empty())) {
            next.extra.insert(crate::steam::HEADER_URL.into(), header.clone());
        }
        if !md.websites.is_empty() && next.websites.is_empty() {
            next.websites = md.websites.clone();
        }
    }

    // Asked only when the ratings were: the age ratings fill skips a record marked asked.
    if chosen(AGE_RATINGS) && args.candidates.iter().any(|c| matches!(c.source, "igdb" | "steam")) {
        next.extra.insert(AGE_CHECKED.into(), serde_json::Value::String(now_iso()));
    }
    next.app_name = args.app_name.to_owned();
    next.runner = args.runner.to_owned();
    next.fetched_at = Some(now_iso());
    next.fetch_error = None;
    next
}

/// `withPreferredSource`: puts `source` first for every field (no-op for
/// `None` / `"auto"`).
pub fn with_preferred_source(priority: &IndexMap<String, Vec<String>>, source: Option<&str>) -> IndexMap<String, Vec<String>> {
    let Some(source) = source.filter(|s| *s != "auto") else {
        return priority.clone();
    };
    let mut next = priority.clone();
    for field in ALL_FIELDS {
        let rest: Vec<String> =
            priority.get(field).map(|l| l.iter().filter(|s| s.as_str() != source).cloned().collect()).unwrap_or_default();
        let mut list = vec![source.to_owned()];
        list.extend(rest);
        next.insert(field.to_owned(), list);
    }
    next
}

#[cfg(test)]
mod tests {
    use super::*;
    use heronite_data::settings::default_field_priority;

    fn priority() -> IndexMap<String, Vec<String>> {
        METADATA_FIELDS.iter().map(|f| (f.to_string(), default_field_priority(f))).collect()
    }

    fn md(f: impl FnOnce(&mut CollectionGameMetadata)) -> CollectionGameMetadata {
        let mut m = blank_metadata("a", "sideload");
        f(&mut m);
        m
    }

    fn merge<'a>(
        current: Option<&'a CollectionGameMetadata>,
        cands: &'a [MergeCandidate<'a>],
        pr: &'a IndexMap<String, Vec<String>>,
        overwrite: bool,
        preserve: bool,
        picks: Option<&'a IndexMap<String, String>>,
    ) -> CollectionGameMetadata {
        merge_metadata(MergeArgs {
            app_name: "a",
            runner: "sideload",
            current,
            candidates: cands,
            priority: pr,
            overwrite_existing: overwrite,
            preserve_manual: preserve,
            field_picks: picks,
            fields: None,
        })
    }

    #[test]
    fn highest_priority_source_with_a_value_wins_per_field() {
        let igdb = md(|m| {
            m.description = Some("igdb text".into());
            m.genres = vec!["RPG".into()];
            m.igdb_id = Some(JsNum(7.0));
            m.igdb_slug = Some("slug".into());
            m.title = Some("Title".into());
        });
        let steam = md(|m| {
            m.description = Some("steam text".into());
            m.release_date = Some("2020".into());
            m.features = vec!["Co-op".into()];
            m.critic_score = Some(JsNum(0.0));
            m.steam_app_id = Some("42".into());
        });
        let pr = priority();
        let cands = [MergeCandidate { source: "igdb", metadata: &igdb }, MergeCandidate { source: "steam", metadata: &steam }];
        let out = merge(None, &cands, &pr, true, false, None);
        assert_eq!(out.description.as_deref(), Some("igdb text"));
        assert_eq!(out.field_sources["description"], "igdb");
        assert_eq!(out.release_date.as_deref(), Some("2020"));
        assert_eq!(out.field_sources["releaseDate"], "steam");
        assert_eq!(out.field_sources["features"], "steam");
        // 0 is a score (typeof number), not an empty value.
        assert_eq!(out.critic_score, Some(JsNum(0.0)));
        assert_eq!(out.igdb_id, Some(JsNum(7.0)));
        assert_eq!(out.igdb_slug.as_deref(), Some("slug"));
        assert_eq!(out.steam_app_id.as_deref(), Some("42"));
        assert_eq!(out.title.as_deref(), Some("Title"));
        assert!(out.fetched_at.is_some() && out.fetch_error.is_none());
        assert!(!out.field_sources.contains_key("themes"));
    }

    #[test]
    fn existing_values_kept_without_overwrite_and_manual_locked() {
        let current = md(|m| {
            m.description = Some("mine".into());
            m.field_sources.insert("description".into(), "manual".into());
            m.genres = vec!["Old".into()];
            m.field_sources.insert("genres".into(), "igdb".into());
            m.fetch_error = Some("boom".into());
        });
        let igdb = md(|m| {
            m.description = Some("igdb".into());
            m.genres = vec!["New".into()];
            m.publishers = vec!["Pub".into()];
        });
        let pr = priority();
        let cands = [MergeCandidate { source: "igdb", metadata: &igdb }];

        // overwrite=false: filled fields stay, empty ones get filled.
        let out = merge(Some(&current), &cands, &pr, false, false, None);
        assert_eq!(out.description.as_deref(), Some("mine"));
        assert_eq!(out.genres, ["Old"]);
        assert_eq!(out.publishers, ["Pub"]);
        assert!(out.fetch_error.is_none());

        // overwrite=true + preserve_manual: only the manual field is locked.
        let out = merge(Some(&current), &cands, &pr, true, true, None);
        assert_eq!(out.description.as_deref(), Some("mine"));
        assert_eq!(out.field_sources["description"], "manual");
        assert_eq!(out.genres, ["New"]);

        // overwrite=true, preserve_manual=false: manual is replaced.
        let out = merge(Some(&current), &cands, &pr, true, false, None);
        assert_eq!(out.description.as_deref(), Some("igdb"));
        assert_eq!(out.field_sources["description"], "igdb");
    }

    #[test]
    fn field_picks() {
        let current = md(|m| {
            m.description = Some("keepme".into());
            m.genres = vec!["G".into()];
        });
        let igdb = md(|m| {
            m.description = Some("igdb".into());
            m.genres = vec!["IG".into()];
        });
        let steam = md(|m| {
            m.genres = vec!["SG".into()];
            m.description = Some("steam".into());
        });
        let pr = priority();
        let cands = [MergeCandidate { source: "igdb", metadata: &igdb }, MergeCandidate { source: "steam", metadata: &steam }];
        let picks: IndexMap<String, String> = [
            ("description".to_string(), "keep".to_string()),
            ("genres".to_string(), "steam".to_string()),
            ("series".to_string(), "lutris".to_string()),
        ]
        .into_iter()
        .collect();
        let out = merge(Some(&current), &cands, &pr, false, false, Some(&picks));
        assert_eq!(out.description.as_deref(), Some("keepme"));
        // 'keep' without a recorded source marks the field manual.
        assert_eq!(out.field_sources["description"], "manual");
        // A pick forces replacement even without overwrite.
        assert_eq!(out.genres, ["SG"]);
        assert_eq!(out.field_sources["genres"], "steam");
        // A pick whose source has nothing falls back to the priority list.
        let with_series = md(|m| m.series = Some("S".into()));
        let cands2 = [MergeCandidate { source: "igdb", metadata: &with_series }];
        let out = merge(None, &cands2, &pr, true, false, Some(&picks));
        assert_eq!(out.series.as_deref(), Some("S"));
        assert_eq!(out.field_sources["series"], "igdb");
    }

    #[test]
    fn preferred_source_goes_first() {
        let pr = priority();
        let out = with_preferred_source(&pr, Some("lutris"));
        assert_eq!(out["description"], ["lutris", "igdb", "steam", "store", "playnite", "launchbox"]);
        assert_eq!(out["gameModes"], ["lutris", "igdb", "launchbox"]);
        assert_eq!(with_preferred_source(&pr, Some("auto")), pr);
        assert_eq!(with_preferred_source(&pr, None), pr);
    }

    #[test]
    fn a_cleared_field_reads_as_empty() {
        let mut m = md(|m| {
            m.description = Some("d".into());
            m.genres = vec!["G".into()];
            m.critic_score = Some(JsNum(80.0));
            m.cover_url = Some("c".into());
            m.title = Some("T".into());
        });
        m.cover_square_url = Some("s".into());
        m.cover_wide_url = Some("w".into());
        for field in ALL_FIELDS {
            clear_field(&mut m, field);
            assert!(!has_field_value(Some(&m), field), "{field}");
        }
        assert_eq!(m.title.as_deref(), Some("T"), "not a field: kept");
    }

    #[test]
    fn websites_and_slugs_only_fill_gaps() {
        let current = md(|m| m.lutris_slug = Some("have".into()));
        let lutris = md(|m| {
            m.lutris_slug = Some("other".into());
            m.websites = vec!["https://x".into()];
        });
        let pr = priority();
        let cands = [MergeCandidate { source: "lutris", metadata: &lutris }];
        let out = merge(Some(&current), &cands, &pr, false, false, None);
        assert_eq!(out.lutris_slug.as_deref(), Some("have"));
        assert_eq!(out.websites, ["https://x"]);
    }

    #[test]
    fn age_ratings_add_up_with_the_first_source_winning_a_board() {
        use heronite_data::age_rating::RatingSystem;
        let rating = |s, r| AgeRating::new(s, r, Vec::new()).unwrap();
        let igdb = md(|m| m.age_ratings = vec![rating(RatingSystem::Esrb, "T"), rating(RatingSystem::Pegi, "12")]);
        let steam = md(|m| m.age_ratings = vec![rating(RatingSystem::Pegi, "16"), rating(RatingSystem::ClassInd, "10")]);
        let pr = priority();
        let cands = [MergeCandidate { source: "steam", metadata: &steam }, MergeCandidate { source: "igdb", metadata: &igdb }];
        let out = merge(None, &cands, &pr, true, false, None);
        let got: Vec<(&str, &str)> = out.age_ratings.iter().map(|r| (r.system.as_str(), r.rating.as_str())).collect();
        assert_eq!(got, [("esrb", "T"), ("pegi", "12"), ("classind", "10")]);
        assert_eq!(out.field_sources[AGE_RATINGS], "igdb");
        assert!(out.extra.contains_key(AGE_CHECKED));
        // Only Steam rated it.
        let bare = md(|_| {});
        let cands = [MergeCandidate { source: "igdb", metadata: &bare }, MergeCandidate { source: "steam", metadata: &steam }];
        assert_eq!(merge(None, &cands, &pr, true, false, None).field_sources[AGE_RATINGS], "steam");
    }

    #[test]
    fn only_the_chosen_fields_are_merged() {
        use heronite_data::age_rating::RatingSystem;
        let current = md(|m| {
            m.genres = vec!["Old".into()];
            m.field_sources.insert("genres".into(), "steam".into());
        });
        let igdb = md(|m| {
            m.description = Some("igdb".into());
            m.genres = vec!["New".into()];
            m.age_ratings = vec![AgeRating::new(RatingSystem::Esrb, "T", Vec::new()).unwrap()];
        });
        let pr = priority();
        let cands = [MergeCandidate { source: "igdb", metadata: &igdb }];
        let fields = ["description".to_string()];
        let out = merge_metadata(MergeArgs {
            app_name: "a",
            runner: "sideload",
            current: Some(&current),
            candidates: &cands,
            priority: &pr,
            overwrite_existing: true,
            preserve_manual: false,
            field_picks: None,
            fields: Some(&fields),
        });
        assert_eq!(out.description.as_deref(), Some("igdb"));
        assert_eq!((out.genres.as_slice(), out.field_sources["genres"].as_str()), (["Old".to_string()].as_slice(), "steam"));
        assert!(out.age_ratings.is_empty() && !out.field_sources.contains_key(AGE_RATINGS));
        assert!(!out.extra.contains_key(AGE_CHECKED), "ratings not asked: the age ratings fill still may");
    }

    #[test]
    fn each_cover_family_is_a_field_of_its_own() {
        let pr: IndexMap<String, Vec<String>> = ALL_FIELDS.iter().map(|f| (f.to_string(), default_field_priority(f))).collect();
        let current = md(|m| {
            m.cover_url = Some("tall".into());
            m.field_sources.insert("cover".into(), "igdb".into());
        });
        let sgdb = md(|m| {
            m.cover_url = Some("sgdb tall".into());
            m.cover_square_url = Some("sq".into());
            m.cover_wide_url = Some("sgdb wide".into());
        });
        let steam = md(|m| m.cover_wide_url = Some("header".into()));
        let cands = [MergeCandidate { source: "steamgriddb", metadata: &sgdb }, MergeCandidate { source: "steam", metadata: &steam }];
        // Without naming them, the families are left alone.
        let out = merge(Some(&current), &cands, &pr, true, false, None);
        assert!(out.cover_square_url.is_none() && out.cover_wide_url.is_none());
        let fields = ["coverSquare".to_string(), "coverWide".to_string()];
        let out = merge_metadata(MergeArgs {
            app_name: "a",
            runner: "sideload",
            current: Some(&current),
            candidates: &cands,
            priority: &pr,
            overwrite_existing: false,
            preserve_manual: true,
            field_picks: None,
            fields: Some(&fields),
        });
        assert_eq!((out.cover_square_url.as_deref(), out.field_sources["coverSquare"].as_str()), (Some("sq"), "steamgriddb"));
        assert_eq!((out.cover_wide_url.as_deref(), out.field_sources["coverWide"].as_str()), (Some("header"), "steam"));
        assert_eq!((out.cover_url.as_deref(), out.field_sources["cover"].as_str()), (Some("tall"), "igdb"), "the tall cover stays");
    }

    #[test]
    fn copy_field_takes_the_value_and_its_source() {
        let from = md(|m| {
            m.series = Some("S".into());
            m.field_sources.insert("series".into(), "igdb".into());
        });
        let mut to = md(|m| {
            m.series = Some("Other".into());
            m.field_sources.insert("series".into(), "manual".into());
            m.genres = vec!["G".into()];
            m.field_sources.insert("genres".into(), "steam".into());
        });
        copy_field(Some(&from), &mut to, "series");
        assert_eq!((to.series.as_deref(), to.field_sources["series"].as_str()), (Some("S"), "igdb"));
        copy_field(Some(&from), &mut to, "genres");
        assert!(to.genres.is_empty() && !to.field_sources.contains_key("genres"));
        copy_field(None, &mut to, "series");
        assert!(to.series.is_none() && !to.field_sources.contains_key("series"));
    }
}
