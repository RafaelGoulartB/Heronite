//! `find --api` / `backup --api` command lines and their JSON output.

use heronite_launch::probe::Probe;
use serde_json::Value;

use crate::pathutil::join;

/// `ludusavi find --api ...` lookups tried in order (Steam id, GOG id, title).
pub fn find_commands(title: &str, steam_app_id: Option<&str>, runner_name: &str, store_game_id: Option<&str>) -> Vec<Vec<String>> {
    let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
    let mut out = Vec::new();
    if let Some(id) = steam_app_id.filter(|i| !i.is_empty()) {
        out.push(s(&["find", "--api", "--steam-id", id]));
    }
    if runner_name == "gog" {
        if let Some(id) = store_game_id.filter(|i| !i.is_empty() && i.chars().all(|c| c.is_ascii_digit())) {
            out.push(s(&["find", "--api", "--gog-id", id]));
        }
    }
    out.push(s(&["find", "--api", title]));
    out
}

/// Inputs of [`backup_command`].
#[derive(Debug, Clone)]
pub struct BackupCommand<'a> {
    /// Use Ludusavi's own config (no `--path`/`--format` overrides).
    pub use_installed: bool,
    pub configured_path: &'a str,
    pub format: &'a str,
    pub compression: &'a str,
    pub wine_prefix: Option<&'a str>,
    pub game_name: &'a str,
}

/// `ludusavi backup --force --api --no-cloud-sync [overrides] <game>`.
pub fn backup_command(c: &BackupCommand) -> Vec<String> {
    let mut cmd: Vec<String> = ["backup", "--force", "--api", "--no-cloud-sync"].iter().map(|s| s.to_string()).collect();
    if !c.use_installed {
        cmd.extend(["--path".into(), c.configured_path.into()]);
        cmd.extend(["--format".into(), c.format.into()]);
        if c.format == "zip" {
            cmd.extend(["--compression".into(), c.compression.into()]);
        }
    }
    if let Some(p) = c.wine_prefix {
        cmd.extend(["--wine-prefix".into(), p.into()]);
    }
    cmd.push(c.game_name.into());
    cmd
}

/// `parseJson`: `{}` for empty or invalid output.
pub fn parse_json(text: &str) -> Value {
    let t = text.trim();
    if t.is_empty() {
        return Value::Object(Default::default());
    }
    serde_json::from_str(t).unwrap_or_else(|_| Value::Object(Default::default()))
}

/// `bestFindMatch`: the game with the highest `score` of a `find --api` reply
/// (the first one on ties).
pub fn best_find_match(payload: &Value) -> Option<String> {
    let games = payload.get("games")?.as_object()?;
    let mut best: Option<(&String, f64)> = None;
    for (name, info) in games {
        let score = info.get("score").and_then(Value::as_f64).unwrap_or(0.0);
        if best.is_none_or(|(_, b)| score > b) {
            best = Some((name, score));
        }
    }
    best.map(|(n, _)| n.clone())
}

/// `overall.totalGames` of a backup reply.
pub fn total_games(payload: &Value) -> u64 {
    payload.get("overall").and_then(|o| o.get("totalGames")).and_then(|v| v.as_u64().or_else(|| v.as_f64().map(|f| f as u64))).unwrap_or(0)
}

/// `gameNamesFromPayload`.
pub fn game_names(payload: &Value) -> Vec<String> {
    payload.get("games").and_then(Value::as_object).map(|g| g.keys().cloned().collect()).unwrap_or_default()
}

/// `winePrefixForLudusavi`: the prefix itself or its `pfx` child, whichever
/// has `drive_c`.
pub fn wine_prefix_for_ludusavi(prefix: &str, probe: &dyn Probe) -> Option<String> {
    if prefix.is_empty() {
        return None;
    }
    if probe.exists(&join(prefix, "drive_c")) {
        return Some(prefix.to_string());
    }
    let pfx = join(prefix, "pfx");
    probe.exists(&join(&pfx, "drive_c")).then_some(pfx)
}

#[cfg(test)]
mod tests {
    use super::*;
    use heronite_launch::probe::FakeProbe;

    #[test]
    fn command_construction() {
        let c = backup_command(&BackupCommand {
            use_installed: true,
            configured_path: "",
            format: "zip",
            compression: "deflate",
            wine_prefix: None,
            game_name: "Tony Hawk's Underground",
        });
        assert_eq!(c, ["backup", "--force", "--api", "--no-cloud-sync", "Tony Hawk's Underground"]);
        let c = backup_command(&BackupCommand {
            use_installed: false,
            configured_path: "/b",
            format: "zip",
            compression: "zstd",
            wine_prefix: Some("/pfx"),
            game_name: "G",
        });
        assert_eq!(
            c,
            [
                "backup",
                "--force",
                "--api",
                "--no-cloud-sync",
                "--path",
                "/b",
                "--format",
                "zip",
                "--compression",
                "zstd",
                "--wine-prefix",
                "/pfx",
                "G"
            ]
        );
        let c = backup_command(&BackupCommand {
            use_installed: false,
            configured_path: "/b",
            format: "simple",
            compression: "zstd",
            wine_prefix: None,
            game_name: "G",
        });
        assert!(!c.contains(&"--compression".to_string()));
        assert_eq!(
            find_commands("Title", Some("42"), "sideload", None),
            vec![vec!["find", "--api", "--steam-id", "42"], vec!["find", "--api", "Title"]]
        );
        assert_eq!(
            find_commands("T", None, "gog", Some("123")),
            vec![vec!["find", "--api", "--gog-id", "123"], vec!["find", "--api", "T"]]
        );
        assert_eq!(find_commands("T", None, "gog", Some("abc")).len(), 1);
    }

    #[test]
    fn parses_ludusavi_json() {
        let find = parse_json(r#"{"games":{"Low":{"score":0.2},"High":{"score":0.9},"Mid":{"score":0.5}}}"#);
        assert_eq!(best_find_match(&find).as_deref(), Some("High"));
        assert_eq!(best_find_match(&parse_json("")), None);
        assert_eq!(best_find_match(&parse_json("not json")), None);
        let backup = parse_json(r#"{"overall":{"totalGames":2},"games":{"A":{},"B":{}}}"#);
        assert_eq!(total_games(&backup), 2);
        assert_eq!(game_names(&backup), ["A", "B"]);
        assert_eq!(total_games(&parse_json("{}")), 0);
    }

    #[test]
    fn wine_prefix_detection() {
        let probe = FakeProbe::new("/h").with_path("/p1/drive_c").with_path("/p2/pfx/drive_c");
        assert_eq!(wine_prefix_for_ludusavi("/p1", &probe).as_deref(), Some("/p1"));
        assert_eq!(wine_prefix_for_ludusavi("/p2", &probe).as_deref(), Some("/p2/pfx"));
        assert_eq!(wine_prefix_for_ludusavi("/p3", &probe), None);
        assert_eq!(wine_prefix_for_ludusavi("", &probe), None);
    }
}
