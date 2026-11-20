use serde_json::{json, Value};

use super::*;

const SHA: &str = "0000000000000000000000000000000000000000000000000000000000000000";

fn good_entry() -> Value {
    json!({
        "id": "exampledb",
        "name": "Example DB",
        "version": "1.2.0",
        "api": 1,
        "author": "Someone",
        "homepage": "https://example.org",
        "kind": "details",
        "adult": false,
        "description": "Games from Example DB.",
        "fields": ["description", "cover", "notAField"],
        "applies": { "to": "emulated", "asked": "fallback", "game_kind": { "name": "Visual novels", "signals": ["Visual Novel"] } },
        "hosts": ["api.example.org", "*.cdn.example.org"],
        "images": ["images.example.net"],
        "settings": [{ "label": "API key", "required": true }],
        "files": {
            "source.toml": { "url": "https://lists.example.org/exampledb/source.toml", "sha256": SHA, "size": 900 },
            "source.js": { "url": "https://lists.example.org/exampledb/source.js", "sha256": SHA, "size": 4000 }
        },
        "later": "an unknown key is left out"
    })
}

fn list(entries: Vec<Value>) -> Vec<u8> {
    serde_json::to_vec(&json!({ "index": 1, "name": "Tests", "extensions": entries })).unwrap()
}

fn with(edit: impl FnOnce(&mut Value)) -> Result<Index, IndexError> {
    let mut e = good_entry();
    edit(&mut e);
    parse(&list(vec![e]))
}

#[test]
fn a_good_list_reads_whole() {
    let index = parse(&list(vec![good_entry()])).unwrap();
    assert_eq!(index.name.as_deref(), Some("Tests"));
    let e = &index.entries[0];
    assert_eq!((e.id.as_str(), e.name.as_str(), e.version.to_string().as_str()), ("exampledb", "Example DB", "1.2.0"));
    assert_eq!(e.kind, SourceKind::Details);
    assert_eq!(e.fields, ["description", "cover"], "an unknown field is left out");
    assert_eq!(e.applies_to, AppliesTo::Emulated);
    assert_eq!(e.asked, Asked::AsFallback);
    assert_eq!(e.game_kind.as_ref().map(|k| k.name.as_str()), Some("Visual novels"));
    assert_eq!(e.hosts, ["api.example.org", "*.cdn.example.org"]);
    assert_eq!(e.images, ["images.example.net"]);
    assert_eq!(e.settings, [IndexSetting { label: "API key".into(), required: true }]);
    assert_eq!(e.manifest.size, 900);
    assert!(e.supported());
    assert!(index.get("exampledb").is_some());
}

#[test]
fn a_newer_contract_is_listed_but_not_supported() {
    let index = with(|e| e["api"] = json!(API_VERSION + 1)).unwrap();
    assert!(!index.entries[0].supported());
}

#[test]
fn the_format_version_is_checked_first() {
    assert_eq!(parse(br#"{"index": 2, "extensions": "anything"}"#), Err(IndexError::NewerFormat(2)));
    assert!(matches!(parse(br#"{"extensions": []}"#), Err(IndexError::NotJson(_))));
    assert!(matches!(parse(br#"{"index": "1", "extensions": []}"#), Err(IndexError::NotJson(_))));
    assert!(matches!(parse(b"[1, 2]"), Err(IndexError::NotJson(_))));
    assert!(matches!(parse(b"not json"), Err(IndexError::NotJson(_))));
    assert_eq!(parse(br#"{"index": 1, "extensions": []}"#).unwrap().entries.len(), 0, "an empty list is a list");
}

#[test]
fn a_list_says_its_type() {
    let typed = |t: &str| parse(format!(r#"{{"index": 1, "type": {t}, "extensions": []}}"#).as_bytes());
    assert!(typed(r#""metadata""#).is_ok());
    assert_eq!(typed(r#""themes""#), Err(IndexError::WrongType("themes".into())));
    assert_eq!(typed("3"), Err(IndexError::WrongType("3".into())), "a type that is not text is another type");
    assert_eq!(typed(r#""Metadata""#), Err(IndexError::WrongType("Metadata".into())), "spelt as written");
    // A list written by hand without one is a list of metadata extensions.
    assert!(parse(br#"{"index": 1, "extensions": []}"#).is_ok());
    // The type is asked before the entries: another type's entries are not read as metadata's.
    assert_eq!(parse(br#"{"index": 1, "type": "themes", "extensions": [{"id": 1}]}"#), Err(IndexError::WrongType("themes".into())));
}

#[test]
fn each_wrong_entry_refuses_the_list() {
    let cases: Vec<(&str, Box<dyn FnOnce(&mut Value)>)> = vec![
        ("bad id", Box::new(|e| e["id"] = json!("Bad_Id"))),
        ("built-in id", Box::new(|e| e["id"] = json!("igdb"))),
        (
            "no name",
            Box::new(|e| {
                e.as_object_mut().unwrap().remove("name");
            }),
        ),
        ("long name", Box::new(|e| e["name"] = json!("x".repeat(41)))),
        ("bad version", Box::new(|e| e["version"] = json!("one"))),
        ("no api", Box::new(|e| e["api"] = json!(0))),
        ("bad kind", Box::new(|e| e["kind"] = json!("music"))),
        ("no field", Box::new(|e| e["fields"] = json!(["nothing"]))),
        ("http homepage", Box::new(|e| e["homepage"] = json!("http://example.org"))),
        ("bad applies", Box::new(|e| e["applies"] = json!({ "to": "everyone" }))),
        ("bad host", Box::new(|e| e["hosts"] = json!(["127.0.0.1"]))),
        ("host with path", Box::new(|e| e["hosts"] = json!(["example.org/api"]))),
        ("http file", Box::new(|e| e["files"]["source.js"]["url"] = json!("http://lists.example.org/source.js"))),
        ("file with password", Box::new(|e| e["files"]["source.js"]["url"] = json!("https://u:p@lists.example.org/source.js"))),
        (
            "no script",
            Box::new(|e| {
                e["files"].as_object_mut().unwrap().remove("source.js");
            }),
        ),
        ("bad sha", Box::new(|e| e["files"]["source.toml"]["sha256"] = json!("abc"))),
        ("short sha", Box::new(|e| e["files"]["source.toml"]["sha256"] = json!(&SHA[1..]))),
        ("script too big", Box::new(|e| e["files"]["source.js"]["size"] = json!(MAX_SCRIPT + 1))),
        ("manifest too big", Box::new(|e| e["files"]["source.toml"]["size"] = json!(MAX_MANIFEST + 1))),
        ("empty file", Box::new(|e| e["files"]["source.toml"]["size"] = json!(0))),
        ("setting without label", Box::new(|e| e["settings"] = json!([{ "required": true }]))),
    ];
    for (what, edit) in cases {
        match with(edit) {
            Err(IndexError::Invalid(msg)) => assert!(!msg.is_empty(), "{what}"),
            other => panic!("{what}: {other:?}"),
        }
    }
}

#[test]
fn an_entry_listed_twice_or_too_many_refuse_the_list() {
    assert!(matches!(parse(&list(vec![good_entry(), good_entry()])), Err(IndexError::Invalid(m)) if m.contains("twice")));
    let many: Vec<Value> = (0..=MAX_ENTRIES)
        .map(|i| {
            let mut e = good_entry();
            e["id"] = json!(format!("source-{i}"));
            e
        })
        .collect();
    assert!(matches!(parse(&list(many)), Err(IndexError::Invalid(m)) if m.contains("more than")));
    let huge = vec![b' '; MAX_INDEX_BYTES as usize + 1];
    assert!(matches!(parse(&huge), Err(IndexError::Invalid(m)) if m.contains("larger")));
}

#[test]
fn https_addresses_only() {
    assert!(https_url("https://raw.githubusercontent.com/a/b/index.json"));
    assert!(!https_url("http://raw.githubusercontent.com/a/b/index.json"));
    assert!(!https_url("https://user@example.org/index.json"));
    assert!(!https_url("file:///etc/passwd"));
    assert!(!https_url("example.org/index.json"));
}
