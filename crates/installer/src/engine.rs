//! Running a script: prepare the folders and tools, fetch the files, run the
//! steps in order, then add the game to the Collection ([`crate::finish`]).
//! Everything happens on the calling thread; the [`Host`] hears about it,
//! answers the script's questions and can stop it.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use heronite_data::DataPaths;
use heronite_i18n::tr;
use heronite_launch::PlanContext;
use serde_json::Value;

use crate::archive;
use crate::exec::{run_ok, Io};
use crate::files::{self, Source};
use crate::fsops;
use crate::script::Script;
use crate::steps::{Action, Step};
use crate::text::pairs;
use crate::vars::Vars;

/// The error text of a stopped install.
pub const CANCELLED: &str = "cancelled";

/// What to install and where.
#[derive(Debug, Clone)]
pub struct Job {
    pub script: Script,
    /// Id of the local game it becomes (`sideload::generate_app_name`).
    pub app_name: String,
    pub title: String,
    /// `$GAMEDIR`.
    pub game_dir: PathBuf,
    /// The Wine/Proton to use, as a game's `wineVersion` setting stores it.
    pub wine_version: Option<Value>,
    /// Files the user picked, by file id.
    pub provided: HashMap<String, PathBuf>,
    /// The screen size in pixels (`$RESOLUTION`).
    pub resolution: Option<(u32, u32)>,
    /// Downloads go to `<cache_root>/<game slug>` (`$CACHE`).
    pub cache_root: PathBuf,
    pub paths: DataPaths,
    pub home: String,
    /// Picks up a failed install at one of its steps.
    pub resume: Option<Resume>,
}

/// Where a retried install starts, and what it keeps from the failed run.
#[derive(Debug, Clone, Default)]
pub struct Resume {
    /// Index into the script's steps: the ones before it are not run again.
    pub from: usize,
    /// The answers given before (`INPUT`, `INPUT_<id>`, `DISC`).
    pub answers: Vec<(String, String)>,
    /// The first run created the game's folder.
    pub created_dir: bool,
}

impl Job {
    pub fn cache_dir(&self) -> PathBuf {
        self.cache_root.join(self.script.folder_name())
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// Now on this entry of [`stages`].
    Stage(usize),
    /// Progress inside the stage (`None`: unknown).
    Progress { fraction: Option<f32>, text: String },
    /// A line of output of a program.
    Log(String),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Question {
    /// Pick one of `(value, label)`.
    Menu { description: String, options: Vec<(String, String)>, preselect: Option<String> },
    /// The folder of a disc or mounted image holding `requires`.
    Disc { message: String, requires: String, wrong: Option<PathBuf> },
}

#[derive(Debug, Clone, PartialEq)]
pub enum Answer {
    Choice(String),
    Folder(PathBuf),
}

pub trait Host: Send + Sync {
    fn event(&self, event: Event);
    fn cancelled(&self) -> bool;
    /// Blocks until the user answers; `None` stops the install.
    fn ask(&self, question: Question) -> Option<Answer>;
}

#[derive(Debug, Clone)]
pub struct Outcome {
    pub app_name: String,
    pub title: String,
    /// Things the user should know (settings of the script that were left
    /// out, a program that was not found).
    pub notes: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct Failure {
    pub message: String,
    pub cancelled: bool,
    /// The game's folder did not exist (or was empty) before: removing it
    /// undoes the install.
    pub created_dir: bool,
    /// The step that failed (index into the script's steps), when one did.
    pub step: Option<usize>,
    /// The answers given so far, for a retry ([`Resume::answers`]).
    pub answers: Vec<(String, String)>,
}

/// The stages of an install, as the progress list shows them.
pub fn stages(script: &Script) -> Vec<String> {
    let mut out = vec![tr!("Prepare the installation").to_string()];
    out.extend(script.steps.iter().map(Step::summary));
    out.push(tr!("Add the game to the Collection").to_string());
    out
}

/// Installs `job`. On failure the game's folder is left as it is.
pub fn install(job: &Job, host: &dyn Host) -> Result<Outcome, Failure> {
    let created_dir = match &job.resume {
        Some(r) => r.created_dir,
        None => !job.game_dir.exists() || fs_empty(&job.game_dir),
    };
    let mut st = State::new(job, host);
    let result = st.run_all();
    result.map_err(|message| Failure {
        cancelled: message == CANCELLED || host.cancelled(),
        message,
        created_dir,
        step: st.failed_step,
        answers: st.answers.clone(),
    })
}

fn fs_empty(dir: &Path) -> bool {
    std::fs::read_dir(dir).map(|mut d| d.next().is_none()).unwrap_or(false)
}

/// Deletes the game's folder after a failed or stopped install, when the
/// install created it. Never a folder that holds the user's home.
pub fn remove_game_dir(job: &Job) -> Result<(), String> {
    let dir = &job.game_dir;
    let home = Path::new(&job.home);
    if dir.components().count() < 3 || home.starts_with(dir) || dir == home {
        return Err(tr!("Won't delete {path}", path = dir.display()));
    }
    match std::fs::remove_dir_all(dir) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!("{}: {e}", dir.display())),
    }
}

pub(crate) struct State<'a> {
    pub job: &'a Job,
    pub host: &'a dyn Host,
    pub vars: Vars,
    /// Where each file id is now.
    pub files: HashMap<String, PathBuf>,
    pub plan: PlanContext,
    pub notes: Vec<String>,
    accepted: Option<i32>,
    failed_step: Option<usize>,
    answers: Vec<(String, String)>,
}

impl<'a> State<'a> {
    fn new(job: &'a Job, host: &'a dyn Host) -> Self {
        let mut vars = base_vars(job);
        let answers = job.resume.as_ref().map(|r| r.answers.clone()).unwrap_or_default();
        for (k, v) in &answers {
            vars.set(k.clone(), v.clone());
        }
        Self {
            job,
            host,
            vars,
            files: HashMap::new(),
            plan: PlanContext::load(&job.paths),
            notes: Vec::new(),
            accepted: None,
            failed_step: None,
            answers,
        }
    }

    pub fn cancelled(&self) -> bool {
        self.host.cancelled()
    }

    pub fn log(&self, line: &str) {
        self.host.event(Event::Log(line.to_string()));
    }

    pub fn progress(&self, fraction: Option<f32>, text: impl Into<String>) {
        self.host.event(Event::Progress { fraction, text: text.into() });
    }

    pub fn sub(&self, s: &str) -> String {
        self.vars.substitute(s)
    }

    /// A file id's current path, else the substituted path (relative paths
    /// are inside the game's folder).
    pub fn path(&self, s: &str) -> PathBuf {
        let key = s.trim().trim_start_matches('$');
        if let Some(p) = self.files.get(key).or_else(|| self.files.get(&key.replace('-', "_"))) {
            return p.clone();
        }
        let text = self.sub(s.trim());
        let p = PathBuf::from(&text);
        if p.is_absolute() {
            p
        } else {
            self.job.game_dir.join(p)
        }
    }

    /// [`State::path`] with the case of existing folders fixed.
    pub fn existing_path(&self, s: &str) -> PathBuf {
        fsops::fix_case(&self.path(s))
    }

    /// The script's `system.env`, substituted.
    /// Sets a variable the user's answer gave (kept for a retry).
    fn remember(&mut self, name: impl Into<String>, value: String) {
        let name = name.into();
        self.vars.set(name.clone(), value.clone());
        self.answers.retain(|(k, _)| *k != name);
        self.answers.push((name, value));
    }

    pub fn script_env(&self) -> Vec<(String, String)> {
        pairs(self.job.script.system.get("env")).into_iter().map(|(k, v)| (k, self.sub(&v))).collect()
    }

    /// Runs a command of the install (output to the log).
    pub fn run(&self, cmd: &mut Command) -> Result<(), String> {
        let cancel = || self.host.cancelled();
        let log = |l: &str| self.log(l);
        run_ok(cmd, &Io { cancel: &cancel, log: &log }, self.accepted)
    }

    pub fn io_parts(&self) -> (impl Fn() -> bool + '_, impl Fn(&str) + '_) {
        (move || self.host.cancelled(), move |l: &str| self.log(l))
    }

    fn run_all(&mut self) -> Result<Outcome, String> {
        let job = self.job;
        self.host.event(Event::Stage(0));
        let runner = crate::runners::runner(&job.script.runner)?;
        std::fs::create_dir_all(&job.game_dir).map_err(|e| format!("{}: {e}", job.game_dir.display()))?;
        std::fs::create_dir_all(job.cache_dir()).map_err(|e| format!("{}: {e}", job.cache_dir().display()))?;
        if runner == crate::runners::Runner::Wine {
            self.prepare_wine()?;
        }
        self.fetch_files()?;
        let from = job.resume.as_ref().map_or(0, |r| r.from);
        for (i, step) in job.script.steps.iter().enumerate() {
            if self.cancelled() {
                return Err(CANCELLED.into());
            }
            self.host.event(Event::Stage(i + 1));
            if i < from {
                continue;
            }
            self.accepted = step.return_code;
            let result = self.run_step(step);
            self.accepted = None;
            if let Err(e) = result {
                if e == CANCELLED || self.cancelled() {
                    self.failed_step = Some(i);
                    return Err(CANCELLED.into());
                }
                self.failed_step = Some(i);
                return Err(tr!("{step}: {error}", step = step.summary(), error = e));
            }
        }
        if self.cancelled() {
            return Err(CANCELLED.into());
        }
        self.host.event(Event::Stage(job.script.steps.len() + 1));
        let outcome = crate::finish::add_game(self, runner)?;
        let _ = std::fs::remove_dir_all(job.cache_dir());
        Ok(outcome)
    }

    fn fetch_files(&mut self) -> Result<(), String> {
        let job = self.job;
        let total = job.script.files.len();
        for (i, spec) in job.script.files.iter().enumerate() {
            if self.cancelled() {
                return Err(CANCELLED.into());
            }
            let path = match (&spec.source, job.provided.get(&spec.id)) {
                (_, Some(p)) => p.clone(),
                (Source::Ask(_), None) => return Err(tr!("No file was selected for \"{file}\"", file = spec.prompt())),
                (Source::Unsupported(raw), None) => return Err(tr!("The file {raw} cannot be downloaded here", raw)),
                (Source::Local(raw), None) => {
                    let text = self.sub(raw);
                    let p = PathBuf::from(&text);
                    let p = if p.is_absolute() { p } else { job.script.script_dir.clone().unwrap_or_default().join(p) };
                    if !p.exists() {
                        return Err(tr!("{path} does not exist", path = p.display()));
                    }
                    p
                }
                (Source::Url(url), None) => self.download(spec, url, i, total)?,
            };
            if let Some(sum) = &spec.checksum {
                self.progress(None, tr!("Checking {file}", file = files::percent_decode(&spec.file_name())));
                let (cancel, _) = self.io_parts();
                files::verify(&path, sum, &cancel)?;
            }
            self.vars.set(spec.id.clone(), path.to_string_lossy().into_owned());
            self.files.insert(spec.id.clone(), path);
        }
        Ok(())
    }

    fn download(&self, spec: &files::FileSpec, url: &str, index: usize, total: usize) -> Result<PathBuf, String> {
        let dest = self.job.cache_dir().join(spec.file_name());
        let cancel = || self.host.cancelled();
        if dest.is_file() {
            // Kept from an earlier attempt: used again when it still checks out.
            let ok = match &spec.checksum {
                Some(sum) => files::verify(&dest, sum, &cancel).is_ok(),
                None => dest.metadata().map(|m| m.len() > 0).unwrap_or(false),
            };
            if ok {
                return Ok(dest);
            }
        }
        let name = spec.file_name();
        let label = if total > 1 { format!("{name} ({}/{total})", index + 1) } else { name.clone() };
        self.progress(Some(0.0), tr!("Downloading {file}", file = label.clone()));
        let mut last = std::time::Instant::now();
        let mut report = |got: u64, size: Option<u64>| {
            if last.elapsed().as_millis() < 200 {
                return;
            }
            last = std::time::Instant::now();
            let text = match size {
                Some(t) => tr!("Downloading {file}: {done} of {total}", file = label.clone(), done = mib(got), total = mib(t)),
                None => tr!("Downloading {file}: {done}", file = label.clone(), done = mib(got)),
            };
            self.progress(size.map(|t| got as f32 / t.max(1) as f32), text);
        };
        files::download(url, &dest, spec.referer.as_deref(), &mut report, &cancel)?;
        Ok(dest)
    }

    fn run_step(&mut self, step: &Step) -> Result<(), String> {
        match &step.action {
            Action::Chmodx(f) => fsops::make_executable(&self.existing_path(f)),
            Action::Mkdir(d) => {
                let p = self.path(d);
                std::fs::create_dir_all(&p).map_err(|e| format!("{}: {e}", p.display()))
            }
            Action::Execute(e) => self.execute(e),
            Action::Extract(x) => {
                let dst = x.dst.as_deref().map(|d| self.path(d)).unwrap_or_else(|| self.job.game_dir.clone());
                let format = x.format.as_deref().and_then(archive::named);
                for file in self.matching(&x.file)? {
                    self.progress(None, tr!("Extracting {file}", file = crate::steps::short(&file.to_string_lossy())));
                    let (cancel, log) = self.io_parts();
                    archive::extract(&file, &dst, format, !x.nomerge, &Io { cancel: &cancel, log: &log })?;
                }
                Ok(())
            }
            Action::InputMenu(m) => {
                let q = Question::Menu {
                    description: self.sub(&m.description),
                    options: m.options.clone(),
                    preselect: m.preselect.as_deref().map(|p| self.sub(p)),
                };
                match self.host.ask(q) {
                    Some(Answer::Choice(v)) => {
                        self.remember("INPUT", v.clone());
                        if let Some(id) = &m.id {
                            self.remember(format!("INPUT_{id}"), v);
                        }
                        Ok(())
                    }
                    _ => Err(CANCELLED.into()),
                }
            }
            Action::InsertDisc { requires, message } => self.insert_disc(requires, message.as_deref()),
            Action::Merge { src, dst, optional, remove_source } => {
                let from = self.existing_path(src);
                if !from.exists() {
                    return if *optional { Ok(()) } else { Err(tr!("{path} does not exist", path = from.display())) };
                }
                let to = self.path(dst);
                let copied = fsops::merge(&from, &to)?;
                self.relink(src, copied);
                if *remove_source && from.is_dir() {
                    let _ = std::fs::remove_dir_all(&from);
                }
                Ok(())
            }
            Action::Move { src, dst, optional } => {
                let from = self.existing_path(src);
                if !from.exists() {
                    return if *optional { Ok(()) } else { Err(tr!("{path} does not exist", path = from.display())) };
                }
                let target =
                    if self.picked(&from) { fsops::copy_into(&from, &self.path(dst))? } else { fsops::move_into(&from, &self.path(dst))? };
                self.relink(src, Some(target));
                Ok(())
            }
            Action::Rename { src, dst } => {
                let from = self.existing_path(src);
                let to = self.path(dst);
                if self.picked(&from) {
                    if to.exists() {
                        return Err(tr!("Cannot rename to {path}: it already exists", path = to.display()));
                    }
                    fsops::copy_into(&from, &to)?;
                } else {
                    fsops::rename(&from, &to)?;
                }
                self.relink(src, Some(to));
                Ok(())
            }
            Action::SubstituteVars(f) => {
                let p = self.existing_path(f);
                let text = std::fs::read_to_string(&p).map_err(|e| format!("{}: {e}", p.display()))?;
                fsops::write_text(&p, &self.sub(&text), false)
            }
            Action::WriteFile { file, content, append } => fsops::write_text(&self.path(file), &self.sub(content), *append),
            Action::WriteJson { file, data, merge } => fsops::write_json(&self.path(file), data, *merge),
            Action::WriteConfig { file, data, merge } => {
                let data: Vec<(String, Vec<(String, String)>)> =
                    data.iter().map(|(s, keys)| (s.clone(), keys.iter().map(|(k, v)| (k.clone(), self.sub(v))).collect())).collect();
                fsops::write_ini(&self.path(file), &data, *merge)
            }
            Action::Task(t) => self.run_task(t),
            Action::InstallOrExtract(f) => {
                let p = self.existing_path(f);
                if p.to_string_lossy().to_ascii_lowercase().ends_with(".exe") {
                    return self.wine_exec(&p, "", None, None, &[]);
                }
                let dst = self.job.game_dir.join("drive_c").join(self.job.script.folder_name());
                let (cancel, log) = self.io_parts();
                archive::extract(&p, &dst, None, true, &Io { cancel: &cancel, log: &log })
            }
            Action::ExtractOrRun { file, dst, merge_dst } => {
                let src = self.existing_path(file);
                let out = self.path(dst.as_deref().unwrap_or("$CACHE"));
                {
                    let (cancel, log) = self.io_parts();
                    archive::extract(&src, &out, None, true, &Io { cancel: &cancel, log: &log })?;
                }
                let setup = std::fs::read_dir(&out)
                    .map_err(|e| e.to_string())?
                    .flatten()
                    .map(|e| e.path())
                    .find(|p| p.is_file() && p.to_string_lossy().to_ascii_lowercase().ends_with(".exe") && is_setup_program(p));
                if let Some(exe) = setup {
                    return self.wine_exec(&exe, "", None, None, &[]);
                }
                let target = self.path(merge_dst.as_deref().or(dst.as_deref()).unwrap_or("$GAMEDIR"));
                if target != out {
                    fsops::merge(&out, &target)?;
                }
                Ok(())
            }
            Action::Unsupported { name, reason } => Err(format!("{name}: {reason}")),
        }
    }

    /// A file the user picked (moving it would take it from where they keep
    /// it; it is copied instead).
    fn picked(&self, path: &Path) -> bool {
        self.job.provided.values().any(|p| p == path)
    }

    /// After a file id's file moved: steps after this one find it there.
    fn relink(&mut self, src: &str, to: Option<PathBuf>) {
        let key = src.trim().trim_start_matches('$').to_string();
        if let (true, Some(to)) = (self.files.contains_key(&key), to) {
            self.vars.set(key.clone(), to.to_string_lossy().into_owned());
            self.files.insert(key, to);
        }
    }

    /// The files a path names (a `*` / `?` pattern in its last part matches
    /// several).
    fn matching(&self, spec: &str) -> Result<Vec<PathBuf>, String> {
        let p = self.existing_path(spec);
        if p.exists() {
            return Ok(vec![p]);
        }
        let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        if name.contains(['*', '?']) {
            let dir = p.parent().map(Path::to_path_buf).unwrap_or_default();
            let mut found: Vec<PathBuf> = std::fs::read_dir(&dir)
                .map(|d| d.flatten().filter(|e| wildcard(&name, &e.file_name().to_string_lossy())).map(|e| e.path()).collect())
                .unwrap_or_default();
            found.sort();
            if !found.is_empty() {
                return Ok(found);
            }
        }
        Err(tr!("{path} does not exist", path = p.display()))
    }

    fn execute(&self, e: &crate::steps::Execute) -> Result<(), String> {
        let mut cmd = match (&e.command, &e.file) {
            (Some(command), _) => {
                let shell = crate::exec::which_any(&["bash", "sh"]).unwrap_or_else(|| "sh".into());
                let mut c = Command::new(shell);
                c.arg("-c").arg(self.sub(command.trim()));
                c
            }
            (None, Some(file)) => {
                let program = self.existing_path(file);
                if program.is_file() {
                    let _ = fsops::make_executable(&program);
                }
                let mut c = Command::new(&program);
                c.args(split_args(&self.sub(&e.args)));
                c
            }
            (None, None) => return Err(tr!("execute needs a file or a command").into()),
        };
        let cwd = e.working_dir.as_deref().map(|w| self.path(w)).filter(|p| p.is_dir()).unwrap_or_else(|| self.job.game_dir.clone());
        cmd.current_dir(cwd);
        cmd.envs(self.script_env());
        cmd.envs(e.env.iter().map(|(k, v)| (k.clone(), self.sub(v))));
        if e.terminal {
            self.log(tr!("This step asked for a terminal; its output is shown here instead."));
        }
        self.run(&mut cmd)
    }

    fn insert_disc(&mut self, requires: &str, message: Option<&str>) -> Result<(), String> {
        let requires = self.sub(requires);
        let found = mounted_discs(&self.job.home).into_iter().find(|d| fsops::fix_case(&d.join(&requires)).exists());
        if let Some(disc) = found {
            self.remember("DISC", disc.to_string_lossy().into_owned());
            return Ok(());
        }
        let message = message.map(|m| self.sub(m)).unwrap_or_else(|| tr!("Insert or mount the game disc, then select its folder.").into());
        let mut wrong = None;
        loop {
            match self.host.ask(Question::Disc { message: message.clone(), requires: requires.clone(), wrong: wrong.clone() }) {
                Some(Answer::Folder(dir)) if fsops::fix_case(&dir.join(&requires)).exists() => {
                    self.remember("DISC", dir.to_string_lossy().into_owned());
                    return Ok(());
                }
                Some(Answer::Folder(dir)) => wrong = Some(dir),
                _ => return Err(CANCELLED.into()),
            }
        }
    }
}

fn mib(bytes: u64) -> String {
    format!("{:.1} MB", bytes as f64 / 1_048_576.0)
}

/// Arguments of a command line (quotes respected; plain split if unbalanced).
pub(crate) fn split_args(s: &str) -> Vec<String> {
    shlex::split(s).unwrap_or_else(|| s.split_whitespace().map(str::to_string).collect())
}

/// `*` and `?` wildcards.
fn wildcard(pattern: &str, name: &str) -> bool {
    fn go(p: &[char], n: &[char]) -> bool {
        match (p.first(), n.first()) {
            (None, None) => true,
            (Some('*'), _) => go(&p[1..], n) || (!n.is_empty() && go(p, &n[1..])),
            (Some('?'), Some(_)) => go(&p[1..], &n[1..]),
            (Some(a), Some(b)) => a == b && go(&p[1..], &n[1..]),
            _ => false,
        }
    }
    let p: Vec<char> = pattern.chars().collect();
    let n: Vec<char> = name.chars().collect();
    go(&p, &n)
}

/// Whether a Windows program is an NSIS or Inno Setup installer.
fn is_setup_program(path: &Path) -> bool {
    use std::io::Read;
    let mut head = vec![0u8; 512 * 1024];
    let n = std::fs::File::open(path).and_then(|mut f| f.read(&mut head)).unwrap_or(0);
    let head = &head[..n];
    let has = |needle: &[u8]| head.windows(needle.len()).any(|w| w == needle);
    has(b"NullsoftInst") || has(b"Inno Setup")
}

/// Folders where discs and disc images get mounted.
fn mounted_discs(home: &str) -> Vec<PathBuf> {
    let user = std::env::var("USER").unwrap_or_default();
    let mut roots =
        vec![PathBuf::from("/run/media").join(&user), PathBuf::from("/media").join(&user), PathBuf::from("/media"), PathBuf::from("/mnt")];
    roots.retain(|r| r.is_dir() && r != Path::new(home));
    roots.iter().flat_map(|r| std::fs::read_dir(r).into_iter().flatten().flatten().map(|e| e.path()).filter(|p| p.is_dir())).collect()
}

/// The variables every script starts with; the script's own `variables`
/// come last and may replace them.
fn base_vars(job: &Job) -> Vars {
    let mut v = Vars::new();
    let user = std::env::var("USER").unwrap_or_default();
    v.set("GAMEDIR", job.game_dir.to_string_lossy());
    v.set("CACHE", job.cache_dir().to_string_lossy());
    v.set("HOME", job.home.clone());
    v.set("USER", user);
    v.set("INPUT", "");
    v.set("VERSION", job.script.version.clone());
    if let Some(dir) = &job.script.script_dir {
        v.set("SCRIPTDIR", dir.to_string_lossy());
    }
    for steam in [".local/share/Steam", ".steam/steam"] {
        let p = Path::new(&job.home).join(steam);
        if p.is_dir() {
            v.set("STEAM_DATA_DIR", p.to_string_lossy());
            break;
        }
    }
    if let Some((w, h)) = job.resolution {
        v.set("RESOLUTION", format!("{w}x{h}"));
        v.set("RESOLUTION_WIDTH", w.to_string());
        v.set("RESOLUTION_HEIGHT", h.to_string());
        v.set("RESOLUTION_WIDTH_HEX", format!("0x{w:x}"));
        v.set("RESOLUTION_HEIGHT_HEX", format!("0x{h:x}"));
    }
    if let Some(bin) = job.wine_version.as_ref().and_then(|w| w.get("bin")).and_then(Value::as_str) {
        v.set("WINEBIN", bin);
    }
    for (k, val) in &job.script.variables {
        v.set(k.clone(), val.clone());
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wildcards() {
        assert!(wildcard("setup_*.exe", "setup_game_1.0.exe"));
        assert!(wildcard("disk?.zip", "disk1.zip"));
        assert!(!wildcard("disk?.zip", "disk10.zip"));
        assert!(wildcard("*", ""));
    }

    #[test]
    fn arguments_keep_quotes() {
        assert_eq!(split_args("/S /D=\"C:\\Program Files\""), ["/S", "/D=C:\\Program Files"]);
        assert_eq!(split_args("a 'b"), ["a", "'b"]);
    }
}
