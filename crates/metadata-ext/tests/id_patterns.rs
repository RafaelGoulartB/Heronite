//! `[ids] pattern` through the real install and gather: the code found in a
//! game's title or install folder reaches the script as
//! `game.ids.<extension id>`, a hit found by it is sure only when it is
//! that code, two different codes pass none, a code typed in the search is
//! passed too, and a loose pattern is refused at install with the rule it
//! breaks.

mod support;

use std::fs;
use std::path::Path;

use heronite_data::json_store;
use heronite_data::paths::DataPaths;
use heronite_metadata::*;
use heronite_metadata_ext::{ExtensionError, ExtensionsExt};
use serde_json::{json, Value};
use support::{endpoints, folder, manifest, Req, Resp, Server};

const HOST: &str = "api.codedb.example.org";
const PATTERN: &str = r"\b(?:RJ|VJ)[0-9]{6,8}\b";

/// Finds a game by its code when the app passed one (`matchedBy` that id),
/// else by its title; tells the service what it got, so the test sees it.
const SCRIPT: &str = r#"
export async function find(game, ctx) {
  const code = game.ids.codedb || "";
  const r = await ctx.http.get("https://api.codedb.example.org/find", { query: { code, title: game.title } });
  const rows = r.json();
  return rows.map((w) => (code ? { id: w.code, title: w.name, matchedBy: { id: "codedb" } } : { id: w.code, title: w.name }));
}
export async function fetch(id, ctx) {
  const r = await ctx.http.get("https://api.codedb.example.org/work/" + id);
  return r.status === 200 ? r.json() : null;
}
"#;

/// Knows RJ123456 ("Its Japanese Name") and VJ0099887 ("Midnight Garden");
/// a lookup by title finds nothing. A code its catalogue has under another
/// work (RJ555555) answers with a different code, as a careless script
/// might.
fn mock() -> Server {
    Server::start(move |req: &Req| match req.path() {
        "/api.codedb.example.org/find" => match req.query("code").as_deref() {
            Some("RJ123456") => Resp::json(json!([{ "code": "RJ123456", "name": "Its Japanese Name" }])),
            Some("VJ0099887") => Resp::json(json!([{ "code": "VJ0099887", "name": "Midnight Garden" }])),
            Some("RJ555555") => Resp::json(json!([{ "code": "RJ999999", "name": "Something Else" }])),
            _ => Resp::json(json!([])),
        },
        "/api.codedb.example.org/work/RJ123456" => {
            Resp::json(json!({ "title": "Its Japanese Name", "description": "Found by its code.", "genres": ["Adventure"] }))
        }
        "/api.codedb.example.org/work/VJ0099887" => Resp::json(json!({ "title": "Midnight Garden", "description": "Also by code." })),
        "/api.codedb.example.org/work/RJ999999" => Resp::json(json!({ "title": "Something Else", "description": "Not it." })),
        _ => Resp::status(404),
    })
}

fn write(path: &Path, v: Value) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    json_store::write(path, &v).unwrap();
}

fn codedb(pattern: &str) -> String {
    let body = format!("[ids]\npattern = '{pattern}'\n\n[network]\nhosts = [\"{HOST}\"]\nrate = \"10/s\"");
    manifest("codedb", "1.0.0", "details", &["description", "genres"], &body)
}

fn game(title: &str, folder: &str) -> Value {
    json!({"appName": title, "runner": "sideload", "title": title, "playniteId": "", "source": "manual",
           "launchKind": "executable", "windowsInstallDirectory": format!("C:\\Games\\{folder}")})
}

fn args(app: &str) -> MetadataApplyArgs {
    MetadataApplyArgs { app_name: app.into(), runner: "sideload".into(), title: app.into(), ..Default::default() }
}

fn found(preview: &MetadataPreview) -> Option<SourceAnswer> {
    preview.sources.iter().find(|s| s.source == "codedb").map(|s| s.answer.clone())
}

fn matched_by(preview: &MetadataPreview) -> Option<MatchedBy> {
    match found(preview) {
        Some(SourceAnswer::Found { hit, .. }) => hit.matched_by,
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_code_in_the_title_or_folder_is_passed_and_matches_surely() {
    let dir = tempfile::tempdir().unwrap();
    let paths = DataPaths::at(dir.path());
    write(
        &paths.local_meta(),
        json!({"games": {
            "My Game [RJ123456]": game("My Game [RJ123456]", "My Game"),
            "Midnight": game("Midnight", "VJ0099887 Midnight Garden v1.1"),
            "Two Codes RJ123456": game("Two Codes RJ123456", "VJ0099887"),
            "Careless": game("Careless", "RJ555555"),
            "Plain": game("Plain", "Plain"),
        }}),
    );
    write(&paths.collection_settings(), json!({"metadata": {}}));
    let server = mock();
    let svc = MetadataService::with_endpoints(paths, endpoints(&server, &[HOST]));
    let pkg = folder(&dir.path().join("packages"), "codedb", &codedb(PATTERN), SCRIPT);
    svc.install_extension(&pkg, true).unwrap();
    let last_code = || server.log.lock().iter().rev().find(|r| r.path().ends_with("/find")).and_then(|r| r.query("code"));

    // In the title: passed, found by it, sure although the names differ.
    let preview = svc.preview_game_metadata(&args("My Game [RJ123456]")).unwrap();
    assert_eq!(last_code().as_deref(), Some("RJ123456"));
    assert_eq!(matched_by(&preview), Some(MatchedBy::SourceId));
    let saved = svc.apply_game_metadata(&args("My Game [RJ123456]")).unwrap();
    assert_eq!(saved.description.as_deref(), Some("Found by its code."));
    assert_eq!(heronite_metadata::record::source_match(&saved, "codedb").map(|m| m.id), Some("RJ123456".to_string()));

    // In the install folder's name.
    let preview = svc.preview_game_metadata(&args("Midnight")).unwrap();
    assert_eq!(last_code().as_deref(), Some("VJ0099887"));
    assert_eq!(matched_by(&preview), Some(MatchedBy::SourceId));

    // Two different codes: none is passed; the title finds nothing.
    let preview = svc.preview_game_metadata(&args("Two Codes RJ123456")).unwrap();
    assert_eq!(last_code().as_deref(), Some(""));
    assert_eq!(found(&preview), Some(SourceAnswer::NotFound));

    // A hit named as found by the code but that is another code: a guess.
    let preview = svc.preview_game_metadata(&args("Careless")).unwrap();
    assert_eq!(last_code().as_deref(), Some("RJ555555"));
    assert_eq!(matched_by(&preview), Some(MatchedBy::FuzzyTitle));

    // No code: nothing passed.
    svc.preview_game_metadata(&args("Plain")).unwrap();
    assert_eq!(last_code().as_deref(), Some(""));

    // A code typed in the search is passed like one in a title.
    let search = svc.search_sources("RJ123456", Some(&["codedb".to_string()]));
    assert_eq!(search.asked, ["codedb"]);
    assert_eq!(last_code().as_deref(), Some("RJ123456"));
}

#[test]
fn a_loose_pattern_is_refused_at_install_with_the_rule_it_breaks() {
    let dir = tempfile::tempdir().unwrap();
    let paths = DataPaths::at(dir.path());
    write(&paths.collection_settings(), json!({"metadata": {}}));
    let svc = MetadataService::new(paths);
    for (i, (pattern, rule)) in
        [(r"RJ[0-9]{6,8}", "word boundary"), (r"\b[0-9]{6,8}\b", "literal prefix"), (r"\bRJ[0-9]+\b", "upper bound")]
            .into_iter()
            .enumerate()
    {
        let pkg = folder(&dir.path().join("packages"), &format!("loose{i}"), &codedb(pattern), SCRIPT);
        match svc.install_extension(&pkg, true) {
            Err(ExtensionError::Manifest(why)) => assert!(why.contains(rule), "{pattern}: {why}"),
            other => panic!("{pattern}: {other:?}"),
        }
    }
    assert!(svc.sources().get("codedb").is_none());
}
