//! `nxm://` links, what the "Mod manager download" button of nexusmods.com
//! opens.

use std::fmt;

use heronite_data::paths::percent_decode;
use heronite_i18n::tr;

/// `nxm://<domain>/mods/<mod>/files/<file>?key=..&expires=..&user_id=..`
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NxmLink {
    /// Nexus game domain (`skyrimspecialedition`), lower case.
    pub domain: String,
    pub mod_id: u64,
    pub file_id: u64,
    /// One-time key that lets a free account get the download link.
    pub key: Option<String>,
    /// Unix seconds the key stops working.
    pub expires: Option<u64>,
    pub user_id: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NxmError {
    /// A collection revision link (`nxm://<domain>/collections/<slug>/revisions/<n>`).
    Collection,
    Invalid,
}

impl fmt::Display for NxmError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            NxmError::Collection => f.write_str(tr!("Nexus Mods collections are not supported yet: install their mods one by one")),
            NxmError::Invalid => f.write_str(tr!("This is not a Nexus Mods download link")),
        }
    }
}

impl std::error::Error for NxmError {}

const SCHEME: &str = "nxm://";

pub fn is_nxm(url: &str) -> bool {
    let url = url.trim();
    url.get(..SCHEME.len()).is_some_and(|s| s.eq_ignore_ascii_case(SCHEME))
}

pub fn parse_nxm(url: &str) -> Result<NxmLink, NxmError> {
    let url = url.trim();
    if !is_nxm(url) {
        return Err(NxmError::Invalid);
    }
    let rest = &url[SCHEME.len()..];
    let rest = rest.split('#').next().unwrap_or("");
    let (path, query) = rest.split_once('?').unwrap_or((rest, ""));
    let parts: Vec<&str> = path.trim_matches('/').split('/').collect();
    let domain = parts.first().filter(|d| !d.is_empty()).map(|d| d.to_ascii_lowercase()).ok_or(NxmError::Invalid)?;
    let keyword = |i: usize, word: &str| parts.get(i).is_some_and(|p| p.eq_ignore_ascii_case(word));
    if keyword(1, "collections") && keyword(3, "revisions") && parts.len() == 5 {
        return Err(NxmError::Collection);
    }
    if !(parts.len() == 5 && keyword(1, "mods") && keyword(3, "files")) {
        return Err(NxmError::Invalid);
    }
    let id = |s: &str| s.parse::<u64>().map_err(|_| NxmError::Invalid);
    let mut link = NxmLink { domain, mod_id: id(parts[2])?, file_id: id(parts[4])?, key: None, expires: None, user_id: None };
    for pair in query.split('&') {
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        let v = percent_decode(&v.replace('+', " "));
        match k.to_ascii_lowercase().as_str() {
            "key" if !v.is_empty() => link.key = Some(v),
            "expires" => link.expires = v.parse().ok(),
            "user_id" => link.user_id = v.parse().ok(),
            _ => {}
        }
    }
    Ok(link)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_download_button_link() {
        let l = parse_nxm("nxm://skyrimspecialedition/mods/12604/files/35407?key=Ab-1_x&expires=1790000000&user_id=4242").unwrap();
        assert_eq!(
            l,
            NxmLink {
                domain: "skyrimspecialedition".into(),
                mod_id: 12604,
                file_id: 35407,
                key: Some("Ab-1_x".into()),
                expires: Some(1_790_000_000),
                user_id: Some(4242),
            }
        );
    }

    #[test]
    fn scheme_case_trailing_slash_and_no_query() {
        let l = parse_nxm("  NXM://Stardewvalley/mods/1/files/2/ ").unwrap();
        assert_eq!((l.domain.as_str(), l.mod_id, l.file_id, l.key, l.expires), ("stardewvalley", 1, 2, None, None));
        assert!(is_nxm("Nxm://x") && !is_nxm("https://nxm://x") && !is_nxm("nx"));
    }

    #[test]
    fn keys_are_percent_decoded() {
        let l = parse_nxm("nxm://g/mods/1/files/2?key=a%2Bb%3D&expires=5").unwrap();
        assert_eq!(l.key.as_deref(), Some("a+b="));
    }

    #[test]
    fn collections_are_recognised_and_the_rest_rejected() {
        assert_eq!(parse_nxm("nxm://skyrimspecialedition/collections/abc123/revisions/7"), Err(NxmError::Collection));
        for bad in [
            "https://skyrimspecialedition/mods/1/files/2",
            "nxm://",
            "nxm://g",
            "nxm://g/mods/1",
            "nxm://g/mods/x/files/2",
            "nxm://g/mods/1/files/2/extra",
            "nxm://g/collections/abc",
        ] {
            assert_eq!(parse_nxm(bad), Err(NxmError::Invalid), "{bad}");
        }
    }
}
