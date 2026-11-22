//! Reading what Playnite's games carry over, with this system's paths (the
//! rest of Playnite's tests use Windows' own).

use std::path::{Path, PathBuf, MAIN_SEPARATOR as SEP};

use heronite_data::game_actions::ActionKind;

use super::*;
use crate::litedb::write::{file, guid_bytes};
use crate::transfer::{Rom, Start};

fn guid(text: &str) -> Bson {
    Bson::Binary(4, guid_bytes(text))
}

fn s(text: &str) -> Bson {
    Bson::String(text.into())
}

fn doc(fields: Vec<(&str, Bson)>) -> Document {
    Document(fields.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
}

fn named(id: &str, name: &str) -> Document {
    doc(vec![("_id", guid(id)), ("Name", s(name))])
}

fn action(fields: Vec<(&str, Bson)>) -> Bson {
    Bson::Document(doc(fields))
}

fn game(id: &str, name: &str, extra: Vec<(&str, Bson)>) -> Document {
    let mut fields = vec![("_id", guid(id)), ("Name", s(name)), ("IsInstalled", Bson::Bool(true))];
    fields.extend(extra);
    doc(fields)
}

const DUCK: &str = "b289681d-4922-4bc4-b54f-9fb0cf2525dc";
const PCSX2: &str = "c289681d-4922-4bc4-b54f-9fb0cf2525dc";
const RPG: &str = "10000000-0000-0000-0000-000000000001";
const CDPR: &str = "20000000-0000-0000-0000-000000000001";
const BEATEN: &str = "30000000-0000-0000-0000-000000000001";
const COUCH: &str = "40000000-0000-0000-0000-000000000001";
const PS1: &str = "50000000-0000-0000-0000-000000000001";

fn write(path: &Path, bytes: &[u8]) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, bytes).unwrap();
}

/// A Playnite in `root` with an emulator of each kind of profile.
fn playnite(root: &Path, games: &[Document]) -> Install {
    let install = Install { program: root.join("Playnite"), library: root.join("Playnite").join("library") };
    let lib = &install.library;
    write(&lib.join("games.db"), &file(games));
    write(&lib.join("genres.db"), &file(&[named(RPG, "Role Playing")]));
    write(&lib.join("companies.db"), &file(&[named(CDPR, "CD PROJEKT RED")]));
    write(&lib.join("completionstatuses.db"), &file(&[named(BEATEN, "Beaten")]));
    write(&lib.join("categories.db"), &file(&[named(COUCH, "Couch")]));
    write(&lib.join("platforms.db"), &file(&[named(PS1, "Sony PlayStation")]));
    let duck_dir = root.join("emu").join("duck");
    let pcsx2_dir = root.join("emu").join("pcsx2");
    write(&duck_dir.join("bin").join("DuckStation-Qt-x64-ReleaseLTCG.exe"), b"");
    write(&pcsx2_dir.join("pcsx2-qt.exe"), b"");
    let emulators = [
        doc(vec![
            ("_id", guid(DUCK)),
            ("Name", s("DuckStation")),
            ("InstallDir", s(&duck_dir.to_string_lossy())),
            ("BuiltInConfigId", s("duckstation")),
            (
                "BuiltinProfiles",
                Bson::Array(vec![action(vec![("_id", s("#builtin_1")), ("Name", s("Default")), ("BuiltInProfileName", s("Default"))])]),
            ),
        ]),
        doc(vec![
            ("_id", guid(PCSX2)),
            ("Name", s("PCSX2")),
            ("InstallDir", s(&pcsx2_dir.to_string_lossy())),
            (
                "CustomProfiles",
                Bson::Array(vec![action(vec![
                    ("_id", s("#custom_1")),
                    ("Name", s("Fullscreen")),
                    ("Executable", s(&format!("{{EmulatorDir}}{SEP}pcsx2-qt.exe"))),
                    ("Arguments", s(r#"-fullscreen "{ImagePath}""#)),
                ])]),
            ),
        ]),
    ];
    write(&lib.join("emulators.db"), &file(&emulators));
    let yaml = "Id: duckstation\nName: DuckStation\nProfiles:\n  - Name: Default\n    StartupArguments: '-batch -fullscreen \"{ImagePath}\"'\n    StartupExecutable: ^duckstation-qt-x64-ReleaseLTCG\\.exe$\n";
    write(&install.program.join("Emulation").join("Emulators").join("DuckStation").join("emulator.yaml"), yaml.as_bytes());
    install
}

fn path(root: &Path, parts: &[&str]) -> PathBuf {
    parts.iter().fold(root.to_path_buf(), |p, part| p.join(part))
}

#[test]
fn its_games_come_with_how_they_start_and_what_they_carry() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path();
    let witcher_dir = path(root, &["Games", "Witcher 3"]);
    let sotn_dir = path(root, &["ROMs", "PS1"]);
    let games = [
        game(
            "00000001-0000-0000-0000-000000000001",
            "The Witcher 3",
            vec![
                ("InstallDirectory", s(&witcher_dir.to_string_lossy())),
                ("Hidden", Bson::Bool(true)),
                ("Favorite", Bson::Bool(true)),
                ("UserScore", Bson::Int32(90)),
                ("CriticScore", Bson::Int32(93)),
                ("CompletionStatusId", guid(BEATEN)),
                ("CategoryIds", Bson::Array(vec![guid(COUCH)])),
                ("GenreIds", Bson::Array(vec![guid(RPG)])),
                ("DeveloperIds", Bson::Array(vec![guid(CDPR)])),
                ("ReleaseDate", s("2015-5-19")),
                ("Added", Bson::DateTime(1_622_505_600_000)),
                ("Notes", s("Blood and Wine next")),
                ("Description", s("<p>Geralt.</p>")),
                ("Links", Bson::Array(vec![action(vec![("Name", s("Store")), ("Url", s("https://example.com/w3"))])])),
                (
                    "GameActions",
                    Bson::Array(vec![
                        action(vec![("Name", s("Wiki")), ("Type", s("URL")), ("Path", s("https://witcher.example"))]),
                        action(vec![
                            ("Name", s("Play")),
                            ("Type", s("File")),
                            ("Path", s(&format!("{{InstallDir}}{SEP}bin{SEP}witcher3.exe"))),
                            ("Arguments", s("-skipintro")),
                            ("WorkingDir", s("{InstallDir}")),
                            ("IsPlayAction", Bson::Bool(true)),
                        ]),
                        action(vec![("Name", s("Settings")), ("Type", s("File")), ("Path", s(&format!("{{InstallDir}}{SEP}config.exe")))]),
                    ]),
                ),
            ],
        ),
        game(
            "00000002-0000-0000-0000-000000000002",
            "Castlevania",
            vec![
                ("InstallDirectory", s(&sotn_dir.to_string_lossy())),
                ("PlatformIds", Bson::Array(vec![guid(PS1)])),
                ("Roms", Bson::Array(vec![action(vec![("Name", s("Disc")), ("Path", s(&format!("{{InstallDir}}{SEP}sotn.cue")))])])),
                (
                    "GameActions",
                    Bson::Array(vec![action(vec![
                        ("Type", s("Emulator")),
                        ("EmulatorId", guid(DUCK)),
                        ("EmulatorProfileId", s("#builtin_1")),
                        ("AdditionalArguments", s("-nogui")),
                        ("IsPlayAction", Bson::Bool(true)),
                    ])]),
                ),
            ],
        ),
        game(
            "00000003-0000-0000-0000-000000000003",
            "Okami",
            vec![
                ("Roms", Bson::Array(vec![action(vec![("Path", s(&path(root, &["ROMs", "okami.iso"]).to_string_lossy()))])])),
                (
                    "GameActions",
                    Bson::Array(vec![action(vec![
                        ("Type", s("Emulator")),
                        ("EmulatorId", guid(PCSX2)),
                        ("EmulatorProfileId", s("#custom_1")),
                        ("OverrideDefaultArgs", Bson::Bool(true)),
                        ("Arguments", s(r#"-batch -- "{ImagePath}""#)),
                        ("IsPlayAction", Bson::Bool(true)),
                    ])]),
                ),
            ],
        ),
        // Started by a link, and a hidden game only Playnite can start.
        game(
            "00000004-0000-0000-0000-000000000004",
            "Linked",
            vec![(
                "GameActions",
                Bson::Array(vec![action(vec![("Type", s("URL")), ("Path", s("steam://run/1")), ("IsPlayAction", Bson::Bool(true))])]),
            )],
        ),
        game("00000005-0000-0000-0000-000000000005", "Hidden", vec![("Hidden", Bson::Bool(true))]),
    ];
    let install = playnite(root, &games);
    let found = read(&install).unwrap();
    let titles: Vec<_> = found.games.iter().map(|g| g.title.as_str()).collect();
    assert_eq!(titles, ["Castlevania", "Linked", "Okami", "The Witcher 3"]);
    let by = |title: &str| found.games.iter().find(|g| g.title == title).unwrap();

    let linked = by("Linked");
    assert!(linked.transfer.is_none());
    assert_eq!(linked.command, "explorer.exe playnite://playnite/start/00000004-0000-0000-0000-000000000004");

    let witcher = by("The Witcher 3").transfer.as_deref().unwrap();
    assert_eq!(
        witcher.start,
        Start::Program {
            path: path(&witcher_dir, &["bin", "witcher3.exe"]).to_string_lossy().into_owned(),
            args: "-skipintro".into(),
            working_dir: Some(witcher_dir.to_string_lossy().into_owned()),
        }
    );
    assert!(witcher.favourite && witcher.hidden);
    assert_eq!((witcher.stars, witcher.status.clone()), (Some(5), Some((BEATEN.to_string(), "Beaten".to_string()))));
    assert_eq!((witcher.categories.as_slice(), witcher.notes.as_deref()), (&["Couch".to_string()][..], Some("Blood and Wine next")));
    assert_eq!(witcher.added.as_deref(), Some("2021-06-01"));
    assert_eq!(witcher.library, install.library);
    let details = &witcher.details;
    assert_eq!(
        (details.genres.as_slice(), details.developers.as_slice()),
        (&["Role Playing".to_string()][..], &["CD PROJEKT RED".to_string()][..])
    );
    assert_eq!((details.release_date.as_deref(), details.critic_score), (Some("2015-05-19"), Some(93)));
    assert_eq!(
        (details.links.as_slice(), details.description.as_deref()),
        (&["https://example.com/w3".to_string()][..], Some("<p>Geralt.</p>"))
    );
    let actions: Vec<_> = witcher.actions.iter().map(|a| (a.name.as_str(), a.kind, a.target.as_str())).collect();
    let settings = path(&witcher_dir, &["config.exe"]).to_string_lossy().into_owned();
    assert_eq!(actions, [("Wiki", ActionKind::Open, "https://witcher.example"), ("Settings", ActionKind::Program, settings.as_str())]);
    assert!(witcher.actions[0].id != witcher.actions[1].id);

    let sotn = by("Castlevania").transfer.as_deref().unwrap();
    let duck = path(root, &["emu", "duck", "bin", "DuckStation-Qt-x64-ReleaseLTCG.exe"]);
    let rom = Rom { name: Some("Disc".into()), path: path(&sotn_dir, &["sotn.cue"]).to_string_lossy().into_owned() };
    let args = start::shlex_literal(r#"-batch -fullscreen "{ImagePath}" -nogui"#);
    assert_eq!(
        sotn.start,
        Start::Emulator { name: "DuckStation".into(), program: duck.to_string_lossy().into_owned(), args, roms: vec![rom] }
    );
    assert_eq!(sotn.details.platforms, ["Sony PlayStation"]);

    let okami = by("Okami").transfer.as_deref().unwrap();
    let Start::Emulator { program, args, .. } = &okami.start else { panic!("{:?}", okami.start) };
    assert_eq!(Path::new(program), path(root, &["emu", "pcsx2", "pcsx2-qt.exe"]));
    assert_eq!(args, r#"-batch -- "{ImagePath}""#);
}
