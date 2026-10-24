//! What installing (or updating) a package takes: itself and the
//! dependencies not installed yet, dependencies first.

use std::collections::HashSet;

use super::{compare_versions, Package, Version};
use crate::http::now_iso;
use crate::{ModEntry, ModList, ModSource, UpdateInfo};

/// One package to install.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Planned {
    pub namespace: String,
    pub name: String,
    pub version: Version,
    /// Installed because something needs it, not because the user asked.
    pub is_dependency: bool,
    /// Id of the installed mod this one replaces (an update).
    pub replaces: Option<String>,
}

fn same(a: &str, b: &str) -> bool {
    a.eq_ignore_ascii_case(b)
}

fn installed<'a>(list: &'a ModList, namespace: &str, name: &str) -> Option<&'a ModEntry> {
    list.mods
        .iter()
        .find(|m| matches!(&m.source, ModSource::Thunderstore { namespace: n, name: p, .. } if same(n, namespace) && same(p, name)))
}

fn key(namespace: &str, name: &str) -> String {
    format!("{}-{}", namespace.to_ascii_lowercase(), name.to_ascii_lowercase())
}

/// The plan to install `namespace/name`, dependencies first, each package
/// once (cycles do not loop). `fetch(namespace, name)` gives a package with
/// its latest version; every package installs at that version.
///
/// The package itself is always planned: asking for it is asking to install,
/// update or redo it (`replaces` names the installed copy). A dependency
/// already installed at the required version or newer is left alone.
/// `version`, the one a link named, only appears in logs: the latest is
/// what gets installed, as r2modman does.
pub fn resolve(
    list: &ModList,
    namespace: &str,
    name: &str,
    version: Option<&str>,
    fetch: &mut dyn FnMut(&str, &str) -> Result<Package, String>,
) -> Result<Vec<Planned>, String> {
    let root = fetch(namespace, name)?;
    if let Some(wanted) = version.filter(|w| compare_versions(w, &root.latest.version_number).is_ne()) {
        log::debug!("thunderstore: {namespace}-{name}: {wanted} was asked, {} is the latest", root.latest.version_number);
    }
    let mut plan = Vec::new();
    let mut seen = HashSet::from([key(&root.namespace, &root.name), key(namespace, name)]);
    visit(list, root, false, fetch, &mut seen, &mut plan)?;
    Ok(plan)
}

fn visit(
    list: &ModList,
    package: Package,
    is_dependency: bool,
    fetch: &mut dyn FnMut(&str, &str) -> Result<Package, String>,
    seen: &mut HashSet<String>,
    plan: &mut Vec<Planned>,
) -> Result<(), String> {
    for dep in &package.latest.dependencies {
        if !seen.insert(key(&dep.namespace, &dep.name)) {
            continue;
        }
        let have = installed(list, &dep.namespace, &dep.name);
        if have.is_some_and(|m| compare_versions(m.version.as_deref().unwrap_or(""), &dep.version).is_ge()) {
            continue;
        }
        let dep_package = fetch(&dep.namespace, &dep.name)?;
        visit(list, dep_package, true, fetch, seen, plan)?;
    }
    let replaces = installed(list, &package.namespace, &package.name).map(|m| m.id.clone());
    plan.push(Planned { namespace: package.namespace, name: package.name, version: package.latest, is_dependency, replaces });
    Ok(())
}

/// The installed Thunderstore mods that have a newer version: `(mod id,
/// update)`. A package `fetch` fails for is skipped (offline, removed).
pub fn latest_versions(list: &ModList, fetch: &mut dyn FnMut(&str, &str) -> Result<Package, String>) -> Vec<(String, UpdateInfo)> {
    let checked_at = now_iso();
    let mut out = Vec::new();
    for m in &list.mods {
        let ModSource::Thunderstore { namespace, name, .. } = &m.source else { continue };
        match fetch(namespace, name) {
            Ok(p) if compare_versions(&p.latest.version_number, m.version.as_deref().unwrap_or("")).is_gt() => {
                out.push((m.id.clone(), UpdateInfo { version: p.latest.version_number, file_id: None, checked_at: checked_at.clone() }));
            }
            Ok(_) => {}
            Err(e) => log::debug!("thunderstore: no update check for {namespace}-{name}: {e}"),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::super::DepRef;
    use super::*;

    fn dep(s: &str) -> DepRef {
        DepRef::parse(s).unwrap()
    }

    fn package(ns: &str, name: &str, version: &str, deps: &[&str]) -> Package {
        Package {
            namespace: ns.into(),
            name: name.into(),
            latest: Version { version_number: version.into(), dependencies: deps.iter().map(|d| dep(d)).collect(), ..Default::default() },
            ..Default::default()
        }
    }

    fn site(packages: Vec<Package>) -> HashMap<String, Package> {
        packages.into_iter().map(|p| (key(&p.namespace, &p.name), p)).collect()
    }

    fn installed_mod(id: &str, ns: &str, name: &str, version: &str) -> ModEntry {
        ModEntry {
            id: id.into(),
            version: Some(version.into()),
            source: ModSource::Thunderstore { community: "c".into(), namespace: ns.into(), name: name.into() },
            ..Default::default()
        }
    }

    fn run(list: &ModList, site: &HashMap<String, Package>, ns: &str, name: &str) -> (Result<Vec<Planned>, String>, Vec<String>) {
        let mut asked = Vec::new();
        let result = resolve(list, ns, name, None, &mut |n, p| {
            asked.push(format!("{n}-{p}"));
            site.get(&key(n, p)).cloned().ok_or_else(|| format!("no {n}-{p}"))
        });
        (result, asked)
    }

    fn names(plan: &[Planned]) -> Vec<String> {
        plan.iter().map(|p| format!("{}{}", p.name, if p.is_dependency { "*" } else { "" })).collect()
    }

    #[test]
    fn dependencies_come_first_and_once() {
        let site = site(vec![
            package("me", "Mod", "1.0.0", &["BepInEx-BepInExPack-5.4.2100", "lib-Lib-1.0.0"]),
            package("lib", "Lib", "1.2.0", &["BepInEx-BepInExPack-5.4.2100"]),
            package("BepInEx", "BepInExPack", "5.4.2305", &[]),
        ]);
        let (plan, _) = run(&ModList::default(), &site, "me", "Mod");
        let plan = plan.unwrap();
        assert_eq!(names(&plan), ["BepInExPack*", "Lib*", "Mod"]);
        assert_eq!(plan[0].version.version_number, "5.4.2305");
        assert!(plan.iter().all(|p| p.replaces.is_none()));
    }

    #[test]
    fn installed_dependencies_are_skipped_when_new_enough() {
        let site = site(vec![
            package("me", "Mod", "1.0.0", &["bepinex-bepinexpack-5.4.2100", "lib-Lib-1.3.0"]),
            package("BepInEx", "BepInExPack", "5.4.2305", &[]),
            package("lib", "Lib", "1.4.0", &[]),
        ]);
        let mut list = ModList::default();
        list.mods.push(installed_mod("ts-BepInEx-BepInExPack", "BepInEx", "BepInExPack", "5.4.2100"));
        list.mods.push(installed_mod("ts-lib-Lib", "lib", "Lib", "1.2.0"));
        let (plan, asked) = run(&list, &site, "me", "Mod");
        let plan = plan.unwrap();
        // The pack is current enough, Lib 1.2.0 is older than 1.3.0 and is updated.
        assert_eq!(names(&plan), ["Lib*", "Mod"]);
        assert_eq!(plan[0].replaces.as_deref(), Some("ts-lib-Lib"));
        assert_eq!(plan[0].version.version_number, "1.4.0");
        assert!(!asked.iter().any(|a| a.contains("BepInExPack")), "{asked:?}");
    }

    #[test]
    fn the_asked_package_always_installs_and_names_its_old_copy() {
        let site = site(vec![package("me", "Mod", "2.0.0", &[])]);
        let mut list = ModList::default();
        list.mods.push(installed_mod("ts-me-Mod", "ME", "mod", "2.0.0"));
        let plan = run(&list, &site, "me", "Mod").0.unwrap();
        assert_eq!((names(&plan), plan[0].replaces.as_deref()), (vec!["Mod".to_string()], Some("ts-me-Mod")));
    }

    #[test]
    fn cycles_end() {
        let site = site(vec![package("a", "A", "1.0.0", &["b-B-1.0.0"]), package("b", "B", "1.0.0", &["a-A-1.0.0", "b-B-1.0.0"])]);
        let (plan, asked) = run(&ModList::default(), &site, "a", "A");
        assert_eq!(names(&plan.unwrap()), ["B*", "A"]);
        assert_eq!(asked, ["a-A", "b-B"]);
    }

    #[test]
    fn a_failing_fetch_fails_the_plan() {
        let site = site(vec![package("a", "A", "1.0.0", &["gone-Gone-1.0.0"])]);
        assert_eq!(run(&ModList::default(), &site, "a", "A").0, Err("no gone-Gone".to_string()));
    }

    #[test]
    fn updates_are_the_newer_latest_versions() {
        let mut list = ModList::default();
        list.mods.push(installed_mod("old", "a", "A", "1.0.0"));
        list.mods.push(installed_mod("same", "b", "B", "2.0.0"));
        list.mods.push(installed_mod("gone", "c", "C", "1.0.0"));
        list.mods.push(ModEntry { id: "file".into(), ..Default::default() });
        let site = site(vec![package("a", "A", "1.10.0", &[]), package("b", "B", "2.0.0", &[])]);
        let found = latest_versions(&list, &mut |n, p| site.get(&key(n, p)).cloned().ok_or_else(|| "offline".to_string()));
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].0.as_str(), found[0].1.version.as_str(), found[0].1.file_id), ("old", "1.10.0", None));
    }
}
