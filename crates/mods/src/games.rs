//! How the popular modded games lay out their mods: the folder loose mod
//! files go to, Bethesda's plugin list, BepInEx, and the game's names on
//! Nexus Mods and Thunderstore. A game that is not here is [`Kind::Generic`]
//! with its files going to the game folder (the user can pick a folder in the
//! manager, [`GameLinks::mod_root`]).

use crate::model::GameLinks;

/// How a game loads mods.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// Bethesda's engines: mods go into `Data`, plugins are listed in
    /// `Plugins.txt`, script extenders go next to the executable.
    Bethesda,
    /// Unity games modded through BepInEx (Thunderstore's games): r2modman's
    /// layout, `BepInEx/plugins/<Namespace-Name>`.
    BepInEx,
    Generic,
}

/// Bethesda's `Plugins.txt`, in `%LOCALAPPDATA%/<folder>` of the prefix.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PluginsSpec {
    /// Folder names under `AppData/Local`, the first that exists wins (the
    /// Steam, GOG and Epic builds use different ones).
    pub appdata_folders: &'static [&'static str],
    /// `*Name.esp` marks an active plugin (Skyrim SE, Fallout 4, Starfield);
    /// older games list only the active plugins, without the star.
    pub star_format: bool,
    /// Masters the game loads by itself: never written to the list.
    pub implicit: &'static [&'static str],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KnownGame {
    pub name: &'static str,
    pub steam_app_ids: &'static [&'static str],
    /// Other titles the game is sold under (store titles), compared after
    /// [`normalize_title`].
    pub titles: &'static [&'static str],
    pub nexus_domain: Option<&'static str>,
    pub thunderstore: Option<&'static str>,
    pub kind: Kind,
    /// Folder (relative to the game folder) loose mod files go to; `""` =
    /// the game folder.
    pub mod_root: &'static str,
    pub plugins: Option<PluginsSpec>,
}

const SKYRIM_SE_MASTERS: &[&str] = &["Skyrim.esm", "Update.esm", "Dawnguard.esm", "HearthFires.esm", "Dragonborn.esm"];
const FALLOUT4_MASTERS: &[&str] = &[
    "Fallout4.esm",
    "DLCRobot.esm",
    "DLCworkshop01.esm",
    "DLCCoast.esm",
    "DLCworkshop02.esm",
    "DLCworkshop03.esm",
    "DLCNukaWorld.esm",
    "DLCUltraHighResolution.esm",
];

const fn bethesda(
    name: &'static str,
    steam_app_ids: &'static [&'static str],
    titles: &'static [&'static str],
    nexus: &'static str,
    plugins: PluginsSpec,
) -> KnownGame {
    KnownGame {
        name,
        steam_app_ids,
        titles,
        nexus_domain: Some(nexus),
        thunderstore: None,
        kind: Kind::Bethesda,
        mod_root: "Data",
        plugins: Some(plugins),
    }
}

const fn bepinex(
    name: &'static str,
    steam_app_ids: &'static [&'static str],
    community: &'static str,
    nexus: Option<&'static str>,
) -> KnownGame {
    KnownGame {
        name,
        steam_app_ids,
        titles: &[],
        nexus_domain: nexus,
        thunderstore: Some(community),
        kind: Kind::BepInEx,
        mod_root: "BepInEx/plugins",
        plugins: None,
    }
}

const fn generic(name: &'static str, steam_app_ids: &'static [&'static str], nexus: &'static str, mod_root: &'static str) -> KnownGame {
    KnownGame {
        name,
        steam_app_ids,
        titles: &[],
        nexus_domain: Some(nexus),
        thunderstore: None,
        kind: Kind::Generic,
        mod_root,
        plugins: None,
    }
}

pub const KNOWN: &[KnownGame] = &[
    bethesda(
        "Skyrim Special Edition",
        &["489830"],
        &["The Elder Scrolls V: Skyrim Special Edition", "The Elder Scrolls V: Skyrim Anniversary Edition", "Skyrim Anniversary Edition"],
        "skyrimspecialedition",
        PluginsSpec {
            appdata_folders: &["Skyrim Special Edition", "Skyrim Special Edition GOG", "Skyrim Special Edition EPIC"],
            star_format: true,
            implicit: SKYRIM_SE_MASTERS,
        },
    ),
    bethesda(
        "Skyrim",
        &["72850"],
        &["The Elder Scrolls V: Skyrim"],
        "skyrim",
        PluginsSpec { appdata_folders: &["Skyrim"], star_format: false, implicit: &["Skyrim.esm", "Update.esm"] },
    ),
    bethesda(
        "Enderal: Forgotten Stories (Special Edition)",
        &["976620"],
        &["Enderal Special Edition"],
        "enderalspecialedition",
        PluginsSpec { appdata_folders: &["Enderal Special Edition"], star_format: true, implicit: SKYRIM_SE_MASTERS },
    ),
    bethesda(
        "Fallout 4",
        &["377160"],
        &["Fallout 4: Game of the Year Edition", "Fallout 4 GOTY"],
        "fallout4",
        PluginsSpec { appdata_folders: &["Fallout4"], star_format: true, implicit: FALLOUT4_MASTERS },
    ),
    bethesda(
        "Fallout: New Vegas",
        &["22380", "22490"],
        &["Fallout New Vegas", "Fallout: New Vegas Ultimate Edition"],
        "newvegas",
        PluginsSpec { appdata_folders: &["FalloutNV"], star_format: false, implicit: &["FalloutNV.esm"] },
    ),
    bethesda(
        "Fallout 3",
        &["22300", "22370"],
        &["Fallout 3: Game of the Year Edition", "Fallout 3 GOTY"],
        "fallout3",
        PluginsSpec { appdata_folders: &["Fallout3"], star_format: false, implicit: &["Fallout3.esm"] },
    ),
    bethesda(
        "The Elder Scrolls IV: Oblivion",
        &["22330"],
        &["Oblivion", "The Elder Scrolls IV: Oblivion Game of the Year Edition Deluxe"],
        "oblivion",
        PluginsSpec { appdata_folders: &["Oblivion"], star_format: false, implicit: &["Oblivion.esm"] },
    ),
    bethesda(
        "Starfield",
        &["1716740"],
        &[],
        "starfield",
        PluginsSpec {
            appdata_folders: &["Starfield"],
            star_format: true,
            implicit: &[
                "Starfield.esm",
                "Constellation.esm",
                "OldMars.esm",
                "BlueprintShips-Starfield.esm",
                "SFBGS003.esm",
                "SFBGS004.esm",
                "SFBGS006.esm",
                "SFBGS007.esm",
                "SFBGS008.esm",
                "ShatteredSpace.esm",
            ],
        },
    ),
    KnownGame {
        name: "The Elder Scrolls III: Morrowind",
        steam_app_ids: &["22320"],
        titles: &["Morrowind", "The Elder Scrolls III: Morrowind Game of the Year Edition"],
        nexus_domain: Some("morrowind"),
        thunderstore: None,
        kind: Kind::Generic,
        mod_root: "Data Files",
        plugins: None,
    },
    generic("The Elder Scrolls IV: Oblivion Remastered", &["2623190"], "oblivionremastered", ""),
    generic("Cyberpunk 2077", &["1091500"], "cyberpunk2077", ""),
    generic("The Witcher 3: Wild Hunt", &["292030"], "witcher3", "Mods"),
    generic("Baldur's Gate 3", &["1086940"], "baldursgate3", ""),
    generic("Stardew Valley", &["413150"], "stardewvalley", "Mods"),
    generic("Elden Ring", &["1245620"], "eldenring", ""),
    generic("Dark Souls III", &["374320"], "darksouls3", ""),
    generic("Sekiro: Shadows Die Twice", &["814380"], "sekiro", ""),
    generic("Monster Hunter: World", &["582010"], "monsterhunterworld", ""),
    generic("Monster Hunter Rise", &["1446780"], "monsterhunterrise", ""),
    generic("Mount & Blade II: Bannerlord", &["261550"], "mountandblade2bannerlord", "Modules"),
    generic("Mount & Blade: Warband", &["48700"], "mbwarband", "Modules"),
    generic("Kingdom Come: Deliverance", &["379430"], "kingdomcomedeliverance", "Mods"),
    generic("Kingdom Come: Deliverance II", &["1771300"], "kingdomcomedeliverance2", "Mods"),
    generic("Hogwarts Legacy", &["990080"], "hogwartslegacy", ""),
    generic("Palworld", &["1623730"], "palworld", ""),
    generic("No Man's Sky", &["275850"], "nomanssky", "GAMEDATA/MODS"),
    generic("RimWorld", &["294100"], "rimworld", "Mods"),
    generic("Kenshi", &["233860"], "kenshi", "mods"),
    generic("Dragon Age: Origins", &["17450", "47810"], "dragonage", ""),
    generic("Mass Effect Legendary Edition", &["1328670"], "masseffectlegendaryedition", ""),
    generic("Divinity: Original Sin 2", &["435150"], "divinityoriginalsin2definitiveedition", ""),
    generic("Darkest Dungeon", &["262060"], "darkestdungeon", "mods"),
    generic("Subnautica", &["264710"], "subnautica", ""),
    generic("Resident Evil 4", &["2050650"], "residentevil42023", ""),
    generic("Starbound", &["211820"], "starbound", "mods"),
    bepinex("Lethal Company", &["1966720"], "lethal-company", Some("lethalcompany")),
    bepinex("Risk of Rain 2", &["632360"], "riskofrain2", Some("riskofrain2")),
    bepinex("Valheim", &["892970"], "valheim", Some("valheim")),
    bepinex("Content Warning", &["2881650"], "content-warning", None),
    bepinex("R.E.P.O.", &["3241660"], "repo", None),
    bepinex("PEAK", &["3527290"], "peak", None),
    bepinex("Dyson Sphere Program", &["1366540"], "dyson-sphere-program", Some("dysonsphereprogram")),
    bepinex("Inscryption", &["1092790"], "inscryption", None),
    bepinex("Sun Haven", &["1432860"], "sun-haven", None),
    bepinex("Muck", &["1625450"], "muck", None),
    bepinex("Lethal League Blaze", &["553310"], "lethal-league-blaze", None),
    bepinex("ULTRAKILL", &["1229490"], "ultrakill", None),
    bepinex("Hades II", &["1145350"], "hades-ii", None),
];

/// Lower case, letters and digits only, `&` = `and`, without "the" and the
/// usual edition words: titles from stores and mod sites compare equal.
pub fn normalize_title(title: &str) -> String {
    let lower = title.to_lowercase().replace('&', " and ").replace(['™', '®'], "");
    let words: Vec<String> = lower.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).map(str::to_string).collect();
    const NOISE: &[&str] = &["the", "edition", "goty", "game", "of", "year", "definitive", "enhanced", "complete", "remastered"];
    let kept: Vec<&str> = words.iter().map(String::as_str).filter(|w| !NOISE.contains(w)).collect();
    if kept.is_empty() {
        words.concat()
    } else {
        kept.concat()
    }
}

/// The known game with this Steam app id, else with this title.
pub fn find(steam_app_id: Option<&str>, title: &str) -> Option<&'static KnownGame> {
    if let Some(id) = steam_app_id.map(str::trim).filter(|s| !s.is_empty()) {
        if let Some(g) = KNOWN.iter().find(|g| g.steam_app_ids.contains(&id)) {
            return Some(g);
        }
    }
    let t = normalize_title(title);
    if t.is_empty() {
        return None;
    }
    KNOWN.iter().find(|g| normalize_title(g.name) == t || g.titles.iter().any(|x| normalize_title(x) == t))
}

pub fn by_nexus_domain(domain: &str) -> Option<&'static KnownGame> {
    KNOWN.iter().find(|g| g.nexus_domain.is_some_and(|d| d.eq_ignore_ascii_case(domain)))
}

pub fn by_thunderstore(community: &str) -> Option<&'static KnownGame> {
    KNOWN.iter().find(|g| g.thunderstore.is_some_and(|c| c.eq_ignore_ascii_case(community)))
}

/// How the mods of one Collection game are laid out: the known game's
/// table entry with the user's choices of the manager on top.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Profile {
    pub kind: Kind,
    /// Relative to the game folder, `/`-separated, `""` = the game folder.
    pub mod_root: String,
    pub plugins: Option<PluginsSpec>,
}

impl Profile {
    /// `known` from [`find`]; a game linked to a Thunderstore community and
    /// not in the table is a BepInEx game.
    pub fn of(links: &GameLinks, known: Option<&'static KnownGame>) -> Profile {
        let kind = match known {
            Some(k) => k.kind,
            None if links.thunderstore.is_some() => Kind::BepInEx,
            None => Kind::Generic,
        };
        let default_root = match (known, kind) {
            (Some(k), _) => k.mod_root,
            (None, Kind::BepInEx) => "BepInEx/plugins",
            _ => "",
        };
        let mod_root = links.mod_root.as_deref().unwrap_or(default_root).trim_matches('/').to_string();
        Profile { kind, mod_root, plugins: known.and_then(|k| k.plugins) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_by_steam_id_then_title() {
        assert_eq!(find(Some("489830"), "whatever").map(|g| g.name), Some("Skyrim Special Edition"));
        assert_eq!(find(None, "The Elder Scrolls V: Skyrim Special Edition").map(|g| g.name), Some("Skyrim Special Edition"));
        assert_eq!(find(None, "Fallout 4 GOTY").map(|g| g.name), Some("Fallout 4"));
        assert_eq!(find(None, "Cyberpunk 2077™").map(|g| g.nexus_domain), Some(Some("cyberpunk2077")));
        assert_eq!(find(None, "Lethal Company").map(|g| g.thunderstore), Some(Some("lethal-company")));
        assert!(find(None, "Some Indie Game").is_none());
        assert!(find(None, "").is_none());
    }

    #[test]
    fn titles_normalize() {
        assert_eq!(normalize_title("Mount & Blade II: Bannerlord"), normalize_title("Mount and Blade II Bannerlord"));
        assert_eq!(normalize_title("The Witcher 3: Wild Hunt - Game of the Year Edition"), normalize_title("The Witcher 3: Wild Hunt"));
        assert_eq!(normalize_title("The Game"), "thegame", "only noise words: keep them all");
    }

    #[test]
    fn profile_follows_links() {
        let skyrim = find(Some("489830"), "");
        let p = Profile::of(&GameLinks::default(), skyrim);
        assert_eq!((p.kind, p.mod_root.as_str()), (Kind::Bethesda, "Data"));
        assert!(p.plugins.is_some());
        let ts = GameLinks { thunderstore: Some("some-game".into()), ..Default::default() };
        assert_eq!(Profile::of(&ts, None).mod_root, "BepInEx/plugins");
        let custom = GameLinks { mod_root: Some("/Mods/".into()), ..Default::default() };
        assert_eq!(Profile::of(&custom, None).mod_root, "Mods");
        assert_eq!(Profile::of(&GameLinks::default(), None).kind, Kind::Generic);
    }

    #[test]
    fn table_has_no_duplicates() {
        let mut ids: Vec<&str> = KNOWN.iter().flat_map(|g| g.steam_app_ids.iter().copied()).collect();
        let n = ids.len();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), n);
    }
}
