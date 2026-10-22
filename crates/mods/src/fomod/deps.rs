//! `dependencies`, `visible` and `moduleDependencies`: conditions on files
//! (the game's and the other mods') and on the flags the wizard sets.

use std::collections::BTreeMap;

use heronite_i18n::tr;

/// What the app knows about a file a `fileDependency` names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileState {
    /// There and in use (a plugin that is on, a file of an enabled mod).
    Active,
    /// There but not in use.
    Inactive,
    Missing,
}

#[derive(Debug, Clone)]
pub enum Dependency {
    /// `operator="Or"` when `or`, else all of `items`; empty holds.
    Group {
        or: bool,
        items: Vec<Dependency>,
    },
    File {
        file: String,
        state: FileState,
    },
    /// An unset flag has the value `""`.
    Flag {
        flag: String,
        value: String,
    },
    /// Game, FOMM and script extender versions: taken as met, as the game is
    /// not run by the installer.
    Satisfied,
}

pub type Flags = BTreeMap<String, String>;
pub type FileLookup<'a> = &'a dyn Fn(&str) -> FileState;

impl Dependency {
    pub fn holds(&self, flags: &Flags, files: FileLookup) -> bool {
        match self {
            Dependency::Group { or, items } => {
                items.is_empty()
                    || if *or { items.iter().any(|d| d.holds(flags, files)) } else { items.iter().all(|d| d.holds(flags, files)) }
            }
            Dependency::File { file, state } => files(file) == *state,
            Dependency::Flag { flag, value } => flags.get(flag).map(String::as_str).unwrap_or("") == value,
            Dependency::Satisfied => true,
        }
    }

    /// Appends what keeps this from holding, one line per unmet file.
    pub fn explain_unmet(&self, flags: &Flags, files: FileLookup, out: &mut Vec<String>) {
        if self.holds(flags, files) {
            return;
        }
        match self {
            Dependency::Group { items, .. } => items.iter().for_each(|d| d.explain_unmet(flags, files, out)),
            Dependency::File { file, state } => out.push(match (state, files(file)) {
                (FileState::Active, FileState::Inactive) => tr!("{file} is installed but not enabled", file),
                (FileState::Active, _) => tr!("{file} is not installed", file),
                (FileState::Inactive, _) => tr!("{file} must be installed and not enabled", file),
                (FileState::Missing, _) => tr!("{file} must not be installed", file),
            }),
            Dependency::Flag { flag, value } => out.push(tr!("The option {flag} is not set to {value}", flag, value)),
            Dependency::Satisfied => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flags(pairs: &[(&str, &str)]) -> Flags {
        pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    fn files(name: &str) -> FileState {
        match name {
            "on.esp" => FileState::Active,
            "off.esp" => FileState::Inactive,
            _ => FileState::Missing,
        }
    }

    fn file(name: &str, state: FileState) -> Dependency {
        Dependency::File { file: name.into(), state }
    }

    #[test]
    fn flags_compare_exactly_and_unset_is_empty() {
        let f = flags(&[("a", "On")]);
        let dep = |v: &str| Dependency::Flag { flag: "a".into(), value: v.into() };
        assert!(dep("On").holds(&f, &files));
        assert!(!dep("on").holds(&f, &files));
        let unset = Dependency::Flag { flag: "z".into(), value: String::new() };
        assert!(unset.holds(&f, &files));
        assert!(!Dependency::Flag { flag: "z".into(), value: "x".into() }.holds(&f, &files));
    }

    #[test]
    fn groups_and_or_empty() {
        let f = Flags::new();
        let yes = file("on.esp", FileState::Active);
        let no = file("on.esp", FileState::Missing);
        assert!(Dependency::Group { or: false, items: vec![yes.clone(), yes.clone()] }.holds(&f, &files));
        assert!(!Dependency::Group { or: false, items: vec![yes.clone(), no.clone()] }.holds(&f, &files));
        assert!(Dependency::Group { or: true, items: vec![no.clone(), yes] }.holds(&f, &files));
        assert!(!Dependency::Group { or: true, items: vec![no.clone(), no] }.holds(&f, &files));
        assert!(Dependency::Group { or: false, items: vec![] }.holds(&f, &files));
    }

    #[test]
    fn explains_the_unmet_files_only() {
        let dep = Dependency::Group {
            or: false,
            items: vec![file("on.esp", FileState::Active), file("off.esp", FileState::Active), file("gone.esp", FileState::Active)],
        };
        let mut out = Vec::new();
        dep.explain_unmet(&Flags::new(), &files, &mut out);
        assert_eq!(out, vec!["off.esp is installed but not enabled", "gone.esp is not installed"]);
    }
}
