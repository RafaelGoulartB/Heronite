//! The entry points the app calls, on the `MetadataService` it already has:
//! load what is installed (at startup), inspect a package before the user
//! confirms, install, remove, list. Everything is blocking (reads files,
//! compiles a script): call it from a worker thread.
//!
//! An installed extension is on: it is installed only after the user
//! confirmed, having seen what it does ([`PackageInfo`]). What the user set
//! for it (on or off, options, order) lives in the settings like any
//! source's, keyed by its id: an update keeps it, and so does a removal
//! unless asked to forget it.

use std::path::Path;
use std::sync::Arc;

use heronite_data::settings;
use heronite_metadata::sources::{AppliesTo, Asked, GameKind, SourceHealth, SourceOption};
use heronite_metadata::{MetadataService, SourceDescriptor, SourceKind};

use crate::limits::Limits;
use crate::manifest::{one_line, Manifest};
use crate::package::{self, Package};
use crate::source::{BrokenExtension, ScriptSource};
use crate::store;
use crate::ExtensionError;

/// What the installer shows before anything is installed.
#[derive(Debug, Clone, PartialEq)]
pub struct PackageInfo {
    pub id: String,
    pub name: String,
    pub version: String,
    pub author: String,
    pub homepage: Option<String>,
    pub kind: SourceKind,
    /// The manifest's `description` (the "about" line), as written.
    pub description: String,
    /// The fields it fills (`ALL_FIELDS` ids).
    pub provides: Vec<String>,
    /// Who it is for.
    pub applies_to: AppliesTo,
    pub asked: Asked,
    pub game_kind: Option<GameKind>,
    /// The hosts it will contact (`*.example.org` = its subdomains,
    /// `http://` = without encryption too).
    pub hosts: Vec<String>,
    /// Further hosts its pictures come from.
    pub image_hosts: Vec<String>,
    /// Its rate (`2/s`, `30/min`).
    pub rate: String,
    /// The settings it asks for.
    pub settings: Vec<SourceOption>,
    pub adult: bool,
    pub sha256: String,
    pub has_icon: bool,
    pub readme: Option<String>,
    /// Keys of the manifest left out, for the details.
    pub warnings: Vec<String>,
    /// The version installed now, if one is.
    pub installed: Option<String>,
    /// Who made the version installed now, or the one removed while what
    /// the user set for it was kept. `None` with `author_changed`: an
    /// installed version whose manifest no longer reads.
    pub installed_author: Option<String>,
    /// That author is not this package's (the name is the author's own
    /// word: nothing proves who wrote either), or is unknown.
    pub author_changed: bool,
    /// Hosts (and picture hosts) it connects to that the version installed,
    /// or removed with its settings kept, did not.
    pub new_hosts: Vec<String>,
    /// Values are saved for its settings (keys included), which it would
    /// read once installed.
    pub saved_settings: bool,
    /// What installing it would do.
    pub action: InstallAction,
    /// Its descriptor, as Settings will list it.
    pub descriptor: SourceDescriptor,
}

impl PackageInfo {
    /// Whether the installer offers to keep or forget the saved settings:
    /// there are some, and it is made by someone else or connects to new
    /// hosts.
    pub fn asks_about_settings(&self) -> bool {
        self.saved_settings && (self.author_changed || !self.new_hosts.is_empty())
    }

    /// Whether the saved settings are kept unless the user says otherwise:
    /// not when another author, or new hosts, would receive them.
    pub fn keeps_settings_by_default(&self) -> bool {
        !self.author_changed && self.new_hosts.is_empty()
    }
}

/// What the user approved in the installer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Approval {
    /// [`PackageInfo::sha256`] of the package shown: another package found
    /// at the path is refused ([`ExtensionError::Changed`]).
    pub sha256: String,
    /// Keep the values saved for its settings.
    pub keep_settings: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstallAction {
    New,
    /// A newer version of the one installed.
    Update,
    /// The version installed, again: asks to confirm.
    Reinstall,
    /// An older version than the one installed: asks to confirm.
    Downgrade,
}

/// What [`ExtensionsExt::install_extension`] did.
#[derive(Debug, Clone, PartialEq)]
pub enum InstallOutcome {
    Installed(InstalledExtension),
    /// It replaced version `from`, keeping what the user set for it.
    Updated {
        from: String,
        extension: InstalledExtension,
    },
    /// The same or an older version than the installed one: nothing was
    /// done; call again with `confirmed` to go ahead.
    NeedsConfirmation {
        installed: String,
        offered: String,
        downgrade: bool,
    },
}

/// The state of an installed extension.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExtensionState {
    Loaded,
    /// Stopped after failing too often (until switched on again or updated).
    Failing(String),
    /// Could not be loaded.
    LoadFailed(String),
}

/// An installed extension, as the list of them shows it.
#[derive(Debug, Clone, PartialEq)]
pub struct InstalledExtension {
    pub id: String,
    /// The manifest's, or the id when it cannot be read.
    pub name: String,
    pub version: String,
    pub author: String,
    pub kind: Option<SourceKind>,
    /// ISO 8601 (UTC).
    pub installed_at: String,
    pub from: String,
    /// The list it was installed from (its address); none for a file.
    pub list: Option<String>,
    /// Where its script may send requests, as the manifest writes them.
    pub hosts: Vec<String>,
    /// Further hosts its pictures may come from.
    pub image_hosts: Vec<String>,
    pub sha256: String,
    pub state: ExtensionState,
}

fn version_order(installed: &str, offered: &semver::Version) -> InstallAction {
    match semver::Version::parse(installed) {
        Ok(v) if *offered > v => InstallAction::Update,
        Ok(v) if *offered == v => InstallAction::Reinstall,
        Ok(_) => InstallAction::Downgrade,
        Err(_) => InstallAction::Update,
    }
}

/// The installed manifest of `id`, if it reads.
fn installed_manifest(service: &MetadataService, id: &str) -> Option<Manifest> {
    let dir = store::root(&service.paths).join(id);
    let text = std::fs::read_to_string(dir.join(package::MANIFEST)).ok()?;
    Manifest::parse(&text).ok()
}

/// The author and hosts of what was there before `id`: the installed
/// version, else the one removed with its settings kept.
fn before(service: &MetadataService, id: &str) -> Option<store::Kept> {
    match installed_manifest(service, id) {
        Some(m) => Some(store::Kept { author: m.author.clone(), hosts: all_hosts(&m), extra: Default::default() }),
        None => store::read_kept(&service.paths, id),
    }
}

fn all_hosts(m: &Manifest) -> Vec<String> {
    m.network.hosts.iter().chain(&m.network.images).map(|h| h.text()).collect()
}

fn same_author(a: &str, b: &str) -> bool {
    a.trim().to_lowercase() == b.trim().to_lowercase()
}

/// Whether values are saved for any setting of `id`.
fn saved_settings(service: &MetadataService, id: &str) -> bool {
    service.metadata_settings().sources.get(id).is_some_and(|p| !p.options.is_empty())
}

fn info(service: &MetadataService, pkg: &Package) -> PackageInfo {
    let m = &pkg.manifest;
    let installed = installed_manifest(service, &m.id).map(|i| i.version.to_string());
    let action = installed.as_deref().map_or(InstallAction::New, |v| version_order(v, &m.version));
    let before = before(service, &m.id);
    // A folder is installed but its manifest no longer reads: who made it
    // is unknown, so the settings saved for it are not handed over unasked.
    let unknown = installed.is_none() && store::root(&service.paths).join(&m.id).is_dir();
    let new_hosts = before.as_ref().map_or_else(Vec::new, |b| all_hosts(m).into_iter().filter(|h| !b.hosts.contains(h)).collect());
    PackageInfo {
        id: m.id.clone(),
        name: m.name.clone(),
        version: m.version.to_string(),
        author: m.author.clone(),
        homepage: m.homepage.clone(),
        kind: m.kind,
        description: m.description.clone(),
        provides: m.provides.clone(),
        applies_to: m.applies_to.clone(),
        asked: m.asked,
        game_kind: m.game_kind.clone(),
        hosts: m.network.hosts.iter().map(|h| h.text()).collect(),
        image_hosts: m.network.images.iter().map(|h| h.text()).collect(),
        rate: m.network.rate.text(),
        settings: m.settings.clone(),
        adult: m.adult,
        sha256: pkg.sha256.clone(),
        has_icon: pkg.icon.is_some(),
        readme: pkg.readme.clone(),
        warnings: m.warnings.clone(),
        installed,
        author_changed: unknown || before.as_ref().is_some_and(|b| !same_author(&b.author, &m.author)),
        installed_author: if unknown { None } else { before.map(|b| b.author) },
        new_hosts,
        saved_settings: saved_settings(service, &m.id),
        action,
        descriptor: m.descriptor(),
    }
}

/// Extension entry points of the metadata service.
pub trait ExtensionsExt {
    /// Adds every installed extension to the registry; one that cannot be
    /// loaded is listed in the error state. Call once at startup.
    fn load_extensions(&self) -> Vec<InstalledExtension>;

    /// Reads and checks a package (a folder or a zip) and its script,
    /// without installing anything.
    fn inspect_extension(&self, path: &Path) -> Result<PackageInfo, ExtensionError>;

    /// Installs (or updates) the package at `path` and adds it to the
    /// registry: asked from the next fetch on. The same or an older
    /// version than the installed one needs `confirmed`. The values saved
    /// for its settings are forgotten when it comes from another author or
    /// connects to new hosts ([`PackageInfo::keeps_settings_by_default`]).
    fn install_extension(&self, path: &Path, confirmed: bool) -> Result<InstallOutcome, ExtensionError>;

    /// Installs what the installer showed and the user approved: the same
    /// package (by its SHA-256), any version, keeping or forgetting the
    /// saved settings as chosen.
    fn install_approved(&self, path: &Path, approval: &Approval) -> Result<InstallOutcome, ExtensionError>;

    /// Removes extension `id` (files and registry). What the user set for
    /// it stays in the settings for a later install, unless
    /// `forget_settings`.
    fn remove_extension(&self, id: &str, forget_settings: bool) -> Result<(), ExtensionError>;

    /// The installed extensions, by id.
    fn installed_extensions(&self) -> Vec<InstalledExtension>;
}

fn state_of(service: &MetadataService, id: &str) -> ExtensionState {
    match service.sources().health(id) {
        SourceHealth::Ok => ExtensionState::Loaded,
        SourceHealth::Failing(why) => ExtensionState::Failing(why),
        SourceHealth::LoadFailed(why) => ExtensionState::LoadFailed(why),
    }
}

fn installed_entry(service: &MetadataService, id: &str, dir: &Path) -> InstalledExtension {
    let record = store::read_record(dir).unwrap_or_default();
    let manifest = installed_manifest(service, id);
    InstalledExtension {
        id: id.to_owned(),
        name: manifest.as_ref().map_or_else(|| id.to_owned(), |m| m.name.clone()),
        version: manifest.as_ref().map_or_else(|| record.version.clone(), |m| m.version.to_string()),
        author: manifest.as_ref().map(|m| m.author.clone()).unwrap_or_default(),
        kind: manifest.as_ref().map(|m| m.kind),
        installed_at: record.installed_at,
        from: record.from,
        list: record.list,
        hosts: manifest.as_ref().map(|m| m.network.hosts.iter().map(|h| h.text()).collect()).unwrap_or_default(),
        image_hosts: manifest.as_ref().map(|m| m.network.images.iter().map(|h| h.text()).collect()).unwrap_or_default(),
        sha256: record.sha256,
        state: state_of(service, id),
    }
}

/// Loads the installed folder `dir` of `id` as a source.
fn load(id: &str, dir: &Path) -> Result<ScriptSource, Box<(Option<SourceDescriptor>, ExtensionError)>> {
    let pkg = package::read_installed(dir).map_err(|e| Box::new((None, e)))?;
    if pkg.manifest.id != id {
        let e = ExtensionError::Manifest(format!("id \"{}\" is not its folder's name, {id}", pkg.manifest.id));
        return Err(Box::new((None, e)));
    }
    let descriptor = pkg.manifest.descriptor();
    // A module that ran past its time while loading left a thread running:
    // it is not loaded again (nor started) until an update replaces it.
    let record = store::read_record(dir).unwrap_or_default();
    if let Some(why) = record.failing.as_deref().and_then(|f| f.strip_prefix(STUCK_LOADING)) {
        return Err(Box::new((Some(descriptor), ExtensionError::Script(why.to_owned()))));
    }
    ScriptSource::new(pkg.manifest, &pkg.script, Limits::default(), Some(dir.to_owned())).map_err(|e| {
        if let ExtensionError::Script(m) = &e {
            if crate::source::is_stuck(m) {
                store::set_failing(dir, &record.sha256, Some(&format!("{STUCK_LOADING}{m}")));
            }
        }
        Box::new((Some(descriptor), e))
    })
}

/// How `install.json`'s `failing` starts for a module that ran past its
/// time while loading.
const STUCK_LOADING: &str = "loading: ";

impl ExtensionsExt for MetadataService {
    fn load_extensions(&self) -> Vec<InstalledExtension> {
        store::recover(&self.paths);
        for (id, dir) in store::folders(&self.paths) {
            let added = match load(&id, &dir) {
                Ok(source) => {
                    for w in &source.manifest().warnings {
                        log::info!("Extension {id}: {w}");
                    }
                    self.add_source(Arc::new(source))
                }
                Err(failed) => {
                    let (descriptor, e) = *failed;
                    log::warn!("Extension {id} could not be loaded: {}", one_line(&e.to_string()));
                    self.add_source(Arc::new(BrokenExtension::new(&id, descriptor, &e)))
                }
            };
            if let Err(e) = added {
                log::warn!("Extension {id} left out: {}", one_line(&format!("{e:#}")));
            }
        }
        self.installed_extensions()
    }

    fn inspect_extension(&self, path: &Path) -> Result<PackageInfo, ExtensionError> {
        let pkg = package::read(path)?;
        ScriptSource::new(pkg.manifest.clone(), &pkg.script, Limits::default(), None)?;
        Ok(info(self, &pkg))
    }

    fn install_extension(&self, path: &Path, confirmed: bool) -> Result<InstallOutcome, ExtensionError> {
        install(self, path, confirmed, &path.display().to_string(), None, None)
    }

    fn install_approved(&self, path: &Path, approval: &Approval) -> Result<InstallOutcome, ExtensionError> {
        install(self, path, true, &path.display().to_string(), None, Some(approval))
    }

    fn remove_extension(&self, id: &str, forget_settings: bool) -> Result<(), ExtensionError> {
        // Who made it and where it connected, for a later install of the id.
        let kept =
            installed_manifest(self, id).map(|m| store::Kept { author: m.author.clone(), hosts: all_hosts(&m), extra: Default::default() });
        let registered = self.remove_source(id);
        let deleted = store::delete(&self.paths, id)?;
        if !registered && !deleted {
            return Err(ExtensionError::NotInstalled(id.to_owned()));
        }
        match kept {
            Some(k) if !forget_settings => store::write_kept(&self.paths, id, &k)?,
            _ => store::delete_kept(&self.paths, id),
        }
        if forget_settings {
            settings::update_metadata(&self.paths, |m| {
                m.sources.shift_remove(id);
                m.source_order.retain(|s| s != id);
                for list in m.field_priority.values_mut() {
                    list.retain(|s| s != id);
                }
            })
            .map_err(|e| ExtensionError::Io(format!("{e:#}")))?;
        }
        Ok(())
    }

    fn installed_extensions(&self) -> Vec<InstalledExtension> {
        store::folders(&self.paths).into_iter().map(|(id, dir)| installed_entry(self, &id, &dir)).collect()
    }
}

/// Installs the package at `path` (see [`ExtensionsExt::install_extension`]),
/// recording `from` and the `list` it comes from (a list's address).
/// With `approval`, only the package the user approved, keeping its saved
/// settings as they chose.
pub(crate) fn install(
    service: &MetadataService,
    path: &Path,
    confirmed: bool,
    from: &str,
    list: Option<&str>,
    approval: Option<&Approval>,
) -> Result<InstallOutcome, ExtensionError> {
    let pkg = package::read(path)?;
    if approval.is_some_and(|a| a.sha256 != pkg.sha256) {
        return Err(ExtensionError::Changed);
    }
    // The script must load before anything is written.
    let source = ScriptSource::new(pkg.manifest.clone(), &pkg.script, Limits::default(), None)?;
    let id = pkg.manifest.id.clone();
    let before = installed_manifest(service, &id).map(|m| m.version);
    if let Some(installed) = &before {
        let downgrade = pkg.manifest.version < *installed;
        if !confirmed && pkg.manifest.version <= *installed {
            return Ok(InstallOutcome::NeedsConfirmation {
                installed: installed.to_string(),
                offered: pkg.manifest.version.to_string(),
                downgrade,
            });
        }
    }
    // Read before the files are replaced: who made what was there.
    let shown = info(service, &pkg);
    let keep = approval.map_or_else(|| shown.keeps_settings_by_default(), |a| a.keep_settings);
    // What can fail goes before the files are replaced: an error then
    // leaves the version installed as it was.
    if !keep && shown.saved_settings {
        settings::update_metadata(&service.paths, |m| {
            if let Some(p) = m.sources.get_mut(&id) {
                p.options.clear();
            }
        })
        .map_err(|e| ExtensionError::Io(format!("{e:#}")))?;
    }
    let dir = store::write(&service.paths, &pkg, from, list)?;
    store::delete_kept(&service.paths, &id);
    service.add_source(Arc::new(source.installed_in(dir.clone()))).map_err(|e| ExtensionError::IdTaken(format!("{id}: {e:#}")))?;
    let extension = installed_entry(service, &id, &dir);
    Ok(match before {
        Some(from) => InstallOutcome::Updated { from: from.to_string(), extension },
        None => InstallOutcome::Installed(extension),
    })
}
