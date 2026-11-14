//! Installs, updates and repairs of GOG games: the work gog-api plans
//! ([`heronite_gog_api::plan`]) written to disk, then gogdl's records of the
//! install (the stored build, or `.gogdl-linux-manifest` in a Linux game's
//! folder), so the game is the same as one gogdl installed.
//!
//! Builds of the content system (Windows and macOS) are written by
//! [`heronite_depot`]: v2 chunks from GOG's CDN, v1 `main.bin` ranges in
//! pieces of at most [`RANGE_PIECE`] bytes checked by the whole file's md5,
//! the game's folder and its support folder each a plan of their own. A
//! Linux installer is a zip whose entries are one deflate stream each, so
//! they are streamed out of it ([`linux::copy_entry`]) instead.
//!
//! A run that stops (cancelled, the app closed, the connection lost) leaves
//! a resume journal in `gogdlConfig/heronite-resume`: the next run of the
//! same build keeps the files already written.

use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, Context, Result};
use heronite_data::proc::Cancel;
use heronite_data::DataPaths;
use heronite_depot as depot;
use heronite_epic::disk;
use heronite_gog_api::chunk::SecureLink;
use heronite_gog_api::files::{Checksum, Diff, FileEntry, Root, Source};
use heronite_gog_api::plan::{self, Mode, Prepared};
use heronite_gog_api::{linux, redist, stored, ChunkRef, Client};
use heronite_i18n::tr;

use crate::user;

/// Largest piece of a v1 `main.bin` fetched at once.
pub const RANGE_PIECE: u64 = 10 * 1024 * 1024;
/// Linux installer entries streamed at once.
const LINUX_WORKERS: usize = 4;

/// What to install, update or repair.
#[derive(Debug, Clone, Default)]
pub struct Request {
    pub app_name: String,
    /// `windows`, `osx` or `linux`.
    pub os: String,
    /// A language code (`en-US`).
    pub language: String,
    /// DLC ids to take (those the account owns); `None` leaves DLCs out.
    pub dlcs: Option<Vec<String>>,
    /// A new install's folder goes in here (the build names the folder).
    pub base_path: String,
    /// The installed game's folder (updates and repairs).
    pub game_dir: Option<PathBuf>,
    /// The build to repair against (the installed one).
    pub build_id: Option<String>,
    pub max_workers: u32,
    /// Skip the free space check.
    pub ignore_free_space: bool,
}

/// What an install or update ended with, for the installed list.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Installed {
    pub game_dir: PathBuf,
    /// The folder below the base path.
    pub folder_name: String,
    pub build_id: String,
    pub version_name: String,
    pub version_etag: String,
    /// The DLC ids taken.
    pub dlcs: Vec<String>,
}

/// How a run ended.
#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    Done(Installed),
    Cancelled,
}

/// What a repair reports.
#[derive(Debug, Clone, PartialEq)]
pub enum Step {
    /// Percent of the files checked.
    Verifying(f64),
    Downloading(depot::Progress),
}

/// The client of the account, signed in (a sign-in GOG refuses is the
/// app's `SessionExpired`).
pub fn signed_in_client(paths: &DataPaths) -> Result<Client> {
    let creds = user::credentials(paths)?;
    Ok(Client::with_token(&creds.access_token))
}

/// Installs (`Mode::Install`), updates or repairs `req`. Blocking, network.
pub fn run(paths: &DataPaths, req: &Request, mode: Mode, cancel: &Cancel, on_step: &mut dyn FnMut(Step)) -> Result<Outcome> {
    let client = signed_in_client(paths)?;
    let config_dir = paths.gogdl_config_dir();
    let game_dir_arg = req.game_dir.as_deref();
    let prepared = plan::prepare(
        &client,
        &config_dir,
        &plan::Request {
            game_id: &req.app_name,
            os: &req.os,
            language: &req.language,
            dlcs: req.dlcs.as_deref(),
            branch: None,
            build_id: req.build_id.as_deref().filter(|b| !b.is_empty() && mode == Mode::Repair),
            password: None,
            game_dir: game_dir_arg,
        },
        mode,
    )
    .map_err(|err| {
        if err.downcast_ref::<plan::NoRecord>().is_some() {
            anyhow!(tr!("There is no record of this game's files, so it cannot be repaired"))
        } else {
            err
        }
    })?;
    let game_dir = match game_dir_arg {
        Some(dir) => dir.to_path_buf(),
        None => {
            if req.base_path.trim().is_empty() {
                bail!(tr!("Choose where to install the game"));
            }
            Path::new(req.base_path.trim()).join(folder_of(&prepared.folder_name, &req.app_name))
        }
    };
    let support_dir = stored::support_dir(&config_dir, &req.app_name);
    let mut diff = prepared.diff.clone();
    if mode == Mode::Repair {
        match broken(&diff, &game_dir, &support_dir, cancel, &mut |p| on_step(Step::Verifying(p)))? {
            Some(b) => diff = b,
            None => return Ok(Outcome::Cancelled),
        }
        log::info!("gog: repairing {}: {} file(s) to fetch again", req.app_name, diff.files.len() + diff.links.len());
    }
    log::info!(
        "gog: {:?} of {} ({}, build {}): {} file(s) to write, {} to delete",
        mode,
        req.app_name,
        req.os,
        prepared.build_id,
        diff.files.len(),
        diff.deleted.len()
    );
    let written = if prepared.generation == 0 {
        write_linux(&client, &prepared, &diff, &game_dir, paths, req, mode, cancel, &mut |p| on_step(Step::Downloading(p)))?
    } else {
        write_content(&client, &prepared, &diff, &game_dir, &support_dir, paths, req, cancel, &mut |p| on_step(Step::Downloading(p)))?
    };
    if written == depot::Outcome::Cancelled {
        return Ok(Outcome::Cancelled);
    }
    if prepared.generation == 0 {
        if mode != Mode::Repair {
            linux::write_record(&game_dir, &linux::record(&prepared.installers, &game_dir))?;
        }
    } else if let Some(doc) = prepared.stored.as_ref().filter(|_| mode != Mode::Repair) {
        stored::write(&config_dir, &req.app_name, doc)?;
    }
    for state in state_files(paths, &req.app_name) {
        let _ = std::fs::remove_file(state);
    }
    Ok(Outcome::Done(Installed {
        game_dir,
        folder_name: folder_of(&prepared.folder_name, &req.app_name).to_string(),
        build_id: prepared.build_id.clone(),
        version_name: prepared.version_name.clone(),
        version_etag: prepared.version_etag.clone().unwrap_or_default(),
        dlcs: prepared.dlcs.iter().map(|d| d.id.clone()).collect(),
    }))
}

/// The shared redistributables `ids` in `dir` (`gogdl redist`), with the
/// folder's record listing them once they are in place. Blocking, network.
pub fn install_redist(dir: &Path, ids: &[String], cancel: &Cancel) -> Result<depot::Outcome> {
    // The folder is shared by every game: two first launches at once would
    // write the same files and journal. One at a time, in this process (the
    // mutex) and across processes (the lock file).
    static IN_PROCESS: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _here = IN_PROCESS.lock().unwrap_or_else(|e| e.into_inner());
    std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    let lock_path = dir.join(".heronite-lock");
    let lock = std::fs::File::options()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&lock_path)
        .with_context(|| format!("opening {}", lock_path.display()))?;
    lock.lock().with_context(|| format!("locking {}", lock_path.display()))?;
    let client = Client::anonymous();
    let repo = client.repository()?;
    let (diff, installed) = redist::shared_plan(&client, &repo, dir, ids)?;
    let mut links = HashMap::new();
    if diff.files.iter().any(|f| matches!(f.source, Source::Chunks { .. })) {
        links.insert(redist::PRODUCT.to_string(), client.dependency_link()?);
    }
    let (plan, pieces) = depot_plan(&diff, Root::Game);
    let source = Cdn { client: &client, links: Mutex::new(links), pieces, generation: 2 };
    std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    let options = depot::Options { workers: 0, max_memory: 0, state_file: Some(dir.join(".heronite-resume")) };
    let outcome = depot::install(dir, &plan, &source, &options, cancel, &mut |_| {})?;
    if outcome == depot::Outcome::Done {
        redist::write_installed(dir, &repo, &installed)?;
        let _ = std::fs::remove_file(dir.join(".heronite-resume"));
    }
    Ok(outcome)
}

/// The resume journals of a game's runs.
fn state_files(paths: &DataPaths, app_name: &str) -> [PathBuf; 3] {
    let dir = paths.gogdl_config_dir().join("heronite-resume");
    ["game", "support", "linux"].map(|part| dir.join(format!("{app_name}.{part}")))
}

/// Where a v2 chunk or a v1 range comes from.
#[derive(Debug, Clone)]
enum Piece {
    Chunk { product: String, chunk: ChunkRef },
    Range { product: String, offset: u64, size: u64 },
}

/// The CDN of a build. Chunk ids of the plans are indexes into `pieces`.
struct Cdn<'a> {
    client: &'a Client,
    links: Mutex<HashMap<String, SecureLink>>,
    pieces: Vec<Piece>,
    generation: u8,
}

impl Cdn<'_> {
    /// The product's secure link, asked for again when it is old (v2: its
    /// path is always `/`; redistributables have their own).
    fn link(&self, product: &str) -> Result<SecureLink> {
        let mut links = self.links.lock().unwrap_or_else(|e| e.into_inner());
        let link = links.get(product).ok_or_else(|| anyhow!("no download address for product {product}"))?;
        if link.is_stale() && self.generation == 2 {
            let fresh =
                if product == redist::PRODUCT { self.client.dependency_link()? } else { self.client.secure_link(product, "/", 2, None)? };
            links.insert(product.to_string(), fresh);
        }
        Ok(links[product].clone())
    }
}

impl depot::Source for Cdn<'_> {
    fn fetch(&self, chunk: &depot::Chunk, _cancel: &Cancel) -> Result<Vec<u8>> {
        let i: usize = chunk.id.parse().context("chunk id")?;
        match &self.pieces[i] {
            Piece::Chunk { product, chunk } => self.client.fetch_chunk(&self.link(product)?, chunk),
            Piece::Range { product, offset, size } => self.client.fetch_range(&self.link(product)?, *offset, *size),
        }
    }
}

fn hash_of(checksum: &Option<Checksum>) -> Option<depot::Hash> {
    match checksum.as_ref()? {
        Checksum::Md5(h) => hex::decode(h).ok()?.try_into().ok().map(depot::Hash::Md5),
        Checksum::Sha256(h) => hex::decode(h).ok()?.try_into().ok().map(depot::Hash::Sha256),
        Checksum::Crc32(_) => None,
    }
}

/// The depot plan of `diff`'s files below `root` (symlinks and folders go
/// with the game's folder) and the pieces its chunk ids point at.
fn depot_plan(diff: &Diff, root: Root) -> (depot::Plan, Vec<Piece>) {
    let mut pieces: Vec<Piece> = Vec::new();
    let mut chunks: Vec<depot::Chunk> = Vec::new();
    let mut by_md5: HashMap<String, usize> = HashMap::new();
    let mut files = Vec::new();
    for f in diff.files.iter().filter(|f| f.root == root) {
        let mut parts = Vec::new();
        match &f.source {
            Source::Chunks { product, parts: list } => {
                for p in list {
                    if let Some(old) = p.old_offset {
                        parts.push(depot::Part::Local { path: f.path.clone(), offset: old, size: p.size });
                        continue;
                    }
                    let index = *by_md5.entry(p.chunk.compressed_md5.clone()).or_insert_with(|| {
                        pieces.push(Piece::Chunk { product: product.clone(), chunk: p.chunk.clone() });
                        chunks.push(depot::Chunk {
                            id: (pieces.len() - 1).to_string(),
                            download_size: p.chunk.compressed_size,
                            size: p.chunk.size,
                        });
                        pieces.len() - 1
                    });
                    parts.push(depot::Part::Chunk { chunk: index, offset: p.offset, size: p.size });
                }
            }
            Source::Range { product, offset } => {
                let mut at = 0;
                while at < f.size {
                    let size = RANGE_PIECE.min(f.size - at);
                    pieces.push(Piece::Range { product: product.clone(), offset: offset + at, size });
                    chunks.push(depot::Chunk { id: (pieces.len() - 1).to_string(), download_size: size, size });
                    parts.push(depot::Part::Chunk { chunk: pieces.len() - 1, offset: 0, size });
                    at += size;
                }
            }
            Source::Empty => {}
            Source::Zip { .. } => {
                log::warn!("gog: {} comes from an installer, not the content system; skipped", f.path);
                continue;
            }
        }
        files.push(depot::FileTask {
            path: f.path.clone(),
            size: f.size,
            parts,
            hash: hash_of(&f.checksum),
            executable: f.executable,
            symlink: None,
        });
    }
    let mut dirs = Vec::new();
    if root == Root::Game {
        for l in &diff.links {
            files.push(depot::FileTask {
                path: l.path.clone(),
                size: 0,
                parts: Vec::new(),
                hash: None,
                executable: false,
                symlink: Some(l.target.clone()),
            });
        }
        dirs = diff.dirs.clone();
    }
    let remove = diff.deleted.iter().filter(|(r, _)| *r == root).map(|(_, p)| p.clone()).collect();
    (depot::Plan { chunks, files, remove, dirs }, pieces)
}

/// Fails when `root`'s disk does not have `needed` bytes free.
fn check_space(root: &Path, needed: u64) -> Result<()> {
    // The folder may not exist yet: its first existing parent decides.
    let mut at = root;
    while !at.exists() {
        match at.parent() {
            Some(p) => at = p,
            None => break,
        }
    }
    let Some(space) = disk::check(&at.to_string_lossy()) else { return Ok(()) };
    if needed > space.free {
        bail!(tr!("Not enough disk space"));
    }
    if !space.writable {
        bail!(tr!("No write access to {path}", path = at.display()));
    }
    Ok(())
}

/// Adds up the reports of plans written one after the other, so the queue
/// shows one download.
#[derive(Default)]
struct Totals {
    download: u64,
    write: u64,
    files: usize,
    /// What the plans before the running one did.
    done: depot::Progress,
}

impl Totals {
    fn add(&mut self, plan: &depot::Plan) {
        self.download += plan.download_size();
        self.write += plan.write_size();
        self.files += plan.files.len();
    }

    fn report(&self, p: &depot::Progress) -> depot::Progress {
        let downloaded = self.done.downloaded + p.downloaded;
        let download_total = self.download.max(downloaded);
        let remaining = download_total.saturating_sub(downloaded);
        let eta = if p.down_speed > 0. { Some(Duration::from_secs_f64(remaining as f64 / p.down_speed)) } else { p.eta };
        depot::Progress {
            downloaded,
            download_total,
            written: self.done.written + p.written,
            // A resumed plan writes less than it holds.
            write_total: self.write.saturating_sub(self.done.resumed + p.resumed),
            resumed: self.done.resumed + p.resumed,
            files_done: self.done.files_done + p.files_done,
            files_total: self.files,
            down_speed: p.down_speed,
            disk_speed: p.disk_speed,
            eta,
        }
    }

    fn finish(&mut self, last: &depot::Progress) {
        self.done = depot::Progress { down_speed: 0., disk_speed: 0., eta: None, ..last.clone() };
    }
}

/// Writes a content-system build: the support folder's files, then the
/// game's.
#[allow(clippy::too_many_arguments)]
fn write_content(
    client: &Client,
    prepared: &Prepared,
    diff: &Diff,
    game_dir: &Path,
    support_dir: &Path,
    paths: &DataPaths,
    req: &Request,
    cancel: &Cancel,
    on_progress: &mut dyn FnMut(depot::Progress),
) -> Result<depot::Outcome> {
    let (game, game_pieces) = depot_plan(diff, Root::Game);
    let (support, support_pieces) = depot_plan(diff, Root::Support);
    if !req.ignore_free_space {
        check_space(game_dir, depot::space_needed(game_dir, &game) + depot::space_needed(support_dir, &support))?;
    }
    let mut totals = Totals::default();
    totals.add(&support);
    totals.add(&game);
    let [game_state, support_state, _] = state_files(paths, &req.app_name);
    if let Some(dir) = game_state.parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    }
    for (root, plan, pieces, state) in [(support_dir, support, support_pieces, support_state), (game_dir, game, game_pieces, game_state)] {
        if plan.files.is_empty() && plan.remove.is_empty() && plan.dirs.is_empty() {
            continue;
        }
        std::fs::create_dir_all(root).with_context(|| format!("creating {}", root.display()))?;
        let source = Cdn { client, links: Mutex::new(prepared.links.clone()), pieces, generation: prepared.generation };
        let options = depot::Options { workers: req.max_workers as usize, max_memory: 0, state_file: Some(state) };
        let mut last = depot::Progress::default();
        let outcome = depot::install(root, &plan, &source, &options, cancel, &mut |p| {
            last = p.clone();
            on_progress(totals.report(p));
        })?;
        if outcome == depot::Outcome::Cancelled {
            return Ok(outcome);
        }
        totals.finish(&totals.report(&last));
    }
    Ok(depot::Outcome::Done)
}

/// The resume journal of a Linux install: a line naming the installers,
/// then one finished file per line.
struct Journal {
    path: PathBuf,
    done: HashSet<String>,
    file: Mutex<Option<std::fs::File>>,
}

impl Journal {
    fn open(path: PathBuf, fingerprint: &str) -> Self {
        let text = std::fs::read_to_string(&path).unwrap_or_default();
        let mut lines = text.lines();
        let done = if lines.next() == Some(fingerprint) { lines.map(str::to_string).collect() } else { HashSet::new() };
        let file = (|| -> std::io::Result<std::fs::File> {
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir)?;
            }
            let mut f = std::fs::File::create(&path)?;
            writeln!(f, "{fingerprint}")?;
            for d in &done {
                writeln!(f, "{d}")?;
            }
            Ok(f)
        })();
        if let Err(err) = &file {
            log::warn!("gog: the resume journal {}: {err}", path.display());
        }
        Self { path, done, file: Mutex::new(file.ok()) }
    }

    fn record(&self, name: &str) {
        if let Some(f) = self.file.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
            let _ = writeln!(f, "{name}").and_then(|_| f.sync_data());
        }
    }
}

/// A writer that fails once the run is cancelled, so a stream stops.
struct Cancellable<'a, W> {
    inner: W,
    cancel: &'a Cancel,
}

impl<W: Write> Write for Cancellable<'_, W> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        if self.cancel.is_requested() {
            return Err(std::io::Error::new(std::io::ErrorKind::Interrupted, "cancelled"));
        }
        self.inner.write(buf)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

/// The game's folder name GOG gives (`installDirectory`) when it is a plain
/// folder name, else the game's id.
fn folder_of<'a>(name: &'a str, game_id: &'a str) -> &'a str {
    depot::folder_name(name).unwrap_or(game_id)
}

/// Where a game file of a Linux build lives in the game's folder, checked:
/// a plain relative path whose folders are not links (a link of the build,
/// made by an earlier install, could lead outside the game's folder).
fn target(game_dir: &Path, rel: &str) -> Result<PathBuf> {
    depot::check_path(rel)?;
    let mut at = game_dir.to_path_buf();
    let parts: Vec<&str> = rel.split('/').collect();
    for part in &parts[..parts.len().saturating_sub(1)] {
        at.push(part);
        if std::fs::symlink_metadata(&at).is_ok_and(|m| m.file_type().is_symlink()) {
            bail!("{rel} goes through the link {}", at.display());
        }
    }
    Ok(game_dir.join(rel))
}

/// Whether a link at `rel` pointing to `link_target` (relative to the link's
/// folder) stays inside the game's folder.
#[cfg(unix)]
fn link_stays_inside(rel: &str, link_target: &str) -> bool {
    if link_target.is_empty() || link_target.starts_with('/') || link_target.contains('\0') {
        return false;
    }
    let mut depth: Vec<&str> = rel.split('/').collect();
    depth.pop();
    for part in link_target.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                if depth.pop().is_none() {
                    return false;
                }
            }
            p => depth.push(p),
        }
    }
    true
}

/// Streams the files of a Linux installer into the game's folder.
#[allow(clippy::too_many_arguments)]
fn write_linux(
    client: &Client,
    prepared: &Prepared,
    diff: &Diff,
    game_dir: &Path,
    paths: &DataPaths,
    req: &Request,
    mode: Mode,
    cancel: &Cancel,
    on_progress: &mut dyn FnMut(depot::Progress),
) -> Result<depot::Outcome> {
    // The entry of each file, found by its product and place.
    let mut entries: HashMap<(&str, String), (&linux::Installer, &linux::Entry)> = HashMap::new();
    for inst in &prepared.installers {
        for e in inst.data_entries() {
            if let Some(f) = inst.file_of(e) {
                entries.insert((inst.product.as_str(), f.path), (inst, e));
            }
        }
    }
    // The build, the folder and the kind of run: a repair never takes what a
    // cancelled install or update of the same build left.
    let fingerprint = prepared.installers.iter().map(|i| format!("{}:{}", i.product, i.entries.len())).collect::<Vec<_>>().join(",")
        + &format!(";{};{mode:?};{}", prepared.version_name, game_dir.display());
    let journal = Journal::open(state_files(paths, &req.app_name)[2].clone(), &fingerprint);
    let files: Vec<&FileEntry> = diff.files.iter().filter(|f| f.root == Root::Game).collect();
    let kept = |f: &FileEntry| {
        journal.done.contains(&f.path) && std::fs::symlink_metadata(game_dir.join(&f.path)).is_ok_and(|m| m.is_file() && m.len() == f.size)
    };
    let todo: Vec<&FileEntry> = files.iter().copied().filter(|f| !kept(f)).collect();
    let resumed: u64 = files.iter().filter(|f| !todo.iter().any(|t| t.path == f.path)).map(|f| f.size).sum();
    if !req.ignore_free_space {
        let replaced: u64 = todo.iter().filter_map(|f| std::fs::metadata(game_dir.join(&f.path)).ok()).map(|m| m.len()).sum();
        check_space(game_dir, todo.iter().map(|f| f.size).sum::<u64>().saturating_sub(replaced))?;
    }
    std::fs::create_dir_all(game_dir).with_context(|| format!("creating {}", game_dir.display()))?;
    for d in &diff.dirs {
        std::fs::create_dir_all(target(game_dir, d)?)?;
    }

    let download_total: u64 = todo.iter().map(|f| f.download_size()).sum();
    let write_total: u64 = todo.iter().map(|f| f.size).sum();
    let downloaded = AtomicU64::new(0);
    let written = AtomicU64::new(0);
    let files_done = AtomicUsize::new(files.len() - todo.len());
    let failed = AtomicBool::new(false);
    let error: Mutex<Option<anyhow::Error>> = Mutex::new(None);
    let next = AtomicUsize::new(0);
    let started = Instant::now();
    let report = |down_speed: f64, disk_speed: f64| {
        let w = written.load(Ordering::Relaxed);
        let d = downloaded.load(Ordering::Relaxed);
        let eta = (down_speed > 0.).then(|| Duration::from_secs_f64(download_total.saturating_sub(d) as f64 / down_speed));
        depot::Progress {
            downloaded: d,
            download_total,
            written: w,
            write_total,
            resumed,
            files_done: files_done.load(Ordering::Relaxed),
            files_total: files.len(),
            down_speed,
            disk_speed,
            eta,
        }
    };

    let work = || {
        while !failed.load(Ordering::Relaxed) && !cancel.is_requested() {
            let i = next.fetch_add(1, Ordering::Relaxed);
            let Some(f) = todo.get(i) else { break };
            let result = (|| -> Result<()> {
                let dest = target(game_dir, &f.path)?;
                if let Some(dir) = dest.parent() {
                    std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
                }
                let part = dest.with_file_name(format!("{}.heronite-part", dest.file_name().unwrap_or_default().to_string_lossy()));
                // A stale part file (or a link in its place) goes first: the
                // new one is created, never opened through what is there.
                if std::fs::symlink_metadata(&part).is_ok() {
                    std::fs::remove_file(&part).with_context(|| format!("removing {}", part.display()))?;
                }
                let out = std::fs::File::options()
                    .write(true)
                    .create_new(true)
                    .open(&part)
                    .with_context(|| format!("creating {}", part.display()))?;
                match &f.source {
                    // An empty file has no data in the installer.
                    Source::Empty => drop(out),
                    Source::Zip { product, .. } => {
                        let (inst, entry) = entries
                            .get(&(product.as_str(), f.path.clone()))
                            .ok_or_else(|| anyhow!("{} is not in the installer", f.path))?;
                        let mut out = Cancellable { inner: std::io::BufWriter::with_capacity(1 << 20, out), cancel };
                        let copied = linux::copy_entry(client, inst, entry, &mut out, &mut |n| {
                            downloaded.fetch_add(n, Ordering::Relaxed);
                        });
                        let flushed = out.flush();
                        drop(out);
                        if let Err(err) = copied.and_then(|n| flushed.map(|_| n).map_err(Into::into)) {
                            let _ = std::fs::remove_file(&part);
                            return Err(err);
                        }
                    }
                    _ => {
                        let _ = std::fs::remove_file(&part);
                        bail!("{} is not in the installer", f.path);
                    }
                }
                // On disk before the journal says it is done.
                if let Ok(file) = std::fs::File::open(&part) {
                    file.sync_data().with_context(|| format!("writing {}", part.display()))?;
                }
                if f.executable {
                    heronite_platform::fs::make_executable(&part)?;
                }
                heronite_platform::fs::rename_replacing(&part, &dest).with_context(|| format!("writing {}", dest.display()))?;
                written.fetch_add(f.size, Ordering::Relaxed);
                files_done.fetch_add(1, Ordering::Relaxed);
                journal.record(&f.path);
                Ok(())
            })();
            if let Err(err) = result {
                if !cancel.is_requested() {
                    failed.store(true, Ordering::Relaxed);
                    error.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert(err);
                }
                break;
            }
        }
    };
    std::thread::scope(|s| {
        let handles: Vec<_> = (0..LINUX_WORKERS.min(todo.len().max(1))).map(|_| s.spawn(work)).collect();
        let mut down_meter = (Instant::now(), 0u64);
        let mut disk_meter = (Instant::now(), 0u64);
        while handles.iter().any(|h| !h.is_finished()) {
            std::thread::sleep(Duration::from_millis(250));
            let now = Instant::now();
            let rate = |m: &mut (Instant, u64), value: u64| {
                let secs = now.duration_since(m.0).as_secs_f64();
                let r = if secs > 0. { value.saturating_sub(m.1) as f64 / secs } else { 0. };
                if secs > 3. {
                    *m = (now, value);
                }
                r
            };
            let down = rate(&mut down_meter, downloaded.load(Ordering::Relaxed));
            let disk = rate(&mut disk_meter, written.load(Ordering::Relaxed));
            on_progress(report(down, disk));
        }
    });
    if let Some(err) = error.into_inner().unwrap_or_else(|e| e.into_inner()) {
        if is_disk_full(&err) {
            bail!(tr!("Not enough disk space"));
        }
        return Err(err);
    }
    if cancel.is_requested() {
        return Ok(depot::Outcome::Cancelled);
    }
    on_progress(report(write_total as f64 / started.elapsed().as_secs_f64().max(0.001), 0.));

    for link in &diff.links {
        make_link(game_dir, &link.path, &link.target)?;
    }
    for (_, rel) in diff.deleted.iter().filter(|(r, _)| *r == Root::Game) {
        if let Ok(path) = target(game_dir, rel) {
            let _ = std::fs::remove_file(path);
        }
    }
    drop(journal.file);
    let _ = std::fs::remove_file(&journal.path);
    Ok(depot::Outcome::Done)
}

fn is_disk_full(err: &anyhow::Error) -> bool {
    err.chain().any(|e| e.downcast_ref::<std::io::Error>().is_some_and(|io| io.kind() == std::io::ErrorKind::StorageFull))
}

/// A symbolic link of a Linux build (its target relative to its folder).
#[cfg(unix)]
fn make_link(game_dir: &Path, rel: &str, link_target: &str) -> Result<()> {
    if !link_stays_inside(rel, link_target) {
        bail!("the link {rel} points outside the game's folder ({link_target})");
    }
    let path = target(game_dir, rel)?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    if std::fs::symlink_metadata(&path).is_ok() {
        std::fs::remove_file(&path)?;
    }
    std::os::unix::fs::symlink(link_target, &path).with_context(|| format!("linking {}", path.display()))
}

#[cfg(not(unix))]
fn make_link(_game_dir: &Path, rel: &str, _target: &str) -> Result<()> {
    log::info!("gog: {rel} is a link, which this system does not make");
    Ok(())
}

/// The files of `diff` missing or damaged on disk (a repair fetches only
/// those); `None` when cancelled. `on_percent` gets the bytes checked.
fn broken(diff: &Diff, game_dir: &Path, support_dir: &Path, cancel: &Cancel, on_percent: &mut dyn FnMut(f64)) -> Result<Option<Diff>> {
    let total: u64 = diff.files.iter().map(|f| f.size).sum::<u64>().max(1);
    let checked = AtomicU64::new(0);
    let next = AtomicUsize::new(0);
    let bad: Mutex<Vec<usize>> = Mutex::new(Vec::new());
    let error: Mutex<Option<anyhow::Error>> = Mutex::new(None);
    let check = || {
        while !cancel.is_requested() {
            let i = next.fetch_add(1, Ordering::Relaxed);
            let Some(f) = diff.files.get(i) else { break };
            let root = if f.root == Root::Support { support_dir } else { game_dir };
            match heronite_gog_api::files::matches_on_disk(&root.join(&f.path), f) {
                Ok(true) => {}
                Ok(false) => bad.lock().unwrap_or_else(|e| e.into_inner()).push(i),
                Err(err) => {
                    error.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert(err);
                }
            }
            checked.fetch_add(f.size, Ordering::Relaxed);
        }
    };
    std::thread::scope(|s| {
        let workers = std::thread::available_parallelism().map_or(4, |n| n.get()).clamp(2, 8);
        let handles: Vec<_> = (0..workers).map(|_| s.spawn(check)).collect();
        while handles.iter().any(|h| !h.is_finished()) {
            std::thread::sleep(Duration::from_millis(250));
            on_percent((checked.load(Ordering::Relaxed) as f64 / total as f64 * 100.).min(100.));
        }
    });
    if cancel.is_requested() {
        return Ok(None);
    }
    if let Some(err) = error.into_inner().unwrap_or_else(|e| e.into_inner()) {
        return Err(err);
    }
    on_percent(100.);
    let mut bad = bad.into_inner().unwrap_or_else(|e| e.into_inner());
    bad.sort_unstable();
    let mut out = Diff { dirs: diff.dirs.clone(), ..Default::default() };
    out.files = bad.into_iter().map(|i| diff.files[i].clone()).collect();
    out.links = diff.links.iter().filter(|l| std::fs::symlink_metadata(game_dir.join(&l.path)).is_err()).cloned().collect();
    Ok(Some(out))
}

#[cfg(test)]
mod tests {
    use super::*;
    use heronite_gog_api::files::ChunkPart;
    use std::io::Read;
    use std::net::TcpListener;
    use std::sync::Arc;

    #[cfg(unix)]
    #[test]
    fn linux_builds_never_write_through_or_link_outside_the_game_folder() {
        assert!(link_stays_inside("lib/libfoo.so", "libfoo.so.1"));
        assert!(link_stays_inside("bin/game", "../data/game.x86_64"));
        assert!(!link_stays_inside("lib", "../../.."));
        assert!(!link_stays_inside("bin/x", "../../y"));
        assert!(!link_stays_inside("x", "/etc/passwd"));
        assert!(!link_stays_inside("x", ""));

        let dir = tempfile::tempdir().unwrap();
        let game = dir.path().join("game");
        std::fs::create_dir_all(&game).unwrap();
        assert!(make_link(&game, "lib", "../../..").is_err());
        let outside = dir.path().join("outside");
        std::fs::create_dir_all(&outside).unwrap();
        std::os::unix::fs::symlink(&outside, game.join("lib")).unwrap();
        assert!(target(&game, "lib/evil.so").is_err());
        assert!(target(&game, "lib").is_ok(), "the link itself may be replaced");
        assert!(target(&game, "data/file").is_ok());
    }

    fn md5(b: &[u8]) -> String {
        use md5::Digest;
        hex::encode(md5::Md5::digest(b))
    }

    fn zlib(b: &[u8]) -> Vec<u8> {
        let mut e = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        e.write_all(b).unwrap();
        e.finish().unwrap()
    }

    /// A local CDN serving `files` by path; returns its address and the
    /// paths asked.
    fn cdn(files: Vec<(String, Vec<u8>)>) -> (String, Arc<Mutex<Vec<String>>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let asked = Arc::new(Mutex::new(Vec::new()));
        let log = asked.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut s) = stream else { continue };
                let mut req = [0u8; 4096];
                let n = s.read(&mut req).unwrap_or(0);
                let text = String::from_utf8_lossy(&req[..n]).into_owned();
                let path = text.split_whitespace().nth(1).unwrap_or("").to_string();
                log.lock().unwrap().push(path.clone());
                let range =
                    text.lines().find_map(|l| l.strip_prefix("Range: bytes=").or_else(|| l.strip_prefix("range: bytes="))).and_then(|r| {
                        let (a, b) = r.trim().split_once('-')?;
                        Some((a.parse::<usize>().ok()?, b.parse::<usize>().ok()?))
                    });
                match files.iter().find(|(p, _)| *p == path) {
                    Some((_, body)) => {
                        let (status, body) = match range {
                            Some((a, b)) => ("206 Partial Content", &body[a..=b.min(body.len() - 1)]),
                            None => ("200 OK", &body[..]),
                        };
                        let _ = write!(s, "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
                        let _ = s.write_all(body);
                    }
                    None => {
                        let _ = write!(s, "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
                    }
                }
            }
        });
        (format!("http://{addr}"), asked)
    }

    fn link(base: &str) -> SecureLink {
        SecureLink::parse(&serde_json::json!({"urls": [{"endpoint_name": "local", "url_format": "{base}{path}", "parameters": {"base": base, "path": ""}, "priority": 1}]}))
            .unwrap()
    }

    fn v2_file(path: &str, root: Root, pieces: &[&[u8]]) -> (FileEntry, Vec<(String, Vec<u8>)>) {
        let mut parts = Vec::new();
        let mut served = Vec::new();
        for p in pieces {
            let z = zlib(p);
            let chunk = ChunkRef { md5: md5(p), compressed_md5: md5(&z), size: p.len() as u64, compressed_size: z.len() as u64 };
            served.push((format!("/{}", heronite_gog_api::api::galaxy_path(&chunk.compressed_md5)), z));
            parts.push(ChunkPart { chunk, offset: 0, size: p.len() as u64, old_offset: None });
        }
        let whole: Vec<u8> = pieces.concat();
        let entry = FileEntry {
            path: path.into(),
            root,
            size: whole.len() as u64,
            source: Source::Chunks { product: "42".into(), parts },
            checksum: Some(Checksum::Md5(md5(&whole))),
            executable: false,
        };
        (entry, served)
    }

    fn prepared(diff: Diff, base: &str) -> Prepared {
        Prepared {
            diff,
            folder_name: "The Game".into(),
            generation: 2,
            links: HashMap::from([("42".to_string(), link(base))]),
            installers: Vec::new(),
            stored: Some(serde_json::json!({"version": 2, "buildId": "900"})),
            build_id: "900".into(),
            version_name: "1.5".into(),
            version_etag: Some("\"tag\"".into()),
            dlcs: Vec::new(),
        }
    }

    #[test]
    fn content_builds_write_the_game_and_support_folders_with_one_progress() {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path().join("data"));
        let (a, mut served) = v2_file("bin/game.exe", Root::Game, &[b"hello ", b"world"]);
        let (b, more) = v2_file("42/setup.exe", Root::Support, &[b"support"]);
        served.extend(more);
        // A chunk two files share is fetched once.
        let (c, more) = v2_file("data/copy.bin", Root::Game, &[b"hello "]);
        served.extend(more);
        let (base, asked) = cdn(served);
        let mut diff = Diff { files: vec![a, b, c], dirs: vec!["empty".into()], ..Default::default() };
        diff.deleted.push((Root::Game, "old.dat".into()));
        let game = dir.path().join("Games/The Game");
        std::fs::create_dir_all(&game).unwrap();
        std::fs::write(game.join("old.dat"), "x").unwrap();
        let support = dir.path().join("support");
        let req = Request { app_name: "42".into(), ..Default::default() };
        let p = prepared(diff.clone(), &base);
        let mut reports = Vec::new();
        let out = write_content(&Client::anonymous(), &p, &diff, &game, &support, &paths, &req, &Cancel::new(), &mut |r| reports.push(r))
            .unwrap();
        assert_eq!(out, depot::Outcome::Done);
        assert_eq!(std::fs::read(game.join("bin/game.exe")).unwrap(), b"hello world");
        assert_eq!(std::fs::read(game.join("data/copy.bin")).unwrap(), b"hello ");
        assert_eq!(std::fs::read(support.join("42/setup.exe")).unwrap(), b"support");
        assert!(game.join("empty").is_dir());
        assert!(!game.join("old.dat").exists());
        assert_eq!(asked.lock().unwrap().len(), 3, "{:?}", asked.lock().unwrap());
        let last = reports.last().unwrap();
        assert_eq!((last.files_done, last.files_total), (3, 3));
        assert_eq!(last.percent(), 100.);
    }

    #[test]
    fn v1_files_are_fetched_in_ranges_and_checked_whole() {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path().join("data"));
        let main: Vec<u8> = (0..(RANGE_PIECE * 2 + 100)).map(|i| (i % 251) as u8).collect();
        let (base, asked) = cdn(vec![("/main.bin".into(), main.clone())]);
        let file = |size: u64, sum: &[u8]| FileEntry {
            path: "game.dat".into(),
            root: Root::Game,
            size,
            source: Source::Range { product: "42".into(), offset: 50 },
            checksum: Some(Checksum::Md5(md5(sum))),
            executable: false,
        };
        let size = RANGE_PIECE * 2;
        let good = file(size, &main[50..50 + size as usize]);
        let (plan, _) = depot_plan(&Diff { files: vec![good.clone()], ..Default::default() }, Root::Game);
        assert_eq!(plan.chunks.len(), 2);
        let game = dir.path().join("g");
        let mut p = prepared(Diff::default(), &base);
        p.generation = 1;
        let diff = Diff { files: vec![good], ..Default::default() };
        let req = Request { app_name: "42".into(), ignore_free_space: true, ..Default::default() };
        write_content(&Client::anonymous(), &p, &diff, &game, &dir.path().join("s"), &paths, &req, &Cancel::new(), &mut |_| {}).unwrap();
        assert_eq!(std::fs::read(game.join("game.dat")).unwrap(), &main[50..50 + size as usize]);
        assert!(asked.lock().unwrap().iter().all(|a| a == "/main.bin"));
        // A wrong md5 fails the file.
        let bad = Diff { files: vec![file(10, b"other")], ..Default::default() };
        let err = write_content(
            &Client::anonymous(),
            &p,
            &bad,
            &dir.path().join("h"),
            &dir.path().join("s"),
            &paths,
            &req,
            &Cancel::new(),
            &mut |_| {},
        );
        assert!(err.is_err());
    }

    #[test]
    fn unchanged_bytes_of_an_update_are_read_from_the_old_file() {
        let (a, _) = v2_file("big.pak", Root::Game, &[b"aaaa", b"bbbb"]);
        let mut a = a;
        if let Source::Chunks { parts, .. } = &mut a.source {
            parts[0].old_offset = Some(4);
        }
        let (plan, pieces) = depot_plan(&Diff { files: vec![a], ..Default::default() }, Root::Game);
        assert_eq!(pieces.len(), 1);
        assert_eq!(plan.files[0].parts[0], depot::Part::Local { path: "big.pak".into(), offset: 4, size: 4 });
        assert!(matches!(plan.files[0].parts[1], depot::Part::Chunk { chunk: 0, offset: 0, size: 4 }));
    }

    #[test]
    fn repair_finds_the_damaged_and_missing_files() {
        let dir = tempfile::tempdir().unwrap();
        let (good, _) = v2_file("good", Root::Game, &[b"fine"]);
        let (bad, _) = v2_file("bad", Root::Game, &[b"want"]);
        let (gone, _) = v2_file("42/gone", Root::Support, &[b"x"]);
        std::fs::write(dir.path().join("good"), "fine").unwrap();
        std::fs::write(dir.path().join("bad"), "oops").unwrap();
        let diff = Diff { files: vec![good, bad, gone], ..Default::default() };
        let mut percents = Vec::new();
        let out = broken(&diff, dir.path(), &dir.path().join("support"), &Cancel::new(), &mut |p| percents.push(p)).unwrap().unwrap();
        assert_eq!(out.files.iter().map(|f| f.path.as_str()).collect::<Vec<_>>(), ["bad", "42/gone"]);
        assert_eq!(percents.last(), Some(&100.));
        let cancel = Cancel::new();
        cancel.request();
        assert!(broken(&diff, dir.path(), dir.path(), &cancel, &mut |_| {}).unwrap().is_none());
    }

    #[test]
    fn totals_add_plans_up() {
        let mut t = Totals { download: 300, write: 1000, files: 4, ..Default::default() };
        let first = t.report(&depot::Progress { downloaded: 100, written: 400, write_total: 400, files_done: 1, ..Default::default() });
        assert_eq!((first.downloaded, first.written, first.write_total), (100, 400, 1000));
        t.finish(&first);
        let second = t.report(&depot::Progress { downloaded: 200, written: 600, resumed: 0, files_done: 3, ..Default::default() });
        assert_eq!((second.downloaded, second.written, second.files_done), (300, 1000, 4));
        assert_eq!(second.percent(), 100.);
    }
}
