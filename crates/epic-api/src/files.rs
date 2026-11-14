//! Writing legendary's JSON files the way legendary does: two-space indent,
//! keys sorted, replaced atomically.

use std::path::Path;

use anyhow::{Context, Result};
use serde_json::{Map, Value};

/// `value` with every object's keys sorted (Python's `sort_keys=True`).
pub fn sorted(value: &Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            Value::Object(keys.into_iter().map(|k| (k.clone(), sorted(&map[k]))).collect::<Map<_, _>>())
        }
        Value::Array(items) => Value::Array(items.iter().map(sorted).collect()),
        other => other.clone(),
    }
}

/// Replaces `path` with `value`. `private` files (sign-ins) are readable by
/// the user only.
pub fn write_json(path: &Path, value: &Value, private: bool) -> Result<()> {
    let bytes = heronite_data::json_store::to_bytes_indented(&sorted(value), heronite_data::json_store::INDENT_2)?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    }
    heronite_data::temps::write_atomic(path, &bytes, private.then_some(0o600)).with_context(|| format!("writing {}", path.display()))
}

/// Reads a JSON file; `None` when it is missing, empty or not JSON.
pub fn read_json(path: &Path) -> Option<Value> {
    heronite_data::json_store::read_value(path).ok().filter(|v| !v.is_null())
}

/// legendary's `clean_filename`: drops the characters file systems refuse.
pub fn clean_filename(name: &str) -> String {
    name.chars().filter(|c| !"<>:\"/\\|?*".contains(*c)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn files_are_written_with_sorted_keys() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("x/user.json");
        write_json(&path, &json!({"b": 1, "a": {"d": 2, "c": [ {"z": 0, "y": 1} ]}}), true).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert_eq!(
            text,
            "{\n  \"a\": {\n    \"c\": [\n      {\n        \"y\": 1,\n        \"z\": 0\n      }\n    ],\n    \"d\": 2\n  },\n  \"b\": 1\n}"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        }
        assert_eq!(read_json(&path).unwrap()["b"], 1);
        assert!(read_json(&dir.path().join("none.json")).is_none());
        assert_eq!(clean_filename("a:b/c?.manifest"), "abc.manifest");
    }
}
