//! The example extensions against the real services, to check the scripts
//! and the recorded fixtures against today's answers. Opt in with
//! `HERONITE_LIVE=1`. Writes go to a tempdir.

mod support;

use heronite_data::paths::DataPaths;
use heronite_metadata::*;
use heronite_metadata_ext::ExtensionsExt;
use support::example;

fn enabled() -> bool {
    std::env::var_os("HERONITE_LIVE").is_some()
}

fn lookup(name: &str, game: SourceGame) -> MetadataCandidate {
    let dir = tempfile::tempdir().unwrap();
    let svc = MetadataService::new(DataPaths::at(dir.path()));
    svc.install_extension(&example(name), false).unwrap();
    let sources = svc.sources();
    let source = sources.lookups().find(|s| s.descriptor().id == name).unwrap();
    match source.lookup(&SourceContext::new(&svc.paths, &svc.http), &game) {
        Lookup::Found(c) => c,
        other => panic!("{name}: {other:?}"),
    }
}

fn portal2(steam: Option<&str>, language: Option<&str>) -> SourceGame {
    SourceGame {
        app_name: "a".into(),
        runner: "sideload".into(),
        title: "Portal 2".into(),
        steam_app_id: steam.map(Into::into),
        language: language.map(Into::into),
        ..Default::default()
    }
}

#[test]
fn live_wikipedia() {
    if !enabled() {
        return;
    }
    let c = lookup("wikipedia", portal2(None, Some("pt-BR")));
    assert_eq!((c.match_id.as_deref(), c.matched_by), (Some("Portal_2"), Some(MatchedBy::ExactTitle)));
    assert!(c.metadata.description.as_deref().is_some_and(|d| d.contains("Valve")));
    assert_eq!(c.metadata.release_date.as_deref(), Some("2011"));
}

#[test]
fn live_pcgamingwiki() {
    if !enabled() {
        return;
    }
    let c = lookup("pcgamingwiki", portal2(Some("620"), None));
    assert_eq!((c.match_id.as_deref(), c.matched_by), (Some("Portal 2"), Some(MatchedBy::SteamId)));
    assert_eq!(c.metadata.series.as_deref(), Some("Portal"));
    assert!(c.metadata.features.iter().any(|f| f.contains("ontroller support")), "{:?}", c.metadata.features);
}
