//! The graphics card that draws Heronite's own windows, kept in
//! `AppPaths::interface_gpu()`: it belongs to this machine, not to the
//! library. Unset (the default), GPUI picks the card the compositor draws on.
//!
//! A card the compositor cannot show leaves the window black, with no way to
//! reach the setting again. So a new choice is on trial: the first start with
//! it marks it `tried`, and a start that finds it tried but not `confirmed`
//! (the user never said it looks right) goes back to automatic.
//!
//! - `device`: the card's PCI ids, `vendor:device` in hex (`10de:25a2`);
//! - `confirmed`: the user kept it after seeing it;
//! - `tried`: a start already drew with it.

use anyhow::Result;
use serde_json::{json, Value};

use crate::json_store;
use crate::paths::AppPaths;

/// A graphics card by its PCI ids.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InterfaceGpu {
    pub vendor_id: u16,
    pub device_id: u16,
}

impl InterfaceGpu {
    /// `10de:25a2`.
    pub fn parse(s: &str) -> Option<Self> {
        let (vendor, device) = s.trim().split_once(':')?;
        let hex = |t: &str| u16::from_str_radix(t.trim().trim_start_matches("0x"), 16).ok();
        Some(Self { vendor_id: hex(vendor)?, device_id: hex(device)? })
    }

    pub fn id(self) -> String {
        format!("{:04x}:{:04x}", self.vendor_id, self.device_id)
    }
}

/// The saved choice.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Saved {
    pub gpu: InterfaceGpu,
    pub confirmed: bool,
    pub tried: bool,
}

/// The saved card; `None`: automatic (also for a missing or unreadable file).
pub fn load(paths: &AppPaths) -> Option<Saved> {
    let value = json_store::read_value(&paths.interface_gpu()).ok()?;
    let flag = |key: &str| value.get(key).and_then(Value::as_bool).unwrap_or(false);
    Some(Saved { gpu: InterfaceGpu::parse(value.get("device")?.as_str()?)?, confirmed: flag("confirmed"), tried: flag("tried") })
}

/// Saves a new choice (`None`: automatic), on trial until [`confirm`].
pub fn save(paths: &AppPaths, gpu: Option<InterfaceGpu>) -> Result<()> {
    write(paths, gpu.map(|gpu| Saved { gpu, confirmed: false, tried: false }))
}

/// A start draws with the card on trial.
pub fn mark_tried(paths: &AppPaths, saved: Saved) -> Result<()> {
    write(paths, Some(Saved { tried: true, ..saved }))
}

/// The user kept the card.
pub fn confirm(paths: &AppPaths, saved: Saved) -> Result<()> {
    write(paths, Some(Saved { confirmed: true, ..saved }))
}

/// Writes `saved` as it is (`None`: automatic), keeping the other keys of
/// the file.
pub fn write(paths: &AppPaths, saved: Option<Saved>) -> Result<()> {
    json_store::update_value(&paths.interface_gpu(), |doc| {
        if !doc.is_object() {
            *doc = json!({});
        }
        let obj = doc.as_object_mut().expect("object");
        for key in ["device", "confirmed", "tried"] {
            obj.remove(key);
        }
        if let Some(s) = saved {
            obj.insert("device".into(), Value::from(s.gpu.id()));
            obj.insert("confirmed".into(), Value::from(s.confirmed));
            obj.insert("tried".into(), Value::from(s.tried));
        }
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_parse_and_print() {
        let g = InterfaceGpu::parse("10de:25A2").unwrap();
        assert_eq!(g, InterfaceGpu { vendor_id: 0x10de, device_id: 0x25a2 });
        assert_eq!(g.id(), "10de:25a2");
        assert_eq!(InterfaceGpu::parse("0x8086:0x46a6").unwrap().device_id, 0x46a6);
        assert_eq!(InterfaceGpu::parse("10de"), None);
        assert_eq!(InterfaceGpu::parse("zz:25a2"), None);
    }

    #[test]
    fn a_choice_is_on_trial_until_confirmed_and_other_keys_stay() {
        let dir = tempfile::tempdir().unwrap();
        let p = AppPaths { config: dir.path().join("cfg"), cache: dir.path().join("cache") };
        assert_eq!(load(&p), None);
        std::fs::create_dir_all(&p.config).unwrap();
        std::fs::write(p.interface_gpu(), r#"{"future": 1}"#).unwrap();
        let gpu = InterfaceGpu { vendor_id: 0x8086, device_id: 0x46a6 };
        save(&p, Some(gpu)).unwrap();
        let s = load(&p).unwrap();
        assert_eq!(s, Saved { gpu, confirmed: false, tried: false });
        mark_tried(&p, s).unwrap();
        assert_eq!(load(&p).unwrap(), Saved { gpu, confirmed: false, tried: true });
        confirm(&p, load(&p).unwrap()).unwrap();
        assert_eq!(load(&p).unwrap(), Saved { gpu, confirmed: true, tried: true });
        save(&p, None).unwrap();
        assert_eq!(load(&p), None);
        assert_eq!(json_store::read_value(&p.interface_gpu()).unwrap()["future"], 1);
        std::fs::write(p.interface_gpu(), r#"{"device": 3}"#).unwrap();
        assert_eq!(load(&p), None);
    }
}
