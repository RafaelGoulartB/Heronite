//! File operations of the steps: copy / move / rename with the scripts'
//! semantics, text, JSON and INI writes, and paths written with the wrong
//! case (Windows games are case-insensitive, Linux folders are not).

use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};

use heronite_i18n::tr;
use serde_json::Value;

fn err(path: &Path, e: io::Error) -> String {
    format!("{}: {e}", path.display())
}

/// `path` with each missing component replaced by an existing entry of the
/// same name in another case (`Program Files (x86)/game/GAME.EXE` finds
/// `Game.exe`). Components that exist in no case are kept.
pub fn fix_case(path: &Path) -> PathBuf {
    if path.exists() {
        return path.to_path_buf();
    }
    let mut out = PathBuf::new();
    for comp in path.components() {
        match comp {
            Component::Normal(name) => {
                let candidate = out.join(name);
                if candidate.exists() || out.as_os_str().is_empty() {
                    out = candidate;
                    continue;
                }
                let wanted = name.to_string_lossy().to_lowercase();
                let found = fs::read_dir(&out)
                    .ok()
                    .and_then(|d| d.flatten().map(|e| e.file_name()).find(|n| n.to_string_lossy().to_lowercase() == wanted));
                out.push(found.as_deref().unwrap_or(name));
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// Copies `src` into the folder `dst`: a file goes inside it, a folder's
/// contents are merged into it (existing files replaced). Returns where a
/// copied file ended up.
pub fn merge(src: &Path, dst: &Path) -> Result<Option<PathBuf>, String> {
    fs::create_dir_all(dst).map_err(|e| err(dst, e))?;
    if src.is_file() {
        let target = dst.join(src.file_name().unwrap_or_default());
        if src.parent() != Some(dst) {
            fs::copy(src, &target).map_err(|e| err(&target, e))?;
        }
        return Ok(Some(target));
    }
    copy_tree(src, dst)?;
    Ok(None)
}

fn copy_tree(src: &Path, dst: &Path) -> Result<(), String> {
    fs::create_dir_all(dst).map_err(|e| err(dst, e))?;
    for entry in fs::read_dir(src).map_err(|e| err(src, e))?.flatten() {
        let from = entry.path();
        let to = dst.join(entry.file_name());
        let ty = entry.file_type().map_err(|e| err(&from, e))?;
        if ty.is_dir() {
            copy_tree(&from, &to)?;
        } else if ty.is_symlink() {
            let link = fs::read_link(&from).map_err(|e| err(&from, e))?;
            let _ = fs::remove_file(&to);
            heronite_platform::fs::symlink(&link, &to).map_err(|e| err(&to, e))?;
        } else {
            if to.is_dir() {
                fs::remove_dir_all(&to).map_err(|e| err(&to, e))?;
            }
            fs::copy(&from, &to).map_err(|e| err(&to, e))?;
        }
    }
    Ok(())
}

/// Moves `src` to `dst` (renaming; copy + delete across file systems).
fn relocate(src: &Path, dst: &Path) -> Result<(), String> {
    if fs::rename(src, dst).is_ok() {
        return Ok(());
    }
    if src.is_dir() {
        copy_tree(src, dst)?;
        fs::remove_dir_all(src).map_err(|e| err(src, e))
    } else {
        fs::copy(src, dst).map_err(|e| err(dst, e))?;
        fs::remove_file(src).map_err(|e| err(src, e))
    }
}

/// Moves the contents of the folder `src` into `dst`, merging folders and
/// replacing files.
pub fn move_contents(src: &Path, dst: &Path) -> Result<(), String> {
    fs::create_dir_all(dst).map_err(|e| err(dst, e))?;
    for entry in fs::read_dir(src).map_err(|e| err(src, e))?.flatten() {
        let from = entry.path();
        let to = dst.join(entry.file_name());
        if from.is_dir() && !from.is_symlink() && to.is_dir() {
            move_contents(&from, &to)?;
            let _ = fs::remove_dir_all(&from);
        } else {
            if to.is_dir() && !to.is_symlink() {
                fs::remove_dir_all(&to).map_err(|e| err(&to, e))?;
            } else if to.exists() || to.is_symlink() {
                fs::remove_file(&to).map_err(|e| err(&to, e))?;
            }
            relocate(&from, &to)?;
        }
    }
    Ok(())
}

/// The `move` step: into `dst` when it is a folder, else to `dst`. A file
/// already at the destination is left alone (the safest choice for a
/// script run twice). Returns where `src` ended up.
pub fn move_into(src: &Path, dst: &Path) -> Result<PathBuf, String> {
    let target = if dst.is_dir() { dst.join(src.file_name().unwrap_or_default()) } else { dst.to_path_buf() };
    if src.is_file() {
        if src.parent() == Some(dst) {
            return Ok(src.to_path_buf());
        }
        if target.exists() {
            log::info!("{} exists, not moving {}", target.display(), src.display());
            return Ok(target);
        }
    }
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent).map_err(|e| err(parent, e))?;
    }
    if src.is_dir() && target.is_dir() {
        move_contents(src, &target)?;
        let _ = fs::remove_dir_all(src);
    } else {
        relocate(src, &target)?;
    }
    Ok(target)
}

/// [`move_into`] that copies: for files the user picked, which stay where
/// they keep them.
pub fn copy_into(src: &Path, dst: &Path) -> Result<PathBuf, String> {
    let target = if dst.is_dir() { dst.join(src.file_name().unwrap_or_default()) } else { dst.to_path_buf() };
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent).map_err(|e| err(parent, e))?;
    }
    if src.is_dir() {
        copy_tree(src, &target)?;
    } else {
        fs::copy(src, &target).map_err(|e| err(&target, e))?;
    }
    Ok(target)
}

/// The `rename` step: `dst` must not exist (an empty folder is replaced).
pub fn rename(src: &Path, dst: &Path) -> Result<(), String> {
    if !src.exists() {
        return Err(tr!("Cannot rename {path}: it does not exist", path = src.display()));
    }
    if dst.is_dir() {
        let _ = fs::remove_dir(dst);
    }
    if dst.exists() {
        return Err(tr!("Cannot rename to {path}: it already exists", path = dst.display()));
    }
    if let Some(parent) = dst.parent() {
        fs::create_dir_all(parent).map_err(|e| err(parent, e))?;
    }
    relocate(src, dst)
}

pub fn make_executable(path: &Path) -> Result<(), String> {
    fs::metadata(path).map_err(|_| tr!("Cannot make {path} executable: it does not exist", path = path.display()))?;
    heronite_platform::fs::add_mode(path, 0o111).map_err(|e| err(path, e))
}

pub fn write_text(path: &Path, content: &str, append: bool) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| err(parent, e))?;
    }
    if append {
        use std::io::Write;
        let mut f = fs::OpenOptions::new().create(true).append(true).open(path).map_err(|e| err(path, e))?;
        f.write_all(content.as_bytes()).map_err(|e| err(path, e))
    } else {
        fs::write(path, content).map_err(|e| err(path, e))
    }
}

/// Writes `data` as JSON; with `merge`, into what the file holds (maps are
/// merged key by key, other values replaced).
pub fn write_json(path: &Path, data: &Value, merge: bool) -> Result<(), String> {
    let mut doc = if merge {
        fs::read_to_string(path).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok()).unwrap_or(Value::Object(Default::default()))
    } else {
        Value::Object(Default::default())
    };
    deep_merge(&mut doc, data);
    let text = serde_json::to_string_pretty(&doc).map_err(|e| e.to_string())?;
    write_text(path, &text, false)
}

fn deep_merge(into: &mut Value, from: &Value) {
    match (into, from) {
        (Value::Object(a), Value::Object(b)) => {
            for (k, v) in b {
                match a.get_mut(k) {
                    Some(existing) if existing.is_object() && v.is_object() => deep_merge(existing, v),
                    _ => {
                        a.insert(k.clone(), v.clone());
                    }
                }
            }
        }
        (into, from) => *into = from.clone(),
    }
}

/// Sets `(section, [(key, value)])` in an INI file. With `merge`, the rest
/// of the file (comments, order, the spacing of edited lines, CRLF) stays.
pub fn write_ini(path: &Path, data: &[(String, Vec<(String, String)>)], merge: bool) -> Result<(), String> {
    let old = if merge { fs::read(path).map(|b| String::from_utf8_lossy(&b).into_owned()).unwrap_or_default() } else { String::new() };
    let text = edit_ini(&old, data);
    write_text(path, &text, false)
}

/// [`write_ini`] on text.
pub fn edit_ini(text: &str, data: &[(String, Vec<(String, String)>)]) -> String {
    let eol = if text.contains("\r\n") { "\r\n" } else { "\n" };
    let mut lines: Vec<String> = text.lines().map(str::to_string).collect();
    for (section, keys) in data {
        for (key, value) in keys {
            set_ini_key(&mut lines, section, key, value);
        }
    }
    let mut out = lines.join(eol);
    out.push_str(eol);
    out
}

fn section_name(line: &str) -> Option<&str> {
    let t = line.trim();
    t.strip_prefix('[').and_then(|s| s.strip_suffix(']')).map(str::trim)
}

fn set_ini_key(lines: &mut Vec<String>, section: &str, key: &str, value: &str) {
    let start = lines.iter().position(|l| section_name(l) == Some(section));
    let Some(start) = start else {
        if lines.last().is_some_and(|l| !l.trim().is_empty()) {
            lines.push(String::new());
        }
        lines.push(format!("[{section}]"));
        lines.push(format!("{key}={value}"));
        return;
    };
    let end = lines[start + 1..].iter().position(|l| section_name(l).is_some()).map(|i| start + 1 + i).unwrap_or(lines.len());
    for line in &mut lines[start + 1..end] {
        let t = line.trim_start();
        if t.starts_with(';') || t.starts_with('#') {
            continue;
        }
        let Some(sep) = line.find(['=', ':']) else { continue };
        if line[..sep].trim() != key {
            continue;
        }
        let rest = &line[sep + 1..];
        let pad = &rest[..rest.len() - rest.trim_start().len()];
        *line = format!("{}{pad}{value}", &line[..sep + 1]);
        return;
    }
    // After the last non-empty line of the section.
    let mut at = end;
    while at > start + 1 && lines[at - 1].trim().is_empty() {
        at -= 1;
    }
    lines.insert(at, format!("{key}={value}"));
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn fixes_the_case_of_existing_components() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("Program Files/My Game")).unwrap();
        fs::write(dir.path().join("Program Files/My Game/Game.exe"), "").unwrap();
        let wrong = dir.path().join("program files/MY GAME/game.EXE");
        assert_eq!(fix_case(&wrong), dir.path().join("Program Files/My Game/Game.exe"));
        let missing = dir.path().join("program files/Other/x.exe");
        assert_eq!(fix_case(&missing), dir.path().join("Program Files/Other/x.exe"));
    }

    #[test]
    fn merge_move_and_rename() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        fs::create_dir_all(d.join("src/sub")).unwrap();
        fs::write(d.join("src/a.txt"), "new").unwrap();
        fs::write(d.join("src/sub/b.txt"), "b").unwrap();
        fs::create_dir_all(d.join("dst")).unwrap();
        fs::write(d.join("dst/a.txt"), "old").unwrap();
        merge(&d.join("src"), &d.join("dst")).unwrap();
        assert_eq!(fs::read_to_string(d.join("dst/a.txt")).unwrap(), "new");
        assert!(d.join("dst/sub/b.txt").exists());
        assert!(d.join("src/a.txt").exists(), "merge copies");

        let target = merge(&d.join("src/a.txt"), &d.join("one")).unwrap();
        assert_eq!(target, Some(d.join("one/a.txt")));

        let moved = move_into(&d.join("src"), &d.join("dst")).unwrap();
        assert_eq!(moved, d.join("dst/src"));
        assert!(d.join("dst/src/sub/b.txt").exists() && !d.join("src").exists());

        rename(&d.join("dst/src"), &d.join("renamed")).unwrap();
        assert!(d.join("renamed/a.txt").exists());
        assert!(rename(&d.join("renamed"), &d.join("dst")).is_err(), "never over a folder with files");
    }

    #[test]
    fn move_contents_merges_folders() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        fs::create_dir_all(d.join("x/data")).unwrap();
        fs::write(d.join("x/data/1"), "x").unwrap();
        fs::write(d.join("x/file"), "x").unwrap();
        fs::create_dir_all(d.join("y/data")).unwrap();
        fs::write(d.join("y/data/2"), "y").unwrap();
        fs::write(d.join("y/file"), "y").unwrap();
        move_contents(&d.join("x"), &d.join("y")).unwrap();
        assert!(d.join("y/data/1").exists() && d.join("y/data/2").exists());
        assert_eq!(fs::read_to_string(d.join("y/file")).unwrap(), "x");
    }

    #[test]
    fn json_merges_maps() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("c/settings.json");
        write_json(&f, &json!({"a": {"b": 1, "c": 2}, "list": [1]}), true).unwrap();
        write_json(&f, &json!({"a": {"c": 3}, "list": [2]}), true).unwrap();
        let v: Value = serde_json::from_str(&fs::read_to_string(&f).unwrap()).unwrap();
        assert_eq!(v, json!({"a": {"b": 1, "c": 3}, "list": [2]}));
        write_json(&f, &json!({"z": 1}), false).unwrap();
        let v: Value = serde_json::from_str(&fs::read_to_string(&f).unwrap()).unwrap();
        assert_eq!(v, json!({"z": 1}));
    }

    #[test]
    fn ini_edits_keep_the_file() {
        let text = "; comment\r\n[Video]\r\nWidth = 800\r\nHeight=600\r\n\r\n[Audio]\r\nVolume=5\r\n";
        let data = vec![
            ("Video".to_string(), vec![("Width".to_string(), "1920".to_string()), ("Fullscreen".into(), "1".into())]),
            ("Input".to_string(), vec![("Pad".to_string(), "on".to_string())]),
        ];
        let out = edit_ini(text, &data);
        assert_eq!(
            out,
            "; comment\r\n[Video]\r\nWidth = 1920\r\nHeight=600\r\nFullscreen=1\r\n\r\n[Audio]\r\nVolume=5\r\n\r\n[Input]\r\nPad=on\r\n"
        );
        assert_eq!(edit_ini("", &data[1..]), "[Input]\nPad=on\n");
    }
}
