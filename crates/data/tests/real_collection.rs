//! End-to-end checks against a copy of the real data.
//! `HERONITE_FIXTURE=/tmp/heronite-fixture cargo test -p heronite-data`

use heronite_data::collection::*;
use heronite_data::{art, DataPaths, Library};

fn fixture() -> Option<DataPaths> {
    std::env::var_os("HERONITE_FIXTURE").map(DataPaths::at)
}

#[test]
fn groups_and_filters_look_sane() {
    let Some(paths) = fixture() else { return };
    let lib = Library::load(&paths).unwrap();
    println!("games: {}", lib.games.len());

    let all = filter_games(&lib.games, &ViewQuery::default());
    assert_eq!(all.len(), lib.games.len());

    let groups = group_by_status(&all, &lib.statuses);
    println!("groups:");
    for g in &groups {
        println!("  {:<16} {:>4}  {:?}", g.name, g.games.len(), g.color);
    }
    assert_eq!(groups.iter().map(|g| g.games.len()).sum::<usize>(), all.len());
    // Real library: Epic games without an overlay entry land in "Not Played",
    // nothing should end up in "Other".
    assert!(groups.iter().all(|g| !g.is_other()));
    // Group order follows the status sortOrder.
    let orders: Vec<_> = groups
        .iter()
        .map(|g| lib.statuses.iter().position(|s| s.id == g.id).unwrap())
        .collect();
    assert!(orders.windows(2).all(|w| w[0] < w[1]));

    let installed = filter_games(&lib.games, &ViewQuery { install: InstallFilter::Installed, ..Default::default() });
    let uninstalled = filter_games(&lib.games, &ViewQuery { install: InstallFilter::Uninstalled, ..Default::default() });
    println!("installed={} uninstalled={}", installed.len(), uninstalled.len());
    assert_eq!(installed.len() + uninstalled.len(), all.len());

    let portal = filter_games(&lib.games, &ViewQuery { search: " PORTAL ".into(), ..Default::default() });
    println!("search portal: {:?}", portal.iter().map(|g| g.title.as_str()).collect::<Vec<_>>());
    assert!(!portal.is_empty());

    let opts = collect_facet_options(&lib.games);
    for kind in FACET_KINDS {
        println!("facet {:<10} {} values", kind.as_str(), opts.get(kind).len());
    }
    let valve = filter_games(
        &lib.games,
        &ViewQuery { facets: FacetFilters::default().toggled(FacetKind::Developer, "valve"), ..Default::default() },
    );
    println!("developer=valve: {}", valve.len());
    assert!(!valve.is_empty());

    let portal2 = lib.get("sideload_steam_620").unwrap();
    for g in related_games_for(portal2, &lib.games) {
        println!("related {:?}: {:?}", g.kind, g.games.iter().map(|x| x.title.as_str()).collect::<Vec<_>>());
    }

    let ctx = art::ArtContext::new(&lib.paths, lib.settings.screenshots_folder());
    println!("cover chain: {:?}", art::card_cover(&ctx, portal2));
    println!("stage chain: {:?}", art::stage_art(&ctx, portal2));
}
