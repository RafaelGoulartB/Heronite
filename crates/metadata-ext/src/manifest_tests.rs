use super::*;

const GOOD: &str = r#"
id = "example-db"
name = "Example DB"
version = "1.2.0"
api = 1
author = "Someone"
homepage = "https://example.org"
kind = "details"
adult = false
description = "Games from Example DB."

[provides]
fields = ["description", "genres", "cover", "coverWide"]

[applies]
platforms = ["snes", "n64"]
asked = "fallback"
game_kind = { name = "Retro games", signals = ["Retro"] }

[network]
hosts = ["api.example.org", "*.img.example.org", "http://legacy.example.org"]
images = ["cdn.example.net"]
rate = "30/min"

[[settings]]
key = "apiKey"
label = "API key"
type = "secret"
required = true
link = "https://example.org/key"
help = "Get one on your profile."

[[settings]]
key = "region"
label = "Region"
type = "select"
options = ["eu", { value = "us", label = "America" }]
default = "us"

[[settings]]
key = "nsfw"
label = "Adult pictures"
type = "switch"
default = true
"#;

/// `GOOD` with `from` replaced by `to` (which must occur once).
fn with(from: &str, to: &str) -> String {
    assert_eq!(GOOD.matches(from).count(), 1, "{from}");
    GOOD.replace(from, to)
}

fn error(text: &str) -> String {
    match Manifest::parse(text) {
        Ok(_) => panic!("parsed:\n{text}"),
        Err(ExtensionError::Manifest(m)) => m,
        Err(e) => panic!("{e:?}"),
    }
}

#[test]
fn a_full_manifest_becomes_a_descriptor() {
    let m = Manifest::parse(GOOD).unwrap();
    assert!(m.warnings.is_empty(), "{:?}", m.warnings);
    assert_eq!(m.version, semver::Version::new(1, 2, 0));
    assert_eq!(m.network.rate, Rate { count: 30, per: Duration::from_secs(60) });
    assert_eq!(m.network.rate.interval(), Duration::from_secs(2));
    assert_eq!(
        m.network.hosts.iter().map(HostRule::text).collect::<Vec<_>>(),
        ["api.example.org", "*.img.example.org", "http://legacy.example.org"]
    );
    let d = m.descriptor();
    assert_eq!((d.id.as_str(), d.name.as_str(), d.kind), ("example-db", "Example DB", SourceKind::Details));
    assert_eq!(d.provides, ["description", "genres", "cover", "coverWide"]);
    assert_eq!(d.applies_to, AppliesTo::Platforms(vec!["nintendo_super_nes".into(), "nintendo_64".into()]), "the app's platform ids");
    assert!(d.is_fallback() && d.searchable && d.network && !d.hidden);
    assert_eq!(d.game_kind.as_ref().unwrap().signals, ["Retro"]);
    assert_eq!(d.needs, Needs::ApiKey, "a required setting is a key");
    assert_eq!(d.about.as_deref(), Some("Games from Example DB."));
    let ext = d.extension.as_ref().unwrap();
    assert_eq!((ext.version.as_str(), ext.author.as_str(), ext.adult), ("1.2.0", "Someone", false));
    assert_eq!(d.options.len(), 3);
    let region = &d.options[1];
    assert_eq!(region.kind, OptionKind::Select);
    assert_eq!(region.choices.iter().map(|c| (c.value.as_str(), c.label.as_str())).collect::<Vec<_>>(), [("eu", "eu"), ("us", "America")]);
    assert_eq!(region.default, Some(OptionValue::Text("us".into())));
    assert_eq!(d.options[2].default, Some(OptionValue::Switch(true)));
    assert!(d.options[0].required && d.options[0].kind == OptionKind::Secret);
    // The descriptor reads back through its JSON form, as the registry keeps it.
    let back: SourceDescriptor = serde_json::from_value(serde_json::to_value(&d).unwrap()).unwrap();
    assert_eq!(back, d);
}

#[test]
fn the_smallest_manifest_takes_defaults() {
    let m = Manifest::parse(
        r#"id = "ab"
name = "AB"
version = "0.1.0"
api = 1
author = "x"
kind = "pictures"
description = "d"
[provides]
fields = ["cover"]"#,
    )
    .unwrap();
    let d = m.descriptor();
    assert_eq!(d.applies_to, AppliesTo::All);
    assert!(!d.is_fallback() && !d.network && d.options.is_empty());
    assert_eq!(d.needs, Needs::Nothing);
    assert_eq!(m.network.rate, Rate::DEFAULT);
}

#[test]
fn the_contract_version_is_read_first() {
    assert_eq!(Manifest::parse(&with("api = 1", "api = 2")).unwrap_err(), ExtensionError::NewerApi(2));
    // Even when the rest is something this build cannot read.
    assert_eq!(Manifest::parse("api = 3\nkind = 7\n[network]\nwhatever = 1").unwrap_err(), ExtensionError::NewerApi(3));
    assert!(error(&with("api = 1", "api = 0")).contains("not a contract version"));
    assert!(error(&with("api = 1", "api = \"1\"")).contains("whole number"));
    assert!(error(&with("api = 1\n", "")).contains("api is missing"));
    assert!(ExtensionError::NewerApi(2).to_string().contains("newer Heronite"));
}

#[test]
fn ids_are_checked_and_never_a_built_in_one() {
    for bad in ["A", "a", "has space", "under_score", "-dash", "dash-", "two--dashes", "é", &"x".repeat(41)] {
        assert!(error(&with("id = \"example-db\"", &format!("id = \"{bad}\""))).contains("lowercase"), "{bad}");
    }
    for taken in ["igdb", "steam", "launchbox", "steamgriddb", "howlongtobeat", "manual", "keep", "auto", "store", "playnite"] {
        assert_eq!(
            Manifest::parse(&with("id = \"example-db\"", &format!("id = \"{taken}\""))).unwrap_err(),
            ExtensionError::IdTaken(taken.into())
        );
    }
    assert!(valid_id("pcgamingwiki") && valid_id("a1") && valid_id("my-source-2"));
}

#[test]
fn every_required_key_and_type_is_checked() {
    let cases: &[(&str, &str, &str)] = &[
        ("id = \"example-db\"\n", "", "id is missing"),
        ("name = \"Example DB\"", "name = \"\"", "name is empty"),
        ("name = \"Example DB\"", "name = 3", "name must be text"),
        ("name = \"Example DB\"", "name = \"An extension name that is far too long for a row\"", "longer than 40"),
        ("name = \"Example DB\"", "name = \"Tab\\there\"", "control characters"),
        ("version = \"1.2.0\"", "version = \"1.2\"", "semantic version"),
        ("version = \"1.2.0\"\n", "", "version is missing"),
        ("author = \"Someone\"\n", "", "author is missing"),
        ("homepage = \"https://example.org\"", "homepage = \"ftp://example.org\"", "http(s) address"),
        ("kind = \"details\"", "kind = \"covers\"", "details, pictures or extras"),
        ("kind = \"details\"\n", "", "kind is missing"),
        ("adult = false", "adult = \"no\"", "true or false"),
        ("description = \"Games from Example DB.\"\n", "", "description is missing"),
        ("[provides]\nfields = [\"description\", \"genres\", \"cover\", \"coverWide\"]", "", "[provides] is missing"),
        ("fields = [\"description\", \"genres\", \"cover\", \"coverWide\"]", "fields = \"description\"", "list of text"),
        ("fields = [\"description\", \"genres\", \"cover\", \"coverWide\"]", "fields = [\"engine\"]", "no field this app knows"),
        ("platforms = [\"snes\", \"n64\"]", "platforms = [\"snes\"]\nto = \"all\"", "one of to, platforms and stores"),
        ("platforms = [\"snes\", \"n64\"]", "to = \"consoles\"", "all, emulated, pc or steam"),
        ("platforms = [\"snes\", \"n64\"]", "platforms = [\"holodeck\"]", "no platform this app knows"),
        ("platforms = [\"snes\", \"n64\"]", "platforms = []", "applies.platforms is empty"),
        ("asked = \"fallback\"", "asked = \"sometimes\"", "always or fallback"),
        ("rate = \"30/min\"", "rate = \"fast\"", "not a rate"),
        ("rate = \"30/min\"", "rate = \"0/s\"", "not a rate"),
        ("rate = \"30/min\"", "rate = \"100/s\"", "faster than"),
        ("hosts = [\"api.example.org\", \"*.img.example.org\", \"http://legacy.example.org\"]", "hosts = [\"10.0.0.1\"]", "IP address"),
        (
            "hosts = [\"api.example.org\", \"*.img.example.org\", \"http://legacy.example.org\"]",
            "hosts = [\"api.example.org/v1\"]",
            "not a host name",
        ),
        ("images = [\"cdn.example.net\"]", "images = [\"ftp://cdn.example.net\"]", "only http and https"),
        ("key = \"apiKey\"", "key = \"api key\"", "letters, digits and _"),
        ("key = \"region\"", "key = \"apiKey\"", "used twice"),
        ("type = \"secret\"", "type = \"password\"", "text, secret, select or switch"),
        ("options = [\"eu\", { value = \"us\", label = \"America\" }]", "options = []", "select without options"),
        ("default = \"us\"", "default = \"asia\"", "does not fit its type"),
        ("default = true", "default = \"yes\"", "does not fit its type"),
        ("options = [\"eu\", { value = \"us\", label = \"America\" }]", "options = [\"eu\", \"eu\"]", "listed twice"),
        ("link = \"https://example.org/key\"", "link = \"javascript:alert(1)\"", "http(s) address"),
    ];
    for (from, to, expected) in cases {
        let message = error(&with(from, to));
        assert!(message.contains(expected), "{from} -> {to}: {message}");
    }
    assert!(error("not toml at all = = =").contains("not valid TOML"));
    let many_hosts: Vec<String> = (0..17).map(|i| format!("\"h{i}.example.org\"")).collect();
    let hosts_line = "hosts = [\"api.example.org\", \"*.img.example.org\", \"http://legacy.example.org\"]";
    assert!(error(&with(hosts_line, &format!("hosts = [{}]", many_hosts.join(",")))).contains("more than 16 hosts"));
}

#[test]
fn the_audience_uses_the_apps_platforms_and_pc_games() {
    let applies = |line: &str| Manifest::parse(&with("platforms = [\"snes\", \"n64\"]", line)).unwrap();
    assert_eq!(applies("to = \"pc\"").applies_to, AppliesTo::Pc);
    assert_eq!(applies("to = \"emulated\"").applies_to, AppliesTo::Emulated);
    // Ids, names and aliases name one platform; one the app does not know is left out.
    let m = applies("platforms = [\"Super Nintendo Entertainment System\", \"sfc\", \"sony_playstation\", \"holodeck\"]");
    assert_eq!(m.applies_to, AppliesTo::Platforms(vec!["nintendo_super_nes".into(), "sony_playstation".into()]));
    assert!(m.warnings.iter().any(|w| w.contains("\"holodeck\"")), "{:?}", m.warnings);
    // Never a download, a widened audience or a lead: an extension follows the sources already there.
    let d = Manifest::parse(GOOD).unwrap().descriptor();
    assert!(d.download.is_none() && d.also_for.is_none() && d.leads_for.is_none());
    assert!(d.needs != Needs::Download && d.only_on.is_empty() && !d.manual_only);
}

#[test]
fn a_pictures_source_provides_pictures_only() {
    let text = with("kind = \"details\"", "kind = \"pictures\"");
    assert!(error(&text).contains("pictures only, not \"description\""));
}

#[test]
fn unknown_keys_are_refused_in_network_and_left_out_elsewhere() {
    assert!(error(&with("rate = \"30/min\"", "rate = \"30/min\"\nproxy = \"x\"")).contains("network.proxy"));
    let m = Manifest::parse(&format!("{}\n[later]\nx = 1", with("adult = false", "adult = false\nlicense = \"MIT\""))).unwrap();
    assert!(m.warnings.iter().any(|w| w.contains("license")), "{:?}", m.warnings);
    assert!(m.warnings.iter().any(|w| w.contains("later")));
    let m = Manifest::parse(&with("asked = \"fallback\"", "asked = \"fallback\"\nregion = \"eu\"")).unwrap();
    assert!(m.warnings.iter().any(|w| w.contains("applies.region")));
    let m = Manifest::parse(&with("fields = [\"description\",", "fields = [\"engine\", \"description\",")).unwrap();
    assert_eq!(m.provides, ["description", "genres", "cover", "coverWide"]);
    assert!(m.warnings.iter().any(|w| w.contains("\"engine\"")));
}

#[test]
fn text_shown_on_one_line_stays_on_one_line() {
    // A line break only in the description.
    for (from, to) in [
        ("name = \"Example DB\"", "name = \"Example\\nDB\""),
        ("author = \"Someone\"", "author = \"Someone\\n[INFO] Extension trusted\""),
        ("label = \"API key\"", "label = \"API\\nkey\""),
    ] {
        assert!(error(&with(from, to)).contains("line breaks"), "{to}");
    }
    let m = Manifest::parse(&with("description = \"Games from Example DB.\"", "description = \"Games.\\nMore games.\"")).unwrap();
    assert_eq!(m.description, "Games.\nMore games.");
    // Invisible formatting: a direction override, a zero-width space.
    for to in ["author = \"evil\u{202e}moc.elpmaxe\"", "name = \"Exam\u{200b}ple\"", "description = \"Games\u{2066}.\""] {
        let from = to.split(" = ").next().unwrap();
        let line = GOOD.lines().find(|l| l.starts_with(&format!("{from} = "))).unwrap();
        assert!(error(&with(line, to)).contains("invisible"), "{to:?}");
    }
    // An unknown key is the author's text: escaped before it is logged.
    let m = Manifest::parse(&with("adult = false", "adult = false\n\"x\\n[WARN] forged\" = 1\n\"y\u{202e}z\" = 2")).unwrap();
    assert!(m.warnings.iter().all(|w| !w.contains('\n') && !w.contains('\u{202e}')), "{:?}", m.warnings);
    assert!(m.warnings.iter().any(|w| w.contains("x\\n[WARN] forged")), "{:?}", m.warnings);
    assert_eq!(one_line("a\nb\u{202e}c"), "a\\nb\\u{202e}c");
}
