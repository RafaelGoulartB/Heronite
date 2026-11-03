//! First-launch setup of a Windows game in its Wine prefix: the shared
//! redistributables (Visual C++ runtimes, DirectX...) and the game's own
//! installer scripts, as the list of programs to run before the first start.
//!
//! The data comes from the game's stored build (gogdl's manifest) and from
//! the redist manifest in `<data>/tools/redist/gog` (`.gogdl-redist-manifest`).

use std::path::{Path, PathBuf};

use anyhow::{anyhow, Result};
use heronite_data::json_store;
use heronite_data::DataPaths;
use heronite_i18n::tr;
use heronite_launch::plan::SetupStep;
use serde_json::Value;

use crate::installed;
use crate::lang;
use crate::{manifest_file, support_dir};

/// The script interpreter every GOG installer script needs.
const INTERPRETER: &str = "ISI";
const INTERPRETER_EXE: &str = "__redist/ISI/scriptinterpreter.exe";
const REDIST_MANIFEST: &str = ".gogdl-redist-manifest";

/// Programs to run in the prefix before the game's first launch, in order.
/// A game without a manifest (an imported one) has none.
pub fn first_launch_steps(paths: &DataPaths, app_name: &str, install_path: &str) -> Result<Vec<SetupStep>> {
    steps_with(paths, app_name, install_path, &fetch_redist)
}

/// [`first_launch_steps`] with `fetch` downloading missing redistributables.
fn steps_with(
    paths: &DataPaths,
    app_name: &str,
    install_path: &str,
    fetch: &dyn Fn(&Path, &[String]) -> Result<()>,
) -> Result<Vec<SetupStep>> {
    let manifest = json_store::read_value(&manifest_file(&paths.gogdl_config_dir(), app_name))?;
    if !manifest.is_object() {
        return Ok(Vec::new());
    }
    let redist_dir = paths.gog_redist_dir();
    let deps = manifest_dependencies(&manifest);
    let redist = ensure_redist(&redist_dir, &deps, fetch)?;

    let saved = installed::get(paths, app_name);
    let language = manifest
        .get("HGLInstallLanguage")
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| saved.as_ref().map(|s| s.language.clone()).filter(|l| !l.is_empty()))
        .unwrap_or_else(|| "en-US".to_string());
    let version_name = saved.as_ref().map(|s| s.version.clone()).filter(|v| !v.is_empty());
    let ctx = Ctx {
        app_name,
        install_path,
        redist_dir: &redist_dir,
        support_dir: support_dir(&paths.gogdl_config_dir(), app_name),
        manifest: &manifest,
        redist: &redist,
        language,
        version_name,
        saved_build_id: saved.map(|s| s.build_id).filter(|b| !b.is_empty()),
    };
    Ok(ctx.steps(&deps))
}

/// Redistributable ids the game needs: v2 manifests list them, v1 ones mark
/// the depots that are redistributables.
fn manifest_dependencies(manifest: &Value) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut add = |id: &str| {
        if !id.is_empty() && !out.iter().any(|o| o == id) {
            out.push(id.to_string());
        }
    };
    if let Some(list) = manifest.get("dependencies").and_then(Value::as_array) {
        list.iter().filter_map(Value::as_str).for_each(&mut add);
    }
    if let Some(depots) = manifest.pointer("/product/depots").and_then(Value::as_array) {
        depots.iter().filter_map(|d| d.get("redist").and_then(Value::as_str)).for_each(&mut add);
    }
    out
}

fn strings(v: Option<&Value>) -> Vec<String> {
    v.and_then(Value::as_array).map(|l| l.iter().filter_map(|s| s.as_str().map(str::to_string)).collect()).unwrap_or_default()
}

/// Downloads the redistributables that are not there yet and returns the
/// redist manifest (`Null` when there is none). `fetch` downloads ids into
/// the folder.
fn ensure_redist(dir: &Path, deps: &[String], fetch: &dyn Fn(&Path, &[String]) -> Result<()>) -> Result<Value> {
    let manifest_path = dir.join(REDIST_MANIFEST);
    let current = json_store::read_value(&manifest_path).unwrap_or(Value::Null);
    let installed_ids = strings(current.get("HGLInstalled"));
    let mut needed: Vec<String> = vec![INTERPRETER.to_string()];
    needed.extend(deps.iter().filter(|d| d.as_str() != INTERPRETER).cloned());

    let depots = current.get("depots").and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default();
    let in_game_folder = |id: &str| {
        depots
            .iter()
            .find(|d| d.get("dependencyId").and_then(Value::as_str) == Some(id))
            .and_then(|d| d.pointer("/executable/path").and_then(Value::as_str))
            .is_some_and(|p| !is_redist_path(p))
    };
    let missing = needed.iter().any(|id| !installed_ids.contains(id) && !in_game_folder(id));
    if !missing {
        return Ok(current);
    }

    // The record lists only the ids of the last download: ask for the old ones too.
    let mut ids = needed;
    for id in installed_ids {
        if !ids.contains(&id) {
            ids.push(id);
        }
    }
    std::fs::create_dir_all(dir)?;
    fetch(dir, &ids)?;
    Ok(json_store::read_value(&manifest_path).unwrap_or(Value::Null))
}

/// Downloads the shared redistributables `ids` into `dir`. Blocking, network.
fn fetch_redist(dir: &Path, ids: &[String]) -> Result<()> {
    match crate::download::install_redist(dir, ids, &heronite_data::proc::Cancel::new()) {
        Ok(_) => Ok(()),
        Err(err) => Err(anyhow!(tr!("Could not download the Windows components this game needs: {error}", error = format!("{err:#}")))),
    }
}

/// Redistributables under `__redist` are the ones downloaded next to the
/// shared manifest; the rest ship in the game's folder.
fn is_redist_path(path: &str) -> bool {
    path.replace('\\', "/").trim_start_matches('/').to_ascii_lowercase().starts_with("__redist")
}

struct Ctx<'a> {
    app_name: &'a str,
    install_path: &'a str,
    redist_dir: &'a Path,
    support_dir: PathBuf,
    manifest: &'a Value,
    redist: &'a Value,
    /// Language code of the install, e.g. `en-US`.
    language: String,
    /// Version name saved at install time.
    version_name: Option<String>,
    saved_build_id: Option<String>,
}

impl Ctx<'_> {
    fn steps(&self, deps: &[String]) -> Vec<SetupStep> {
        let mut steps: Vec<SetupStep> = deps.iter().filter_map(|d| self.dependency_step(d)).collect();
        steps.extend(self.script_steps());
        steps
    }

    fn step(&self, parts: Vec<String>) -> SetupStep {
        SetupStep { parts, cwd: Some(self.install_path.to_string()), ..Default::default() }
    }

    fn dependency_step(&self, id: &str) -> Option<SetupStep> {
        let depot = self.redist.get("depots")?.as_array()?.iter().find(|d| d.get("dependencyId").and_then(Value::as_str) == Some(id));
        let Some(depot) = depot else {
            log::info!("GOG dependency {id} is not in the redistributable list");
            return None;
        };
        let rel = depot.pointer("/executable/path").and_then(Value::as_str).filter(|p| !p.is_empty())?;
        let args = split_args(depot.pointer("/executable/arguments").and_then(Value::as_str).unwrap_or_default());
        let base = if is_redist_path(rel) { self.redist_dir } else { Path::new(self.install_path) };
        let Some(exe) = resolve(base, rel) else {
            log::info!("GOG dependency {id}: {rel} is missing, skipping it");
            return None;
        };
        let exe = exe.display().to_string();
        let parts = if id == "PHYSXLEGACY" {
            let mut p = vec!["msiexec".to_string(), "/i".to_string(), exe];
            p.extend(args);
            p.push("/qb".into());
            p
        } else {
            let mut p = vec![exe];
            p.extend(args);
            p
        };
        Some(self.step(parts))
    }

    fn script_steps(&self) -> Vec<SetupStep> {
        match self.manifest.get("version").and_then(Value::as_u64) {
            Some(1) => self.v1_steps(),
            _ => self.v2_steps(),
        }
    }

    /// The base game and the DLCs that were installed.
    fn product_ids(&self) -> Vec<String> {
        let mut ids = vec![self.app_name.to_string()];
        if let Some(dlcs) = self.manifest.get("HGLdlcs").and_then(Value::as_array) {
            for id in dlcs.iter().filter_map(|d| crate::gamesdb::id_text(d.get("id"))) {
                if !ids.contains(&id) {
                    ids.push(id);
                }
            }
        }
        ids
    }

    fn version_name(&self, build_id: &str) -> String {
        self.version_name
            .clone()
            .or_else(|| self.manifest.get("versionName").and_then(Value::as_str).map(str::to_string))
            .unwrap_or_else(|| build_id.to_string())
    }

    /// Flags the installer scripts take (`nodesktopshorctut` is GOG's own spelling).
    fn flags(&self, product_id: &str, lang_name: &str, build_id: &str, lang_code: Option<&str>, support: Option<&Path>) -> Vec<String> {
        let mut flags: Vec<String> = vec![
            "/VERYSILENT".into(),
            format!("/DIR={}", self.install_path),
            format!("/Language={lang_name}"),
            format!("/LANG={lang_name}"),
            format!("/ProductId={product_id}"),
            "/galaxyclient".into(),
            format!("/buildId={build_id}"),
            format!("/versionName={}", self.version_name(build_id)),
        ];
        if let Some(code) = lang_code {
            flags.push(format!("/lang-code={code}"));
        }
        if let Some(dir) = support {
            flags.push(format!("/supportDir={}", dir.display()));
        }
        flags.extend(["/nodesktopshorctut".into(), "/nodesktopshortcut".into()]);
        flags
    }

    fn v2_build_id(&self) -> String {
        crate::gamesdb::id_text(self.manifest.get("buildId")).or_else(|| self.saved_build_id.clone()).unwrap_or_default()
    }

    fn v2_steps(&self) -> Vec<SetupStep> {
        let build_id = self.v2_build_id();
        let name = lang::english_name(&self.language);
        let ids = self.product_ids();
        if self.manifest.get("scriptInterpreter").and_then(Value::as_bool) == Some(true) {
            let Some(exe) = resolve(self.redist_dir, INTERPRETER_EXE) else {
                log::info!("the GOG script interpreter is missing, skipping the installer scripts");
                return Vec::new();
            };
            return ids
                .iter()
                .map(|pid| {
                    let mut parts = vec![exe.display().to_string()];
                    parts.extend(self.flags(pid, &name, &build_id, Some(&self.language), Some(&self.support_dir)));
                    self.step(parts)
                })
                .collect();
        }
        let products = self.manifest.get("products").and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default();
        let mut steps = Vec::new();
        for product in products {
            let Some(pid) = crate::gamesdb::id_text(product.get("productId")).filter(|p| ids.contains(p)) else { continue };
            let Some(rel) = product.get("temp_executable").and_then(Value::as_str).filter(|t| !t.is_empty()) else { continue };
            let Some(exe) = resolve(&self.support_dir.join(&pid), rel) else {
                log::info!("GOG installer script {rel} of {pid} is missing, skipping it");
                continue;
            };
            let mut parts = vec![exe.display().to_string()];
            parts.extend(self.flags(&pid, &name.to_lowercase(), &build_id, Some(&self.language), None));
            steps.push(self.step(parts));
        }
        steps
    }

    fn v1_steps(&self) -> Vec<SetupStep> {
        let build_id = crate::gamesdb::id_text(self.manifest.pointer("/product/timestamp")).unwrap_or_default();
        let name = lang::english_name(&self.language);
        let ids = self.product_ids();
        let commands = self.manifest.pointer("/product/support_commands").and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default();
        let mut steps = Vec::new();
        for command in commands {
            let Some(pid) = crate::gamesdb::id_text(command.get("gameID")).filter(|p| ids.contains(p)) else { continue };
            let wanted = strings(command.get("languages"));
            if !wanted.iter().any(|l| l.eq_ignore_ascii_case("Neutral") || lang::matches(&self.language, l)) {
                continue;
            }
            let Some(rel) = command.get("executable").and_then(Value::as_str).filter(|e| !e.is_empty()) else { continue };
            let Some(exe) = resolve(&self.support_dir.join(&pid), rel) else {
                log::info!("GOG installer script {rel} of {pid} is missing, skipping it");
                continue;
            };
            let mut parts = vec![exe.display().to_string()];
            parts.extend(self.flags(&pid, &name, &build_id, None, None));
            steps.push(self.step(parts));
        }
        steps
    }
}

/// The file `rel` (Windows or Unix separators, any letter case) below `base`.
fn resolve(base: &Path, rel: &str) -> Option<PathBuf> {
    let mut current = base.to_path_buf();
    for part in rel.replace('\\', "/").split('/').filter(|p| !p.is_empty() && *p != ".") {
        if part == ".." {
            return None;
        }
        let direct = current.join(part);
        if direct.exists() {
            current = direct;
            continue;
        }
        let found = std::fs::read_dir(&current).ok()?.flatten().find(|e| e.file_name().to_string_lossy().eq_ignore_ascii_case(part))?;
        current = found.path();
    }
    current.is_file().then_some(current)
}

/// Splits a command line the way a shell would: spaces separate words,
/// quotes group them.
fn split_args(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut word = String::new();
    let mut in_word = false;
    let mut quote: Option<char> = None;
    for c in text.chars() {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (Some(_), c) => word.push(c),
            (None, '"' | '\'') => {
                quote = Some(c);
                in_word = true;
            }
            (None, c) if c.is_whitespace() => {
                if in_word {
                    out.push(std::mem::take(&mut word));
                    in_word = false;
                }
            }
            (None, c) => {
                word.push(c);
                in_word = true;
            }
        }
    }
    if in_word {
        out.push(word);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::setup;
    use serde_json::json;
    use std::sync::Mutex;

    const REDIST: &str = r#"{"depots":[
        {"dependencyId":"ISI","executable":{"path":"__redist/ISI/scriptinterpreter.exe","arguments":""}},
        {"dependencyId":"MSVC2010","executable":{"path":"__redist\\MSVC2010\\vcredist_x86.exe","arguments":"/q /norestart"}},
        {"dependencyId":"PHYSXLEGACY","executable":{"path":"__redist/PhysXLegacy/PhysX.msi","arguments":"REBOOT=ReallySuppress \"A B\""}},
        {"dependencyId":"INGAME","executable":{"path":"redist/helper.exe","arguments":"/s"}}
    ],"build_id":"1","HGLInstalled":["ISI","MSVC2010","PHYSXLEGACY"]}"#;

    fn touch(path: &Path) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, "x").unwrap();
    }

    /// `rel` (`/`-separated) below `base`, with this system's separators.
    fn under(base: &Path, rel: &str) -> PathBuf {
        let mut p = base.to_path_buf();
        p.extend(rel.split('/'));
        p
    }

    fn write_manifest(paths: &DataPaths, app: &str, manifest: Value) {
        let file = manifest_file(&paths.gogdl_config_dir(), app);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(file, manifest.to_string()).unwrap();
    }

    /// A download of redistributables that records the ids it was asked
    /// for and fails.
    struct Fetches(Mutex<Vec<String>>);

    impl Fetches {
        fn new() -> Self {
            Self(Mutex::new(Vec::new()))
        }

        fn failing(&self) -> impl Fn(&Path, &[String]) -> Result<()> + '_ {
            |_, ids| {
                self.0.lock().unwrap().push(ids.join(","));
                Err(anyhow!("no network"))
            }
        }

        fn calls(&self) -> Vec<String> {
            self.0.lock().unwrap().clone()
        }
    }

    struct World {
        _dir: tempfile::TempDir,
        paths: DataPaths,
        game: String,
    }

    /// Redist files, a redist manifest and a game folder.
    fn world() -> World {
        let (dir, paths) = setup();
        let redist = paths.gog_redist_dir();
        for f in ["__redist/ISI/scriptinterpreter.exe", "__redist/MSVC2010/vcredist_x86.exe", "__redist/PhysXLegacy/PhysX.msi"] {
            touch(&under(&redist, f));
        }
        std::fs::write(redist.join(REDIST_MANIFEST), REDIST).unwrap();
        let game = dir.path().join("Games").join("The Game");
        std::fs::create_dir_all(&game).unwrap();
        World { _dir: dir, paths, game: game.display().to_string() }
    }

    fn flags(w: &World, pid: &str, build: &str, lang: &str, code: Option<&str>, support: bool) -> Vec<String> {
        let mut v = vec![
            "/VERYSILENT".to_string(),
            format!("/DIR={}", w.game),
            format!("/Language={lang}"),
            format!("/LANG={lang}"),
            format!("/ProductId={pid}"),
            "/galaxyclient".into(),
            format!("/buildId={build}"),
            "/versionName=2.0".into(),
        ];
        if let Some(c) = code {
            v.push(format!("/lang-code={c}"));
        }
        if support {
            v.push(format!("/supportDir={}", support_dir(&w.paths.gogdl_config_dir(), "42").display()));
        }
        v.extend(["/nodesktopshorctut".into(), "/nodesktopshortcut".into()]);
        v
    }

    fn save_game(w: &World) {
        let mut entry = installed::Installed::new("42");
        entry.version = "2.0".into();
        entry.build_id = "900".into();
        installed::upsert(&w.paths, &entry).unwrap();
    }

    #[test]
    fn v2_with_the_script_interpreter() {
        let w = world();
        save_game(&w);
        write_manifest(
            &w.paths,
            "42",
            json!({"version": 2, "dependencies": ["MSVC2010", "PHYSXLEGACY"], "scriptInterpreter": true, "buildId": "900",
                   "HGLInstallLanguage": "de-DE", "HGLdlcs": [{"id": "77", "title": "DLC"}], "products": [{"productId": "42"}]}),
        );
        let fetches = Fetches::new();
        let steps = steps_with(&w.paths, "42", &w.game, &fetches.failing()).unwrap();
        let redist = w.paths.gog_redist_dir();
        let isi = under(&redist, "__redist/ISI/scriptinterpreter.exe").display().to_string();
        let mut expected = vec![
            vec![under(&redist, "__redist/MSVC2010/vcredist_x86.exe").display().to_string(), "/q".into(), "/norestart".into()],
            vec![
                "msiexec".into(),
                "/i".into(),
                under(&redist, "__redist/PhysXLegacy/PhysX.msi").display().to_string(),
                "REBOOT=ReallySuppress".into(),
                "A B".into(),
                "/qb".into(),
            ],
        ];
        for pid in ["42", "77"] {
            let mut p = vec![isi.clone()];
            p.extend(flags(&w, pid, "900", "German", Some("de-DE"), true));
            expected.push(p);
        }
        assert_eq!(steps.iter().map(|s| s.parts.clone()).collect::<Vec<_>>(), expected);
        assert!(steps.iter().all(|s| s.cwd.as_deref() == Some(w.game.as_str())));
        // Windows asks for administrator rights when an installer needs them.
        assert!(steps.iter().all(|s| !s.elevate));
        // Everything was in place, so nothing was downloaded.
        assert!(fetches.calls().is_empty());
    }

    #[test]
    fn v2_with_temp_executables_runs_the_game_scripts_lowercase() {
        let w = world();
        save_game(&w);
        touch(&under(&support_dir(&w.paths.gogdl_config_dir(), "42"), "42/Setup.EXE"));
        write_manifest(
            &w.paths,
            "42",
            json!({"version": 2, "dependencies": [], "scriptInterpreter": false, "buildId": 900,
                   "products": [{"productId": "42", "temp_executable": "setup.exe"}, {"productId": "77", "temp_executable": "dlc.exe"},
                                {"productId": "88"}], "HGLdlcs": [{"id": "77"}]}),
        );
        let steps = first_launch_steps(&w.paths, "42", &w.game).unwrap();
        assert_eq!(steps.len(), 1, "{steps:?}");
        // Windows' folders do not mind the case: the file is found as GOG names it.
        let file = if cfg!(windows) { "42/setup.exe" } else { "42/Setup.EXE" };
        let mut expected = vec![under(&support_dir(&w.paths.gogdl_config_dir(), "42"), file).display().to_string()];
        expected.extend(flags(&w, "42", "900", "english", Some("en-US"), false));
        assert_eq!(steps[0].parts, expected);
    }

    #[test]
    fn v1_runs_the_scripts_for_the_install_language() {
        let w = world();
        save_game(&w);
        for f in ["neutral.exe", "english.exe", "german.exe"] {
            touch(&support_dir(&w.paths.gogdl_config_dir(), "42").join("42").join(f));
        }
        write_manifest(
            &w.paths,
            "42",
            json!({"version": 1, "HGLInstallLanguage": "en-US", "HGLdlcs": [],
                   "product": {"timestamp": 1234, "rootGameID": "42",
                     "depots": [{"redist": "MSVC2010"}, {"gameIDs": ["42"]}],
                     "support_commands": [
                       {"languages": ["Neutral"], "executable": "neutral.exe", "gameID": "42"},
                       {"languages": ["English"], "executable": "english.exe", "gameID": "42"},
                       {"languages": ["German"], "executable": "german.exe", "gameID": "42"},
                       {"languages": ["Neutral"], "executable": "other.exe", "gameID": "99"}]}}),
        );
        let steps = first_launch_steps(&w.paths, "42", &w.game).unwrap();
        let redist = w.paths.gog_redist_dir();
        assert_eq!(steps.len(), 3, "{steps:?}");
        assert_eq!(steps[0].parts[0], under(&redist, "__redist/MSVC2010/vcredist_x86.exe").display().to_string());
        let support = support_dir(&w.paths.gogdl_config_dir(), "42");
        for (step, file) in steps[1..].iter().zip(["neutral.exe", "english.exe"]) {
            let mut expected = vec![support.join("42").join(file).display().to_string()];
            expected.extend(flags(&w, "42", "1234", "English", None, false));
            assert_eq!(step.parts, expected);
        }
    }

    #[test]
    fn dependencies_in_the_game_folder_are_resolved_there() {
        let w = world();
        touch(&under(Path::new(&w.game), "Redist/Helper.exe"));
        write_manifest(&w.paths, "42", json!({"version": 2, "dependencies": ["INGAME"], "scriptInterpreter": true, "buildId": "1"}));
        let steps = first_launch_steps(&w.paths, "42", &w.game).unwrap();
        let file = if cfg!(windows) { "redist/helper.exe" } else { "Redist/Helper.exe" };
        assert_eq!(steps[0].parts, [under(Path::new(&w.game), file).display().to_string(), "/s".to_string()]);
    }

    #[test]
    fn games_without_a_manifest_need_nothing() {
        let w = world();
        assert!(first_launch_steps(&w.paths, "42", &w.game).unwrap().is_empty());
    }

    #[test]
    fn missing_redistributables_are_downloaded_first() {
        let (_dir, paths) = setup();
        let redist = paths.gog_redist_dir();
        write_manifest(&paths, "42", json!({"version": 2, "dependencies": ["MSVC2010"], "scriptInterpreter": true, "buildId": "1"}));
        let asked = Mutex::new(Vec::new());
        let fetch = |dir: &Path, ids: &[String]| -> Result<()> {
            asked.lock().unwrap().push((dir.to_path_buf(), ids.join(",")));
            touch(&under(dir, "__redist/ISI/scriptinterpreter.exe"));
            touch(&under(dir, "__redist/MSVC2010/vcredist_x86.exe"));
            std::fs::write(dir.join(REDIST_MANIFEST), REDIST).unwrap();
            Ok(())
        };
        let steps = steps_with(&paths, "42", "/games/x", &fetch).unwrap();
        assert_eq!(asked.into_inner().unwrap(), [(redist, "ISI,MSVC2010".to_string())]);
        assert_eq!(steps.len(), 2);
        assert!(steps[0].parts[0].ends_with("vcredist_x86.exe"));
        assert!(steps[1].parts[0].ends_with("scriptinterpreter.exe"));
    }

    #[test]
    fn a_missing_redistributable_that_cannot_be_fetched_is_an_error() {
        let (_dir, paths) = setup();
        write_manifest(&paths, "42", json!({"version": 2, "dependencies": [], "scriptInterpreter": true}));
        let fetches = Fetches::new();
        let err = steps_with(&paths, "42", "/games/x", &fetches.failing()).unwrap_err().to_string();
        assert!(err.contains("no network"), "{err}");
    }

    #[test]
    fn already_installed_ids_are_kept_when_more_are_added() {
        let w = world();
        write_manifest(&w.paths, "42", json!({"version": 2, "dependencies": ["VC2013"], "scriptInterpreter": true, "buildId": "1"}));
        // VC2013 is not installed yet: it is asked for, plus what is there.
        let fetches = Fetches::new();
        let _ = steps_with(&w.paths, "42", &w.game, &fetches.failing());
        assert_eq!(fetches.calls(), ["ISI,VC2013,MSVC2010,PHYSXLEGACY"]);
    }

    #[test]
    fn command_lines_are_split_like_a_shell() {
        assert_eq!(split_args("/q /norestart"), ["/q", "/norestart"]);
        assert_eq!(split_args("  a  \"b c\" 'd e'f \"\" "), ["a", "b c", "d ef", ""]);
        assert!(split_args("   ").is_empty());
        assert!(is_redist_path("__redist\\ISI\\x.exe") && !is_redist_path("redist/x.exe"));
    }

    #[test]
    fn paths_resolve_without_regard_to_case_and_never_leave_the_folder() {
        let dir = tempfile::tempdir().unwrap();
        touch(&dir.path().join("Dir/File.EXE"));
        assert!(resolve(dir.path(), "dir\\file.exe").is_some());
        assert!(resolve(dir.path(), "Dir/missing.exe").is_none());
        assert!(resolve(dir.path(), "Dir/../Dir/File.EXE").is_none());
        assert!(resolve(dir.path(), "Dir").is_none());
    }
}
