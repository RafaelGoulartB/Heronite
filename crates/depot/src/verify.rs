//! Checking installed files against the hashes of their manifest.

use std::fs::File;
use std::io::{self, Read};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use anyhow::Result;
use crossbeam_channel::RecvTimeoutError;
use heronite_data::proc::Cancel;

use crate::hash::Hash;
use crate::plan::join;

/// A file as its manifest describes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Expected {
    /// Relative to the install folder, `/` between folders.
    pub path: String,
    pub size: u64,
    pub hash: Hash,
}

/// What is wrong with a file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Problem {
    Missing,
    /// It has this many bytes instead.
    Size(u64),
    Hash,
}

/// Hashes the `files` under `root` with `workers` threads (`0`: one per
/// processor) and gives the ones that do not match, in the order of `files`;
/// `None` when cancelled. `on_progress` gets the bytes checked and the bytes
/// to check, a few times a second and once at the end. A file that cannot be
/// read is reported as missing.
pub fn verify(
    root: &Path,
    files: &[Expected],
    workers: usize,
    cancel: &Cancel,
    on_progress: &mut dyn FnMut(u64, u64),
) -> Result<Option<Vec<(String, Problem)>>> {
    let total: u64 = files.iter().map(|f| f.size).sum();
    let workers = crate::workers(workers).min(files.len().max(1));
    let next = AtomicUsize::new(0);
    let stop = AtomicBool::new(false);
    let (tx, rx) = crossbeam_channel::unbounded::<Event>();
    let mut problems: Vec<(usize, Problem)> = Vec::new();
    let mut done = 0u64;
    std::thread::scope(|s| {
        for _ in 0..workers {
            let tx = tx.clone();
            let (next, stop) = (&next, &stop);
            s.spawn(move || loop {
                if stop.load(Ordering::Relaxed) || cancel.is_requested() {
                    break;
                }
                let i = next.fetch_add(1, Ordering::Relaxed);
                let Some(f) = files.get(i) else { break };
                let problem = check(root, f, &|n| {
                    let _ = tx.send(Event::Read(n));
                    !stop.load(Ordering::Relaxed) && !cancel.is_requested()
                });
                let _ = tx.send(Event::Checked(i, problem));
            });
        }
        drop(tx);
        let mut last = Instant::now();
        loop {
            match rx.recv_timeout(Duration::from_millis(100)) {
                Ok(Event::Read(n)) => done += n,
                Ok(Event::Checked(i, Some(p))) => problems.push((i, p)),
                Ok(Event::Checked(_, None)) => {}
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => break,
            }
            if cancel.is_requested() {
                stop.store(true, Ordering::Relaxed);
            }
            if last.elapsed() >= Duration::from_millis(250) {
                last = Instant::now();
                on_progress(done.min(total), total);
            }
        }
    });
    if cancel.is_requested() {
        return Ok(None);
    }
    on_progress(total, total);
    problems.sort_by_key(|(i, _)| *i);
    Ok(Some(problems.into_iter().map(|(i, p)| (files[i].path.clone(), p)).collect()))
}

enum Event {
    Read(u64),
    Checked(usize, Option<Problem>),
}

/// One file's problem, if any. `read` gets the bytes read so far and says
/// whether to go on.
fn check(root: &Path, f: &Expected, read: &dyn Fn(u64) -> bool) -> Option<Problem> {
    let Ok(path) = join(root, &f.path) else { return Some(Problem::Missing) };
    let mut file = match File::open(&path) {
        Ok(file) => file,
        Err(_) => return Some(Problem::Missing),
    };
    match file.metadata() {
        Ok(m) if !m.is_file() => return Some(Problem::Missing),
        Ok(m) if m.len() != f.size => return Some(Problem::Size(m.len())),
        Err(_) => return Some(Problem::Missing),
        Ok(_) => {}
    }
    let mut hasher = f.hash.hasher();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = match file.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => {
                log::warn!("reading {}: {e}", path.display());
                return Some(Problem::Missing);
            }
        };
        hasher.update(&buf[..n]);
        if !read(n as u64) {
            return None;
        }
    }
    (hasher.finish() != f.hash).then_some(Problem::Hash)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sha1(bytes: &[u8]) -> Hash {
        Hash::Sha1([0; 20]).of_reader(&mut &bytes[..]).unwrap()
    }

    #[test]
    fn reports_missing_resized_and_changed_files() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("d")).unwrap();
        std::fs::write(dir.path().join("ok"), b"hello").unwrap();
        std::fs::write(dir.path().join("d/changed"), b"hellO").unwrap();
        std::fs::write(dir.path().join("short"), b"hell").unwrap();
        let files = vec![
            Expected { path: "ok".into(), size: 5, hash: sha1(b"hello") },
            Expected { path: "gone".into(), size: 5, hash: sha1(b"hello") },
            Expected { path: "d/changed".into(), size: 5, hash: sha1(b"hello") },
            Expected { path: "short".into(), size: 5, hash: sha1(b"hello") },
        ];
        let mut last = (0, 0);
        let out = verify(dir.path(), &files, 2, &Cancel::new(), &mut |d, t| last = (d, t)).unwrap().unwrap();
        assert_eq!(
            out,
            [("gone".to_string(), Problem::Missing), ("d/changed".to_string(), Problem::Hash), ("short".to_string(), Problem::Size(4))]
        );
        assert_eq!(last, (20, 20));
    }

    #[test]
    fn cancelled_gives_none() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a"), b"x").unwrap();
        let cancel = Cancel::new();
        cancel.request();
        let files = [Expected { path: "a".into(), size: 1, hash: sha1(b"x") }];
        assert!(verify(dir.path(), &files, 1, &cancel, &mut |_, _| {}).unwrap().is_none());
    }
}
