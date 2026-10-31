//! Games with a newer version: `nile list-updates --json` prints a JSON array
//! of product ids, saved to `nile_store/updates.json` as `{"updates": [...]}`.

use anyhow::{bail, Result};
use heronite_data::json_store;
use heronite_data::DataPaths;
use heronite_i18n::tr;
use serde_json::Value;

use crate::cmd::Nile;

const KEY: &str = "updates";

/// Asks nile (network) which installed games have an update and saves the
/// answer.
pub fn check(paths: &DataPaths, nile: &Nile) -> Result<Vec<String>> {
    let out = nile.run(&["list-updates", "--json"])?;
    if !out.success() {
        let why = out.error_lines();
        if why.is_empty() {
            bail!(tr!("nile exited with {code}", code = format!("{:?}", out.code)));
        }
        bail!(tr!("Failed to check for updates: {error}", error = why));
    }
    let Some(ids) = parse_ids(&out.stdout) else { bail!(tr!("nile gave an unexpected answer when checking for updates")) };
    save(paths, &ids)?;
    Ok(ids)
}

/// The array in nile's stdout; nothing printed means no updates. Log lines go
/// to stderr, but a stray line before the array is tolerated.
fn parse_ids(stdout: &str) -> Option<Vec<String>> {
    let Some(line) = stdout.lines().map(str::trim).find(|l| l.starts_with('[')) else {
        return stdout.trim().is_empty().then(Vec::new);
    };
    match serde_json::from_str::<Value>(line).ok()? {
        Value::Array(list) => Some(list.iter().filter_map(Value::as_str).map(str::to_string).collect()),
        _ => None,
    }
}

fn save(paths: &DataPaths, ids: &[String]) -> Result<()> {
    json_store::update_value(&paths.nile_store(KEY), |doc| {
        doc.as_object_mut().expect("object").insert(KEY.into(), ids.iter().cloned().map(Value::String).collect());
        Ok(())
    })
}

/// The last saved answer of [`check`], without the network.
pub fn cached_updates(paths: &DataPaths) -> Vec<String> {
    match json_store::read_value(&paths.nile_store(KEY)) {
        Ok(doc) => doc
            .get(KEY)
            .and_then(Value::as_array)
            .map(|l| l.iter().filter_map(Value::as_str).map(str::to_string).collect())
            .unwrap_or_default(),
        Err(err) => {
            log::warn!("reading saved nile updates: {err:#}");
            Vec::new()
        }
    }
}

/// Removes a game from the saved list (it was updated or uninstalled).
pub fn forget(paths: &DataPaths, app_name: &str) {
    let mut ids = cached_updates(paths);
    let before = ids.len();
    ids.retain(|id| id != app_name);
    if ids.len() != before {
        if let Err(err) = save(paths, &ids) {
            log::warn!("saving nile updates: {err:#}");
        }
    }
}

/// Forgets every saved update (sign-out).
pub(crate) fn clear(paths: &DataPaths) {
    if paths.nile_store(KEY).exists() {
        if let Err(err) = save(paths, &[]) {
            log::warn!("saving nile updates: {err:#}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cmd::testing::{Env, Reply};

    #[test]
    fn output_is_parsed() {
        assert_eq!(parse_ids("[\"a\", \"b\"]\n").unwrap(), ["a", "b"]);
        assert_eq!(parse_ids("[]").unwrap(), Vec::<String>::new());
        assert_eq!(parse_ids("").unwrap(), Vec::<String>::new());
        assert_eq!(parse_ids("  \n").unwrap(), Vec::<String>::new());
        assert_eq!(parse_ids("noise\n[\"a\"]\n").unwrap(), ["a"]);
        assert!(parse_ids("Games with updates:\nCool a\n").is_none());
        assert!(parse_ids("[not json").is_none());
    }

    #[test]
    fn check_saves_the_answer() {
        let env = Env::new();
        let nile = env.nile(Reply::new().out(r#"["amzn1.adg.product.AAA","amzn1.adg.product.BBB"]"#).err("DEBUG [CLI]:\t x"));
        assert!(cached_updates(&env.paths).is_empty());
        let ids = check(&env.paths, &nile).unwrap();
        assert_eq!(ids, ["amzn1.adg.product.AAA", "amzn1.adg.product.BBB"]);
        assert_eq!(env.calls(), ["list-updates --json"]);
        assert_eq!(cached_updates(&env.paths), ids);

        forget(&env.paths, "amzn1.adg.product.AAA");
        assert_eq!(cached_updates(&env.paths), ["amzn1.adg.product.BBB"]);
        clear(&env.paths);
        assert!(cached_updates(&env.paths).is_empty());
    }

    #[test]
    fn empty_stdout_means_no_updates() {
        let env = Env::new();
        let nile = env.nile(Reply::new().err("ERROR [CLI]:\t No games installed"));
        assert_eq!(check(&env.paths, &nile).unwrap(), Vec::<String>::new());
        assert_eq!(json_store::read_value(&env.paths.nile_store("updates")).unwrap()["updates"], serde_json::json!([]));
    }

    #[test]
    fn failures_are_reported_and_keep_the_saved_list() {
        let env = Env::new();
        save(&env.paths, &["x".to_string()]).unwrap();
        let nile = env.nile(Reply::new().err("Traceback (most recent call last):").exit(1));
        let err = check(&env.paths, &nile).unwrap_err().to_string();
        assert!(err.contains("Traceback"), "{err}");
        assert_eq!(cached_updates(&env.paths), ["x"]);
    }

    #[test]
    fn forgetting_without_a_file_is_harmless() {
        let env = Env::new();
        forget(&env.paths, "x");
        assert!(!env.paths.nile_store("updates").exists());
    }
}
