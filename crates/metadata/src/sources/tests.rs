use std::collections::BTreeSet;
use std::sync::Arc;

use heronite_data::models::LocalGameMeta;
use heronite_data::settings::{default_field_priority, METADATA_FIELDS, SOURCE_IDS};
use parking_lot::RwLock;

use super::*;
use crate::igdb::Igdb;
use crate::providers::StoreLibraries;
use crate::steam::SteamState;
use crate::types::{MatchedBy, MetadataApplyArgs};

fn registry() -> SourceRegistry {
    SourceRegistry::builtin(Shared {
        igdb: Arc::new(Igdb::new()),
        steam: Arc::new(SteamState::default()),
        stores: Arc::new(StoreLibraries::default()),
        playnite: Arc::new(RwLock::new(None)),
    })
}

fn ids<'a>(list: impl Iterator<Item = &'a SourceDescriptor>) -> Vec<&'a str> {
    list.map(|d| d.id.as_str()).collect()
}

#[test]
fn lookups_keep_the_provider_order() {
    let reg = registry();
    let lookups: Vec<&str> = reg.lookups().map(|s| s.descriptor().id.as_str()).collect();
    assert_eq!(lookups, ["igdb", "steam", "store", "lutris", "playnite"]);
    assert_eq!(
        ids(reg.descriptors()),
        ["igdb", "steam", "store", "lutris", "playnite", "steamgriddb", "webimages", "howlongtobeat", "steamtags", "protondb"]
    );
    assert_eq!(ids(reg.descriptors()), ids(builtin_descriptors().iter()));
    let unique: BTreeSet<_> = reg.descriptors().map(|d| d.id.as_str()).collect();
    assert_eq!(unique.len(), builtin_descriptors().len());
}

#[test]
fn only_details_sources_have_a_lookup_for_now() {
    let reg = registry();
    for d in reg.descriptors() {
        assert_eq!(reg.has_lookup(&d.id), d.kind == SourceKind::Details, "{}", d.id);
    }
    for id in ["steamgriddb", "webimages"] {
        assert_eq!(reg.get(id).map(|d| d.kind), Some(SourceKind::Pictures));
    }
    for id in ["howlongtobeat", "steamtags", "protondb"] {
        assert_eq!(reg.get(id).map(|d| d.kind), Some(SourceKind::Extras));
    }
    assert!(!reg.has_lookup("manual") && reg.get("manual").is_none());
}

#[test]
fn offered_details_sources_leave_playnite_out() {
    assert_eq!(ids(offered(SourceKind::Details)), ["igdb", "steam", "store", "lutris"]);
    assert!(registry().get("playnite").is_some_and(|d| d.hidden));
    let names: Vec<&str> = offered(SourceKind::Details).map(|d| d.name.as_str()).collect();
    assert_eq!(names, ["IGDB", "Steam", "Store", "Lutris"]);
}

#[test]
fn network_sources_are_igdb_steam_and_lutris() {
    let reg = registry();
    let online: Vec<&str> = reg.lookups().map(|s| s.descriptor()).filter(|d| d.network).map(|d| d.id.as_str()).collect();
    assert_eq!(online, ["igdb", "steam", "lutris"]);
}

/// `heronite_data::settings` keeps the ids of the default priorities and of
/// the saved-priority filter; they must be the registered details sources.
#[test]
fn data_crate_ids_match_the_details_sources() {
    let reg = registry();
    let details: BTreeSet<&str> = reg.descriptors().filter(|d| d.kind == SourceKind::Details).map(|d| d.id.as_str()).collect();
    let saved: BTreeSet<&str> = SOURCE_IDS.iter().copied().filter(|id| *id != "manual").collect();
    assert_eq!(details, saved);
    for field in METADATA_FIELDS {
        for id in default_field_priority(field) {
            assert!(reg.has_lookup(&id), "default priority of {field} names {id}, not a details source");
        }
    }
    for d in reg.descriptors().filter(|d| d.kind == SourceKind::Details) {
        assert!(d.provides.iter().all(|f| METADATA_FIELDS.contains(&f.as_str())), "{}", d.id);
    }
}

fn args(steam: Option<&str>) -> MetadataApplyArgs {
    MetadataApplyArgs {
        app_name: "a".into(),
        runner: "sideload".into(),
        title: "Game".into(),
        steam_app_id: steam.map(Into::into),
        ..Default::default()
    }
}

fn overlay(f: impl FnOnce(&mut LocalGameMeta)) -> LocalGameMeta {
    let mut m: LocalGameMeta = serde_json::from_str(r#"{"appName":"a","runner":"sideload"}"#).unwrap();
    f(&mut m);
    m
}

#[test]
fn source_game_takes_the_steam_id_from_the_overlay_when_missing() {
    let o = overlay(|m| m.steam_app_id = Some("620".into()));
    assert_eq!(SourceGame::new(&args(None), Some(&o)).steam_app_id.as_deref(), Some("620"));
    assert_eq!(SourceGame::new(&args(Some("")), Some(&o)).steam_app_id.as_deref(), Some("620"));
    assert_eq!(SourceGame::new(&args(Some("400")), Some(&o)).steam_app_id.as_deref(), Some("400"));
    assert_eq!(SourceGame::new(&args(Some("")), None).steam_app_id, None);
}

#[test]
fn applies_to_answers_for_a_game() {
    let plain = SourceGame::new(&args(None), None);
    let steam = SourceGame::new(&args(Some("620")), None);
    let rom = overlay(|m| {
        m.source = "emulator".into();
        m.launch_kind = "emulator".into();
        m.platform_id = Some("snes".into());
    });
    let emulated = SourceGame::new(&args(None), Some(&rom));
    let epic = SourceGame { runner: "legendary".into(), ..plain.clone() };

    assert!(AppliesTo::All.matches(&plain));
    assert!(!AppliesTo::HasSteamId.matches(&plain) && AppliesTo::HasSteamId.matches(&steam));
    assert!(AppliesTo::Emulated.matches(&emulated) && !AppliesTo::Emulated.matches(&steam));
    let snes = AppliesTo::Platforms(vec!["snes".into()]);
    assert!(snes.matches(&emulated) && !snes.matches(&plain));
    let stores = AppliesTo::Stores(vec!["legendary".into(), "emulator".into()]);
    assert!(stores.matches(&epic) && stores.matches(&emulated) && !stores.matches(&plain));

    let reg = registry();
    let steam_source = reg.get("steam").unwrap();
    assert!(!steam_source.applies_to.matches(&plain) && steam_source.applies_to.matches(&steam));
    assert!(!reg.needs_overlay());
}

#[test]
fn a_manifest_descriptor_takes_defaults() {
    let d: SourceDescriptor = serde_json::from_str(r#"{"id":"vndb","name":"VNDB","kind":"details"}"#).unwrap();
    assert_eq!(d.applies_to, AppliesTo::All);
    assert_eq!(d.needs, Needs::Nothing);
    assert!(d.provides.is_empty() && d.finds_by.is_empty() && !d.network && !d.hidden);

    let d: SourceDescriptor = serde_json::from_str(
        r#"{"id":"x","name":"X","kind":"extras","provides":["timeToBeat"],"appliesTo":{"platforms":["snes"]},
            "findsBy":["romHash","fuzzyTitle"],"needs":"download","network":true}"#,
    )
    .unwrap();
    assert!(d.provides(extra_fields::TIME_TO_BEAT));
    assert_eq!(d.applies_to, AppliesTo::Platforms(vec!["snes".into()]));
    assert_eq!(d.finds_by, [MatchedBy::RomHash, MatchedBy::FuzzyTitle]);
    assert_eq!(d.needs, Needs::Download);

    for d in builtin_descriptors() {
        let back: SourceDescriptor = serde_json::from_value(serde_json::to_value(d).unwrap()).unwrap();
        assert_eq!(&back, d);
    }
    assert_eq!(serde_json::to_value(AppliesTo::HasSteamId).unwrap(), "hasSteamId");
}
