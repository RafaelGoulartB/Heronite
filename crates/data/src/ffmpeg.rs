//! Where ffmpeg and ffprobe are: on the `PATH` first (the system's own,
//! which on Linux plays the trailers' sound), else the copies downloaded
//! into `<data>/tools/bin` (`heronite_launch::ffmpeg_install`, on Windows
//! when a trailer is played). Everything that runs them (the trailers,
//! the posters of replays, cutting a replay) starts them through
//! [`command`].
//!
//! What is found (or not) is remembered, so asking again costs one file
//! check at most; [`forget`] (after a download) makes the next call look
//! again. A remembered program that is gone (uninstalled, quarantined) is
//! looked for again too.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;

use crate::paths::DataPaths;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tool {
    Ffmpeg,
    Ffprobe,
}

impl Tool {
    pub const ALL: [Tool; 2] = [Tool::Ffmpeg, Tool::Ffprobe];

    pub const fn name(self) -> &'static str {
        match self {
            Tool::Ffmpeg => "ffmpeg",
            Tool::Ffprobe => "ffprobe",
        }
    }

    /// `ffmpeg` (`ffmpeg.exe` on Windows).
    pub fn file_name(self) -> String {
        format!("{}{}", self.name(), std::env::consts::EXE_SUFFIX)
    }

    const fn index(self) -> usize {
        match self {
            Tool::Ffmpeg => 0,
            Tool::Ffprobe => 1,
        }
    }
}

/// The folder the downloaded copies go to: `<data>/tools/bin`.
pub fn data_bin_dir(paths: &DataPaths) -> PathBuf {
    paths.tools_dir().join("bin")
}

/// Where the downloaded `tool` is (or would be).
pub fn data_bin(paths: &DataPaths, tool: Tool) -> PathBuf {
    data_bin_dir(paths).join(tool.file_name())
}

/// Per tool: `None` = not looked for yet, `Some(None)` = not there.
static FOUND: Mutex<[Option<Option<PathBuf>>; 2]> = Mutex::new([None, None]);

/// `tool` on the `PATH`, else in this data folder's `tools/bin`.
pub fn find(tool: Tool) -> Option<PathBuf> {
    let mut found = FOUND.lock().unwrap_or_else(|e| e.into_inner());
    match &found[tool.index()] {
        Some(Some(p)) if p.is_file() => return Some(p.clone()),
        Some(None) => return None,
        _ => {}
    }
    let p = find_in(tool, std::env::var_os("PATH"), &data_bin_dir(&DataPaths::detect()));
    found[tool.index()] = Some(p.clone());
    p
}

/// Makes the next [`find`] look again (ffmpeg was just downloaded).
pub fn forget() {
    *FOUND.lock().unwrap_or_else(|e| e.into_inner()) = [None, None];
}

/// `tool` in the folders of `path_var` (a `PATH`; its empty or relative
/// entries are left out), else in `data_bin_dir`.
pub fn find_in(tool: Tool, path_var: Option<OsString>, data_bin_dir: &Path) -> Option<PathBuf> {
    let file = tool.file_name();
    let mut dirs: Vec<PathBuf> =
        path_var.as_deref().map(|p| std::env::split_paths(p).filter(|d| d.is_absolute()).collect()).unwrap_or_default();
    dirs.push(data_bin_dir.to_path_buf());
    dirs.into_iter().map(|d| d.join(&file)).find(|p| p.is_file())
}

/// Whether both ffmpeg and ffprobe are there.
pub fn available() -> bool {
    Tool::ALL.iter().all(|t| find(*t).is_some())
}

/// A command for `tool`: the one [`find`] finds, else its bare name (which
/// then fails to start, as before).
pub fn command(tool: Tool) -> Command {
    Command::new(find(tool).unwrap_or_else(|| PathBuf::from(tool.name())))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_path_comes_before_the_data_folder() {
        let dir = tempfile::tempdir().unwrap();
        let (on_path, data) = (dir.path().join("usr"), dir.path().join("data"));
        std::fs::create_dir_all(&on_path).unwrap();
        std::fs::create_dir_all(&data).unwrap();
        let path_var = std::env::join_paths([&on_path]).ok();
        assert_eq!(find_in(Tool::Ffmpeg, path_var.clone(), &data), None);

        std::fs::write(data.join(Tool::Ffmpeg.file_name()), b"").unwrap();
        assert_eq!(find_in(Tool::Ffmpeg, path_var.clone(), &data), Some(data.join(Tool::Ffmpeg.file_name())));
        assert_eq!(find_in(Tool::Ffmpeg, None, &data), Some(data.join(Tool::Ffmpeg.file_name())));

        std::fs::write(on_path.join(Tool::Ffmpeg.file_name()), b"").unwrap();
        assert_eq!(find_in(Tool::Ffmpeg, path_var.clone(), &data), Some(on_path.join(Tool::Ffmpeg.file_name())));
        assert_eq!(find_in(Tool::Ffprobe, path_var, &data), None, "each program is looked for on its own");
    }

    #[test]
    fn relative_path_entries_are_left_out() {
        let dir = tempfile::tempdir().unwrap();
        let data = dir.path().join("data");
        // `a;;b` holds an empty entry, which would mean the current folder.
        let path_var = Some(OsString::from(if cfg!(windows) { "relative;;" } else { "relative::" }));
        assert_eq!(find_in(Tool::Ffmpeg, path_var, &data), None);
    }

    #[test]
    fn the_names_are_programs_of_this_system() {
        assert_eq!(Tool::Ffprobe.file_name(), if cfg!(windows) { "ffprobe.exe" } else { "ffprobe" });
        let paths = DataPaths::at("/hr");
        assert_eq!(data_bin(&paths, Tool::Ffmpeg), paths.tools_dir().join("bin").join(Tool::Ffmpeg.file_name()));
    }
}
