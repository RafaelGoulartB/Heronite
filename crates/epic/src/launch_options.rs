//! The ways an installed Epic game can start besides its default, read from
//! what legendary keeps on disk (no network, no legendary run):
//!
//! * the game's own options: `extraLaunchOption_<n>_Name` / `_Args` /
//!   `_PlatformRestriction` in the custom attributes of its metadata
//!   (`metadata/<app>.json`); options restricted to other platforms are left
//!   out;
//! * its installed DLCs (`installed.json`, matched to the game through
//!   `mainGameItem`): one with an executable of its own starts instead of the
//!   game; one with an `AdditionalCommandLine` passes those arguments to it.
//!
//! The pick is stored with [`heronite_launch::launch_option`].

use std::collections::BTreeMap;
use std::path::Path;

use heronite_data::{json_store, DataPaths};
pub use heronite_launch::launch_option::LaunchOption;
use serde_json::Value;

fn metadata(dir: &Path, app_name: &str) -> Option<Value> {
    json_store::read_value(&dir.join("metadata").join(format!("{app_name}.json"))).ok().and_then(|v| v.get("metadata").cloned())
}

fn attr<'a>(meta: &'a Value, name: &str) -> Option<&'a str> {
    meta.pointer(&format!("/customAttributes/{name}/value")).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty())
}

/// `extraLaunchOption_<n>_*` of a game's metadata, in their order.
fn own_options(meta: &Value, platform: &str) -> Vec<LaunchOption> {
    let Some(attrs) = meta.get("customAttributes").and_then(Value::as_object) else { return Vec::new() };
    let mut numbers: BTreeMap<String, ()> = BTreeMap::new();
    for key in attrs.keys() {
        if let Some(n) = key.strip_prefix("extraLaunchOption_").and_then(|r| r.split('_').next()) {
            numbers.insert(n.to_string(), ());
        }
    }
    numbers
        .keys()
        .filter_map(|n| {
            let name = attr(meta, &format!("extraLaunchOption_{n}_Name"))?.to_string();
            let parameters = attr(meta, &format!("extraLaunchOption_{n}_Args")).unwrap_or_default().to_string();
            let allowed = attr(meta, &format!("extraLaunchOption_{n}_PlatformRestriction"))
                .is_none_or(|r| r.split(',').any(|p| p.trim().eq_ignore_ascii_case(platform)));
            allowed.then_some(LaunchOption::Basic { name, parameters })
        })
        .collect()
}

/// Every option of the installed game `app_name` (`platform` as installed,
/// usually `Windows`). Empty when it has none or is not installed.
pub fn list(data_paths: &DataPaths, app_name: &str) -> Vec<LaunchOption> {
    let dir = data_paths.legendary_config_dir();
    let installed = json_store::read_value(&dir.join("installed.json")).unwrap_or(Value::Null);
    let Some(entry) = installed.get(app_name) else { return Vec::new() };
    let platform = entry.get("platform").and_then(Value::as_str).unwrap_or("Windows");
    let Some(meta) = metadata(&dir, app_name) else { return Vec::new() };
    let mut out = own_options(&meta, platform);
    let Some(game_id) = meta.get("id").and_then(Value::as_str) else { return out };
    let Some(all) = installed.as_object() else { return out };
    for (dlc_app, dlc) in all {
        if dlc_app == app_name || dlc.get("is_dlc").and_then(Value::as_bool) != Some(true) {
            continue;
        }
        let Some(dlc_meta) = metadata(&dir, dlc_app) else { continue };
        if dlc_meta.pointer("/mainGameItem/id").and_then(Value::as_str) != Some(game_id) {
            continue;
        }
        let title = dlc.get("title").and_then(Value::as_str).unwrap_or_default().to_string();
        if dlc.get("executable").and_then(Value::as_str).is_some_and(|e| !e.trim().is_empty()) {
            out.push(LaunchOption::Dlc { app_name: dlc_app.clone(), title });
        } else if let Some(args) = attr(&dlc_meta, "AdditionalCommandLine") {
            out.push(LaunchOption::Basic { name: title, parameters: args.to_string() });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn write(dir: &Path, rel: &str, v: Value) {
        let p = dir.join(rel);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, v.to_string()).unwrap();
    }

    #[test]
    fn options_come_from_the_metadata_and_the_installed_dlcs() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(tmp.path());
        let dir = paths.legendary_config_dir();
        let a = |v: &str| serde_json::json!({"type": "STRING", "value": v});
        write(
            &dir,
            "installed.json",
            serde_json::json!({
                "Penguin": {"title": "GNOG", "platform": "Windows", "executable": "GNOG.exe"},
                "Editor": {"title": "GNOG Editor", "is_dlc": true, "executable": "Editor.exe"},
                "Story": {"title": "Story Mode", "is_dlc": true, "executable": ""},
                "Other": {"title": "Not ours", "is_dlc": true, "executable": "x.exe"},
            }),
        );
        write(
            &dir,
            "metadata/Penguin.json",
            serde_json::json!({"metadata": {"id": "gnog-id", "customAttributes": {
                "extraLaunchOption_002_Name": a("Launch in Eyegaze"), "extraLaunchOption_002_Args": a("--eyegaze"),
                "extraLaunchOption_001_Name": a("Launch in VR"), "extraLaunchOption_001_Args": a("--launchvr"),
                "extraLaunchOption_001_PlatformRestriction": a("Windows"),
                "extraLaunchOption_003_Name": a("Mac only"), "extraLaunchOption_003_PlatformRestriction": a("Mac"),
            }}}),
        );
        write(&dir, "metadata/Editor.json", serde_json::json!({"metadata": {"mainGameItem": {"id": "gnog-id"}}}));
        write(
            &dir,
            "metadata/Story.json",
            serde_json::json!({"metadata": {"mainGameItem": {"id": "gnog-id"}, "customAttributes": {"AdditionalCommandLine": a("-story")}}}),
        );
        write(&dir, "metadata/Other.json", serde_json::json!({"metadata": {"mainGameItem": {"id": "someone-else"}}}));

        let names: Vec<String> = list(&paths, "Penguin").iter().map(|o| o.label().to_string()).collect();
        assert_eq!(names, ["Launch in VR", "Launch in Eyegaze", "GNOG Editor", "Story Mode"]);
        let all = list(&paths, "Penguin");
        assert_eq!(all[0], LaunchOption::Basic { name: "Launch in VR".into(), parameters: "--launchvr".into() });
        assert_eq!(all[2], LaunchOption::Dlc { app_name: "Editor".into(), title: "GNOG Editor".into() });
        assert_eq!(all[3], LaunchOption::Basic { name: "Story Mode".into(), parameters: "-story".into() });
        assert!(list(&paths, "Missing").is_empty(), "not installed");
    }
}
