use super::*;
use crate::library::tests_support::game_with;

fn setup(games: &[(&str, &[&str], &[&str])]) -> (tempfile::TempDir, DataPaths) {
    let dir = tempfile::tempdir().unwrap();
    let paths = DataPaths::at(dir.path());
    let mut file = MetadataFile::default();
    for (app, developers, genres) in games {
        file.games.insert(
            format!("sideload:{app}"),
            CollectionGameMetadata {
                app_name: app.to_string(),
                runner: "sideload".into(),
                developers: developers.iter().map(|s| s.to_string()).collect(),
                publishers: developers.iter().map(|s| s.to_string()).collect(),
                genres: genres.iter().map(|s| s.to_string()).collect(),
                ..Default::default()
            },
        );
    }
    std::fs::create_dir_all(paths.metadata().parent().unwrap()).unwrap();
    json_store::write(&paths.metadata(), &file).unwrap();
    (dir, paths)
}

fn metadata(paths: &DataPaths, app: &str) -> CollectionGameMetadata {
    json_store::read::<MetadataFile>(&paths.metadata()).unwrap().games[&format!("sideload:{app}")].clone()
}

fn strings(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| s.to_string()).collect()
}

#[test]
fn merge_rewrites_every_game_and_remembers_it() {
    let (_d, paths) = setup(&[("a", &["Valve Corporation"], &["RPG"]), ("b", &["Valve", "Hidden Path"], &[]), ("c", &["Bungie"], &[])]);
    let changed = rename(&paths, Field::Company, &strings(&["Valve Corporation", "Valve"]), "Valve").unwrap();
    assert_eq!(changed, 1, "only the game with the other spelling changes");
    let a = metadata(&paths, "a");
    assert_eq!(a.developers, ["Valve"]);
    assert_eq!(a.publishers, ["Valve"], "a company is both");
    assert_eq!(a.genres, ["RPG"], "other fields stay");
    assert_eq!(metadata(&paths, "b").developers, ["Valve", "Hidden Path"]);

    let rules = NamesFile::load(&paths).unwrap().rules();
    assert_eq!(rules, [Rule { field: Field::Company, from: "Valve Corporation".into(), to: Some("Valve".into()) }]);

    let mut fresh = CollectionGameMetadata { developers: strings(&["VALVE CORPORATION", "Valve"]), ..Default::default() };
    assert!(NameRules::load(&paths).apply_metadata(&mut fresh));
    assert_eq!(fresh.developers, ["Valve"], "later downloads follow the rule, repeats dropped");
}

#[test]
fn rules_chain_and_removals() {
    let (_d, paths) = setup(&[("a", &[], &["Role-playing (RPG)"]), ("b", &[], &["RPG", "Indie"])]);
    rename(&paths, Field::Genre, &strings(&["Role-playing (RPG)"]), "RPG").unwrap();
    rename(&paths, Field::Genre, &strings(&["RPG"]), "Role-playing").unwrap();
    assert_eq!(metadata(&paths, "a").genres, ["Role-playing"]);
    assert_eq!(metadata(&paths, "b").genres, ["Role-playing", "Indie"]);
    let file = NamesFile::load(&paths).unwrap();
    assert_eq!(file.renames["genres"]["Role-playing (RPG)"], "Role-playing", "the older rule follows");
    assert_eq!(file.renames["genres"]["RPG"], "Role-playing");

    remove(&paths, Field::Genre, &strings(&["Role-playing"])).unwrap();
    assert!(metadata(&paths, "a").genres.is_empty());
    assert_eq!(metadata(&paths, "b").genres, ["Indie"]);
    let rules = NameRules::load(&paths);
    let mut list = strings(&["Role-playing (RPG)", "Indie"]);
    let mut md = CollectionGameMetadata { genres: list.clone(), ..Default::default() };
    rules.apply_metadata(&mut md);
    list.remove(0);
    assert_eq!(md.genres, list);

    forget(&paths, Field::Genre, "RPG").unwrap();
    assert!(!NamesFile::load(&paths).unwrap().renames["genres"].contains_key("RPG"));
}

#[test]
fn renaming_back_drops_the_old_rule() {
    let (_d, paths) = setup(&[("a", &["Valve"], &[])]);
    rename(&paths, Field::Company, &strings(&["Valve"]), "Valve Software").unwrap();
    rename(&paths, Field::Company, &strings(&["Valve Software"]), "Valve").unwrap();
    assert_eq!(metadata(&paths, "a").developers, ["Valve"]);
    let rules = NamesFile::load(&paths).unwrap().rules();
    assert_eq!(rules, [Rule { field: Field::Company, from: "Valve Software".into(), to: Some("Valve".into()) }]);
}

#[test]
fn series_and_facets_follow_the_rules() {
    let file = NamesFile {
        renames: [
            ("series".to_string(), [("Metal Gear Solid".to_string(), "Metal Gear".to_string())].into_iter().collect()),
            ("companies".to_string(), [("Konami Digital Entertainment".to_string(), "Konami".to_string())].into_iter().collect()),
            ("features".to_string(), [("#category_contrast_controls".to_string(), String::new())].into_iter().collect()),
        ]
        .into_iter()
        .collect(),
        ..Default::default()
    };
    let rules = NameRules::from_file(&file);
    let mut md = CollectionGameMetadata { series: Some("Metal Gear Solid".into()), ..Default::default() };
    assert!(rules.apply_metadata(&mut md));
    assert_eq!(md.series.as_deref(), Some("Metal Gear"));

    let game = game_with("mgs", "legendary", |info, _, md, _, _| {
        info.developer = Some("Konami Digital Entertainment".into());
        md.features = strings(&["#category_contrast_controls", "Single-player"]);
    });
    let mut facets = game.facets.clone();
    rules.apply_facets(&mut facets);
    assert_eq!(facets[FacetKind::Developer.index()][0].value, "Konami", "the store's developer too");
    assert_eq!(facets[FacetKind::Developer.index()][0].key, "konami");
    let features: Vec<&str> = facets[FacetKind::Feature.index()].iter().map(|i| i.value.as_str()).collect();
    assert_eq!(features, ["Single-player"]);
}

#[test]
fn values_count_games_once_per_company() {
    let games = vec![
        game_with("a", "sideload", |_, _, md, _, _| {
            md.developers = strings(&["Valve"]);
            md.publishers = strings(&["Valve"]);
        }),
        game_with("b", "sideload", |_, _, md, _, _| md.developers = strings(&["VALVE"])),
        game_with("c", "sideload", |_, _, md, _, _| md.publishers = strings(&["Valve"])),
        game_with("d", "sideload", |_, _, md, _, _| md.publishers = strings(&["Bungie"])),
    ];
    let v = values(&games, Field::Company);
    assert_eq!(
        v,
        [
            FieldValue { name: "Bungie".into(), key: "bungie".into(), games: 1 },
            FieldValue { name: "Valve".into(), key: "valve".into(), games: 3 },
        ]
    );
}
