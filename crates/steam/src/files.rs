//! Steam's own files, for when the client is closed: the launch options in
//! `userdata/<user>/config/localconfig.vdf`, the compatibility tool in
//! `config/config.vdf` (`CompatToolMapping`), the tools on disk and the
//! game's prefix under `steamapps/compatdata`.
//!
//! Steam writes these files as soon as a setting changes, so reading them is
//! accurate while it runs too. Writing them while it runs is not: it keeps
//! its own copy in memory and overwrites the file.

use std::fs;
use std::path::{Path, PathBuf};

use heronite_launch::steam::{read_manifests, SteamLocator};
use heronite_launch::vdf::Vdf;

use crate::vdf_edit;
use crate::CompatTool;

const LOCAL_ROOT: [&str; 5] = ["UserLocalConfigStore", "Software", "Valve", "Steam", "apps"];
const MAPPING_ROOT: [&str; 5] = ["InstallConfigStore", "Software", "Valve", "Steam", "CompatToolMapping"];
/// Steam's priority for a tool the user picked (`nCompatToolPriority`).
const USER_PRIORITY: &str = "250";
/// Offset between a SteamID64 and the account id `userdata` folders use.
const STEAMID64_BASE: u64 = 76561197960265728;

pub fn config_vdf(root: &Path) -> PathBuf {
    root.join("config").join("config.vdf")
}

pub fn localconfig_vdf(root: &Path, user: &str) -> PathBuf {
    root.join("userdata").join(user).join("config").join("localconfig.vdf")
}

/// The account whose settings apply: the one logged in, else the most
/// recent login, else the only `userdata` folder. `home` holds `.steam`.
pub fn current_user(root: &Path, home: &Path) -> Option<String> {
    let registries = [home.join(".steam/registry.vdf"), home.join(".var/app/com.valvesoftware.Steam/.steam/registry.vdf")];
    let active = registries.iter().find_map(|file| {
        let doc = Vdf::parse(&fs::read_to_string(file).ok()?).ok()?;
        let mut node = &doc;
        for key in ["Registry", "HKCU", "Software", "Valve", "Steam", "ActiveProcess"] {
            node = node.get(key)?;
        }
        node.str_of("ActiveUser").filter(|id| !id.is_empty() && *id != "0").map(str::to_string)
    });
    let has_folder = |id: &String| root.join("userdata").join(id).join("config").is_dir();
    if let Some(id) = active.filter(has_folder) {
        return Some(id);
    }
    if let Some(id) = recent_login(&root.join("config").join("loginusers.vdf")).filter(has_folder) {
        return Some(id);
    }
    let folders = heronite_shortcuts::steam::user_folders(&root.join("userdata")).ok()?;
    match folders.as_slice() {
        [only] => Some(only.clone()),
        _ => None,
    }
}

/// The account id of `MostRecent`, else of the latest `Timestamp`.
fn recent_login(file: &Path) -> Option<String> {
    let doc = Vdf::parse(&fs::read_to_string(file).ok()?).ok()?;
    let users = doc.get("users")?.entries();
    let recent = users
        .iter()
        .find(|(_, u)| u.str_of("MostRecent") == Some("1"))
        .or_else(|| users.iter().max_by_key(|(_, u)| u.str_of("Timestamp").and_then(|t| t.parse::<u64>().ok()).unwrap_or(0)))?;
    recent.0.parse::<u64>().ok().and_then(|id| id.checked_sub(STEAMID64_BASE)).map(|id| id.to_string())
}

pub fn read_launch_options(root: &Path, user: &str, app_id: &str) -> Result<String, String> {
    let file = localconfig_vdf(root, user);
    let text = fs::read_to_string(&file).map_err(|e| format!("{}: {e}", file.display()))?;
    let path = [&LOCAL_ROOT[..], &[app_id, "LaunchOptions"]].concat();
    vdf_edit::get(&text, &path).map(Option::unwrap_or_default).map_err(|e| format!("{}: {e}", file.display()))
}

/// The tool picked for the game, `""` for Steam's default.
pub fn read_compat_tool(root: &Path, app_id: &str) -> String {
    let text = fs::read_to_string(config_vdf(root)).unwrap_or_default();
    let path = [&MAPPING_ROOT[..], &[app_id, "name"]].concat();
    vdf_edit::get(&text, &path).ok().flatten().unwrap_or_default()
}

pub fn write_launch_options(root: &Path, user: &str, app_id: &str, line: &str) -> Result<(), String> {
    let path = [&LOCAL_ROOT[..], &[app_id, "LaunchOptions"]].concat();
    edit_file(&localconfig_vdf(root, user), |text| vdf_edit::set(text, &path, line))
}

/// `""` goes back to Steam's default (it drops the mapping, as Steam does).
pub fn write_compat_tool(root: &Path, app_id: &str, tool: &str) -> Result<(), String> {
    let entry = [&MAPPING_ROOT[..], &[app_id]].concat();
    edit_file(&config_vdf(root), |text| {
        if tool.is_empty() {
            return vdf_edit::remove(text, &entry);
        }
        let mut out = vdf_edit::set(text, &[&entry[..], &["name"]].concat(), tool)?;
        for (key, value) in [("config", ""), ("priority", USER_PRIORITY)] {
            let path = [&entry[..], &[key]].concat();
            if vdf_edit::get(&out, &path)?.is_none() || key == "priority" {
                out = vdf_edit::set(&out, &path, value)?;
            }
        }
        Ok(out)
    })
}

/// Rewrites `file` through `edit`, keeping its permissions and the previous
/// version next to it (`<name>.heronite.bak`).
fn edit_file(file: &Path, edit: impl FnOnce(&str) -> Result<String, vdf_edit::EditError>) -> Result<(), String> {
    let shown = file.display();
    let text = fs::read_to_string(file).map_err(|e| format!("{shown}: {e}"))?;
    let out = edit(&text).map_err(|e| format!("{shown}: {e}"))?;
    if out == text {
        return Ok(());
    }
    let backup = file.with_file_name(format!("{}.heronite.bak", file.file_name().unwrap_or_default().to_string_lossy()));
    if let Err(e) = fs::write(&backup, &text) {
        log::warn!("Could not back up {shown} to {}: {e}", backup.display());
    }
    #[cfg(unix)]
    let mode = {
        use std::os::unix::fs::PermissionsExt;
        fs::metadata(file).ok().map(|m| m.permissions().mode() & 0o7777)
    };
    #[cfg(not(unix))]
    let mode = None;
    heronite_data::temps::write_atomic(file, out.as_bytes(), mode).map_err(|e| format!("{shown}: {e}"))
}

/// Tools Steam would list, read from disk: the installed Valve Proton builds
/// and every `compatibilitytools.d` entry. Steam itself also lists Valve
/// builds it has not downloaded yet; only the running client knows those.
pub fn compat_tools(locator: &SteamLocator) -> Vec<CompatTool> {
    let mut out: Vec<CompatTool> = Vec::new();
    for m in read_manifests(locator) {
        if let Some(name) = valve_tool_name(&m.name) {
            out.push(CompatTool { name, display: m.name.clone(), path: locator.install_dir(&m.app_id) });
        }
    }
    for dir in compat_tool_dirs(locator) {
        let Ok(rd) = fs::read_dir(&dir) else { continue };
        let mut tools: Vec<CompatTool> = rd.flatten().filter_map(|e| read_compat_tool_vdf(&e.path())).flatten().collect();
        tools.sort_by_key(|t| t.display.to_lowercase());
        out.extend(tools);
    }
    let mut seen = std::collections::HashSet::new();
    out.retain(|t| seen.insert(t.name.clone()));
    out
}

/// Where Steam looks for custom tools.
fn compat_tool_dirs(locator: &SteamLocator) -> Vec<PathBuf> {
    let mut dirs = vec![
        locator.steam_path.join("compatibilitytools.d"),
        locator.home.join(".steam/root/compatibilitytools.d"),
        PathBuf::from("/usr/share/steam/compatibilitytools.d"),
        PathBuf::from("/usr/local/share/steam/compatibilitytools.d"),
    ];
    let mut seen = std::collections::HashSet::new();
    dirs.retain(|d| d.is_dir() && seen.insert(fs::canonicalize(d).unwrap_or_else(|_| d.clone())));
    dirs
}

/// The tools a `compatibilitytool.vdf` declares (`compat_tools/<name>/display_name`).
fn read_compat_tool_vdf(dir: &Path) -> Option<Vec<CompatTool>> {
    let doc = Vdf::parse(&fs::read_to_string(dir.join("compatibilitytool.vdf")).ok()?).ok()?;
    let tools = doc.get("compatibilitytools")?.get("compat_tools")?;
    Some(
        tools
            .entries()
            .iter()
            .map(|(name, t)| CompatTool {
                name: name.clone(),
                display: t.str_of("display_name").unwrap_or(name).to_string(),
                // `.`, a path relative to the manifest's folder, or an absolute one.
                path: t.str_of("install_path").map(|p| dir.join(p)),
            })
            .collect(),
    )
}

/// The tools in `compatibilitytools.d` whose manifest appeared (or changed)
/// after `since`: Steam reads the folder only when it starts. The change
/// time, not the modification time: archives keep their files' old dates.
pub fn tools_added_since(locator: &SteamLocator, since: std::time::SystemTime) -> Vec<CompatTool> {
    use std::os::unix::fs::MetadataExt;
    let since = since.duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64);
    let mut out = Vec::new();
    for dir in compat_tool_dirs(locator) {
        let Ok(rd) = fs::read_dir(&dir) else { continue };
        for e in rd.flatten() {
            let changed = fs::metadata(e.path().join("compatibilitytool.vdf")).is_ok_and(|m| m.ctime() > since);
            if changed {
                out.extend(read_compat_tool_vdf(&e.path()).into_iter().flatten());
            }
        }
    }
    out.sort_by_key(|t| t.display.to_lowercase());
    out
}

/// The tool Steam itself maps the game to (its per-game pick, else the
/// global default), as its `compat_log` last recorded it: the user's choice
/// hides it everywhere else.
pub fn automatic_tool(root: &Path, app_id: &str) -> Option<String> {
    let logs = root.join("logs");
    ["compat_log.txt", "compat_log.previous.txt"]
        .iter()
        .find_map(|f| fs::read_to_string(logs.join(f)).ok().and_then(|text| automatic_tool_in(&text, app_id)))
}

/// Last `Mapping AppID <id> to tool "<name>" with priority <p>` (or the
/// `Skip mapping` variant Steam logs when the user's choice wins) below the
/// user's priority.
fn automatic_tool_in(log: &str, app_id: &str) -> Option<String> {
    let needle = format!("apping AppID {app_id} to tool \"");
    log.lines().rev().find_map(|line| {
        let rest = &line[line.find(&needle)? + needle.len()..];
        let (name, rest) = rest.split_once('"')?;
        let priority: u32 = rest.trim_start().strip_prefix("with priority ")?.split(|c: char| !c.is_ascii_digit()).next()?.parse().ok()?;
        (priority < USER_PRIORITY.parse().unwrap_or(250) && !name.is_empty()).then(|| name.to_string())
    })
}

/// Steam's internal name of a Valve Proton app: `Proton 9.0` is `proton_9`,
/// `Proton 6.3` is `proton_63`.
fn valve_tool_name(app_name: &str) -> Option<String> {
    let rest = app_name.strip_prefix("Proton ")?;
    match rest {
        "Experimental" => return Some("proton_experimental".into()),
        "Hotfix" => return Some("proton_hotfix".into()),
        _ => {}
    }
    let version = rest.split_whitespace().next()?;
    let (major, minor) = version.split_once('.')?;
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    if !digits(major) || !digits(minor) {
        return None;
    }
    Some(if minor == "0" { format!("proton_{major}") } else { format!("proton_{major}{minor}") })
}

/// The game's prefix as Steam made it (after its first run through Proton).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Prefix {
    /// `steamapps/compatdata/<appid>` (holds `pfx`).
    pub compatdata: PathBuf,
    /// Folder of the Proton build that last ran the game (has `proton`), from
    /// the prefix's `config_info`.
    pub proton: Option<PathBuf>,
    /// First line of `config_info` (`GE-Proton9-20`, `9.0-4`...).
    pub proton_version: String,
}

/// Steam keeps it in the library the game is installed in; another library
/// may hold a stale one from an older install.
pub fn prefix(locator: &SteamLocator, app_id: &str) -> Option<Prefix> {
    let steamapps_of_game = locator.install_dir(app_id).and_then(|dir| dir.parent()?.parent().map(Path::to_path_buf));
    let candidates: Vec<PathBuf> = steamapps_of_game
        .into_iter()
        .chain(locator.library_roots().into_iter().map(|root| root.join("steamapps")))
        .map(|apps| apps.join("compatdata").join(app_id))
        .filter(|dir| dir.join("pfx").is_dir())
        .collect();
    let compatdata = candidates.iter().find(|dir| dir.join("config_info").is_file()).or(candidates.first())?.clone();
    let info = fs::read_to_string(compatdata.join("config_info")).unwrap_or_default();
    let mut lines = info.lines();
    let proton_version = lines.next().unwrap_or("").trim().to_string();
    let proton = lines.next().and_then(proton_dir_of_fonts).filter(|d| d.join("proton").is_file());
    Some(Prefix { compatdata, proton, proton_version })
}

/// `<proton>/files/share/fonts/` (or `dist/` in old builds) → `<proton>`.
fn proton_dir_of_fonts(line: &str) -> Option<PathBuf> {
    let p = Path::new(line.trim().trim_end_matches('/'));
    let share = p.parent()?;
    (p.file_name()? == "fonts" && share.file_name()? == "share").then(|| share.parent()?.parent().map(Path::to_path_buf))?
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(p: &Path, text: &str) {
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, text).unwrap();
    }

    #[test]
    fn valve_names() {
        assert_eq!(valve_tool_name("Proton 9.0 (Beta)").as_deref(), Some("proton_9"));
        assert_eq!(valve_tool_name("Proton 6.3").as_deref(), Some("proton_63"));
        assert_eq!(valve_tool_name("Proton 5.13").as_deref(), Some("proton_513"));
        assert_eq!(valve_tool_name("Proton Experimental").as_deref(), Some("proton_experimental"));
        assert_eq!(valve_tool_name("Proton EasyAntiCheat Runtime"), None);
        assert_eq!(valve_tool_name("Portal 2"), None);
    }

    #[test]
    fn the_logged_in_user_wins_then_the_latest_login() {
        let dir = tempfile::tempdir().unwrap();
        let (home, root) = (dir.path().join("home"), dir.path().join("Steam"));
        for id in ["111", "314512607"] {
            fs::create_dir_all(root.join("userdata").join(id).join("config")).unwrap();
        }
        assert_eq!(current_user(&root, &home), None, "two users and nothing else to go by");
        write(
            &root.join("config/loginusers.vdf"),
            "\"users\" { \"76561197960265839\" { \"Timestamp\" \"5\" } \"76561198274778335\" { \"Timestamp\" \"9\" } }",
        );
        assert_eq!(current_user(&root, &home).as_deref(), Some("314512607"));
        write(
            &home.join(".steam/registry.vdf"),
            "\"Registry\" { \"HKCU\" { \"Software\" { \"Valve\" { \"Steam\" { \"ActiveProcess\" { \"ActiveUser\" \"111\" } } } } } }",
        );
        assert_eq!(current_user(&root, &home).as_deref(), Some("111"));
    }

    #[test]
    fn writes_and_reads_back_both_settings() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(&localconfig_vdf(root, "7"), "\"UserLocalConfigStore\"\n{\n\t\"Software\"\n\t{\n\t}\n}\n");
        write(&config_vdf(root), "\"InstallConfigStore\"\n{\n\t\"Software\"\n\t{\n\t\t\"Valve\"\n\t\t{\n\t\t}\n\t}\n}\n");
        assert_eq!(read_launch_options(root, "7", "400").unwrap(), "");
        write_launch_options(root, "7", "400", "A=\"1\" %command%").unwrap();
        assert_eq!(read_launch_options(root, "7", "400").unwrap(), "A=\"1\" %command%");
        assert!(localconfig_vdf(root, "7").with_file_name("localconfig.vdf.heronite.bak").exists());
        write_compat_tool(root, "400", "GE-Proton9-20").unwrap();
        assert_eq!(read_compat_tool(root, "400"), "GE-Proton9-20");
        let text = fs::read_to_string(config_vdf(root)).unwrap();
        assert!(text.contains("\t\t\t\t\t\"400\"\n\t\t\t\t\t{\n\t\t\t\t\t\t\"name\"\t\t\"GE-Proton9-20\"\n\t\t\t\t\t\t\"config\"\t\t\"\"\n\t\t\t\t\t\t\"priority\"\t\t\"250\"\n"));
        write_compat_tool(root, "400", "proton_9").unwrap();
        assert_eq!(read_compat_tool(root, "400"), "proton_9");
        write_compat_tool(root, "400", "").unwrap();
        assert_eq!(read_compat_tool(root, "400"), "");
        assert!(!fs::read_to_string(config_vdf(root)).unwrap().contains("\"400\""));
    }

    #[test]
    fn prefix_and_proton_from_config_info() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let steam = dir.path().join("Steam");
        let proton = steam.join("compatibilitytools.d/GE-Proton9-20");
        write(&proton.join("proton"), "");
        let compat = steam.join("steamapps/compatdata/400");
        fs::create_dir_all(compat.join("pfx")).unwrap();
        write(&compat.join("config_info"), &format!("GE-Proton9-20\n{}/files/share/fonts/\n", proton.display()));
        let locator = SteamLocator { home, steam_path: steam.clone(), system_roots: vec![steam.clone()] };
        let p = prefix(&locator, "400").unwrap();
        assert_eq!(p.compatdata, compat);
        assert_eq!(p.proton.as_deref(), Some(proton.as_path()));
        assert_eq!(p.proton_version, "GE-Proton9-20");
        assert!(prefix(&locator, "500").is_none());
    }

    #[test]
    fn custom_tools_come_from_their_vdf() {
        let dir = tempfile::tempdir().unwrap();
        let steam = dir.path().join("Steam");
        write(
            &steam.join("compatibilitytools.d/ge/compatibilitytool.vdf"),
            "\"compatibilitytools\" { \"compat_tools\" { \"GE-Proton9-20\" { \"install_path\" \".\" \"display_name\" \"GE-Proton9-20 (GE)\" } } }",
        );
        write(
            &steam.join("steamapps/appmanifest_2805730.acf"),
            "\"AppState\" { \"appid\" \"2805730\" \"name\" \"Proton 9.0\" \"StateFlags\" \"4\" }",
        );
        let locator = SteamLocator { home: dir.path().join("home"), steam_path: steam.clone(), system_roots: vec![steam.clone()] };
        let tools = compat_tools(&locator);
        assert_eq!(
            tools,
            [
                CompatTool { name: "proton_9".into(), display: "Proton 9.0".into(), path: None },
                CompatTool {
                    name: "GE-Proton9-20".into(),
                    display: "GE-Proton9-20 (GE)".into(),
                    path: Some(steam.join("compatibilitytools.d/ge/."))
                },
            ]
        );
    }

    #[test]
    fn the_automatic_tool_comes_from_the_compat_log() {
        let log = "[x] Mapping AppID 400 to tool \"proton_9\" with priority 75\n\
            [x] Mapping AppID 4000 to tool \"other\" with priority 90\n\
            [x] Skip mapping AppID 400 to tool \"proton_experimental\" with priority 90: mapping to tool \"GE\" with priority 250 already exists.\n\
            [x] Mapping AppID 400 to tool \"GE\" with priority 250\n";
        assert_eq!(automatic_tool_in(log, "400").as_deref(), Some("proton_experimental"));
        assert_eq!(automatic_tool_in(log, "500"), None);
    }
}
