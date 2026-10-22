//! `ModuleConfig.xml` and `info.xml` to [`Installer`]. Element and attribute
//! names are matched ignoring case: hand-written files are not always right
//! about it, and the installers that ship with the games' tools are lenient.

use std::path::Path;

use roxmltree::{Document, Node, ParsingOptions};

use super::deps::{Dependency, FileState};
use super::resolve;
use super::schema::*;
use super::text::{decode, escape_bare_ampersands};

type Elem<'a, 'i> = Node<'a, 'i>;

/// Parses the XML in `bytes`, retrying with bare `&` escaped, and reads it
/// with `read`.
pub(super) fn with_document<T>(bytes: &[u8], read: impl FnOnce(&Document) -> T) -> Result<T, String> {
    let text = decode(bytes);
    let options = || ParsingOptions { allow_dtd: true, ..Default::default() };
    match Document::parse_with_options(&text, options()) {
        Ok(doc) => Ok(read(&doc)),
        Err(first) => {
            let fixed = escape_bare_ampersands(&text);
            match Document::parse_with_options(&fixed, options()) {
                Ok(doc) => Ok(read(&doc)),
                Err(_) => Err(first.to_string()),
            }
        }
    }
}

fn is(n: Elem, name: &str) -> bool {
    n.is_element() && n.tag_name().name().eq_ignore_ascii_case(name)
}

fn kids<'a, 'i>(n: Elem<'a, 'i>, name: &'a str) -> impl Iterator<Item = Elem<'a, 'i>> {
    n.children().filter(move |c| is(*c, name))
}

fn kid<'a, 'i>(n: Elem<'a, 'i>, name: &'a str) -> Option<Elem<'a, 'i>> {
    kids(n, name).next()
}

fn attr<'a>(n: Elem<'a, '_>, name: &str) -> Option<&'a str> {
    n.attributes().find(|a| a.name().eq_ignore_ascii_case(name)).map(|a| a.value())
}

fn text(n: Elem) -> String {
    n.children().filter(|c| c.is_text()).filter_map(|c| c.text()).collect::<String>().trim().to_string()
}

fn truthy(n: Elem, name: &str) -> bool {
    attr(n, name).is_some_and(|v| matches!(v.trim().to_ascii_lowercase().as_str(), "true" | "1" | "yes"))
}

/// `\` and `/` both separate; no leading `./`, no repeated or trailing
/// separators.
pub(super) fn normalize_path(raw: &str) -> String {
    let parts: Vec<&str> = raw.trim().split(['/', '\\']).filter(|p| !p.is_empty() && *p != ".").collect();
    parts.join("/")
}

/// Sorts `items` by name as the `order` attribute says; no attribute means
/// ascending, as the schema does.
fn sort_by_order<T>(items: &mut [T], order: Option<&str>, name: impl Fn(&T) -> String) {
    match order.map(|o| o.trim().to_ascii_lowercase()).as_deref() {
        Some("explicit") => {}
        Some("descending") => items.sort_by_key(|i| std::cmp::Reverse(name(i).to_lowercase())),
        _ => items.sort_by_key(|i| name(i).to_lowercase()),
    }
}

pub(super) struct Reader<'p> {
    pub root: &'p Path,
}

impl Reader<'_> {
    pub fn module_config(&self, doc: &Document) -> Installer {
        let config = doc.root_element();
        let mut installer = Installer::default();
        installer.info.name = kid(config, "moduleName").map(text).unwrap_or_default();
        installer.info.image = kid(config, "moduleImage").and_then(|n| self.image(n));
        installer.module_dependencies = kid(config, "moduleDependencies").map(dependency);
        installer.required = kid(config, "requiredInstallFiles").map(files).unwrap_or_default();
        if let Some(steps) = kid(config, "installSteps") {
            installer.steps = kids(steps, "installStep").map(|s| self.step(s)).collect();
            sort_by_order(&mut installer.steps, attr(steps, "order"), |s| s.name.clone());
        }
        if let Some(patterns) = kid(config, "conditionalFileInstalls").and_then(|c| kid(c, "patterns")) {
            installer.conditional = kids(patterns, "pattern")
                .map(|p| ConditionalInstall {
                    when: kid(p, "dependencies").map(dependency).unwrap_or(Dependency::Satisfied),
                    files: kid(p, "files").map(files).unwrap_or_default(),
                })
                .collect();
        }
        installer
    }

    /// Fills `info` from `info.xml`'s `Name`, `Author`, `Version`, `Website`
    /// and `Description`.
    pub fn info_xml(&self, doc: &Document, info: &mut Info) {
        for n in doc.root_element().children().filter(|c| c.is_element()) {
            let slot = match n.tag_name().name().to_ascii_lowercase().as_str() {
                "name" => &mut info.name,
                "author" => &mut info.author,
                "version" => &mut info.version,
                "website" => &mut info.website,
                "description" => &mut info.description,
                _ => continue,
            };
            let value = text(n);
            if !value.is_empty() {
                *slot = value;
            }
        }
    }

    fn image(&self, n: Elem) -> Option<std::path::PathBuf> {
        resolve(self.root, attr(n, "path")?)
    }

    fn step(&self, s: Elem) -> Step {
        let mut groups = Vec::new();
        let mut order = None;
        if let Some(container) = kid(s, "optionalFileGroups") {
            order = attr(container, "order");
            groups = kids(container, "group").map(|g| self.group(g)).collect();
        }
        sort_by_order(&mut groups, order, |g: &Group| g.name.clone());
        Step { name: attr(s, "name").unwrap_or("").trim().to_string(), visible: kid(s, "visible").map(dependency), groups }
    }

    fn group(&self, g: Elem) -> Group {
        let mut plugins = Vec::new();
        let mut order = None;
        if let Some(container) = kid(g, "plugins") {
            order = attr(container, "order");
            plugins = kids(container, "plugin").map(|p| self.plugin(p)).collect();
        }
        sort_by_order(&mut plugins, order, |p: &Plugin| p.name.clone());
        Group { name: attr(g, "name").unwrap_or("").trim().to_string(), kind: group_kind(attr(g, "type")), plugins }
    }

    fn plugin(&self, p: Elem) -> Plugin {
        let flags = kid(p, "conditionFlags")
            .map(|c| kids(c, "flag").filter_map(|f| Some((attr(f, "name")?.trim().to_string(), text(f)))).collect())
            .unwrap_or_default();
        Plugin {
            name: attr(p, "name").unwrap_or("").trim().to_string(),
            description: kid(p, "description").map(text).unwrap_or_default(),
            image: kid(p, "image").and_then(|n| self.image(n)),
            files: kid(p, "files").map(files).unwrap_or_default(),
            flags,
            kind: kid(p, "typeDescriptor").map(descriptor).unwrap_or(TypeDescriptor::Static(PluginType::Optional)),
        }
    }
}

fn group_kind(s: Option<&str>) -> GroupKind {
    match s.map(|s| s.trim().to_ascii_lowercase()).as_deref() {
        Some("selectatleastone") => GroupKind::SelectAtLeastOne,
        Some("selectatmostone") => GroupKind::SelectAtMostOne,
        Some("selectexactlyone") => GroupKind::SelectExactlyOne,
        Some("selectall") => GroupKind::SelectAll,
        _ => GroupKind::SelectAny,
    }
}

fn plugin_type(n: Option<Elem>) -> PluginType {
    match n.and_then(|n| attr(n, "name")).map(|s| s.trim().to_ascii_lowercase()).as_deref() {
        Some("required") => PluginType::Required,
        Some("recommended") => PluginType::Recommended,
        Some("notusable") => PluginType::NotUsable,
        Some("couldbeusable") => PluginType::CouldBeUsable,
        _ => PluginType::Optional,
    }
}

fn descriptor(td: Elem) -> TypeDescriptor {
    if let Some(dt) = kid(td, "dependencyType") {
        let patterns = kid(dt, "patterns")
            .map(|ps| {
                kids(ps, "pattern")
                    .map(|p| {
                        let when = kid(p, "dependencies").map(dependency).unwrap_or(Dependency::Satisfied);
                        (when, plugin_type(kid(p, "type")))
                    })
                    .collect()
            })
            .unwrap_or_default();
        return TypeDescriptor::Dynamic { default: plugin_type(kid(dt, "defaultType")), patterns };
    }
    TypeDescriptor::Static(plugin_type(kid(td, "type")))
}

/// `n` is a composite (`dependencies`, `visible`, `moduleDependencies`).
fn dependency(n: Elem) -> Dependency {
    let or = attr(n, "operator").is_some_and(|o| o.trim().eq_ignore_ascii_case("or"));
    let items = n
        .children()
        .filter(|c| c.is_element())
        .filter_map(|c| {
            let name = c.tag_name().name().to_ascii_lowercase();
            match name.as_str() {
                "dependencies" => Some(dependency(c)),
                "filedependency" => Some(Dependency::File {
                    file: normalize_path(attr(c, "file")?),
                    state: match attr(c, "state").map(|s| s.trim().to_ascii_lowercase()).as_deref() {
                        Some("inactive") => FileState::Inactive,
                        Some("missing") => FileState::Missing,
                        _ => FileState::Active,
                    },
                }),
                "flagdependency" => Some(Dependency::Flag {
                    flag: attr(c, "flag")?.trim().to_string(),
                    value: attr(c, "value").unwrap_or("").trim().to_string(),
                }),
                other if other.ends_with("dependency") => Some(Dependency::Satisfied),
                _ => None,
            }
        })
        .collect();
    Dependency::Group { or, items }
}

/// The `file` and `folder` children of `n`.
fn files(n: Elem) -> Vec<FileItem> {
    n.children().filter(|c| is(*c, "file") || is(*c, "folder")).filter_map(file_item).collect()
}

fn file_item(n: Elem) -> Option<FileItem> {
    let is_folder = is(n, "folder");
    let source = normalize_path(attr(n, "source")?);
    if source.is_empty() && !is_folder {
        return None;
    }
    let mut destination = match attr(n, "destination") {
        Some(d) => normalize_path(d),
        None => source.clone(),
    };
    // A file sent to a folder (nothing, or a trailing separator) keeps its name.
    let into_folder = attr(n, "destination").is_some_and(|d| d.trim().is_empty() || d.trim().ends_with(['/', '\\']));
    if !is_folder && into_folder {
        let name = source.rsplit('/').next().unwrap_or(&source);
        destination = if destination.is_empty() { name.to_string() } else { format!("{destination}/{name}") };
    }
    Some(FileItem {
        source,
        destination,
        is_folder,
        priority: attr(n, "priority").and_then(|p| p.trim().parse().ok()).unwrap_or(0),
        always_install: truthy(n, "alwaysInstall"),
        install_if_usable: truthy(n, "installIfUsable"),
    })
}
