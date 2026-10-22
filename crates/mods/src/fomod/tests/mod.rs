//! Fixtures are written here (no third-party files): each test builds the
//! archive it needs in a temp folder.

mod flow;
mod read;

use std::path::Path;

use tempfile::TempDir;

use super::*;

/// A content root holding `fomod/ModuleConfig.xml` (and any `extra` file).
fn archive(config: &str, extra: &[(&str, &[u8])]) -> TempDir {
    let dir = TempDir::new().unwrap();
    write(dir.path(), "fomod/ModuleConfig.xml", config.as_bytes());
    for (path, bytes) in extra {
        write(dir.path(), path, bytes);
    }
    dir
}

fn write(root: &Path, rel: &str, bytes: &[u8]) {
    let path = root.join(rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, bytes).unwrap();
}

fn load_xml(config: &str) -> (TempDir, Installer) {
    let dir = archive(config, &[]);
    let installer = load(dir.path()).unwrap();
    (dir, installer)
}

fn states(list: &[(&'static str, FileState)]) -> Box<dyn Fn(&str) -> FileState + Send> {
    let list: Vec<_> = list.to_vec();
    Box::new(move |f| list.iter().find(|(n, _)| n.eq_ignore_ascii_case(f)).map_or(FileState::Missing, |(_, s)| *s))
}

fn wizard_with(config: &str, files: &[(&'static str, FileState)], previous: Option<&crate::FomodChoices>) -> Wizard {
    let (_dir, installer) = load_xml(config);
    Wizard::new(installer, states(files), previous)
}

fn wizard(config: &str) -> Wizard {
    wizard_with(config, &[], None)
}

/// `(source, destination)` of every rule.
fn copies(w: &Wizard) -> Vec<(String, String)> {
    w.files().into_iter().map(|r| (r.source, r.destination)).collect()
}

fn picked(w: &Wizard) -> Vec<Vec<bool>> {
    w.step().groups.iter().map(|g| g.plugins.iter().map(|p| p.selected).collect()).collect()
}
