//! Thunderstore (`thunderstore.io`): the listing of a community, packages,
//! their dependencies and `ror2mm://` install links. No account is needed.

mod api;
mod communities;
mod resolve;

use std::cmp;

use heronite_data::paths::percent_decode;
use heronite_i18n::tr;

pub use api::{package, search, version, DepRef, Listing, Ordering, Package, SearchPage, Version};
pub use communities::{communities, Community};
pub use resolve::{latest_versions, resolve, Planned};

/// `ror2mm://v1/install/thunderstore.io/<namespace>/<name>/<version>/`, what
/// the "Install with Mod Manager" button of a package page opens (the scheme
/// is r2modman's, from Risk of Rain 2 Mod Manager).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ror2mmLink {
    pub namespace: String,
    pub name: String,
    pub version: Option<String>,
}

const SCHEME: &str = "ror2mm://";
const PREFIX: &str = "v1/install/thunderstore.io/";

pub fn is_ror2mm(url: &str) -> bool {
    url.trim().get(..SCHEME.len()).is_some_and(|s| s.eq_ignore_ascii_case(SCHEME))
}

pub fn parse_ror2mm(url: &str) -> Result<Ror2mmLink, String> {
    let invalid = || tr!("This is not a Thunderstore install link").to_string();
    let url = url.trim();
    if !is_ror2mm(url) {
        return Err(invalid());
    }
    let rest = url[SCHEME.len()..].split(['?', '#']).next().unwrap_or("");
    let rest = rest.get(..PREFIX.len()).filter(|p| p.eq_ignore_ascii_case(PREFIX)).map(|_| &rest[PREFIX.len()..]).ok_or_else(invalid)?;
    let parts: Vec<String> = rest.trim_matches('/').split('/').map(percent_decode).collect();
    let word = |s: &str| !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
    match parts.as_slice() {
        [ns, name] if word(ns) && word(name) => Ok(Ror2mmLink { namespace: ns.clone(), name: name.clone(), version: None }),
        [ns, name, v] if word(ns) && word(name) && is_version(v) => {
            Ok(Ror2mmLink { namespace: ns.clone(), name: name.clone(), version: Some(v.clone()) })
        }
        _ => Err(invalid()),
    }
}

/// `1.2.3` style: dot separated numbers.
fn is_version(s: &str) -> bool {
    !s.is_empty() && s.split('.').all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
}

/// Numeric comparison of `a.b.c` versions; a missing part is 0 and a
/// non-numeric one (`1.2.3-beta`) counts by its leading digits.
pub fn compare_versions(a: &str, b: &str) -> cmp::Ordering {
    let parts = |v: &str| -> Vec<u64> {
        v.trim()
            .trim_start_matches(['v', 'V'])
            .split('.')
            .map(|p| p.chars().take_while(char::is_ascii_digit).collect::<String>().parse().unwrap_or(0))
            .collect()
    };
    let (a, b) = (parts(a), parts(b));
    (0..a.len().max(b.len()))
        .map(|i| a.get(i).unwrap_or(&0).cmp(b.get(i).unwrap_or(&0)))
        .find(|o| o.is_ne())
        .unwrap_or(cmp::Ordering::Equal)
}

/// BepInEx's own packs (`BepInEx-BepInExPack`, `bbepis-BepInExPack`,
/// `denikson-BepInExPack_Valheim`, …): they hold the loader, not a plugin, and
/// install into the game folder itself.
pub fn is_bepinex_pack(_namespace: &str, name: &str) -> bool {
    name.to_ascii_lowercase().starts_with("bepinexpack")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn install_links() {
        let l = parse_ror2mm("ror2mm://v1/install/thunderstore.io/notnotnotswipez/MoreCompany/1.14.0/").unwrap();
        assert_eq!(l, Ror2mmLink { namespace: "notnotnotswipez".into(), name: "MoreCompany".into(), version: Some("1.14.0".into()) });
        let no_version = parse_ror2mm(" ROR2MM://v1/install/thunderstore.io/BepInEx/BepInExPack ").unwrap();
        assert_eq!((no_version.namespace.as_str(), no_version.version), ("BepInEx", None));
        assert!(is_ror2mm("Ror2mm://x") && !is_ror2mm("nxm://x"));
    }

    #[test]
    fn bad_install_links() {
        for bad in [
            "https://thunderstore.io/package/a/b/",
            "ror2mm://v1/install/example.org/a/b/1.0.0/",
            "ror2mm://v2/install/thunderstore.io/a/b/1.0.0/",
            "ror2mm://v1/install/thunderstore.io/a/",
            "ror2mm://v1/install/thunderstore.io/a/b/1.0.0/extra",
            "ror2mm://v1/install/thunderstore.io/a/b/latest/",
            "ror2mm://v1/install/thunderstore.io/a/../1.0.0/",
            "ror2mm://v1/install/thunderstore.io//b/1.0.0/",
        ] {
            assert!(parse_ror2mm(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn versions_compare_numerically() {
        assert_eq!(compare_versions("1.10.0", "1.9.9"), cmp::Ordering::Greater);
        assert_eq!(compare_versions("1.2", "1.2.0"), cmp::Ordering::Equal);
        assert_eq!(compare_versions("5.4.2100", "5.4.2000"), cmp::Ordering::Greater);
        assert_eq!(compare_versions("v1.0.0", "1.0.1"), cmp::Ordering::Less);
        assert_eq!(compare_versions("2.0.0-beta", "2.0.0"), cmp::Ordering::Equal);
        assert_eq!(compare_versions("", "0.0.1"), cmp::Ordering::Less);
    }

    #[test]
    fn bepinex_packs() {
        assert!(is_bepinex_pack("BepInEx", "BepInExPack"));
        assert!(is_bepinex_pack("bbepis", "BepInExPack"));
        assert!(is_bepinex_pack("denikson", "BepInExPack_Valheim"));
        assert!(!is_bepinex_pack("x", "BepInEx_Tools"));
        assert!(!is_bepinex_pack("BepInEx", "MoreCompany"));
    }
}
