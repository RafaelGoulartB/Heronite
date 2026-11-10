use serde_json::json;

use super::*;

fn rules() -> Vec<HostRule> {
    ["api.example.org", "*.img.example.org"].iter().map(|r| HostRule::parse(r).unwrap()).collect()
}

fn keys<'a>(title: &'a str, ids: &'a Map<String, Value>, hashes: &'a Map<String, Value>) -> GameKeys<'a> {
    GameKeys { title, year: None, ids, hashes }
}

/// `url` as a picture of an extension is kept: marked as one.
fn mark(url: &str) -> String {
    heronite_images::public::marked(url)
}

fn all_fields() -> Vec<String> {
    heronite_data::settings::ALL_FIELDS.iter().map(|f| f.to_string()).collect()
}

#[test]
fn dates_in_the_usual_spellings() {
    for (text, want) in [
        ("2011-04-18", Some("2011-04-18")),
        ("2011/04/18", Some("2011-04-18")),
        ("2011-04", Some("2011-04")),
        ("2011", Some("2011")),
        ("April 18, 2011", Some("2011-04-18")),
        ("Apr 18 2011", Some("2011-04-18")),
        ("18 April 2011", Some("2011-04-18")),
        ("18 Apr 2011.", Some("2011-04-18")),
        ("April 2011", Some("2011-04")),
        ("2011-02-30", None),
        ("2011-13", None),
        ("1850", None),
        ("TBA", None),
        ("", None),
        ("Q3 2011", None),
    ] {
        assert_eq!(parse_date(text).as_deref(), want, "{text}");
    }
}

#[test]
fn a_title_match_is_sure_only_when_the_titles_agree() {
    let (ids, hashes) = (Map::new(), Map::new());
    let found = json!([
        { "id": "1", "title": "Portal 2: The Lab", "year": 2011 },
        { "id": "2", "title": "portal-2", "year": 2011, "thumb": "https://x.img.example.org/a.jpg" },
        { "id": "3", "title": "Portal 2", "confidence": 0.2 },
        { "id": "", "title": "no id" },
        { "id": "4" },
        { "id": 5, "title": "<b>Portal</b> 2", "year": 1800, "thumb": "https://elsewhere.org/a.jpg", "platform": "PC" },
    ]);
    // The game's year picks "2" among the two of its title.
    let game = GameKeys { year: Some(2011), ..keys("Portal 2", &ids, &hashes) };
    let hits = hits(&found, &game, &rules());
    let summary: Vec<(&str, MatchedBy)> = hits.iter().map(|h| (h.id.as_str(), h.matched_by)).collect();
    assert_eq!(
        summary,
        [("2", MatchedBy::ExactTitle), ("1", MatchedBy::FuzzyTitle), ("3", MatchedBy::FuzzyTitle), ("5", MatchedBy::FuzzyTitle)],
        "the same title first, a doubtful one is a guess, no id or no title is dropped"
    );
    assert_eq!(hits[0].thumb.as_deref(), Some(mark("https://x.img.example.org/a.jpg").as_str()));
    assert_eq!(
        (hits[3].title.as_str(), hits[3].year, hits[3].thumb.as_deref()),
        ("Portal 2", None, None),
        "plain text; no year 1800; no foreign host"
    );
    assert_eq!(hits[3].platform.as_deref(), Some("PC"));
}

#[test]
fn a_hit_may_carry_its_name_in_another_script_and_flag_its_thumbnail() {
    let (ids, hashes) = (Map::new(), Map::new());
    let found = json!([
        { "id": "v2002", "title": "STEINS;GATE", "altTitle": "シュタインズ・ゲート", "thumb": "https://api.example.org/t.jpg", "adult": true },
        { "id": "v97", "title": "Saya no Uta", "altTitle": "SAYA NO UTA", "thumb": "https://api.example.org/s.jpg" },
        { "id": "v3", "title": "Other", "altTitle": 7 },
    ]);
    let hits = hits(&found, &keys("シュタインズ・ゲート", &ids, &hashes), &rules());
    // The game's title is the other name: sure, as if it were the title.
    assert_eq!((hits[0].id.as_str(), hits[0].matched_by), ("v2002", MatchedBy::ExactTitle));
    assert_eq!(hits[0].alt_title.as_deref(), Some("シュタインズ・ゲート"));
    assert!(hits[0].thumb_adult && !hits[1].thumb_adult);
    assert!(hits[0].is_named(Some("Steins;Gate")) && hits[0].is_named(Some("シュタインズ・ゲート")) && !hits[0].is_named(None));
    // The same name twice, or not text: no other name.
    assert_eq!((hits[1].alt_title.as_deref(), hits[2].alt_title.as_deref()), (None, None));
    assert_eq!(hits[1].matched_by, MatchedBy::FuzzyTitle);
}

#[test]
fn several_games_of_the_title_are_a_guess() {
    let (ids, hashes) = (Map::new(), Map::new());
    let found = json!([
        { "id": "doom-1993", "title": "Doom (1993 video game)", "year": 1993 },
        { "id": "doom-2016", "title": "Doom (2016 video game)", "year": 2016 },
    ]);
    // Both read "Doom" once normalised, as the Wikipedia example's titles do.
    let found =
        json!(found.as_array().unwrap().iter().map(|h| json!({ "id": h["id"], "title": "Doom", "year": h["year"] })).collect::<Vec<_>>());
    let judged = |year: Option<i32>, found: &Value| -> Vec<MatchedBy> {
        let game = GameKeys { year, ..keys("Doom", &ids, &hashes) };
        hits(found, &game, &rules()).iter().map(|h| h.matched_by).collect()
    };
    assert_eq!(judged(None, &found), [MatchedBy::FuzzyTitle, MatchedBy::FuzzyTitle], "the game's year unknown");
    assert_eq!(judged(Some(2005), &found), [MatchedBy::FuzzyTitle, MatchedBy::FuzzyTitle], "neither of its year");
    let by_year: Vec<(String, MatchedBy)> = {
        let game = GameKeys { year: Some(2016), ..keys("Doom", &ids, &hashes) };
        hits(&found, &game, &rules()).into_iter().map(|h| (h.id, h.matched_by)).collect()
    };
    assert_eq!(by_year, [("doom-2016".into(), MatchedBy::ExactTitle), ("doom-1993".into(), MatchedBy::FuzzyTitle)]);
    // Two of the same year cannot be told apart either.
    let twins = json!([{ "id": "a", "title": "Doom", "year": 2016 }, { "id": "b", "title": "Doom", "year": 2016 }]);
    assert_eq!(judged(Some(2016), &twins), [MatchedBy::FuzzyTitle, MatchedBy::FuzzyTitle]);
    // One hit of the title is sure as before.
    assert_eq!(judged(None, &json!([{ "id": "a", "title": "Doom" }])), [MatchedBy::ExactTitle]);
}

#[test]
fn an_id_or_hash_match_names_one_the_game_has() {
    let ids: Map<String, Value> = json!({"steam": "620", "igdb": "72"}).as_object().unwrap().clone();
    let hashes: Map<String, Value> = json!({"md5": "abc"}).as_object().unwrap().clone();
    let found = json!([
        { "id": "a", "title": "Other name", "matchedBy": { "id": "steam" } },
        { "id": "b", "title": "Other name", "matchedBy": { "id": "igdb" } },
        { "id": "c", "title": "Other name", "matchedBy": { "hash": "md5" } },
        { "id": "d", "title": "Other name", "matchedBy": { "id": "gog" } },
        { "id": "e", "title": "Other name", "matchedBy": { "hash": "sha1" } },
        { "id": "f", "title": "Other name", "matchedBy": "exactTitle" },
    ]);
    let by: Vec<MatchedBy> = hits(&found, &keys("Portal 2", &ids, &hashes), &rules()).iter().map(|h| h.matched_by).collect();
    assert_eq!(
        by,
        [MatchedBy::SteamId, MatchedBy::SourceId, MatchedBy::RomHash, MatchedBy::FuzzyTitle, MatchedBy::FuzzyTitle, MatchedBy::FuzzyTitle],
        "an id the game lacks, or a claim of a title match, is a title guess"
    );
    assert!(hits(&json!({"not": "a list"}), &keys("x", &ids, &hashes), &rules()).is_empty());
    let many: Vec<Value> = (0..40).map(|i| json!({"id": i.to_string(), "title": "x"})).collect();
    assert_eq!(hits(&Value::Array(many), &keys("x", &ids, &hashes), &rules()).len(), 25);
}

#[test]
fn a_record_is_clamped_like_the_built_in_mappings() {
    let long = "word ".repeat(1000);
    let value = json!({
        "title": "  Portal   2 ",
        "description": format!("<p>Hello <b>there</b></p>{long}"),
        "releaseDate": "April 18, 2011",
        "developers": ["Valve", "valve", " ", "<i>Valve</i> South", 7],
        "genres": (0..30).map(|i| format!("G{i}")).collect::<Vec<_>>(),
        "series": "Portal",
        "score": 95.4,
        "features": ["Controller"],
        "ageRatings": [{ "system": "esrb", "rating": "E10+", "descriptors": ["Fantasy Violence"] }, { "system": "nope", "rating": "X" }],
        "websites": ["https://portal2.example.org", "javascript:alert(1)", "ftp://x.org"],
        "images": [
            { "kind": "cover", "url": "https://api.example.org/c.jpg" },
            { "kind": "cover", "url": "https://api.example.org/second.jpg" },
            { "kind": "cover", "shape": "square", "url": "http://api.example.org/s.jpg" },
            { "kind": "cover", "shape": "wide", "url": "https://x.img.example.org/w.jpg", "adult": true },
            { "kind": "background", "url": "file:///etc/passwd" },
            { "kind": "logo", "url": "https://api.example.org/l.png" },
        ],
        "extras": { "engine": "Source" },
    });
    let r = record(&value, "a", "sideload", &all_fields(), None, &rules(), false).unwrap();
    let md = &r.metadata;
    assert_eq!(md.title.as_deref(), Some("Portal 2"));
    let description = md.description.as_deref().unwrap();
    assert!(description.starts_with("Hello there") && !description.contains('<'));
    assert_eq!(description.chars().count(), 2000);
    assert_eq!(md.release_date.as_deref(), Some("2011-04-18"));
    assert_eq!(md.developers, ["Valve", "Valve South"]);
    assert_eq!(md.genres.len(), 12);
    assert_eq!(md.critic_score.map(|s| s.get()), Some(95.0));
    assert_eq!(md.age_ratings.len(), 1);
    assert_eq!(md.websites, ["https://portal2.example.org"]);
    assert_eq!(md.cover_url.as_deref(), Some(mark("https://api.example.org/c.jpg").as_str()), "the first cover wins");
    assert_eq!(md.cover_square_url, None, "plain http to a host listed for https only");
    assert_eq!(md.cover_wide_url.as_deref(), Some(mark("https://x.img.example.org/w.jpg").as_str()));
    assert!(md.cover_url.as_deref().is_some_and(heronite_images::public::is_extension_url), "downloaded the safe way, later too");
    assert_eq!(md.hero_url, None);
    assert_eq!(r.adult, ["coverWide"]);

    // Only what the manifest provides, and of that what was asked.
    let r = record(&value, "a", "sideload", &["description".into(), "cover".into()], None, &rules(), false).unwrap();
    assert!(r.metadata.developers.is_empty() && r.metadata.series.is_none() && r.metadata.cover_wide_url.is_none());
    assert!(r.metadata.description.is_some() && r.metadata.cover_url.is_some() && r.adult.is_empty());
    let r = record(&value, "a", "sideload", &all_fields(), Some(&["cover".into()]), &rules(), false).unwrap();
    assert!(r.metadata.description.is_none() && r.metadata.cover_url.is_some());
    // An adult source: every picture is adult.
    let r = record(&value, "a", "sideload", &all_fields(), None, &rules(), true).unwrap();
    assert_eq!(r.adult, ["cover", "coverWide"]);
    assert!(record(&Value::Null, "a", "sideload", &all_fields(), None, &rules(), false).is_none());
    assert!(record(&json!({"score": 400}), "a", "sideload", &all_fields(), None, &rules(), false).unwrap().metadata.critic_score.is_none());
}
