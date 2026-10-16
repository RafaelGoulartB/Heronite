//! How a game's own actions (`heronite_data::game_actions`) look wherever
//! they are listed: the menu under Play, the game settings, the console.

use heronite_data::game_actions::{Action, ActionKind};
use heronite_i18n::tr;

use crate::icons::Icon;

pub fn icon_of(a: &Action) -> Icon {
    match a.kind {
        _ if a.plays() => Icon::PlayArrow,
        ActionKind::Program => Icon::Build,
        ActionKind::Command => Icon::Terminal,
        ActionKind::Open if is_web(&a.target) => Icon::Language,
        ActionKind::Open => Icon::OpenInNew,
    }
}

pub fn kind_label(kind: ActionKind) -> &'static str {
    match kind {
        ActionKind::Program => tr!("Program"),
        ActionKind::Command => tr!("Command"),
        ActionKind::Open => tr!("Link or file"),
    }
}

/// What it starts, short: the file name, the command or the address.
pub fn target_summary(a: &Action) -> String {
    let t = a.target.trim();
    match a.kind {
        ActionKind::Program => {
            let name = std::path::Path::new(t).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| t.to_string());
            if a.args.trim().is_empty() {
                name
            } else {
                format!("{name} {}", a.args.trim())
            }
        }
        _ => t.to_string(),
    }
}

/// The line under the name in menus: "Counts as playing" or what it starts.
pub fn menu_note(a: &Action) -> String {
    if a.plays() {
        format!("{} \u{b7} {}", tr!("Counts as playing"), target_summary(a))
    } else {
        target_summary(a)
    }
}

/// A name for an action saved without one: the program's file name, the
/// command's first word, the address's host or the file's name.
pub fn default_name(kind: ActionKind, target: &str) -> String {
    let t = target.trim();
    match kind {
        ActionKind::Command => t.split_whitespace().next().map(|w| w.rsplit('/').next().unwrap_or(w).to_string()).unwrap_or_default(),
        ActionKind::Open if is_web(t) => t.split("://").nth(1).and_then(|r| r.split('/').next()).unwrap_or(t).to_string(),
        _ => std::path::Path::new(t).file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| t.to_string()),
    }
}

fn is_web(target: &str) -> bool {
    let t = target.trim().to_ascii_lowercase();
    t.starts_with("http://") || t.starts_with("https://")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_from_the_target() {
        assert_eq!(default_name(ActionKind::Program, "/g/ModOrganizer.exe"), "ModOrganizer");
        assert_eq!(default_name(ActionKind::Command, "./tools/patch.sh --all"), "patch.sh");
        assert_eq!(default_name(ActionKind::Open, "https://pcgamingwiki.com/wiki/Control"), "pcgamingwiki.com");
        assert_eq!(default_name(ActionKind::Open, "/g/Manual.pdf"), "Manual");
    }
}
