//! The work of an install job, on its thread: get the file (download, or use
//! the one on disk), unpack it, ask the FOMOD installer's questions, build
//! the mod folder and put the mod in the list.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use gpui_kit::*;
use heronite_i18n::tr;
use heronite_mods::fomod::{self, FileState, Wizard};
use heronite_mods::games::{self, Profile};
use heronite_mods::install::{self, InstallCtx, Plan, Source};
use heronite_mods::nexus::{self, NxmLink};
use heronite_mods::{download, thunderstore, FomodChoices, ModEntry, ModSource};

use super::facts;
use super::jobs::{Answer, JobEnv, Msg, Outcome};
use super::{FomodRequest, JobStage, ModsEvent, ModsService};

/// How a mod is going to be built, for one archive.
struct Build {
    /// The mod as known so far; `id` empty = made from the name.
    entry: ModEntry,
    /// The mod this one takes the place of.
    replace: Option<String>,
    /// Options of an earlier run of the FOMOD installer.
    previous: Option<FomodChoices>,
}

fn cancelled() -> String {
    tr!("The installation was cancelled").to_string()
}

/// Unpacks `path` (a folder is used where it is), asks the installer's
/// questions if it has any, and builds the mod. Returns the mod as it is in
/// the list now.
fn install_archive(env: &JobEnv, path: &Path, build: Build) -> Result<ModEntry, String> {
    let Build { mut entry, mut replace, previous } = build;
    let _turn = env.install_turn().ok_or_else(cancelled)?;
    env.stage(JobStage::Extracting);
    let dirs = env.root.game(&env.key);
    let mut list = env.shared.ensure_loaded(&env.root, &env.key, &env.game.title);
    let found = facts::resolve(&env.paths, &env.game);
    let known = games::find(env.game.steam_app_id.as_deref(), &env.game.title);
    let profile = Profile::of(&list.links, known);
    // The same file again is a reinstall, not a second copy.
    if replace.is_none() && !entry.id.is_empty() && list.get(&entry.id).is_some() {
        replace = Some(entry.id.clone());
    }
    let source = if path.is_dir() { Source::Folder(path.to_path_buf()) } else { Source::Archive(path.to_path_buf()) };
    let staged = install::stage(&dirs, &source, &env.cancel)?;

    let plan = match staged.fomod.as_deref() {
        Some(root) => {
            let mod_dirs = list.enabled().filter(|m| replace.as_deref() != Some(m.id.as_str())).map(|m| dirs.mod_dir(&m.id)).collect();
            let state = file_state(found.dir.clone(), profile.mod_root.clone(), mod_dirs);
            ask_installer(env, root, &mut entry, state, previous.as_ref())?
        }
        None => {
            if fomod::find_scripted(&staged.root).is_some() {
                env.notice(
                    tr!(
                        "{name} has a scripted installer, which Heronite cannot run: its files were put in as they are",
                        name = entry.name.clone()
                    ),
                    false,
                );
            }
            Plan::Auto
        }
    };

    env.stage(JobStage::Installing);
    let ctx = InstallCtx { dirs: &dirs, game_dir: found.dir.as_deref(), profile: &profile };
    let id = {
        // The commit writes the list it is given: keep the others' writes out,
        // and give it the latest list so a toggle made meanwhile is not lost.
        let _io = env.shared.lock_io();
        list = env.shared.list();
        install::commit(&ctx, &mut list, &staged, plan, entry, replace.as_deref(), &env.cancel)?
    };
    let built = list.get(&id).cloned().ok_or_else(|| tr!("The mod was not added to the list").to_string())?;
    env.shared.merge(built.clone(), replace.as_deref());
    env.shared.save(&dirs).map_err(|e| tr!("Could not save the list of mods: {error}", error = e))?;
    env.send(Msg::ListChanged { game_key: env.key.clone() });
    Ok(built)
}

/// The installer's wizard: the plan is its result, or the files as they are
/// when it cannot be read or the user chose so.
fn ask_installer(
    env: &JobEnv,
    root: &Path,
    entry: &mut ModEntry,
    state: Box<dyn Fn(&str) -> FileState + Send>,
    previous: Option<&FomodChoices>,
) -> Result<Plan, String> {
    let installer = match fomod::load(root) {
        Ok(i) => i,
        Err(e) => {
            env.notice(
                tr!(
                    "The installer of {name} could not be read ({error}): its files were put in as they are",
                    name = entry.name.clone(),
                    error = e
                ),
                false,
            );
            return Ok(Plan::Auto);
        }
    };
    let wizard = Arc::new(Mutex::new(Wizard::new(installer, state, previous)));
    env.stage(JobStage::NeedsChoices);
    let request = FomodRequest { job_id: env.id, game_key: env.key.clone(), mod_name: entry.name.clone(), wizard: wizard.clone() };
    match env.ask_fomod(request) {
        Some(Answer::Finish) => {
            let wizard = wizard.lock().unwrap_or_else(|e| e.into_inner());
            entry.fomod = Some(wizard.choices());
            Ok(Plan::Rules(wizard.files()))
        }
        Some(Answer::Plain) => Ok(Plan::Auto),
        Some(Answer::Cancel) | None => Err(cancelled()),
    }
}

/// A file an installer's condition names (`Skyrim.esm`, `SKSE/Plugins/x.dll`)
/// is active when the game has it or an enabled mod brings it, whatever the case.
fn file_state(game_dir: Option<PathBuf>, mod_root: String, mod_dirs: Vec<PathBuf>) -> Box<dyn Fn(&str) -> FileState + Send> {
    Box::new(move |file| {
        let file = file.replace('\\', "/");
        let file = file.trim_matches('/');
        if file.is_empty() {
            return FileState::Missing;
        }
        let rel = if mod_root.is_empty() { file.to_string() } else { format!("{mod_root}/{file}") };
        let there = |base: &Path| fomod::resolve(base, &rel).is_some();
        if game_dir.as_deref().is_some_and(there) || mod_dirs.iter().any(|d| there(d)) {
            FileState::Active
        } else {
            FileState::Missing
        }
    })
}

/// Downloads into the game's `downloads/` folder, within the limit of
/// downloads at once.
fn fetch(env: &JobEnv, url: &str, headers: &[(&str, &str)], name_hint: Option<&str>) -> Result<PathBuf, String> {
    env.stage(JobStage::Downloading);
    let _slot = env.download_slot().ok_or_else(cancelled)?;
    let dirs = env.root.game(&env.key);
    let mut progress = env.progress();
    download::download(url, &dirs.downloads(), name_hint, headers, &mut progress, &env.cancel)
}

/// The text a file mod is called by: its name without the extension.
fn name_of_file(path: &Path) -> String {
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let lower = name.to_lowercase();
    let stem = [".tar.gz", ".tar.xz", ".tar.bz2", ".tar.zst"]
        .iter()
        .find(|ext| lower.ends_with(**ext))
        .map(|ext| name[..name.len() - ext.len()].to_string())
        .or_else(|| path.file_stem().map(|s| s.to_string_lossy().into_owned()))
        .unwrap_or(name);
    stem.trim().to_string()
}

/// What the Nexus links and the Browse tab share: from the mod page to the
/// installed mod.
struct NexusRequest {
    domain: String,
    mod_id: u64,
    /// `None`: the page's main file.
    file_id: Option<u64>,
    /// The link's key and expiry (free accounts).
    link: Option<NxmLink>,
    replace: Option<String>,
    updating: bool,
}

fn install_nexus(env: &JobEnv, req: NexusRequest) -> Result<Outcome, String> {
    let Some(account) = env.account.as_ref() else {
        return Err(tr!("Add your Nexus Mods API key in Settings → Mods to download from Nexus Mods").to_string());
    };
    let key = account.api_key.as_str();
    env.stage(JobStage::Downloading);
    let page = nexus::mod_info(key, &req.domain, req.mod_id)?;
    if !page.name.is_empty() {
        env.name(&page.name);
    }
    let file_id = match req.file_id {
        Some(id) => id,
        None => {
            let files = nexus::files(key, &req.domain, req.mod_id)?;
            nexus::main_file(&files)
                .map(|f| f.file_id)
                .ok_or_else(|| tr!("This mod has no main file to install: pick a file on its page").to_string())?
        }
    };
    let file = nexus::file_info(key, &req.domain, req.mod_id, file_id)?;
    let link = match req.link {
        Some(link) => NxmLink { file_id, ..link },
        None => NxmLink { domain: req.domain.clone(), mod_id: req.mod_id, file_id, key: None, expires: None, user_id: None },
    };
    let url = nexus::download_link(key, &link)?;
    let path = fetch(env, &url, &[], Some(&file.file_name).filter(|n| !n.is_empty()).map(String::as_str))?;
    let entry = ModEntry {
        id: format!("nexus-{}-{file_id}", req.mod_id),
        name: if page.name.is_empty() { file.name.clone() } else { page.name.clone() },
        version: file.version.clone().or(page.version.clone()),
        author: page.author.clone(),
        summary: page.summary.clone(),
        picture: page.picture_url.clone(),
        enabled: true,
        source: ModSource::Nexus { domain: req.domain, mod_id: req.mod_id, file_id },
        archive: path.file_name().map(|n| n.to_string_lossy().into_owned()),
        ..Default::default()
    };
    let built = install_archive(env, &path, Build { entry, replace: req.replace, previous: None })?;
    Ok(Outcome::Installed { name: built.name, updated: req.updating, dependencies: 0 })
}

struct ThunderstoreRequest {
    namespace: String,
    name: String,
    version: Option<String>,
    /// The game's community.
    community: Option<String>,
    updating: bool,
}

/// The package and what it needs, dependencies first, each as a mod of its own.
fn install_thunderstore(env: &JobEnv, req: ThunderstoreRequest) -> Result<Outcome, String> {
    env.stage(JobStage::Downloading);
    let list = env.shared.ensure_loaded(&env.root, &env.key, &env.game.title);
    let plan = thunderstore::resolve(&list, &req.namespace, &req.name, req.version.as_deref(), &mut |ns, n| thunderstore::package(ns, n))?;
    let dependencies = plan.iter().filter(|p| p.is_dependency).count();
    env.dependencies(dependencies);
    let community = req
        .community
        .clone()
        .or_else(|| list.links.thunderstore.clone())
        .or_else(|| heronite_mods::links::thunderstore_for(&env.root.cache_dir(), env.game.steam_app_id.as_deref(), &env.game.title))
        .unwrap_or_default();
    let mut installed = None;
    for planned in plan {
        if env.cancelled() {
            return Err(cancelled());
        }
        env.name(&readable(&planned.name));
        let path = fetch(env, &planned.version.download_url, &[], None)?;
        let entry = ModEntry {
            id: format!("ts-{}-{}", planned.namespace.to_lowercase(), planned.name.to_lowercase()),
            name: readable(&planned.name),
            version: Some(planned.version.version_number.clone()),
            author: Some(planned.namespace.clone()),
            summary: Some(planned.version.description.clone()).filter(|d| !d.is_empty()),
            picture: Some(planned.version.icon.clone()).filter(|i| !i.is_empty()),
            enabled: true,
            source: ModSource::Thunderstore {
                community: community.clone(),
                namespace: planned.namespace.clone(),
                name: planned.name.clone(),
            },
            archive: path.file_name().map(|n| n.to_string_lossy().into_owned()),
            depends_on: planned.version.dependencies.iter().map(|d| format!("{}-{}", d.namespace, d.name)).collect(),
            ..Default::default()
        };
        let built = install_archive(env, &path, Build { entry, replace: planned.replaces.clone(), previous: None })?;
        if !planned.is_dependency {
            installed = Some(built.name);
        }
    }
    Ok(Outcome::Installed { name: installed.unwrap_or_else(|| readable(&req.name)), updated: req.updating, dependencies })
}

/// Thunderstore names use `_` for spaces.
fn readable(name: &str) -> String {
    name.replace('_', " ")
}

impl ModsService {
    pub(super) fn start_path_install(&mut self, game_key: &str, path: PathBuf, cx: &mut Context<Self>) {
        let path = std::path::absolute(&path).unwrap_or(path);
        let name = name_of_file(&path);
        let entry = ModEntry {
            name: name.clone(),
            enabled: true,
            source: ModSource::File,
            archive: Some(path.to_string_lossy().into_owned()),
            ..Default::default()
        };
        self.start_job(game_key, name, cx, move |env| {
            let built = install_archive(env, &path, Build { entry, replace: None, previous: None })?;
            Ok(Outcome::Installed { name: built.name, updated: false, dependencies: 0 })
        });
    }

    pub(super) fn start_nexus_install(&mut self, game_key: &str, mod_id: u64, file_id: Option<u64>, cx: &mut Context<Self>) {
        let Some(domain) = self.games.get(game_key).and_then(|g| g.info.nexus_domain.clone()) else {
            self.notice(game_key, tr!("This game is not linked to a game on Nexus Mods yet"), true, cx);
            return;
        };
        let req = NexusRequest { domain, mod_id, file_id, link: None, replace: None, updating: false };
        self.start_job(game_key, tr!("Nexus Mods mod {id}", id = mod_id), cx, move |env| install_nexus(env, req));
    }

    /// A Nexus download link (the key and expiry let a free account through).
    pub(super) fn start_nexus_link(&mut self, game_key: &str, link: NxmLink, cx: &mut Context<Self>) {
        let req = NexusRequest {
            domain: link.domain.clone(),
            mod_id: link.mod_id,
            file_id: Some(link.file_id),
            link: Some(link.clone()),
            replace: None,
            updating: false,
        };
        self.start_job(game_key, tr!("Nexus Mods mod {id}", id = link.mod_id), cx, move |env| install_nexus(env, req));
    }

    pub(super) fn start_thunderstore_install(&mut self, game_key: &str, namespace: &str, name: &str, cx: &mut Context<Self>) {
        self.start_thunderstore(game_key, namespace, name, None, false, cx);
    }

    pub(super) fn start_thunderstore(
        &mut self,
        game_key: &str,
        namespace: &str,
        name: &str,
        version: Option<String>,
        updating: bool,
        cx: &mut Context<Self>,
    ) {
        let community = self.games.get(game_key).and_then(|g| g.info.thunderstore.clone());
        let req = ThunderstoreRequest { namespace: namespace.to_string(), name: name.to_string(), version, community, updating };
        self.start_job(game_key, readable(name), cx, move |env| install_thunderstore(env, req));
    }

    pub(super) fn start_update(&mut self, game_key: &str, mod_id: &str, cx: &mut Context<Self>) {
        let Some(entry) = self.games.get(game_key).and_then(|g| g.list.get(mod_id)).cloned() else { return };
        let Some(update) = entry.update.clone() else {
            self.notice(game_key, tr!("{name} has no newer version", name = entry.name.clone()), false, cx);
            return;
        };
        match entry.source.clone() {
            ModSource::Nexus { domain, mod_id: nexus_mod, .. } => {
                let Some(file_id) = update.file_id else {
                    self.notice(
                        game_key,
                        tr!("{name} has several newer files: pick one on its Nexus Mods page", name = entry.name.clone()),
                        true,
                        cx,
                    );
                    return;
                };
                let req = NexusRequest {
                    domain,
                    mod_id: nexus_mod,
                    file_id: Some(file_id),
                    link: None,
                    replace: Some(entry.id.clone()),
                    updating: true,
                };
                self.start_job(game_key, entry.name.clone(), cx, move |env| install_nexus(env, req));
            }
            ModSource::Thunderstore { namespace, name, .. } => {
                self.start_thunderstore(game_key, &namespace, &name, Some(update.version), true, cx);
            }
            ModSource::File => self.notice(game_key, tr!("A mod added from a file cannot be updated from here"), true, cx),
        }
    }

    pub(super) fn start_reinstall(&mut self, game_key: &str, mod_id: &str, cx: &mut Context<Self>) {
        let Some(entry) = self.games.get(game_key).and_then(|g| g.list.get(mod_id)).cloned() else { return };
        let Some(archive) = entry.archive.clone() else {
            self.notice(
                game_key,
                tr!("{name} has no kept file to install again: add the mod once more", name = entry.name.clone()),
                true,
                cx,
            );
            return;
        };
        let downloads = self.root.game(game_key).downloads();
        let name = entry.name.clone();
        let previous = entry.fomod.clone();
        let replace = Some(entry.id.clone());
        let entry = ModEntry { update: None, ..entry };
        self.start_job(game_key, name, cx, move |env| {
            let path = if Path::new(&archive).is_absolute() { PathBuf::from(&archive) } else { downloads.join(&archive) };
            if !path.exists() {
                return Err(tr!("{path} is no longer there: add the mod once more", path = path.display()));
            }
            let built = install_archive(env, &path, Build { entry, replace, previous })?;
            Ok(Outcome::Installed { name: built.name, updated: false, dependencies: 0 })
        });
    }

    pub(super) fn notice(&mut self, game_key: &str, message: impl Into<String>, error: bool, cx: &mut Context<Self>) {
        cx.emit(ModsEvent::Notice { game_key: game_key.to_string(), message: message.into(), error });
    }
}

#[cfg(test)]
mod tests {
    use super::{file_state, name_of_file};
    use heronite_mods::fomod::FileState;
    use std::path::Path;

    #[test]
    fn files_are_called_by_their_name_without_the_extension() {
        assert_eq!(name_of_file(Path::new("/d/SkyUI_5_2_SE-12604-5-2SE.7z")), "SkyUI_5_2_SE-12604-5-2SE");
        assert_eq!(name_of_file(Path::new("/d/Pack.tar.gz")), "Pack");
        assert_eq!(name_of_file(Path::new("/d/My Mod folder")), "My Mod folder");
    }

    #[test]
    fn installer_conditions_see_the_game_and_the_enabled_mods_whatever_the_case() {
        let t = tempfile::tempdir().unwrap();
        let game = t.path().join("game");
        let other = t.path().join("mod");
        std::fs::create_dir_all(game.join("Data")).unwrap();
        std::fs::write(game.join("Data/Skyrim.esm"), b"x").unwrap();
        std::fs::create_dir_all(other.join("Data/SKSE/Plugins")).unwrap();
        std::fs::write(other.join("Data/SKSE/Plugins/engine.dll"), b"x").unwrap();
        let state = file_state(Some(game), "Data".into(), vec![other]);
        assert_eq!(state("skyrim.ESM"), FileState::Active);
        assert_eq!(state("skse\\plugins\\Engine.dll"), FileState::Active);
        assert_eq!(state("Update.esm"), FileState::Missing);
        assert_eq!(state(""), FileState::Missing);
    }
}
