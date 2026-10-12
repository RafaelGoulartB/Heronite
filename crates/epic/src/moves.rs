//! Moving an installed game to another folder (`moveOnUnix` +
//! `changeGameInstallPath`), resumable.
//!
//! Moving is three steps: copy the files (rsync `--remove-source-files`),
//! remove the emptied source, then tell legendary where the game is now
//! (`legendary move <app> <new_parent> --skip-move`). A kill in between leaves
//! the game split between two folders with `installed.json` still pointing at
//! the old one, so a small journal in this app's own config folder
//! (`epic-move.json`, never inside Heroic's) records the move before it
//! touches anything. [`resume_interrupted_move`] finishes it at the next
//! start; starting the same move again by hand resumes it too.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use heronite_data::{json_store, proc, AppPaths, DataPaths};
use heronite_i18n::tr;
use serde::{Deserialize, Serialize};

use crate::cache_store::CacheStore;
use crate::cmd::Legendary;
use crate::library;
use crate::ops::OpResult;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Phase {
    /// Files are being moved (some may already be in the new folder).
    Copying,
    /// Every file moved; legendary still has to be told.
    Recording,
}

/// What `epic-move.json` holds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MoveJournal {
    pub app_name: String,
    /// The install folder being emptied.
    pub source: String,
    /// `<new_parent>/<name of source>`.
    pub destination: String,
    pub new_parent: String,
    pub phase: Phase,
}

/// What an interrupted move needs, from the state of both folders.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    /// Nothing was moved (the source is complete, the destination has no
    /// files): drop the journal.
    NeverStarted,
    /// Files are in both folders: run the copy again, then record.
    FinishCopy,
    /// Only the destination has files: record the new path.
    Record,
    /// Neither folder has files.
    Lost,
}

/// Which programs move the files. `rsync: None` falls back to `mv` / `cp`.
#[derive(Debug, Clone)]
pub struct Tools {
    pub rsync: Option<String>,
}

impl Tools {
    pub fn detect() -> Self {
        let has_rsync = proc::output(Command::new("rsync").arg("--version")).is_ok_and(|o| o.success());
        Self { rsync: has_rsync.then(|| "rsync".to_string()) }
    }
}

pub fn journal_path(data_paths: &DataPaths) -> PathBuf {
    AppPaths::for_data(data_paths).epic_move_journal()
}

fn read_journal(file: &Path) -> Option<MoveJournal> {
    match json_store::read::<Option<MoveJournal>>(file) {
        Ok(j) => j,
        Err(err) => {
            log::warn!("ignoring an unreadable move journal {}: {err:#}", file.display());
            None
        }
    }
}

fn write_journal(file: &Path, journal: &MoveJournal) -> anyhow::Result<()> {
    json_store::write(file, journal)
}

fn clear_journal(file: &Path) {
    if let Err(err) = fs::remove_file(file) {
        if err.kind() != std::io::ErrorKind::NotFound {
            log::warn!("removing {}: {err}", file.display());
        }
    }
}

/// The interrupted move to finish, if any. A journal that makes no sense is
/// deleted (it is never acted on).
pub fn pending_move(data_paths: &DataPaths) -> Option<MoveJournal> {
    let file = journal_path(data_paths);
    let journal = read_journal(&file)?;
    if let Err(why) = validate(&journal) {
        log::warn!("dropping an invalid move journal: {why}");
        clear_journal(&file);
        return None;
    }
    Some(journal)
}

/// Sanity checks before anything is moved or deleted on the journal's word.
fn validate(j: &MoveJournal) -> Result<(), String> {
    let (source, destination, parent) = (Path::new(&j.source), Path::new(&j.destination), Path::new(&j.new_parent));
    if j.app_name.is_empty() {
        return Err("no game".into());
    }
    if !source.is_absolute() || !destination.is_absolute() || !parent.is_absolute() {
        return Err("paths must be absolute".into());
    }
    let Some(name) = source.file_name() else { return Err("the source is not a folder".into()) };
    if source.parent().is_none() || source.components().count() < 3 {
        return Err("the source is too close to the root".into());
    }
    if destination != parent.join(name) {
        return Err("the destination is not inside the new folder".into());
    }
    if destination == source || parent.starts_with(source) || source.starts_with(destination) {
        return Err("the folders contain each other".into());
    }
    Ok(())
}

/// `true` when anything but folders is inside `dir`.
fn has_files(dir: &Path) -> bool {
    let Ok(entries) = fs::read_dir(dir) else { return false };
    entries.flatten().any(|e| match e.file_type() {
        Ok(t) if t.is_dir() => has_files(&e.path()),
        _ => true,
    })
}

/// Removes `dir` and the folders below it when no file is inside (empty
/// folders only: `remove_dir` refuses anything else).
fn remove_empty_tree(dir: &Path) {
    if has_files(dir) {
        return;
    }
    if let Ok(entries) = fs::read_dir(dir) {
        for e in entries.flatten() {
            if e.file_type().is_ok_and(|t| t.is_dir()) {
                remove_empty_tree(&e.path());
            }
        }
    }
    let _ = fs::remove_dir(dir);
}

/// Reads the state of both folders.
pub fn decide(j: &MoveJournal) -> Decision {
    match (has_files(Path::new(&j.source)), has_files(Path::new(&j.destination))) {
        (true, false) => Decision::NeverStarted,
        (true, true) => Decision::FinishCopy,
        (false, true) => Decision::Record,
        (false, false) => Decision::Lost,
    }
}

/// `moveOnUnix` + `changeGameInstallPath`: moves the install folder into
/// `new_parent` and tells legendary where it is now.
pub fn move_install(data_paths: &DataPaths, legendary: &Legendary, app_name: &str, install_path: &str, new_parent: &str) -> OpResult {
    move_install_with(data_paths, legendary, &Tools::detect(), app_name, install_path, new_parent)
}

pub fn move_install_with(
    data_paths: &DataPaths,
    legendary: &Legendary,
    tools: &Tools,
    app_name: &str,
    install_path: &str,
    new_parent: &str,
) -> OpResult {
    let source = PathBuf::from(install_path);
    let parent = PathBuf::from(new_parent);
    if install_path.is_empty() || !source.is_dir() {
        return OpResult::err(tr!("No install path found"));
    }
    if parent.starts_with(&source) {
        return OpResult::err(tr!(
            "New install path ({new_parent}) cannot be inside the current install path ({install_path}).",
            new_parent,
            install_path
        ));
    }
    let Some(name) = source.file_name() else { return OpResult::err(tr!("Invalid install path")) };
    let destination = parent.join(name);
    let journal = MoveJournal {
        app_name: app_name.into(),
        source: install_path.into(),
        destination: destination.to_string_lossy().into_owned(),
        new_parent: new_parent.into(),
        phase: Phase::Copying,
    };
    if let Err(why) = validate(&journal) {
        return OpResult::err(tr!("Can't move {install_path}: {error}", install_path, error = why));
    }

    // A journal of the same move: this is a retry, finish it. Another pending
    // move has to finish first (unless it turns out to be a dead one).
    let file = journal_path(data_paths);
    let mut resuming = false;
    if let Some(old) = read_journal(&file) {
        if old.app_name == journal.app_name && old.source == journal.source && old.destination == journal.destination {
            resuming = true;
        } else if validate(&old).is_ok() && matches!(decide(&old), Decision::FinishCopy | Decision::Record) {
            return OpResult::err(tr!(
                "The move of {game} was interrupted and has to finish first (it resumes when the launcher starts)",
                game = old.app_name
            ));
        } else {
            clear_journal(&file);
        }
    }
    let merge = resuming || has_files(&destination);
    if destination.exists() && has_files(&destination) && !resuming {
        return OpResult::err(tr!("{path} already exists and is not empty", path = destination.display()));
    }
    if destination.is_file() {
        return OpResult::err(tr!("{path} already exists and is not empty", path = destination.display()));
    }
    if let Err(err) = fs::create_dir_all(&parent) {
        return OpResult::err(format!("{err}"));
    }
    // Written before the first file moves: after this the move can be finished.
    if let Err(err) = write_journal(&file, &journal) {
        return OpResult::err(tr!("Could not write the move journal: {error}", error = format!("{err:#}")));
    }
    run_copy_and_record(data_paths, legendary, tools, &journal, merge)
}

/// The copy phase and what follows it.
fn run_copy_and_record(data_paths: &DataPaths, legendary: &Legendary, tools: &Tools, j: &MoveJournal, merge: bool) -> OpResult {
    let (source, destination) = (Path::new(&j.source), Path::new(&j.destination));
    if let Err(message) = move_dir(tools, source, destination, merge) {
        if !has_files(destination) {
            // Nothing moved: the failure left the game where it was.
            remove_empty_tree(destination);
            clear_journal(&journal_path(data_paths));
        }
        return OpResult::err(message);
    }
    finish_move(data_paths, legendary, j)
}

/// Every file is in the new folder: drop the old one and record the path.
fn finish_move(data_paths: &DataPaths, legendary: &Legendary, j: &MoveJournal) -> OpResult {
    let file = journal_path(data_paths);
    let source = Path::new(&j.source);
    let recording = MoveJournal { phase: Phase::Recording, ..j.clone() };
    if let Err(err) = write_journal(&file, &recording) {
        log::warn!("updating the move journal: {err:#}");
    }
    // Only empty folders may be left in the source now.
    if has_files(source) {
        return OpResult::err(tr!("Some files could not be moved out of {path}", path = source.display()));
    }
    if source.exists() {
        if let Err(e) = fs::remove_dir_all(source) {
            return OpResult::err(tr!("Copied, but the old folder could not be removed: {error}", error = e));
        }
    }
    let args = ["move", j.app_name.as_str(), j.new_parent.as_str(), "--skip-move"];
    match legendary.run(&args, &[]) {
        Err(err) => OpResult::err(format!("{err:#}")),
        Ok(out) if out.success() => {
            let _ = library::write_library(data_paths);
            clear_journal(&file);
            OpResult::ok(j.destination.clone())
        }
        Ok(out) => OpResult::err(tr!("Moved, but legendary could not record the new path: {error}", error = out.error_lines())),
    }
}

/// Finishes the move an earlier run was interrupted in. `None`: no move was
/// pending. Blocking; run it like the other Epic operations so legendary is
/// not used at the same time by the download queue.
pub fn resume_interrupted_move(data_paths: &DataPaths, legendary: &Legendary) -> Option<OpResult> {
    resume_interrupted_move_with(data_paths, legendary, &Tools::detect())
}

pub fn resume_interrupted_move_with(data_paths: &DataPaths, legendary: &Legendary, tools: &Tools) -> Option<OpResult> {
    let j = pending_move(data_paths)?;
    let file = journal_path(data_paths);
    let title = game_title(data_paths, &j.app_name).unwrap_or_else(|| j.app_name.clone());
    let decision = decide(&j);
    log::info!("resuming the interrupted move of {} ({:?}, {decision:?})", j.app_name, j.phase);
    let result = match decision {
        Decision::NeverStarted => {
            remove_empty_tree(Path::new(&j.destination));
            clear_journal(&file);
            return Some(OpResult::ok(tr!("The move of {title} never started; nothing was changed", title)));
        }
        Decision::Lost => {
            clear_journal(&file);
            return Some(OpResult::err(tr!(
                "The move of {title} was interrupted and neither {source} nor {destination} has files any more",
                title,
                source = j.source,
                destination = j.destination
            )));
        }
        Decision::FinishCopy => run_copy_and_record(data_paths, legendary, tools, &j, true),
        Decision::Record => finish_move(data_paths, legendary, &j),
    };
    Some(if result.ok { OpResult::ok(tr!("Finished moving {title}", title)) } else { result })
}

fn game_title(data_paths: &DataPaths, app_name: &str) -> Option<String> {
    let library = CacheStore::library(data_paths).get("library")?;
    library
        .as_array()?
        .iter()
        .find(|g| g.get("app_name").and_then(|v| v.as_str()) == Some(app_name))?
        .get("title")?
        .as_str()
        .map(str::to_string)
}

/// Moves the contents of `source` into `destination` (which may exist and be
/// empty, or hold part of the files when `merge`). With rsync it is resumable
/// and `source` keeps its emptied folders; without it `mv` / `cp` are used and
/// `source` is gone afterwards.
fn move_dir(tools: &Tools, source: &Path, destination: &Path, merge: bool) -> Result<(), String> {
    let run = |cmd: &mut Command| -> Result<(), String> {
        let out = proc::output(cmd).map_err(|e| e.to_string())?;
        if out.success() {
            Ok(())
        } else {
            let why = out.stderr_text().trim().to_string();
            Err(if why.is_empty() { format!("{:?} exited with {:?}", cmd.get_program(), out.code) } else { why })
        }
    };
    if let Some(rsync) = &tools.rsync {
        let origin = format!("{}/", source.display());
        return run(Command::new(rsync).args(["--archive", "--no-human-readable", "--remove-source-files"]).arg(&origin).arg(destination));
    }
    if merge {
        // `mv` would nest the folder inside a destination that already exists.
        let origin = format!("{}/.", source.display());
        run(Command::new("cp").arg("-a").arg(&origin).arg(format!("{}/", destination.display())))?;
        return fs::remove_dir_all(source).map_err(|e| tr!("Copied, but the old folder could not be removed: {error}", error = e));
    }
    if destination.is_dir() {
        // `mv src dest` puts src inside an existing folder: an empty one is replaced instead.
        fs::remove_dir(destination).map_err(|e| format!("{} already exists: {e}", destination.display()))?;
    }
    run(Command::new("mv").arg("-f").arg(source).arg(destination))
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::cmd::testing::{calls, fake};
    use std::os::unix::fs::PermissionsExt;

    struct Fx {
        dir: tempfile::TempDir,
        data_paths: DataPaths,
        lg: Legendary,
        source: PathBuf,
        parent: PathBuf,
    }

    impl Fx {
        fn new(legendary_body: &str) -> Self {
            let dir = tempfile::tempdir().unwrap();
            let data_paths = DataPaths::at(dir.path().join("h"));
            fs::create_dir_all(data_paths.legendary_config_dir()).unwrap();
            let lg = fake(dir.path(), legendary_body);
            let source = dir.path().join("old/Control");
            fs::create_dir_all(source.join("sub")).unwrap();
            fs::write(source.join("a.txt"), "a").unwrap();
            fs::write(source.join("sub/b.txt"), "b").unwrap();
            let parent = dir.path().join("new");
            Self { dir, data_paths, lg, source, parent }
        }

        fn dest(&self) -> PathBuf {
            self.parent.join("Control")
        }

        fn journal(&self) -> Option<MoveJournal> {
            read_journal(&journal_path(&self.data_paths))
        }

        fn mv(&self, tools: &Tools) -> OpResult {
            move_install_with(&self.data_paths, &self.lg, tools, "Calluna", self.source.to_str().unwrap(), self.parent.to_str().unwrap())
        }

        fn resume(&self, tools: &Tools) -> Option<OpResult> {
            resume_interrupted_move_with(&self.data_paths, &self.lg, tools)
        }

        fn legendary_moves(&self) -> usize {
            calls(self.dir.path()).iter().filter(|c| c.starts_with("move ")).count()
        }

        /// A stand-in for rsync: copies everything, then removes the files of the origin
        /// (what `--remove-source-files` does), like the real one but without needing it.
        fn fake_rsync(&self, name: &str, body: &str) -> Tools {
            let bin = self.dir.path().join(name);
            fs::write(&bin, format!("#!/bin/sh\n{body}\n")).unwrap();
            fs::set_permissions(&bin, fs::Permissions::from_mode(0o755)).unwrap();
            Tools { rsync: Some(bin.to_string_lossy().into_owned()) }
        }

        fn complete_rsync(&self) -> Tools {
            // $4 = "<source>/", $5 = destination
            self.fake_rsync("rsync-ok", "mkdir -p \"$5\" && cp -a \"$4.\" \"$5/\" && find \"$4\" -type f -delete")
        }

        /// Moves only `a.txt`, then dies like a killed rsync.
        fn dying_rsync(&self) -> Tools {
            self.fake_rsync("rsync-dies", "mkdir -p \"$5/sub\" && mv \"$4a.txt\" \"$5/a.txt\" && exit 20")
        }
    }

    const NO_RSYNC: Tools = Tools { rsync: None };

    #[test]
    fn a_clean_move_leaves_no_journal_and_records_the_path() {
        let fx = Fx::new("true");
        let r = fx.mv(&fx.complete_rsync());
        assert!(r.ok, "{}", r.message);
        assert_eq!(r.message, fx.dest().to_string_lossy());
        assert!(fx.dest().join("sub/b.txt").exists() && fx.dest().join("a.txt").exists());
        assert!(!fx.source.exists());
        assert!(fx.journal().is_none());
        assert_eq!(calls(fx.dir.path()), [format!("move Calluna {} --skip-move", fx.parent.display())]);
    }

    #[test]
    fn the_journal_is_written_before_the_first_file_moves_and_names_the_phase() {
        // The fake rsync reads the journal while it runs.
        let fx = Fx::new("true");
        let journal = journal_path(&fx.data_paths);
        let tools = fx
            .fake_rsync("rsync-peek", &format!("cp \"{}\" \"{}\"; exit 20", journal.display(), fx.dir.path().join("peek.json").display()));
        let r = fx.mv(&tools);
        assert!(!r.ok);
        let peek: MoveJournal = serde_json::from_str(&fs::read_to_string(fx.dir.path().join("peek.json")).unwrap()).unwrap();
        assert_eq!(peek.phase, Phase::Copying);
        assert_eq!(peek.app_name, "Calluna");
        assert_eq!(peek.source, fx.source.to_string_lossy());
        assert_eq!(peek.destination, fx.dest().to_string_lossy());
        assert_eq!(peek.new_parent, fx.parent.to_string_lossy());
        // The journal lives outside the data folder.
        assert!(!journal.starts_with(&fx.data_paths.root));
    }

    #[test]
    fn a_failure_before_anything_moved_is_clean() {
        let fx = Fx::new("true");
        let tools = fx.fake_rsync("rsync-fails", "mkdir -p \"$5/sub\"; echo 'no space left' >&2; exit 11");
        let r = fx.mv(&tools);
        assert!(!r.ok && r.message.contains("no space left"), "{}", r.message);
        assert!(fx.journal().is_none(), "nothing was touched: no journal");
        assert!(!fx.dest().exists(), "the empty folders rsync made are removed");
        assert!(fx.source.join("a.txt").exists() && fx.source.join("sub/b.txt").exists());
        assert_eq!(fx.legendary_moves(), 0);
    }

    #[test]
    fn an_interrupted_copy_is_finished_by_the_resume() {
        let fx = Fx::new("true");
        let r = fx.mv(&fx.dying_rsync());
        assert!(!r.ok);
        // Split game, journal kept.
        assert!(fx.dest().join("a.txt").exists() && fx.source.join("sub/b.txt").exists());
        assert_eq!(fx.journal().unwrap().phase, Phase::Copying);
        assert_eq!(decide(&fx.journal().unwrap()), Decision::FinishCopy);
        assert_eq!(fx.legendary_moves(), 0);

        let r = fx.resume(&fx.complete_rsync()).unwrap();
        assert!(r.ok, "{}", r.message);
        assert_eq!(r.message, "Finished moving Calluna");
        assert!(fx.dest().join("a.txt").exists() && fx.dest().join("sub/b.txt").exists());
        assert!(!fx.source.exists());
        assert!(fx.journal().is_none());
        assert_eq!(fx.legendary_moves(), 1);
        // Nothing left to do afterwards.
        assert!(fx.resume(&fx.complete_rsync()).is_none());
    }

    #[test]
    fn retrying_the_same_move_by_hand_resumes_it() {
        let fx = Fx::new("true");
        assert!(!fx.mv(&fx.dying_rsync()).ok);
        // Without the journal this would be "already exists and is not empty".
        let r = fx.mv(&fx.complete_rsync());
        assert!(r.ok, "{}", r.message);
        assert!(fx.dest().join("sub/b.txt").exists() && !fx.source.exists() && fx.journal().is_none());
    }

    #[test]
    fn without_a_journal_a_non_empty_destination_is_still_refused() {
        let fx = Fx::new("true");
        fs::create_dir_all(fx.dest()).unwrap();
        fs::write(fx.dest().join("f"), "x").unwrap();
        let r = fx.mv(&fx.complete_rsync());
        assert!(!r.ok && r.message.contains("not empty"), "{}", r.message);
        assert!(fx.journal().is_none() && fx.source.join("a.txt").exists());
    }

    #[test]
    fn a_failed_legendary_call_keeps_the_recording_phase_for_the_resume() {
        let fx = Fx::new("if [ \"$1\" = move ] && [ ! -f \"$(dirname \"$0\")/ok\" ]; then echo 'ERROR: busy' >&2; exit 1; fi");
        let r = fx.mv(&fx.complete_rsync());
        assert!(!r.ok && r.message.contains("could not record"), "{}", r.message);
        assert!(!fx.source.exists() && fx.dest().join("a.txt").exists());
        let j = fx.journal().unwrap();
        assert_eq!(j.phase, Phase::Recording);
        assert_eq!(decide(&j), Decision::Record);
        // legendary works now.
        fs::write(fx.dir.path().join("ok"), "").unwrap();
        let r = fx.resume(&NO_RSYNC).unwrap();
        assert!(r.ok, "{}", r.message);
        assert!(fx.journal().is_none());
        assert_eq!(fx.legendary_moves(), 2);
    }

    #[test]
    fn a_move_that_never_started_just_drops_the_journal() {
        let fx = Fx::new("true");
        // rsync had only created the (empty) destination folders when it was killed.
        fs::create_dir_all(fx.dest().join("sub")).unwrap();
        let j = MoveJournal {
            app_name: "Calluna".into(),
            source: fx.source.to_string_lossy().into(),
            destination: fx.dest().to_string_lossy().into(),
            new_parent: fx.parent.to_string_lossy().into(),
            phase: Phase::Copying,
        };
        write_journal(&journal_path(&fx.data_paths), &j).unwrap();
        assert_eq!(decide(&j), Decision::NeverStarted);
        let r = fx.resume(&fx.complete_rsync()).unwrap();
        assert!(r.ok && r.message.contains("never started"), "{}", r.message);
        assert!(fx.journal().is_none());
        assert!(!fx.dest().exists());
        assert!(fx.source.join("sub/b.txt").exists(), "the source is untouched");
        assert_eq!(fx.legendary_moves(), 0);
    }

    #[test]
    fn a_move_with_nothing_left_is_reported_and_dropped() {
        let fx = Fx::new("true");
        let j = MoveJournal {
            app_name: "Calluna".into(),
            source: fx.dir.path().join("gone/Control").to_string_lossy().into(),
            destination: fx.dir.path().join("also-gone/Control").to_string_lossy().into(),
            new_parent: fx.dir.path().join("also-gone").to_string_lossy().into(),
            phase: Phase::Copying,
        };
        write_journal(&journal_path(&fx.data_paths), &j).unwrap();
        let r = fx.resume(&NO_RSYNC).unwrap();
        assert!(!r.ok && r.message.contains("neither"), "{}", r.message);
        assert!(fx.journal().is_none());
    }

    #[test]
    fn journals_that_make_no_sense_are_dropped_without_touching_anything() {
        let fx = Fx::new("true");
        for (source, destination, parent) in [
            ("/", "/x", "/x"),
            ("relative/Control", "/n/Control", "/n"),
            ("/home", "/n/home", "/n"),
            (fx.source.to_str().unwrap(), "/elsewhere/Other", "/elsewhere"),
            (fx.source.to_str().unwrap(), fx.source.to_str().unwrap(), fx.source.parent().unwrap().to_str().unwrap()),
        ] {
            let j = MoveJournal {
                app_name: "Calluna".into(),
                source: source.into(),
                destination: destination.into(),
                new_parent: parent.into(),
                phase: Phase::Recording,
            };
            write_journal(&journal_path(&fx.data_paths), &j).unwrap();
            assert!(fx.resume(&NO_RSYNC).is_none(), "{source}");
            assert!(fx.journal().is_none());
        }
        assert!(fx.source.join("a.txt").exists());
        // Garbage in the file is ignored too.
        fs::write(journal_path(&fx.data_paths), "{not json").unwrap();
        assert!(fx.resume(&NO_RSYNC).is_none());
    }

    #[test]
    fn another_unfinished_move_blocks_a_new_one_but_a_dead_one_does_not() {
        let fx = Fx::new("true");
        assert!(!fx.mv(&fx.dying_rsync()).ok);
        let other = fx.dir.path().join("other/Hades");
        fs::create_dir_all(&other).unwrap();
        fs::write(other.join("f"), "x").unwrap();
        let r = move_install_with(
            &fx.data_paths,
            &fx.lg,
            &NO_RSYNC,
            "Other",
            other.to_str().unwrap(),
            fx.dir.path().join("n2").to_str().unwrap(),
        );
        assert!(!r.ok && r.message.contains("Calluna"), "{}", r.message);
        assert!(other.join("f").exists());

        // Once the first move is gone for good, its journal no longer blocks.
        fs::remove_dir_all(&fx.source).unwrap();
        fs::remove_dir_all(fx.dest()).unwrap();
        let r = move_install_with(
            &fx.data_paths,
            &fx.lg,
            &NO_RSYNC,
            "Other",
            other.to_str().unwrap(),
            fx.dir.path().join("n2").to_str().unwrap(),
        );
        assert!(r.ok, "{}", r.message);
    }

    #[test]
    fn mv_does_not_nest_inside_an_existing_empty_destination() {
        let fx = Fx::new("true");
        fs::create_dir_all(fx.dest()).unwrap();
        let r = fx.mv(&NO_RSYNC);
        assert!(r.ok, "{}", r.message);
        assert!(fx.dest().join("a.txt").exists() && fx.dest().join("sub/b.txt").exists());
        assert!(!fx.dest().join("Control").exists(), "the folder was nested");
        assert!(!fx.source.exists() && fx.journal().is_none());
    }

    #[test]
    fn the_cp_fallback_merges_into_a_partly_filled_destination() {
        let fx = Fx::new("true");
        assert!(!fx.mv(&fx.dying_rsync()).ok);
        let r = fx.resume(&NO_RSYNC).unwrap();
        assert!(r.ok, "{}", r.message);
        assert!(fx.dest().join("a.txt").exists() && fx.dest().join("sub/b.txt").exists());
        assert!(!fx.dest().join("Control").exists());
        assert!(!fx.source.exists() && fx.journal().is_none());
    }

    #[test]
    fn real_rsync_moves_a_temp_folder_when_installed() {
        let tools = Tools::detect();
        if tools.rsync.is_none() {
            return;
        }
        let fx = Fx::new("true");
        let r = fx.mv(&tools);
        assert!(r.ok, "{}", r.message);
        assert!(fx.dest().join("sub/b.txt").exists() && !fx.source.exists() && fx.journal().is_none());
    }

    #[test]
    fn app_paths_of_a_fixture_never_point_at_the_real_config() {
        let data_paths = DataPaths::at("/tmp/heronite-fixture-x");
        let app = AppPaths::for_data(&data_paths);
        assert_eq!(app.epic_move_journal(), PathBuf::from("/tmp/heronite-fixture-x.heronite/config/epic-move.json"));
    }
}
