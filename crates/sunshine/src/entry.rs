//! One of Heronite's applications in Sunshine, and how it is told from the
//! user's own: the `heronite-id` key, which Sunshine keeps as it is (it
//! stores what it is sent, minus `index`) and its web UI sends back when the
//! entry is edited there.
//!
//! Heronite owns the keys it writes (`name`, `cmd`, `image-path`,
//! `prep-cmd`, `auto-detach`, `wait-all`, the marker); keys the user adds in
//! Sunshine's web UI (an output log, a working folder) stay.

use serde_json::{json, Map, Value};

/// The key that marks an application as Heronite's.
pub const MARKER: &str = "heronite-id";

/// What one of Heronite's applications should be.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// Stable id: `<runner>_<appName>` for a game, [`CONSOLE_ID`] for console mode.
    pub id: String,
    pub name: String,
    /// The command Sunshine runs (Heronite's launch path).
    pub cmd: String,
    /// A PNG on this computer; `None`: Sunshine's default box.
    pub image_path: Option<String>,
    /// Run when the stream's application ends (Sunshine's undo command).
    pub undo: Option<String>,
}

/// The id of the entry that opens console mode.
pub const CONSOLE_ID: &str = "console";

/// The marker of an application, when it is one of Heronite's.
pub fn marker(app: &Value) -> Option<&str> {
    app.get(MARKER).and_then(Value::as_str).filter(|s| !s.is_empty())
}

impl Entry {
    fn prep(&self) -> Option<Value> {
        self.undo.as_ref().map(|undo| json!([{ "do": "", "undo": undo, "elevated": false }]))
    }

    /// Writes Heronite's keys into `app`, keeping the others.
    pub fn apply_to(&self, app: &mut Map<String, Value>) {
        app.insert("name".into(), json!(self.name));
        app.insert("cmd".into(), json!(self.cmd));
        match &self.image_path {
            Some(p) => app.insert("image-path".into(), json!(p)),
            None => app.remove("image-path"),
        };
        // Sunshine drops an empty `prep-cmd` list when it saves.
        match self.prep() {
            Some(p) => app.insert("prep-cmd".into(), p),
            None => app.remove("prep-cmd"),
        };
        // Heronite hands the launch to its running instance and exits at once:
        // the stream must not end with it.
        app.insert("auto-detach".into(), json!(true));
        app.insert("wait-all".into(), json!(true));
        app.insert(MARKER.into(), json!(self.id));
    }

    /// A new application.
    pub fn to_app(&self) -> Value {
        let mut app = Map::new();
        self.apply_to(&mut app);
        Value::Object(app)
    }

    /// Whether `app` already holds what this entry writes.
    pub fn matches(&self, app: &Value) -> bool {
        let Some(obj) = app.as_object() else { return false };
        let mut wanted = obj.clone();
        self.apply_to(&mut wanted);
        // Older Sunshine versions stored booleans as strings: compare as written.
        let same = |k: &str| match (obj.get(k), wanted.get(k)) {
            (Some(Value::String(s)), Some(Value::Bool(b))) => s == if *b { "true" } else { "false" },
            (a, b) => a == b,
        };
        ["name", "cmd", "image-path", "auto-detach", "wait-all", MARKER].into_iter().all(same)
            && same_prep(obj.get("prep-cmd"), wanted.get("prep-cmd"))
    }
}

/// `prep-cmd` lists compare by their commands (`elevated` may come back as a string).
fn same_prep(a: Option<&Value>, b: Option<&Value>) -> bool {
    let cmds = |v: Option<&Value>| -> Vec<(String, String)> {
        v.and_then(Value::as_array)
            .into_iter()
            .flatten()
            .map(|c| {
                let s = |k: &str| c.get(k).and_then(Value::as_str).unwrap_or_default().to_string();
                (s("do"), s("undo"))
            })
            .collect()
    };
    cmds(a) == cmds(b)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry() -> Entry {
        Entry {
            id: "gog_1".into(),
            name: "Quail".into(),
            cmd: "heronite --launch gog_1".into(),
            image_path: Some("/covers/gog_1.png".into()),
            undo: Some("heronite stop".into()),
        }
    }

    #[test]
    fn apply_keeps_the_users_keys_and_marks_the_entry() {
        let mut app = json!({ "name": "Old", "output": "/tmp/log", "image-path": "/x.png" }).as_object().unwrap().clone();
        entry().apply_to(&mut app);
        assert_eq!(app["output"], "/tmp/log");
        assert_eq!(app["name"], "Quail");
        assert_eq!(app[MARKER], "gog_1");
        assert_eq!(app["prep-cmd"][0]["undo"], "heronite stop");
        assert_eq!(marker(&Value::Object(app)), Some("gog_1"));
        assert_eq!(marker(&json!({ "name": "Desktop" })), None);
        assert_eq!(marker(&json!({ MARKER: "" })), None);
    }

    #[test]
    fn no_image_or_undo_removes_those_keys() {
        let e = Entry { image_path: None, undo: None, ..entry() };
        let mut app = entry().to_app().as_object().unwrap().clone();
        e.apply_to(&mut app);
        assert!(!app.contains_key("image-path"));
        assert!(!app.contains_key("prep-cmd"));
    }

    #[test]
    fn matches_ignores_user_keys_and_old_string_booleans() {
        let e = entry();
        let mut app = e.to_app();
        app["output"] = json!("log.txt");
        assert!(e.matches(&app));
        app["auto-detach"] = json!("true");
        app["prep-cmd"][0]["elevated"] = json!("false");
        assert!(e.matches(&app));
        app["name"] = json!("Renamed");
        assert!(!e.matches(&app));
        let mut no_prep = e.to_app();
        no_prep.as_object_mut().unwrap().remove("prep-cmd");
        assert!(!e.matches(&no_prep));
    }
}
