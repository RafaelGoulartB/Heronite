//! The builds of a game for one system (`/products/<id>/os/<os>/builds`) and
//! the one an install uses, picked the way gogdl picks it.

use anyhow::{anyhow, bail, Result};
use heronite_i18n::tr;
use serde_json::Value;

use crate::api::{self, Client};

/// One build of the list.
#[derive(Debug, Clone, PartialEq)]
pub struct Build {
    pub build_id: String,
    /// The id older (v1) builds are known by; what `gogdl info` printed as
    /// their `buildId`.
    pub legacy_build_id: String,
    pub version_name: String,
    /// `None` for the public, default branch.
    pub branch: Option<String>,
    /// 1 or 2.
    pub generation: u8,
    /// The build's meta document.
    pub link: String,
}

impl Build {
    fn parse(item: &Value) -> Option<Self> {
        let text = |k: &str| item.get(k).and_then(api::id_text).unwrap_or_default();
        Some(Self {
            build_id: text("build_id"),
            legacy_build_id: text("legacy_build_id"),
            version_name: text("version_name"),
            branch: item.get("branch").and_then(Value::as_str).map(str::to_string),
            generation: item.get("generation").and_then(Value::as_u64).unwrap_or(2) as u8,
            link: item.get("link").and_then(Value::as_str)?.to_string(),
        })
    }

    /// The id an install records: `build_id`, or `legacy_build_id` for v1.
    pub fn recorded_id(&self) -> &str {
        if self.generation == 1 && !self.legacy_build_id.is_empty() {
            &self.legacy_build_id
        } else {
            &self.build_id
        }
    }
}

/// The builds of a game for one system, newest first, as GOG sends them.
#[derive(Debug, Clone, PartialEq)]
pub struct Builds {
    pub items: Vec<Build>,
    /// The answer as it came, which `gogdl info` printed as `builds`.
    pub raw: Value,
}

impl Builds {
    pub fn parse(doc: &Value) -> Self {
        let items = doc.get("items").and_then(Value::as_array).map(|l| l.iter().filter_map(Build::parse).collect()).unwrap_or_default();
        Self { items, raw: doc.clone() }
    }

    /// gogdl's choice: the first build, else the newest of the default
    /// branch, else the newest of `branch`, and `build_id` (either id) over
    /// all of them.
    pub fn pick(&self, branch: Option<&str>, build_id: Option<&str>) -> Option<&Build> {
        let mut target = self.items.first()?;
        if let Some(b) = self.items.iter().find(|b| b.branch.is_none()) {
            target = b;
        }
        if let Some(b) = self.items.iter().find(|b| b.branch.as_deref() == branch) {
            target = b;
        }
        if let Some(id) = build_id.filter(|id| !id.is_empty()) {
            if let Some(b) = self.items.iter().find(|b| b.build_id == id || b.legacy_build_id == id) {
                target = b;
            }
        }
        Some(target)
    }

    /// Named branches, `None` (the default) first, as `available_branches`.
    pub fn branches(&self) -> Vec<Option<String>> {
        let mut out: Vec<Option<String>> = vec![None];
        for b in self.items.iter().filter_map(|b| b.branch.clone()) {
            if !out.contains(&Some(b.clone())) {
                out.push(Some(b));
            }
        }
        out
    }
}

/// Why a game cannot be installed for a system.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NoBuilds;

impl std::fmt::Display for NoBuilds {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("the game has no builds for this system")
    }
}

impl std::error::Error for NoBuilds {}

impl Client {
    /// The builds of `game_id` for `os` (`windows` or `osx`). A game the
    /// content system has none of is a [`NoBuilds`]. `password` opens a
    /// private branch.
    pub fn builds(&self, game_id: &str, os: &str, password: Option<&str>) -> Result<Builds> {
        let mut url = api::builds_url(game_id, os, 2);
        if let Some(p) = password.filter(|p| !p.is_empty()) {
            url.push_str(&format!("&password={}", api::encode(p, false)));
        }
        let resp = self.get(&url, &[("Accept", "application/json")])?;
        if resp.status == 404 {
            return Err(anyhow!(NoBuilds).context(tr!("This game cannot be installed for {platform}", platform = os)));
        }
        api::check(&resp, &url)?;
        let builds = Builds::parse(&resp.json()?);
        if builds.items.is_empty() {
            return Err(anyhow!(NoBuilds).context(tr!("This game cannot be installed for {platform}", platform = os)));
        }
        Ok(builds)
    }

    /// The meta document of `build` and its `ETag`.
    pub fn meta(&self, build: &Build) -> Result<(Value, Option<String>)> {
        let (doc, etag) = self.get_zlib_json(&build.link)?;
        if !doc.is_object() {
            bail!("the build of the game has no manifest");
        }
        Ok((doc, etag))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn doc() -> Value {
        json!({"total_count": 4, "items": [
            {"build_id": "40", "legacy_build_id": null, "version_name": "beta 2", "branch": "beta", "generation": 2, "link": "l40"},
            {"build_id": "30", "version_name": "1.2", "branch": null, "generation": 2, "link": "l30"},
            {"build_id": "20", "version_name": "1.1", "branch": null, "generation": 2, "link": "l20"},
            {"build_id": "10", "legacy_build_id": 777, "version_name": "1.0", "branch": null, "generation": 1, "link": "l10"},
            {"build_id": "bad"}
        ]})
    }

    #[test]
    fn the_default_branch_wins_unless_asked_otherwise() {
        let b = Builds::parse(&doc());
        assert_eq!(b.items.len(), 4);
        assert_eq!(b.pick(None, None).unwrap().build_id, "30");
        assert_eq!(b.pick(Some("beta"), None).unwrap().build_id, "40");
        assert_eq!(b.pick(Some("nope"), None).unwrap().build_id, "30");
        assert_eq!(b.pick(None, Some("20")).unwrap().build_id, "20");
        assert_eq!(b.pick(None, Some("777")).unwrap().build_id, "10");
        assert_eq!(b.pick(None, Some("999")).unwrap().build_id, "30");
        assert_eq!(b.branches(), [None, Some("beta".to_string())]);
        assert!(Builds::parse(&json!({})).pick(None, None).is_none());
    }

    #[test]
    fn v1_builds_record_their_legacy_id() {
        let b = Builds::parse(&doc());
        assert_eq!(b.items[3].recorded_id(), "777");
        assert_eq!(b.items[1].recorded_id(), "30");
    }
}
