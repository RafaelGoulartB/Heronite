//! The interface language picked in Settings (`language` in
//! `AppPaths::appearance()`): `"system"` (the default), `"en"`, `"pt-BR"`, `"es"` or `"de"`.

use anyhow::Result;
use heronite_i18n::Preference;
use serde_json::Value;

use crate::json_store;
use crate::paths::AppPaths;

/// A missing or unreadable file follows the system.
pub fn load(paths: &AppPaths) -> Preference {
    json_store::read_value(&paths.appearance())
        .ok()
        .and_then(|v| v.get("language").and_then(Value::as_str).map(Preference::parse))
        .unwrap_or_default()
}

/// Writes the choice, keeping the other keys of the file.
pub fn save(paths: &AppPaths, pref: Preference) -> Result<()> {
    json_store::update_value(&paths.appearance(), |doc| {
        if !doc.is_object() {
            *doc = Value::Object(Default::default());
        }
        doc.as_object_mut().expect("object").insert("language".into(), Value::from(pref.as_str()));
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use heronite_i18n::Lang;

    #[test]
    fn round_trip_keeps_other_keys() {
        let dir = tempfile::tempdir().unwrap();
        let p = AppPaths { config: dir.path().join("cfg"), cache: dir.path().join("cache") };
        assert_eq!(load(&p), Preference::System);
        std::fs::create_dir_all(&p.config).unwrap();
        std::fs::write(p.appearance(), r#"{"interfaceBlur": 15}"#).unwrap();
        save(&p, Preference::Fixed(Lang::PtBr)).unwrap();
        assert_eq!(load(&p), Preference::Fixed(Lang::PtBr));
        assert_eq!(json_store::read_value(&p.appearance()).unwrap()["interfaceBlur"], 15);
        std::fs::write(p.appearance(), r#"{"language": 3}"#).unwrap();
        assert_eq!(load(&p), Preference::System);
    }
}
