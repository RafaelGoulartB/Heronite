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
use crate::manifest::Manifest;
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
    /// What installing it would do.
    pub action: InstallAction,
    /// Its descriptor, as Settings will list it.
    pub descriptor: SourceDescriptor,
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

fn info(service: &MetadataService, pkg: &Package) -> PackageInfo {
    let m = &pkg.manifest;
    let installed = installed_manifest(service, &m.id).map(|i| i.version.to_string());
    let action = installed.as_deref().map_or(InstallAction::New, |v| version_order(v, &m.version));
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
    /// version than the installed one needs `confirmed`.
    fn install_extension(&self, path: &Path, confirmed: bool) -> Result<InstallOutcome, ExtensionError>;

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
    ScriptSource::new(pkg.manifest, &pkg.script, Limits::default(), Some(dir.to_owned())).map_err(|e| Box::new((Some(descriptor), e)))
}

impl ExtensionsExt for MetadataService {
    fn load_extensions(&self) -> Vec<InstalledExtension> {
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
                    log::warn!("Extension {id} could not be loaded: {e}");
                    self.add_source(Arc::new(BrokenExtension::new(&id, descriptor, &e)))
                }
            };
            if let Err(e) = added {
                log::warn!("Extension {id} left out: {e:#}");
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
        let pkg = package::read(path)?;
        // The script must load before anything is written.
        ScriptSource::new(pkg.manifest.clone(), &pkg.script, Limits::default(), None)?;
        let id = pkg.manifest.id.clone();
        let before = installed_manifest(self, &id).map(|m| m.version);
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
        let dir = store::write(&self.paths, &pkg, &path.display().to_string())?;
        let source = ScriptSource::new(pkg.manifest.clone(), &pkg.script, Limits::default(), Some(dir.clone()))?;
        self.add_source(Arc::new(source)).map_err(|e| ExtensionError::IdTaken(format!("{id}: {e:#}")))?;
        let extension = installed_entry(self, &id, &dir);
        Ok(match before {
            Some(from) => InstallOutcome::Updated { from: from.to_string(), extension },
            None => InstallOutcome::Installed(extension),
        })
    }

    fn remove_extension(&self, id: &str, forget_settings: bool) -> Result<(), ExtensionError> {
        let registered = self.remove_source(id);
        let deleted = store::delete(&self.paths, id)?;
        if !registered && !deleted {
            return Err(ExtensionError::NotInstalled(id.to_owned()));
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
