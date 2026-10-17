//! Launch presets (`local_library/launch_presets.json`, Heronite's): named
//! sets of environment variables, wrapper commands and game arguments saved
//! from one game and applied to others, such as "NVIDIA DLSS" or "Old
//! DirectX game".
//!
//! Applying adds to what the game has: a variable the game already has takes
//! the preset's value, wrappers and arguments it already has are not doubled.

use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

use crate::paths::DataPaths;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Preset {
    pub name: String,
    /// `(name, value)`, in order.
    #[serde(default)]
    pub env: Vec<(String, String)>,
    /// `(program, arguments)`, in order.
    #[serde(default)]
    pub wrappers: Vec<(String, String)>,
    /// Game arguments, as typed.
    #[serde(default)]
    pub args: String,
}

impl Preset {
    pub fn is_empty(&self) -> bool {
        self.env.is_empty() && self.wrappers.is_empty() && self.args.trim().is_empty()
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct File {
    #[serde(default)]
    presets: Vec<Preset>,
    #[serde(flatten)]
    extra: Map<String, Value>,
}

fn path(paths: &DataPaths) -> std::path::PathBuf {
    paths.local_library().join("launch_presets.json")
}

/// The presets, by name; none when the file is missing or unreadable.
pub fn load(paths: &DataPaths) -> Vec<Preset> {
    crate::json_store::read::<File>(&path(paths)).map(|f| f.presets).unwrap_or_default()
}

/// Changes the saved list. A file that cannot be read is an error, never
/// replaced: the presets in it are not lost to the next save.
fn modify(paths: &DataPaths, f: impl FnOnce(&mut Vec<Preset>)) -> Result<()> {
    crate::json_store::update::<File, _>(&path(paths), |file| {
        f(&mut file.presets);
        file.presets.sort_by_key(|p| p.name.to_lowercase());
        Ok(())
    })
}

/// Adds `preset`, replacing one with the same name (any case).
pub fn put(paths: &DataPaths, preset: Preset) -> Result<()> {
    modify(paths, |all| {
        all.retain(|p| !p.name.eq_ignore_ascii_case(&preset.name));
        all.push(preset);
    })
}

pub fn remove(paths: &DataPaths, name: &str) -> Result<()> {
    modify(paths, |all| all.retain(|p| p.name != name))
}

fn pairs(settings: &Map<String, Value>, key: &str, a: &str, b: &str) -> Vec<(String, String)> {
    let text = |v: Option<&Value>| v.and_then(Value::as_str).unwrap_or("").to_string();
    match settings.get(key) {
        Some(Value::Array(list)) => list.iter().map(|e| (text(e.get(a)), text(e.get(b)))).filter(|(x, _)| !x.trim().is_empty()).collect(),
        _ => Vec::new(),
    }
}

fn pairs_value(list: &[(String, String)], a: &str, b: &str) -> Value {
    Value::Array(list.iter().map(|(x, y)| json!({ a: x, b: y })).collect())
}

/// What a game's settings would save as `name`.
pub fn capture(name: &str, settings: &Map<String, Value>) -> Preset {
    Preset {
        name: name.trim().to_string(),
        env: pairs(settings, "enviromentOptions", "key", "value"),
        wrappers: pairs(settings, "wrapperOptions", "exe", "args"),
        args: settings.get("launcherArgs").and_then(Value::as_str).unwrap_or("").trim().to_string(),
    }
}

/// The settings to write so the game gets `preset` (only the keys that change).
pub fn apply(preset: &Preset, settings: &Map<String, Value>) -> Vec<(String, Value)> {
    let mut out = Vec::new();
    let env = pairs(settings, "enviromentOptions", "key", "value");
    let mut merged = env.clone();
    for (k, v) in &preset.env {
        match merged.iter_mut().find(|(key, _)| key == k) {
            Some(slot) => slot.1 = v.clone(),
            None => merged.push((k.clone(), v.clone())),
        }
    }
    if merged != env {
        out.push(("enviromentOptions".into(), pairs_value(&merged, "key", "value")));
    }
    let wrappers = pairs(settings, "wrapperOptions", "exe", "args");
    let mut more = wrappers.clone();
    for w in &preset.wrappers {
        if !more.contains(w) {
            more.push(w.clone());
        }
    }
    if more != wrappers {
        out.push(("wrapperOptions".into(), pairs_value(&more, "exe", "args")));
    }
    let args = settings.get("launcherArgs").and_then(Value::as_str).unwrap_or("").trim().to_string();
    let extra = preset.args.trim();
    if !extra.is_empty() && !args.contains(extra) {
        let joined = if args.is_empty() { extra.to_string() } else { format!("{args} {extra}") };
        out.push(("launcherArgs".into(), Value::String(joined)));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map(v: Value) -> Map<String, Value> {
        v.as_object().unwrap().clone()
    }

    #[test]
    fn capture_then_apply_adds_without_doubling() {
        let source = map(json!({
            "enviromentOptions": [{"key": "PROTON_ENABLE_NVAPI", "value": "1"}, {"key": "", "value": "x"}],
            "wrapperOptions": [{"exe": "prime-run", "args": ""}],
            "launcherArgs": " -dx11 "
        }));
        let p = capture(" DLSS ", &source);
        assert_eq!(p.name, "DLSS");
        assert_eq!(p.env, [("PROTON_ENABLE_NVAPI".to_string(), "1".to_string())]);
        assert_eq!(p.args, "-dx11");
        // Applied to the same game: nothing changes.
        assert!(apply(&p, &source).is_empty());
        let other = map(json!({
            "enviromentOptions": [{"key": "PROTON_ENABLE_NVAPI", "value": "0"}, {"key": "A", "value": "b"}],
            "launcherArgs": "-windowed"
        }));
        let changes = apply(&p, &other);
        assert_eq!(changes[0].1, json!([{"key": "PROTON_ENABLE_NVAPI", "value": "1"}, {"key": "A", "value": "b"}]));
        assert_eq!(changes[1].1, json!([{"exe": "prime-run", "args": ""}]));
        assert_eq!(changes[2], ("launcherArgs".to_string(), json!("-windowed -dx11")));
        assert!(Preset::default().is_empty());
    }

    #[test]
    fn presets_are_saved_by_name() {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path().join("data"));
        assert!(load(&paths).is_empty());
        put(&paths, Preset { name: "b".into(), args: "-x".into(), ..Default::default() }).unwrap();
        put(&paths, Preset { name: "A".into(), args: "-y".into(), ..Default::default() }).unwrap();
        put(&paths, Preset { name: "a".into(), args: "-z".into(), ..Default::default() }).unwrap();
        let names: Vec<String> = load(&paths).into_iter().map(|p| p.name).collect();
        assert_eq!(names, ["a", "b"], "same name in another case replaces it");
        remove(&paths, "b").unwrap();
        assert_eq!(load(&paths).len(), 1);
    }

    #[test]
    fn a_file_that_cannot_be_read_is_never_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path().join("data"));
        let broken = r#"{"presets": [{"name": "DLSS", "args": "-dx12"}"#;
        std::fs::create_dir_all(paths.local_library()).unwrap();
        std::fs::write(path(&paths), broken).unwrap();
        assert!(load(&paths).is_empty());
        assert!(put(&paths, Preset { name: "new".into(), ..Default::default() }).is_err());
        assert!(remove(&paths, "DLSS").is_err());
        assert_eq!(std::fs::read_to_string(path(&paths)).unwrap(), broken);
    }
}
