//! A client's games folder under Wine: a folder out of its prefix where it
//! installs its games (`heronite_data::store_folders`, by the client's id),
//! chosen on its page. None is set at first, and the client keeps its own
//! folder, inside the prefix.
//!
//! * The client sees it through `Z:\` ([`windows_path`]); umu's container
//!   mounts it, and the folders the client had before, with the folders
//!   shown to Proton (`heronite_launch::shared_folders::add_store_folder`).
//! * The EA app and Ubisoft Connect keep their download folder in a settings
//!   file, which Heronite writes ([`apply`]) when the folder is chosen and
//!   before it opens the client ([`crate::run::open`]): the EA app's
//!   `machine.ini` (`machine.downloadinplacedir`), Ubisoft Connect's
//!   `settings.yaml` (`misc: game_installation_path`). Battle.net and the
//!   Rockstar Games Launcher ask where to install each game ([`sets_it`]).
//!
//! Games already installed stay where they are: the clients keep the folder
//! of each game.

use std::path::{Path, PathBuf};

use heronite_data::clients::{drive_c, prefix_of, Client};
use heronite_data::{game_config, store_folders, DataPaths};

use crate::files::read_text;
use crate::winpath::join_ci;
use crate::{user_dirs, Place};

const EA_MACHINE: &str = "ProgramData/EA Desktop/machine.ini";
const EA_USER_SETTINGS: &str = "AppData/Local/Electronic Arts/EA Desktop";
const EA_KEY: &str = "machine.downloadinplacedir";
const EA_USER_KEY: &str = "user.downloadinplacedir";
/// Where the EA app installs games by default.
const EA_DEFAULT: &str = r"C:\Program Files\EA Games\";

const UBISOFT_SETTINGS: &str = "AppData/Local/Ubisoft Game Launcher/settings.yaml";
const UBISOFT_KEY: &str = "game_installation_path";
/// Where Ubisoft Connect installs games by default.
const UBISOFT_DEFAULT: &str = "C:/Program Files (x86)/Ubisoft/Ubisoft Game Launcher/games/";

/// The games folder chosen for `client`, when one is.
pub fn of(paths: &DataPaths, client: Client, home: &str) -> Option<PathBuf> {
    let global = game_config::load_global(paths, home);
    store_folders::own(&global, client.id(), home).map(PathBuf::from)
}

/// Whether Heronite sets the folder in the client itself (the EA app,
/// Ubisoft Connect); Battle.net and the Rockstar Games Launcher ask for it
/// when a game is installed.
pub fn sets_it(client: Client) -> bool {
    matches!(client, Client::Ea | Client::Ubisoft)
}

/// `folder` as the client sees it: through `Z:`, which Wine makes the
/// whole filesystem (`Z:\data\Games\EA`).
pub fn windows_path(folder: &Path) -> String {
    let parts: Vec<String> = folder
        .components()
        .filter(|c| matches!(c, std::path::Component::Normal(_)))
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect();
    format!("Z:\\{}", parts.join("\\"))
}

/// Writes the games folder chosen for `client` into its settings (the EA app,
/// Ubisoft Connect), so the games it installs from now on go there. Nothing
/// happens without a folder, a prefix, or while the folder is missing (an
/// unplugged drive): the client keeps the folder it had.
pub fn apply(paths: &DataPaths, client: Client, home: &str) {
    let (Some(prefix), Some(folder)) = (prefix_of(paths, client, home), of(paths, client, home)) else { return };
    // Not before the setup is done: it writes the client's first settings
    // where there are none (`setup::write_client_settings`).
    if !sets_it(client) || crate::find_exe(client, &Place::Prefix(prefix.clone())).is_none() {
        return;
    }
    if !folder.is_dir() {
        log::warn!("{}: the games folder {} is missing, not set", client.name(), folder.display());
        return;
    }
    let win = windows_path(&folder);
    write(client, &prefix, &win, None);
}

/// After the games folder `old` of `client` was cleared: the client's own
/// folder again, where its settings still name `old` (not where the user
/// changed it in the client since).
pub fn release(paths: &DataPaths, client: Client, home: &str, old: &Path) {
    let Some(prefix) = prefix_of(paths, client, home) else { return };
    let default = match client {
        Client::Ea => EA_DEFAULT,
        Client::Ubisoft => UBISOFT_DEFAULT,
        Client::BattleNet | Client::Rockstar => return,
    };
    let was = windows_path(old);
    write(client, &prefix, default, Some(&was));
}

/// Sets the client's folder to `win`; with `only_if`, only where it is that
/// folder now.
fn write(client: Client, prefix: &Path, win: &str, only_if: Option<&str>) {
    let mut files: Vec<(PathBuf, &str)> = Vec::new();
    match client {
        Client::Ea => {
            files.push((join_ci(&drive_c(prefix), EA_MACHINE), EA_KEY));
            // A user's own setting wins over the machine's: changed where it is there.
            for user in user_dirs(prefix) {
                let dir = join_ci(&user, EA_USER_SETTINGS);
                for ini in std::fs::read_dir(&dir).into_iter().flatten().flatten().map(|e| e.path()) {
                    let name = ini.file_name().map(|n| n.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
                    if name.starts_with("user_")
                        && name.ends_with(".ini")
                        && read_text(&ini).is_some_and(|t| ini_get(&t, EA_USER_KEY).is_some())
                    {
                        files.push((ini, EA_USER_KEY));
                    }
                }
            }
        }
        Client::Ubisoft => files.extend(user_dirs(prefix).into_iter().map(|u| (join_ci(&u, UBISOFT_SETTINGS), UBISOFT_KEY))),
        Client::BattleNet | Client::Rockstar => {}
    }
    for (file, key) in files {
        let text = read_text(&file).unwrap_or_default();
        let (now, changed) = match client {
            Client::Ea => {
                let value = format!("{}\\", win.trim_end_matches('\\'));
                if only_if.is_some_and(|w| !ini_get(&text, key).is_some_and(|v| same(&v, w))) {
                    continue;
                }
                let changed = ini_set(&text, key, &value);
                (value, changed)
            }
            _ => {
                let value = format!("{}/", win.replace('\\', "/").trim_end_matches('/'));
                if only_if.is_some_and(|w| !yaml_misc_get(&text).is_some_and(|v| same(&v, w))) {
                    continue;
                }
                let changed = yaml_misc_set(&text, &value);
                (value, changed)
            }
        };
        if changed == text {
            continue;
        }
        let written = file.parent().map_or(Ok(()), std::fs::create_dir_all).and_then(|_| std::fs::write(&file, changed));
        match written {
            Ok(()) => log::info!("{}: games go to {now} ({})", client.name(), file.display()),
            Err(e) => log::warn!("{}: {e}", file.display()),
        }
    }
}

/// Whether two Windows paths name the same folder (case, slashes and a
/// final separator aside).
fn same(a: &str, b: &str) -> bool {
    let norm = |s: &str| s.trim().trim_matches('"').replace('\\', "/").trim_end_matches('/').to_lowercase();
    norm(a) == norm(b)
}

/// The line ending of `text` (`\r\n` when it has one).
fn eol(text: &str) -> &'static str {
    if text.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    }
}

fn ini_get(text: &str, key: &str) -> Option<String> {
    text.lines().find_map(|l| {
        let (k, v) = l.split_once('=')?;
        k.trim().eq_ignore_ascii_case(key).then(|| v.trim().to_string())
    })
}

/// `text` with `key=value` (replaced where the key is, else added at the end).
fn ini_set(text: &str, key: &str, value: &str) -> String {
    let nl = eol(text);
    let line = format!("{key}={value}");
    let mut found = false;
    let mut lines: Vec<String> = text
        .lines()
        .map(|l| match l.split_once('=') {
            Some((k, _)) if k.trim().eq_ignore_ascii_case(key) => {
                found = true;
                line.clone()
            }
            _ => l.to_string(),
        })
        .collect();
    if !found {
        lines.push(line);
    }
    let mut out = lines.join(nl);
    out.push_str(nl);
    out
}

/// The `misc:` section's lines of a `settings.yaml`: `(index of misc:, end)`.
fn misc_section(lines: &[&str]) -> Option<(usize, usize)> {
    let start = lines.iter().position(|l| l.trim_end() == "misc:")?;
    let end = lines[start + 1..].iter().position(|l| !l.is_empty() && !l.starts_with(' ')).map_or(lines.len(), |i| start + 1 + i);
    Some((start, end))
}

fn yaml_misc_get(text: &str) -> Option<String> {
    let lines: Vec<&str> = text.lines().collect();
    let (start, end) = misc_section(&lines)?;
    lines[start + 1..end].iter().find_map(|l| {
        let v = l.trim_start().strip_prefix(UBISOFT_KEY)?.trim_start().strip_prefix(':')?.trim();
        Some(v.trim_matches('\'').trim_matches('"').replace("''", "'"))
    })
}

/// A YAML value: plain where it can be, else in single quotes.
fn yaml_value(v: &str) -> String {
    let plain = !v.is_empty()
        && !v.contains(": ")
        && !v.contains(" #")
        && !v.starts_with(|c: char| c.is_whitespace() || "'\"!&*%@`|>{[,?-#".contains(c))
        && !v.ends_with(char::is_whitespace);
    if plain {
        v.to_string()
    } else {
        format!("'{}'", v.replace('\'', "''"))
    }
}

/// `text` with `misc: game_installation_path:` set to `value`.
fn yaml_misc_set(text: &str, value: &str) -> String {
    let nl = eol(text);
    let line = format!("  {UBISOFT_KEY}: {}", yaml_value(value));
    let mut lines: Vec<String> = text.lines().map(str::to_string).collect();
    let refs: Vec<&str> = text.lines().collect();
    match misc_section(&refs) {
        Some((start, end)) => {
            match (start + 1..end)
                .find(|i| refs[*i].trim_start().strip_prefix(UBISOFT_KEY).is_some_and(|r| r.trim_start().starts_with(':')))
            {
                Some(i) => lines[i] = line,
                None => lines.insert(start + 1, line),
            }
        }
        None => {
            lines.push("misc:".into());
            lines.push(line);
        }
    }
    let mut out = lines.join(nl);
    out.push_str(nl);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A Proton prefix of `client` where it is installed, set up in `paths`.
    #[cfg(unix)]
    fn prefix(paths: &DataPaths, client: Client, root: &Path) -> PathBuf {
        let prefix = root.join(client.prefix_folder());
        std::fs::create_dir_all(prefix.join("pfx/drive_c/users/steamuser")).unwrap();
        // Installed: its program is there.
        let exe = prefix.join("pfx/drive_c").join(client.exe_candidates()[0]);
        std::fs::create_dir_all(exe.parent().unwrap()).unwrap();
        std::fs::write(exe, "MZ").unwrap();
        game_config::set_game_setting(paths, client.config_app(), "/home/u", "winePrefix", serde_json::json!(prefix)).unwrap();
        prefix
    }

    #[test]
    fn the_folder_is_seen_through_z() {
        assert_eq!(windows_path(Path::new("/data/Games/EA")), r"Z:\data\Games\EA");
        assert_eq!(windows_path(Path::new("/data/Games/EA/")), r"Z:\data\Games\EA");
        assert_eq!(windows_path(Path::new("/")), r"Z:\");
    }

    #[test]
    fn ini_values_are_replaced_or_added() {
        let text = "machine.other=1\r\nMachine.DownloadInPlaceDir=C:\\Program Files\\EA Games\\\r\n";
        let out = ini_set(text, EA_KEY, r"Z:\data\EA\");
        assert_eq!(out, "machine.other=1\r\nmachine.downloadinplacedir=Z:\\data\\EA\\\r\n");
        assert_eq!(ini_get(&out, EA_KEY).as_deref(), Some(r"Z:\data\EA\"));
        assert_eq!(ini_set("", EA_KEY, "Z:\\x\\"), "machine.downloadinplacedir=Z:\\x\\\n");
    }

    #[test]
    fn the_ubisoft_folder_is_set_in_misc() {
        let text = "language:\r\n  code: en-US\r\nmisc:\r\n  enable_auto_wall_posts: true\r\n  game_installation_path: C:/Program Files (x86)/Ubisoft/Ubisoft Game Launcher/games/\r\n  installer_cache_path: C:/ProgramData/x/\r\noverlay:\r\n  enabled: false\r\n";
        let out = yaml_misc_set(text, "Z:/data/Games/Ubisoft/");
        assert_eq!(
            out,
            "language:\r\n  code: en-US\r\nmisc:\r\n  enable_auto_wall_posts: true\r\n  game_installation_path: Z:/data/Games/Ubisoft/\r\n  installer_cache_path: C:/ProgramData/x/\r\noverlay:\r\n  enabled: false\r\n"
        );
        assert_eq!(yaml_misc_get(&out).as_deref(), Some("Z:/data/Games/Ubisoft/"));
        // The key is only looked for in `misc`.
        let other = "overlay:\n  game_installation_path: x\n";
        assert_eq!(yaml_misc_set(other, "Z:/g/"), "overlay:\n  game_installation_path: x\nmisc:\n  game_installation_path: Z:/g/\n");
        let no_key = "misc:\n  enable_auto_wall_posts: true\nuser:\n  closebehavior: CloseBehavior_Close\n";
        assert_eq!(
            yaml_misc_set(no_key, "Z:/g/"),
            "misc:\n  game_installation_path: Z:/g/\n  enable_auto_wall_posts: true\nuser:\n  closebehavior: CloseBehavior_Close\n"
        );
        // A path YAML would misread is quoted.
        let odd = yaml_misc_set("", "Z:/a: b/it's/");
        assert_eq!(odd, "misc:\n  game_installation_path: 'Z:/a: b/it''s/'\n");
        assert_eq!(yaml_misc_get(&odd).as_deref(), Some("Z:/a: b/it's/"));
    }

    #[cfg(unix)]
    #[test]
    fn the_ea_app_and_ubisoft_connect_get_the_folder_and_give_it_back() {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path().join("data"));
        let games = dir.path().join("Games/EA");
        std::fs::create_dir_all(&games).unwrap();
        let ea = prefix(&paths, Client::Ea, dir.path());
        let machine = ea.join("pfx/drive_c").join(EA_MACHINE);
        std::fs::create_dir_all(machine.parent().unwrap()).unwrap();
        std::fs::write(&machine, "machine.downloadinplacedir=C:\\Program Files\\EA Games\\\nmachine.other=1\n").unwrap();
        let user_dir = ea.join("pfx/drive_c/users/steamuser").join(EA_USER_SETTINGS);
        std::fs::create_dir_all(&user_dir).unwrap();
        std::fs::write(user_dir.join("user_1.ini"), "user.downloadinplacedir=C:\\Mine\\\n").unwrap();
        std::fs::write(user_dir.join("user_2.ini"), "user.other=1\n").unwrap();

        // Nothing chosen: nothing changes.
        apply(&paths, Client::Ea, "/home/u");
        assert!(std::fs::read_to_string(&machine).unwrap().starts_with("machine.downloadinplacedir=C:\\Program Files"));

        store_folders::set(&paths, "/home/u", Client::Ea.id(), Some(games.to_str().unwrap())).unwrap();
        apply(&paths, Client::Ea, "/home/u");
        let win = format!("{}\\", windows_path(&games));
        assert!(win.starts_with(r"Z:\"));
        assert_eq!(ini_get(&std::fs::read_to_string(&machine).unwrap(), EA_KEY), Some(win.clone()));
        assert!(std::fs::read_to_string(&machine).unwrap().contains("machine.other=1"));
        assert_eq!(ini_get(&std::fs::read_to_string(user_dir.join("user_1.ini")).unwrap(), EA_USER_KEY), Some(win.clone()));
        assert_eq!(std::fs::read_to_string(user_dir.join("user_2.ini")).unwrap(), "user.other=1\n");

        // Cleared: the EA app's own folder again.
        store_folders::set(&paths, "/home/u", Client::Ea.id(), None).unwrap();
        release(&paths, Client::Ea, "/home/u", &games);
        assert_eq!(ini_get(&std::fs::read_to_string(&machine).unwrap(), EA_KEY).as_deref(), Some(EA_DEFAULT));

        // Ubisoft Connect, whose settings file is not there yet.
        let ubi = prefix(&paths, Client::Ubisoft, dir.path());
        store_folders::set(&paths, "/home/u", Client::Ubisoft.id(), Some(games.to_str().unwrap())).unwrap();
        apply(&paths, Client::Ubisoft, "/home/u");
        let yaml = std::fs::read_to_string(ubi.join("pfx/drive_c/users/steamuser").join(UBISOFT_SETTINGS)).unwrap();
        assert_eq!(yaml_misc_get(&yaml), Some(format!("{}/", windows_path(&games).replace('\\', "/"))));
        // Changed in the client since: left alone.
        let file = ubi.join("pfx/drive_c/users/steamuser").join(UBISOFT_SETTINGS);
        std::fs::write(&file, yaml_misc_set("", "D:/Other/")).unwrap();
        release(&paths, Client::Ubisoft, "/home/u", &games);
        assert_eq!(yaml_misc_get(&std::fs::read_to_string(&file).unwrap()).as_deref(), Some("D:/Other/"));
    }

    #[cfg(unix)]
    #[test]
    fn battlenet_and_rockstar_settings_are_never_written() {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path().join("data"));
        let games = dir.path().join("Games/Blizzard");
        std::fs::create_dir_all(&games).unwrap();
        let bnet = prefix(&paths, Client::BattleNet, dir.path());
        store_folders::set(&paths, "/home/u", Client::BattleNet.id(), Some(games.to_str().unwrap())).unwrap();
        let before: Vec<_> = walk(&bnet);
        apply(&paths, Client::BattleNet, "/home/u");
        release(&paths, Client::BattleNet, "/home/u", &games);
        assert_eq!(walk(&bnet), before);
        assert!(!sets_it(Client::BattleNet) && !sets_it(Client::Rockstar) && sets_it(Client::Ea) && sets_it(Client::Ubisoft));
    }

    #[cfg(unix)]
    fn walk(dir: &Path) -> Vec<PathBuf> {
        let mut out = Vec::new();
        for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
            let p = e.path();
            if e.file_type().is_ok_and(|t| t.is_dir()) {
                out.extend(walk(&p));
            }
            out.push(p);
        }
        out.sort();
        out
    }
}
