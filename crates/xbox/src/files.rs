//! What a game package says about itself: `MicrosoftGame.config` (the games
//! built with Microsoft's Game Development Kit, which the Xbox app installs
//! under `XboxGames`) and `AppxManifest.xml` (every package). Both are read
//! leniently: a file that is missing or broken says nothing.

use std::path::Path;

/// `MicrosoftGame.config`: a GDK game.
pub const GAME_CONFIG: &str = "MicrosoftGame.config";
/// `xboxservices.config`: a Store game that signs in to Xbox (older UWP
/// games have no `MicrosoftGame.config`).
pub const XBOX_SERVICES: &str = "xboxservices.config";
pub const MANIFEST: &str = "AppxManifest.xml";

/// What `MicrosoftGame.config` names.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GameConfig {
    /// `ShellVisuals/@DefaultDisplayName`, when it is plain text.
    pub title: Option<String>,
    /// The `Id` of each `Executable` (an application of the package), PC
    /// ones first.
    pub apps: Vec<String>,
    /// `StoreId`: the game's product id in the Store (`9NKX70BBCDRN`).
    pub store_id: Option<String>,
}

/// What `AppxManifest.xml` names.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Manifest {
    /// `Properties/DisplayName`, else the first shown application's
    /// `VisualElements/@DisplayName`, when it is plain text.
    pub title: Option<String>,
    /// The `Id` of each application shown in the Start menu, in order.
    pub apps: Vec<String>,
}

/// Plain text, not an `ms-resource:` reference that only Windows resolves.
fn plain(s: &str) -> Option<String> {
    let s = s.trim();
    (!s.is_empty() && !s.to_ascii_lowercase().starts_with("ms-resource:")).then(|| s.to_string())
}

fn named<'a, 'i>(node: roxmltree::Node<'a, 'i>, name: &'static str) -> impl Iterator<Item = roxmltree::Node<'a, 'i>> {
    node.descendants().filter(move |n| n.is_element() && n.tag_name().name().eq_ignore_ascii_case(name))
}

fn attr<'a>(node: roxmltree::Node<'a, '_>, name: &str) -> Option<&'a str> {
    node.attributes().find(|a| a.name().eq_ignore_ascii_case(name)).map(|a| a.value())
}

/// Text of a file, without a byte order mark; `None` when it cannot be read.
fn read(path: &Path) -> Option<String> {
    let bytes = std::fs::read(path).ok()?;
    let text = match bytes.as_slice() {
        [0xFF, 0xFE, rest @ ..] => {
            String::from_utf16_lossy(&rest.as_chunks::<2>().0.iter().map(|c| u16::from_le_bytes(*c)).collect::<Vec<_>>())
        }
        [0xEF, 0xBB, 0xBF, rest @ ..] => String::from_utf8_lossy(rest).into_owned(),
        _ => String::from_utf8_lossy(&bytes).into_owned(),
    };
    Some(text)
}

/// `MicrosoftGame.config` in `root`, when it is there and reads as XML.
pub fn game_config(root: &Path) -> Option<GameConfig> {
    parse_game_config(&read(&root.join(GAME_CONFIG))?)
}

fn parse_game_config(text: &str) -> Option<GameConfig> {
    let doc = roxmltree::Document::parse(text).ok()?;
    let root = doc.root();
    let title = named(root, "ShellVisuals").find_map(|n| attr(n, "DefaultDisplayName").and_then(plain));
    let mut executables: Vec<(bool, String)> = named(root, "Executable")
        .filter_map(|n| {
            let id = attr(n, "Id").map(str::trim).filter(|s| !s.is_empty())?;
            let pc = attr(n, "TargetDeviceFamily").is_none_or(|f| f.eq_ignore_ascii_case("PC"));
            Some((pc, id.to_string()))
        })
        .collect();
    // PC first, each group in the file's order.
    executables.sort_by_key(|(pc, _)| !pc);
    let mut apps: Vec<String> = Vec::new();
    for (_, id) in executables {
        if !apps.contains(&id) {
            apps.push(id);
        }
    }
    let store_id = named(root, "StoreId").find_map(|n| n.text()).map(str::trim).filter(|s| !s.is_empty()).map(str::to_string);
    Some(GameConfig { title, apps, store_id })
}

/// `AppxManifest.xml` in `root`, when it is there and reads as XML.
pub fn manifest(root: &Path) -> Option<Manifest> {
    parse_manifest(&read(&root.join(MANIFEST))?)
}

fn parse_manifest(text: &str) -> Option<Manifest> {
    let doc = roxmltree::Document::parse(text).ok()?;
    let root = doc.root();
    let property = named(root, "Properties").flat_map(|p| named(p, "DisplayName")).find_map(|n| n.text().and_then(plain));
    let mut apps = Vec::new();
    let mut shown_title = None;
    for app in named(root, "Application") {
        let Some(id) = attr(app, "Id").map(str::trim).filter(|s| !s.is_empty()) else { continue };
        let visuals = named(app, "VisualElements").next();
        // `AppListEntry="none"`: a helper the Start menu does not show.
        if visuals.and_then(|v| attr(v, "AppListEntry")).is_some_and(|e| e.eq_ignore_ascii_case("none")) {
            continue;
        }
        if shown_title.is_none() {
            shown_title = visuals.and_then(|v| attr(v, "DisplayName")).and_then(plain);
        }
        apps.push(id.to_string());
    }
    Some(Manifest { title: property.or(shown_title), apps })
}

#[cfg(test)]
mod tests {
    use super::*;

    const CONFIG: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<Game configVersion="1">
  <Identity Name="Microsoft.624F8B84B80" Publisher="CN=A4954634-DF4B-47C7-AB70-D3215D246AF1" Version="3.0.0.0" />
  <ExecutableList>
    <Executable Name="Launcher.exe" TargetDeviceFamily="Scarlett" Id="Console" />
    <Executable Name="ForzaHorizon5.exe" TargetDeviceFamily="PC" Id="Game" />
    <Executable Name="Tool.exe" Id="Tool" />
  </ExecutableList>
  <ShellVisuals DefaultDisplayName="Forza Horizon 5" PublisherDisplayName="Xbox Game Studios" StoreLogo="StoreLogo.png" />
  <StoreId> 9NKX70BBCDRN </StoreId>
</Game>"#;

    #[test]
    fn a_game_config_names_its_title_store_id_and_pc_programs_first() {
        let c = parse_game_config(CONFIG).unwrap();
        assert_eq!(c.title.as_deref(), Some("Forza Horizon 5"));
        assert_eq!(c.apps, ["Game", "Tool", "Console"]);
        assert_eq!(c.store_id.as_deref(), Some("9NKX70BBCDRN"));
    }

    #[test]
    fn resource_titles_and_broken_files_say_nothing() {
        let c = parse_game_config(r#"<Game><ShellVisuals DefaultDisplayName="ms-resource:Title"/></Game>"#).unwrap();
        assert_eq!(c, GameConfig::default());
        assert_eq!(parse_game_config("<Game><unclosed></Game>"), None);
    }

    const MANIFEST_XML: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<Package xmlns="http://schemas.microsoft.com/appx/manifest/foundation/windows10"
         xmlns:uap="http://schemas.microsoft.com/appx/manifest/uap/windows10">
  <Identity Name="Microsoft.SunriseBaseGame" Publisher="CN=Microsoft Corporation" Version="1.0.0.0" />
  <Properties>
    <DisplayName>ms-resource:AppDisplayName</DisplayName>
  </Properties>
  <Applications>
    <Application Id="Helper" Executable="Helper.exe">
      <uap:VisualElements DisplayName="Helper" AppListEntry="none" />
    </Application>
    <Application Id="App" Executable="Sunrise.exe">
      <uap:VisualElements DisplayName="Sea of Thieves" Square150x150Logo="Logo.png" />
    </Application>
  </Applications>
</Package>"#;

    #[test]
    fn a_manifest_names_its_shown_applications() {
        let m = parse_manifest(MANIFEST_XML).unwrap();
        assert_eq!(m.apps, ["App"]);
        assert_eq!(m.title.as_deref(), Some("Sea of Thieves"), "the property is a resource: the shown application's name");
        let plain = MANIFEST_XML.replace("ms-resource:AppDisplayName", "Sea of Thieves (Store)");
        assert_eq!(parse_manifest(&plain).unwrap().title.as_deref(), Some("Sea of Thieves (Store)"));
    }

    #[test]
    fn reads_the_files_in_either_encoding() {
        let dir = tempfile::tempdir().unwrap();
        let utf16: Vec<u8> =
            [0xFF, 0xFE].into_iter().chain(CONFIG.replace("utf-8", "utf-16").encode_utf16().flat_map(u16::to_le_bytes)).collect();
        std::fs::write(dir.path().join(GAME_CONFIG), utf16).unwrap();
        assert_eq!(game_config(dir.path()).unwrap().store_id.as_deref(), Some("9NKX70BBCDRN"));
        std::fs::write(dir.path().join(MANIFEST), [&[0xEF, 0xBB, 0xBF][..], MANIFEST_XML.as_bytes()].concat()).unwrap();
        assert_eq!(manifest(dir.path()).unwrap().apps, ["App"]);
        assert_eq!(manifest(&dir.path().join("missing")), None);
    }
}
