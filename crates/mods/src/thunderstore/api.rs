//! Thunderstore's listing and package endpoints.

use heronite_i18n::tr;
use serde::Deserialize;

use crate::http::{self, null_default};

const BASE: &str = "https://thunderstore.io/api";

/// The `ordering` of a listing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Ordering {
    #[default]
    MostDownloaded,
    LastUpdated,
    TopRated,
    Newest,
}

impl Ordering {
    fn param(self) -> &'static str {
        match self {
            Ordering::MostDownloaded => "most-downloaded",
            Ordering::LastUpdated => "last-updated",
            Ordering::TopRated => "top-rated",
            Ordering::Newest => "newest",
        }
    }
}

/// One package of a community's listing.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Listing {
    pub namespace: String,
    pub name: String,
    pub description: String,
    pub icon_url: String,
    pub download_count: u64,
    pub rating_count: u64,
    /// RFC 3339.
    pub last_updated: String,
    /// Category names.
    pub categories: Vec<String>,
    pub is_deprecated: bool,
    pub is_nsfw: bool,
    pub is_pinned: bool,
    /// Bytes of the latest version.
    pub size: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SearchPage {
    /// Packages matching, over all pages.
    pub count: u64,
    pub has_next: bool,
    pub packages: Vec<Listing>,
}

#[derive(Deserialize)]
struct RawPage {
    #[serde(default)]
    count: u64,
    next: Option<String>,
    #[serde(default, deserialize_with = "null_default")]
    results: Vec<RawListing>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct RawListing {
    namespace: String,
    name: String,
    #[serde(deserialize_with = "null_default")]
    description: String,
    #[serde(deserialize_with = "null_default")]
    icon_url: String,
    download_count: u64,
    rating_count: u64,
    #[serde(deserialize_with = "null_default")]
    last_updated: String,
    #[serde(deserialize_with = "null_default")]
    categories: Vec<RawCategory>,
    is_deprecated: bool,
    is_nsfw: bool,
    is_pinned: bool,
    size: u64,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct RawCategory {
    name: String,
}

impl From<RawListing> for Listing {
    fn from(r: RawListing) -> Self {
        Listing {
            namespace: r.namespace,
            name: r.name,
            description: r.description,
            icon_url: r.icon_url,
            download_count: r.download_count,
            rating_count: r.rating_count,
            last_updated: r.last_updated,
            categories: r.categories.into_iter().map(|c| c.name).filter(|n| !n.is_empty()).collect(),
            is_deprecated: r.is_deprecated,
            is_nsfw: r.is_nsfw,
            is_pinned: r.is_pinned,
            size: r.size,
        }
    }
}

impl From<RawPage> for SearchPage {
    fn from(r: RawPage) -> Self {
        SearchPage {
            count: r.count,
            has_next: r.next.is_some(),
            packages: r
                .results
                .into_iter()
                .filter(|l| !l.namespace.is_empty() && !l.name.is_empty())
                .map(Listing::from)
                .filter(|l| !is_mod_manager(l))
                .collect(),
        }
    }
}

/// Mod managers published as packages (r2modman, Gale…): programs to run,
/// not mods to put in the game, so the manager does not offer them.
fn is_mod_manager(l: &Listing) -> bool {
    let name = l.name.to_ascii_lowercase();
    l.categories.iter().any(|c| c.eq_ignore_ascii_case("Tools"))
        && (name.contains("modman") || name.contains("modmanager") || name.contains("mod_manager"))
}

/// Packages of `community` matching `query` (empty: all), 1-based `page`.
/// Deprecated and NSFW packages are left out, as on the site, and so are mod
/// managers.
pub fn search(community: &str, query: &str, ordering: Ordering, page: u32) -> Result<SearchPage, String> {
    let url = format!(
        "{BASE}/cyberstorm/listing/{}/?q={}&ordering={}&page={}",
        http::encode(community),
        http::encode(query.trim()),
        ordering.param(),
        page.max(1)
    );
    let raw: RawPage = http::get_json(&url, &[])?;
    Ok(raw.into())
}

/// `Namespace-Name-1.2.3`, a dependency as the API writes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DepRef {
    pub namespace: String,
    pub name: String,
    pub version: String,
}

impl DepRef {
    /// Names never hold a `-`, so the three parts split cleanly; anything
    /// else is rejected.
    pub fn parse(s: &str) -> Option<DepRef> {
        let mut parts = s.trim().split('-');
        let (namespace, name, version) = (parts.next()?, parts.next()?, parts.next()?);
        let version_ok = !version.is_empty() && version.split('.').all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()));
        (parts.next().is_none() && !namespace.is_empty() && !name.is_empty() && version_ok).then(|| DepRef {
            namespace: namespace.to_string(),
            name: name.to_string(),
            version: version.to_string(),
        })
    }
}

/// One version of a package.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Version {
    pub version_number: String,
    pub description: String,
    pub icon: String,
    pub dependencies: Vec<DepRef>,
    pub download_url: String,
    pub website_url: Option<String>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct RawVersion {
    version_number: String,
    #[serde(deserialize_with = "null_default")]
    description: String,
    #[serde(deserialize_with = "null_default")]
    icon: String,
    #[serde(deserialize_with = "null_default")]
    dependencies: Vec<String>,
    #[serde(deserialize_with = "null_default")]
    download_url: String,
    website_url: Option<String>,
}

impl From<RawVersion> for Version {
    fn from(r: RawVersion) -> Self {
        let dependencies = r
            .dependencies
            .iter()
            .filter_map(|d| {
                let parsed = DepRef::parse(d);
                if parsed.is_none() {
                    log::warn!("thunderstore: skipping malformed dependency {d:?}");
                }
                parsed
            })
            .collect();
        Version {
            version_number: r.version_number,
            description: r.description,
            icon: r.icon,
            dependencies,
            download_url: r.download_url,
            website_url: r.website_url.map(|u| u.trim().to_string()).filter(|u| !u.is_empty()),
        }
    }
}

/// A package with its latest version.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Package {
    pub namespace: String,
    pub name: String,
    pub latest: Version,
    pub is_deprecated: bool,
    /// Identifiers of the communities listing it.
    pub communities: Vec<String>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct RawPackage {
    namespace: String,
    name: String,
    is_deprecated: bool,
    latest: Option<RawVersion>,
    #[serde(deserialize_with = "null_default")]
    community_listings: Vec<RawListingIn>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct RawListingIn {
    community: String,
}

impl TryFrom<RawPackage> for Package {
    type Error = String;

    fn try_from(r: RawPackage) -> Result<Self, String> {
        let latest = r.latest.ok_or_else(|| tr!("Thunderstore has no version of {name}", name = r.name))?;
        Ok(Package {
            namespace: r.namespace,
            name: r.name,
            latest: latest.into(),
            is_deprecated: r.is_deprecated,
            communities: r.community_listings.into_iter().map(|l| l.community).filter(|c| !c.is_empty()).collect(),
        })
    }
}

pub fn package(namespace: &str, name: &str) -> Result<Package, String> {
    let url = format!("{BASE}/experimental/package/{}/{}/", http::encode(namespace), http::encode(name));
    let raw: RawPackage = http::get_json(&url, &[])?;
    raw.try_into()
}

pub fn version(namespace: &str, name: &str, version: &str) -> Result<Version, String> {
    let url = format!("{BASE}/experimental/package/{}/{}/{}/", http::encode(namespace), http::encode(name), http::encode(version));
    let raw: RawVersion = http::get_json(&url, &[])?;
    Ok(raw.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mod_managers_are_not_offered() {
        let raw: RawPage = serde_json::from_str(
            r#"{"count":2,"next":null,"previous":null,"results":[
                {"categories":[{"id":"660","name":"Tools","slug":"tools"}],"name":"r2modman","namespace":"ebkr"},
                {"categories":[{"id":"660","name":"Tools","slug":"tools"}],"name":"LethalConfig","namespace":"AinaVT"}]}"#,
        )
        .unwrap();
        let names: Vec<String> = SearchPage::from(raw).packages.into_iter().map(|p| p.name).collect();
        assert_eq!(names, ["LethalConfig"]);
    }

    #[test]
    fn a_listing_page() {
        let raw: RawPage = serde_json::from_str(
            r#"{"count":2246,"next":"https://thunderstore.io/api/cyberstorm/listing/lethal-company/?page=2","previous":null,"results":[
                {"categories":[{"id":"658","name":"Mods","slug":"mods"},{"id":"682","name":"BepInEx","slug":"bepinex"}],
                 "community_identifier":"lethal-company","description":"A stable lobby player count expansion mod.",
                 "download_count":27023884,"icon_url":"https://ccdn.thunderstore.io/live/repository/icons/x.png","is_deprecated":false,
                 "is_nsfw":false,"is_pinned":true,"last_updated":"2026-04-25T01:28:42.076106Z","name":"MoreCompany",
                 "namespace":"notnotnotswipez","rating_count":641,"size":10530391,"datetime_created":"2023-11-16T04:46:28.298342Z"},
                {"name":"","namespace":"x"}]}"#,
        )
        .unwrap();
        let page = SearchPage::from(raw);
        assert_eq!((page.count, page.has_next, page.packages.len()), (2246, true, 1));
        let p = &page.packages[0];
        assert_eq!(
            (p.namespace.as_str(), p.name.as_str(), p.download_count, p.rating_count, p.size),
            ("notnotnotswipez", "MoreCompany", 27_023_884, 641, 10_530_391)
        );
        assert_eq!((p.categories.as_slice(), p.is_pinned, p.is_nsfw), (&["Mods".to_string(), "BepInEx".to_string()][..], true, false));
        let last: RawPage = serde_json::from_str(r#"{"count":0,"next":null,"previous":null,"results":[]}"#).unwrap();
        assert_eq!(SearchPage::from(last), SearchPage::default());
    }

    #[test]
    fn dependencies() {
        assert_eq!(
            DepRef::parse("BepInEx-BepInExPack-5.4.2100"),
            Some(DepRef { namespace: "BepInEx".into(), name: "BepInExPack".into(), version: "5.4.2100".into() })
        );
        for bad in ["", "BepInEx", "BepInEx-BepInExPack", "a-b-1.0.0-extra", "a-b-latest", "-b-1.0.0", "a--1.0.0", "a-b-1..0"] {
            assert_eq!(DepRef::parse(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn a_package() {
        let raw: RawPackage = serde_json::from_str(
            r#"{"namespace":"notnotnotswipez","name":"MoreCompany","full_name":"notnotnotswipez-MoreCompany","owner":"notnotnotswipez",
                "package_url":"https://thunderstore.io/package/notnotnotswipez/MoreCompany/","date_updated":"2026-04-25T01:28:42.076106Z",
                "rating_score":-1,"is_pinned":false,"is_deprecated":false,"total_downloads":-1,
                "latest":{"namespace":"notnotnotswipez","name":"MoreCompany","version_number":"1.14.0","description":"A stable lobby.",
                          "icon":"https://ccdn.thunderstore.io/i.png","dependencies":["BepInEx-BepInExPack-5.4.2100","broken"],
                          "download_url":"https://thunderstore.io/package/download/notnotnotswipez/MoreCompany/1.14.0/","downloads":970251,
                          "website_url":"https://github.com/notnotnotswipez/MoreCompany","is_active":true},
                "community_listings":[{"has_nsfw_content":false,"categories":["Mods"],"community":"lethal-company","review_status":"approved"}]}"#,
        )
        .unwrap();
        let p = Package::try_from(raw).unwrap();
        assert_eq!((p.latest.version_number.as_str(), p.communities.as_slice()), ("1.14.0", &["lethal-company".to_string()][..]));
        assert_eq!(p.latest.dependencies.len(), 1);
        assert_eq!(p.latest.download_url, "https://thunderstore.io/package/download/notnotnotswipez/MoreCompany/1.14.0/");
        assert!(Package::try_from(RawPackage::default()).is_err());
    }

    #[test]
    fn a_version_without_a_website() {
        let raw: RawVersion = serde_json::from_str(
            r#"{"namespace":"a","name":"b","version_number":"1.0.0","dependencies":[],"download_url":"u","downloads":1,"website_url":"","is_active":true}"#,
        )
        .unwrap();
        let v = Version::from(raw);
        assert_eq!((v.website_url, v.dependencies.len(), v.icon.as_str()), (None, 0, ""));
    }

    #[test]
    #[ignore = "network"]
    fn live_search_package_version() {
        let page = search("lethal-company", "morecompany", Ordering::MostDownloaded, 1).unwrap();
        assert!(page.packages.iter().any(|p| p.name == "MoreCompany"));
        let pkg = package("notnotnotswipez", "MoreCompany").unwrap();
        assert!(pkg.communities.iter().any(|c| c == "lethal-company"));
        let v = version("notnotnotswipez", "MoreCompany", "1.11.1").unwrap();
        assert_eq!(v.version_number, "1.11.1");
        assert!(package("NoSuch", "Thing").unwrap_err().contains("Not found"));
    }
}
