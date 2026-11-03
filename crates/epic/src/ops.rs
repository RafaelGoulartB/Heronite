//! Game operations (port of the `install`, `update`, `repair`, `uninstall`,
//! `moveInstall` and `importGame` parts of `LegendaryGame`), natively: see
//! [`crate::download`] for what installs, updates and repairs write.

use std::path::Path;

use anyhow::{bail, Result};
use heronite_data::game_config;
use heronite_data::sideload::{self, PrefixOutcome};
use heronite_data::DataPaths;
use heronite_depot as depot;
use heronite_epic_api::installed::{self, Install, InstalledGame};
use heronite_epic_api::library::{self as lib, GameMeta};
use heronite_epic_api::{info as api_info, legendary_config, Manifest};
use heronite_i18n::tr;
use serde_json::{json, Value};

use crate::client::Epic;
use crate::cmd::Cancel;
use crate::download::{self, RepairStep, Request};
use crate::library;
use crate::manager::{Job, JobResult};
use crate::progress::Progress;
use crate::queue::{DmElement, DmStatus, ElementKind};
use crate::third_party::{self, Provider};

/// Settings of `config.json` that change how games download (every store's).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DownloadSettings {
    pub max_workers: u32,
    pub no_https: bool,
}

impl DownloadSettings {
    pub fn load(data_paths: &DataPaths) -> Self {
        let g = game_config::load_global(data_paths, &home());
        Self {
            max_workers: g.get("maxWorkers").and_then(Value::as_u64).unwrap_or(0) as u32,
            no_https: g.get("downloadNoHttps").and_then(Value::as_bool).unwrap_or(false),
        }
    }
}

/// The user's folder, for `~` in the settings (`HOME`; Windows sets none
/// of its own: the profile folder there).
pub fn home() -> String {
    if cfg!(windows) {
        if let Some(home) = heronite_platform::dirs::home() {
            return home.to_string_lossy().into_owned();
        }
    }
    std::env::var("HOME").unwrap_or_default()
}

/// The install tags of a new install: the ones picked in the install
/// dialog, else (a game with selective downloads installed without asking)
/// the untagged files and the required components, like `--skip-sdl`;
/// `None` installs every file.
fn install_tags(data_paths: &DataPaths, app_name: &str, picked: &[String]) -> Option<Vec<String>> {
    if !picked.is_empty() {
        return Some(picked.to_vec());
    }
    let sdl = crate::info::game_sdl(data_paths, app_name);
    let required = sdl.iter().find(|s| s.required)?;
    let mut tags = vec![String::new()];
    tags.extend(required.tags.iter().cloned());
    Some(tags)
}

/// Port of `LegendaryGame.install` / `update` as a download-manager job.
pub struct EpicJob {
    pub data_paths: DataPaths,
    pub epic: Epic,
}

impl EpicJob {
    fn run_install(&self, el: &DmElement, cancel: &Cancel, on_progress: &mut dyn FnMut(Progress)) -> JobResult {
        let p = &el.params;
        match third_party::managed_by(&p.game_info) {
            Some(Ok(provider)) => return self.run_third_party(el, provider, cancel, on_progress),
            Some(Err(app)) => return JobResult::error(format!("{} is managed by {app}, which is not supported", p.title())),
            None => {}
        }
        let platform = if p.platform_to_install.is_empty() { "Windows" } else { p.platform_to_install.as_str() };
        let update = el.kind == ElementKind::Update;
        let req = Request {
            app_name: p.app_name.clone(),
            platform: platform.to_string(),
            base_path: p.path.clone(),
            install_tags: if update { None } else { install_tags(&self.data_paths, &p.app_name, &p.sdl_list) },
            ignore_free_space: p.ignore_free_space,
        };
        let settings = DownloadSettings::load(&self.data_paths);
        match download::install(&self.epic, &req, update, &settings, cancel, &mut |d| on_progress(Progress::from(d))) {
            Err(_) if cancel.is_requested() => JobResult::abort(),
            Err(err) => {
                log::error!("epic: {} of {} failed: {err:#}", if update { "update" } else { "install" }, p.app_name);
                JobResult::error(format!("{err:#}"))
            }
            Ok(depot::Outcome::Cancelled) => JobResult::abort(),
            Ok(depot::Outcome::Done) => {
                if let Err(err) = library::write_library(&self.data_paths) {
                    log::warn!("rebuilding the library: {err:#}");
                }
                JobResult::done()
            }
        }
    }

    /// `installEA` / `installUbisoft` on Linux: fetch the store's installer
    /// (run in the prefix at the first launch) and mark the game installed.
    fn run_third_party(&self, el: &DmElement, provider: Provider, cancel: &Cancel, on_progress: &mut dyn FnMut(Progress)) -> JobResult {
        // Windows: a store app already installed opens the game; nothing to fetch.
        let have_app = cfg!(windows) && heronite_launch::legendary_windows::link_handler_installed(provider.link_scheme());
        if have_app {
            log::info!("{} is installed: no installer to fetch", provider.label());
        } else if let Err(err) = third_party::ensure_installer(&self.data_paths, provider, cancel, on_progress) {
            if cancel.is_requested() {
                return JobResult::abort();
            }
            return JobResult::error(tr!(
                "Could not download the {store} installer: {error}",
                store = provider.label(),
                error = format!("{err:#}")
            ));
        }
        let platform = if el.params.platform_to_install.is_empty() { "Windows" } else { el.params.platform_to_install.as_str() };
        if let Err(err) = third_party::add_installed(&self.data_paths, &el.params.app_name, platform) {
            return JobResult::error(format!("{err:#}"));
        }
        if let Err(err) = library::write_library(&self.data_paths) {
            log::warn!("rebuilding the library: {err:#}");
        }
        JobResult::done()
    }
}

impl Job for EpicJob {
    fn run(&self, el: &DmElement, cancel: &Cancel, on_progress: &mut dyn FnMut(Progress)) -> JobResult {
        self.run_install(el, cancel, on_progress)
    }
}

/// Result of a one-off operation, for the UI to show.
#[derive(Debug, Clone, Default)]
pub struct OpResult {
    pub ok: bool,
    pub aborted: bool,
    /// Done, but `message` is a problem to read (shown as an error).
    pub warning: bool,
    pub message: String,
}

impl OpResult {
    pub(crate) fn ok(message: impl Into<String>) -> Self {
        Self { ok: true, aborted: false, warning: false, message: message.into() }
    }
    pub fn warn(message: impl Into<String>) -> Self {
        Self { ok: true, aborted: false, warning: true, message: message.into() }
    }
    pub(crate) fn err(message: impl Into<String>) -> Self {
        Self { ok: false, aborted: false, warning: false, message: message.into() }
    }
    pub(crate) fn cancelled() -> Self {
        Self { ok: false, aborted: true, warning: false, message: tr!("Cancelled").into() }
    }
}

/// Percent of `repair` (file verification, then re-download of broken files).
#[derive(Debug, Clone, PartialEq)]
pub enum RepairUpdate {
    Verifying(f64),
    Downloading(Progress),
}

/// `repair`: verify the files and download what is missing or corrupted.
pub fn repair(data_paths: &DataPaths, epic: &Epic, app_name: &str, cancel: &Cancel, mut on_update: impl FnMut(RepairUpdate)) -> OpResult {
    let settings = DownloadSettings::load(data_paths);
    let out = download::repair(epic, app_name, &settings, cancel, &mut |step| match step {
        RepairStep::Verifying(p) => on_update(RepairUpdate::Verifying(p)),
        RepairStep::Downloading(p) => on_update(RepairUpdate::Downloading(Progress::from(&p))),
    });
    match out {
        Err(_) if cancel.is_requested() => OpResult::cancelled(),
        Err(err) => OpResult::err(format!("{err:#}")),
        Ok(depot::Outcome::Cancelled) => OpResult::cancelled(),
        Ok(depot::Outcome::Done) => {
            let _ = library::write_library(data_paths);
            OpResult::ok(tr!("Finished repairing"))
        }
    }
}

/// `uninstall` and rebuilds the library file. The caller removes the
/// prefix / settings afterwards if the user asked for it. An EA app /
/// Ubisoft Connect game only leaves `third-party-installed.json` (the game
/// itself lives inside the prefix, with its store app).
pub fn uninstall(data_paths: &DataPaths, epic: &Epic, app_name: &str) -> OpResult {
    let r = uninstall_game(data_paths, epic, app_name);
    if r.ok {
        // Installed again, it runs its prerequisites again (Windows).
        heronite_launch::native_setup::forget(data_paths, "legendary", app_name);
    }
    r
}

fn uninstall_game(data_paths: &DataPaths, epic: &Epic, app_name: &str) -> OpResult {
    if third_party::is_installed(data_paths, app_name) {
        return match third_party::remove_installed(data_paths, app_name) {
            Ok(_) => {
                let _ = library::write_library(data_paths);
                OpResult::ok(tr!("Uninstalled"))
            }
            Err(err) => OpResult::err(format!("{err:#}")),
        };
    }
    let dir = &epic.config_dir;
    let Some(game) = installed::get(dir, app_name) else {
        let _ = library::write_library(data_paths);
        return OpResult::ok(tr!("Uninstalled"));
    };
    // A folder already gone has nothing to delete: only the record goes.
    if install_folder_missing(data_paths, app_name) {
        return force_uninstall(data_paths, epic, app_name);
    }
    // Its DLCs first (they share the folder), then the game and its folder
    // once empty (legendary's `uninstall_game`).
    let mut problem = None;
    if !game.is_dlc {
        for dlc in GameMeta::load(dir, app_name).map(|g| lib::dlcs_of(dir, &g)).unwrap_or_default() {
            if let Some(idlc) = installed::get(dir, &dlc.app_name) {
                if let Err(err) = download::delete_files(dir, &idlc, false) {
                    problem.get_or_insert(err);
                }
                if let Err(err) = installed::remove(dir, &idlc.app_name) {
                    return OpResult::err(format!("{err:#}"));
                }
            }
        }
    }
    if let Err(err) = download::delete_files(dir, &game, !game.is_dlc) {
        problem.get_or_insert(err);
    }
    if let Err(err) = installed::remove(dir, app_name) {
        return OpResult::err(format!("{err:#}"));
    }
    let _ = std::fs::remove_file(download::state_file(dir, app_name));
    let _ = library::write_library(data_paths);
    match problem {
        None => OpResult::ok(tr!("Uninstalled")),
        Some(err) => {
            log::warn!("epic: uninstalling {app_name}: {err:#}");
            OpResult::warn(tr!(
                "Uninstalled, but some files are left in {path}: {error}",
                path = game.install_path.as_str(),
                error = format!("{err:#}")
            ))
        }
    }
}

/// The install is recorded at a folder that no longer exists.
fn install_folder_missing(data_paths: &DataPaths, app_name: &str) -> bool {
    let path = installed::get(&data_paths.legendary_config_dir(), app_name).map(|g| g.install_path).unwrap_or_default();
    !path.is_empty() && !Path::new(&path).exists()
}

/// `forceUninstall`: forget the install in `installed.json` but keep the files.
pub fn force_uninstall(data_paths: &DataPaths, epic: &Epic, app_name: &str) -> OpResult {
    match installed::remove(&epic.config_dir, app_name) {
        Err(err) => OpResult::err(format!("{err:#}")),
        Ok(()) => {
            let _ = library::write_library(data_paths);
            OpResult::ok(tr!("Removed from the installed games"))
        }
    }
}

/// Points the install of `app_name` (and of its DLCs in the same folder)
/// at `new_path` without moving anything (`legendary move --skip-move`).
pub(crate) fn record_install_path(config_dir: &Path, app_name: &str, new_path: &str) -> Result<()> {
    let Some(mut game) = installed::get(config_dir, app_name) else { bail!(tr!("The game is not installed")) };
    let old = std::mem::replace(&mut game.install_path, new_path.to_string());
    installed::set(config_dir, &game)?;
    for (_, mut other) in installed::read_all(config_dir) {
        if other.is_dlc && other.install_path == old && other.app_name != app_name {
            other.install_path = new_path.to_string();
            installed::set(config_dir, &other)?;
        }
    }
    Ok(())
}

/// The game's folder was moved or renamed outside the app: checks that
/// `folder` holds the game, then points the install at it without moving
/// anything. Epic games keep the folder's own name (only the parent
/// changes), so a renamed folder is refused with a hint.
pub fn relocate(data_paths: &DataPaths, epic: &Epic, app_name: &str, folder: &str) -> OpResult {
    let folder = folder.trim().trim_end_matches(SEPARATORS);
    let dir = Path::new(folder);
    if !dir.is_dir() {
        return OpResult::err(tr!("The folder does not exist"));
    }
    let Some(entry) = installed::get(&epic.config_dir, app_name) else { return OpResult::err(tr!("The game is not installed")) };
    let old_name = Path::new(entry.install_path.trim_end_matches(SEPARATORS)).file_name();
    if old_name.is_some() && !same_name(dir.file_name(), old_name) {
        let name = old_name.map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        return OpResult::err(tr!("Epic games keep the name of their folder: pick a folder named {name}, or rename it back.", name));
    }
    let exe = entry.executable.replace('\\', "/");
    if !exe.is_empty() && !dir.join(&exe).exists() {
        return OpResult::err(tr!("That folder does not have the game's {exe}", exe));
    }
    match record_install_path(&epic.config_dir, app_name, folder) {
        Err(err) => OpResult::err(format!("{err:#}")),
        Ok(()) => {
            let _ = library::write_library(data_paths);
            OpResult::ok(tr!("Install location changed"))
        }
    }
}

/// What may end a folder path: `/`, and `\` on Windows (Epic paths are
/// written `C:\Games\Control` there).
const SEPARATORS: &[char] = if cfg!(windows) { &['/', '\\'] } else { &['/'] };

/// Folder names compare the way the file system does: without case on
/// Windows.
fn same_name(a: Option<&std::ffi::OsStr>, b: Option<&std::ffi::OsStr>) -> bool {
    match (a.and_then(|a| a.to_str()), b.and_then(|b| b.to_str())) {
        (Some(x), Some(y)) if cfg!(windows) => x.eq_ignore_ascii_case(y),
        _ => a == b,
    }
}

/// `importGame`: records an existing folder as the install of a game (and
/// of its DLCs found in it), like `legendary import --with-dlcs`. The
/// Epic Games Launcher's own record in the folder (`.egstore`) gives the
/// installed version; without one the files must be verified before the
/// game can update. Blocking, network.
pub fn import_game(data_paths: &DataPaths, epic: &Epic, app_name: &str, folder: &str, platform: &str) -> OpResult {
    match import(epic, app_name, folder, platform, None) {
        Ok(()) => {
            let _ = library::write_library(data_paths);
            OpResult::ok(tr!("Game imported"))
        }
        Err(err) => OpResult::err(format!("{err:#}")),
    }
}

/// The work of [`import_game`]; `egl_guid` names the launcher's install
/// (`egl`).
pub(crate) fn import(epic: &Epic, app_name: &str, folder: &str, platform: &str, egl_guid: Option<&str>) -> Result<()> {
    let dir = &epic.config_dir;
    let path = Path::new(folder);
    if !path.is_dir() {
        bail!(tr!("The folder does not exist"));
    }
    if installed::get(dir, app_name).is_some() {
        bail!(tr!("The game is already installed"));
    }
    let session = epic.session()?;
    let game = GameMeta::load(dir, app_name)
        .ok_or_else(|| anyhow::anyhow!(tr!("{app_name} is not in the Epic library: refresh it and try again", app_name)))?;
    let platform = if platform.is_empty() { "Windows" } else { platform };
    import_one(epic, &session, &game, path, platform, egl_guid, true)?;
    // Its DLCs in the same folder.
    for dlc in lib::dlcs_of(dir, &game) {
        if installed::get(dir, &dlc.app_name).is_some() || !dlc.asset_infos.contains_key(platform) {
            continue;
        }
        if let Err(err) = import_one(epic, &session, &dlc, path, platform, None, false) {
            log::info!("epic: DLC {} is not in {}: {err:#}", dlc.app_name, path.display());
        }
    }
    Ok(())
}

/// Records `game` as installed in `path` when its files are there.
fn import_one(
    epic: &Epic,
    session: &heronite_epic_api::Session,
    game: &GameMeta,
    path: &Path,
    platform: &str,
    egl_guid: Option<&str>,
    check_exe: bool,
) -> Result<()> {
    let dir = &epic.config_dir;
    let egl = (!game.is_dlc()).then(|| installed::egl_install(path, &game.app_name, egl_guid)).flatten();
    let (manifest, base_urls, needs_verification) = match &egl {
        Some(e) => (Manifest::parse(&e.manifest)?, game.base_urls.clone(), e.needs_verification),
        None => {
            let f = api_info::fetch_manifest(&epic.client, session.access_token(), game, platform)?;
            (f.manifest, f.info.base_urls, true)
        }
    };
    let found = manifest.files.iter().filter(|f| path.join(&f.filename).exists()).count();
    if found == 0 {
        bail!(tr!("None of the files of {title} are in {path}", title = game.app_title.as_str(), path = path.display()));
    }
    let exe = manifest.meta.launch_exe.trim_start_matches('/');
    if check_exe && !exe.is_empty() && !heronite_epic_api::saves::case_insensitive(&path.join(exe)).exists() {
        bail!(tr!("That folder does not have the game's {exe}", exe));
    }
    if (found as f64) < manifest.files.len() as f64 * 0.95 {
        log::warn!("epic: {} of {} files of {} found in {}", found, manifest.files.len(), game.app_name, path.display());
    }
    installed::save_manifest(dir, &game.app_name, platform, &manifest)?;
    let exe_override = legendary_config::cached_exe_override(dir, &game.app_name);
    let install_size = manifest.files.iter().map(|f| f.size()).sum();
    let mut record = InstalledGame::from_install(&Install {
        game,
        manifest: &manifest,
        install_path: &path.to_string_lossy(),
        platform,
        install_tags: &[],
        install_size,
        base_urls: &base_urls,
        is_dlc: game.is_dlc(),
        egl_guid: egl.as_ref().map_or("", |e| e.guid.as_str()),
        is_preloaded: false,
        exe_override: exe_override.as_deref(),
    });
    record.needs_verification = needs_verification;
    installed::set(dir, &record)
}

/// `moveOnUnix` + `changeGameInstallPath`: moves the install folder into
/// `new_parent` and records where it is now (resumable, see
/// [`crate::moves`]).
pub fn move_install(data_paths: &DataPaths, epic: &Epic, app_name: &str, install_path: &str, new_parent: &str) -> OpResult {
    crate::moves::move_install(data_paths, epic, app_name, install_path, new_parent)
}

/// `removePrefix`: deletes the game's Wine prefix, with Heroic's sanity checks
/// (see [`sideload::remove_prefix`]); `Skipped` says why it was kept.
pub fn remove_prefix(data_paths: &DataPaths, app_name: &str) -> PrefixOutcome {
    sideload::remove_prefix(data_paths, app_name, &home())
}

/// [`remove_prefix`] for an uninstall: `Some(note)` when the prefix stayed,
/// for the toast.
pub fn remove_prefix_note(data_paths: &DataPaths, app_name: &str) -> Option<String> {
    match remove_prefix(data_paths, app_name) {
        PrefixOutcome::Removed(_) => None,
        PrefixOutcome::Skipped(why) => {
            log::warn!("Wine prefix of {app_name} kept: {why}");
            Some(tr!("Wine prefix kept: {why}", why))
        }
    }
}

/// `removeSettingsAndLogs`.
pub fn remove_settings_and_logs(data_paths: &DataPaths, app_name: &str) {
    for ext in [".json", ".log", "-lastPlay.log"] {
        let f = data_paths.games_config_dir().join(format!("{app_name}{ext}"));
        if f.exists() {
            let _ = std::fs::remove_file(f);
        }
    }
}

/// Writes the Wine settings picked in the install dialog (Heroic's
/// `writeConfig` at install time on Linux).
pub fn write_wine_config(data_paths: &DataPaths, app_name: &str, wine_prefix: &str, wine_version: Option<&Value>) -> Result<()> {
    if wine_prefix.is_empty() && wine_version.is_none() {
        return Ok(());
    }
    let mut changes: Vec<(String, Value)> = Vec::new();
    if !wine_prefix.is_empty() {
        changes.push(("winePrefix".into(), json!(wine_prefix)));
    }
    if let Some(v) = wine_version {
        changes.push(("wineVersion".into(), v.clone()));
        changes.push(("wineCrossoverBottle".into(), json!("")));
    }
    game_config::set_game_settings(data_paths, app_name, &home(), &changes)
}

/// Removes `<path>/<folder_name>` (the partial download of a cancelled
/// install). Refuses anything that is not a plain folder name.
pub fn remove_partial_download(path: &str, folder_name: &str) -> bool {
    if path.is_empty() || folder_name.is_empty() || folder_name.contains(SEPARATORS) || folder_name == ".." || folder_name == "." {
        return false;
    }
    let target = Path::new(path.trim_matches('\'')).join(folder_name);
    target.is_dir() && std::fs::remove_dir_all(target).is_ok()
}

impl JobResult {
    pub fn done() -> Self {
        Self { status: DmStatus::Done, error: None }
    }
    pub fn abort() -> Self {
        Self { status: DmStatus::Abort, error: None }
    }
    pub fn error(message: impl Into<String>) -> Self {
        Self { status: DmStatus::Error, error: Some(message.into()) }
    }
}

/// Convenience: errors if legendary is not logged in.
pub fn ensure_logged_in(data_paths: &DataPaths) -> Result<()> {
    if !crate::user::is_logged_in(data_paths) {
        bail!(tr!("Log in to your Epic account first"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::download::testing::{serve_build, sign_in_and_own, Server};
    use crate::queue::InstallParams;

    struct Fx {
        dir: tempfile::TempDir,
        data_paths: DataPaths,
        server: Server,
        job: EpicJob,
    }

    impl Fx {
        fn new() -> Self {
            let dir = tempfile::tempdir().unwrap();
            let data_paths = DataPaths::at(dir.path().join("h"));
            let server = Server::start();
            sign_in_and_own(&data_paths.legendary_config_dir(), "Calluna", "Control");
            let job = EpicJob { data_paths: data_paths.clone(), epic: server.epic(&data_paths.legendary_config_dir()) };
            Self { dir, data_paths, server, job }
        }

        fn epic(&self) -> &Epic {
            &self.job.epic
        }

        fn games(&self) -> String {
            self.dir.path().join("games").to_string_lossy().into_owned()
        }

        fn element(&self, kind: ElementKind) -> DmElement {
            DmElement::new(
                kind,
                InstallParams {
                    app_name: "Calluna".into(),
                    path: self.games(),
                    runner: "legendary".into(),
                    platform_to_install: "Windows".into(),
                    game_info: json!({"title": "Control"}),
                    ..Default::default()
                },
            )
        }

        /// Installs the build `version` through the job.
        fn install(&self, version: &str, files: &[(&str, &[u8])]) -> JobResult {
            serve_build(&self.server, "Calluna", version, files);
            self.job.run(&self.element(ElementKind::Install), &Cancel::new(), &mut |_| {})
        }

        fn root(&self) -> std::path::PathBuf {
            self.dir.path().join("games").join("Control")
        }
    }

    #[test]
    fn install_and_update_jobs_report_progress_and_rebuild_the_library() {
        let fx = Fx::new();
        serve_build(&fx.server, "Calluna", "1", &[("Control.exe", b"MZ 1")]);
        let mut seen = Vec::new();
        let r = fx.job.run(&fx.element(ElementKind::Install), &Cancel::new(), &mut |p| seen.push(p));
        assert_eq!(r.status, DmStatus::Done, "{:?}", r.error);
        assert_eq!(seen.last().map(|p| p.percent), Some(100.));
        assert_eq!(std::fs::read(fx.root().join("Control.exe")).unwrap(), b"MZ 1");
        // The library file was rebuilt with the game installed.
        let lib = crate::cache_store::CacheStore::library(&fx.data_paths).get("library").unwrap();
        assert_eq!(lib[0]["is_installed"], true, "{lib}");

        serve_build(&fx.server, "Calluna", "2", &[("Control.exe", b"MZ 2")]);
        let r = fx.job.run(&fx.element(ElementKind::Update), &Cancel::new(), &mut |_| {});
        assert_eq!(r.status, DmStatus::Done, "{:?}", r.error);
        assert_eq!(std::fs::read(fx.root().join("Control.exe")).unwrap(), b"MZ 2");
        assert_eq!(installed::get(&fx.epic().config_dir, "Calluna").unwrap().version, "2");
    }

    /// Windows: the folder is written as Windows writes it.
    #[cfg(windows)]
    #[test]
    fn a_windows_base_path_is_kept_as_it_is() {
        let fx = Fx::new();
        assert_eq!(fx.install("1", &[("Control.exe", b"MZ")]).status, DmStatus::Done);
        let rec = installed::get(&fx.epic().config_dir, "Calluna").unwrap();
        assert!(rec.install_path.ends_with(r"games\Control"), "{}", rec.install_path);
    }

    #[test]
    fn failures_and_unsupported_stores_are_errors() {
        let fx = Fx::new();
        // Epic does not know the build.
        let r = fx.job.run(&fx.element(ElementKind::Install), &Cancel::new(), &mut |_| {});
        assert_eq!(r.status, DmStatus::Error);
        assert!(r.error.is_some_and(|e| !e.is_empty()));
        let mut third = fx.element(ElementKind::Install);
        third.params.game_info = json!({"title": "Overwatch", "thirdPartyManagedApp": "Battle.net"});
        let r = fx.job.run(&third, &Cancel::new(), &mut |_| {});
        assert_eq!(r.status, DmStatus::Error);
        assert!(r.error.unwrap().contains("Battle.net, which is not supported"));
    }

    #[test]
    fn an_expired_sign_in_fails_the_job_with_its_message() {
        let fx = Fx::new();
        let user = fx.epic().config_dir.join("user.json");
        std::fs::write(
            &user,
            json!({"account_id": "a", "access_token": "t", "refresh_token": "r", "expires_at": "2000-01-01T00:00:00.000Z"}).to_string(),
        )
        .unwrap();
        fx.server.route(
            "/account/api/oauth/token",
            400,
            r#"{"errorCode": "errors.com.epicgames.account.auth_token.invalid_refresh_token"}"#,
        );
        let r = fx.job.run(&fx.element(ElementKind::Install), &Cancel::new(), &mut |_| {});
        assert_eq!(r.status, DmStatus::Error);
        assert!(crate::session::expired_message(&r.error.unwrap()));
    }

    #[test]
    fn ea_and_ubisoft_games_install_and_uninstall_without_epics_cdn() {
        let fx = Fx::new();
        // The installer is already there: nothing to download.
        let redist = third_party::redist_dir(&fx.data_paths);
        std::fs::create_dir_all(&redist).unwrap();
        std::fs::write(redist.join("UbisoftConnectInstaller.exe"), "MZ").unwrap();
        let mut el = fx.element(ElementKind::Install);
        el.params.app_name = "Anchovy".into();
        el.params.path = "thirdParty".into();
        el.params.game_info = json!({"title": "Rayman Legends", "thirdPartyManagedApp": "UbisoftConnect"});
        let r = fx.job.run(&el, &Cancel::new(), &mut |_| {});
        assert_eq!(r.status, DmStatus::Done, "{:?}", r.error);
        assert!(third_party::is_installed(&fx.data_paths, "Anchovy"));
        assert!(fx.data_paths.legendary_library().exists());
        let r = uninstall(&fx.data_paths, fx.epic(), "Anchovy");
        assert!(r.ok, "{}", r.message);
        assert!(!third_party::is_installed(&fx.data_paths, "Anchovy"));
        assert!(fx.server.seen.lock().unwrap().is_empty(), "Epic was not asked");
    }

    #[test]
    fn cancelled_jobs_abort() {
        let fx = Fx::new();
        let cancel = Cancel::new();
        cancel.request();
        serve_build(&fx.server, "Calluna", "1", &[("Control.exe", b"MZ")]);
        let r = fx.job.run(&fx.element(ElementKind::Install), &cancel, &mut |_| {});
        assert_eq!(r.status, DmStatus::Abort);
        assert!(installed::get(&fx.epic().config_dir, "Calluna").is_none());
    }

    #[test]
    fn uninstall_deletes_the_game_and_force_uninstall_only_forgets_it() {
        let fx = Fx::new();
        assert_eq!(fx.install("1", &[("Control.exe", b"MZ"), ("data/a.pak", b"pak")]).status, DmStatus::Done);
        assert!(force_uninstall(&fx.data_paths, fx.epic(), "Calluna").ok);
        assert!(installed::get(&fx.epic().config_dir, "Calluna").is_none());
        assert!(fx.root().join("Control.exe").exists(), "the files stay");

        assert_eq!(fx.install("1", &[("Control.exe", b"MZ"), ("data/a.pak", b"pak")]).status, DmStatus::Done);
        std::fs::write(fx.root().join("data/save.sav"), "mine").unwrap();
        let r = uninstall(&fx.data_paths, fx.epic(), "Calluna");
        assert!(r.ok && !r.warning, "{}", r.message);
        assert!(installed::get(&fx.epic().config_dir, "Calluna").is_none());
        assert!(!fx.root().join("Control.exe").exists() && !fx.root().join("data/a.pak").exists());
        assert!(fx.root().join("data/save.sav").exists(), "files the game made stay");
    }

    #[test]
    fn an_uninstalled_game_runs_its_first_launch_setup_again() {
        let fx = Fx::new();
        assert_eq!(fx.install("1", &[("Control.exe", b"MZ")]).status, DmStatus::Done);
        heronite_launch::native_setup::record(&fx.data_paths, "legendary", "Calluna", "/games/Control").unwrap();
        assert!(uninstall(&fx.data_paths, fx.epic(), "Calluna").ok);
        assert!(!heronite_launch::native_setup::is_recorded(&fx.data_paths, "legendary", "Calluna", "/games/Control"));
    }

    #[test]
    fn a_game_whose_folder_is_gone_is_only_forgotten() {
        let fx = Fx::new();
        assert_eq!(fx.install("1", &[("Control.exe", b"MZ")]).status, DmStatus::Done);
        std::fs::remove_dir_all(fx.root()).unwrap();
        let r = uninstall(&fx.data_paths, fx.epic(), "Calluna");
        assert!(r.ok, "{}", r.message);
        assert!(installed::get(&fx.epic().config_dir, "Calluna").is_none());
    }

    #[test]
    fn import_records_a_folder_with_the_game_and_refuses_others() {
        let fx = Fx::new();
        serve_build(&fx.server, "Calluna", "3", &[("Control.exe", b"MZ")]);
        let empty = fx.dir.path().join("empty");
        std::fs::create_dir_all(&empty).unwrap();
        let r = import_game(&fx.data_paths, fx.epic(), "Calluna", empty.to_str().unwrap(), "Windows");
        assert!(!r.ok && r.message.contains("None of the files"), "{}", r.message);
        assert!(!import_game(&fx.data_paths, fx.epic(), "Calluna", "/nonexistent-dir", "Windows").ok);
        let r = import_game(&fx.data_paths, fx.epic(), "Other", empty.to_str().unwrap(), "Windows");
        assert!(!r.ok && r.message.contains("not in the Epic library"), "{}", r.message);

        let game = fx.dir.path().join("Control");
        std::fs::create_dir_all(&game).unwrap();
        std::fs::write(game.join("Control.exe"), "MZ").unwrap();
        let r = import_game(&fx.data_paths, fx.epic(), "Calluna", game.to_str().unwrap(), "Windows");
        assert!(r.ok, "{}", r.message);
        let rec = installed::get(&fx.epic().config_dir, "Calluna").unwrap();
        assert_eq!((rec.version.as_str(), rec.install_path.as_str()), ("3", game.to_str().unwrap()));
        assert!(rec.needs_verification, "not from the Epic Games Launcher: to be checked");
        let again = import_game(&fx.data_paths, fx.epic(), "Calluna", game.to_str().unwrap(), "Windows");
        assert!(!again.ok && again.message.contains("already installed"), "{}", again.message);
    }

    #[test]
    fn relocate_checks_the_folder_then_records_it() {
        let fx = Fx::new();
        let cfg = &fx.epic().config_dir;
        let old = if cfg!(windows) { r"D:\old\Control" } else { "/old/Control" };
        let game = InstalledGame {
            app_name: "Calluna".into(),
            install_path: old.into(),
            executable: "Bin\\Control.exe".into(),
            ..Default::default()
        };
        installed::set(cfg, &game).unwrap();
        let moved = fx.dir.path().join("disk2").join("Control");
        std::fs::create_dir_all(moved.join("Bin")).unwrap();
        // Not the game yet: its executable is missing.
        assert!(!relocate(&fx.data_paths, fx.epic(), "Calluna", moved.to_str().unwrap()).ok);
        std::fs::write(moved.join("Bin").join("Control.exe"), "").unwrap();
        let renamed = fx.dir.path().join("disk2").join("Ctrl");
        std::fs::create_dir_all(&renamed).unwrap();
        let r = relocate(&fx.data_paths, fx.epic(), "Calluna", renamed.to_str().unwrap());
        assert!(!r.ok && r.message.contains("Control"), "a renamed folder is refused: {}", r.message);
        assert!(!relocate(&fx.data_paths, fx.epic(), "Other", moved.to_str().unwrap()).ok, "not installed");
        assert!(!relocate(&fx.data_paths, fx.epic(), "Calluna", "/nonexistent-dir/Control").ok);
        assert_eq!(installed::get(cfg, "Calluna").unwrap().install_path, old, "nothing changed yet");
        let sep = std::path::MAIN_SEPARATOR;
        let r = relocate(&fx.data_paths, fx.epic(), "Calluna", &format!("{}{sep}", moved.display()));
        assert!(r.ok, "{}", r.message);
        assert_eq!(installed::get(cfg, "Calluna").unwrap().install_path, moved.to_string_lossy());
    }

    /// Windows' folder names ignore case: `control` is the game's `Control`.
    #[cfg(windows)]
    #[test]
    fn relocate_ignores_the_case_of_the_folder_name_on_windows() {
        let fx = Fx::new();
        let cfg = &fx.epic().config_dir;
        let game = InstalledGame {
            app_name: "Calluna".into(),
            install_path: r"C:\Games\Control".into(),
            executable: "Control.exe".into(),
            ..Default::default()
        };
        installed::set(cfg, &game).unwrap();
        let moved = fx.dir.path().join("control");
        std::fs::create_dir_all(&moved).unwrap();
        std::fs::write(moved.join("Control.exe"), "").unwrap();
        let r = relocate(&fx.data_paths, fx.epic(), "Calluna", moved.to_str().unwrap());
        assert!(r.ok, "{}", r.message);
    }

    #[test]
    fn repair_reports_verification_and_download() {
        let fx = Fx::new();
        assert_eq!(fx.install("1", &[("Control.exe", b"MZ"), ("data/a.pak", b"pak")]).status, DmStatus::Done);
        std::fs::write(fx.root().join("data/a.pak"), "broken").unwrap();
        let mut updates = Vec::new();
        let r = repair(&fx.data_paths, fx.epic(), "Calluna", &Cancel::new(), |u| updates.push(u));
        assert!(r.ok, "{}", r.message);
        assert_eq!(std::fs::read(fx.root().join("data/a.pak")).unwrap(), b"pak");
        assert!(matches!(updates[0], RepairUpdate::Verifying(_)));
        assert!(updates.iter().any(|u| matches!(u, RepairUpdate::Downloading(_))));
        assert!(!repair(&fx.data_paths, fx.epic(), "Other", &Cancel::new(), |_| {}).ok);
    }

    #[test]
    fn move_install_moves_the_folder_and_records_it() {
        let fx = Fx::new();
        let src = fx.dir.path().join("old").join("Control");
        std::fs::create_dir_all(src.join("sub")).unwrap();
        std::fs::write(src.join("sub").join("a.txt"), "x").unwrap();
        let cfg = &fx.epic().config_dir;
        installed::set(
            cfg,
            &InstalledGame { app_name: "Calluna".into(), install_path: src.to_string_lossy().into(), ..Default::default() },
        )
        .unwrap();
        let new_parent = fx.dir.path().join("new");
        let r = move_install(&fx.data_paths, fx.epic(), "Calluna", src.to_str().unwrap(), new_parent.to_str().unwrap());
        assert!(r.ok, "{}", r.message);
        assert!(new_parent.join("Control").join("sub").join("a.txt").exists());
        assert!(!src.exists());
        assert_eq!(installed::get(cfg, "Calluna").unwrap().install_path, new_parent.join("Control").to_string_lossy());
        // Into itself, and onto a non-empty destination: refused.
        let again = move_install(
            &fx.data_paths,
            fx.epic(),
            "Calluna",
            new_parent.join("Control").to_str().unwrap(),
            new_parent.join("Control").join("x").to_str().unwrap(),
        );
        assert!(!again.ok);
        let clash = fx.dir.path().join("clash").join("Control");
        std::fs::create_dir_all(&clash).unwrap();
        std::fs::write(clash.join("f"), "x").unwrap();
        let r = move_install(
            &fx.data_paths,
            fx.epic(),
            "Calluna",
            new_parent.join("Control").to_str().unwrap(),
            fx.dir.path().join("clash").to_str().unwrap(),
        );
        assert!(!r.ok && r.message.contains("not empty"));
    }

    #[test]
    fn prefix_removal_has_sanity_checks() {
        let dir = tempfile::tempdir().unwrap();
        let data_paths = DataPaths::at(dir.path().join("h"));
        let prefix = dir.path().join("pfx");
        std::fs::create_dir_all(prefix.join("drive_c")).unwrap();
        game_config::set_game_setting(&data_paths, "A", &home(), "winePrefix", json!(prefix.to_str().unwrap())).unwrap();
        assert!(matches!(remove_prefix(&data_paths, "A"), PrefixOutcome::Removed(_)));
        assert!(!prefix.exists());
        // A folder without drive_c/pfx is left alone.
        let other = dir.path().join("other");
        std::fs::create_dir_all(&other).unwrap();
        std::fs::write(other.join("file"), "x").unwrap();
        game_config::set_game_setting(&data_paths, "B", &home(), "winePrefix", json!(other.to_str().unwrap())).unwrap();
        assert!(matches!(remove_prefix(&data_paths, "B"), PrefixOutcome::Skipped(_)));
        assert!(remove_prefix_note(&data_paths, "B").is_some_and(|n| n.contains("drive_c")));
        assert!(other.exists());
        // The default install path is never deleted (a temp one: never touch real folders in tests).
        let default_install = dir.path().join("Games/Heroic");
        std::fs::create_dir_all(default_install.join("drive_c")).unwrap();
        game_config::set_global_setting(&data_paths, &home(), "defaultInstallPath", json!(default_install.to_str().unwrap())).unwrap();
        game_config::set_game_setting(&data_paths, "C", &home(), "winePrefix", json!(default_install.to_str().unwrap())).unwrap();
        assert!(matches!(remove_prefix(&data_paths, "C"), PrefixOutcome::Skipped(_)));
        assert!(default_install.exists());
    }

    #[test]
    fn partial_downloads_are_removed_only_for_plain_folder_names() {
        let dir = tempfile::tempdir().unwrap();
        let game = dir.path().join("Control");
        std::fs::create_dir_all(&game).unwrap();
        assert!(!remove_partial_download(dir.path().to_str().unwrap(), "../x"));
        assert!(!remove_partial_download(dir.path().to_str().unwrap(), ""));
        assert!(game.exists());
        assert!(remove_partial_download(dir.path().to_str().unwrap(), "Control"));
        assert!(!game.exists());
    }

    #[test]
    fn wine_choices_are_written_to_the_game_config() {
        let dir = tempfile::tempdir().unwrap();
        let data_paths = DataPaths::at(dir.path().join("h"));
        let wine = json!({"bin": "/p/proton", "name": "GE", "type": "proton"});
        write_wine_config(&data_paths, "Calluna", "/pfx/Control", Some(&wine)).unwrap();
        let s = game_config::game_overrides(&data_paths, "Calluna");
        assert_eq!(s["winePrefix"], "/pfx/Control");
        assert_eq!(s["wineVersion"]["name"], "GE");
        assert_eq!(s["wineCrossoverBottle"], "");
        let raw = std::fs::read_to_string(data_paths.game_config("Calluna")).unwrap();
        assert!(raw.contains("\n  \"Calluna\""), "2-space indent expected");
    }
}
