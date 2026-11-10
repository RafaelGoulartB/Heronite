//! Whether a graphics mod is risky for a game. Three sources, strongest
//! first:
//!
//! 1. `Banned=1` in ReShade's `Compatibility.ini`: the game bans or blocks
//!    ReShade. Installing is refused. ([`Risk::banned`] comes only from
//!    ReShade's list; the other sources apply to any mod.)
//! 2. AreWeAntiCheatYet (`heronite_data::anticheat`): the anti-cheats the
//!    game is known to use. A warning the user must accept.
//! 3. Files of known anti-cheats in the game's folders, for games missing
//!    from that list. Also a warning.
//!
//! The `eacRuntime` / `battlEyeRuntime` settings are not a source: they are
//! on by default for every game.

use std::path::{Path, PathBuf};

use heronite_data::anticheat::AntiCheatInfo;

/// Where a warning comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    /// areweanticheatyet.com.
    List,
    /// A file or folder of the anti-cheat.
    File(PathBuf),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    /// The anti-cheat's name as AreWeAntiCheatYet writes it.
    pub name: String,
    pub source: Source,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Risk {
    pub banned: bool,
    /// One per anti-cheat, list first.
    pub findings: Vec<Finding>,
}

/// What a known anti-cheat means for a mod.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Policy {
    /// Installed once the user accepts the risk (ReShade's standard build).
    Warn,
    /// Not offered while any anti-cheat is known (mods anti-cheats block
    /// more often, such as ReShade's add-on build).
    Block,
}

impl Risk {
    /// Whether `policy` refuses the mod: always for a game that bans
    /// ReShade, and for any known anti-cheat under [`Policy::Block`].
    pub fn blocks(&self, policy: Policy) -> bool {
        self.banned || (policy == Policy::Block && !self.findings.is_empty())
    }

    pub fn names(&self) -> Vec<String> {
        self.findings.iter().map(|f| f.name.clone()).collect()
    }

    /// Anti-cheats the user has not accepted yet (`acked`: those accepted).
    pub fn unacknowledged(&self, acked: &[String]) -> Vec<String> {
        self.names().into_iter().filter(|n| !acked.iter().any(|a| a.eq_ignore_ascii_case(n))).collect()
    }
}

const EAC: &str = "Easy Anti-Cheat";
const BATTLEYE: &str = "BattlEye";

/// Names found in game folders: `*` matches any text, case is ignored.
const MARKERS: [(&str, &str); 14] = [
    ("EasyAntiCheat", EAC),
    ("EasyAntiCheat_EOS", EAC),
    ("start_protected_game.exe", EAC),
    ("EasyAntiCheat_x64.dll", EAC),
    ("EasyAntiCheat_x86.dll", EAC),
    ("BattlEye", BATTLEYE),
    ("BEService*.exe", BATTLEYE),
    ("BEClient*.dll", BATTLEYE),
    ("GameGuard", "nProtect GameGuard"),
    ("XignCode", "XIGNCODE3"),
    ("mhyprot*.sys", "mhyprot"),
    ("PnkBstr*.exe", "PunkBuster"),
    ("ACE-*", "Anti-Cheat Expert"),
    ("vgk.sys", "Vanguard"),
];

fn matches(pattern: &str, name: &str) -> bool {
    let (p, n) = (pattern.to_ascii_lowercase(), name.to_ascii_lowercase());
    match p.split_once('*') {
        Some((pre, post)) => n.len() >= pre.len() + post.len() && n.starts_with(pre) && n.ends_with(post),
        None => p == n,
    }
}

/// Anti-cheat files and folders directly inside `dirs` (no deeper: these
/// sit next to the game's executable or at the top of its install).
pub fn files_found(dirs: &[&Path]) -> Vec<Finding> {
    let mut out: Vec<Finding> = Vec::new();
    for dir in dirs {
        let Ok(entries) = std::fs::read_dir(dir) else { continue };
        let mut names: Vec<(String, PathBuf)> =
            entries.flatten().map(|e| (e.file_name().to_string_lossy().into_owned(), e.path())).collect();
        names.sort();
        for (name, path) in names {
            if let Some((_, ac)) = MARKERS.iter().find(|(p, _)| matches(p, &name)) {
                if !out.iter().any(|f| f.name == *ac) {
                    out.push(Finding { name: ac.to_string(), source: Source::File(path) });
                }
            }
        }
    }
    out
}

pub fn assess(banned: bool, listed: Option<&AntiCheatInfo>, dirs: &[&Path]) -> Risk {
    let mut findings: Vec<Finding> = listed
        .map(|l| l.anticheats.iter().filter(|a| !a.trim().is_empty()).map(|a| Finding { name: a.clone(), source: Source::List }).collect())
        .unwrap_or_default();
    for f in files_found(dirs) {
        if !findings.iter().any(|x| x.name.eq_ignore_ascii_case(&f.name)) {
            findings.push(f);
        }
    }
    Risk { banned, findings }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn banned_blocks_and_the_list_warns() {
        let listed = AntiCheatInfo { status: "Denied".into(), anticheats: vec!["Easy Anti-Cheat".into(), "BattlEye".into()] };
        let r = assess(true, Some(&listed), &[]);
        assert!(r.banned);
        assert_eq!(r.names(), ["Easy Anti-Cheat", "BattlEye"]);
        assert!(assess(false, None, &[]).findings.is_empty());
        let empty = AntiCheatInfo { status: "Supported".into(), anticheats: Vec::new() };
        assert!(assess(false, Some(&empty), &[]).findings.is_empty());
    }

    #[test]
    fn files_of_anti_cheats_in_the_folder_warn() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("EasyAntiCheat")).unwrap();
        std::fs::write(dir.path().join("BEService_x64.exe"), b"").unwrap();
        std::fs::write(dir.path().join("Game.exe"), b"").unwrap();
        let r = assess(false, None, &[dir.path()]);
        assert_eq!(r.names(), ["BattlEye", "Easy Anti-Cheat"]);
        assert_eq!(r.findings[1].source, Source::File(dir.path().join("EasyAntiCheat")));
        let listed = AntiCheatInfo { status: "Broken".into(), anticheats: vec!["Easy Anti-Cheat".into()] };
        let both = assess(false, Some(&listed), &[dir.path()]);
        assert_eq!(both.names(), ["Easy Anti-Cheat", "BattlEye"]);
        assert_eq!(both.findings[0].source, Source::List);
    }

    #[test]
    fn a_new_anti_cheat_asks_again() {
        let r = Risk { banned: false, findings: vec![Finding { name: "Easy Anti-Cheat".into(), source: Source::List }] };
        assert_eq!(r.unacknowledged(&[]), ["Easy Anti-Cheat"]);
        assert!(r.unacknowledged(&["easy anti-cheat".into()]).is_empty());
        let more = Risk { findings: [r.findings.clone(), vec![Finding { name: "BattlEye".into(), source: Source::List }]].concat(), ..r };
        assert_eq!(more.unacknowledged(&["Easy Anti-Cheat".into()]), ["BattlEye"]);
    }

    #[test]
    fn block_refuses_any_anti_cheat_and_warn_only_a_ban() {
        let none = Risk::default();
        let listed = Risk { banned: false, findings: vec![Finding { name: "BattlEye".into(), source: Source::List }] };
        let banned = Risk { banned: true, findings: Vec::new() };
        assert!(!none.blocks(Policy::Block) && !none.blocks(Policy::Warn));
        assert!(listed.blocks(Policy::Block) && !listed.blocks(Policy::Warn));
        assert!(banned.blocks(Policy::Block) && banned.blocks(Policy::Warn));
    }

    #[test]
    fn patterns_match_without_case() {
        assert!(matches("BEClient*.dll", "beclient_x64.DLL"));
        assert!(matches("ACE-*", "ACE-Base64.dll"));
        assert!(!matches("vgk.sys", "vgk.sys.bak"));
    }
}
