//! The Setup card's words: one row per group of a game's settings, saying
//! what this game sets ("Runs with GE-Proton 9-20", "Graphics HDR · FSR 4"),
//! or "Default" when it changed nothing there. The game page and the console
//! game page draw the rows; [`read`] builds them off the UI thread.

use heronite_data::collection::Game;
use heronite_data::{game_config, DataPaths};
use heronite_i18n::{tr, trn};
use heronite_launch::fps_limit;
use serde_json::{Map, Value};

use super::changes::{changed_groups, Group};
use super::logic::{self, GameFacts, RunKind, Section};

/// How many settings a row names before "+N".
const NAMED: usize = 2;
/// Game arguments longer than this are cut (with "…").
const ARGS_CHARS: usize = 18;

/// One row of the Setup card.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SetupRow {
    /// The group the row opens.
    pub section: Section,
    pub label: &'static str,
    pub value: String,
    /// The game sets something of this group (else the value is "Default",
    /// drawn dim).
    pub changed: bool,
}

/// The rows of a game with these groups. `runs_with`: the Wine / Proton
/// build (or Steam's tool) Compatibility names first; `actions`: the user's
/// own actions of the game, which Start counts.
pub fn summarize(
    sections: &[Section],
    settings: &Map<String, Value>,
    base: &Map<String, Value>,
    runs_with: Option<&str>,
    actions: usize,
) -> Vec<SetupRow> {
    sections
        .iter()
        .map(|&section| {
            let groups = changed_groups(section, settings, base);
            let mut label = section.label();
            let mut parts: Vec<String> = Vec::new();
            let named = section == Section::Compatibility && runs_with.is_some();
            if named {
                label = tr!("Runs with");
                parts.push(runs_with.filter(|n| !n.trim().is_empty()).map_or_else(|| tr!("Default").to_string(), str::to_string));
            }
            for g in &groups {
                if named && g.id == "wine-version" {
                    continue;
                }
                let w = word(g, settings);
                if !parts.contains(&w) {
                    parts.push(w);
                }
            }
            let mut changed = !groups.is_empty();
            if section == Section::Start && actions > 0 {
                parts.push(trn!(actions, "{n} action", "{n} actions").to_string());
                changed = true;
            }
            let value = if parts.is_empty() { tr!("Default").to_string() } else { join(&parts) };
            SetupRow { section, label, value, changed }
        })
        .collect()
}

/// A changed setting in a few words: the arguments themselves, the FPS
/// limit as a number, else the row's label.
fn word(g: &Group, settings: &Map<String, Value>) -> String {
    match g.id {
        "launcherArgs" => match settings.get("launcherArgs").and_then(Value::as_str).map(str::trim) {
            Some(args) if !args.is_empty() => cut(args, ARGS_CHARS),
            _ => heronite_i18n::tr(g.label).to_string(),
        },
        "fps" => match settings.get(fps_limit::KEY_LIMIT).and_then(number).filter(|n| *n > 0) {
            Some(fps) => tr!("{fps} FPS", fps).to_string(),
            None => heronite_i18n::tr(g.label).to_string(),
        },
        _ => heronite_i18n::tr(g.label).to_string(),
    }
}

fn number(v: &Value) -> Option<u64> {
    match v {
        Value::Number(n) => n.as_u64(),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}

fn cut(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let mut s: String = text.chars().take(max - 1).collect();
    s.push('\u{2026}');
    s
}

/// The first few parts, then how many more.
fn join(parts: &[String]) -> String {
    let mut s = parts.iter().take(NAMED).cloned().collect::<Vec<_>>().join(" \u{b7} ");
    if parts.len() > NAMED {
        s.push_str(&format!(" \u{b7} +{}", parts.len() - NAMED));
    }
    s
}

/// The rows a short window shows: only the changed ones, unless "Show all"
/// is on. The second value: some rows are hidden ("Show all" is offered).
pub fn folded(rows: &[SetupRow], short: bool, show_all: bool) -> (Vec<&SetupRow>, bool) {
    if !short || show_all {
        return (rows.iter().collect(), false);
    }
    let shown: Vec<&SetupRow> = rows.iter().filter(|r| r.changed).collect();
    let hidden = shown.len() < rows.len();
    (shown, hidden)
}

/// The Setup rows of an installed game (none otherwise): reads its settings
/// (a Steam game's from Steam) and its actions. Blocking: call it off the
/// UI thread.
pub fn read(paths: &DataPaths, game: &Game) -> Vec<SetupRow> {
    if !game.installed {
        return Vec::new();
    }
    let facts = GameFacts::of(game);
    let sections = facts.sections();
    if sections.is_empty() {
        return Vec::new();
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| "/".into());
    let actions = heronite_data::game_actions::load(paths).remove(&game.key).map_or(0, |a| a.len());
    let steam_app = facts.steam_app_id.as_deref().filter(|_| facts.run_kind == RunKind::Steam);
    let (settings, base, runs_with) = match steam_app {
        Some(app_id) => {
            let (settings, tool) = match crate::services::steam_library::steam(paths).read(app_id) {
                Ok(snap) => {
                    let tool = snap
                        .tools
                        .iter()
                        .find(|t| t.name == snap.compat_tool)
                        .map(|t| t.display.clone())
                        .filter(|_| !snap.compat_tool.is_empty());
                    (heronite_steam::options::parse(&snap.launch_options).settings, tool)
                }
                Err(_) => (heronite_steam::options::defaults(), None),
            };
            (settings, heronite_steam::options::defaults(), tool)
        }
        None => {
            let settings = game_config::load_game(paths, &game.app_name, &home);
            let base = game_config::default_game_settings(&game_config::load_global(paths, &home), false, &home);
            let wine = (facts.run_kind == RunKind::Wine).then(|| logic::wine_of(&settings).name);
            (settings, base, wine)
        }
    };
    let runs_with = match facts.run_kind {
        RunKind::Steam => Some(runs_with.unwrap_or_default()),
        RunKind::Wine => runs_with,
        _ => None,
    };
    summarize(&sections, &settings, &base, runs_with.as_deref(), actions)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn map(v: Value) -> Map<String, Value> {
        v.as_object().cloned().unwrap()
    }

    const ALL: [Section; 5] = [Section::Start, Section::Compatibility, Section::Graphics, Section::Performance, Section::Saves];

    #[test]
    fn nothing_changed_reads_default_but_names_the_build() {
        let rows = summarize(&ALL, &Map::new(), &Map::new(), Some("GE-Proton9-20"), 0);
        assert_eq!(rows.len(), 5);
        assert_eq!(rows[1].label, "Runs with");
        assert_eq!(rows[1].value, "GE-Proton9-20");
        assert!(!rows[1].changed);
        assert!(rows.iter().filter(|r| r.section != Section::Compatibility).all(|r| r.value == "Default" && !r.changed));
    }

    #[test]
    fn rows_name_what_the_game_sets() {
        let settings = map(json!({
            "launcherArgs": "-dx11",
            "enableHDR": true,
            "heroniteGpuFsr4Upgrade": true,
            "heroniteFpsLimit": 60,
            "showMangohud": true,
            "useGameMode": true,
        }));
        let rows = summarize(&ALL, &settings, &Map::new(), None, 2);
        let value = |s: Section| rows.iter().find(|r| r.section == s).unwrap().value.clone();
        assert_eq!(value(Section::Start), "-dx11 \u{b7} 2 actions");
        // Without a build, Compatibility is named like its group.
        assert_eq!(rows[1].label, "Compatibility");
        assert_eq!(value(Section::Performance), "MangoHud \u{b7} 60 FPS \u{b7} +1");
        assert!(rows.iter().find(|r| r.section == Section::Graphics).unwrap().changed);
    }

    #[test]
    fn long_arguments_are_cut() {
        let settings = map(json!({ "launcherArgs": "-windowed -nointro -skipmovies" }));
        let rows = summarize(&[Section::Start], &settings, &Map::new(), None, 0);
        assert_eq!(rows[0].value.chars().count(), ARGS_CHARS);
        assert!(rows[0].value.ends_with('\u{2026}'));
    }

    #[test]
    fn short_windows_keep_only_the_changed_rows() {
        let row = |section, changed| SetupRow { section, label: "", value: String::new(), changed };
        let rows = [row(Section::Start, false), row(Section::Graphics, true), row(Section::Saves, false)];
        assert_eq!(folded(&rows, false, false), (rows.iter().collect(), false));
        let (shown, hidden) = folded(&rows, true, false);
        assert_eq!(shown.iter().map(|r| r.section).collect::<Vec<_>>(), [Section::Graphics]);
        assert!(hidden);
        assert_eq!(folded(&rows, true, true), (rows.iter().collect(), false));
        // Nothing hidden when every row is changed.
        let all = [row(Section::Start, true)];
        assert!(!folded(&all, true, false).1);
    }
}
