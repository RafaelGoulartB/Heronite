//! The games of the Microsoft Store and PC Game Pass on Windows: the Xbox
//! app (or the Store) installs them as packages, each with its folder and
//! its applications. Heronite reads, without starting anything, which of
//! the user's packages are games ([`games`]) and keeps them in the
//! Collection ([`collection`]); Windows starts them by their application's
//! id (`heronite_launch`, `launchKind` `xbox`).
//!
//! A package is a game when its folder has `MicrosoftGame.config` (games
//! made with Microsoft's Game Development Kit, which the Xbox app installs
//! under `XboxGames`) or `xboxservices.config` (Store games that sign in to
//! Xbox). Owned and Game Pass games are the same packages: what tells them
//! apart is the user's licence, which only Microsoft's services know.
//!
//! * [`files`]: what `MicrosoftGame.config` and `AppxManifest.xml` say.

pub mod collection;
pub mod files;

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use heronite_platform::packages::{self, Package};

/// The Xbox app's package family.
pub const XBOX_APP_FAMILY: &str = "Microsoft.GamingApp_8wekyb3d8bbwe";
/// The Xbox app's AUMID: what opens its window.
pub const XBOX_APP: &str = "Microsoft.GamingApp_8wekyb3d8bbwe!Microsoft.Xbox.App";

/// Microsoft's own packages that carry Xbox files and are not games.
const NOT_GAMES: [&str; 8] = [
    "Microsoft.GamingApp",
    "Microsoft.GamingServices",
    "Microsoft.XboxApp",
    "Microsoft.XboxGameCallableUI",
    "Microsoft.XboxGameOverlay",
    "Microsoft.XboxGamingOverlay",
    "Microsoft.XboxIdentityProvider",
    "Microsoft.XboxSpeechToTextOverlay",
];

/// A game installed as a package.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct XboxGame {
    /// The package family name (`Microsoft.624F8B84B80_8wekyb3d8bbwe`):
    /// the same across versions, it names the Collection entry
    /// ([`collection::app_name`]).
    pub family: String,
    /// The application Windows starts (`Game`, `App`).
    pub app_id: String,
    pub title: String,
    /// The package's folder: where the game's programs run from.
    pub folder: PathBuf,
    /// The game's product id in the Store, when the package names it.
    pub store_id: Option<String>,
}

impl XboxGame {
    /// What Windows starts: `<family>!<app id>`.
    pub fn aumid(&self) -> String {
        format!("{}!{}", self.family, self.app_id)
    }
}

/// The games among this user's packages, by title. Reads the registry and
/// a few small files; empty on other systems.
pub fn games() -> Vec<XboxGame> {
    games_in(&packages::installed(), &packages::resolve)
}

/// Whether the Xbox app is installed for this user.
pub fn xbox_app_installed() -> bool {
    packages::installed().iter().any(|p| p.family_name() == XBOX_APP_FAMILY)
}

/// The games among `list`; `resolve` turns a package's display name into
/// text ([`packages::resolve`]).
pub fn games_in(list: &[Package], resolve: &dyn Fn(&str, &str) -> Option<String>) -> Vec<XboxGame> {
    // One package per family: the newest version whose folder is there.
    let mut newest: HashMap<String, &Package> = HashMap::new();
    for p in list.iter().filter(|p| is_game(p)) {
        let family = p.family_name();
        match newest.get(&family) {
            Some(known) if version(&known.full_name) >= version(&p.full_name) => {}
            _ => {
                newest.insert(family, p);
            }
        }
    }
    let mut out: Vec<XboxGame> = newest.into_iter().filter_map(|(family, p)| game(family, p, resolve)).collect();
    out.sort_by(|a, b| a.title.to_lowercase().cmp(&b.title.to_lowercase()).then_with(|| a.family.cmp(&b.family)));
    out
}

fn is_game(p: &Package) -> bool {
    !NOT_GAMES.iter().any(|n| p.name().eq_ignore_ascii_case(n))
        && [files::GAME_CONFIG, files::XBOX_SERVICES].iter().any(|f| has_file(&p.root, f))
}

fn has_file(dir: &Path, name: &str) -> bool {
    dir.join(name).is_file()
}

/// `1.2.3.0` of a full name, as numbers to compare.
fn version(full_name: &str) -> Vec<u64> {
    full_name.split('_').nth(1).unwrap_or_default().split('.').map(|n| n.parse().unwrap_or(0)).collect()
}

fn game(family: String, p: &Package, resolve: &dyn Fn(&str, &str) -> Option<String>) -> Option<XboxGame> {
    let config = files::game_config(&p.root).unwrap_or_default();
    let manifest = files::manifest(&p.root).unwrap_or_default();
    let app_id = app_id(&p.apps, &config.apps, &manifest.apps)?;
    let title = resolve(&p.full_name, &p.display_name)
        .or(config.title)
        .or(manifest.title)
        .unwrap_or_else(|| p.name().rsplit('.').next().unwrap_or(p.name()).to_string());
    Some(XboxGame { family, app_id, title, folder: p.root.clone(), store_id: config.store_id })
}

/// The application to start: the game's own program (`MicrosoftGame.config`),
/// else the first one the Start menu shows, among those Windows registered.
/// When the registry names none, the files are trusted.
fn app_id(registered: &[String], config: &[String], manifest: &[String]) -> Option<String> {
    let known = |id: &&String| registered.is_empty() || registered.iter().any(|r| r.eq_ignore_ascii_case(id));
    config
        .iter()
        .find(known)
        .or_else(|| manifest.iter().find(known))
        .or_else(|| registered.first())
        .or_else(|| config.first())
        .or_else(|| manifest.first())
        .cloned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn package(dir: &Path, full_name: &str, display: &str, apps: &[&str], files: &[(&str, &str)]) -> Package {
        let root = dir.join(full_name);
        std::fs::create_dir_all(&root).unwrap();
        for (name, text) in files {
            std::fs::write(root.join(name), text).unwrap();
        }
        Package { full_name: full_name.into(), root, display_name: display.into(), apps: apps.iter().map(|a| a.to_string()).collect() }
    }

    fn no_lookup(_: &str, text: &str) -> Option<String> {
        (!text.is_empty() && !text.starts_with("ms-resource:")).then(|| text.to_string())
    }

    const GDK: &str = r#"<Game><ExecutableList><Executable Name="FH5.exe" Id="Game"/></ExecutableList>
        <ShellVisuals DefaultDisplayName="Forza Horizon 5"/><StoreId>9NKX70BBCDRN</StoreId></Game>"#;
    const UWP: &str = r#"<Package><Properties><DisplayName>ms-resource:Name</DisplayName></Properties>
        <Applications><Application Id="App"><VisualElements DisplayName="Sea of Thieves"/></Application></Applications></Package>"#;

    #[test]
    fn finds_the_games_and_how_to_start_them() {
        let dir = tempfile::tempdir().unwrap();
        let list = [
            package(
                dir.path(),
                "Microsoft.624F8B84B80_3.0.0.0_x64__8wekyb3d8bbwe",
                "ms-resource:Title",
                &["Game"],
                &[(files::GAME_CONFIG, GDK)],
            ),
            package(
                dir.path(),
                "Microsoft.SeaofThieves_2.0.0.0_x64__8wekyb3d8bbwe",
                "ms-resource:Name",
                &[],
                &[(files::XBOX_SERVICES, "{}"), (files::MANIFEST, UWP)],
            ),
            package(
                dir.path(),
                "Microsoft.WindowsCalculator_11.0.0.0_x64__8wekyb3d8bbwe",
                "Calculator",
                &["App"],
                &[(files::MANIFEST, UWP)],
            ),
            package(
                dir.path(),
                "Microsoft.GamingApp_2607.0.0.0_x64__8wekyb3d8bbwe",
                "Xbox",
                &["Microsoft.Xbox.App"],
                &[(files::XBOX_SERVICES, "{}")],
            ),
        ];
        let games = games_in(&list, &no_lookup);
        let got: Vec<_> = games.iter().map(|g| (g.title.as_str(), g.aumid(), g.store_id.as_deref())).collect();
        assert_eq!(
            got,
            [
                ("Forza Horizon 5", "Microsoft.624F8B84B80_8wekyb3d8bbwe!Game".to_string(), Some("9NKX70BBCDRN")),
                ("Sea of Thieves", "Microsoft.SeaofThieves_8wekyb3d8bbwe!App".to_string(), None),
            ]
        );
        assert_eq!(games[0].folder, list[0].root);
    }

    #[test]
    fn the_resolved_name_wins_and_the_newest_version_is_kept() {
        let dir = tempfile::tempdir().unwrap();
        let list = [
            package(dir.path(), "Studio.Game_1.10.0.0_x64__abc", "ms-resource:Title", &["Game"], &[(files::GAME_CONFIG, GDK)]),
            package(dir.path(), "Studio.Game_1.9.0.0_x64__abc", "ms-resource:Title", &["Game"], &[(files::GAME_CONFIG, GDK)]),
        ];
        let resolve = |full: &str, _: &str| Some(format!("Localised {full}"));
        let games = games_in(&list, &resolve);
        assert_eq!(games.len(), 1);
        assert_eq!(games[0].title, "Localised Studio.Game_1.10.0.0_x64__abc");
        assert_eq!(games[0].family, "Studio.Game_abc");
    }

    #[test]
    fn picks_the_registered_application() {
        let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        assert_eq!(app_id(&s(&["Game"]), &s(&["Console", "Game"]), &[]).as_deref(), Some("Game"));
        assert_eq!(app_id(&s(&["Other"]), &s(&["Game"]), &[]).as_deref(), Some("Other"));
        assert_eq!(app_id(&[], &s(&["Game"]), &s(&["App"])).as_deref(), Some("Game"));
        assert_eq!(app_id(&[], &[], &s(&["App"])).as_deref(), Some("App"));
        assert_eq!(app_id(&[], &[], &[]), None);
    }

    #[test]
    fn a_package_without_a_name_is_named_after_itself() {
        let dir = tempfile::tempdir().unwrap();
        let list = [package(dir.path(), "Studio.HiddenGem_1.0.0.0_x64__abc", "", &["App"], &[(files::XBOX_SERVICES, "{}")])];
        assert_eq!(games_in(&list, &no_lookup)[0].title, "HiddenGem");
    }
}
