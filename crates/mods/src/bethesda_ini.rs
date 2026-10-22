//! Fallout 4 and Starfield ignore loose files (textures, meshes, sounds)
//! unless their `*Custom.ini` in the prefix's `Documents/My Games/<game>`
//! turns the archive check off. The keys are set while the game runs and the
//! user's file is put back afterwards, like `Plugins.txt`.

use std::fs;
use std::path::{Path, PathBuf};

use crate::file_swap::Swap;
use crate::fsutil::find_ci;
use crate::games::KnownGame;
use crate::plugins::prefix_users;

const SECTION: &str = "Archive";
/// `(key, value)` the game needs under `[Archive]`.
const KEYS: [(&str, &str); 2] = [("bInvalidateOlderFiles", "1"), ("sResourceDataDirsFinal", "")];

/// Puts the user's `*Custom.ini` back when dropped.
#[derive(Debug)]
pub struct IniGuard {
    _swap: Swap,
    file: PathBuf,
}

impl IniGuard {
    /// The file that was changed.
    pub fn file(&self) -> &Path {
        &self.file
    }
}

/// `(folder under My Games, file name)` of the games that need the keys,
/// found by their Nexus Mods domain.
fn spec_of(known: Option<&KnownGame>) -> Option<(&'static str, &'static str)> {
    match known?.nexus_domain? {
        "fallout4" => Some(("Fallout4", "Fallout4Custom.ini")),
        "starfield" => Some(("Starfield", "StarfieldCustom.ini")),
        _ => None,
    }
}

/// Makes sure the game reads loose files. `None` when the game does not need
/// it, the prefix has no user folder yet, or the file already says so.
pub fn ensure_loose_files(prefix: &Path, known: Option<&KnownGame>) -> Result<Option<IniGuard>, String> {
    let Some((folder, name)) = spec_of(known) else { return Ok(None) };
    let users = prefix_users(prefix);
    let has_folder = |home: &Path| documents_of(home).is_some_and(|d| d.join("My Games").join(folder).is_dir());
    let Some((_, home)) =
        users.iter().find(|(_, h)| has_folder(h)).or_else(|| users.iter().find(|(n, _)| n == "steamuser")).or(users.first())
    else {
        return Ok(None);
    };
    let dir = documents_of(home).unwrap_or_else(|| home.join("Documents")).join("My Games").join(folder);
    let file = dir.join(name);
    Swap::recover(&file)?;

    let original = fs::read(&file).ok();
    let text = original.as_deref().map(|b| String::from_utf8_lossy(b).into_owned()).unwrap_or_default();
    let Some(content) = with_keys(&text) else { return Ok(None) };
    fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let swap = Swap::replace(&file, original.as_deref(), content.as_bytes())?;
    log::info!("mods: {} lets {folder} read loose files", file.display());
    Ok(Some(IniGuard { _swap: swap, file }))
}

/// The user's `Documents` folder (Wine also names it `My Documents`).
fn documents_of(home: &Path) -> Option<PathBuf> {
    ["Documents", "My Documents"]
        .iter()
        .map(|n| home.join(n))
        .find(|p| p.is_dir())
        .or_else(|| find_ci(home, "Documents").filter(|(_, is_dir)| *is_dir).map(|(n, _)| home.join(n)))
}

/// `text` with the keys set under `[Archive]`, every other line kept; `None`
/// when they are set already.
fn with_keys(text: &str) -> Option<String> {
    let (bom, body) = match text.strip_prefix('\u{feff}') {
        Some(rest) => ("\u{feff}", rest),
        None => ("", text),
    };
    let eol = if body.contains('\n') && !body.contains("\r\n") { "\n" } else { "\r\n" };
    let mut lines: Vec<String> = body.lines().map(str::to_string).collect();
    let mut changed = false;

    let header = |l: &str| l.trim().strip_prefix('[').and_then(|r| r.split(']').next()).map(str::trim).map(str::to_string);
    let start = lines.iter().position(|l| header(l).is_some_and(|h| h.eq_ignore_ascii_case(SECTION)));
    let Some(start) = start else {
        if lines.last().is_some_and(|l| !l.trim().is_empty()) {
            lines.push(String::new());
        }
        lines.push(format!("[{SECTION}]"));
        lines.extend(KEYS.iter().map(|(k, v)| format!("{k}={v}")));
        return Some(format!("{bom}{}{eol}", lines.join(eol)));
    };
    let end = lines[start + 1..].iter().position(|l| l.trim().starts_with('[')).map_or(lines.len(), |i| start + 1 + i);

    let mut missing = Vec::new();
    for (key, value) in KEYS {
        let mut found = false;
        for line in &mut lines[start + 1..end] {
            let Some((k, v)) = line.split_once('=') else { continue };
            if !k.trim().eq_ignore_ascii_case(key) || k.trim_start().starts_with([';', '#']) {
                continue;
            }
            found = true;
            if v.trim() != value {
                *line = format!("{key}={value}");
                changed = true;
            }
        }
        if !found {
            missing.push(format!("{key}={value}"));
        }
    }
    if !missing.is_empty() {
        changed = true;
        let last = (start + 1..end).rev().find(|&i| !lines[i].trim().is_empty()).unwrap_or(start);
        for (i, line) in missing.into_iter().enumerate() {
            lines.insert(last + 1 + i, line);
        }
    }
    changed.then(|| format!("{bom}{}{eol}", lines.join(eol)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::games::find;

    #[test]
    fn keys_are_added_or_fixed_and_everything_else_is_kept() {
        assert_eq!(with_keys("").unwrap(), "[Archive]\r\nbInvalidateOlderFiles=1\r\nsResourceDataDirsFinal=\r\n");
        assert_eq!(
            with_keys("[General]\nsLanguage=EN\n").unwrap(),
            "[General]\nsLanguage=EN\n\n[Archive]\nbInvalidateOlderFiles=1\nsResourceDataDirsFinal=\n"
        );
        assert_eq!(
            with_keys("[Archive]\r\nbInvalidateOlderFiles=0\r\n\r\n[Display]\r\niSize=1\r\n").unwrap(),
            "[Archive]\r\nbInvalidateOlderFiles=1\r\nsResourceDataDirsFinal=\r\n\r\n[Display]\r\niSize=1\r\n"
        );
        assert_eq!(
            with_keys("[archive]\nSRESOURCEDATADIRSFINAL=STRINGS\\\nbinvalidateolderfiles = 1\n").unwrap(),
            "[archive]\nsResourceDataDirsFinal=\nbinvalidateolderfiles = 1\n"
        );
        assert_eq!(
            with_keys("\u{feff}[Archive]\nx=1\n").unwrap(),
            "\u{feff}[Archive]\nx=1\nbInvalidateOlderFiles=1\nsResourceDataDirsFinal=\n"
        );
        assert_eq!(with_keys("[Archive]\nbInvalidateOlderFiles=1\nsResourceDataDirsFinal=\n[X]\na=b\n"), None);
    }

    fn fo4() -> Option<&'static KnownGame> {
        find(Some("377160"), "")
    }

    fn prefix(t: &Path) -> PathBuf {
        let p = t.join("prefix");
        fs::create_dir_all(p.join("drive_c/users/steamuser")).unwrap();
        fs::create_dir_all(p.join("drive_c/users/Public")).unwrap();
        p
    }

    #[test]
    fn the_file_is_made_changed_and_put_back() {
        let t = tempfile::tempdir().unwrap();
        let prefix = prefix(t.path());
        let dir = prefix.join("drive_c/users/steamuser/Documents/My Games/Fallout4");
        let file = dir.join("Fallout4Custom.ini");

        let guard = ensure_loose_files(&prefix, fo4()).unwrap().unwrap();
        assert!(fs::read_to_string(&file).unwrap().contains("bInvalidateOlderFiles=1"));
        drop(guard);
        assert!(!file.exists() && !dir.join("Fallout4Custom.ini.heronite-new").exists());

        fs::write(&file, "[Launcher]\r\nbEnableFileSelection=1\r\n").unwrap();
        let guard = ensure_loose_files(&prefix, fo4()).unwrap().unwrap();
        let now = fs::read_to_string(&file).unwrap();
        assert!(now.starts_with("[Launcher]\r\nbEnableFileSelection=1\r\n") && now.contains("[Archive]"), "{now}");
        // A crash: the next run starts from the user's file.
        std::mem::forget(guard);
        let again = ensure_loose_files(&prefix, fo4()).unwrap().unwrap();
        assert_eq!(fs::read_to_string(&file).unwrap().matches("[Archive]").count(), 1);
        drop(again);
        assert_eq!(fs::read_to_string(&file).unwrap(), "[Launcher]\r\nbEnableFileSelection=1\r\n");

        fs::write(&file, "[Archive]\nbInvalidateOlderFiles=1\nsResourceDataDirsFinal=\n").unwrap();
        assert!(ensure_loose_files(&prefix, fo4()).unwrap().is_none(), "already set: nothing to undo");
    }

    #[test]
    fn other_games_and_empty_prefixes_are_left_alone() {
        let t = tempfile::tempdir().unwrap();
        let prefix = prefix(t.path());
        assert!(ensure_loose_files(&prefix, find(Some("489830"), "")).unwrap().is_none());
        assert!(ensure_loose_files(&prefix, None).unwrap().is_none());
        assert!(ensure_loose_files(&t.path().join("none"), fo4()).unwrap().is_none());
        let sf = find(Some("1716740"), "").expect("Starfield is known");
        let g = ensure_loose_files(&prefix, Some(sf)).unwrap().unwrap();
        assert!(g.file().ends_with("My Games/Starfield/StarfieldCustom.ini"));
    }

    #[test]
    fn the_users_existing_documents_folder_wins() {
        let t = tempfile::tempdir().unwrap();
        let prefix = prefix(t.path());
        let dir = prefix.join("drive_c/users/bob/My Documents/My Games/Fallout4");
        fs::create_dir_all(&dir).unwrap();
        let g = ensure_loose_files(&prefix, fo4()).unwrap().unwrap();
        assert_eq!(g.file(), dir.join("Fallout4Custom.ini"));
    }
}
