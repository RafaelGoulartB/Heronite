//! Looks saved under a name: four built in, the user's own in
//! `appearance.json` (`profiles`), and a file format to share one.

use std::path::Path;

use anyhow::{bail, Context, Result};
use serde_json::{json, Value};

use super::*;

/// A built-in look. `id` is stable; the app translates the names.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Preset {
    pub id: &'static str,
    pub look: Appearance,
}

/// The built-in looks; the first is today's.
pub fn presets() -> [Preset; 4] {
    let d = Appearance::DEFAULT;
    [
        Preset { id: "default", look: d },
        Preset {
            id: "cinema",
            look: Appearance {
                background: BackgroundSource::Screenshot,
                background_slideshow: true,
                cover_hover: CoverHover::Glow,
                cover_label: CoverLabel::Nothing,
                cover_corners: Corners::Round,
                accent: Accent::Game,
                ..d
            },
        },
        Preset {
            id: "minimal",
            look: Appearance {
                interface_mode: InterfaceMode::Solid,
                solid_tint: SolidTint::Graphite,
                background: BackgroundSource::Solid,
                cover_badges: false,
                cover_hover: CoverHover::Nothing,
                cover_corners: Corners::Square,
                accent: Accent::Mint,
                density: Density::Compact,
                motion: Motion::Reduced,
                ..d
            },
        },
        Preset {
            id: "contrast",
            look: Appearance {
                high_contrast: true,
                accent: Accent::Amber,
                interface_opacity: 96.,
                collection_opacity: 70.,
                cover_label: CoverLabel::Title,
                cover_badges: true,
                motion: Motion::Reduced,
                ..d
            },
        },
    ]
}

/// A look the user saved.
#[derive(Clone, Debug, PartialEq)]
pub struct Profile {
    pub name: String,
    pub look: Appearance,
}

/// Marker key of an exported look (its value is the format version).
const EXPORT_KEY: &str = "heroniteAppearance";

/// The user's profiles, in the order they were saved. Entries without a
/// name are skipped.
pub fn load_profiles(paths: &AppPaths) -> Vec<Profile> {
    let Ok(doc) = json_store::read_value(&paths.appearance()) else { return Vec::new() };
    doc.get("profiles")
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .filter_map(|v| {
                    let name = v.get("name").and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty())?;
                    Some(Profile { name: name.to_owned(), look: Appearance::from_value(v) })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Replaces the stored profiles; the rest of the file is kept.
pub fn save_profiles(paths: &AppPaths, profiles: &[Profile]) -> Result<()> {
    let list: Vec<Value> = profiles
        .iter()
        .map(|p| {
            let mut v = p.look.to_value();
            v["name"] = json!(p.name);
            v
        })
        .collect();
    json_store::update_value(&paths.appearance(), |doc| {
        if !doc.is_object() {
            *doc = Value::Object(Default::default());
        }
        doc["profiles"] = Value::Array(list);
        Ok(())
    })
}

/// Writes `look` to `path` as a file another install can import.
pub fn export_look(path: &Path, name: &str, look: &Appearance) -> Result<()> {
    let mut v = look.to_value();
    v[EXPORT_KEY] = json!(1);
    v["name"] = json!(name);
    json_store::write(path, &v)
}

/// Reads a file written by [`export_look`]: the look and its name (the file
/// name when it has none).
pub fn import_look(path: &Path) -> Result<(String, Appearance)> {
    let v = json_store::read_value(path).with_context(|| format!("reading {}", path.display()))?;
    if v.get(EXPORT_KEY).is_none() {
        bail!("{} is not an exported look", path.display());
    }
    let name = v
        .get("name")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .or_else(|| path.file_stem().map(|s| s.to_string_lossy().into_owned()))
        .unwrap_or_default();
    Ok((name, Appearance::from_value(&v)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths(dir: &Path) -> AppPaths {
        AppPaths { config: dir.join("cfg"), cache: dir.join("cache") }
    }

    #[test]
    fn first_preset_is_todays_look() {
        assert_eq!(presets()[0].look, Appearance::DEFAULT);
        let ids: Vec<_> = presets().iter().map(|p| p.id).collect();
        assert_eq!(ids, ["default", "cinema", "minimal", "contrast"]);
    }

    #[test]
    fn profiles_live_next_to_the_settings() {
        let dir = tempfile::tempdir().unwrap();
        let p = paths(dir.path());
        assert!(load_profiles(&p).is_empty());
        let look = Appearance { accent: Accent::Rose, density: Density::Compact, ..Appearance::DEFAULT };
        look.save(&p).unwrap();
        let saved = vec![Profile { name: "Night".into(), look: presets()[1].look }];
        save_profiles(&p, &saved).unwrap();
        assert_eq!(load_profiles(&p), saved);
        // Saving the settings keeps the profiles, and the other way round.
        look.save(&p).unwrap();
        assert_eq!(load_profiles(&p), saved);
        assert_eq!(Appearance::load(&p), look);
    }

    #[test]
    fn export_and_import() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("Cozy.json");
        let look = Appearance { accent: Accent::Lavender, cover_hover: CoverHover::Lift, ..Appearance::DEFAULT };
        export_look(&file, "", &look).unwrap();
        let (name, back) = import_look(&file).unwrap();
        assert_eq!((name.as_str(), back), ("Cozy", look));
        std::fs::write(&file, "{\"accent\":\"rose\"}").unwrap();
        assert!(import_look(&file).is_err());
    }
}
