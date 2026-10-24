//! FOMOD installers: the `fomod/ModuleConfig.xml` most Nexus Mods archives of
//! Bethesda games (and others) ship. [`load`] reads one into an [`Installer`];
//! a [`Wizard`] walks the user through it and ends in the [`CopyRule`]s to
//! apply (MO2 and Vortex behave the same way).
//!
//! [`CopyRule`]: crate::CopyRule

mod deps;
mod parse;
mod schema;
mod text;
mod wizard;

use std::path::{Path, PathBuf};

use heronite_i18n::tr;

pub use deps::{Dependency, FileState};
pub use schema::{ConditionalInstall, FileItem, Group, GroupKind, Info, Installer, Plugin, PluginType, Step, TypeDescriptor};
pub use wizard::{GroupView, PluginView, StepView, Wizard};

/// The folder that holds `fomod/ModuleConfig.xml`: `content` itself, or what
/// it wraps in one or two single-folder levels.
pub fn find(content: &Path) -> Option<PathBuf> {
    find_with(content, |fomod| child_named(fomod, "ModuleConfig.xml").is_some())
}

/// Like [`find`] for a `fomod/script.cs` (a C# installer script), which
/// cannot be run: [`load`] explains it.
pub fn find_scripted(content: &Path) -> Option<PathBuf> {
    find_with(content, |fomod| child_named(fomod, "script.cs").is_some())
}

fn find_with(content: &Path, has: impl Fn(&Path) -> bool) -> Option<PathBuf> {
    let mut dir = content.to_path_buf();
    for _ in 0..3 {
        if child_named(&dir, "fomod").is_some_and(|f| f.is_dir() && has(&f)) {
            return Some(dir);
        }
        dir = only_subfolder(&dir)?;
    }
    None
}

/// The single folder inside `dir`, loose files aside.
fn only_subfolder(dir: &Path) -> Option<PathBuf> {
    let mut folders = std::fs::read_dir(dir).ok()?.flatten().map(|e| e.path()).filter(|p| p.is_dir());
    let first = folders.next()?;
    folders.next().is_none().then_some(first)
}

/// The entry of `dir` called `name`, ignoring case.
fn child_named(dir: &Path, name: &str) -> Option<PathBuf> {
    let exact = dir.join(name);
    if exact.exists() {
        return Some(exact);
    }
    std::fs::read_dir(dir).ok()?.flatten().find(|e| e.file_name().to_string_lossy().eq_ignore_ascii_case(name)).map(|e| e.path())
}

/// `rel` (`\` or `/` separated, any case) inside `content_root`, as it is
/// spelled on disk. `None` when it is not there or leaves the folder.
pub fn resolve(content_root: &Path, rel: &str) -> Option<PathBuf> {
    let mut path = content_root.to_path_buf();
    for part in rel.split(['/', '\\']).map(str::trim).filter(|p| !p.is_empty() && *p != ".") {
        if part == ".." {
            return None;
        }
        path = child_named(&path, part)?;
    }
    Some(path)
}

/// Reads the installer of `content_root` (the folder [`find`] gave).
pub fn load(content_root: &Path) -> Result<Installer, String> {
    let fomod =
        child_named(content_root, "fomod").filter(|f| f.is_dir()).ok_or_else(|| tr!("This mod has no FOMOD installer").to_string())?;
    let Some(config) = child_named(&fomod, "ModuleConfig.xml") else {
        return Err(if child_named(&fomod, "script.cs").is_some() {
            tr!("This mod uses a scripted installer (script.cs), which Heronite cannot run").to_string()
        } else {
            tr!("This mod has no FOMOD installer").to_string()
        });
    };
    let bytes = std::fs::read(&config).map_err(|e| tr!("could not read ModuleConfig.xml: {error}", error = e))?;
    let reader = parse::Reader { root: content_root };
    let mut installer = parse::with_document(&bytes, |doc| reader.module_config(doc))
        .map_err(|e| tr!("ModuleConfig.xml is not valid: {error}", error = e))?;
    // info.xml only decorates the wizard: a broken one is ignored.
    if let Some(bytes) = child_named(&fomod, "info.xml").and_then(|p| std::fs::read(p).ok()) {
        let _ = parse::with_document(&bytes, |doc| reader.info_xml(doc, &mut installer.info));
    }
    Ok(installer)
}

#[cfg(test)]
mod tests;
