//! How an installed game starts (legendary's `get_launch_parameters`, what
//! `legendary launch --json` prints): its program and folder, the arguments
//! of its build and of the catalog, and the Epic sign-in it reads from the
//! command line. Games of the EA app and Ubisoft Connect get the link their
//! store opens instead.

use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Context, Result};
use serde_json::{json, Value};

use crate::auth::Session;
use crate::client::Client;
use crate::installed::{self, InstalledGame};
use crate::library::GameMeta;

/// Options of a launch.
#[derive(Debug, Clone, Copy, Default)]
pub struct LaunchOptions<'a> {
    /// No network: the game gets no sign-in (`--offline`).
    pub offline: bool,
    /// The game's language (`-epiclocale`); the client's otherwise.
    pub language: Option<&'a str>,
    /// Another program of the install to start (`--override-exe`).
    pub override_exe: Option<&'a str>,
    /// Arguments after the game's own.
    pub user_args: &'a [String],
}

/// What starts the game.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct LaunchParams {
    /// Relative to `game_directory`, with `/`.
    pub game_executable: String,
    pub game_directory: String,
    pub working_directory: String,
    /// The build's and the catalog's arguments.
    pub game_parameters: Vec<String>,
    /// The Epic sign-in, ownership token and ids.
    pub egl_parameters: Vec<String>,
    pub user_parameters: Vec<String>,
}

impl LaunchParams {
    /// The program's full path.
    pub fn executable_path(&self) -> PathBuf {
        Path::new(&self.game_directory).join(&self.game_executable)
    }

    /// Every argument in the order legendary passes them.
    pub fn args(&self) -> Vec<String> {
        self.game_parameters.iter().chain(&self.user_parameters).chain(&self.egl_parameters).cloned().collect()
    }

    /// `legendary launch --json`'s object (no wrapper, Wine or environment
    /// of its own: the caller's launch plan has them).
    pub fn to_json(&self) -> Value {
        json!({
            "game_parameters": self.game_parameters,
            "game_executable": self.game_executable,
            "game_directory": self.game_directory,
            "egl_parameters": self.egl_parameters,
            "launch_command": [],
            "working_directory": self.working_directory,
            "user_parameters": self.user_parameters,
            "environment": {},
            "pre_launch_command": "",
            "pre_launch_wait": false,
        })
    }
}

/// Splits like Python's `shlex.split(text, posix=False)`, which legendary
/// uses for the build's arguments: whitespace separates, quotes group and
/// stay in the argument.
pub fn split_args(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quote: Option<char> = None;
    for c in text.chars() {
        match quote {
            Some(q) => {
                cur.push(c);
                if c == q {
                    quote = None;
                }
            }
            None if c.is_whitespace() => {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
            }
            None => {
                if c == '"' || c == '\'' {
                    quote = Some(c);
                }
                cur.push(c);
            }
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// The installed game a launch of `app_name` starts: the game itself, or the
/// base game of a launchable add-on (with the add-on's metadata).
fn resolve(config_dir: &Path, app_name: &str) -> Result<(InstalledGame, GameMeta)> {
    let mut install = installed::get(config_dir, app_name);
    if install.as_ref().is_none_or(|g| g.executable.is_empty()) {
        if let Some(game) = GameMeta::load(config_dir, app_name).filter(GameMeta::is_launchable_addon) {
            let base = game.metadata.pointer("/mainGameItem/releaseInfo/0/appId").and_then(Value::as_str).unwrap_or_default().to_string();
            install = installed::get(config_dir, &base);
        }
    }
    let install = install.ok_or_else(|| anyhow!("{app_name} is not installed"))?;
    if install.is_dlc && install.executable.is_empty() {
        bail!("{app_name} is a DLC without a program: start its game instead");
    }
    if !Path::new(&install.install_path).exists() {
        bail!("The install folder {} does not exist", install.install_path);
    }
    // An add-on's own metadata: its sandbox and command line.
    let meta = GameMeta::load(config_dir, app_name).ok_or_else(|| anyhow!("the metadata of {app_name} is missing; refresh the library"))?;
    Ok((install, meta))
}

/// What starts `app_name`. Online, Epic gives a one-time sign-in code (and
/// an ownership token for DRM that checks one, written to `tmp/`).
pub fn launch_params(
    client: &Client,
    config_dir: &Path,
    session: Option<&Session>,
    app_name: &str,
    opts: &LaunchOptions,
) -> Result<LaunchParams> {
    let (install, game) = resolve(config_dir, app_name)?;
    let exe = match opts.override_exe.filter(|e| !e.is_empty()) {
        Some(e) => {
            let exe = e.replace('\\', "/");
            if !Path::new(&install.install_path).join(&exe).exists() {
                bail!("The program {exe} is not in the install folder");
            }
            exe
        }
        None => install.executable.replace('\\', "/").trim_start_matches('/').to_string(),
    };
    let exe_path = Path::new(&install.install_path).join(&exe);
    let working = exe_path.parent().map(|p| p.to_string_lossy().into_owned()).unwrap_or_else(|| install.install_path.clone());
    let mut p = LaunchParams {
        game_executable: exe,
        game_directory: install.install_path.clone(),
        working_directory: working,
        ..Default::default()
    };
    p.game_parameters.extend(split_args(&install.launch_parameters));
    if let Some(extra) = game.additional_command_line() {
        p.game_parameters.extend(split_args(extra.trim()));
    }

    let mut token = String::new();
    if !opts.offline {
        let s = session.ok_or_else(|| anyhow!("Not signed in to Epic Games"))?;
        token = client.exchange_code(s).context("getting the sign-in code of the game")?;
    } else if !install.can_run_offline {
        log::warn!("epic: {app_name} is not approved for offline use and may not work");
    }
    let (account, user) = session.map(|s| (s.account_id().to_string(), s.display_name().to_string())).unwrap_or_default();
    p.egl_parameters.extend([
        "-AUTH_LOGIN=unused".to_string(),
        format!("-AUTH_PASSWORD={token}"),
        "-AUTH_TYPE=exchangecode".into(),
        format!("-epicapp={}", install.app_name),
        "-epicenv=Prod".into(),
    ]);
    if install.requires_ot && !opts.offline {
        let s = session.expect("checked above");
        let ovt = client.ownership_token(s, game.namespace(), game.catalog_item_id()).context("getting the game's ownership token")?;
        let path = config_dir.join("tmp").join(format!("{}{}.ovt", game.namespace(), game.catalog_item_id()));
        std::fs::create_dir_all(config_dir.join("tmp"))?;
        heronite_data::temps::write_atomic(&path, &ovt, Some(0o600))?;
        p.egl_parameters.push(format!("-epicovt={}", path.display()));
    }
    let language = opts.language.filter(|l| !l.is_empty()).unwrap_or(&client.locale);
    p.egl_parameters.extend([
        "-EpicPortal".to_string(),
        format!("-epicusername={user}"),
        format!("-epicuserid={account}"),
        format!("-epiclocale={language}"),
        format!("-epicsandboxid={}", game.namespace()),
    ]);
    if let Some(dep) = game.sidecar.as_ref().and_then(|s| s.pointer("/config/deploymentId")).and_then(Value::as_str) {
        p.egl_parameters.push(format!("-epicdeploymentid={dep}"));
    }
    p.user_parameters = opts.user_args.to_vec();
    Ok(p)
}

/// Whether the installed build is older than the one Epic offers now
/// (`legendary launch` refuses to start it then). Refreshes the platform's
/// builds in `assets.json`.
pub fn update_available(client: &Client, config_dir: &Path, session: &Session, app_name: &str) -> Result<bool> {
    let install = installed::get(config_dir, app_name).ok_or_else(|| anyhow!("{app_name} is not installed"))?;
    let list = client.assets(session, &install.platform, "Live")?;
    let latest = list.iter().find(|a| a.app_name == app_name).map(|a| a.build_version.clone());
    let mut all = crate::library::read_assets(config_dir);
    if all.get(&install.platform) != Some(&list) {
        all.insert(install.platform.clone(), list);
        crate::files::write_json(&crate::library::assets_file(config_dir), &serde_json::to_value(&all)?, false)?;
    }
    let latest = latest.ok_or_else(|| anyhow!("{app_name} is not among the account's builds"))?;
    Ok(latest != install.version)
}

/// `application/x-www-form-urlencoded` of `pairs` (Python's `urlencode`).
fn urlencode(pairs: &[(String, String)]) -> String {
    use percent_encoding::{utf8_percent_encode, AsciiSet, NON_ALPHANUMERIC};
    const SAFE: &AsciiSet = &NON_ALPHANUMERIC.remove(b'_').remove(b'.').remove(b'-').remove(b'~').remove(b' ');
    let enc = |s: &str| utf8_percent_encode(s, SAFE).to_string().replace(' ', "+");
    pairs.iter().map(|(k, v)| format!("{}={}", enc(k), enc(v))).collect::<Vec<_>>().join("&")
}

/// Python's `parse_qsl`: `a=1&b=2` into pairs (blank values dropped).
fn parse_qsl(text: &str) -> Vec<(String, String)> {
    let dec = |s: &str| percent_encoding::percent_decode_str(&s.replace('+', " ")).decode_utf8_lossy().into_owned();
    text.split(['&', ';']).filter_map(|kv| kv.split_once('=')).filter(|(_, v)| !v.is_empty()).map(|(k, v)| (dec(k), dec(v))).collect()
}

/// The sign-in a store app passes on to the game: `[(key, value)]`.
fn store_app_params(client: &Client, session: Option<&Session>, offline: bool, game: &GameMeta) -> Result<Vec<(String, String)>> {
    let token =
        if offline { "0".to_string() } else { client.exchange_code(session.ok_or_else(|| anyhow!("Not signed in to Epic Games"))?)? };
    let (account, user) = session.map(|s| (s.account_id().to_string(), s.display_name().to_string())).unwrap_or_default();
    let mut params = vec![
        ("AUTH_PASSWORD".to_string(), token),
        ("AUTH_TYPE".into(), "exchangecode".into()),
        ("epicusername".into(), user),
        ("epicuserid".into(), account),
        ("epiclocale".into(), client.locale.clone()),
    ];
    // legendary reads this attribute with this spelling for store app games.
    if let Some(extra) = game.custom_attribute("AdditionalCommandline") {
        params.extend(parse_qsl(extra));
    }
    Ok(params)
}

/// The EA app's link that starts (and first activates) an EA game
/// (`legendary launch --origin --json`).
pub fn origin_uri(client: &Client, config_dir: &Path, session: Option<&Session>, app_name: &str, offline: bool) -> Result<String> {
    let game = GameMeta::load(config_dir, app_name).ok_or_else(|| anyhow!("{app_name} is not in the Epic library; refresh it first"))?;
    Ok(format!("link2ea://launchgame/{app_name}?{}", urlencode(&store_app_params(client, session, offline, &game)?)))
}

/// Ubisoft Connect's link that starts a Ubisoft game (`--ubisoft`).
pub fn ubisoft_uri(client: &Client, config_dir: &Path, session: Option<&Session>, app_name: &str, offline: bool) -> Result<String> {
    let game = GameMeta::load(config_dir, app_name).ok_or_else(|| anyhow!("{app_name} is not in the Epic library; refresh it first"))?;
    let id = game.custom_attribute("GameID").filter(|s| !s.is_empty()).unwrap_or(app_name).to_string();
    Ok(format!("uplay://launch/{id}?{}", urlencode(&store_app_params(client, session, offline, &game)?)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::Endpoints;
    use crate::http::tests::serve;

    #[test]
    fn arguments_split_like_python_without_posix() {
        assert_eq!(split_args(r#"-a  "b c" -d='e f'"#), ["-a", "\"b c\"", "-d='e f'"]);
        assert!(split_args("  ").is_empty());
    }

    fn setup(dir: &Path, requires_ot: bool) {
        let game = GameMeta::from_value(json!({
            "app_name": "Calluna", "app_title": "Control",
            "metadata": {"id": "cat", "namespace": "ns", "customAttributes": {"AdditionalCommandLine": {"value": " -nolauncher "}}},
            "sidecar": {"config": {"deploymentId": "dep1"}, "rev": 1}
        }))
        .unwrap();
        game.save(dir).unwrap();
        let folder = dir.join("game");
        std::fs::create_dir_all(folder.join("bin")).unwrap();
        installed::set(
            dir,
            &InstalledGame {
                app_name: "Calluna".into(),
                install_path: folder.to_string_lossy().into_owned(),
                executable: "\\bin\\Control.exe".into(),
                launch_parameters: "-dx12".into(),
                can_run_offline: true,
                requires_ot,
                ..Default::default()
            },
        )
        .unwrap();
    }

    #[test]
    fn an_offline_launch_needs_no_network() {
        let dir = tempfile::tempdir().unwrap();
        setup(dir.path(), false);
        let session = Session::from_value(json!({"access_token": "t", "account_id": "acc", "displayName": "Player"})).unwrap();
        let client = Client::new(Endpoints::all_at("http://127.0.0.1:9"));
        let args = vec!["-windowed".to_string()];
        let opts = LaunchOptions { offline: true, language: Some("de"), user_args: &args, ..Default::default() };
        let p = launch_params(&client, dir.path(), Some(&session), "Calluna", &opts).unwrap();
        assert_eq!(p.game_executable, "bin/Control.exe");
        assert!(p.working_directory.ends_with("game/bin"));
        assert_eq!(p.game_parameters, ["-dx12", "-nolauncher"]);
        assert_eq!(
            p.egl_parameters,
            [
                "-AUTH_LOGIN=unused",
                "-AUTH_PASSWORD=",
                "-AUTH_TYPE=exchangecode",
                "-epicapp=Calluna",
                "-epicenv=Prod",
                "-EpicPortal",
                "-epicusername=Player",
                "-epicuserid=acc",
                "-epiclocale=de",
                "-epicsandboxid=ns",
                "-epicdeploymentid=dep1"
            ]
        );
        assert_eq!(p.args().last().unwrap(), "-epicdeploymentid=dep1");
        assert_eq!(p.to_json()["user_parameters"], json!(["-windowed"]));
        assert!(launch_params(&client, dir.path(), None, "Other", &LaunchOptions::default()).is_err());
    }

    #[test]
    fn online_launches_get_a_code_and_an_ownership_token() {
        let dir = tempfile::tempdir().unwrap();
        setup(dir.path(), true);
        let (url, server) = serve(vec![(200, r#"{"code":"EXCHANGE"}"#), (200, "OVT-BYTES")]);
        let session = Session::from_value(json!({"access_token": "t", "account_id": "acc", "displayName": "P"})).unwrap();
        let client = Client::new(Endpoints::all_at(&url));
        let p = launch_params(&client, dir.path(), Some(&session), "Calluna", &LaunchOptions::default()).unwrap();
        assert!(p.egl_parameters.contains(&"-AUTH_PASSWORD=EXCHANGE".to_string()));
        let ovt = p.egl_parameters.iter().find_map(|a| a.strip_prefix("-epicovt=")).unwrap();
        assert_eq!(std::fs::read(ovt).unwrap(), b"OVT-BYTES");
        assert!(ovt.ends_with("tmp/nscat.ovt"));
        let seen = server.join().unwrap();
        assert!(seen[1].contains("nsCatalogItemId=ns%3Acat"), "{seen:?}");
    }

    #[test]
    fn store_app_links() {
        let dir = tempfile::tempdir().unwrap();
        GameMeta::from_value(json!({"app_name": "Ori", "metadata": {"customAttributes": {"AdditionalCommandline": {"value": "a=1&b=x y"}, "GameID": {"value": "635"}}}}))
            .unwrap()
            .save(dir.path())
            .unwrap();
        let session = Session::from_value(json!({"access_token": "t", "account_id": "acc", "displayName": "Pl ayer"})).unwrap();
        let client = Client::new(Endpoints::all_at("http://127.0.0.1:9"));
        let uri = origin_uri(&client, dir.path(), Some(&session), "Ori", true).unwrap();
        assert_eq!(
            uri,
            "link2ea://launchgame/Ori?AUTH_PASSWORD=0&AUTH_TYPE=exchangecode&epicusername=Pl+ayer&epicuserid=acc&epiclocale=en&a=1&b=x+y"
        );
        assert!(ubisoft_uri(&client, dir.path(), Some(&session), "Ori", true).unwrap().starts_with("uplay://launch/635?AUTH_PASSWORD=0"));
    }
}
