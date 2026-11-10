//! What `ModuleConfig.xml` says, parsed and sorted but not yet evaluated.

use std::path::PathBuf;

use super::deps::Dependency;

/// A loaded FOMOD installer.
#[derive(Debug, Clone, Default)]
pub struct Installer {
    pub info: Info,
    /// Conditions on the game and its mods that must hold to install at all.
    pub module_dependencies: Option<Dependency>,
    /// Installed whatever the user picks.
    pub required: Vec<FileItem>,
    /// In the order the wizard shows them (the `order` attribute applied).
    pub steps: Vec<Step>,
    pub conditional: Vec<ConditionalInstall>,
}

/// `fomod/info.xml` plus the module's name and picture.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Info {
    pub name: String,
    pub author: String,
    pub version: String,
    pub website: String,
    pub description: String,
    /// The `moduleImage`, found in the archive.
    pub image: Option<PathBuf>,
}

#[derive(Debug, Clone)]
pub struct Step {
    pub name: String,
    /// The step is shown only when this holds with the flags set so far.
    pub visible: Option<Dependency>,
    pub groups: Vec<Group>,
}

#[derive(Debug, Clone)]
pub struct Group {
    pub name: String,
    pub kind: GroupKind,
    pub plugins: Vec<Plugin>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GroupKind {
    SelectAtLeastOne,
    SelectAtMostOne,
    SelectExactlyOne,
    SelectAll,
    SelectAny,
}

/// An option of a group.
#[derive(Debug, Clone)]
pub struct Plugin {
    pub name: String,
    pub description: String,
    pub image: Option<PathBuf>,
    pub files: Vec<FileItem>,
    /// Set while the plugin is selected.
    pub flags: Vec<(String, String)>,
    pub kind: TypeDescriptor,
}

/// How an option presents itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PluginType {
    Required,
    Recommended,
    Optional,
    NotUsable,
    CouldBeUsable,
}

#[derive(Debug, Clone)]
pub enum TypeDescriptor {
    Static(PluginType),
    /// The first pattern whose dependencies hold, else the default.
    Dynamic {
        default: PluginType,
        patterns: Vec<(Dependency, PluginType)>,
    },
}

/// A `file` or `folder` to copy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileItem {
    /// Relative to the content root, `/` separated, as written.
    pub source: String,
    /// Relative to the mod root, `/` separated; `""` = the mod root. A folder's
    /// contents go there.
    pub destination: String,
    pub is_folder: bool,
    pub priority: i64,
    /// Installed even when its plugin is not selected.
    pub always_install: bool,
    /// Installed when its plugin is not selected but is usable.
    pub install_if_usable: bool,
}

#[derive(Debug, Clone)]
pub struct ConditionalInstall {
    pub when: Dependency,
    pub files: Vec<FileItem>,
}
