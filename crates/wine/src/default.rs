//! The global default Wine/Proton. Heroic fills `wineVersion` with the first
//! Wine it finds; here a new setup gets the newest Proton found (one from the
//! Wine Manager, Steam's `compatibilitytools.d`, ...) as soon as there is one,
//! so the first Windows game launches without a trip to the settings.

use std::cmp::Ordering;
use std::path::Path;

use heronite_data::{game_config, DataPaths};

use crate::detect::list_installations;
use crate::types::WineInstallation;

/// Whether `w` can run games: Proton needs its script, Wine its `wineserver`
/// too (like `validWine` at launch).
pub fn usable(w: &WineInstallation) -> bool {
    if w.bin.is_empty() || !Path::new(&w.bin).exists() {
        return false;
    }
    match w.kind.as_str() {
        "proton" => true,
        "wine" => {
            let sibling = Path::new(&w.bin).with_file_name("wineserver");
            w.wineserver.as_deref().is_some_and(|s| Path::new(s).exists()) || sibling.exists()
        }
        _ => false,
    }
}

/// The build a new setup should use: Proton before Wine, GE-Proton before
/// other Protons, then the newest version.
pub fn best(list: &[WineInstallation]) -> Option<&WineInstallation> {
    let rank = |w: &WineInstallation| -> u8 {
        match w.kind.as_str() {
            "proton" if w.name.starts_with("GE-Proton") => 3,
            "proton" => 2,
            "wine" => 1,
            _ => 0,
        }
    };
    list.iter().filter(|w| rank(w) > 0).max_by(|a, b| rank(a).cmp(&rank(b)).then_with(|| natural_cmp(&a.name, &b.name)))
}

/// Compares names with their numbers as numbers (`GE-Proton10-3` after `GE-Proton9-27`).
fn natural_cmp(a: &str, b: &str) -> Ordering {
    let (mut a, mut b) = (a.as_bytes(), b.as_bytes());
    loop {
        match (a.first(), b.first()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(x), Some(y)) if x.is_ascii_digit() && y.is_ascii_digit() => {
                let na = a.iter().take_while(|c| c.is_ascii_digit()).count();
                let nb = b.iter().take_while(|c| c.is_ascii_digit()).count();
                let (da, db) = (trim_zeros(&a[..na]), trim_zeros(&b[..nb]));
                let ord = da.len().cmp(&db.len()).then_with(|| da.cmp(db));
                if ord != Ordering::Equal {
                    return ord;
                }
                (a, b) = (&a[na..], &b[nb..]);
            }
            (Some(x), Some(y)) => {
                let ord = x.to_ascii_lowercase().cmp(&y.to_ascii_lowercase());
                if ord != Ordering::Equal {
                    return ord;
                }
                (a, b) = (&a[1..], &b[1..]);
            }
        }
    }
}

fn trim_zeros(d: &[u8]) -> &[u8] {
    let i = d.iter().take_while(|c| **c == b'0').count();
    &d[i.min(d.len().saturating_sub(1))..]
}

/// The configured default when it can run games.
pub fn current(data_paths: &DataPaths) -> Option<WineInstallation> {
    let home = home();
    let global = game_config::load_global(data_paths, &home);
    global.get("wineVersion").and_then(|v| serde_json::from_value::<WineInstallation>(v.clone()).ok()).filter(usable)
}

/// Sets the default to [`best`] when none can run games; the build set.
/// Blocking (scans folders): call off the UI thread.
pub fn ensure_default(data_paths: &DataPaths) -> Option<WineInstallation> {
    if current(data_paths).is_some() {
        return None;
    }
    let list = list_installations(data_paths);
    let pick = best(&list)?.clone();
    let value = serde_json::to_value(&pick).ok()?;
    if let Err(e) = game_config::set_global_setting(data_paths, &home(), "wineVersion", value) {
        log::warn!("could not set the default Wine to {}: {e:#}", pick.name);
        return None;
    }
    log::info!("default Wine set to {} ({})", pick.name, pick.bin);
    Some(pick)
}

fn home() -> String {
    dirs::home_dir().map(|h| h.to_string_lossy().into_owned()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn w(name: &str, kind: &str) -> WineInstallation {
        WineInstallation { bin: format!("/x/{name}"), name: name.into(), kind: kind.into(), ..Default::default() }
    }

    #[test]
    fn newest_ge_proton_wins() {
        let list = [w("wine-ge-8-26", "wine"), w("GE-Proton9-27", "proton"), w("GE-Proton10-3", "proton"), w("Proton-CachyOS", "proton")];
        assert_eq!(best(&list).unwrap().name, "GE-Proton10-3");
    }

    #[test]
    fn proton_before_wine_and_wine_when_alone() {
        assert_eq!(best(&[w("wine-11", "wine"), w("Proton 9.0", "proton")]).unwrap().name, "Proton 9.0");
        assert_eq!(best(&[w("wine-11", "wine")]).unwrap().name, "wine-11");
        assert!(best(&[w("CrossOver", "crossover")]).is_none());
        assert!(best(&[]).is_none());
    }

    #[test]
    fn numbers_compare_as_numbers() {
        assert_eq!(natural_cmp("GE-Proton10-3", "GE-Proton9-27"), Ordering::Greater);
        assert_eq!(natural_cmp("GE-Proton9-7", "GE-Proton9-27"), Ordering::Less);
        assert_eq!(natural_cmp("a01", "a1"), Ordering::Equal);
    }

    #[test]
    fn a_missing_binary_is_not_usable() {
        assert!(!usable(&w("GE-Proton10-3", "proton")));
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("proton");
        std::fs::write(&bin, "").unwrap();
        assert!(usable(&WineInstallation {
            bin: bin.to_string_lossy().into(),
            name: "P".into(),
            kind: "proton".into(),
            ..Default::default()
        }));
        let wine = dir.path().join("wine");
        std::fs::write(&wine, "").unwrap();
        let wine = WineInstallation { bin: wine.to_string_lossy().into(), name: "W".into(), kind: "wine".into(), ..Default::default() };
        assert!(!usable(&wine));
        std::fs::write(dir.path().join("wineserver"), "").unwrap();
        assert!(usable(&wine));
    }
}
