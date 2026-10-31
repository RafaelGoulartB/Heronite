//! Naming and deleting one backup.
//!
//! The name is Ludusavi's own backup comment (`backups edit --comment`), so
//! Ludusavi shows it too. Ludusavi has no command to delete a backup: the
//! entry is cut from the game's `mapping.yaml` and its zip or folder removed,
//! which Ludusavi reads back like any other set of backups.

use std::fs;
use std::path::Path;

use heronite_i18n::tr;

use super::super::backup::InFlight;
use super::super::LudusaviEnv;
use super::Tool;

/// Names a backup; an empty `name` clears it.
pub fn set_name(env: &LudusaviEnv, tool: &Tool, root: &Path, game: &str, backup_id: &str, name: &str) -> Result<(), String> {
    // Ludusavi rewrites the game's mapping.yaml, like a backup does.
    let Some(_guard) = InFlight::acquire() else {
        return Err(tr!("Ludusavi is busy with another backup or restore. Try again in a moment.").into());
    };
    let mut args: Vec<String> = ["--no-manifest-update", "backups", "edit", "--path"].iter().map(|s| s.to_string()).collect();
    args.push(root.to_string_lossy().into_owned());
    args.extend(["--backup".into(), backup_id.into(), "--comment".into(), name.trim().into(), game.into()]);
    let out = env.runner.run(&tool.binary, &args)?;
    if out.code == 0 {
        Ok(())
    } else {
        Err(Some(out.stderr.trim().to_string())
            .filter(|e| !e.is_empty())
            .unwrap_or_else(|| tr!("Ludusavi could not change the backup").into()))
    }
}

fn indent(line: &str) -> usize {
    line.len() - line.trim_start().len()
}

/// The `- name: <id>` value of a line, unquoted.
fn entry_name(line: &str) -> Option<&str> {
    let v = line.trim_start().strip_prefix("- name:")?.trim();
    Some(v.strip_prefix(['"', '\'']).and_then(|v| v.strip_suffix(['"', '\''])).unwrap_or(v))
}

/// `mapping.yaml` without the backup `id` (and, for a full backup, its
/// differential children), the names of the entries cut, and whether any
/// backup is left. `None` when there is no such backup.
fn cut_entry(yaml: &str, id: &str) -> Option<(String, Vec<String>, bool)> {
    let lines: Vec<&str> = yaml.lines().collect();
    let start = lines.iter().position(|l| entry_name(l) == Some(id))?;
    let level = indent(lines[start]);
    let end = (start + 1..lines.len()).find(|&i| !lines[i].trim().is_empty() && indent(lines[i]) <= level).unwrap_or(lines.len());
    let cut: Vec<String> = lines[start..end].iter().filter_map(|l| entry_name(l)).map(str::to_string).collect();
    let mut kept: Vec<String> = lines[..start].iter().chain(&lines[end..]).map(|l| l.to_string()).collect();
    // A list left empty (`children:` with nothing under it) stays a list.
    if let Some(key) = start.checked_sub(1).filter(|&k| lines[k].trim_end().ends_with(':')) {
        if lines.get(end).is_none_or(|next| indent(next) <= indent(lines[key])) {
            kept[key].push_str(" []");
        }
    }
    let any_left = kept.iter().any(|l| entry_name(l).is_some());
    let mut text = kept.join("\n");
    if yaml.ends_with('\n') {
        text.push('\n');
    }
    Some((text, cut, any_left))
}

fn remove_path(path: &Path) -> std::io::Result<()> {
    match fs::symlink_metadata(path) {
        Ok(m) if m.is_dir() => fs::remove_dir_all(path),
        Ok(_) => fs::remove_file(path),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

/// Writes `path` through a temporary file next to it, so a crash or a full
/// disk never leaves a half-written mapping (which would hide every backup
/// of the game from Ludusavi).
fn write_replacing(path: &Path, text: &str) -> std::io::Result<()> {
    let tmp = path.with_extension("yaml.heronite-tmp");
    let written = (|| {
        let mut f = fs::File::create(&tmp)?;
        std::io::Write::write_all(&mut f, text.as_bytes())?;
        f.sync_all()?;
        fs::rename(&tmp, path)
    })();
    if written.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    written
}

/// Deletes a backup from the game's folder under a backup root. The last
/// backup of a game takes the whole game folder with it.
pub fn delete_backup(game_dir: &Path, backup_id: &str) -> Result<(), String> {
    let Some(_guard) = InFlight::acquire() else {
        return Err(tr!("Ludusavi is busy with another backup or restore. Try again in a moment.").into());
    };
    let mapping = game_dir.join("mapping.yaml");
    let yaml = fs::read_to_string(&mapping).map_err(|e| format!("{}: {e}", mapping.display()))?;
    let Some((text, cut, any_left)) = cut_entry(&yaml, backup_id) else {
        return Err(tr!("This backup is not in the folder any more.").into());
    };
    log::info!("Deleting the Ludusavi backup {backup_id} of {} ({cut:?})", game_dir.display());
    if !any_left {
        return fs::remove_dir_all(game_dir).map_err(|e| format!("{}: {e}", game_dir.display()));
    }
    write_replacing(&mapping, &text).map_err(|e| format!("{}: {e}", mapping.display()))?;
    for name in cut {
        if name == "." {
            // The old simple format keeps its full backup in drive-* folders.
            for e in fs::read_dir(game_dir).into_iter().flatten().filter_map(Result::ok) {
                if e.file_name().to_string_lossy().starts_with("drive-") {
                    remove_path(&e.path()).map_err(|err| format!("{}: {err}", e.path().display()))?;
                }
            }
        } else if !name.contains('/') && !(cfg!(windows) && name.contains('\\')) && name != ".." {
            let path = game_dir.join(&name);
            remove_path(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const YAML: &str = "---\nname: Elden Ring\ndrives:\n  drive-0: \"\"\nbackups:\n  - name: backup-1.zip\n    when: \"2026-10-11T18:29:41Z\"\n    os: linux\n    files:\n      /p/a.sav:\n        hash: x\n        size: 1\n    registry:\n      hash: ~\n    children:\n      - name: backup-1-diff.zip\n        when: \"2026-10-12T00:00:00Z\"\n        files: {}\n  - name: backup-2.zip\n    when: \"2026-10-13T00:00:00Z\"\n    comment: Antes do chefe\n    files: {}\n    children: []\n";

    #[test]
    fn cuts_a_full_backup_with_its_children() {
        let (text, cut, left) = cut_entry(YAML, "backup-1.zip").unwrap();
        assert_eq!(cut, ["backup-1.zip", "backup-1-diff.zip"]);
        assert!(left);
        assert!(text.starts_with("---\nname: Elden Ring\ndrives:\n  drive-0: \"\"\nbackups:\n  - name: backup-2.zip\n"));
        assert!(text.ends_with("children: []\n"));
    }

    #[test]
    fn cuts_one_child_or_the_last_backup() {
        let (text, cut, left) = cut_entry(YAML, "backup-1-diff.zip").unwrap();
        assert_eq!((cut, left), (vec!["backup-1-diff.zip".to_string()], true));
        assert!(text.contains("    children: []\n  - name: backup-2.zip"));
        let (_, _, left) = cut_entry(&cut_entry(YAML, "backup-1.zip").unwrap().0, "backup-2.zip").unwrap();
        assert!(!left);
        assert!(cut_entry(YAML, "missing").is_none());
        assert_eq!(entry_name("  - name: \".\""), Some("."));
    }

    #[test]
    fn deletes_files_and_the_last_folder() {
        let d = tempfile::tempdir().unwrap();
        let game = d.path().join("Elden Ring");
        fs::create_dir_all(&game).unwrap();
        fs::write(game.join("mapping.yaml"), YAML).unwrap();
        for f in ["backup-1.zip", "backup-1-diff.zip", "backup-2.zip"] {
            fs::write(game.join(f), "z").unwrap();
        }
        // The in-flight guard is shared with the backup tests running alongside.
        let delete = |id: &str| {
            let _l = super::super::super::backup::test_lock();
            delete_backup(&game, id)
        };
        delete("backup-1.zip").unwrap();
        assert!(!game.join("backup-1.zip").exists() && !game.join("backup-1-diff.zip").exists());
        assert!(game.join("backup-2.zip").exists());
        assert!(!fs::read_to_string(game.join("mapping.yaml")).unwrap().contains("backup-1"));
        assert!(delete("backup-1.zip").is_err());
        delete("backup-2.zip").unwrap();
        assert!(!game.exists());
    }
}
