//! [`install`] against a fake source.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{bail, Result};
use heronite_data::proc::Cancel;
use parking_lot::Mutex;

use crate::*;

/// The bytes of chunk `id`: `size` bytes that differ from chunk to chunk.
fn bytes_of(id: &str, size: u64) -> Vec<u8> {
    let seed = id.bytes().fold(7u32, |h, b| h.wrapping_mul(31).wrapping_add(b as u32));
    (0..size as u32).map(|i| (seed.wrapping_add(i.wrapping_mul(2654435761)) >> 13) as u8).collect()
}

#[derive(Default)]
struct Fake {
    fetches: Mutex<HashMap<String, usize>>,
    /// Chunks that fail this many times before they work.
    flaky: Mutex<HashMap<String, usize>>,
    /// Chunks that never work.
    broken: Vec<String>,
    /// Chunks that come back with other bytes.
    wrong: Vec<String>,
    /// Requested once this many chunks were fetched.
    cancel_after: Option<(usize, Arc<Cancel>)>,
    delay: Duration,
}

impl Fake {
    fn count(&self, id: &str) -> usize {
        self.fetches.lock().get(id).copied().unwrap_or(0)
    }

    fn total(&self) -> usize {
        self.fetches.lock().values().sum()
    }
}

impl Source for Fake {
    fn fetch(&self, chunk: &Chunk, _cancel: &Cancel) -> Result<Vec<u8>> {
        std::thread::sleep(self.delay);
        if self.broken.contains(&chunk.id) {
            bail!("HTTP 404");
        }
        if let Some(left) = self.flaky.lock().get_mut(&chunk.id).filter(|n| **n > 0) {
            *left -= 1;
            bail!("connection reset");
        }
        let n = {
            let mut f = self.fetches.lock();
            *f.entry(chunk.id.clone()).or_default() += 1;
            f.values().sum::<usize>()
        };
        if let Some((after, cancel)) = &self.cancel_after {
            if n >= *after {
                cancel.request();
            }
        }
        let mut data = bytes_of(&chunk.id, chunk.size);
        if self.wrong.contains(&chunk.id) {
            data.reverse();
        }
        Ok(data)
    }
}

/// A plan builder: chunks by name, files by their parts.
#[derive(Default)]
struct Build {
    plan: Plan,
    ids: HashMap<String, usize>,
}

impl Build {
    fn chunk(&mut self, id: &str, size: u64) -> usize {
        let plan = &mut self.plan;
        *self.ids.entry(id.to_string()).or_insert_with(|| {
            plan.chunks.push(Chunk { id: id.to_string(), download_size: size / 2, size });
            plan.chunks.len() - 1
        })
    }

    /// A file of `(chunk, offset, size)` pieces; its hash from those bytes.
    fn file(&mut self, path: &str, pieces: &[(&str, u64, u64)]) -> Vec<u8> {
        let mut content = Vec::new();
        let mut parts = Vec::new();
        for &(id, offset, size) in pieces {
            let c = self.chunk(id, 64);
            content.extend_from_slice(&bytes_of(id, 64)[offset as usize..(offset + size) as usize]);
            parts.push(Part::Chunk { chunk: c, offset, size });
        }
        self.plan.files.push(FileTask {
            path: path.into(),
            size: content.len() as u64,
            parts,
            hash: Some(sha1(&content)),
            ..Default::default()
        });
        content
    }
}

fn sha1(bytes: &[u8]) -> Hash {
    Hash::Sha1([0; 20]).of_reader(&mut &bytes[..]).unwrap()
}

fn opts(workers: usize) -> Options {
    Options { workers, ..Default::default() }
}

fn run(root: &Path, plan: &Plan, source: &Fake, opts: &Options, cancel: &Cancel) -> (Result<Outcome>, Vec<Progress>) {
    let mut seen = Vec::new();
    let out = install(root, plan, source, opts, cancel, &mut |p| seen.push(p.clone()));
    (out, seen)
}

#[test]
fn writes_files_from_shared_chunks() {
    let dir = tempfile::tempdir().unwrap();
    let mut b = Build::default();
    let a = b.file("bin/a.exe", &[("c1", 0, 64), ("c2", 0, 10)]);
    let c = b.file("data/b.pak", &[("c2", 10, 54), ("c1", 32, 32), ("c3", 0, 1)]);
    b.plan.files[0].executable = true;
    b.plan.files.push(FileTask { path: "empty".into(), ..Default::default() });
    b.plan.dirs.push("saves".into());
    let fake = Fake::default();
    let (out, seen) = run(dir.path(), &b.plan, &fake, &opts(3), &Cancel::new());
    assert_eq!(out.unwrap(), Outcome::Done);
    assert_eq!(std::fs::read(dir.path().join("bin/a.exe")).unwrap(), a);
    assert_eq!(std::fs::read(dir.path().join("data/b.pak")).unwrap(), c);
    assert_eq!(std::fs::read(dir.path().join("empty")).unwrap(), b"");
    assert!(dir.path().join("saves").is_dir());
    // Every chunk once, however many files use it.
    assert_eq!((fake.count("c1"), fake.count("c2"), fake.count("c3")), (1, 1, 1));
    let last = seen.last().unwrap();
    assert_eq!((last.written, last.write_total, last.files_done, last.files_total), (161, 161, 3, 3));
    assert_eq!((last.downloaded, last.download_total), (96, 96));
    assert_eq!(last.percent(), 100.);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(dir.path().join("bin/a.exe")).unwrap().permissions().mode();
        assert_eq!(mode & 0o111, 0o111);
    }
    assert!(!dir.path().join("bin/a.exe.heronite-part").exists());
}

#[test]
fn an_update_copies_unchanged_data_from_the_old_files() {
    let dir = tempfile::tempdir().unwrap();
    let old_a = bytes_of("old-a", 100);
    let old_b = bytes_of("old-b", 50);
    std::fs::write(dir.path().join("a"), &old_a).unwrap();
    std::fs::write(dir.path().join("b"), &old_b).unwrap();
    std::fs::write(dir.path().join("gone"), b"x").unwrap();
    let mut b = Build::default();
    let new = b.chunk("new", 64);
    // `a` keeps its start and takes the end of `b`; `b` changes too and keeps
    // its own start. `b` must not be replaced before `a` read it.
    let mut a_bytes = old_a[..60].to_vec();
    a_bytes.extend_from_slice(&old_b[30..]);
    a_bytes.extend_from_slice(&bytes_of("new", 64)[..4]);
    let mut b_bytes = old_b[..10].to_vec();
    b_bytes.extend_from_slice(&bytes_of("new", 64)[4..]);
    b.plan.files.push(FileTask {
        path: "b".into(),
        size: b_bytes.len() as u64,
        parts: vec![Part::Local { path: "b".into(), offset: 0, size: 10 }, Part::Chunk { chunk: new, offset: 4, size: 60 }],
        hash: Some(sha1(&b_bytes)),
        ..Default::default()
    });
    b.plan.files.push(FileTask {
        path: "a".into(),
        size: a_bytes.len() as u64,
        parts: vec![
            Part::Local { path: "a".into(), offset: 0, size: 60 },
            Part::Local { path: "b".into(), offset: 30, size: 20 },
            Part::Chunk { chunk: new, offset: 0, size: 4 },
        ],
        hash: Some(sha1(&a_bytes)),
        ..Default::default()
    });
    b.plan.remove.push("gone".into());
    let fake = Fake::default();
    let (out, _) = run(dir.path(), &b.plan, &fake, &opts(2), &Cancel::new());
    assert_eq!(out.unwrap(), Outcome::Done);
    assert_eq!(std::fs::read(dir.path().join("a")).unwrap(), a_bytes);
    assert_eq!(std::fs::read(dir.path().join("b")).unwrap(), b_bytes);
    assert!(!dir.path().join("gone").exists());
    assert_eq!(fake.total(), 1);
}

#[test]
fn removing_files_also_removes_folders_left_empty() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("x/y")).unwrap();
    std::fs::write(dir.path().join("x/y/old"), b"1").unwrap();
    std::fs::write(dir.path().join("x/keep"), b"1").unwrap();
    let plan = Plan { remove: vec!["x/y/old".into(), "never/was".into()], ..Default::default() };
    let (out, _) = run(dir.path(), &plan, &Fake::default(), &opts(1), &Cancel::new());
    assert_eq!(out.unwrap(), Outcome::Done);
    assert!(!dir.path().join("x/y").exists());
    assert!(dir.path().join("x/keep").exists());
}

#[test]
fn a_cancelled_run_resumes_where_it_stopped() {
    let dir = tempfile::tempdir().unwrap();
    let mut b = Build::default();
    let mut contents = Vec::new();
    for i in 0..20 {
        let id = format!("c{i}");
        contents.push(b.file(&format!("f{i}"), &[(id.as_str(), 0, 64)]));
    }
    let state = dir.path().join("state/journal");
    let options = Options { workers: 1, state_file: Some(state.clone()), ..Default::default() };
    let cancel = Cancel::new();
    let fake = Fake { cancel_after: Some((8, cancel.clone())), delay: Duration::from_millis(5), ..Default::default() };
    let (out, _) = run(&dir.path().join("game"), &b.plan, &fake, &options, &cancel);
    assert_eq!(out.unwrap(), Outcome::Cancelled);
    assert!(state.exists());
    let first = fake.total();
    assert!(first < 20, "{first}");
    let leftovers: Vec<_> = std::fs::read_dir(dir.path().join("game"))
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name().to_string_lossy().ends_with(".heronite-part"))
        .collect();
    assert!(leftovers.is_empty(), "{leftovers:?}");

    let again = Fake::default();
    let (out, seen) = run(&dir.path().join("game"), &b.plan, &again, &options, &Cancel::new());
    assert_eq!(out.unwrap(), Outcome::Done);
    for (i, c) in contents.iter().enumerate() {
        assert_eq!(&std::fs::read(dir.path().join(format!("game/f{i}"))).unwrap(), c);
    }
    let last = seen.last().unwrap();
    assert!(last.resumed > 0, "{last:?}");
    assert_eq!(last.resumed + last.written, 20 * 64);
    assert!(again.total() <= 20 - (last.resumed as usize / 64));
    assert!(!state.exists());
}

#[test]
fn a_resumed_run_checks_what_the_journal_kept() {
    let dir = tempfile::tempdir().unwrap();
    let mut b = Build::default();
    let mut contents = Vec::new();
    for i in 0..20 {
        let id = format!("c{i}");
        contents.push(b.file(&format!("f{i}"), &[(id.as_str(), 0, 64)]));
    }
    let state = dir.path().join("journal");
    let options = Options { workers: 1, state_file: Some(state.clone()), ..Default::default() };
    let cancel = Cancel::new();
    let fake = Fake { cancel_after: Some((8, cancel.clone())), delay: Duration::from_millis(5), ..Default::default() };
    let game = dir.path().join("game");
    let (out, _) = run(&game, &b.plan, &fake, &options, &cancel);
    assert_eq!(out.unwrap(), Outcome::Cancelled);
    // A kept file lost its data (a crash before it reached the disk): it is
    // written again.
    assert!(game.join("f0").exists());
    std::fs::write(game.join("f0"), vec![0u8; 64]).unwrap();
    let (out, _) = run(&game, &b.plan, &Fake::default(), &options, &Cancel::new());
    assert_eq!(out.unwrap(), Outcome::Done);
    for (i, c) in contents.iter().enumerate() {
        assert_eq!(&std::fs::read(game.join(format!("f{i}"))).unwrap(), c);
    }

    // The same plan for another folder does not take a folder's journal.
    let cancel = Cancel::new();
    let fake = Fake { cancel_after: Some((8, cancel.clone())), delay: Duration::from_millis(5), ..Default::default() };
    let (out, _) = run(&dir.path().join("one"), &b.plan, &fake, &options, &cancel);
    assert_eq!(out.unwrap(), Outcome::Cancelled);
    let (out, seen) = run(&dir.path().join("other"), &b.plan, &Fake::default(), &options, &Cancel::new());
    assert_eq!(out.unwrap(), Outcome::Done);
    assert_eq!(seen.last().unwrap().resumed, 0);
    assert!(!state.exists());
}

#[test]
fn a_rename_cut_short_is_told_by_the_disk() {
    let dir = tempfile::tempdir().unwrap();
    let mut b = Build::default();
    let content = b.file("f", &[("c", 0, 64)]);
    let root = dir.path().join("game");
    std::fs::create_dir_all(&root).unwrap();
    // The earlier run renamed `f` and stopped before saying so.
    std::fs::write(root.join("f"), &content).unwrap();
    let root_id = crate::plan::hex(&<sha2::Sha256 as sha2::Digest>::digest(root.canonicalize().unwrap().to_string_lossy().as_bytes()));
    let state = dir.path().join("j");
    std::fs::write(&state, format!("heronite-depot 1 {} {root_id}\nw 0\nr 0\n", b.plan.fingerprint())).unwrap();
    let options = Options { state_file: Some(state), ..Default::default() };
    let fake = Fake::default();
    let (out, seen) = run(&root, &b.plan, &fake, &options, &Cancel::new());
    assert_eq!(out.unwrap(), Outcome::Done);
    assert_eq!(fake.total(), 0, "the file in place is the new one");
    assert_eq!(seen.last().unwrap().resumed, 64);
}

#[test]
fn a_journal_of_another_plan_is_ignored() {
    let dir = tempfile::tempdir().unwrap();
    let state = dir.path().join("j");
    let mut b = Build::default();
    let content = b.file("f", &[("c", 0, 64)]);
    std::fs::write(&state, "heronite-depot 1 0000\np 0\n").unwrap();
    std::fs::write(dir.path().join("f"), vec![0u8; 64]).unwrap();
    let options = Options { state_file: Some(state), ..Default::default() };
    let (out, _) = run(dir.path(), &b.plan, &Fake::default(), &options, &Cancel::new());
    assert_eq!(out.unwrap(), Outcome::Done);
    assert_eq!(std::fs::read(dir.path().join("f")).unwrap(), content);
}

#[test]
fn chunks_dropped_over_budget_are_fetched_again() {
    let dir = tempfile::tempdir().unwrap();
    let mut b = Build::default();
    // `shared` is used first and last, with other chunks between that do not
    // fit beside it.
    let mut expected = Vec::new();
    expected.push(b.file("first", &[("shared", 0, 64)]));
    for i in 0..6 {
        let id = format!("mid{i}");
        expected.push(b.file(&format!("mid{i}"), &[(id.as_str(), 0, 64)]));
    }
    expected.push(b.file("last", &[("shared", 0, 64)]));
    let fake = Fake::default();
    let options = Options { workers: 2, max_memory: 128, ..Default::default() };
    let (out, seen) = run(dir.path(), &b.plan, &fake, &options, &Cancel::new());
    assert_eq!(out.unwrap(), Outcome::Done);
    for (f, content) in b.plan.files.iter().zip(&expected) {
        assert_eq!(&std::fs::read(dir.path().join(&f.path)).unwrap(), content);
    }
    assert_eq!(fake.count("shared"), 2);
    let last = seen.last().unwrap();
    assert!(last.downloaded > last.download_total);
}

#[test]
fn a_big_budget_keeps_shared_chunks() {
    let dir = tempfile::tempdir().unwrap();
    let mut b = Build::default();
    b.file("first", &[("shared", 0, 64)]);
    for i in 0..6 {
        let id = format!("mid{i}");
        b.file(&format!("mid{i}"), &[(id.as_str(), 0, 64)]);
    }
    b.file("last", &[("shared", 0, 64)]);
    let fake = Fake::default();
    let (out, _) = run(dir.path(), &b.plan, &fake, &opts(4), &Cancel::new());
    assert_eq!(out.unwrap(), Outcome::Done);
    assert_eq!(fake.count("shared"), 1);
}

#[test]
fn flaky_chunks_are_tried_again() {
    let dir = tempfile::tempdir().unwrap();
    let mut b = Build::default();
    let content = b.file("f", &[("c", 0, 64)]);
    let fake = Fake { flaky: Mutex::new(HashMap::from([("c".to_string(), 1)])), ..Default::default() };
    let (out, _) = run(dir.path(), &b.plan, &fake, &opts(1), &Cancel::new());
    assert_eq!(out.unwrap(), Outcome::Done);
    assert_eq!(std::fs::read(dir.path().join("f")).unwrap(), content);
}

#[test]
fn a_chunk_that_never_works_fails_the_run() {
    let dir = tempfile::tempdir().unwrap();
    let mut b = Build::default();
    b.file("f", &[("c", 0, 64)]);
    let fake = Fake { broken: vec!["c".into()], ..Default::default() };
    let cancel = Cancel::new();
    // Cancelling during the retries ends them quickly, as a cancel.
    let c2 = cancel.clone();
    let t = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(300));
        c2.request();
    });
    let (out, _) = run(dir.path(), &b.plan, &fake, &opts(1), &cancel);
    t.join().unwrap();
    assert_eq!(out.unwrap(), Outcome::Cancelled);
    assert!(!dir.path().join("f").exists());
    assert!(!dir.path().join("f.heronite-part").exists());
}

#[test]
fn damaged_data_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let mut b = Build::default();
    b.file("f", &[("c", 0, 64)]);
    let fake = Fake { wrong: vec!["c".into()], ..Default::default() };
    let (out, _) = run(dir.path(), &b.plan, &fake, &opts(1), &Cancel::new());
    let err = out.unwrap_err().to_string();
    assert!(err.contains("damaged"), "{err}");
    assert!(!dir.path().join("f").exists());
    assert!(!dir.path().join("f.heronite-part").exists());
}

#[test]
fn a_chunk_of_the_wrong_size_is_refused() {
    struct Short;
    impl Source for Short {
        fn fetch(&self, chunk: &Chunk, _: &Cancel) -> Result<Vec<u8>> {
            Ok(vec![0; chunk.size as usize - 1])
        }
    }
    let dir = tempfile::tempdir().unwrap();
    let mut b = Build::default();
    b.file("f", &[("c", 0, 8)]);
    let err = install(dir.path(), &b.plan, &Short, &opts(1), &Cancel::new(), &mut |_| {}).unwrap_err().to_string();
    assert!(err.contains("bytes instead of"), "{err}");
}

#[test]
fn unsafe_plans_write_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let plan = Plan { files: vec![FileTask { path: "../escape".into(), ..Default::default() }], ..Default::default() };
    assert!(install(&dir.path().join("game"), &plan, &Fake::default(), &opts(1), &Cancel::new(), &mut |_| {}).is_err());
    assert!(!dir.path().join("escape").exists());
}

#[cfg(unix)]
#[test]
fn links_are_made_and_folders_through_old_links_refused() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("game");
    let mut b = Build::default();
    b.file("lib/libx.so.1", &[("c", 0, 64)]);
    b.plan.files.push(FileTask { path: "lib/libx.so".into(), symlink: Some("libx.so.1".into()), ..Default::default() });
    let (out, _) = run(&root, &b.plan, &Fake::default(), &opts(1), &Cancel::new());
    assert_eq!(out.unwrap(), Outcome::Done);
    assert_eq!(std::fs::read_link(root.join("lib/libx.so")).unwrap(), Path::new("libx.so.1"));
    assert_eq!(std::fs::read(root.join("lib/libx.so")).unwrap().len(), 64);

    // A link already in the folder that leads outside it is not followed.
    let outside = dir.path().join("outside");
    std::fs::create_dir_all(&outside).unwrap();
    std::os::unix::fs::symlink(&outside, root.join("evil")).unwrap();
    let mut b = Build::default();
    b.file("evil/x", &[("c", 0, 64)]);
    assert!(install(&root, &b.plan, &Fake::default(), &opts(1), &Cancel::new(), &mut |_| {}).is_err());
    assert!(!outside.join("x").exists());
    // Nor are folders created behind it before the run stops.
    let mut b = Build::default();
    b.file("evil/deep/tree/x", &[("c", 0, 64)]);
    assert!(install(&root, &b.plan, &Fake::default(), &opts(1), &Cancel::new(), &mut |_| {}).is_err());
    assert!(!outside.join("deep").exists());
}

#[test]
fn space_counts_growth_and_room_for_one_copy() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a"), vec![0u8; 100]).unwrap();
    let file = |path: &str, size: u64| FileTask { path: path.into(), size, ..Default::default() };
    let plan = Plan { files: vec![file("a", 150), file("new", 30)], ..Default::default() };
    // 50 more for `a`, 30 for `new`, and `a`'s 100 old bytes while both exist.
    assert_eq!(space_needed(dir.path(), &plan), 180);
}
