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

fn portal2(language: Option<&str>) -> SourceGame {
    SourceGame {
        app_name: "a".into(),
        runner: "sideload".into(),
        title: "Portal 2".into(),
        language: language.map(Into::into),
        ..Default::default()
    }
}

#[test]
fn live_wikipedia() {
    if !enabled() {
        return;
    }
    let c = lookup("wikipedia", portal2(Some("pt-BR")));
    assert_eq!((c.match_id.as_deref(), c.matched_by), (Some("Portal_2"), Some(MatchedBy::ExactTitle)));
    assert!(c.metadata.description.as_deref().is_some_and(|d| d.contains("Valve")));
    assert_eq!(c.metadata.release_date.as_deref(), Some("2011"));
}

#[test]
fn live_vndb() {
    if !enabled() {
        return;
    }
    let game = SourceGame { app_name: "a".into(), runner: "sideload".into(), title: "Steins;Gate".into(), ..Default::default() };
    let c = lookup("vndb", game);
    assert_eq!((c.match_id.as_deref(), c.matched_by), (Some("v2002"), Some(MatchedBy::ExactTitle)));
    let md = &c.metadata;
    assert_eq!(md.release_date.as_deref(), Some("2009-10-15"));
    assert!(md.description.as_deref().is_some_and(|d| d.contains("Steins;Gate") && !d.contains("[url")));
    assert!(md.cover_url.as_deref().is_some_and(|u| u.starts_with("https://t.vndb.org/")) && md.hero_url.is_some());
    assert!(!md.developers.is_empty() && !md.themes.is_empty() && md.critic_score.is_some());
    assert_eq!(md.genres, ["Visual Novel"]);
}
