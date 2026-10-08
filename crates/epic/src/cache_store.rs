//! Heroic's `CacheStore` (`store_cache/<name>.json`): values next to a
//! `__timestamp.<key>` date, expired after a lifespan.

use anyhow::Result;
use heronite_data::json_store;
use heronite_data::HeroicPaths;
use serde_json::{Map, Value};

use crate::util::{js_date_now, parse_js_date};

pub struct CacheStore<'a> {
    heroic: &'a HeroicPaths,
    name: &'static str,
    /// Minutes; `None` never expires.
    lifespan: Option<u64>,
}

impl<'a> CacheStore<'a> {
    pub fn new(heroic: &'a HeroicPaths, name: &'static str, lifespan_minutes: Option<u64>) -> Self {
        Self { heroic, name, lifespan: lifespan_minutes }
    }

    /// `installStore` (6 hours).
    pub fn install_info(heroic: &'a HeroicPaths) -> Self {
        Self::new(heroic, "legendary_install_info", Some(60 * 6))
    }

    /// `libraryStore` (never expires).
    pub fn library(heroic: &'a HeroicPaths) -> Self {
        Self::new(heroic, "legendary_library", None)
    }

    /// `gamesOverrideStore` (7 days).
    pub fn games_override(heroic: &'a HeroicPaths) -> Self {
        Self::new(heroic, "legendary_games_override", Some(60 * 24 * 7))
    }

    pub fn path(&self) -> std::path::PathBuf {
        self.heroic.root.join("store_cache").join(format!("{}.json", self.name))
    }

    pub fn get(&self, key: &str) -> Option<Value> {
        self.get_at(key, now_secs())
    }

    fn get_at(&self, key: &str, now: i64) -> Option<Value> {
        let doc = json_store::read_value(&self.path()).ok()?;
        let value = doc.get(key)?.clone();
        if let Some(minutes) = self.lifespan {
            let stamp = doc.get("__timestamp")?.get(key)?.as_str()?;
            let at = parse_js_date(stamp)?;
            if (now - at) as f64 / 60. > minutes as f64 {
                return None;
            }
        }
        Some(value)
    }

    pub fn set(&self, key: &str, value: Value) -> Result<()> {
        json_store::update_value(&self.path(), |doc| {
            let obj = doc.as_object_mut().expect("object");
            obj.insert(key.to_string(), value);
            let stamps = obj.entry("__timestamp").or_insert_with(|| Value::Object(Map::new()));
            if let Value::Object(m) = stamps {
                m.insert(key.to_string(), Value::String(js_date_now()));
            }
            Ok(())
        })
    }

    pub fn delete(&self, key: &str) -> Result<()> {
        json_store::update_value(&self.path(), |doc| {
            if let Some(obj) = doc.as_object_mut() {
                obj.remove(key);
                if let Some(Value::Object(m)) = obj.get_mut("__timestamp") {
                    m.remove(key);
                }
            }
            Ok(())
        })
    }

    /// `store.clear()`: the file becomes `{}`.
    pub fn clear(&self) -> Result<()> {
        json_store::write(&self.path(), &Value::Object(Map::new()))
    }
}

fn now_secs() -> i64 {
    chrono::Utc::now().timestamp()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn values_expire_after_the_lifespan() {
        let dir = tempfile::tempdir().unwrap();
        let heroic = HeroicPaths::at(dir.path());
        let store = CacheStore::install_info(&heroic);
        assert!(store.get("a").is_none());
        store.set("a", json!({"manifest": {"download_size": 5}})).unwrap();
        assert_eq!(store.get("a").unwrap()["manifest"]["download_size"], 5);
        // Seven hours later the 6 hour entry is gone.
        assert!(store.get_at("a", now_secs() + 7 * 3600).is_none());
        let raw = std::fs::read_to_string(store.path()).unwrap();
        assert!(raw.contains("\"__timestamp\""), "{raw}");
        store.delete("a").unwrap();
        assert!(store.get("a").is_none());
    }

    #[test]
    fn library_store_never_expires() {
        let dir = tempfile::tempdir().unwrap();
        let heroic = HeroicPaths::at(dir.path());
        let store = CacheStore::library(&heroic);
        store.set("library", json!([1])).unwrap();
        assert!(store.get_at("library", now_secs() + 3650 * 86400).is_some());
    }
}
