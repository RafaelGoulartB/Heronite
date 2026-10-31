//! Pure rules of a restore: the redirects that retarget a backup to the
//! game's current prefix (Linux) or folders (Windows), what each file of the
//! backup is, and Ludusavi's `restore --api` reply.
//!
//! Ludusavi keeps every file under the absolute path it was backed up from:
//! `/old/compatdata/1245620/pfx/drive_c/users/steamuser/...` on Linux,
//! `C:/Users/<name>/...` on Windows. Redirects are plain path-prefix
//! substitutions applied top to bottom, each seeing the result of the ones
//! before, so the specific ones come first and `C:` last.
//!
//! On Linux a backup lands in the game's Wine prefix. On Windows games run
//! natively, so files go back to the paths they were backed up from, with
//! only what moved redirected (another Steam library, another Windows user,
//! a Wine prefix of a backup made on Linux).

use std::path::Path;

use serde_json::Value;

use super::target::RestoreTarget;

/// A `restore` redirect of Ludusavi's config.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Redirect {
    pub source: String,
    pub target: String,
}

/// Ludusavi's `change` of a file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Change {
    /// Not on disk yet.
    New,
    /// On disk with other content: the restore overwrites it.
    Different,
    Same,
    Unknown,
}

impl Change {
    fn parse(v: Option<&str>) -> Self {
        match v {
            Some("New") => Change::New,
            Some("Different") => Change::Different,
            Some("Same") => Change::Same,
            _ => Change::Unknown,
        }
    }
}

/// Where a file of the backup lands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileKind {
    /// In the active prefix, the game's folder or Steam's `userdata`; on
    /// Windows any path of this PC.
    Save,
    /// A whole `*.reg` file of the prefix. Ludusavi backs up the full Wine
    /// registry when the game keeps settings there, so restoring it would
    /// also roll back every other program of the prefix.
    Registry,
    /// Somewhere else on this PC (an old Steam library, another prefix).
    Outside,
    /// A Windows path with no place on Linux (another drive letter); on
    /// Windows a Linux path or a drive this PC does not have.
    Unmapped,
}

/// One file of a reply to `restore --api` (preview or real).
#[derive(Debug, Clone, PartialEq)]
pub struct ReplyFile {
    /// Where it is (or would be) written.
    pub target: String,
    /// The path it was backed up from (the target when not redirected).
    pub original: String,
    pub bytes: u64,
    pub change: Change,
    /// Left out through `restore.toggledPaths`.
    pub ignored: bool,
    pub failed: bool,
    pub error: Option<String>,
}

/// `C:/...` (any drive letter).
pub fn is_windows_path(p: &str) -> bool {
    let b = p.as_bytes();
    b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':' && (b.len() == 2 || b[2] == b'/' || b[2] == b'\\')
}

fn path_str(p: &Path) -> String {
    let s = p.to_string_lossy();
    let t = s.trim_end_matches('/');
    if t.is_empty() {
        "/".into()
    } else {
        t.to_string()
    }
}

/// The prefix a Linux path lives in (the folder holding `drive_c`), or the
/// folder of a top-level `system.reg` / `user.reg` / `userdef.reg`.
fn wine_prefix_of(p: &str) -> Option<&str> {
    if let Some(i) = p.find("/drive_c/") {
        return Some(&p[..i]);
    }
    let (dir, name) = p.rsplit_once('/')?;
    matches!(name, "system.reg" | "user.reg" | "userdef.reg").then_some(dir).filter(|d| !d.is_empty())
}

/// `<base>/steamapps/common/<dir>` of a path (either slash style).
fn steam_common_dir(p: &str) -> Option<(&str, &str)> {
    let lower = p.to_ascii_lowercase();
    let i = lower.find("/steamapps/common/")?;
    let start = i + "/steamapps/common/".len();
    let end = p[start..].find('/').map(|j| start + j).unwrap_or(p.len());
    Some((&p[..end], &p[start..end]))
}

/// `X:/.../Steam/userdata` of a Windows path.
fn windows_steam_userdata(p: &str) -> Option<&str> {
    let lower = p.to_ascii_lowercase();
    let i = lower.find("/steam/userdata")?;
    let end = i + "/steam/userdata".len();
    (p.len() == end || p.as_bytes()[end] == b'/').then(|| &p[..end])
}

fn push(out: &mut Vec<Redirect>, source: &str, target: &str) {
    if source != target && !out.iter().any(|r| r.source == source) {
        out.push(Redirect { source: source.to_string(), target: target.to_string() });
    }
}

/// The Wine prefix most files of a Linux backup are in (the first on ties).
fn main_wine_prefix(originals: &[String]) -> Option<&str> {
    let mut counts: Vec<(&str, usize)> = Vec::new();
    for o in originals.iter().filter(|o| !is_windows_path(o)) {
        if let Some(src) = wine_prefix_of(o) {
            match counts.iter_mut().find(|(s, _)| *s == src) {
                Some((_, n)) => *n += 1,
                None => counts.push((src, 1)),
            }
        }
    }
    let primary = counts.iter().fold(None::<(&str, usize)>, |best, &(s, n)| match best {
        Some((_, b)) if b >= n => best,
        _ => Some((s, n)),
    });
    primary.map(|(s, _)| s)
}

/// Redirects that put the files of a backup (`originals`, as Ludusavi stored
/// them) into `target`: [`linux_redirects`] on Linux, [`windows_redirects`]
/// on Windows.
pub fn redirects(originals: &[String], target: &RestoreTarget) -> Vec<Redirect> {
    if cfg!(windows) {
        windows_redirects(originals, target)
    } else {
        linux_redirects(originals, target)
    }
}

/// Linux: the files land in the game's prefix. Only the prefix holding most
/// files is moved: a stale copy from another Steam library stays where it
/// was (and is left out by default as [`FileKind::Outside`]) instead of
/// colliding with the real one.
fn linux_redirects(originals: &[String], target: &RestoreTarget) -> Vec<Redirect> {
    let mut out = Vec::new();

    if let Some(dir) = &target.install_dir {
        let want = dir.file_name().map(|n| n.to_string_lossy().to_lowercase());
        for o in originals {
            if let (Some((base, name)), Some(want)) = (steam_common_dir(o), want.as_deref()) {
                if name.to_lowercase() == want {
                    push(&mut out, base, &path_str(dir));
                }
            }
        }
    }
    if let Some(userdata) = &target.steam_userdata {
        for o in originals.iter().filter(|o| is_windows_path(o)) {
            if let Some(src) = windows_steam_userdata(o) {
                push(&mut out, src, &path_str(userdata));
            }
        }
    }

    let Some(prefix) = &target.prefix else { return out };
    let tp = path_str(prefix);
    let tu = target.user.as_deref().unwrap_or("steamuser");
    let user_dir = |u: &str| format!("{tp}/drive_c/users/{}", if u == "Public" { "Public" } else { tu });

    if let Some(src) = main_wine_prefix(originals).filter(|src| *src != tp) {
        let users_root = format!("{src}/drive_c/users/");
        for o in originals {
            if let Some(user) = o.strip_prefix(&users_root).and_then(|rest| rest.split('/').next()) {
                if user != tu && user != "Public" && !user.is_empty() {
                    push(&mut out, &format!("{users_root}{user}"), &user_dir(user));
                }
            }
        }
        push(&mut out, src, &tp);
    }

    let windows: Vec<&String> = originals.iter().filter(|o| is_windows_path(o)).collect();
    for o in &windows {
        let drive = &o[..2];
        let users = format!("{drive}/Users/");
        if let Some(user) = o.strip_prefix(&users).and_then(|rest| rest.split('/').next()).filter(|u| !u.is_empty()) {
            push(&mut out, &format!("{users}{user}"), &user_dir(user));
        }
    }
    if windows.iter().any(|o| o.starts_with("C:")) {
        push(&mut out, "C:", &format!("{tp}/drive_c"));
    }
    out
}

/// A Windows path as Ludusavi writes it: `/` separators, none at the end.
fn slashes(p: &Path) -> String {
    let s = p.to_string_lossy().replace('\\', "/");
    s.trim_end_matches('/').to_string()
}

/// What follows `dir/` in `path`, case ignored (Windows paths).
fn under_ci<'a>(path: &'a str, dir: &str) -> Option<&'a str> {
    let head = path.get(..dir.len())?;
    let rest = path.get(dir.len()..)?;
    (head.eq_ignore_ascii_case(dir)).then_some(rest)?.strip_prefix('/')
}

/// [`push`] for Windows, where a redirect to the same folder in other case
/// is none.
fn push_ci(out: &mut Vec<Redirect>, source: &str, target: &str) {
    if !source.eq_ignore_ascii_case(target) && !out.iter().any(|r| r.source.eq_ignore_ascii_case(source)) {
        out.push(Redirect { source: source.to_string(), target: target.to_string() });
    }
}

/// User folders that are no one's (`C:/Users/Public`...).
fn shared_user(name: &str) -> bool {
    ["Public", "Default", "Default User", "All Users"].iter().any(|n| n.eq_ignore_ascii_case(name))
}

/// Windows: a native game writes its saves where they were backed up from.
/// What moved is put back on this PC: the game's folder (another Steam
/// library), Steam's `userdata`, the folder of another Windows user
/// (another PC) and, for a backup made on Linux, the Wine prefix holding
/// most files (its users become this user, the rest of `drive_c` the system
/// drive). Linux paths outside it have no place here.
fn windows_redirects(originals: &[String], target: &RestoreTarget) -> Vec<Redirect> {
    let mut out = Vec::new();
    if let Some(dir) = &target.install_dir {
        let want = dir.file_name().map(|n| n.to_string_lossy().to_lowercase());
        for o in originals {
            if let (Some((base, name)), Some(want)) = (steam_common_dir(o), want.as_deref()) {
                if name.to_lowercase() == want {
                    push_ci(&mut out, base, &slashes(dir));
                }
            }
        }
    }
    if let Some(userdata) = &target.steam_userdata {
        for o in originals {
            if let Some(src) = windows_steam_userdata(o) {
                push_ci(&mut out, src, &slashes(userdata));
            }
        }
    }
    let Some(home) = target.user_home.as_deref().map(slashes) else { return out };
    let (users, me) = home.rsplit_once('/').unwrap_or((home.as_str(), ""));
    let drive = home.get(..2).filter(|d| is_windows_path(d)).unwrap_or("C:");

    if let Some(src) = main_wine_prefix(originals) {
        let users_root = format!("{src}/drive_c/users/");
        for o in originals {
            if let Some(user) = o.strip_prefix(&users_root).and_then(|rest| rest.split('/').next()).filter(|u| !u.is_empty()) {
                let to = if shared_user(user) { format!("{users}/{user}") } else { home.clone() };
                push_ci(&mut out, &format!("{users_root}{user}"), &to);
            }
        }
        push_ci(&mut out, &format!("{src}/drive_c"), drive);
    }

    for o in originals.iter().filter(|o| is_windows_path(o)) {
        let Some(user) = under_ci(o, users).and_then(|rest| rest.split('/').next()).filter(|u| !u.is_empty()) else { continue };
        if !user.eq_ignore_ascii_case(me) && !shared_user(user) {
            push_ci(&mut out, &o[..users.len() + 1 + user.len()], &home);
        }
    }
    out
}

/// What a restored file is, from where it lands: [`linux_classify`] on
/// Linux, [`windows_classify`] on Windows.
pub fn classify(target_path: &str, target: &RestoreTarget) -> FileKind {
    if cfg!(windows) {
        windows_classify(target_path, &|drive| Path::new(&format!("{drive}\\")).exists())
    } else {
        linux_classify(target_path, target)
    }
}

/// Windows: a native game keeps its own paths, so every Windows path is a
/// save, unless its drive is not on this PC. Linux paths (a Linux game, the
/// Wine registry, a stale prefix) have no place here.
fn windows_classify(target_path: &str, drive_exists: &dyn Fn(&str) -> bool) -> FileKind {
    if is_windows_path(target_path) && drive_exists(&target_path[..2]) {
        FileKind::Save
    } else {
        FileKind::Unmapped
    }
}

/// Linux: what is in the game's prefix or folders.
fn linux_classify(target_path: &str, target: &RestoreTarget) -> FileKind {
    if is_windows_path(target_path) {
        return FileKind::Unmapped;
    }
    let p = Path::new(target_path);
    if let Some(prefix) = &target.prefix {
        if p.starts_with(prefix) {
            let reg = p.extension().is_some_and(|e| e.eq_ignore_ascii_case("reg"));
            return if reg && p.parent() == Some(prefix.as_path()) { FileKind::Registry } else { FileKind::Save };
        }
    }
    let known = [&target.install_dir, &target.steam_userdata];
    if known.iter().any(|d| d.as_ref().is_some_and(|d| p.starts_with(d))) {
        return FileKind::Save;
    }
    // A native game keeps its own paths, but never writes into some old prefix.
    if target.prefix.is_none() && !target.needs_prefix && wine_prefix_of(target_path).is_none() {
        FileKind::Save
    } else {
        FileKind::Outside
    }
}

/// Ludusavi's `--api` reply for one game: its files, or why there are none.
pub fn parse_reply(payload: &Value, game: &str) -> Result<Vec<ReplyFile>, String> {
    let games = payload.get("games").and_then(Value::as_object);
    let Some(entry) = games.and_then(|g| g.get(game).or_else(|| (g.len() == 1).then(|| g.values().next()).flatten())) else {
        let unknown = payload.pointer("/errors/unknownGames").and_then(Value::as_array).is_some_and(|a| !a.is_empty());
        return Err(if unknown { "unknown-game".into() } else { "no-reply".into() });
    };
    let Some(files) = entry.get("files").and_then(Value::as_object) else { return Ok(Vec::new()) };
    let mut out: Vec<ReplyFile> = files
        .iter()
        .map(|(path, f)| ReplyFile {
            target: path.clone(),
            original: f.get("originalPath").and_then(Value::as_str).unwrap_or(path).to_string(),
            bytes: f.get("bytes").and_then(Value::as_u64).unwrap_or(0),
            change: Change::parse(f.get("change").and_then(Value::as_str)),
            ignored: f.get("ignored").and_then(Value::as_bool).unwrap_or(false),
            failed: f.get("failed").and_then(Value::as_bool).unwrap_or(false),
            error: f.get("error").and_then(|e| e.get("message").or(Some(e))).and_then(Value::as_str).map(str::to_string),
        })
        .collect();
    out.sort_by(|a, b| a.target.cmp(&b.target));
    Ok(out)
}

/// A file of a [`super::RestorePlan`].
#[derive(Debug, Clone, PartialEq)]
pub struct PlannedFile {
    pub target: String,
    pub original: String,
    pub bytes: u64,
    pub change: Change,
    pub kind: FileKind,
    /// Restored when the plan is applied. Only saves start selected.
    pub selected: bool,
}

/// The files of a redirected preview, saves first.
pub fn planned_files(reply: Vec<ReplyFile>, target: &RestoreTarget) -> Vec<PlannedFile> {
    let mut files: Vec<PlannedFile> = reply
        .into_iter()
        .map(|f| {
            let kind = classify(&f.target, target);
            PlannedFile { selected: kind == FileKind::Save, kind, target: f.target, original: f.original, bytes: f.bytes, change: f.change }
        })
        .collect();
    let rank = |k: FileKind| match k {
        FileKind::Save => 0,
        FileKind::Registry => 1,
        FileKind::Outside => 2,
        FileKind::Unmapped => 3,
    };
    files.sort_by(|a, b| rank(a.kind).cmp(&rank(b.kind)).then_with(|| a.target.cmp(&b.target)));
    files
}

#[cfg(test)]
mod tests {
    use super::*;

    fn steam_target() -> RestoreTarget {
        RestoreTarget {
            prefix: Some("/lib/steamapps/compatdata/1245620/pfx".into()),
            user: Some("steamuser".into()),
            install_dir: Some("/lib/steamapps/common/ELDEN RING".into()),
            steam_userdata: Some("/home/u/.local/share/Steam/userdata".into()),
            user_home: None,
            needs_prefix: true,
        }
    }

    /// A Steam game on Windows (forward slashes, so the rules can be tested
    /// on every system).
    fn windows_target() -> RestoreTarget {
        RestoreTarget {
            install_dir: Some("D:/SteamLibrary/steamapps/common/Hollow Knight".into()),
            steam_userdata: Some("C:/Program Files (x86)/Steam/userdata".into()),
            user_home: Some("C:/Users/rafael".into()),
            ..Default::default()
        }
    }

    fn r(source: &str, target: &str) -> Redirect {
        Redirect { source: source.into(), target: target.into() }
    }

    #[test]
    fn same_prefix_needs_no_redirect() {
        let originals =
            vec!["/lib/steamapps/compatdata/1245620/pfx/drive_c/users/steamuser/AppData/Roaming/EldenRing/ER0000.sl2".to_string()];
        assert!(linux_redirects(&originals, &steam_target()).is_empty());
    }

    #[test]
    fn moves_the_main_prefix_and_leaves_a_stale_one() {
        let originals: Vec<String> = [
            "/old/compatdata/1245620/pfx/drive_c/users/steamuser/AppData/Roaming/EldenRing/ER0000.sl2",
            "/old/compatdata/1245620/pfx/drive_c/users/steamuser/AppData/Roaming/EldenRing/ER0000.sl2.bak",
            "/home/u/.local/share/Steam/steamapps/compatdata/1245620/pfx/drive_c/users/steamuser/Application Data/x.vdf",
        ]
        .map(String::from)
        .into();
        assert_eq!(
            linux_redirects(&originals, &steam_target()),
            [r("/old/compatdata/1245620/pfx", "/lib/steamapps/compatdata/1245620/pfx")]
        );
    }

    #[test]
    fn renames_the_wine_user() {
        let t = RestoreTarget { prefix: Some("/p/new".into()), user: Some("rafa".into()), needs_prefix: true, ..Default::default() };
        let originals: Vec<String> =
            ["/p/old/drive_c/users/steamuser/Documents/G/s.sav", "/p/old/drive_c/users/Public/Documents/G/c.ini", "/p/old/user.reg"]
                .map(String::from)
                .into();
        assert_eq!(
            linux_redirects(&originals, &t),
            [r("/p/old/drive_c/users/steamuser", "/p/new/drive_c/users/rafa"), r("/p/old", "/p/new")]
        );
    }

    #[test]
    fn windows_backups_land_in_the_prefix_steam_and_the_game_folder() {
        let originals: Vec<String> = [
            "C:/Users/rafael/AppData/Roaming/EldenRing/ER0000.sl2",
            "C:/Users/Public/Documents/x.ini",
            "C:/Program Files (x86)/Steam/userdata/314512607/1245620/remote/s.dat",
            "D:/SteamLibrary/steamapps/common/ELDEN RING/Game/save.ini",
            "C:/Games/Other/save.dat",
            "E:/elsewhere/file",
        ]
        .map(String::from)
        .into();
        let t = steam_target();
        assert_eq!(
            linux_redirects(&originals, &t),
            [
                r("D:/SteamLibrary/steamapps/common/ELDEN RING", "/lib/steamapps/common/ELDEN RING"),
                r("C:/Program Files (x86)/Steam/userdata", "/home/u/.local/share/Steam/userdata"),
                r("C:/Users/rafael", "/lib/steamapps/compatdata/1245620/pfx/drive_c/users/steamuser"),
                r("C:/Users/Public", "/lib/steamapps/compatdata/1245620/pfx/drive_c/users/Public"),
                r("C:", "/lib/steamapps/compatdata/1245620/pfx/drive_c"),
            ]
        );
    }

    #[test]
    fn native_games_keep_their_paths() {
        let t = RestoreTarget::default();
        let originals = vec!["/home/u/.config/unity3d/Team Cherry/Hollow Knight/user1.dat".to_string()];
        assert!(linux_redirects(&originals, &t).is_empty());
        assert_eq!(linux_classify(&originals[0], &t), FileKind::Save);
        assert_eq!(linux_classify("C:/Users/x/save", &t), FileKind::Unmapped);
        assert_eq!(linux_classify("/old/compatdata/1/pfx/drive_c/users/steamuser/s", &t), FileKind::Outside);
        assert_eq!(linux_classify("/old/prefix/user.reg", &t), FileKind::Outside);
    }

    #[test]
    fn classifies_by_destination() {
        let t = steam_target();
        let pfx = "/lib/steamapps/compatdata/1245620/pfx";
        assert_eq!(linux_classify(&format!("{pfx}/drive_c/users/steamuser/AppData/x"), &t), FileKind::Save);
        assert_eq!(linux_classify(&format!("{pfx}/user.reg"), &t), FileKind::Registry);
        assert_eq!(linux_classify(&format!("{pfx}/drive_c/windows/x.reg"), &t), FileKind::Save);
        assert_eq!(linux_classify("/lib/steamapps/common/ELDEN RING/save.ini", &t), FileKind::Save);
        assert_eq!(linux_classify("/home/u/.local/share/Steam/userdata/1/2/remote/a", &t), FileKind::Save);
        assert_eq!(linux_classify("/home/u/.local/share/Steam/steamapps/compatdata/1245620/pfx/x", &t), FileKind::Outside);
        assert_eq!(linux_classify("/lib/steamapps/compatdata/1245620/pfx2/x", &t), FileKind::Outside);
        assert_eq!(linux_classify("D:/x", &t), FileKind::Unmapped);
        let no_prefix_yet = RestoreTarget { needs_prefix: true, ..Default::default() };
        assert_eq!(linux_classify("/somewhere/save", &no_prefix_yet), FileKind::Outside);
    }

    #[test]
    fn parses_replies() {
        let reply = serde_json::json!({
            "overall": {"totalGames": 1},
            "games": {"Elden Ring": {"decision": "Processed", "files": {
                "/t/b": {"change": "Same", "bytes": 52},
                "/t/a": {"change": "New", "bytes": 10, "originalPath": "C:/a"},
                "/t/c": {"change": "Different", "bytes": 3, "ignored": true},
                "/t/d": {"change": "New", "bytes": 1, "failed": true, "error": {"message": "denied"}}
            }, "registry": {}}}
        });
        let files = parse_reply(&reply, "Elden Ring").unwrap();
        assert_eq!(files.iter().map(|f| f.target.as_str()).collect::<Vec<_>>(), ["/t/a", "/t/b", "/t/c", "/t/d"]);
        assert_eq!((files[0].original.as_str(), files[0].change, files[0].bytes), ("C:/a", Change::New, 10));
        assert_eq!(files[1].original, "/t/b");
        assert!(files[2].ignored && !files[2].failed);
        assert_eq!((files[3].failed, files[3].error.as_deref()), (true, Some("denied")));
        let unknown = serde_json::json!({"errors": {"unknownGames": ["X"]}, "games": {}});
        assert_eq!(parse_reply(&unknown, "X").unwrap_err(), "unknown-game");
        assert_eq!(parse_reply(&serde_json::json!({}), "X").unwrap_err(), "no-reply");
    }

    #[test]
    fn windows_backups_from_this_pc_need_no_redirect() {
        let originals: Vec<String> = [
            "C:/Users/rafael/AppData/LocalLow/Team Cherry/Hollow Knight/user1.dat",
            "c:/users/RAFAEL/AppData/LocalLow/Team Cherry/Hollow Knight/user2.dat",
            "C:/Users/Public/Documents/x.ini",
            "D:/SteamLibrary/steamapps/common/Hollow Knight/save.ini",
            "C:/Program Files (x86)/Steam/userdata/1/367520/remote/a",
        ]
        .map(String::from)
        .into();
        assert!(windows_redirects(&originals, &windows_target()).is_empty());
        assert!(windows_redirects(&originals, &RestoreTarget::default()).is_empty());
    }

    #[test]
    fn windows_backups_from_another_pc_land_in_this_users_folders() {
        let originals: Vec<String> = [
            "C:/Users/bob/AppData/LocalLow/Team Cherry/Hollow Knight/user1.dat",
            "C:/Users/bob/Documents/My Games/x.cfg",
            "C:/Users/Public/Documents/x.ini",
            "C:/Users/Default/x",
            "E:/Steam/steamapps/common/Hollow Knight/save.ini",
            "E:/Steam/userdata/1/367520/remote/a",
            "C:/Games/Other/save.dat",
        ]
        .map(String::from)
        .into();
        assert_eq!(
            windows_redirects(&originals, &windows_target()),
            [
                r("E:/Steam/steamapps/common/Hollow Knight", "D:/SteamLibrary/steamapps/common/Hollow Knight"),
                r("E:/Steam/userdata", "C:/Program Files (x86)/Steam/userdata"),
                r("C:/Users/bob", "C:/Users/rafael"),
            ]
        );
        // The game folder and Steam come in with either slash.
        let backslashes = RestoreTarget {
            install_dir: Some(r"D:\Games\Hollow Knight".into()),
            steam_userdata: None,
            user_home: Some(r"C:\Users\rafael\".into()),
            ..Default::default()
        };
        let got = windows_redirects(&originals, &backslashes);
        assert_eq!(got.last(), Some(&r("C:/Users/bob", "C:/Users/rafael")));
        assert!(got.iter().all(|r| !r.target.contains('\\')), "{got:?}");
    }

    #[test]
    fn linux_backups_land_in_windows_folders() {
        let pfx = "/home/u/.local/share/Steam/steamapps/compatdata/367520/pfx";
        let originals: Vec<String> = [
            format!("{pfx}/drive_c/users/steamuser/AppData/LocalLow/Team Cherry/Hollow Knight/user1.dat"),
            format!("{pfx}/drive_c/users/steamuser/AppData/LocalLow/Team Cherry/Hollow Knight/user2.dat"),
            format!("{pfx}/drive_c/users/Public/Documents/x.ini"),
            format!("{pfx}/drive_c/ProgramData/HK/a.dat"),
            format!("{pfx}/user.reg"),
            "/home/u/.local/share/Steam/userdata/1/367520/remote/a".into(),
            "/home/u/.local/share/Steam/steamapps/common/Hollow Knight/save.ini".into(),
            "/old/compatdata/367520/pfx/drive_c/users/steamuser/stale.dat".into(),
            "/home/u/.config/unity3d/Team Cherry/Hollow Knight/linux.dat".into(),
        ]
        .into();
        assert_eq!(
            windows_redirects(&originals, &windows_target()),
            [
                r("/home/u/.local/share/Steam/steamapps/common/Hollow Knight", "D:/SteamLibrary/steamapps/common/Hollow Knight"),
                r("/home/u/.local/share/Steam/userdata", "C:/Program Files (x86)/Steam/userdata"),
                r(&format!("{pfx}/drive_c/users/steamuser"), "C:/Users/rafael"),
                r(&format!("{pfx}/drive_c/users/Public"), "C:/Users/Public"),
                r(&format!("{pfx}/drive_c"), "C:"),
            ]
        );
    }

    #[test]
    fn windows_restores_every_windows_path_of_this_pc() {
        let drives = |d: &str| d.eq_ignore_ascii_case("C:") || d == "D:";
        assert_eq!(windows_classify("C:/Users/rafael/AppData/x", &drives), FileKind::Save);
        assert_eq!(windows_classify(r"D:\Games\x", &drives), FileKind::Save);
        assert_eq!(windows_classify("c:/x", &drives), FileKind::Save);
        assert_eq!(windows_classify("E:/elsewhere/file", &drives), FileKind::Unmapped);
        assert_eq!(windows_classify("/home/u/.config/x", &drives), FileKind::Unmapped);
        assert_eq!(windows_classify("/p/pfx/user.reg", &drives), FileKind::Unmapped);
    }

    #[cfg(windows)]
    #[test]
    fn windows_plans_select_the_saves() {
        let file = |p: &str| ReplyFile {
            target: p.into(),
            original: p.into(),
            bytes: 1,
            change: Change::New,
            ignored: false,
            failed: false,
            error: None,
        };
        let plan = planned_files(vec![file("/p/pfx/user.reg"), file("C:/Users/u/s")], &RestoreTarget::default());
        assert_eq!(plan.iter().map(|f| (f.kind, f.selected)).collect::<Vec<_>>(), [(FileKind::Save, true), (FileKind::Unmapped, false)]);
    }

    // Linux's rules (`classify` is Windows' there).
    #[cfg(not(windows))]
    #[test]
    fn plans_select_only_saves() {
        let t = steam_target();
        let pfx = "/lib/steamapps/compatdata/1245620/pfx";
        let file = |p: &str| ReplyFile {
            target: p.into(),
            original: p.into(),
            bytes: 1,
            change: Change::New,
            ignored: false,
            failed: false,
            error: None,
        };
        let plan = planned_files(vec![file("/other/x"), file(&format!("{pfx}/system.reg")), file(&format!("{pfx}/drive_c/s"))], &t);
        assert_eq!(
            plan.iter().map(|f| (f.kind, f.selected)).collect::<Vec<_>>(),
            [(FileKind::Save, true), (FileKind::Registry, false), (FileKind::Outside, false)]
        );
    }
}
