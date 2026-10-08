//! Reading and writing electron-store JSON files compatibly with Heronite.
//!
//! electron-store serializes with `JSON.stringify(value, undefined, '\t')`:
//! tab indentation and no trailing newline. Writes here produce the same
//! bytes so diffs stay clean, go through a temp file + rename so a crash or a
//! concurrent reader never sees a partial file, and always re-read the file
//! first (read-modify-write) so fields this app does not model, or changes
//! Heronite made since the last load, survive.

use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use std::time::SystemTime;

use anyhow::{Context, Result};
use parking_lot::Mutex;
use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::Value;

/// Reads a JSON file. A missing or empty file yields `Value::Null`.
pub fn read_value(path: &Path) -> Result<Value> {
    match fs::read(path) {
        Ok(bytes) if bytes.iter().all(u8::is_ascii_whitespace) => Ok(Value::Null),
        Ok(bytes) => serde_json::from_slice(&bytes)
            .with_context(|| format!("parsing {}", path.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Value::Null),
        Err(e) => Err(e).with_context(|| format!("reading {}", path.display())),
    }
}

/// Reads and deserializes a JSON file; missing, empty or `null` files give
/// `T::default()`. Decodes straight from the bytes (no intermediate `Value`).
pub fn read<T: DeserializeOwned + Default>(path: &Path) -> Result<T> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(T::default()),
        Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
    };
    if bytes.iter().all(u8::is_ascii_whitespace) {
        return Ok(T::default());
    }
    let doc: Option<T> = serde_json::from_slice(&bytes)
        .with_context(|| format!("decoding {}", path.display()))?;
    Ok(doc.unwrap_or_default())
}

/// Serializes like electron-store: tab indent, no trailing newline.
pub fn to_electron_store_bytes(value: &impl Serialize) -> Result<Vec<u8>> {
    to_bytes_indented(value, b"\t")
}

/// Indent of the files Heroic writes with `JSON.stringify(x, null, 2)`
/// (`GamesConfig/*.json`, the global `config.json`).
pub const INDENT_2: &[u8] = b"  ";

/// Like [`to_electron_store_bytes`] with another indent string (no trailing
/// newline).
pub fn to_bytes_indented(value: &impl Serialize, indent: &[u8]) -> Result<Vec<u8>> {
    let mut out = Vec::with_capacity(4096);
    let fmt = serde_json::ser::PrettyFormatter::with_indent(indent);
    let mut ser = serde_json::Serializer::with_formatter(&mut out, fmt);
    value.serialize(&mut ser)?;
    Ok(out)
}

/// Atomically replaces `path` with `value`.
pub fn write(path: &Path, value: &impl Serialize) -> Result<()> {
    write_indented(path, value, b"\t")
}

/// [`write`] with another indent string.
pub fn write_indented(path: &Path, value: &impl Serialize, indent: &[u8]) -> Result<()> {
    let bytes = to_bytes_indented(value, indent)?;
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    }
    let tmp = tmp_path(path);
    {
        let mut f = fs::File::create(&tmp).with_context(|| format!("creating {}", tmp.display()))?;
        f.write_all(&bytes)?;
        f.sync_all()?;
    }
    fs::rename(&tmp, path).with_context(|| format!("replacing {}", path.display()))?;
    Ok(())
}

fn tmp_path(path: &Path) -> PathBuf {
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    path.with_file_name(format!(".{name}.heronite-rs-{}.tmp", std::process::id()))
}

fn path_lock(path: &Path) -> Arc<Mutex<()>> {
    static LOCKS: OnceLock<Mutex<HashMap<PathBuf, Arc<Mutex<()>>>>> = OnceLock::new();
    LOCKS
        .get_or_init(Default::default)
        .lock()
        .entry(path.to_path_buf())
        .or_default()
        .clone()
}

/// Read-modify-write of the raw JSON document. The closure gets the current
/// on-disk value (an empty object when the file is missing). Serialized per
/// path inside this process.
pub fn update_value<R>(path: &Path, f: impl FnOnce(&mut Value) -> Result<R>) -> Result<R> {
    update_value_indented(path, b"\t", f)
}

/// [`update_value`] writing with another indent string (e.g. [`INDENT_2`]).
pub fn update_value_indented<R>(
    path: &Path,
    indent: &[u8],
    f: impl FnOnce(&mut Value) -> Result<R>,
) -> Result<R> {
    let lock = path_lock(path);
    let _guard = lock.lock();
    let mut value = read_value(path)?;
    if value.is_null() {
        value = Value::Object(Default::default());
    }
    let result = f(&mut value)?;
    write_indented(path, &value, indent)?;
    Ok(result)
}

/// Typed read-modify-write. `T` must keep unknown fields (use
/// `#[serde(flatten)] extra: serde_json::Map<String, Value>`) so nothing
/// Heronite wrote is lost.
pub fn update<T, R>(path: &Path, f: impl FnOnce(&mut T) -> Result<R>) -> Result<R>
where
    T: DeserializeOwned + Serialize + Default,
{
    let lock = path_lock(path);
    let _guard = lock.lock();
    let mut doc: T = read(path)?;
    let result = f(&mut doc)?;
    write(path, &doc)?;
    Ok(result)
}

/// Modification time, used to notice external changes (Heronite writing).
pub fn mtime(path: &Path) -> Option<SystemTime> {
    fs::metadata(path).and_then(|m| m.modified()).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;
    use serde_json::{json, Map};

    #[test]
    fn matches_json_stringify_tab_format() {
        let v = json!({"games": {"a": {"title": "Ação", "n": 1, "l": [1, 2]}}, "e": []});
        let s = String::from_utf8(to_electron_store_bytes(&v).unwrap()).unwrap();
        // JSON.stringify(v, undefined, '\t') output:
        let expected = "{\n\t\"games\": {\n\t\t\"a\": {\n\t\t\t\"title\": \"Ação\",\n\t\t\t\"n\": 1,\n\t\t\t\"l\": [\n\t\t\t\t1,\n\t\t\t\t2\n\t\t\t]\n\t\t}\n\t},\n\t\"e\": []\n}";
        assert_eq!(s, expected);
    }

    #[derive(Default, Serialize, Deserialize)]
    struct Doc {
        #[serde(default)]
        known: u32,
        #[serde(flatten)]
        extra: Map<String, Value>,
    }

    #[test]
    fn typed_update_preserves_unknown_fields_and_order() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("x.json");
        fs::write(&p, "{\n\t\"z\": 1,\n\t\"known\": 1,\n\t\"a\": {\n\t\t\"b\": true\n\t}\n}").unwrap();
        update::<Doc, _>(&p, |d| {
            d.known = 2;
            Ok(())
        })
        .unwrap();
        let v = read_value(&p).unwrap();
        assert_eq!(v["known"], 2);
        assert_eq!(v["z"], 1);
        assert_eq!(v["a"]["b"], true);
    }

    #[test]
    fn missing_file_reads_default() {
        let dir = tempfile::tempdir().unwrap();
        let d: Doc = read(&dir.path().join("none.json")).unwrap();
        assert_eq!(d.known, 0);
    }
}
