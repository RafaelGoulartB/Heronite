//! Installs, updates and repairs of Epic games: the build's manifest from
//! Epic, its plan ([`heronite_epic_api::manifest::plan`]) written by
//! [`heronite_depot`] with chunks from Epic's CDN, then legendary's records
//! of the install (`installed.json` and the stored manifest), so the game is
//! the same as one legendary installed.
//!
//! A run that stops (cancelled, the app closed, the connection lost) leaves
//! a resume journal in legendary's `tmp` folder: the next run of the same
//! build keeps the files already written.

use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Context, Result};
use heronite_data::proc::Cancel;
use heronite_depot as depot;
use heronite_epic_api::api::{without_https, Asset, Ticket};
use heronite_epic_api::crypt::Secrets;
use heronite_epic_api::files::clean_filename;
use heronite_epic_api::installed::{self, Install, InstalledGame};
use heronite_epic_api::library::{self as lib, GameMeta};
use heronite_epic_api::manifest::plan::{self, InstallPlan, PlanOptions, PlannedPart};
use heronite_epic_api::manifest::{ChunkInfo, Guid, Manifest};
use heronite_epic_api::{info, legendary_config, Client, Session};
use heronite_i18n::tr;

use crate::client::Epic;
use crate::disk;
use crate::ops::DownloadSettings;

/// A new install or an update of an installed game.
#[derive(Debug, Clone, Default)]
pub struct Request {
    pub app_name: String,
    /// `Windows` or `Mac`.
    pub platform: String,
    /// The folder the game's own folder goes in (new installs; an update
    /// keeps the game where it is, a DLC goes into its game's folder).
    pub base_path: String,
    /// Install tags (selective downloads); `None` installs every file. An
    /// update keeps the tags of the install.
    pub install_tags: Option<Vec<String>>,
    pub ignore_free_space: bool,
}

/// What a repair reports.
#[derive(Debug, Clone, PartialEq)]
pub enum RepairStep {
    /// Percent of the files checked.
    Verifying(f64),
    Downloading(depot::Progress),
}

/// The resume journal of `app_name`'s download (legendary's own `.resume`
/// files have another format: the name differs).
pub fn state_file(config_dir: &Path, app_name: &str) -> PathBuf {
    config_dir.join("tmp").join(format!("{}.heronite-resume", clean_filename(app_name)))
}

/// The game a DLC belongs to (`mainGameItem`).
fn main_game(game: &GameMeta) -> Option<String> {
    game.metadata.pointer("/mainGameItem/releaseInfo/0/appId").and_then(|v| v.as_str()).filter(|s| !s.is_empty()).map(str::to_string)
}

/// `FolderName` of the catalog when it is a plain folder name, else the
/// app name.
fn folder_name(game: &GameMeta) -> String {
    game.custom_attribute("FolderName").and_then(depot::folder_name).unwrap_or(&game.app_name).to_string()
}

/// Joins the base path the user picked and the game's folder, the way the
/// system writes paths.
fn join_install_path(base: &str, folder: &str) -> String {
    let base = base.trim().trim_matches('\'');
    Path::new(base).join(folder).to_string_lossy().into_owned()
}

/// The CDN chunks of one build. Chunk ids of the plan are indexes into
/// `chunks`.
struct Cdn<'a> {
    client: &'a Client,
    base_urls: Vec<String>,
    chunks: Vec<ChunkInfo>,
    paths: Vec<String>,
    secrets: Secrets,
    /// Builds served through signed URLs only.
    signed: Option<(Session, Ticket)>,
}

impl depot::Source for Cdn<'_> {
    fn fetch(&self, chunk: &depot::Chunk, _cancel: &Cancel) -> Result<Vec<u8>> {
        let i: usize = chunk.id.parse().context("chunk id")?;
        let (info, path) = (&self.chunks[i], &self.paths[i]);
        match &self.signed {
            Some((session, ticket)) => {
                let urls = self.client.signed_chunk_urls(session, ticket, std::slice::from_ref(path))?;
                let url = urls.get(path).ok_or_else(|| anyhow!("Epic gave no download address for {path}"))?;
                self.client.fetch_signed_chunk(url, info, &self.secrets)
            }
            None => self.client.fetch_chunk(&self.base_urls, path, info, &self.secrets),
        }
    }
}

/// The depot plan of `p` (chunk ids are indexes into `p.chunks`) and the
/// chunks' paths below a base URL.
fn depot_plan(p: &InstallPlan, manifest: &Manifest) -> (depot::Plan, Vec<String>) {
    let index: HashMap<Guid, usize> = p.chunks.iter().enumerate().map(|(i, c)| (c.guid, i)).collect();
    let chunks = p
        .chunks
        .iter()
        .enumerate()
        .map(|(i, c)| depot::Chunk { id: i.to_string(), download_size: c.file_size, size: c.window_size as u64 })
        .collect();
    let files = p
        .files
        .iter()
        .map(|f| depot::FileTask {
            path: f.path.clone(),
            size: f.size,
            parts: f
                .parts
                .iter()
                .map(|part| match part {
                    PlannedPart::Chunk { guid, offset, size } => {
                        depot::Part::Chunk { chunk: index[guid], offset: *offset as u64, size: *size as u64 }
                    }
                    PlannedPart::Local { path, offset, size } => depot::Part::Local { path: path.clone(), offset: *offset, size: *size },
                })
                .collect(),
            hash: f.symlink.is_none().then_some(depot::Hash::Sha1(f.sha1)),
            executable: f.executable,
            symlink: f.symlink.clone(),
        })
        .collect();
    let paths = p.chunks.iter().map(|c| manifest.chunk_path(c)).collect();
    (depot::Plan { chunks, files, remove: p.remove.clone(), dirs: Vec::new() }, paths)
}

fn depot_options(config_dir: &Path, app_name: &str, settings: &DownloadSettings) -> depot::Options {
    depot::Options { workers: settings.max_workers as usize, max_memory: 0, state_file: Some(state_file(config_dir, app_name)) }
}

/// The asset of `app_name` for `platform` in `assets.json` (signed URLs
/// need it).
fn asset_of(config_dir: &Path, app_name: &str, platform: &str) -> Option<Asset> {
    lib::read_assets(config_dir).remove(platform)?.into_iter().find(|a| a.app_name == app_name)
}

/// The CDN source of a build.
fn cdn<'a>(
    epic: &'a Epic,
    session: Option<&Session>,
    app_name: &str,
    platform: &str,
    base_urls: &[String],
    use_signed_url: bool,
    secrets: &Secrets,
    settings: &DownloadSettings,
) -> Result<Cdn<'a>> {
    let signed = match (use_signed_url, session) {
        (true, Some(s)) => {
            let asset = asset_of(&epic.config_dir, app_name, platform).ok_or_else(|| anyhow!("{app_name} is not in the Epic library"))?;
            Some((s.clone(), epic.client.download_ticket(s, &asset)?))
        }
        (true, None) => bail!(tr!("Log in to your Epic account first")),
        _ => None,
    };
    let base_urls = base_urls.iter().map(|u| if settings.no_https { without_https(u) } else { u.clone() }).collect();
    Ok(Cdn { client: &epic.client, base_urls, chunks: Vec::new(), paths: Vec::new(), secrets: secrets.clone(), signed })
}

/// Fails when the install folder's disk does not have the room `plan` needs.
fn check_space(root: &Path, plan: &depot::Plan) -> Result<()> {
    let needed = depot::space_needed(root, plan);
    let Some(space) = disk::check(&root.to_string_lossy()) else { return Ok(()) };
    if needed > space.free {
        bail!(tr!("Not enough disk space"));
    }
    if !space.writable {
        bail!(tr!("No write access to {path}", path = root.display()));
    }
    Ok(())
}

/// Installs or updates `req.app_name` (`update` picks which). Blocking,
/// network. `on_progress` gets the download's progress a few times a second.
pub fn install(
    epic: &Epic,
    req: &Request,
    update: bool,
    settings: &DownloadSettings,
    cancel: &Cancel,
    on_progress: &mut dyn FnMut(&depot::Progress),
) -> Result<depot::Outcome> {
    let dir = &epic.config_dir;
    let session = epic.session()?;
    let game = GameMeta::load(dir, &req.app_name)
        .ok_or_else(|| anyhow!(tr!("{app_name} is not in the Epic library: refresh it and try again", app_name = req.app_name.as_str())))?;
    let old = installed::get(dir, &req.app_name);
    if update && old.is_none() {
        bail!(tr!("The game is not installed"));
    }
    let platform = old.as_ref().map(|o| o.platform.clone()).filter(|p| !p.is_empty()).unwrap_or_else(|| req.platform.clone());
    let platform = if platform.is_empty() { "Windows".to_string() } else { platform };

    // Where the files go: where the game is, its game's folder for a DLC,
    // else a new folder in the base path.
    let is_dlc = game.is_dlc();
    let install_path = match (&old, is_dlc) {
        (Some(o), _) if !o.install_path.is_empty() => o.install_path.clone(),
        (_, true) => {
            let main = main_game(&game).ok_or_else(|| anyhow!("{}: the game of this DLC is unknown", game.app_title))?;
            installed::get(dir, &main)
                .map(|g| g.install_path)
                .filter(|p| !p.is_empty())
                .ok_or_else(|| anyhow!(tr!("Install the game of {title} first", title = game.app_title.as_str())))?
        }
        _ => {
            if req.base_path.trim().is_empty() {
                bail!(tr!("Choose where to install the game"));
            }
            join_install_path(&req.base_path, &folder_name(&game))
        }
    };
    let root = PathBuf::from(&install_path);

    let mut fetched = info::fetch_manifest(&epic.client, session.access_token(), &game, &platform)?;
    let old_manifest = old.as_ref().and_then(|o| installed::installed_manifest(dir, o).ok());
    if let (Some(om), Some(base)) = (&old_manifest, fetched.info.base_urls.first()) {
        let delta = epic.client.delta_manifest(base, &om.meta.build_id(), &fetched.manifest.meta.build_id());
        if let Some(delta) = delta.and_then(|d| Manifest::parse(&d).ok()) {
            log::info!("epic: {} has a delta manifest from {}", req.app_name, om.meta.build_version);
            fetched.manifest.apply_delta(&delta);
        }
    }
    let manifest = &fetched.manifest;
    let tags: Option<Vec<String>> = match &old {
        Some(o) if update => Some(o.install_tags.clone()).filter(|t| !t.is_empty()),
        _ => req.install_tags.clone().filter(|t| !t.is_empty()),
    };
    let planned = plan::plan(
        manifest,
        old_manifest.as_ref(),
        &PlanOptions { install_tags: tags.as_deref(), only: None, root: Some(&root), reuse: true },
    );
    log::info!(
        "epic: {} {} -> {}: {} added, {} changed, {} unchanged, {} to download",
        req.app_name,
        old.as_ref().map_or("-", |o| o.version.as_str()),
        manifest.meta.build_version,
        planned.added,
        planned.changed,
        planned.unchanged,
        planned.download_size
    );
    let (dplan, paths) = depot_plan(&planned, manifest);
    if !req.ignore_free_space {
        check_space(&root, &dplan)?;
    }
    let mut source = cdn(
        epic,
        Some(&session),
        &req.app_name,
        &platform,
        &fetched.info.base_urls,
        fetched.info.use_signed_url,
        &fetched.info.secrets,
        settings,
    )?;
    source.chunks = planned.chunks.clone();
    source.paths = paths;
    let outcome = depot::install(&root, &dplan, &source, &depot_options(dir, &req.app_name, settings), cancel, on_progress)?;
    if outcome == depot::Outcome::Cancelled {
        return Ok(outcome);
    }

    installed::save_manifest(dir, &req.app_name, &platform, manifest)?;
    let exe_override = legendary_config::cached_exe_override(dir, &req.app_name);
    let mut record = InstalledGame::from_install(&Install {
        game: &game,
        manifest,
        install_path: &install_path,
        platform: &platform,
        install_tags: tags.as_deref().unwrap_or_default(),
        install_size: planned.install_size,
        base_urls: &fetched.info.base_urls,
        is_dlc,
        egl_guid: old.as_ref().map_or("", |o| o.egl_guid.as_str()),
        is_preloaded: fetched.info.is_preloaded,
        exe_override: exe_override.as_deref(),
    });
    if let Some(o) = &old {
        // What the user set and what the cloud saves keep.
        record.save_path = o.save_path.clone();
        record.rest = o.rest.clone();
        // Prerequisites the game already ran stay run when they are the same.
        if o.prereq_info.as_ref().and_then(|p| p.get("ids")) == record.prereq_info.as_ref().and_then(|p| p.get("ids")) {
            record.prereq_info = o.prereq_info.clone().or(record.prereq_info);
        }
    }
    installed::set(dir, &record)?;
    let _ = std::fs::remove_file(state_file(dir, &req.app_name));
    Ok(depot::Outcome::Done)
}

/// Writes the EOS overlay's planned install (see
/// [`heronite_epic_api::eos::prepare_install`]). Blocking, network.
pub(crate) fn install_overlay(
    epic: &Epic,
    prepared: &heronite_epic_api::eos::OverlayInstall,
    settings: &DownloadSettings,
    cancel: &Cancel,
    on_progress: &mut dyn FnMut(&depot::Progress),
) -> Result<depot::Outcome> {
    let (dplan, paths) = depot_plan(&prepared.plan, &prepared.manifest);
    let mut source = cdn(epic, None, "", "Windows", &prepared.info.base_urls, false, &prepared.info.secrets, settings)?;
    source.chunks = prepared.plan.chunks.clone();
    source.paths = paths;
    let root = Path::new(&prepared.record.install_path);
    depot::install(
        root,
        &dplan,
        &source,
        &depot::Options { workers: settings.max_workers as usize, ..Default::default() },
        cancel,
        on_progress,
    )
}

/// Checks the files of an installed game against its manifest and fetches
/// the broken ones again, keeping its version (`legendary repair`).
/// Blocking, network when something is broken.
pub fn repair(
    epic: &Epic,
    app_name: &str,
    settings: &DownloadSettings,
    cancel: &Cancel,
    on_step: &mut dyn FnMut(RepairStep),
) -> Result<depot::Outcome> {
    let dir = &epic.config_dir;
    let mut game = installed::get(dir, app_name).ok_or_else(|| anyhow!(tr!("The game is not installed")))?;
    let session = epic.session().ok();
    let meta = GameMeta::load(dir, app_name);
    // The manifest of the installed build; a missing one is the current build's.
    let (manifest, info) = match installed::installed_manifest(dir, &game) {
        Ok(m) => (m, None),
        Err(err) => {
            log::warn!("epic: {err:#}; repairing against the current build");
            let (Some(s), Some(meta)) = (&session, &meta) else { return Err(err) };
            let f = info::fetch_manifest(&epic.client, s.access_token(), meta, &game.platform)?;
            (f.manifest, Some(f.info))
        }
    };
    let root = PathBuf::from(&game.install_path);
    let tags: Vec<String> = game.install_tags.clone();
    let wanted: BTreeSet<String> = manifest.installed_files(&tags).into_iter().collect();
    let expected: Vec<depot::Expected> = manifest
        .files
        .iter()
        .filter(|f| wanted.contains(&f.filename) && f.symlink_target.is_empty())
        .map(|f| depot::Expected { path: f.filename.clone(), size: f.size(), hash: depot::Hash::Sha1(f.hash) })
        .collect();
    let checked = depot::verify(&root, &expected, 0, cancel, &mut |done, total| {
        on_step(RepairStep::Verifying(if total == 0 { 100. } else { done as f64 * 100. / total as f64 }));
    })?;
    let Some(problems) = checked else { return Ok(depot::Outcome::Cancelled) };
    let broken: BTreeSet<String> = problems.into_iter().map(|(path, _)| path).collect();
    // Links are written again whenever they are missing.
    let mut broken = broken;
    for f in manifest.files.iter().filter(|f| wanted.contains(&f.filename) && !f.symlink_target.is_empty()) {
        if std::fs::symlink_metadata(root.join(&f.filename)).is_err() {
            broken.insert(f.filename.clone());
        }
    }
    log::info!("epic: {app_name}: {} of {} files to fetch again", broken.len(), expected.len());

    if !broken.is_empty() {
        let opts =
            PlanOptions { install_tags: (!tags.is_empty()).then_some(tags.as_slice()), only: Some(&broken), root: None, reuse: false };
        let planned = plan::plan(&manifest, None, &opts);
        let (dplan, paths) = depot_plan(&planned, &manifest);
        let (base_urls, signed, secrets) = match &info {
            Some(i) => (i.base_urls.clone(), i.use_signed_url, i.secrets.clone()),
            None => {
                // Fresh CDN addresses when signed in, else the ones of the install.
                let fresh = match (&session, &meta) {
                    (Some(s), Some(m)) => epic
                        .client
                        .manifest_info(s.access_token(), m.namespace(), m.catalog_item_id(), app_name, &game.platform, "Live")
                        .map_err(|e| log::warn!("epic: the CDN of {app_name}: {e:#}"))
                        .ok(),
                    _ => None,
                };
                match fresh {
                    Some(i) => (i.base_urls, i.use_signed_url, i.secrets),
                    None => (game.base_urls.clone(), false, Secrets::new()),
                }
            }
        };
        if base_urls.is_empty() {
            bail!(tr!("Log in to your Epic account first"));
        }
        let mut source = cdn(epic, session.as_ref(), app_name, &game.platform, &base_urls, signed, &secrets, settings)?;
        source.chunks = planned.chunks.clone();
        source.paths = paths;
        let opts = depot::Options { state_file: None, ..depot_options(dir, app_name, settings) };
        let outcome = depot::install(&root, &dplan, &source, &opts, cancel, &mut |p| on_step(RepairStep::Downloading(p.clone())))?;
        if outcome == depot::Outcome::Cancelled {
            return Ok(outcome);
        }
    }
    if info.is_some() {
        // Repaired against the current build: it is that build now.
        installed::save_manifest(dir, app_name, &game.platform, &manifest)?;
        game.version = manifest.meta.build_version.clone();
    }
    game.needs_verification = false;
    installed::set(dir, &game)?;
    Ok(depot::Outcome::Done)
}

/// Deletes an installed game's files (those of its manifest and install
/// tags), then the folders they leave empty, and `install_path` itself when
/// `remove_root` and empty: files the game made (saves, settings) stay, as
/// with legendary. Without a stored manifest nothing is deleted (`Err`).
pub fn delete_files(config_dir: &Path, game: &InstalledGame, remove_root: bool) -> Result<()> {
    let manifest = installed::installed_manifest(config_dir, game)?;
    let root = Path::new(&game.install_path);
    // macOS keeps games in /Applications: never remove that folder.
    let remove_root = remove_root && !(game.platform == "Mac" && game.install_path.ends_with("/Applications"));
    let mut dirs: BTreeSet<PathBuf> = BTreeSet::new();
    let mut failed = 0usize;
    for name in manifest.installed_files(&game.install_tags) {
        if depot::check_path(&name).is_err() {
            continue;
        }
        let path = root.join(&name);
        let mut parent = Path::new(&name).parent();
        while let Some(p) = parent.filter(|p| !p.as_os_str().is_empty()) {
            dirs.insert(p.to_path_buf());
            parent = p.parent();
        }
        match std::fs::remove_file(&path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => {
                log::warn!("epic: deleting {}: {e}", path.display());
                failed += 1;
            }
        }
    }
    // The deepest folders first; one with other files in it stays.
    let mut dirs: Vec<PathBuf> = dirs.into_iter().collect();
    dirs.sort_by_key(|d| std::cmp::Reverse(d.components().count()));
    for d in dirs {
        let _ = std::fs::remove_dir(root.join(d));
    }
    if remove_root {
        let _ = std::fs::remove_dir(root);
    }
    if failed > 0 {
        bail!(tr!("Some files of the game could not be deleted from {path}", path = root.display()));
    }
    Ok(())
}

#[cfg(test)]
pub(crate) mod testing {
    //! A local Epic: the token, manifest and CDN services on one port,
    //! answering by path, with a build made of real chunks.

    use std::collections::HashMap;
    use std::io::{BufRead, BufReader, Read, Write};
    use std::sync::{Arc, Mutex};

    use heronite_epic_api::chunk;
    use heronite_epic_api::manifest::{ChunkPart, FileManifest, Guid, Manifest};
    use heronite_epic_api::{Client, Endpoints};
    use serde_json::json;

    use crate::client::Epic;

    /// Routes of the server: the path (without its query) to the answer.
    pub type Routes = Arc<Mutex<HashMap<String, (u16, Vec<u8>)>>>;

    /// A server answering `routes`, and the paths it was asked for.
    pub struct Server {
        pub url: String,
        pub routes: Routes,
        pub seen: Arc<Mutex<Vec<String>>>,
        /// Milliseconds each answer waits.
        pub delay_ms: Arc<std::sync::atomic::AtomicU64>,
    }

    impl Server {
        pub fn start() -> Self {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let url = format!("http://{}", listener.local_addr().unwrap());
            let routes: Routes = Arc::default();
            let seen: Arc<Mutex<Vec<String>>> = Arc::default();
            let delay_ms: Arc<std::sync::atomic::AtomicU64> = Arc::default();
            let (r, s, d) = (routes.clone(), seen.clone(), delay_ms.clone());
            std::thread::spawn(move || {
                for stream in listener.incoming().flatten() {
                    let (r, s, d) = (r.clone(), s.clone(), d.clone());
                    std::thread::spawn(move || {
                        let mut reader = BufReader::new(stream.try_clone().unwrap());
                        let mut first = String::new();
                        if reader.read_line(&mut first).is_err() {
                            return;
                        }
                        let mut len = 0usize;
                        loop {
                            let mut line = String::new();
                            if reader.read_line(&mut line).is_err() {
                                return;
                            }
                            if let Some(v) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                                len = v.trim().parse().unwrap_or(0);
                            }
                            if line == "\r\n" || line.is_empty() {
                                break;
                            }
                        }
                        let mut body = vec![0; len];
                        let _ = reader.read_exact(&mut body);
                        let path = first.split_whitespace().nth(1).unwrap_or("/").split('?').next().unwrap_or("/").to_string();
                        s.lock().unwrap().push(path.clone());
                        std::thread::sleep(std::time::Duration::from_millis(d.load(std::sync::atomic::Ordering::Relaxed)));
                        let (status, answer) = r.lock().unwrap().get(&path).cloned().unwrap_or((404, b"{}".to_vec()));
                        let mut out = stream;
                        let _ = write!(out, "HTTP/1.1 {status} X\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", answer.len());
                        let _ = out.write_all(&answer);
                    });
                }
            });
            Self { url, routes, seen, delay_ms }
        }

        pub fn route(&self, path: &str, status: u16, body: impl Into<Vec<u8>>) {
            self.routes.lock().unwrap().insert(path.to_string(), (status, body.into()));
        }

        pub fn asked(&self, prefix: &str) -> usize {
            self.seen.lock().unwrap().iter().filter(|p| p.starts_with(prefix)).count()
        }

        pub fn epic(&self, config_dir: &std::path::Path) -> Epic {
            Epic::with_client(config_dir, Client::new(Endpoints::all_at(&self.url)))
        }
    }

    /// A build: `files` (path, contents), each file in chunks of up to
    /// `chunk_size` bytes, served by `server`. Returns the manifest.
    pub fn serve_build(server: &Server, app: &str, version: &str, files: &[(&str, &[u8])]) -> Manifest {
        let mut m = Manifest::new(app, version);
        m.meta.launch_exe = files.first().map(|f| f.0.to_string()).unwrap_or_default();
        // Chunk ids differ between builds (the same id is the same data).
        let build: u32 = version.bytes().fold(7u32, |h, b| h.wrapping_mul(31).wrapping_add(b as u32));
        let mut n = 1u32;
        for (path, data) in files {
            let mut fm = FileManifest { filename: path.to_string(), ..Default::default() };
            fm.hash = sha1_smol::Sha1::from(*data).digest().bytes();
            for piece in data.chunks(4096) {
                let guid = Guid([n, build, 3, 4]);
                n += 1;
                let (bytes, info) = chunk::encode(guid, piece).unwrap();
                server.route(&format!("/cdn/{}", m.chunk_path(&info)), 200, bytes);
                fm.chunk_parts.push(ChunkPart { guid, offset: 0, size: piece.len() as u32 });
                m.chunks.push(info);
            }
            m.files.push(fm);
        }
        let bytes = m.to_bytes().unwrap();
        let hash = sha1_smol::Sha1::from(&bytes).digest().to_string();
        server.route("/cdn/build.manifest", 200, bytes);
        let answer = json!({"elements": [{"hash": hash, "buildVersion": version, "manifests": [{"uri": format!("{}/cdn/build.manifest", server.url)}]}]});
        server.route(
            &format!("/launcher/api/public/assets/v2/platform/Windows/namespace/ns/catalogItem/cat/app/{app}/label/Live"),
            200,
            answer.to_string(),
        );
        m
    }

    /// The EOS overlay's build (`version`), served anonymously.
    pub fn serve_overlay(server: &Server, version: &str) {
        let mut m = Manifest::new(heronite_epic_api::eos::APP_NAME, version);
        let files: [(&str, &[u8]); 2] = [("EOSOVH-Win64-Shipping.dll", b"dll"), ("EOSOverlayRenderer-Win64-Shipping.exe", b"MZ")];
        for (i, (path, data)) in files.iter().enumerate() {
            let guid = Guid([i as u32 + 1, 99, 3, 4]);
            let (bytes, info) = chunk::encode(guid, data).unwrap();
            server.route(&format!("/cdn/{}", m.chunk_path(&info)), 200, bytes);
            let mut fm = FileManifest { filename: path.to_string(), ..Default::default() };
            fm.hash = sha1_smol::Sha1::from(*data).digest().bytes();
            fm.chunk_parts.push(ChunkPart { guid, offset: 0, size: data.len() as u32 });
            m.chunks.push(info);
            m.files.push(fm);
        }
        let bytes = m.to_bytes().unwrap();
        let hash = sha1_smol::Sha1::from(&bytes).digest().to_string();
        server.route("/cdn/overlay.manifest", 200, bytes);
        server.route(
            "/account/api/oauth/token",
            200,
            json!({"access_token": "anon", "expires_at": "2099-01-01T00:00:00.000Z"}).to_string(),
        );
        let answer = json!({"elements": [{"hash": hash, "buildVersion": version, "manifests": [{"uri": format!("{}/cdn/overlay.manifest", server.url)}]}]});
        // The overlay's namespace and catalog item, as Epic publishes them.
        let path = format!(
            "/launcher/api/public/assets/v2/platform/Windows/namespace/302e5ede476149b1bc3e4fe6ae45e50e/catalogItem/cc15684f44d849e89e9bf4cec0508b68/app/{}/label/Live",
            heronite_epic_api::eos::APP_NAME
        );
        server.route(&path, 200, answer.to_string());
    }

    /// legendary's files for a signed-in account that owns `app`.
    pub fn sign_in_and_own(config_dir: &std::path::Path, app: &str, title: &str) {
        std::fs::create_dir_all(config_dir.join("metadata")).unwrap();
        std::fs::write(
            config_dir.join("user.json"),
            json!({"access_token": "tok", "refresh_token": "ref", "account_id": "acc", "displayName": "Player",
                   "expires_at": "2099-01-01T00:00:00.000Z"})
            .to_string(),
        )
        .unwrap();
        std::fs::write(
            config_dir.join("metadata").join(format!("{app}.json")),
            json!({"app_name": app, "app_title": title, "asset_infos": {"Windows": {"app_name": app, "build_version": "2", "namespace": "ns", "catalog_item_id": "cat"}},
                   "base_urls": [], "metadata": {"id": "cat", "namespace": "ns", "title": title,
                   "customAttributes": {"FolderName": {"type": "STRING", "value": title}}, "releaseInfo": [{"appId": app, "platform": ["Windows"]}]}})
            .to_string(),
        )
        .unwrap();
    }
}

#[cfg(test)]
mod tests {
    use super::testing::*;
    use super::*;

    fn read(root: &Path, name: &str) -> Vec<u8> {
        std::fs::read(root.join(name)).unwrap()
    }

    #[test]
    fn an_apostrophe_in_the_install_folder_is_kept() {
        let base = |path: &str| join_install_path(path, "Game");
        let expected = |path: &str| Path::new(path).join("Game").to_string_lossy().into_owned();
        // The folder is what the user picked, apostrophes included.
        assert_eq!(base("/mnt/Rafa's SSD/Games"), expected("/mnt/Rafa's SSD/Games"));
        // The quotes wrapping a path (old configs) are not part of it.
        assert_eq!(base("'/g/Control'"), expected("/g/Control"));
        assert_eq!(base("'/mnt/Rafa's SSD'"), expected("/mnt/Rafa's SSD"));
    }

    #[test]
    fn a_game_installs_updates_and_repairs_like_legendary_records_it() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = dir.path().join("cfg");
        let server = Server::start();
        let epic = server.epic(&cfg);
        sign_in_and_own(&cfg, "Calluna", "Control");
        let big: Vec<u8> = (0..10_000u32).map(|i| (i % 251) as u8).collect();
        serve_build(&server, "Calluna", "1", &[("Control.exe", b"MZ v1"), ("data/big.bin", &big)]);

        let req = Request {
            app_name: "Calluna".into(),
            platform: "Windows".into(),
            base_path: dir.path().join("games").to_string_lossy().into(),
            ..Default::default()
        };
        let mut reports = 0;
        let out = install(&epic, &req, false, &DownloadSettings::default(), &Cancel::new(), &mut |_| reports += 1).unwrap();
        assert_eq!(out, depot::Outcome::Done);
        assert!(reports > 0);
        let root = dir.path().join("games").join("Control");
        assert_eq!(read(&root, "Control.exe"), b"MZ v1");
        assert_eq!(read(&root, "data/big.bin"), big);
        let rec = installed::get(&cfg, "Calluna").unwrap();
        assert_eq!(rec.install_path, root.to_string_lossy());
        assert_eq!((rec.version.as_str(), rec.executable.as_str(), rec.title.as_str()), ("1", "Control.exe", "Control"));
        assert_eq!(rec.install_size, 5 + 10_000);
        assert_eq!(rec.base_urls, [format!("{}/cdn", server.url)]);
        assert!(installed::load_manifest(&cfg, "Calluna", "1", "Windows").is_some());
        assert!(!state_file(&cfg, "Calluna").exists());

        // The user set a save folder; an update keeps it and fetches only
        // what changed.
        let mut rec = rec;
        rec.save_path = Some("/saves".into());
        installed::set(&cfg, &rec).unwrap();
        let chunks_before = server.asked("/cdn/Chunks");
        serve_build(&server, "Calluna", "2", &[("Control.exe", b"MZ v2"), ("data/big.bin", &big)]);
        assert_eq!(install(&epic, &req, true, &DownloadSettings::default(), &Cancel::new(), &mut |_| {}).unwrap(), depot::Outcome::Done);
        assert_eq!(read(&root, "Control.exe"), b"MZ v2");
        let rec = installed::get(&cfg, "Calluna").unwrap();
        assert_eq!(rec.version, "2");
        assert_eq!(rec.save_path.as_deref(), Some("/saves"));
        // Only the program's chunk was fetched again (the big file did not change).
        assert_eq!(server.asked("/cdn/Chunks") - chunks_before, 1);

        // A damaged file is found and fetched again.
        std::fs::write(root.join("data/big.bin"), b"broken").unwrap();
        let mut steps = Vec::new();
        assert_eq!(
            repair(&epic, "Calluna", &DownloadSettings::default(), &Cancel::new(), &mut |s| steps.push(s)).unwrap(),
            depot::Outcome::Done
        );
        assert_eq!(read(&root, "data/big.bin"), big);
        assert!(steps.iter().any(|s| matches!(s, RepairStep::Verifying(p) if *p == 100.)));
        assert!(steps.iter().any(|s| matches!(s, RepairStep::Downloading(_))));

        // Uninstalling leaves what the game made itself.
        std::fs::write(root.join("data/user.cfg"), b"mine").unwrap();
        delete_files(&cfg, &installed::get(&cfg, "Calluna").unwrap(), true).unwrap();
        assert!(!root.join("Control.exe").exists() && !root.join("data/big.bin").exists());
        assert!(root.join("data/user.cfg").exists());
        std::fs::remove_file(root.join("data/user.cfg")).unwrap();
        delete_files(&cfg, &installed::get(&cfg, "Calluna").unwrap(), true).unwrap();
        assert!(!root.exists(), "the empty folder goes too");
    }

    #[test]
    fn installs_need_room_and_a_signed_in_owner() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = dir.path().join("cfg");
        let server = Server::start();
        let epic = server.epic(&cfg);
        let req = Request {
            app_name: "Calluna".into(),
            platform: "Windows".into(),
            base_path: dir.path().to_string_lossy().into(),
            ..Default::default()
        };
        let err = install(&epic, &req, false, &DownloadSettings::default(), &Cancel::new(), &mut |_| {}).unwrap_err();
        assert!(err.to_string().contains("Log in"), "{err}");
        sign_in_and_own(&cfg, "Calluna", "Control");
        let err = install(
            &epic,
            &Request { app_name: "Other".into(), ..req.clone() },
            false,
            &DownloadSettings::default(),
            &Cancel::new(),
            &mut |_| {},
        )
        .unwrap_err();
        assert!(err.to_string().contains("not in the Epic library"), "{err}");
        let err = install(&epic, &req, true, &DownloadSettings::default(), &Cancel::new(), &mut |_| {}).unwrap_err();
        assert!(err.to_string().contains("not installed"), "{err}");
    }

    #[test]
    fn a_cancelled_install_goes_on_where_it_stopped() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = dir.path().join("cfg");
        let server = Server::start();
        let epic = server.epic(&cfg);
        sign_in_and_own(&cfg, "Calluna", "Control");
        let files: Vec<(String, Vec<u8>)> = (0..20).map(|i| (format!("f{i:02}.bin"), vec![i as u8; 9000])).collect();
        let refs: Vec<(&str, &[u8])> = files.iter().map(|(n, d)| (n.as_str(), d.as_slice())).collect();
        serve_build(&server, "Calluna", "1", &refs);
        server.delay_ms.store(30, std::sync::atomic::Ordering::Relaxed);
        let req = Request {
            app_name: "Calluna".into(),
            platform: "Windows".into(),
            base_path: dir.path().join("g").to_string_lossy().into(),
            ..Default::default()
        };
        let cancel = Cancel::new();
        let settings = DownloadSettings { max_workers: 1, no_https: false };
        let out = install(&epic, &req, false, &settings, &cancel, &mut |p| {
            if p.files_done > 3 {
                cancel.request();
            }
        })
        .unwrap();
        assert_eq!(out, depot::Outcome::Cancelled);
        assert!(installed::get(&cfg, "Calluna").is_none(), "not installed until it is complete");
        server.delay_ms.store(0, std::sync::atomic::Ordering::Relaxed);
        let mut resumed = 0;
        let out = install(&epic, &req, false, &settings, &Cancel::new(), &mut |p| resumed = p.resumed).unwrap();
        assert_eq!(out, depot::Outcome::Done);
        assert!(resumed > 0, "the files of the first run are kept");
        assert!(installed::get(&cfg, "Calluna").is_some());
    }
}
