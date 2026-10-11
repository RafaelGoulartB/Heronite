//! Covers and backgrounds picked from a capture point at its file
//! (`localart://screenshots/<game>/<file>` or an absolute path in
//! `local_library/art.json`); a conversion renames the file, so they follow.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use heronite_data::paths::percent_decode;
use heronite_data::{json_store, DataPaths};
use serde_json::Value;

const SCHEME: &str = "localart://screenshots/";

/// Points the art references to `renamed` files (`from` -> `to`, both under
/// the captures `root`) at their new names. Writes `art.json` only when
/// something changed; returns how many references were updated.
pub fn rewrite_art_refs(data_paths: &DataPaths, root: &Path, renamed: &[(PathBuf, PathBuf)]) -> Result<usize, String> {
    if renamed.is_empty() {
        return Ok(0);
    }
    let map: HashMap<&Path, &Path> = renamed.iter().map(|(f, t)| (f.as_path(), t.as_path())).collect();
    let path = data_paths.art();
    let mut doc = json_store::read_value(&path).map_err(|e| format!("{e:#}"))?;
    if rewrite(&mut doc, root, &map) == 0 {
        return Ok(0);
    }
    json_store::update_value(&path, |doc| Ok(rewrite(doc, root, &map))).map_err(|e| format!("{e:#}"))
}

fn rewrite(value: &mut Value, root: &Path, map: &HashMap<&Path, &Path>) -> usize {
    match value {
        Value::String(s) => match renamed_ref(s, root, map) {
            Some(new) => {
                *s = new;
                1
            }
            None => 0,
        },
        Value::Array(items) => items.iter_mut().map(|v| rewrite(v, root, map)).sum(),
        Value::Object(obj) => obj.values_mut().map(|v| rewrite(v, root, map)).sum(),
        _ => 0,
    }
}

fn renamed_ref(s: &str, root: &Path, map: &HashMap<&Path, &Path>) -> Option<String> {
    if let Some(rest) = s.strip_prefix(SCHEME) {
        let cut = rest.find(['?', '#']).unwrap_or(rest.len());
        let (encoded, tail) = rest.split_at(cut);
        let from = root.join(percent_decode(encoded));
        let to = map.get(from.as_path())?;
        // Only the end of the name changes (`.png` -> `.webp` or `-2.webp`),
        // and those characters are never percent-encoded: keep the rest of
        // the URL as Heronite wrote it.
        let stem = from.file_stem()?.to_str()?;
        let new_suffix = to.file_name()?.to_str()?.strip_prefix(stem)?;
        let old_suffix = from.extension()?.to_str()?;
        let base = encoded.strip_suffix(old_suffix)?.strip_suffix('.')?;
        return Some(format!("{SCHEME}{base}{new_suffix}{tail}"));
    }
    let to = map.get(Path::new(s))?;
    Some(to.to_string_lossy().into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn updates_screenshot_urls_and_paths_only() {
        let dir = tempfile::tempdir().unwrap();
        let data_paths = DataPaths::at(dir.path());
        let root = Path::new("/caps");
        json_store::write(
            &data_paths.art(),
            &json!({"games": {
                "sideload:a": {"heroUrl": "localart://screenshots/Elden%20Ring/a%20b.png?v=2", "coverUrl": "localart://screenshots/Elden%20Ring/c.png"},
                "sideload:b": {"heroUrl": "/caps/Elden Ring/d.PNG", "coverUrl": "localart://art/x.png"}
            }}),
        )
        .unwrap();
        let renamed = vec![
            (PathBuf::from("/caps/Elden Ring/a b.png"), PathBuf::from("/caps/Elden Ring/a b.webp")),
            (PathBuf::from("/caps/Elden Ring/c.png"), PathBuf::from("/caps/Elden Ring/c-2.webp")),
            (PathBuf::from("/caps/Elden Ring/d.PNG"), PathBuf::from("/caps/Elden Ring/d.webp")),
        ];
        assert_eq!(rewrite_art_refs(&data_paths, root, &renamed).unwrap(), 3);
        let doc = json_store::read_value(&data_paths.art()).unwrap();
        assert_eq!(doc["games"]["sideload:a"]["heroUrl"], "localart://screenshots/Elden%20Ring/a%20b.webp?v=2");
        assert_eq!(doc["games"]["sideload:a"]["coverUrl"], "localart://screenshots/Elden%20Ring/c-2.webp");
        assert_eq!(doc["games"]["sideload:b"]["heroUrl"], "/caps/Elden Ring/d.webp");
        assert_eq!(doc["games"]["sideload:b"]["coverUrl"], "localart://art/x.png");
        assert_eq!(rewrite_art_refs(&data_paths, root, &renamed).unwrap(), 0);
    }
}
