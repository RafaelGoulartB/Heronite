//! "Folders visible to Proton" (`heroniteSharedFolders`, a Heronite key in
//! Heroic's global defaults): folders outside the ones the Steam Linux
//! Runtime shares by default (home, `/mnt`, `/media`, `/run/media`, `/tmp`)
//! that games and installers run through umu / the runtime should see.
//!
//! * The runtime's container (pressure-vessel) only mounts what it is told
//!   to; `PRESSURE_VESSEL_FILESYSTEMS_RW` adds the folders. umu passes that
//!   variable through untouched (it rewrites `STEAM_COMPAT_MOUNTS`).
//! * Each folder also gets a drive letter in the prefix (`dosdevices/d:` ->
//!   the folder), so Windows file pickers list it next to `C:`. Wine already
//!   shows everything under `Z:\`, the letter is a shortcut.

use std::path::Path;

use serde_json::Value;

pub const KEY: &str = "heroniteSharedFolders";
/// Read by pressure-vessel: extra paths to mount read-write (`:`-separated).
pub const ENV_KEY: &str = "PRESSURE_VESSEL_FILESYSTEMS_RW";

/// The folders of the setting: absolute, without `:` (the list separator),
/// trailing `/` removed, no duplicates, in order.
pub fn parse(v: Option<&Value>) -> Vec<String> {
    let Some(Value::Array(list)) = v else { return Vec::new() };
    let mut out: Vec<String> = Vec::new();
    for item in list {
        let Some(raw) = item.as_str() else { continue };
        let p = raw.trim();
        let p = if p.len() > 1 { p.trim_end_matches('/') } else { p };
        if p.starts_with('/') && p != "/" && !p.contains(':') && !out.iter().any(|o| o == p) {
            out.push(p.to_string());
        }
    }
    out
}

/// `PRESSURE_VESSEL_FILESYSTEMS_RW` with `folders` appended to what is
/// already set (the user's own value first). `None`: nothing to add.
pub fn container_env(folders: &[String], existing: &[Option<String>]) -> Option<String> {
    if folders.is_empty() {
        return None;
    }
    let mut parts: Vec<String> = Vec::new();
    let all = existing.iter().flatten().flat_map(|v| v.split(':')).map(str::to_string).chain(folders.iter().cloned());
    for p in all {
        if !p.is_empty() && !parts.contains(&p) {
            parts.push(p);
        }
    }
    Some(parts.join(":"))
}

/// The `dosdevices` folder of a prefix (Proton keeps the Wine prefix in
/// `<prefix>/pfx`). `None` until Wine/Proton has created it.
fn dosdevices(prefix: &str) -> Option<std::path::PathBuf> {
    let base = Path::new(prefix);
    [base.join("pfx/dosdevices"), base.join("dosdevices")].into_iter().find(|d| d.is_dir())
}

/// Gives each folder a drive letter in the prefix (the first free one from
/// `D:`), unless a drive already points at it. Returns the drives added.
/// A prefix that does not exist yet is left alone: the next run maps them.
pub fn map_drive_letters(prefix: &str, folders: &[String]) -> Vec<(char, String)> {
    let mut added = Vec::new();
    if folders.is_empty() || prefix.trim().is_empty() {
        return added;
    }
    let Some(dir) = dosdevices(prefix) else { return added };
    for folder in folders {
        if !Path::new(folder).is_dir() {
            continue;
        }
        let taken = |letter: char| std::fs::symlink_metadata(dir.join(format!("{letter}:"))).is_ok();
        let mapped = ('a'..='y').any(|l| std::fs::read_link(dir.join(format!("{l}:"))).is_ok_and(|t| t == Path::new(folder)));
        if mapped {
            continue;
        }
        let Some(letter) = ('d'..='y').find(|l| !taken(*l)) else { break };
        #[cfg(unix)]
        match std::os::unix::fs::symlink(folder, dir.join(format!("{letter}:"))) {
            Ok(()) => added.push((letter.to_ascii_uppercase(), folder.clone())),
            Err(e) => log::warn!("could not map {folder} to {letter}: in {}: {e}", dir.display()),
        }
    }
    added
}

/// [`map_drive_letters`] for the prefix of `settings` (not CrossOver, whose
/// bottles are not plain prefixes), logging what it added.
pub fn map_for(settings: &crate::settings::GameSettings) {
    if settings.wine_version.kind == "crossover" {
        return;
    }
    for (letter, folder) in map_drive_letters(&settings.wine_prefix, &settings.shared_folders) {
        log::info!("{folder} is drive {letter}: in {}", settings.wine_prefix);
    }
}

/// Before running a program in a prefix that does not exist yet: creates the
/// prefix (`wineboot -u` with the same Wine/Proton) so the drive letters can
/// be added before the program starts. Without shared folders, or with a
/// prefix already there, nothing happens. Blocking (Proton takes a while to
/// build a prefix); `Ok(true)` when a prefix was created. Not run by tests.
pub fn create_prefix_first(
    ctx: &crate::PlanContext,
    probe: &dyn crate::probe::Probe,
    settings: &crate::settings::GameSettings,
) -> Result<bool, String> {
    if settings.shared_folders.is_empty() || settings.wine_version.kind == "crossover" || settings.wine_prefix.trim().is_empty() {
        return Ok(false);
    }
    if dosdevices(&settings.wine_prefix).is_some() {
        return Ok(false);
    }
    let c = crate::wine_tools::wine_command(ctx, probe, settings, vec!["wineboot".into(), "-u".into()])?;
    log::info!("creating the prefix {} before mapping shared folders", settings.wine_prefix);
    let status = std::process::Command::new(&c.program)
        .args(&c.args)
        .envs(&c.env)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map_err(|e| format!("Could not start {}: {e}", c.program))?;
    if !status.success() {
        log::warn!("wineboot in {} exited with {status}", settings.wine_prefix);
    }
    map_for(settings);
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::fs;

    #[test]
    fn parses_clean_absolute_folders() {
        let v = json!(["/data", "/data/", " /games ", "relative", "/", "/a:b", 3, "/data"]);
        assert_eq!(parse(Some(&v)), vec!["/data".to_string(), "/games".to_string()]);
        assert!(parse(Some(&json!("x"))).is_empty());
        assert!(parse(None).is_empty());
    }

    #[test]
    fn env_keeps_the_users_value() {
        let f = vec!["/data".to_string()];
        assert_eq!(container_env(&f, &[]), Some("/data".into()));
        assert_eq!(container_env(&f, &[Some("/srv:/data".into()), None]), Some("/srv:/data".into()));
        assert_eq!(container_env(&[], &[Some("/srv".into())]), None);
    }

    #[test]
    fn drive_letters_skip_used_ones_and_existing_maps() {
        let dir = tempfile::tempdir().unwrap();
        let prefix = dir.path().join("pfx-root");
        let dos = prefix.join("pfx/dosdevices");
        fs::create_dir_all(&dos).unwrap();
        std::os::unix::fs::symlink("/", dos.join("z:")).unwrap();
        std::os::unix::fs::symlink("/somewhere", dos.join("d:")).unwrap();
        let a = dir.path().join("a");
        let b = dir.path().join("b");
        fs::create_dir_all(&a).unwrap();
        fs::create_dir_all(&b).unwrap();
        let folders = vec![a.to_string_lossy().into_owned(), b.to_string_lossy().into_owned(), "/missing".into()];
        let prefix = prefix.to_string_lossy().into_owned();
        let added = map_drive_letters(&prefix, &folders);
        assert_eq!(added.iter().map(|(l, _)| *l).collect::<Vec<_>>(), vec!['E', 'F']);
        assert_eq!(fs::read_link(dos.join("e:")).unwrap(), a);
        // Already mapped: nothing new.
        assert!(map_drive_letters(&prefix, &folders).is_empty());
    }

    #[test]
    fn a_missing_prefix_is_left_alone() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("new");
        assert!(map_drive_letters(&p.to_string_lossy(), &["/tmp".into()]).is_empty());
        assert!(!p.exists());
    }
}
