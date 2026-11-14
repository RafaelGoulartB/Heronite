//! The resume journal: which files of a plan an earlier run finished.
//!
//! A text file, appended to as files finish so a big install costs one short
//! write per file: a header with the plan's fingerprint, then `w <index>`
//! (written beside its place, waiting to be renamed), `r <index>` (about to
//! be renamed: whether it was is told by the disk) and `p <index>` (in place)
//! lines, each on disk before the run goes on. A journal of another plan is
//! started over.

use std::collections::HashSet;
use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

const MAGIC: &str = "heronite-depot 1";

#[derive(Debug, Default)]
pub(crate) struct Journal {
    path: Option<PathBuf>,
    file: Option<File>,
    pub written: HashSet<usize>,
    /// Renamed, or about to be, when the earlier run stopped.
    pub renaming: HashSet<usize>,
    pub placed: HashSet<usize>,
}

impl Journal {
    /// The journal at `path` for the plan with `fingerprint`; `None` keeps
    /// nothing (a run that cannot be resumed).
    pub fn open(path: Option<&Path>, fingerprint: &str) -> io::Result<Self> {
        let Some(path) = path else { return Ok(Self::default()) };
        let header = format!("{MAGIC} {fingerprint}");
        let mut journal = Self { path: Some(path.to_path_buf()), ..Default::default() };
        let text = std::fs::read_to_string(path).unwrap_or_default();
        let mut lines = text.lines();
        if lines.next() == Some(header.as_str()) {
            for line in lines {
                match line.split_once(' ').map(|(k, n)| (k, n.parse::<usize>())) {
                    Some(("w", Ok(n))) => {
                        journal.written.insert(n);
                    }
                    Some(("r", Ok(n))) => {
                        journal.renaming.insert(n);
                    }
                    Some(("p", Ok(n))) => {
                        journal.written.remove(&n);
                        journal.renaming.remove(&n);
                        journal.placed.insert(n);
                    }
                    // A line cut short by a crash ends what is known.
                    _ => break,
                }
            }
        }
        // Written again whole, so a line cut short is not followed by new ones.
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let mut text = format!("{header}\n");
        for n in &journal.written {
            text.push_str(&format!("w {n}\n"));
        }
        for n in &journal.renaming {
            text.push_str(&format!("r {n}\n"));
        }
        for n in &journal.placed {
            text.push_str(&format!("p {n}\n"));
        }
        std::fs::write(path, text)?;
        journal.file = Some(OpenOptions::new().append(true).open(path)?);
        Ok(journal)
    }

    pub fn written(&mut self, index: usize) -> io::Result<()> {
        self.written.insert(index);
        self.append('w', index)
    }

    /// The file is about to be renamed into place.
    pub fn renaming(&mut self, index: usize) -> io::Result<()> {
        self.renaming.insert(index);
        self.append('r', index)
    }

    pub fn placed(&mut self, index: usize) -> io::Result<()> {
        self.written.remove(&index);
        self.renaming.remove(&index);
        self.placed.insert(index);
        self.append('p', index)
    }

    /// One line, on disk before the run goes on (a crash must not leave a
    /// file the journal says more of than is true).
    fn append(&mut self, kind: char, index: usize) -> io::Result<()> {
        match &mut self.file {
            Some(f) => {
                writeln!(f, "{kind} {index}")?;
                f.sync_data()
            }
            None => Ok(()),
        }
    }

    /// The run finished: nothing is left to resume.
    pub fn finish(mut self) {
        self.file = None;
        if let Some(path) = &self.path {
            if let Err(e) = std::fs::remove_file(path) {
                if e.kind() != io::ErrorKind::NotFound {
                    log::warn!("removing {}: {e}", path.display());
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_what_the_same_plan_finished() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state/journal");
        let mut j = Journal::open(Some(&path), "abc").unwrap();
        j.written(3).unwrap();
        j.placed(1).unwrap();
        j.written(2).unwrap();
        j.placed(2).unwrap();
        j.written(4).unwrap();
        j.renaming(4).unwrap();
        drop(j);
        let j = Journal::open(Some(&path), "abc").unwrap();
        assert_eq!(j.renaming, HashSet::from([4]));
        assert_eq!(j.written, HashSet::from([3, 4]));
        assert_eq!(j.placed, HashSet::from([1, 2]));
        drop(j);
        let other = Journal::open(Some(&path), "def").unwrap();
        assert!(other.written.is_empty() && other.placed.is_empty());
        other.finish();
        assert!(!path.exists());
    }

    #[test]
    fn a_torn_line_ends_the_journal() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("j");
        std::fs::write(&path, format!("{MAGIC} f\np 0\np 1\np")).unwrap();
        let j = Journal::open(Some(&path), "f").unwrap();
        assert_eq!(j.placed, HashSet::from([0, 1]));
    }
}
