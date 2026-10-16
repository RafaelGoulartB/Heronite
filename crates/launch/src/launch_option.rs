//! An Epic game's other ways to start (a VR mode, an editor, a playable
//! DLC): what the player picked, kept in the game's settings as
//! `lastUsedLaunchOption` in the JSON shape a migrated setup already has
//! (`{type: "basic", name, parameters}` / `{type: "dlc", dlcAppName,
//! dlcTitle}`). `heronite_epic::launch_options` lists them.

use serde_json::{json, Value};

/// The game settings key.
pub const KEY: &str = "lastUsedLaunchOption";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LaunchOption {
    /// The game with extra arguments.
    Basic { name: String, parameters: String },
    /// An installed DLC that is a program of its own: legendary launches it
    /// instead of the game.
    Dlc { app_name: String, title: String },
}

impl LaunchOption {
    pub fn label(&self) -> &str {
        match self {
            Self::Basic { name, .. } => name,
            Self::Dlc { title, app_name } => {
                if title.is_empty() {
                    app_name
                } else {
                    title
                }
            }
        }
    }

    pub fn to_value(&self) -> Value {
        match self {
            Self::Basic { name, parameters } => json!({"type": "basic", "name": name, "parameters": parameters}),
            Self::Dlc { app_name, title } => json!({"type": "dlc", "dlcAppName": app_name, "dlcTitle": title}),
        }
    }

    /// `None` for an empty value or a kind this app does not start.
    pub fn from_value(v: &Value) -> Option<Self> {
        let text = |k: &str| v.get(k).and_then(Value::as_str).unwrap_or_default().to_string();
        match v.get("type").and_then(Value::as_str) {
            Some("dlc") => {
                let app_name = text("dlcAppName");
                (!app_name.is_empty()).then(|| Self::Dlc { app_name, title: text("dlcTitle") })
            }
            // Old entries have no type: a basic option.
            Some("basic") | None => {
                let (name, parameters) = (text("name"), text("parameters"));
                (!name.is_empty() || !parameters.is_empty()).then_some(Self::Basic { name, parameters })
            }
            Some(_) => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_through_the_stored_json() {
        let vr = LaunchOption::Basic { name: "Launch in VR".into(), parameters: "--launchvr".into() };
        let dlc = LaunchOption::Dlc { app_name: "Editor".into(), title: "Unreal Editor".into() };
        for o in [&vr, &dlc] {
            assert_eq!(LaunchOption::from_value(&o.to_value()).as_ref(), Some(o));
        }
        assert_eq!(vr.label(), "Launch in VR");
        assert_eq!(dlc.label(), "Unreal Editor");
        assert_eq!(
            LaunchOption::from_value(&json!({"name": "Old", "parameters": "-x"})),
            Some(LaunchOption::Basic { name: "Old".into(), parameters: "-x".into() })
        );
        assert_eq!(LaunchOption::from_value(&json!({"type": "altExe", "executable": "a.exe"})), None);
        assert_eq!(LaunchOption::from_value(&json!({})), None);
        assert_eq!(LaunchOption::from_value(&Value::Null), None);
    }
}
