//! `installed.json` (legendary's `InstalledGame` per app) and the copies of
//! installed builds' manifests in `manifests/`, which updates, repairs and
//! uninstalls read.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

use crate::files;
use crate::library::GameMeta;
use crate::manifest::Manifest;

pub fn installed_file(config_dir: &Path) -> PathBuf {
    config_dir.join("installed.json")
}

/// An installed game, as legendary stores it. Keys this type does not name
/// stay in `rest`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct InstalledGame {
    pub app_name: String,
    pub install_path: String,
    pub title: String,
    pub version: String,
    pub base_urls: Vec<String>,
    pub can_run_offline: bool,
    /// The Epic Games Launcher's id of the install, when it shares it.
    pub egl_guid: String,
    /// Relative to `install_path`.
    pub executable: String,
    /// Bytes.
    pub install_size: u64,
    pub install_tags: Vec<String>,
    pub is_dlc: bool,
    pub launch_parameters: String,
    pub manifest_path: Option<String>,
    pub needs_verification: bool,
    /// `Windows` or `Mac`.
    pub platform: String,
    /// `{ids, name, path, args, installed?}`.
    pub prereq_info: Option<Value>,
    /// `{path, args}`.
    pub uninstaller: Option<Value>,
    /// The game checks an ownership token (DRM).
    pub requires_ot: bool,
    pub save_path: Option<String>,
    pub is_preloaded: bool,
    #[serde(flatten)]
    pub rest: Map<String, Value>,
}

impl Default for InstalledGame {
    fn default() -> Self {
        Self {
            app_name: String::new(),
            install_path: String::new(),
            title: String::new(),
            version: String::new(),
            base_urls: Vec::new(),
            can_run_offline: false,
            egl_guid: String::new(),
            executable: String::new(),
            install_size: 0,
            install_tags: Vec::new(),
            is_dlc: false,
            launch_parameters: String::new(),
            manifest_path: None,
            needs_verification: false,
            platform: "Windows".into(),
            prereq_info: None,
            uninstaller: None,
            requires_ot: false,
            save_path: None,
            is_preloaded: false,
            rest: Map::new(),
        }
    }
}

/// What a finished install, update or import knows of the game.
#[derive(Debug, Clone, Copy)]
pub struct Install<'a> {
    pub game: &'a GameMeta,
    pub manifest: &'a Manifest,
    pub install_path: &'a str,
    pub platform: &'a str,
    pub install_tags: &'a [String],
    /// Bytes of the selected files.
    pub install_size: u64,
    pub base_urls: &'a [String],
    /// A DLC installed into its game's folder.
    pub is_dlc: bool,
    pub egl_guid: &'a str,
    pub is_preloaded: bool,
    /// A program the game starts with instead of the manifest's
    /// ([`crate::legendary_config::exe_override`]); used only when the build has it.
    pub exe_override: Option<&'a str>,
}

impl InstalledGame {
    /// The entry legendary writes when an install ends (`prepare_download`).
    pub fn from_install(i: &Install) -> Self {
        let m = &i.manifest.meta;
        let prereq_info = (!m.prereq_ids.is_empty())
            .then(|| json!({"ids": m.prereq_ids, "name": m.prereq_name, "path": m.prereq_path, "args": m.prereq_args}));
        let uninstaller =
            (!m.uninstall_action_path.is_empty()).then(|| json!({"path": m.uninstall_action_path, "args": m.uninstall_action_args}));
        let attr = |name: &str, default: &str| i.game.custom_attribute(name).unwrap_or(default).to_lowercase();
        let mut executable = m.launch_exe.clone();
        if i.platform != "Mac" {
            if let Some(exe) = i.exe_override {
                let wanted = exe.to_lowercase();
                if i.manifest.files.iter().any(|f| f.filename.to_lowercase() == wanted) {
                    log::info!("epic: {} starts {exe} instead of {executable}", i.game.app_name);
                    executable = exe.to_string();
                }
            }
        }
        InstalledGame {
            app_name: i.game.app_name.clone(),
            install_path: i.install_path.to_string(),
            title: i.game.app_title.clone(),
            version: m.build_version.clone(),
            base_urls: i.base_urls.to_vec(),
            can_run_offline: attr("CanRunOffline", "true") == "true",
            egl_guid: i.egl_guid.to_string(),
            executable,
            install_size: i.install_size,
            install_tags: i.install_tags.to_vec(),
            is_dlc: i.is_dlc,
            launch_parameters: m.launch_command.clone(),
            requires_ot: attr("OwnershipToken", "false") == "true",
            platform: i.platform.to_string(),
            prereq_info,
            uninstaller,
            is_preloaded: i.is_preloaded,
            ..Default::default()
        }
    }

    /// The prerequisites still to run (`prereq_info` not marked `installed`).
    pub fn pending_prereq(&self) -> Option<&Value> {
        self.prereq_info.as_ref().filter(|p| !p.get("installed").and_then(Value::as_bool).unwrap_or(false))
    }
}

/// Every installed game, by app name.
pub fn read_all(config_dir: &Path) -> BTreeMap<String, InstalledGame> {
    let Some(Value::Object(doc)) = files::read_json(&installed_file(config_dir)) else { return BTreeMap::new() };
    doc.into_iter().filter_map(|(k, v)| serde_json::from_value(v).ok().map(|g| (k, g))).collect()
}

pub fn get(config_dir: &Path, app_name: &str) -> Option<InstalledGame> {
    read_all(config_dir).remove(app_name)
}

fn update(config_dir: &Path, f: impl FnOnce(&mut Map<String, Value>)) -> Result<()> {
    let path = installed_file(config_dir);
    let mut doc = match files::read_json(&path) {
        Some(Value::Object(map)) => map,
        _ => Map::new(),
    };
    f(&mut doc);
    files::write_json(&path, &Value::Object(doc), false)
}

/// Records `game` as installed; an entry already there keeps the keys the
/// new one does not set (legendary's `set_installed_game`).
pub fn set(config_dir: &Path, game: &InstalledGame) -> Result<()> {
    let Value::Object(fields) = serde_json::to_value(game)? else { unreachable!("a struct serializes to an object") };
    update(config_dir, |doc| match doc.get_mut(&game.app_name) {
        Some(Value::Object(old)) => old.extend(fields),
        _ => {
            doc.insert(game.app_name.clone(), Value::Object(fields));
        }
    })
}

/// Forgets an install (the files stay).
pub fn remove(config_dir: &Path, app_name: &str) -> Result<()> {
    update(config_dir, |doc| {
        doc.shift_remove(app_name);
    })
}

/// Marks the game's prerequisites as run.
pub fn mark_prereq_installed(config_dir: &Path, app_name: &str) -> Result<()> {
    let Some(mut game) = get(config_dir, app_name) else { return Ok(()) };
    if let Some(Value::Object(p)) = game.prereq_info.as_mut() {
        p.insert("installed".into(), json!(true));
        set(config_dir, &game)?;
    }
    Ok(())
}

pub fn manifests_dir(config_dir: &Path) -> PathBuf {
    config_dir.join("manifests")
}

/// `manifests/<app>_<platform>_<version>.manifest` (legendary's name).
pub fn manifest_file(config_dir: &Path, app_name: &str, version: &str, platform: &str) -> PathBuf {
    manifests_dir(config_dir).join(format!("{}.manifest", files::clean_filename(&format!("{app_name}_{platform}_{version}"))))
}

/// The stored manifest of a build; older legendary versions named it
/// without the platform.
pub fn load_manifest(config_dir: &Path, app_name: &str, version: &str, platform: &str) -> Option<Vec<u8>> {
    std::fs::read(manifest_file(config_dir, app_name, version, platform))
        .or_else(|_| {
            std::fs::read(manifests_dir(config_dir).join(format!("{}.manifest", files::clean_filename(&format!("{app_name}_{version}")))))
        })
        .ok()
}

/// The parsed manifest of an installed game.
pub fn installed_manifest(config_dir: &Path, game: &InstalledGame) -> Result<Manifest> {
    let bytes = load_manifest(config_dir, &game.app_name, &game.version, &game.platform)
        .with_context(|| format!("the manifest of {} {} is missing", game.app_name, game.version))?;
    Manifest::parse(&bytes)
}

/// Stores a build's manifest (decrypted) for later updates and repairs.
pub fn save_manifest(config_dir: &Path, app_name: &str, platform: &str, manifest: &Manifest) -> Result<PathBuf> {
    let path = manifest_file(config_dir, app_name, &manifest.meta.build_version, platform);
    std::fs::create_dir_all(manifests_dir(config_dir))?;
    heronite_data::temps::write_atomic(&path, &manifest.to_bytes()?, None).with_context(|| format!("writing {}", path.display()))?;
    Ok(path)
}

/// Deletes stored manifests no installed build uses (legendary's
/// `clean_manifests`, run by `legendary cleanup`). Returns how many went.
pub fn clean_manifests(config_dir: &Path) -> usize {
    let in_use: std::collections::HashSet<PathBuf> = read_all(config_dir)
        .values()
        .flat_map(|g| {
            [
                manifest_file(config_dir, &g.app_name, &g.version, &g.platform),
                manifests_dir(config_dir).join(format!("{}.manifest", files::clean_filename(&format!("{}_{}", g.app_name, g.version)))),
            ]
        })
        .collect();
    let mut removed = 0;
    for entry in std::fs::read_dir(manifests_dir(config_dir)).into_iter().flatten().flatten() {
        let path = entry.path();
        if path.extension().is_some_and(|e| e == "manifest") && !in_use.contains(&path) && std::fs::remove_file(&path).is_ok() {
            removed += 1;
        }
    }
    removed
}

/// What the Epic Games Launcher left in a game's folder (`.egstore`): the
/// manifest of the build it installed, and whether its install was cut
/// short (legendary's `import_game`).
#[derive(Debug, Clone, PartialEq)]
pub struct EglInstall {
    pub manifest: Vec<u8>,
    /// The launcher's id of the install (`<guid>.manifest`).
    pub guid: String,
    /// A download or update was in progress: the files must be checked.
    pub needs_verification: bool,
}

/// Reads `<folder>/.egstore` for `app_name`, when the launcher installed it.
pub fn egl_install(folder: &Path, app_name: &str, guid: Option<&str>) -> Option<EglInstall> {
    let store = folder.join(".egstore");
    let guid = match guid.filter(|g| !g.is_empty()) {
        Some(g) => g.to_string(),
        None => std::fs::read_dir(&store).ok()?.flatten().find_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            let stem = name.strip_suffix(".mancpn")?.to_string();
            let doc: Value = serde_json::from_slice(&std::fs::read(e.path()).ok()?).ok()?;
            (doc.get("AppName").and_then(Value::as_str) == Some(app_name)).then_some(stem)
        })?,
    };
    let manifest = std::fs::read(store.join(format!("{guid}.manifest"))).ok()?;
    let pending = std::fs::read_dir(store.join("Pending")).map(|mut d| d.next().is_some()).unwrap_or(false);
    Some(EglInstall { manifest, guid, needs_verification: store.join("bps").exists() || pending })
}

/// Empties legendary's `tmp` folder (resume files, ownership tokens).
pub fn clean_tmp(config_dir: &Path) {
    for entry in std::fs::read_dir(config_dir.join("tmp")).into_iter().flatten().flatten() {
        let _ = std::fs::remove_file(entry.path());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::tests::sample;

    /// The shape of a real entry legendary wrote.
    const REAL: &str = r#"{"Calluna": {"app_name": "Calluna", "base_urls": ["https://cdn/Calluna/default"], "can_run_offline": true,
        "egl_guid": "", "executable": "Control.exe", "install_path": "/games/Control", "install_size": 45010292103, "install_tags": [],
        "is_dlc": false, "is_preloaded": false, "launch_parameters": "", "manifest_path": null, "needs_verification": false,
        "platform": "Windows", "prereq_info": null, "requires_ot": false, "save_path": "/pfx/drive_c/users/steamuser/AppData/Local/Remedy",
        "title": "Control", "uninstaller": null, "version": "1.33"}}"#;

    #[test]
    fn entries_round_trip_with_legendarys_keys() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(installed_file(dir.path()), REAL).unwrap();
        let all = read_all(dir.path());
        let g = &all["Calluna"];
        assert_eq!(
            (g.install_size, g.executable.as_str(), g.save_path.as_deref()),
            (45010292103, "Control.exe", Some("/pfx/drive_c/users/steamuser/AppData/Local/Remedy"))
        );
        set(dir.path(), g).unwrap();
        let back = files::read_json(&installed_file(dir.path())).unwrap();
        assert_eq!(back, serde_json::from_str::<Value>(REAL).unwrap());
        remove(dir.path(), "Calluna").unwrap();
        assert!(read_all(dir.path()).is_empty());
    }

    #[test]
    fn a_finished_install_gives_legendarys_entry() {
        let m = sample();
        let game = GameMeta::from_value(json!({"app_name": "Calluna", "app_title": "Control", "metadata": {
            "customAttributes": {"OwnershipToken": {"value": "True"}}}}))
        .unwrap();
        let tags = vec![String::new()];
        let i = Install {
            game: &game,
            manifest: &m,
            install_path: "/games/Control",
            platform: "Windows",
            install_tags: &tags,
            install_size: 220,
            base_urls: &["https://cdn".to_string()],
            is_dlc: false,
            egl_guid: "",
            is_preloaded: false,
            exe_override: Some("data/A.PAK"),
        };
        let g = InstalledGame::from_install(&i);
        assert!(g.can_run_offline && g.requires_ot);
        assert_eq!(g.executable, "data/A.PAK", "an override the build has is used");
        assert_eq!(g.prereq_info.as_ref().unwrap()["path"], "redist/vc.exe");
        assert_eq!(g.uninstaller.as_ref().unwrap()["path"], "uninstall.exe");
        assert_eq!(g.version, "1.33");
        assert!(g.pending_prereq().is_some());
        let g2 = InstalledGame::from_install(&Install { exe_override: Some("missing.exe"), ..i });
        assert_eq!(g2.executable, "Control.exe");

        let dir = tempfile::tempdir().unwrap();
        set(dir.path(), &g).unwrap();
        mark_prereq_installed(dir.path(), "Calluna").unwrap();
        assert!(get(dir.path(), "Calluna").unwrap().pending_prereq().is_none());
    }

    #[test]
    fn epic_games_launcher_installs_are_recognised() {
        let dir = tempfile::tempdir().unwrap();
        let store = dir.path().join(".egstore");
        std::fs::create_dir_all(&store).unwrap();
        std::fs::write(store.join("ABC.mancpn"), r#"{"AppName": "Calluna"}"#).unwrap();
        std::fs::write(store.join("ABC.manifest"), b"m").unwrap();
        let found = egl_install(dir.path(), "Calluna", None).unwrap();
        assert_eq!((found.guid.as_str(), found.manifest.as_slice(), found.needs_verification), ("ABC", &b"m"[..], false));
        std::fs::create_dir_all(store.join("Pending")).unwrap();
        std::fs::write(store.join("Pending/x"), b"").unwrap();
        assert!(egl_install(dir.path(), "Calluna", Some("ABC")).unwrap().needs_verification);
        assert!(egl_install(dir.path(), "Other", None).is_none());
    }

    #[test]
    fn manifests_are_stored_by_app_platform_and_version() {
        let dir = tempfile::tempdir().unwrap();
        let m = sample();
        let path = save_manifest(dir.path(), "Calluna", "Windows", &m).unwrap();
        assert!(path.ends_with("manifests/Calluna_Windows_1.33.manifest"));
        let g = InstalledGame { app_name: "Calluna".into(), version: "1.33".into(), ..Default::default() };
        assert_eq!(installed_manifest(dir.path(), &g).unwrap().files.len(), 3);
        std::fs::write(manifests_dir(dir.path()).join("Old_1.0.manifest"), b"x").unwrap();
        set(dir.path(), &g).unwrap();
        assert_eq!(clean_manifests(dir.path()), 1);
        assert!(path.exists());
    }
}
