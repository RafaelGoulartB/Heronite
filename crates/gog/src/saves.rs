//! Cloud saves of GOG games through `gogdl save-sync`.
//!
//! GOG describes where a game keeps its saves with `<?NAME?>` variables
//! (Windows folders). The locations are resolved once to folders inside the
//! game's Wine prefix and kept in the game's settings (`gogSaves`), so the
//! user can see and edit them. Each location is synced on its own; the time
//! of the last sync is remembered in `gog_store/saveTimestamps.json`.

use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Context, Result};
use heronite_data::{game_config, json_store, DataPaths};
use heronite_i18n::tr;
use heronite_launch::{PlanContext, Probe, SystemProbe};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::cmd::{manifest_file, Gogdl};
use crate::installed::{self, Installed};

/// Where GOG publishes per-game client settings (no authentication).
const REMOTE_CONFIG: &str = "https://remote-config.gog.com/components/galaxy_client/clients";
const COMPONENT_VERSION: &str = "2.0.45";
/// Name of the location GOG uses when a game lists none.
const DEFAULT_NAME: &str = "__default";
const DEFAULT_LOCATION: &str = "<?APPLICATION_DATA_LOCAL?>/GOG.com/Galaxy/Applications/{client}/Storage/Shared/Files";
/// Key in the game's settings with the resolved [`SaveLocation`]s.
pub const SETTINGS_KEY: &str = "gogSaves";

/// One synced folder: `name` is the cloud-side folder name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SaveLocation {
    pub name: String,
    /// Linux path inside the game's Wine prefix.
    pub location: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SyncMode {
    /// Upload and download as needed.
    Both,
    ForceDownload,
    ForceUpload,
}

impl SyncMode {
    fn flag(self) -> Option<&'static str> {
        match self {
            SyncMode::Both => None,
            SyncMode::ForceDownload => Some("--force-download"),
            SyncMode::ForceUpload => Some("--force-upload"),
        }
    }
}

/// The game's id for GOG's client services: gogdl's manifest, else the
/// `goggame-<id>.info` file in the install folder.
pub fn client_id(paths: &DataPaths, app_name: &str, install_path: &str) -> Option<String> {
    let from = |doc: Value| doc.get("clientId").and_then(Value::as_str).filter(|c| !c.is_empty()).map(str::to_string);
    json_store::read_value(&manifest_file(&paths.gogdl_config_dir(), app_name)).ok().and_then(from).or_else(|| {
        std::fs::read_to_string(heronite_launch::gog::info_file(install_path, app_name))
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .and_then(from)
    })
}

fn installed_windows(paths: &DataPaths, app_name: &str) -> Option<Installed> {
    installed::get(paths, app_name).filter(|i| i.platform.eq_ignore_ascii_case("windows"))
}

/// Whether the game can have cloud saves: installed as the Windows build and
/// its client id is known. Reads two small files, no network.
pub fn supported(paths: &DataPaths, app_name: &str) -> bool {
    installed_windows(paths, app_name).is_some_and(|i| client_id(paths, app_name, &i.install_path).is_some())
}

fn remote_config_url(client_id: &str) -> String {
    format!("{REMOTE_CONFIG}/{client_id}?component_version={COMPONENT_VERSION}")
}

/// Name and location template of each folder GOG lists for Windows; empty
/// when the game has no cloud saves.
fn parse_remote(client_id: &str, doc: &Value) -> Vec<(String, String)> {
    let Some(storage) = doc.pointer("/content/Windows/cloudStorage") else { return Vec::new() };
    if storage.get("enabled").and_then(Value::as_bool) == Some(false) {
        return Vec::new();
    }
    let listed: Vec<(String, String)> = storage
        .get("locations")
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .filter_map(|l| {
                    let text = |k: &str| l.get(k).and_then(Value::as_str).filter(|s| !s.is_empty()).map(str::to_string);
                    Some((text("name")?, text("location")?))
                })
                .collect()
        })
        .unwrap_or_default();
    if listed.is_empty() {
        return vec![(DEFAULT_NAME.to_string(), DEFAULT_LOCATION.replace("{client}", client_id))];
    }
    listed
}

/// Where the Windows folders of the user live: `<home>` is
/// `drive_c/users/<user>` of the prefix.
struct Folders<'a> {
    install: &'a str,
    home: PathBuf,
}

impl Folders<'_> {
    fn variable(&self, name: &str) -> Option<String> {
        let sub = |rel: &str| self.home.join(rel).display().to_string();
        Some(match name {
            "INSTALL" => self.install.to_string(),
            "APPLICATION_DATA_LOCAL" => sub("AppData/Local"),
            "APPLICATION_DATA_LOCAL_LOW" => sub("AppData/LocalLow"),
            "APPLICATION_DATA_ROAMING" => sub("AppData/Roaming"),
            "SAVED_GAMES" => sub("Saved Games"),
            "DOCUMENTS" => sub("Documents"),
            _ => return None,
        })
    }
}

/// `<?NAME?>` variables replaced, `\` turned into `/` (in the template only:
/// the substituted paths are already Linux paths) and `.` / `..` / `//`
/// collapsed. `None` for a variable that is not known.
fn resolve_template(template: &str, folders: &Folders) -> Option<String> {
    let mut out = String::new();
    let mut rest = template;
    while let Some(start) = rest.find("<?") {
        out.push_str(&rest[..start].replace('\\', "/"));
        let after = &rest[start + 2..];
        let end = after.find("?>")?;
        out.push_str(&folders.variable(&after[..end])?);
        rest = &after[end + 2..];
    }
    out.push_str(&rest.replace('\\', "/"));
    Some(collapse(&out))
}

/// Lexical `.` / `..` / empty-component removal.
fn collapse(path: &str) -> String {
    let mut parts: Vec<&str> = Vec::new();
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            p => parts.push(p),
        }
    }
    let joined = parts.join("/");
    if path.starts_with('/') {
        format!("/{joined}")
    } else {
        joined
    }
}

/// The user folder inside the prefix: Proton keeps the Windows tree in
/// `<prefix>/pfx` and always names the user `steamuser`.
fn user_home(prefix: &str, proton: bool, user: &str) -> PathBuf {
    let (root, user) = if proton { (Path::new(prefix).join("pfx"), "steamuser") } else { (PathBuf::from(prefix), user) };
    root.join("drive_c/users").join(user)
}

fn system_user() -> String {
    std::env::var("USER").ok().filter(|u| !u.is_empty()).unwrap_or_else(|| "user".to_string())
}

/// The templates resolved against the game's Wine prefix.
fn resolve_all(templates: &[(String, String)], install: &str, prefix: &str, proton: bool, user: &str) -> Vec<SaveLocation> {
    let folders = Folders { install, home: user_home(prefix, proton, user) };
    templates
        .iter()
        .filter_map(|(name, template)| match resolve_template(template, &folders) {
            Some(location) => Some(SaveLocation { name: name.clone(), location }),
            None => {
                log::warn!("gog saves: the location {template} uses a folder this app does not know");
                None
            }
        })
        .collect()
}

fn stored_locations(paths: &DataPaths, app_name: &str) -> Option<Vec<SaveLocation>> {
    let settings = game_config::load_game(paths, app_name, &SystemProbe.home());
    let list = settings.get(SETTINGS_KEY)?.as_array()?;
    Some(list.iter().filter_map(|l| serde_json::from_value(l.clone()).ok()).filter(|l: &SaveLocation| !l.location.is_empty()).collect())
}

fn fetch_remote(url: &str) -> Result<String> {
    let resp = crate::api::get(url, &[])?;
    if resp.status / 100 != 2 {
        bail!(tr!("GOG answered with HTTP {status}", status = resp.status));
    }
    Ok(resp.body)
}

/// The save folders of the game. They are asked from GOG once and kept in the
/// game's settings (`gogSaves`); an empty list means no cloud saves.
/// Blocking, network the first time.
pub fn locations(paths: &DataPaths, app_name: &str) -> Result<Vec<SaveLocation>> {
    locations_with(paths, app_name, false, &fetch_remote)
}

/// Like [`locations`] but asks GOG again and replaces the stored list.
pub fn refresh_locations(paths: &DataPaths, app_name: &str) -> Result<Vec<SaveLocation>> {
    locations_with(paths, app_name, true, &fetch_remote)
}

fn locations_with(paths: &DataPaths, app_name: &str, force: bool, fetch: &dyn Fn(&str) -> Result<String>) -> Result<Vec<SaveLocation>> {
    if !force {
        if let Some(list) = stored_locations(paths, app_name) {
            return Ok(list);
        }
    }
    let game = installed_windows(paths, app_name).ok_or_else(|| anyhow!(tr!("{app} is not installed", app = app_name)))?;
    let client = client_id(paths, app_name, &game.install_path).ok_or_else(|| anyhow!(tr!("This game has no cloud saves")))?;
    let body = fetch(&remote_config_url(&client))?;
    let doc: Value = serde_json::from_str(&body).context("invalid answer from GOG")?;
    let templates = parse_remote(&client, &doc);

    let settings = heronite_launch::settings::load_game_settings(paths, app_name, &PlanContext::load(paths).global, &SystemProbe);
    if settings.wine_prefix.is_empty() && !templates.is_empty() {
        bail!(tr!("This game has no Wine prefix yet; choose one in its Wine settings"));
    }
    let proton = settings.wine_version.kind == "proton";
    let list = resolve_all(&templates, &game.install_path, &settings.wine_prefix, proton, &system_user());
    game_config::set_game_setting(paths, app_name, &SystemProbe.home(), SETTINGS_KEY, json!(list))?;
    Ok(list)
}

fn timestamps_file(paths: &DataPaths) -> PathBuf {
    paths.gog_store("saveTimestamps")
}

/// The time (seconds since the epoch, as gogdl prints it) of the last sync of
/// a location; `"0"` when it never synced.
fn timestamp(paths: &DataPaths, app_name: &str, name: &str) -> String {
    let doc = json_store::read_value(&timestamps_file(paths)).unwrap_or(Value::Null);
    doc.get(app_name).and_then(|g| g.get(name)).and_then(Value::as_str).filter(|t| !t.is_empty()).unwrap_or("0").to_string()
}

fn store_timestamp(paths: &DataPaths, app_name: &str, name: &str, ts: &str) -> Result<()> {
    json_store::update_value(&timestamps_file(paths), |doc| {
        let obj = doc.as_object_mut().ok_or_else(|| anyhow!("saveTimestamps.json is not an object"))?;
        let game = obj.entry(app_name).or_insert_with(|| json!({}));
        if !game.is_object() {
            *game = json!({});
        }
        game[name] = json!(ts);
        Ok(())
    })
}

/// The new sync time gogdl prints as the last line of stdout.
fn parse_timestamp(stdout: &str) -> Option<String> {
    stdout.lines().rev().map(str::trim).find(|l| !l.is_empty()).filter(|l| l.parse::<f64>().is_ok()).map(str::to_string)
}

/// `gogdl save-sync` arguments (without the auth file prefix).
fn sync_args(location: &SaveLocation, app_name: &str, ts: &str, flag: Option<&str>) -> Vec<String> {
    let mut args: Vec<String> = ["save-sync", &location.location, app_name, "--os", "windows", "--ts", ts, "--name", &location.name]
        .iter()
        .map(|s| s.to_string())
        .collect();
    args.extend(flag.map(str::to_string));
    args
}

/// Syncs every save folder of the game. Returns gogdl's log for the message
/// dialog. Blocking.
pub fn sync(paths: &DataPaths, gogdl: &Gogdl, app_name: &str, mode: SyncMode) -> Result<String> {
    run_sync(paths, gogdl, app_name, mode.flag(), &fetch_remote)
}

/// [`sync`] with a gogdl flag: `--skip-upload` (only download),
/// `--skip-download` (only upload), `--force-download`, `--force-upload` or
/// `""` (both ways).
pub fn sync_flag(paths: &DataPaths, gogdl: &Gogdl, app_name: &str, flag: &str) -> Result<String> {
    let flag = match flag {
        "" => None,
        "--skip-upload" | "--skip-download" | "--force-download" | "--force-upload" => Some(flag),
        other => bail!("unknown cloud saves flag {other}"),
    };
    run_sync(paths, gogdl, app_name, flag, &fetch_remote)
}

fn run_sync(
    paths: &DataPaths,
    gogdl: &Gogdl,
    app_name: &str,
    flag: Option<&str>,
    fetch: &dyn Fn(&str) -> Result<String>,
) -> Result<String> {
    let list = locations_with(paths, app_name, false, fetch)?;
    if list.is_empty() {
        return Ok(tr!("This game has no cloud saves").to_string());
    }
    let mut log = String::new();
    let mut failures: Vec<String> = Vec::new();
    for location in &list {
        if let Err(err) = std::fs::create_dir_all(&location.location) {
            failures.push(tr!("Could not create {path}: {error}", path = location.location.as_str(), error = err));
            continue;
        }
        let ts = timestamp(paths, app_name, &location.name);
        let out = gogdl.run(&sync_args(location, app_name, &ts, flag))?;
        if list.len() > 1 {
            log.push_str(&format!("{}\n", location.name));
        }
        for line in out.stderr.lines().map(str::trim).filter(|l| !l.is_empty() && !l.contains("DEBUG")) {
            log.push_str(line);
            log.push('\n');
        }
        if !out.success() {
            let why = out.error_lines();
            let why = if why.is_empty() {
                out.stderr.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or_default().trim().to_string()
            } else {
                why
            };
            log::error!("gog saves of {app_name} ({}): {why}", location.name);
            failures.push(if why.is_empty() { format!("gogdl exit code {:?}", out.code) } else { why });
        } else if let Some(ts) = parse_timestamp(&out.stdout) {
            store_timestamp(paths, app_name, &location.name, &ts)?;
        }
    }
    if failures.is_empty() {
        Ok(log)
    } else {
        Err(anyhow!(tr!("Cloud saves could not be synced: {error}", error = failures.join("\n"))))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    use crate::cmd::testing::{calls, fake, setup};

    #[cfg(unix)]
    fn folders(prefix: &str, proton: bool) -> Folders<'static> {
        Folders { install: "/games/g", home: user_home(prefix, proton, "rafa") }
    }

    // Linux paths and Wine prefixes; GOG is off on Windows (`Feature::StoreAccounts`).
    #[cfg(unix)]
    #[test]
    fn variables_resolve_inside_a_wine_prefix() {
        let f = folders("/pfx", false);
        let r = |t: &str| resolve_template(t, &f).unwrap();
        assert_eq!(r("<?INSTALL?>\\saves\\slot1"), "/games/g/saves/slot1");
        assert_eq!(r("<?APPLICATION_DATA_LOCAL?>\\Studio\\Game"), "/pfx/drive_c/users/rafa/AppData/Local/Studio/Game");
        assert_eq!(r("<?APPLICATION_DATA_ROAMING?>/Game"), "/pfx/drive_c/users/rafa/AppData/Roaming/Game");
        assert_eq!(r("<?APPLICATION_DATA_LOCAL_LOW?>\\X"), "/pfx/drive_c/users/rafa/AppData/LocalLow/X");
        assert_eq!(r("<?SAVED_GAMES?>/Game"), "/pfx/drive_c/users/rafa/Saved Games/Game");
        assert_eq!(r("<?DOCUMENTS?>\\My Games\\G"), "/pfx/drive_c/users/rafa/Documents/My Games/G");
        assert_eq!(r("<?DOCUMENTS?>/a/../b//./c"), "/pfx/drive_c/users/rafa/Documents/b/c");
        assert!(resolve_template("<?NOPE?>/x", &f).is_none());
        assert!(resolve_template("<?INSTALL", &f).is_none());
    }

    // Linux paths and Wine prefixes; GOG is off on Windows (`Feature::StoreAccounts`).
    #[cfg(unix)]
    #[test]
    fn proton_uses_the_pfx_folder_and_steamuser() {
        let f = folders("/pfx", true);
        assert_eq!(resolve_template("<?APPLICATION_DATA_LOCAL?>/G", &f).unwrap(), "/pfx/pfx/drive_c/users/steamuser/AppData/Local/G");
    }

    #[test]
    fn remote_answers_become_templates() {
        let doc = json!({"content": {"Windows": {"cloudStorage": {"enabled": true, "locations": [
            {"name": "saves", "location": "<?INSTALL?>/saves"}, {"name": "", "location": "x"}, {"name": "cfg"}]}}}});
        assert_eq!(parse_remote("77", &doc), [("saves".to_string(), "<?INSTALL?>/saves".to_string())]);
        let none = json!({"content": {"Windows": {"cloudStorage": {"enabled": true, "locations": []}}}});
        assert_eq!(
            parse_remote("77", &none),
            [("__default".to_string(), "<?APPLICATION_DATA_LOCAL?>/GOG.com/Galaxy/Applications/77/Storage/Shared/Files".to_string())]
        );
        assert!(parse_remote("77", &json!({"content": {"Windows": {"cloudStorage": {"enabled": false}}}})).is_empty());
        assert!(parse_remote("77", &json!({"content": {}})).is_empty());
        assert_eq!(remote_config_url("77"), "https://remote-config.gog.com/components/galaxy_client/clients/77?component_version=2.0.45");
    }

    /// An installed game with an info file; returns the install folder.
    #[cfg(unix)]
    fn install(dir: &Path, paths: &DataPaths, platform: &str, client: Option<&str>) -> String {
        let game = dir.join("game");
        std::fs::create_dir_all(&game).unwrap();
        if let Some(c) = client {
            std::fs::write(game.join("goggame-42.info"), json!({"clientId": c}).to_string()).unwrap();
        }
        let mut entry = Installed::new("42");
        entry.platform = platform.into();
        entry.install_path = game.display().to_string();
        installed::upsert(paths, &entry).unwrap();
        game.display().to_string()
    }

    #[cfg(unix)]
    fn set_prefix(paths: &DataPaths, prefix: &Path, proton: bool) {
        let kind = if proton { "proton" } else { "wine" };
        game_config::set_game_setting(paths, "42", "/home/x", "winePrefix", json!(prefix.display().to_string())).unwrap();
        game_config::set_game_setting(paths, "42", "/home/x", "wineVersion", json!({"bin": "/w/bin/wine", "name": "W", "type": kind}))
            .unwrap();
    }

    #[cfg(unix)]
    const REMOTE: &str = r#"{"content":{"Windows":{"cloudStorage":{"enabled":true,"locations":[{"name":"saves","location":"<?APPLICATION_DATA_LOCAL?>\\Game\\Saves"}]}}}}"#;

    // Linux paths and Wine prefixes; GOG is off on Windows (`Feature::StoreAccounts`).
    #[cfg(unix)]
    #[test]
    fn supported_needs_a_windows_install_and_a_client_id() {
        let (dir, paths) = setup();
        assert!(!supported(&paths, "42"));
        install(dir.path(), &paths, "windows", None);
        assert!(!supported(&paths, "42"));
        install(dir.path(), &paths, "windows", Some("777"));
        assert!(supported(&paths, "42"));
        install(dir.path(), &paths, "linux", Some("777"));
        assert!(!supported(&paths, "42"));
    }

    // Linux paths and Wine prefixes; GOG is off on Windows (`Feature::StoreAccounts`).
    #[cfg(unix)]
    #[test]
    fn the_manifest_gives_the_client_id_first() {
        let (dir, paths) = setup();
        let game = install(dir.path(), &paths, "windows", Some("from-info"));
        assert_eq!(client_id(&paths, "42", &game).as_deref(), Some("from-info"));
        let manifest = manifest_file(&paths.gogdl_config_dir(), "42");
        std::fs::create_dir_all(manifest.parent().unwrap()).unwrap();
        std::fs::write(&manifest, r#"{"clientId":"from-manifest"}"#).unwrap();
        assert_eq!(client_id(&paths, "42", &game).as_deref(), Some("from-manifest"));
    }

    // Linux paths and Wine prefixes; GOG is off on Windows (`Feature::StoreAccounts`).
    #[cfg(unix)]
    #[test]
    fn locations_are_fetched_once_and_kept_in_the_game_settings() {
        let (dir, paths) = setup();
        install(dir.path(), &paths, "windows", Some("777"));
        let prefix = dir.path().join("pfx");
        set_prefix(&paths, &prefix, false);
        let asked = std::cell::RefCell::new(Vec::new());
        let fetch = |url: &str| {
            asked.borrow_mut().push(url.to_string());
            Ok(REMOTE.to_string())
        };
        let list = locations_with(&paths, "42", false, &fetch).unwrap();
        let want = prefix.join("drive_c/users").join(system_user()).join("AppData/Local/Game/Saves").display().to_string();
        assert_eq!(list, [SaveLocation { name: "saves".into(), location: want.clone() }]);
        assert_eq!(asked.borrow().len(), 1);
        assert!(asked.borrow()[0].contains("/clients/777?"));

        // The stored list is used (and may be edited); a refresh asks again.
        game_config::set_game_setting(&paths, "42", "/home/x", SETTINGS_KEY, json!([{"name": "saves", "location": "/mine"}])).unwrap();
        assert_eq!(locations_with(&paths, "42", false, &fetch).unwrap()[0].location, "/mine");
        assert_eq!(asked.borrow().len(), 1);
        assert_eq!(locations_with(&paths, "42", true, &fetch).unwrap()[0].location, want);
        assert_eq!(asked.borrow().len(), 2);
    }

    #[cfg(unix)]
    #[test]
    fn a_game_without_cloud_saves_has_an_empty_list() {
        let (dir, paths) = setup();
        install(dir.path(), &paths, "windows", Some("777"));
        set_prefix(&paths, &dir.path().join("pfx"), true);
        let off = |_: &str| Ok(r#"{"content":{"Windows":{"cloudStorage":{"enabled":false}}}}"#.to_string());
        assert!(locations_with(&paths, "42", false, &off).unwrap().is_empty());
        let g = fake(dir.path(), &paths, "echo 1");
        let out = run_sync(&paths, &g, "42", None, &off).unwrap();
        assert!(out.contains("no cloud saves"), "{out}");
        assert!(calls(dir.path()).is_empty());
        let offline = |_: &str| -> Result<String> { bail!("offline") };
        assert!(locations_with(&paths, "99", true, &offline).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn syncing_runs_gogdl_per_location_and_remembers_the_time() {
        let (dir, paths) = setup();
        install(dir.path(), &paths, "windows", Some("777"));
        let prefix = dir.path().join("pfx");
        set_prefix(&paths, &prefix, true);
        let fetch = |_: &str| Ok(REMOTE.to_string());
        let g = fake(dir.path(), &paths, "echo '[SAVES] INFO: Done' >&2; echo '[SAVES] DEBUG: noise' >&2; echo 1700000000.5");
        let out = run_sync(&paths, &g, "42", None, &fetch).unwrap();
        assert!(out.contains("Done") && !out.contains("noise"), "{out}");

        let place = prefix.join("pfx/drive_c/users/steamuser/AppData/Local/Game/Saves");
        assert!(place.is_dir(), "the location is created first");
        let first = &calls(dir.path())[0];
        assert_eq!(first, &format!("save-sync {} 42 --os windows --ts 0 --name saves", place.display()));
        let doc = json_store::read_value(&paths.gog_store("saveTimestamps")).unwrap();
        assert_eq!(doc["42"]["saves"], "1700000000.5");

        // The next run passes the stored time, with the flag of the mode.
        run_sync(&paths, &g, "42", SyncMode::ForceUpload.flag(), &fetch).unwrap();
        assert!(calls(dir.path())[1].ends_with("--ts 1700000000.5 --name saves --force-upload"), "{:?}", calls(dir.path()));
    }

    #[cfg(unix)]
    #[test]
    fn flags_per_mode_and_unknown_flags() {
        assert_eq!(SyncMode::Both.flag(), None);
        assert_eq!(SyncMode::ForceDownload.flag(), Some("--force-download"));
        assert_eq!(SyncMode::ForceUpload.flag(), Some("--force-upload"));
        let loc = SaveLocation { name: "n".into(), location: "/l".into() };
        assert_eq!(sync_args(&loc, "42", "0", Some("--skip-upload")).last().unwrap(), "--skip-upload");
        let (dir, paths) = setup();
        let g = fake(dir.path(), &paths, "true");
        assert!(sync_flag(&paths, &g, "42", "--bogus").is_err());
        assert_eq!(parse_timestamp("noise\n1.5\n"), Some("1.5".to_string()));
        assert_eq!(parse_timestamp("1.5\nnot a time\n"), None);
    }

    #[cfg(unix)]
    #[test]
    fn a_failing_gogdl_is_an_error_and_keeps_the_old_time() {
        let (dir, paths) = setup();
        install(dir.path(), &paths, "windows", Some("777"));
        set_prefix(&paths, &dir.path().join("pfx"), false);
        let fetch = |_: &str| Ok(REMOTE.to_string());
        store_timestamp(&paths, "42", "saves", "5").unwrap();
        let g = fake(dir.path(), &paths, "echo '[API] ERROR: no token' >&2; exit 1");
        let err = run_sync(&paths, &g, "42", None, &fetch).unwrap_err().to_string();
        assert!(err.contains("no token"), "{err}");
        assert_eq!(timestamp(&paths, "42", "saves"), "5");
        assert_eq!(timestamp(&paths, "42", "other"), "0");
    }
}
