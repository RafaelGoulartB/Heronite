//! Files written by one operation in a game's folder, to undo it: when a
//! step fails or is cancelled, what this operation wrote is put back as it
//! was (created files removed, replaced ones restored, new folders removed).

use std::path::{Path, PathBuf};

use super::hash::sha256;

#[derive(Default)]
pub(crate) struct Journal {
    created: Vec<PathBuf>,
    /// Earlier content of files replaced or removed.
    replaced: Vec<(PathBuf, Vec<u8>)>,
    dirs: Vec<PathBuf>,
}

/// Runs `f` with a journal; undoes its writes when it fails.
pub(crate) fn journaled<T>(f: impl FnOnce(&mut Journal) -> Result<T, String>) -> Result<T, String> {
    let mut j = Journal::default();
    f(&mut j).inspect_err(|_| j.rollback())
}

impl Journal {
    pub(crate) fn touched(&self, path: &Path) -> bool {
        self.created.iter().any(|p| p == path) || self.replaced.iter().any(|(p, _)| p == path)
    }

    fn mkdirs(&mut self, dir: &Path) -> Result<(), String> {
        let mut missing = Vec::new();
        let mut d = Some(dir);
        while let Some(x) = d.filter(|x| !x.exists()) {
            missing.push(x.to_path_buf());
            d = x.parent();
        }
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        self.dirs.extend(missing.into_iter().rev());
        Ok(())
    }

    fn backup(&mut self, path: &Path) -> Result<(), String> {
        if self.touched(path) {
            return Ok(());
        }
        match std::fs::read(path) {
            Ok(old) => self.replaced.push((path.to_path_buf(), old)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => self.created.push(path.to_path_buf()),
            Err(e) => return Err(format!("{}: {e}", path.display())),
        }
        Ok(())
    }

    pub(crate) fn write(&mut self, path: &Path, bytes: &[u8]) -> Result<(), String> {
        if let Some(dir) = path.parent() {
            self.mkdirs(dir)?;
        }
        self.backup(path)?;
        heronite_data::temps::write_atomic(path, bytes, None).map_err(|e| format!("{}: {e}", path.display()))
    }

    pub(crate) fn copy(&mut self, from: &Path, to: &Path) -> Result<String, String> {
        let bytes = std::fs::read(from).map_err(|e| format!("{}: {e}", from.display()))?;
        self.write(to, &bytes)?;
        Ok(sha256(&bytes))
    }

    pub(crate) fn remove(&mut self, path: &Path) -> Result<(), String> {
        self.backup(path)?;
        std::fs::remove_file(path).map_err(|e| format!("{}: {e}", path.display()))
    }

    pub(crate) fn rollback(self) {
        for p in self.created.iter().rev() {
            let _ = std::fs::remove_file(p);
        }
        for (p, bytes) in &self.replaced {
            let _ = heronite_data::temps::write_atomic(p, bytes, None);
        }
        for d in self.dirs.iter().rev() {
            let _ = std::fs::remove_dir(d);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rollback_puts_the_folder_back() {
        let dir = tempfile::tempdir().unwrap();
        let kept = dir.path().join("game.ini");
        let gone = dir.path().join("old.dll");
        std::fs::write(&kept, b"original").unwrap();
        std::fs::write(&gone, b"old").unwrap();
        let res: Result<(), String> = journaled(|j| {
            j.write(&kept, b"changed")?;
            j.write(&kept, b"changed twice")?;
            j.write(&dir.path().join("new/deep/file.fx"), b"x")?;
            j.remove(&gone)?;
            assert!(j.touched(&kept) && j.touched(&gone));
            Err("cancelled".into())
        });
        assert!(res.is_err());
        assert_eq!(std::fs::read(&kept).unwrap(), b"original");
        assert_eq!(std::fs::read(&gone).unwrap(), b"old");
        assert!(!dir.path().join("new").exists());
    }

    #[test]
    fn success_keeps_the_writes() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("src.dll");
        std::fs::write(&src, b"abc").unwrap();
        let sha = journaled(|j| j.copy(&src, &dir.path().join("dxgi.dll"))).unwrap();
        assert_eq!(sha, sha256(b"abc"));
        assert_eq!(std::fs::read(dir.path().join("dxgi.dll")).unwrap(), b"abc");
    }
}
