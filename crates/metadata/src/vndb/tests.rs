use super::*;

/// The results of a recorded `POST /vn` answer.
fn results(name: &str) -> Vec<Vn> {
    let path = format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"));
    let body: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    serde_json::from_value(body["results"].clone()).unwrap()
}

fn steins_gate() -> Vn {
    results("vndb_id_v2002.json").remove(0)
}

#[test]
fn maps_every_field_of_a_recorded_entry() {
    let vn = steins_gate();
    let (md, adult) = map_vn("a", "sideload", &vn, VndbOptions::default());
    assert_eq!(md.title.as_deref(), Some("STEINS;GATE"));
    assert_eq!(md.release_date.as_deref(), Some("2009-10-15"));
    assert_eq!(md.developers, ["NITRO PLUS", "MAGES.", "MAGES. Inc."]);
    assert_eq!(md.genres, ["Visual Novel"]);
    assert_eq!(md.critic_score, Some(JsNum(90.0)), "VNDB's 10..=100 rating, rounded");
    assert_eq!(md.cover_url.as_deref(), Some("https://t.vndb.org/cv/19/77819.jpg"));
    let hero = md.hero_url.as_deref().unwrap();
    assert!(hero.starts_with("https://t.vndb.org/sf/"), "{hero}");
    assert!(adult.is_empty());
    // Plain text: no codes, the source note kept, clipped to one line.
    let text = md.description.unwrap();
    assert!(text.starts_with("-Decide The Fate Of All Mankind- CAN YOU CHANGE"), "{text}");
    assert!(text.ends_with("[From Steam]") && !text.contains("[b]") && !text.contains("[url"), "{text}");
    // Themes: content tags only, no spoilers, highest first, at most 12.
    assert_eq!(md.themes.len(), 12);
    assert_eq!(md.themes[0], "Chuunibyou Protagonist");
    assert!(!md.themes.iter().any(|t| t == "Time Travel"), "a minor spoiler");
    assert!(!md.themes.iter().any(|t| t == "One True End" || t == "ADV"), "technical tags");
    // Nothing it does not have.
    assert!(md.publishers.is_empty() && md.platforms.is_empty() && md.series.is_none() && md.age_ratings.is_empty());
}

#[test]
fn the_title_option_changes_the_match_name_only() {
    let saya = results("vndb_lookup_song_of_saya.json").remove(0);
    assert_eq!(saya.shown_titles(false), (Some("Saya no Uta".into()), Some("沙耶の唄".into())));
    assert_eq!(saya.shown_titles(true), (Some("沙耶の唄".into()), Some("Saya no Uta".into())));
    let original = VndbOptions { original_title: true, ..Default::default() };
    assert_eq!(map_vn("a", "sideload", &saya, original).0.title.as_deref(), Some("沙耶の唄"));
    // An entry without another script shows its title either way.
    assert_eq!(steins_gate().shown_titles(true), (Some("STEINS;GATE".into()), None));
}

#[test]
fn partial_and_unknown_release_dates() {
    assert_eq!(release_date(Some("2012")).as_deref(), Some("2012"));
    assert_eq!(release_date(Some("2012-03")).as_deref(), Some("2012-03"));
    assert_eq!(release_date(Some("2012-03-04")).as_deref(), Some("2012-03-04"));
    assert_eq!(release_date(Some("TBA")), None);
    assert_eq!(release_date(Some("unknown")), None);
    assert_eq!(release_date(None), None);
    let fate = results("vndb_search_fate.json");
    let full = fate.iter().find(|v| v.id == "v55735").unwrap();
    assert_eq!(full.released.as_deref(), Some("2012"));
    assert_eq!(hit(full, VndbOptions::default()).unwrap().year, Some(2012));
    let undated = fate.iter().find(|v| v.id == "v12779").unwrap();
    assert_eq!(hit(undated, VndbOptions::default()).unwrap().year, None);
}

#[test]
fn formatting_codes_become_plain_text() {
    let text =
        "[b]Bold[/b] and [i]it[/i]. [spoiler]She dies.[/spoiler]Read [url=https://x.y/]more[/url].\n\n\n\n[From [url=/v17]VNDB[/url]]";
    assert_eq!(plain_text(text), "Bold and it. Read more.\n\n[From VNDB]");
    assert_eq!(plain_text("[SPOILER]a\nb[/SPOILER]x"), "x");
}

#[test]
fn spoilers_lies_and_weak_tags_are_left_out() {
    let tag = |name: &str, rating: f64, spoiler: u8, lie: bool, cat: &str| VnTag {
        name: Some(name.into()),
        rating,
        spoiler,
        lie,
        category: Some(cat.into()),
    };
    let tags = [
        tag("Mystery", 2.5, 0, false, "cont"),
        tag("Twist", 2.9, 2, false, "cont"),
        tag("Fake", 2.9, 0, true, "cont"),
        tag("Sexual", 2.9, 0, false, "ero"),
        tag("ADV", 2.9, 0, false, "tech"),
        tag("Weak", 1.5, 0, false, "cont"),
        tag("Drama", 2.8, 0, false, "cont"),
    ];
    assert_eq!(themes(&tags), ["Drama", "Mystery"]);
}

#[test]
fn an_explicit_cover_is_flagged_and_safe_screenshots_win() {
    let seminar = results("vndb_id_v9.json").remove(0);
    let image = seminar.image.as_ref().unwrap();
    assert!(image.sexual > 1.3 && !image.is_safe());
    let (md, adult) = map_vn("a", "sideload", &seminar, VndbOptions::default());
    assert!(md.cover_url.is_some(), "kept in the candidate, flagged");
    assert!(adult.contains(&"cover".to_string()));
    let h = hit(&seminar, VndbOptions::default()).unwrap();
    assert!(h.thumb_adult && h.thumb_url.is_some());

    let shot = |url: &str, w: u32, h: u32, sexual: f64, votes: u32| VnImage {
        url: Some(url.into()),
        dims: Some([w, h]),
        sexual,
        votecount: votes,
        ..Default::default()
    };
    let shots = [
        shot("https://a/tall.jpg", 600, 900, 0.0, 9),
        shot("https://a/hot.jpg", 1280, 720, 1.8, 30),
        shot("https://a/ok.jpg", 1280, 720, 0.2, 12),
    ];
    assert_eq!(background(&shots), Some(("https://a/ok.jpg".into(), false)));
    // Only explicit landscapes: the first, flagged.
    assert_eq!(background(&shots[..2]), Some(("https://a/hot.jpg".into(), true)));
    assert_eq!(background(&shots[..1]), None, "a portrait is no background");
    // An image nobody voted on is not called safe (VNDB hides it too).
    assert!(!VnImage { votecount: 0, ..Default::default() }.is_safe());
    // No screenshot background when the option is off.
    let off = VndbOptions { screenshot_background: false, ..Default::default() };
    assert!(map_vn("a", "sideload", &steins_gate(), off).0.hero_url.is_none());
}

#[test]
fn titles_match_in_any_script_and_alias() {
    let steins = results("vndb_lookup_steins_gate.json");
    assert_eq!(best_match("Steins;Gate", &steins).map(|(v, by)| (v.id.as_str(), by)), Some(("v2002", MatchedBy::ExactTitle)));
    let ja = results("vndb_lookup_ja_steins_gate.json");
    assert_eq!(best_match("シュタインズ・ゲート", &ja).map(|(v, by)| (v.id.as_str(), by)), Some(("v2002", MatchedBy::ExactTitle)));
    let clannad = results("vndb_lookup_clannad.json");
    assert_eq!(best_match("Clannad", &clannad).map(|(v, by)| (v.id.as_str(), by)), Some(("v4", MatchedBy::ExactTitle)));
    let fate = results("vndb_lookup_fate_stay_night.json");
    assert_eq!(best_match("Fate/stay night", &fate).map(|(v, by)| (v.id.as_str(), by)), Some(("v11", MatchedBy::ExactTitle)));
    // Only close to the English title: a guess.
    let saya = results("vndb_lookup_song_of_saya.json");
    assert_eq!(best_match("The Song of Saya", &saya).map(|(v, by)| (v.id.as_str(), by)), Some(("v97", MatchedBy::FuzzyTitle)));
    assert!(best_match("Portal 2", &steins).is_none());
}

#[test]
fn ids_and_addresses_name_an_entry() {
    assert_eq!(parse_id("v17").as_deref(), Some("v17"));
    assert_eq!(parse_id(" V2002 ").as_deref(), Some("v2002"));
    assert_eq!(parse_id("https://vndb.org/v2002").as_deref(), Some("v2002"));
    assert_eq!(parse_id("vndb.org/v2002/chars#x").as_deref(), Some("v2002"));
    assert_eq!(parse_id("1942"), None, "a bare number is a title");
    assert_eq!(parse_id("v0"), None);
    assert_eq!(parse_id("https://vndb.org/r12"), None);
    assert_eq!(parse_id("Steins;Gate"), None);
}

#[test]
fn search_hits_carry_the_other_title_and_platforms() {
    let fate = results("vndb_search_fate.json");
    let hits: Vec<SourceHit> = fate.iter().filter_map(|v| hit(v, VndbOptions::default())).collect();
    assert_eq!(hits.len(), 20);
    let extra = hits.iter().find(|h| h.id.as_deref() == Some("v7602")).unwrap();
    assert_eq!((extra.title.as_deref(), extra.alt_title.as_deref()), (Some("Fate/EXTRA"), Some("フェイト/エクストラ")));
    assert_eq!(extra.year, Some(2010));
    assert!(extra.platform.is_some() && extra.thumb_url.as_deref().is_some_and(|u| u.starts_with("https://")));
    // Explicit thumbnails are flagged for the registry to drop.
    assert!(hits.iter().any(|h| h.thumb_adult) && hits.iter().any(|h| !h.thumb_adult));
    assert_eq!(platform_name("win"), "Windows");
    assert_eq!(platform_name("x68"), "X68");
}
