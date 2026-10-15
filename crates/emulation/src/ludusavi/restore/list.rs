//! The folders backups are restored from and the backups of one game in
//! each (`ludusavi backups --api`).

use std::fs;
use std::path::{Path, PathBuf};

use heronite_data::models::LudusaviSettings;
use serde_json::Value;

use super::super::backup::resolve_title;
use super::super::detect::LudusaviDetected;
use super::super::last_backup::{backup_root, RootFolders};
use super::super::{parse_json, LudusaviEnv, LudusaviGame};
use super::Tool;

/// Why a folder is in the list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceKind {
    /// Where the backups go (this app's or Ludusavi's backup folder).
    Backups,
    /// Added in Settings → Saves.
    Extra,
    /// Picked for this game only.
    Picked,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackupSource {
    pub root: PathBuf,
    pub kind: SourceKind,
}

/// One backup of `ludusavi backups --api`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackupEntry {
    /// What `restore --backup` takes (`backup-20261011T182941Z.zip`, `.`).
    pub id: String,
    /// ISO date-time.
    pub when: String,
    /// `windows` | `linux` | `mac`, when Ludusavi recorded it.
    pub os: Option<String>,
    /// Locked backups are never removed by the retention.
    pub locked: bool,
    /// The name given in the Backups tab (Ludusavi's comment).
    pub comment: Option<String>,
    /// Size on disk of its zip or folder (`None` when unknown).
    pub bytes: Option<u64>,
}

/// The backups of a game in one folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GameBackups {
    pub source: BackupSource,
    /// Ludusavi's name of the game there (`None`: no backup of it).
    pub game_name: Option<String>,
    /// The game's folder under the source (holds `mapping.yaml`).
    pub dir: Option<PathBuf>,
    /// Newest first.
    pub backups: Vec<BackupEntry>,
    pub error: Option<String>,
}

fn expand(path: &str, home: &str) -> Option<PathBuf> {
    let p = path.trim();
    if p.is_empty() {
        return None;
    }
    Some(match p.strip_prefix("~/") {
        Some(rest) => Path::new(home).join(rest),
        None if p == "~" => PathBuf::from(home),
        None => PathBuf::from(p),
    })
}

fn same(a: &Path, b: &Path) -> bool {
    let norm = |p: &Path| p.to_string_lossy().trim_end_matches('/').to_string();
    norm(a) == norm(b)
}

/// The folders to look for backups in, in order and without repeats: the
/// backup folder, the ones added in Settings, then `picked`. Ludusavi's own
/// restore folder is only listed once added (Settings offers it).
pub fn restore_sources(
    settings: &LudusaviSettings,
    detected: Option<&LudusaviDetected>,
    home: &str,
    picked: Option<&Path>,
) -> Vec<BackupSource> {
    let config_file = detected.and_then(|d| d.config_path.as_deref()).map(Path::new);
    let use_installed = settings.use_installed_config && config_file.is_some();
    let mut candidates: Vec<(PathBuf, SourceKind)> = Vec::new();
    if let Some(root) = backup_root(use_installed, &settings.backup_path, config_file, home)
        .or_else(|| detected.and_then(|d| d.backup_path.as_deref()).and_then(|p| expand(p, home)))
    {
        candidates.push((root, SourceKind::Backups));
    }
    candidates.extend(settings.restore_paths.iter().filter_map(|p| expand(p, home)).map(|p| (p, SourceKind::Extra)));
    candidates.extend(picked.map(|p| (p.to_path_buf(), SourceKind::Picked)));
    let mut out: Vec<BackupSource> = Vec::new();
    for (root, kind) in candidates {
        if !out.iter().any(|s| same(&s.root, &root)) {
            out.push(BackupSource { root, kind });
        }
    }
    out
}

fn size_of(path: &Path) -> u64 {
    match fs::symlink_metadata(path) {
        Ok(m) if m.is_dir() => fs::read_dir(path).map(|it| it.filter_map(Result::ok).map(|e| size_of(&e.path())).sum()).unwrap_or(0),
        Ok(m) => m.len(),
        Err(_) => 0,
    }
}

/// Size of a backup in the game's folder: its zip or folder, or for the
/// old simple-format `.` backup the `drive-*` folders next to `mapping.yaml`.
pub fn backup_size(game_dir: &Path, id: &str) -> Option<u64> {
    if id == "." {
        let drives = fs::read_dir(game_dir).ok()?.filter_map(Result::ok).filter(|e| e.file_name().to_string_lossy().starts_with("drive-"));
        return Some(drives.map(|e| size_of(&e.path())).sum());
    }
    let path = game_dir.join(id);
    path.exists().then(|| size_of(&path))
}

/// The backups of `game` in a `backups --api` reply, newest first.
pub fn parse_backups(payload: &Value, game: &str) -> Vec<BackupEntry> {
    let Some(list) = payload.pointer("/games").and_then(Value::as_object).and_then(|g| g.get(game)).and_then(|g| g.get("backups")) else {
        return Vec::new();
    };
    let text = |v: &Value, k: &str| v.get(k).and_then(Value::as_str).map(str::to_string).filter(|s| !s.is_empty());
    let mut out: Vec<BackupEntry> = list
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|b| {
            Some(BackupEntry {
                id: text(b, "name")?,
                when: text(b, "when").unwrap_or_default(),
                os: text(b, "os"),
                locked: b.get("locked").and_then(Value::as_bool).unwrap_or(false),
                comment: text(b, "comment"),
                bytes: None,
            })
        })
        .collect();
    out.sort_by(|a, b| b.when.cmp(&a.when));
    out
}

/// The backups of `game` in every source. A folder without a backup of it
/// has an empty list; the Steam / GOG id lookup (`ludusavi find`) runs only
/// when the title matches no folder.
pub fn list_backups(env: &LudusaviEnv, tool: &Tool, game: &LudusaviGame, picked: Option<&Path>) -> Vec<GameBackups> {
    let home = env.probe.home();
    let sources = restore_sources(&tool.settings, tool.detected.as_ref(), &home, picked);
    let steam = game.steam_app_id.as_deref();
    // Read once per folder: the second lookup reuses the mappings read.
    let mut roots: Vec<Option<RootFolders>> = sources.iter().map(|s| RootFolders::read(&s.root, steam)).collect();
    let mut found: Vec<Option<(String, PathBuf)>> =
        roots.iter_mut().map(|r| r.as_mut()?.find(&game.title).map(|b| (b.game_name, b.dir))).collect();
    if found.iter().any(Option::is_none) && (steam.is_some_and(|s| !s.is_empty()) || game.runner == "gog") {
        let name = resolve_title(env, &tool.binary, game);
        if name != game.title {
            for (slot, root) in found.iter_mut().zip(&mut roots) {
                if slot.is_none() {
                    *slot = root.as_mut().and_then(|r| r.find(&name)).map(|b| (b.game_name, b.dir));
                }
            }
        }
    }
    sources
        .into_iter()
        .zip(found)
        .map(|(source, name)| {
            let Some((name, dir)) = name else {
                return GameBackups { source, game_name: None, dir: None, backups: Vec::new(), error: None };
            };
            let args: Vec<String> = ["--no-manifest-update", "backups", "--api", "--path"]
                .iter()
                .map(|s| s.to_string())
                .chain([source.root.to_string_lossy().into_owned(), name.clone()])
                .collect();
            match env.runner.run(&tool.binary, &args) {
                Ok(out) => {
                    let mut backups = parse_backups(&parse_json(&out.stdout), &name);
                    for b in &mut backups {
                        b.bytes = backup_size(&dir, &b.id);
                    }
                    let error = (backups.is_empty() && out.code != 0).then(|| out.stderr.trim().to_string()).filter(|e| !e.is_empty());
                    GameBackups { source, game_name: Some(name), dir: Some(dir), backups, error }
                }
                Err(e) => GameBackups { source, game_name: Some(name), dir: Some(dir), backups: Vec::new(), error: Some(e) },
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings(backup_path: &str, extra: &[&str]) -> LudusaviSettings {
        LudusaviSettings {
            use_installed_config: false,
            backup_path: backup_path.into(),
            restore_paths: extra.iter().map(|s| s.to_string()).collect(),
            ..Default::default()
        }
    }

    #[test]
    fn sources_in_order_without_repeats() {
        let detected = LudusaviDetected { restore_path: Some("/win".into()), backup_path: Some("/lud".into()), ..Default::default() };
        let s = restore_sources(&settings("~/Saves", &["/win/", "/old"]), Some(&detected), "/home/u", Some(Path::new("/picked")));
        let got: Vec<(String, SourceKind)> = s.iter().map(|s| (s.root.to_string_lossy().into_owned(), s.kind)).collect();
        assert_eq!(
            got,
            [
                ("/home/u/Saves".into(), SourceKind::Backups),
                ("/win/".into(), SourceKind::Extra),
                ("/old".into(), SourceKind::Extra),
                ("/picked".into(), SourceKind::Picked),
            ]
        );
        // No folder set: Ludusavi's own backup folder.
        let s = restore_sources(&settings("", &[]), Some(&detected), "/h", None);
        assert_eq!(
            s,
            [BackupSource { root: "/lud".into(), kind: SourceKind::Backups }],
            "Ludusavi's restore folder is not added by itself"
        );
        assert!(restore_sources(&settings("", &[]), None, "/h", None).is_empty());
    }

    #[test]
    fn parses_backups_newest_first() {
        let v = serde_json::json!({"games": {"Elden Ring": {"backupPath": "/b/Elden Ring", "backups": [
            {"name": "backup-1.zip", "when": "2026-08-18T00:07:51Z", "os": "windows", "locked": false},
            {"name": "backup-2.zip", "when": "2026-10-11T18:29:41Z", "os": "linux", "locked": true, "comment": "boss"},
            {"when": "2026-10-12T00:00:00Z"}
        ]}}});
        let b = parse_backups(&v, "Elden Ring");
        assert_eq!(b.iter().map(|b| b.id.as_str()).collect::<Vec<_>>(), ["backup-2.zip", "backup-1.zip"]);
        assert!(b[0].locked);
        assert_eq!(b[0].comment.as_deref(), Some("boss"));
        assert_eq!(b[1].os.as_deref(), Some("windows"));
        assert!(parse_backups(&v, "Other").is_empty());
    }

    #[test]
    fn backup_sizes() {
        let d = tempfile::tempdir().unwrap();
        fs::write(d.path().join("backup-1.zip"), vec![0u8; 10]).unwrap();
        fs::create_dir_all(d.path().join("backup-2/drive-0/x")).unwrap();
        fs::write(d.path().join("backup-2/drive-0/x/a"), vec![0u8; 3]).unwrap();
        fs::create_dir_all(d.path().join("drive-C/y")).unwrap();
        fs::write(d.path().join("drive-C/y/b"), vec![0u8; 4]).unwrap();
        assert_eq!(backup_size(d.path(), "backup-1.zip"), Some(10));
        assert_eq!(backup_size(d.path(), "backup-2"), Some(3));
        assert_eq!(backup_size(d.path(), "."), Some(4));
        assert_eq!(backup_size(d.path(), "missing.zip"), None);
    }
}
