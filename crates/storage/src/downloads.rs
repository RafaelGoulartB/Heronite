//! What downloads left behind where there is no Wine (Windows): a store
//! tool's unfinished download (`<name>.part.exe`), the installers of the
//! stores some Epic games need (downloaded once, run once), and the app's
//! own updates (`cache\updates`, the portable copy's `cache\update`) while
//! no update is under way. Files changed in the last hours are left out: a
//! download may still be writing them.

use std::fs;
use std::path::{Path, PathBuf};

use heronite_i18n::tr;

use crate::leftovers::old_enough;
use crate::{Ctx, Group, Item, Kind, State};

pub(crate) fn items(ctx: &Ctx) -> Vec<Item> {
    let mut out = Vec::new();
    for path in files(&ctx.data_paths.tools_dir().join("bin")) {
        if let Some(name) = unfinished(&path) {
            out.push(download(tr!("Unfinished download of {name}", name), path));
        }
    }
    for path in files(&ctx.data_paths.legendary_redist_dir()) {
        let title = match unfinished(&path) {
            Some(name) => tr!("Unfinished download of {name}", name),
            None => path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
        };
        out.push(download(title, path));
    }
    out.extend(updates(ctx));
    out
}

/// The app's downloaded updates: left by an update that failed or was
/// installed; the app clears them at its next start.
fn updates(ctx: &Ctx) -> Option<Item> {
    if ctx.updating {
        return None;
    }
    let paths: Vec<PathBuf> = update_dirs(ctx).into_iter().filter(|p| fs::read_dir(p).is_ok_and(|mut d| d.next().is_some())).collect();
    let mut item = download(tr!("Heronite updates").into(), paths.first()?.clone());
    item.folder = ctx.app.cache.clone();
    item.paths = paths;
    Some(item)
}

/// Where the app downloads its updates (`services::update` in the app, and
/// `heronite_update::Portable::staging` for a portable copy).
pub(crate) fn update_dirs(ctx: &Ctx) -> [PathBuf; 2] {
    [ctx.app.cache.join("updates"), ctx.app.cache.join("update")]
}

fn download(title: String, path: PathBuf) -> Item {
    Item::new(Group::Downloads, Kind::Download, State::Unused, title, path)
}

/// The files of `dir` not changed for a while, in name order.
fn files(dir: &Path) -> Vec<PathBuf> {
    let Ok(rd) = fs::read_dir(dir) else { return Vec::new() };
    let mut out: Vec<PathBuf> =
        rd.flatten().filter(|e| e.file_type().is_ok_and(|t| t.is_file())).map(|e| e.path()).filter(|p| old_enough(p)).collect();
    out.sort();
    out
}

/// `legendary` for `legendary.part.exe`, `EAappInstaller.exe` for
/// `EAappInstaller.exe.part`: the name of what was being downloaded.
fn unfinished(path: &Path) -> Option<String> {
    let name = path.file_name()?.to_str()?;
    let (stem, rest) = name.split_once(".part")?;
    (rest.is_empty() || rest.starts_with('.')).then(|| stem.to_string()).filter(|s| !s.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use heronite_data::{AppPaths, DataPaths};
    use std::time::{Duration, SystemTime};

    fn ctx(dir: &Path) -> Ctx {
        Ctx {
            data_paths: DataPaths::at(dir.join("data")),
            app: AppPaths { config: dir.join("cfg"), cache: dir.join("cache") },
            home: dir.join("home"),
            updating: false,
        }
    }

    fn old_file(path: &Path) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, b"x").unwrap();
        let old = SystemTime::now() - Duration::from_secs(3 * 3600);
        fs::File::options().write(true).open(path).unwrap().set_modified(old).unwrap();
    }

    #[test]
    fn names_what_was_downloaded() {
        assert_eq!(unfinished(Path::new("legendary.part.exe")).as_deref(), Some("legendary"));
        assert_eq!(unfinished(Path::new("EAappInstaller.exe.part")).as_deref(), Some("EAappInstaller.exe"));
        assert_eq!(unfinished(Path::new("legendary.exe")), None);
        assert_eq!(unfinished(Path::new("party.exe")), None);
        assert_eq!(unfinished(Path::new(".part")), None);
    }

    #[test]
    fn lists_old_downloads_and_updates_not_in_use() {
        let dir = tempfile::tempdir().unwrap();
        let mut c = ctx(dir.path());
        let bin = c.data_paths.tools_dir().join("bin");
        old_file(&bin.join("legendary.part.exe"));
        old_file(&bin.join("gogdl.exe"));
        // Being downloaded now: left alone.
        fs::write(bin.join("nile.part.exe"), b"x").unwrap();
        old_file(&c.data_paths.legendary_redist_dir().join("EAappInstaller.exe"));
        old_file(&c.app.cache.join("updates").join("0.19.0").join("Heronite-Setup-x64.exe"));
        fs::create_dir_all(c.app.cache.join("update")).unwrap();

        let titles = |c: &Ctx| items(c).into_iter().map(|i| (i.title, i.paths)).collect::<Vec<_>>();
        assert_eq!(
            titles(&c),
            [
                ("Unfinished download of legendary".to_string(), vec![bin.join("legendary.part.exe")]),
                ("EAappInstaller.exe".to_string(), vec![c.data_paths.legendary_redist_dir().join("EAappInstaller.exe")]),
                // The empty staging folder is left out.
                ("Heronite updates".to_string(), vec![c.app.cache.join("updates")]),
            ]
        );
        c.updating = true;
        assert_eq!(titles(&c).len(), 2, "an update under way keeps its files");
    }
}
