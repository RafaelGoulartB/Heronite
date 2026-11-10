//! What to expect of mods for a game (`docs/MODS.md`, "What to expect"):
//! the overlay only reaches the game's own folder, so games that keep mods
//! elsewhere, or that need a tool of their own, are told apart from the ones
//! whose mods are files in the folder. The manager shows the level as a
//! label and the reason as a sentence.
//!
//! The levels are what the game's way of loading mods lets us expect, not the
//! result of a test of every mod.

use heronite_i18n::tr;

use crate::games::{Kind, KnownGame};

/// What stands between a known game and its mods ([`KnownGame::fit`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fit {
    /// Mods are files in the game's folder: what the overlay does.
    Folder,
    /// Works once the user does something the manager does not do.
    Steps(Step),
    /// Most mods cannot be applied.
    Blocked(Block),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    /// Bethesda: the script extender's loader, and plugins ordered by hand.
    ScriptExtender,
    /// Plugins are turned on in a list of the game's that we do not write.
    PluginList,
    /// The Souls games: ModEngine 2's launcher with arguments, offline.
    ModEngine,
    /// Cyberpunk 2077: REDmod mods need the game started with REDmod on.
    RedMod,
    /// The Witcher 3: script conflicts are merged by Script Merger.
    ScriptMerger,
    /// RE Engine: REFramework first; Fluffy Mod Manager packs may not load.
    ReFramework,
    /// Stardew Valley: SMAPI comes as an installer of its own.
    Smapi,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Block {
    /// Most mods live in Documents or AppData, not in the game's folder.
    OutsideFolder,
    /// Mods are installed by a tool that rewrites the game's files.
    OwnTool,
}

/// How much of a game's mods the manager is expected to handle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    Expected,
    Steps,
    /// A game we have no rules for.
    Untested,
    Unsupported,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reason {
    Folder,
    /// BepInEx, set up by its Thunderstore pack.
    Loader,
    Step(Step),
    Block(Block),
    /// A Linux build of a game whose mods need the Windows mod loader.
    NativeLoader,
    Unknown,
    /// Not in the table, but linked to a Thunderstore community.
    UnknownThunderstore,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Support {
    pub level: Level,
    pub reason: Reason,
}

impl Support {
    /// `kind` is the game's [`crate::games::Profile::kind`] (a game linked to
    /// Thunderstore is a BepInEx game even when the table lacks it); `native`
    /// is a Linux build.
    pub fn of(known: Option<&KnownGame>, kind: Kind, native: bool) -> Support {
        let (level, reason) = match known.map(|k| k.fit) {
            _ if native && kind == Kind::BepInEx => (Level::Unsupported, Reason::NativeLoader),
            Some(Fit::Blocked(b)) => (Level::Unsupported, Reason::Block(b)),
            Some(Fit::Steps(s)) => (Level::Steps, Reason::Step(s)),
            Some(Fit::Folder) if kind == Kind::BepInEx => (Level::Expected, Reason::Loader),
            Some(Fit::Folder) => (Level::Expected, Reason::Folder),
            None if kind == Kind::BepInEx => (Level::Untested, Reason::UnknownThunderstore),
            None => (Level::Untested, Reason::Unknown),
        };
        Support { level, reason }
    }

    /// A few words for a label.
    pub fn label(&self) -> &'static str {
        match self.level {
            Level::Expected => tr!("Should work"),
            Level::Steps => tr!("Needs extra steps"),
            Level::Untested => tr!("Untested game"),
            Level::Unsupported => tr!("Most mods won't work"),
        }
    }

    /// One or two sentences on why, and what to do about it.
    pub fn explain(&self) -> &'static str {
        match self.reason {
            Reason::Folder => {
                tr!("Mods for this game are files in its folder, which is what Heronite handles. Not every mod has been tried.")
            }
            Reason::Loader => {
                tr!("Mods for this game load through BepInEx, which Heronite installs and turns on with them. Not every mod has been tried.")
            }
            Reason::Step(Step::ScriptExtender) => tr!(
                "Simple mods work as they are. Mods that need the script extender only load when the game starts through its loader, and Heronite does not sort plugins: order them yourself in Plugins."
            ),
            Reason::Step(Step::PluginList) => tr!(
                "Files are put in place, but a mod with a plugin (.esp, .esm) also has to be turned on in the game's own plugin list. Heronite does not do that for this game."
            ),
            Reason::Step(Step::ModEngine) => {
                tr!("Mods load through ModEngine 2: add it as a mod, start the game through its launcher with its arguments, and play offline.")
            }
            Reason::Step(Step::RedMod) => tr!(
                "Archive mods and the frameworks (Cyber Engine Tweaks, RED4ext) work as they are. REDmod mods also need the game started with REDmod on, which Heronite does not set."
            ),
            Reason::Step(Step::ScriptMerger) => tr!(
                "Mods work from the Mods folder. When two mods change the same script, the game needs them merged with Script Merger, which Heronite does not run."
            ),
            Reason::Step(Step::ReFramework) => {
                tr!("Most mods need REFramework: add it as a mod first. Mods packed for Fluffy Mod Manager may not load.")
            }
            Reason::Step(Step::Smapi) => {
                tr!("Mods need SMAPI, which has its own installer: run it on the game first. Mods added here then go into Mods.")
            }
            Reason::Block(Block::OutsideFolder) => tr!(
                "Most mods for this game live outside its folder (Documents or AppData), where Heronite cannot put them. Only mods that go into the game's folder work."
            ),
            Reason::Block(Block::OwnTool) => {
                tr!("Mods for this game are installed by a tool of its own that rewrites the game's files, which Heronite does not do.")
            }
            Reason::NativeLoader => {
                tr!("This is the game's Linux build. Heronite sets the mod loader up for the Windows build only, so these mods will not load.")
            }
            Reason::Unknown => tr!(
                "Heronite has no rules for this game, so a mod's files go into the game's folder as they come. If the mod's page names another folder, set it in Game \u{2192} Mods go into."
            ),
            Reason::UnknownThunderstore => tr!(
                "Heronite has no rules for this game. Its Thunderstore mods are installed the way BepInEx expects, which suits most games of that site."
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::games::{find, KNOWN};

    fn of(steam_id: &str, native: bool) -> Support {
        let known = find(Some(steam_id), "");
        Support::of(known, known.map_or(Kind::Generic, |k| k.kind), native)
    }

    #[test]
    fn games_are_told_apart_by_how_they_load_mods() {
        assert_eq!(of("870780", false), Support { level: Level::Expected, reason: Reason::Folder }, "Control");
        assert_eq!(of("1966720", false), Support { level: Level::Expected, reason: Reason::Loader }, "Lethal Company");
        assert_eq!(of("489830", false).reason, Reason::Step(Step::ScriptExtender), "Skyrim SE");
        assert_eq!(of("22320", false).reason, Reason::Step(Step::PluginList), "Morrowind");
        assert_eq!(of("1245620", false).reason, Reason::Step(Step::ModEngine), "Elden Ring");
        assert_eq!(of("1091500", false).reason, Reason::Step(Step::RedMod), "Cyberpunk 2077");
        assert_eq!(of("292030", false).reason, Reason::Step(Step::ScriptMerger), "The Witcher 3");
        assert_eq!(of("2050650", false).reason, Reason::Step(Step::ReFramework), "Resident Evil 4");
        assert_eq!(of("413150", true).reason, Reason::Step(Step::Smapi), "Stardew Valley, Linux build");
        assert_eq!(of("1086940", false), Support { level: Level::Unsupported, reason: Reason::Block(Block::OutsideFolder) }, "BG3");
        assert_eq!(of("1328670", false).reason, Reason::Block(Block::OwnTool), "Mass Effect Legendary Edition");
    }

    #[test]
    fn a_linux_build_of_a_bepinex_game_does_not_load_them() {
        assert_eq!(of("892970", false).level, Level::Expected, "Valheim through Proton");
        assert_eq!(of("892970", true), Support { level: Level::Unsupported, reason: Reason::NativeLoader });
        // A folder of mods the game reads by itself works on a Linux build too.
        assert_eq!(of("294100", true).level, Level::Expected, "RimWorld");
    }

    #[test]
    fn a_game_without_rules_is_untested() {
        assert_eq!(Support::of(None, Kind::Generic, false), Support { level: Level::Untested, reason: Reason::Unknown });
        assert_eq!(Support::of(None, Kind::Generic, true).level, Level::Untested);
        assert_eq!(Support::of(None, Kind::BepInEx, false).reason, Reason::UnknownThunderstore);
        assert_eq!(Support::of(None, Kind::BepInEx, true).level, Level::Unsupported);
    }

    #[test]
    fn every_level_has_its_words() {
        let mut labels = std::collections::HashSet::new();
        let mut sentences = std::collections::HashSet::new();
        for g in KNOWN {
            for native in [false, true] {
                let s = Support::of(Some(g), g.kind, native);
                assert!(!s.label().is_empty() && s.explain().ends_with('.'), "{}", g.name);
                labels.insert(s.label());
                sentences.insert(s.explain());
            }
        }
        assert_eq!(labels.len(), 3, "the table has no untested game");
        assert!(sentences.len() >= 11, "{}", sentences.len());
    }
}
