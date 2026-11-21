//! Writing a [`Plan`] into an install folder.
//!
//! The calling thread writes the files one after another, in the plan's
//! order; worker threads fetch chunks from the [`Source`] ahead of it (see
//! [`crate::fetch`]). Each file is written beside its place under a temporary
//! name and renamed over the old version once complete and checked, so a
//! file's old version stays readable for the [`Part::Local`] parts that copy
//! from it. A file whose old version other unfinished files still read
//! waits, written, until they are done.

use std::collections::{HashMap, HashSet};
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufWriter, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, Result};
use heronite_data::proc::Cancel;
use heronite_i18n::tr;

use crate::fetch::Fetcher;
use crate::journal::Journal;
use crate::plan::{join, key, Chunk, FileTask, Part, Plan};
use crate::progress::{Meter, Progress};
use sha2::Digest;

/// Where chunks come from: a store's CDN, a byte range of an installer...
pub trait Source: Send + Sync {
    /// The chunk's bytes, decoded and checked: exactly `chunk.size` of them.
    /// Called from several threads at once; should give up soon after
    /// `cancel` is requested. A failure is tried again a few times.
    fn fetch(&self, chunk: &Chunk, cancel: &Cancel) -> Result<Vec<u8>>;
}

/// How an install runs.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Options {
    /// Chunks fetched at once; `0` picks from the number of processors.
    pub workers: usize,
    /// Bytes of decoded chunks held at once; `0` is 512 MiB.
    pub max_memory: u64,
    /// The resume journal; `None` starts over every time.
    pub state_file: Option<PathBuf>,
}

/// How a run ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Done,
    /// The journal (if any) lets a later run of the same plan go on.
    Cancelled,
}

const DEFAULT_MEMORY: u64 = 512 << 20;
const REPORT_EVERY: Duration = Duration::from_millis(250);

/// Writes `plan` into `root` with the chunks `source` gives. Blocking: run it
/// on a worker thread. Files an earlier, cancelled run of the same plan
/// finished (per `opts.state_file`) are kept. `on_progress` is called on
/// this thread a few times a second and once at the end.
pub fn install(
    root: &Path,
    plan: &Plan,
    source: &dyn Source,
    opts: &Options,
    cancel: &Cancel,
    on_progress: &mut dyn FnMut(&Progress),
) -> Result<Outcome> {
    plan.validate()?;
    fs::create_dir_all(root).map_err(|e| io_error(e, root))?;
    let mut dirs = Dirs::new(root)?;
    for d in &plan.dirs {
        dirs.ensure(&join(root, d)?)?;
    }
    // The plan and where it is written: a journal of the same plan for another
    // folder is not this one's.
    let root_id = crate::plan::hex(&sha2::Sha256::digest(dirs.root.to_string_lossy().as_bytes()));
    let fingerprint = format!("{} {root_id}", plan.fingerprint());
    let mut journal = Journal::open(opts.state_file.as_deref(), &fingerprint).map_err(|e| anyhow!("resume journal: {e}"))?;
    let states = resume_states(root, plan, &journal);

    let todo: Vec<usize> = (0..plan.files.len()).filter(|i| states[*i] == State::Todo).collect();
    // Unfinished files that read each path's old version.
    let mut readers: HashMap<String, usize> = HashMap::new();
    for &i in &todo {
        for path in local_paths(&plan.files[i]) {
            *readers.entry(path).or_default() += 1;
        }
    }
    let mut waiting: Vec<usize> = (0..plan.files.len()).filter(|i| states[*i] == State::Written).collect();

    let mut reporter = Reporter::new(on_progress);
    let p = &mut reporter.progress;
    p.files_total = plan.files.len();
    p.files_done = plan.files.len() - todo.len();
    p.resumed = (0..plan.files.len()).filter(|i| states[*i] != State::Todo).map(|i| plan.files[i].size).sum();
    p.write_total = todo.iter().map(|&i| plan.files[i].size).sum();

    let workers = crate::workers(opts.workers);
    let max_memory = if opts.max_memory == 0 { DEFAULT_MEMORY } else { opts.max_memory };
    let stop = AtomicBool::new(false);
    let (jobs_tx, jobs_rx) = crossbeam_channel::unbounded::<usize>();
    let (results_tx, results_rx) = crossbeam_channel::unbounded::<(usize, Result<Vec<u8>>)>();

    let outcome = std::thread::scope(|s| {
        for _ in 0..workers {
            let (jobs, results, stop) = (jobs_rx.clone(), results_tx.clone(), &stop);
            s.spawn(move || {
                for c in jobs.iter() {
                    if stop.load(Ordering::Relaxed) || cancel.is_requested() {
                        break;
                    }
                    let got = crate::fetch::with_retries(source, &plan.chunks[c], cancel);
                    if results.send((c, got)).is_err() {
                        break;
                    }
                }
            });
        }
        drop(results_tx);
        let mut fetcher = Fetcher::new(plan, &todo, jobs_tx, results_rx, max_memory, workers * 2);
        reporter.progress.download_total = fetcher.download_total();
        let mut run = Run { root, plan, cancel, fetcher: &mut fetcher, reporter: &mut reporter, dirs: &mut dirs };
        let result = run.write_all(&todo, &mut journal, &mut readers, &mut waiting);
        stop.store(true, Ordering::Relaxed);
        // Dropping the fetcher closes the job queue: the workers end.
        drop(fetcher);
        result
    })?;
    if outcome == Outcome::Cancelled {
        return Ok(outcome);
    }

    for r in &plan.remove {
        let path = join(root, r)?;
        match fs::remove_file(&path) {
            Ok(()) => remove_empty_parents(root, &path),
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(io_error(e, &path)),
        }
    }
    journal.finish();
    reporter.update(true);
    Ok(Outcome::Done)
}

/// Where a file of the plan stands when the run starts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Todo,
    /// Complete under its temporary name, waiting to be renamed.
    Written,
    Placed,
}

/// What the journal says an earlier run finished, when the disk agrees.
fn resume_states(root: &Path, plan: &Plan, journal: &Journal) -> Vec<State> {
    plan.files
        .iter()
        .enumerate()
        .map(|(i, f)| {
            let Ok(dest) = join(root, &f.path) else { return State::Todo };
            let temp = temp_path(&dest);
            // A rename the earlier run was doing happened when the temporary
            // file is gone.
            let renamed = journal.renaming.contains(&i) && fs::symlink_metadata(&temp).is_err();
            if (journal.placed.contains(&i) || renamed) && in_place(&dest, f) && matches_hash(&dest, f) {
                State::Placed
            } else if journal.written.contains(&i)
                && f.symlink.is_none()
                && fs::metadata(&temp).is_ok_and(|m| m.len() == f.size)
                && matches_hash(&temp, f)
            {
                State::Written
            } else {
                State::Todo
            }
        })
        .collect()
}

fn in_place(dest: &Path, f: &FileTask) -> bool {
    match &f.symlink {
        Some(target) => fs::read_link(dest).is_ok_and(|t| t == Path::new(target)),
        None => fs::symlink_metadata(dest).is_ok_and(|m| m.is_file() && m.len() == f.size),
    }
}

/// A file kept from an earlier run is checked again when its hash is
/// known: the disk may have lost what was written before a crash.
fn matches_hash(path: &Path, f: &FileTask) -> bool {
    match &f.hash {
        Some(expected) if f.symlink.is_none() => {
            File::open(path).ok().and_then(|mut file| expected.of_reader(&mut file).ok()).is_some_and(|got| got == *expected)
        }
        _ => true,
    }
}

/// The paths a file copies old data from, each once.
fn local_paths(f: &FileTask) -> HashSet<String> {
    f.parts
        .iter()
        .filter_map(|p| match p {
            Part::Local { path, .. } => Some(key(path)),
            Part::Chunk { .. } => None,
        })
        .collect()
}

/// The temporary name a file is written under, beside its place.
fn temp_path(dest: &Path) -> PathBuf {
    let mut name = dest.file_name().unwrap_or_default().to_os_string();
    name.push(".heronite-part");
    dest.with_file_name(name)
}

/// An I/O failure for the user: a full disk says so.
fn io_error(e: io::Error, path: &Path) -> anyhow::Error {
    if e.kind() == io::ErrorKind::StorageFull {
        anyhow!(tr!("Not enough disk space"))
    } else {
        anyhow!("{}: {e}", path.display())
    }
}

/// Folders created on the way, each checked to be inside the install folder
/// (a link already in it could lead elsewhere).
struct Dirs {
    /// The install folder as given, and where it really is.
    base: PathBuf,
    root: PathBuf,
    made: HashSet<PathBuf>,
}

impl Dirs {
    fn new(root: &Path) -> Result<Self> {
        let real = root.canonicalize().map_err(|e| io_error(e, root))?;
        Ok(Self { made: HashSet::from([root.to_path_buf(), real.clone()]), base: root.to_path_buf(), root: real })
    }

    /// Creates `dir` (inside the install folder) one folder at a time,
    /// checking each one that exists before going into it: a link already
    /// on disk that leads outside the install folder stops the run before
    /// anything is created behind it.
    fn ensure(&mut self, dir: &Path) -> Result<()> {
        if self.made.contains(dir) {
            return Ok(());
        }
        let rel = dir.strip_prefix(&self.base).map_err(|_| anyhow::anyhow!("{} is outside the install folder", dir.display()))?;
        let mut at = self.base.clone();
        for part in rel.components() {
            at.push(part);
            if self.made.contains(&at) {
                continue;
            }
            match fs::symlink_metadata(&at) {
                Ok(meta) if meta.file_type().is_symlink() => {
                    let real = at.canonicalize().map_err(|e| io_error(e, &at))?;
                    if !real.starts_with(&self.root) {
                        bail!("{} leads outside the install folder", at.display());
                    }
                }
                Ok(_) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => match fs::create_dir(&at) {
                    Err(e) if e.kind() != std::io::ErrorKind::AlreadyExists => return Err(io_error(e, &at)),
                    _ => {}
                },
                Err(e) => return Err(io_error(e, &at)),
            }
            self.made.insert(at.clone());
        }
        Ok(())
    }
}

/// Removes the folders above `path` that are now empty, up to `root`.
fn remove_empty_parents(root: &Path, path: &Path) {
    let mut dir = path.parent();
    while let Some(d) = dir.filter(|d| d.starts_with(root) && *d != root) {
        if fs::remove_dir(d).is_err() {
            break;
        }
        dir = d.parent();
    }
}

struct Run<'a, 'p, 'f> {
    root: &'a Path,
    plan: &'a Plan,
    cancel: &'a Cancel,
    fetcher: &'a mut Fetcher<'p>,
    reporter: &'a mut Reporter<'f>,
    dirs: &'a mut Dirs,
}

impl Run<'_, '_, '_> {
    fn write_all(
        &mut self,
        todo: &[usize],
        journal: &mut Journal,
        readers: &mut HashMap<String, usize>,
        waiting: &mut Vec<usize>,
    ) -> Result<Outcome> {
        // Resumed files that no one reads any more go in place first.
        self.place_ready(journal, readers, waiting)?;
        let mut links = Vec::new();
        for &i in todo {
            let f = &self.plan.files[i];
            if f.symlink.is_some() {
                links.push(i);
                continue;
            }
            if !self.write_file(f)? {
                return Ok(Outcome::Cancelled);
            }
            journal.written(i).map_err(|e| anyhow!("resume journal: {e}"))?;
            for path in local_paths(f) {
                if let Some(n) = readers.get_mut(&path) {
                    *n = n.saturating_sub(1);
                }
            }
            waiting.push(i);
            self.place_ready(journal, readers, waiting)?;
            self.reporter.progress.files_done += 1;
            self.reporter.update(false);
        }
        if !waiting.is_empty() {
            bail!("{} files could not be put in place", waiting.len());
        }
        // Links last: no file of the plan is written through one.
        for i in links {
            self.make_link(&self.plan.files[i])?;
            journal.placed(i).map_err(|e| anyhow!("resume journal: {e}"))?;
            self.reporter.progress.files_done += 1;
        }
        Ok(Outcome::Done)
    }

    /// Renames the written files whose old version nothing still reads.
    fn place_ready(&mut self, journal: &mut Journal, readers: &HashMap<String, usize>, waiting: &mut Vec<usize>) -> Result<()> {
        let mut still = Vec::new();
        for &i in waiting.iter() {
            let f = &self.plan.files[i];
            if readers.get(&key(&f.path)).copied().unwrap_or(0) > 0 {
                still.push(i);
                continue;
            }
            let dest = join(self.root, &f.path)?;
            journal.renaming(i).map_err(|e| anyhow!("resume journal: {e}"))?;
            heronite_platform::fs::rename_replacing(&temp_path(&dest), &dest).map_err(|e| io_error(e, &dest))?;
            journal.placed(i).map_err(|e| anyhow!("resume journal: {e}"))?;
        }
        *waiting = still;
        Ok(())
    }

    /// Writes one file under its temporary name; `false` when cancelled.
    fn write_file(&mut self, f: &FileTask) -> Result<bool> {
        let dest = join(self.root, &f.path)?;
        if let Some(parent) = dest.parent() {
            self.dirs.ensure(parent)?;
        }
        let temp = temp_path(&dest);
        // Never through a link left at the temporary name.
        let _ = fs::remove_file(&temp);
        let file = OpenOptions::new().write(true).create_new(true).open(&temp).map_err(|e| io_error(e, &temp))?;
        let result = self.fill(f, file, &temp);
        match result {
            Ok(true) => {
                if f.executable {
                    heronite_platform::fs::make_executable(&temp).map_err(|e| io_error(e, &temp))?;
                }
                Ok(true)
            }
            other => {
                let _ = fs::remove_file(&temp);
                other
            }
        }
    }

    fn fill(&mut self, f: &FileTask, file: File, temp: &Path) -> Result<bool> {
        let mut out = BufWriter::with_capacity(4 << 20, file);
        let mut hasher = f.hash.as_ref().map(|h| h.hasher());
        let mut olds: HashMap<&str, File> = HashMap::new();
        let mut buf = Vec::new();
        for part in &f.parts {
            if self.cancel.is_requested() {
                return Ok(false);
            }
            match part {
                Part::Chunk { chunk, offset, size } => {
                    let reporter = &mut *self.reporter;
                    if !self.fetcher.wait_for(*chunk, self.cancel, &mut |downloaded| {
                        reporter.progress.downloaded = downloaded;
                        reporter.update(false);
                    })? {
                        return Ok(false);
                    }
                    let data = self.fetcher.data(*chunk);
                    let bytes = &data[*offset as usize..(*offset + *size) as usize];
                    out.write_all(bytes).map_err(|e| io_error(e, temp))?;
                    if let Some(h) = &mut hasher {
                        h.update(bytes);
                    }
                    self.fetcher.consumed(*chunk);
                }
                Part::Local { path, offset, size } => {
                    let old = match olds.entry(path.as_str()) {
                        std::collections::hash_map::Entry::Occupied(e) => e.into_mut(),
                        std::collections::hash_map::Entry::Vacant(e) => {
                            let src = join(self.root, path)?;
                            e.insert(File::open(&src).map_err(|err| anyhow!("{}: {err}", src.display()))?)
                        }
                    };
                    old.seek(SeekFrom::Start(*offset)).map_err(|e| anyhow!("{path}: {e}"))?;
                    let mut left = *size;
                    buf.resize(1 << 20, 0);
                    while left > 0 {
                        let n = left.min(buf.len() as u64) as usize;
                        old.read_exact(&mut buf[..n]).map_err(|e| anyhow!("{path} of the installed version: {e}"))?;
                        out.write_all(&buf[..n]).map_err(|e| io_error(e, temp))?;
                        if let Some(h) = &mut hasher {
                            h.update(&buf[..n]);
                        }
                        left -= n as u64;
                    }
                }
            }
            self.reporter.progress.written += part.size();
            self.fetcher.receive_ready()?;
            self.fetcher.prefetch();
            self.reporter.progress.downloaded = self.fetcher.downloaded();
            self.reporter.update(false);
        }
        // On disk before the journal says it is written.
        let file = out.into_inner().map_err(|e| io_error(e.into_error(), temp))?;
        file.sync_data().map_err(|e| io_error(e, temp))?;
        drop(file);
        if let (Some(h), Some(expected)) = (hasher, &f.hash) {
            if h.finish() != *expected {
                bail!("{} came out damaged: its hash does not match", f.path);
            }
        }
        Ok(true)
    }

    fn make_link(&mut self, f: &FileTask) -> Result<()> {
        let target = f.symlink.as_deref().unwrap_or_default();
        let dest = join(self.root, &f.path)?;
        if cfg!(windows) {
            log::info!("skipping the link {} -> {target}: links are for Linux and macOS builds", f.path);
            return Ok(());
        }
        if let Some(parent) = dest.parent() {
            self.dirs.ensure(parent)?;
        }
        match fs::symlink_metadata(&dest) {
            Ok(m) if m.is_dir() => bail!("{} is a folder, not a link", f.path),
            Ok(_) => fs::remove_file(&dest).map_err(|e| io_error(e, &dest))?,
            Err(_) => {}
        }
        heronite_platform::fs::symlink(Path::new(target), &dest).map_err(|e| io_error(e, &dest))
    }
}

/// Throttled progress for the caller.
struct Reporter<'f> {
    on_progress: &'f mut dyn FnMut(&Progress),
    progress: Progress,
    start: Instant,
    last: Option<Instant>,
    down: Meter,
    disk: Meter,
}

impl<'f> Reporter<'f> {
    fn new(on_progress: &'f mut dyn FnMut(&Progress)) -> Self {
        let window = Duration::from_secs(3);
        Self {
            on_progress,
            progress: Progress::default(),
            start: Instant::now(),
            last: None,
            down: Meter::new(window),
            disk: Meter::new(window),
        }
    }

    /// Tells the caller, at most every [`REPORT_EVERY`] unless `force`.
    fn update(&mut self, force: bool) {
        let now = Instant::now();
        if !force && self.last.is_some_and(|t| now.duration_since(t) < REPORT_EVERY) {
            return;
        }
        self.last = Some(now);
        let p = &mut self.progress;
        p.down_speed = self.down.sample(now, p.downloaded);
        p.disk_speed = self.disk.sample(now, p.written);
        let secs = now.duration_since(self.start).as_secs_f64();
        p.eta = (p.written > 0 && secs > 0.).then(|| {
            let rate = p.written as f64 / secs;
            Duration::from_secs_f64(p.write_total.saturating_sub(p.written) as f64 / rate)
        });
        (self.on_progress)(p);
    }
}

/// Bytes of free space the plan needs in `root`: what its files add over the
/// versions they replace, plus room for the biggest replaced file, which
/// exists twice for a moment. Removed files are not counted: they go last.
pub fn space_needed(root: &Path, plan: &Plan) -> u64 {
    let mut grows = 0u64;
    let mut room = 0u64;
    for f in &plan.files {
        let old = join(root, &f.path).ok().and_then(|p| fs::symlink_metadata(p).ok()).filter(|m| m.is_file()).map(|m| m.len());
        match old {
            Some(old) => {
                grows += f.size.saturating_sub(old);
                room = room.max(f.size.min(old));
            }
            None => grows += f.size,
        }
    }
    grows + room
}
