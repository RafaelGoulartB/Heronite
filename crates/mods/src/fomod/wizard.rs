//! The installer's state while the user walks through it: which options are
//! picked, which flags that sets, which steps show, and the files in the end.

use std::collections::HashMap;
use std::sync::Mutex;

use super::deps::{Dependency, FileState, Flags};
use super::schema::*;
use crate::{CopyRule, FomodChoices, FomodGroupChoice, FomodStepChoice};

/// More rounds than a sane installer needs to settle (an option whose type
/// depends on its own flag may never settle; the last state stays).
const SETTLE_ROUNDS: usize = 8;

/// What the UI draws for the current step.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct StepView {
    pub name: String,
    pub groups: Vec<GroupView>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct GroupView {
    pub name: String,
    pub kind: GroupKind,
    pub plugins: Vec<PluginView>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PluginView {
    pub name: String,
    pub description: String,
    /// Found in the archive.
    pub image: Option<std::path::PathBuf>,
    /// The type with the flags and files of now.
    pub state: PluginType,
    pub selected: bool,
    /// False when the user cannot change it: required, not usable, or in a
    /// group that takes all. `Wizard::select` can still refuse a change that
    /// would break the group's rule.
    pub enabled: bool,
}

pub struct Wizard {
    installer: Installer,
    file_state: Box<dyn Fn(&str) -> FileState + Send>,
    /// Answers of `file_state`, which cannot change while the wizard is open.
    file_cache: Mutex<HashMap<String, FileState>>,
    previous: Option<FomodChoices>,
    /// `[step][group][plugin]`, for every step of the installer.
    picked: Vec<Vec<Vec<bool>>>,
    /// Steps whose defaults were already applied.
    started: Vec<bool>,
    /// Indexes of the steps shown, in order; the last is the current one.
    path: Vec<usize>,
    /// Set by the selected plugins of the steps on `path`.
    flags: Flags,
}

impl Wizard {
    /// `file_state` answers `fileDependency` (`Skyrim.esm`, `SKSE/…`).
    /// `previous` preselects the options of an earlier run by name.
    pub fn new(installer: Installer, file_state: Box<dyn Fn(&str) -> FileState + Send>, previous: Option<&FomodChoices>) -> Wizard {
        let picked = installer.steps.iter().map(|s| s.groups.iter().map(|g| vec![false; g.plugins.len()]).collect()).collect();
        let started = vec![false; installer.steps.len()];
        let mut wizard = Wizard {
            installer,
            file_state,
            file_cache: Mutex::new(HashMap::new()),
            previous: previous.cloned(),
            picked,
            started,
            path: Vec::new(),
            flags: Flags::new(),
        };
        if let Some(first) = wizard.next_visible(None) {
            wizard.enter(first);
        }
        wizard
    }

    pub fn info(&self) -> &Info {
        &self.installer.info
    }

    pub fn module_dependencies_met(&self) -> bool {
        self.installer.module_dependencies.as_ref().is_none_or(|d| d.holds(&Flags::new(), &|f| self.lookup(f)))
    }

    /// What `moduleDependencies` still wants, translated; empty when met.
    pub fn missing_module_dependencies(&self) -> Vec<String> {
        let mut out = Vec::new();
        if let Some(d) = &self.installer.module_dependencies {
            d.explain_unmet(&Flags::new(), &|f| self.lookup(f), &mut out);
        }
        out
    }

    pub fn step(&self) -> StepView {
        let Some(si) = self.current() else { return StepView::default() };
        let step = &self.installer.steps[si];
        let groups = step.groups.iter().enumerate().map(|(gi, g)| GroupView {
            name: g.name.clone(),
            kind: g.kind,
            plugins: g
                .plugins
                .iter()
                .enumerate()
                .map(|(pi, p)| {
                    let state = self.plugin_type(p);
                    PluginView {
                        name: p.name.clone(),
                        description: p.description.clone(),
                        image: p.image.clone(),
                        state,
                        selected: self.picked[si][gi][pi],
                        enabled: !is_locked(g.kind, state),
                    }
                })
                .collect(),
        });
        StepView { name: step.name.clone(), groups: groups.collect() }
    }

    /// Picks or clears an option of the current step as its group's rule
    /// allows (a radio group moves the pick, a required option stays on…).
    pub fn select(&mut self, group: usize, plugin: usize, on: bool) {
        let Some(si) = self.current() else { return };
        let Some(g) = self.installer.steps[si].groups.get(group) else { return };
        let Some(p) = g.plugins.get(plugin) else { return };
        let kind = g.kind;
        if is_locked(kind, self.plugin_type(p)) {
            return;
        }
        let types: Vec<PluginType> = g.plugins.iter().map(|p| self.plugin_type(p)).collect();
        let picked = &mut self.picked[si][group];
        match kind {
            GroupKind::SelectExactlyOne | GroupKind::SelectAtMostOne => {
                if on {
                    for (i, t) in types.iter().enumerate() {
                        picked[i] = *t == PluginType::Required;
                    }
                    picked[plugin] = true;
                } else if kind == GroupKind::SelectAtMostOne {
                    picked[plugin] = false;
                }
            }
            GroupKind::SelectAtLeastOne => {
                if on || picked.iter().filter(|b| **b).count() > 1 {
                    picked[plugin] = on;
                }
            }
            GroupKind::SelectAny => picked[plugin] = on,
            GroupKind::SelectAll => {}
        }
        self.settle();
    }

    /// Whether every group of the current step has what its rule asks.
    pub fn can_next(&self) -> bool {
        let Some(si) = self.current() else { return true };
        self.installer.steps[si].groups.iter().enumerate().all(|(gi, g)| {
            let count = self.picked[si][gi].iter().filter(|b| **b).count();
            // A group with nothing usable cannot be satisfied, so it is let go.
            let usable = g.plugins.iter().any(|p| self.plugin_type(p) != PluginType::NotUsable);
            match g.kind {
                GroupKind::SelectExactlyOne => count == 1 || !usable,
                GroupKind::SelectAtLeastOne => count >= 1 || !usable,
                GroupKind::SelectAtMostOne => count <= 1,
                GroupKind::SelectAll | GroupKind::SelectAny => true,
            }
        })
    }

    /// Another step shows after this one, going by the flags set so far.
    pub fn has_next(&self) -> bool {
        self.current().is_some_and(|si| self.next_visible(Some(si)).is_some())
    }

    // Named as the wizard's button; a step is not an iterator item.
    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> bool {
        let Some(si) = self.current().filter(|_| self.can_next()) else { return false };
        match self.next_visible(Some(si)) {
            Some(next) => {
                self.enter(next);
                true
            }
            None => false,
        }
    }

    pub fn has_back(&self) -> bool {
        self.path.len() > 1
    }

    /// Back to the previous step; the flags of the one left are unset.
    pub fn back(&mut self) -> bool {
        if !self.has_back() {
            return false;
        }
        self.path.pop();
        self.settle();
        true
    }

    /// (position of the current step among those shown, how many there are
    /// going by the flags now: later steps may still hide or show).
    pub fn step_index(&self) -> (usize, usize) {
        let Some(si) = self.current() else { return (0, 0) };
        let ahead = (si + 1..self.installer.steps.len()).filter(|&i| self.visible(i)).count();
        (self.path.len(), self.path.len() + ahead)
    }

    /// The options picked in the steps shown so far, by name.
    pub fn choices(&self) -> FomodChoices {
        let steps = self.path.iter().map(|&si| {
            let step = &self.installer.steps[si];
            let groups = step.groups.iter().enumerate().map(|(gi, g)| FomodGroupChoice {
                group: g.name.clone(),
                plugins: g.plugins.iter().zip(&self.picked[si][gi]).filter(|(_, on)| **on).map(|(p, _)| p.name.clone()).collect(),
            });
            FomodStepChoice { step: step.name.clone(), groups: groups.collect() }
        });
        FomodChoices { steps: steps.collect() }
    }

    pub fn flags(&self) -> &Flags {
        &self.flags
    }

    /// The copies the choices make, lowest priority first: the required
    /// files, the files of the options picked in the steps shown (plus those
    /// that install regardless), then the conditional installs the flags
    /// satisfy. Call it once the last step is reached.
    pub fn files(&self) -> Vec<CopyRule> {
        let mut items: Vec<&FileItem> = self.installer.required.iter().collect();
        for &si in &self.path {
            for (gi, g) in self.installer.steps[si].groups.iter().enumerate() {
                for (pi, p) in g.plugins.iter().enumerate() {
                    let picked = self.picked[si][gi][pi];
                    let usable = || self.plugin_type(p) != PluginType::NotUsable;
                    items.extend(p.files.iter().filter(|f| picked || f.always_install || (f.install_if_usable && usable())));
                }
            }
        }
        for c in &self.installer.conditional {
            if self.holds(&c.when) {
                items.extend(&c.files);
            }
        }
        // Stable: equal priorities keep the order above, and later wins.
        items.sort_by_key(|f| f.priority);
        items
            .into_iter()
            .map(|f| CopyRule {
                source: f.source.clone(),
                destination: f.destination.clone(),
                is_folder: f.is_folder,
                priority: f.priority,
            })
            .collect()
    }

    fn current(&self) -> Option<usize> {
        self.path.last().copied()
    }

    fn lookup(&self, file: &str) -> FileState {
        let key = file.to_lowercase();
        if let Some(state) = self.file_cache.lock().ok().and_then(|c| c.get(&key).copied()) {
            return state;
        }
        let state = (self.file_state)(file);
        if let Ok(mut cache) = self.file_cache.lock() {
            cache.insert(key, state);
        }
        state
    }

    fn holds(&self, dep: &Dependency) -> bool {
        dep.holds(&self.flags, &|f| self.lookup(f))
    }

    fn visible(&self, si: usize) -> bool {
        self.installer.steps[si].visible.as_ref().is_none_or(|d| self.holds(d))
    }

    fn next_visible(&self, after: Option<usize>) -> Option<usize> {
        (after.map_or(0, |i| i + 1)..self.installer.steps.len()).find(|&i| self.visible(i))
    }

    fn plugin_type(&self, plugin: &Plugin) -> PluginType {
        match &plugin.kind {
            TypeDescriptor::Static(t) => *t,
            TypeDescriptor::Dynamic { default, patterns } => {
                patterns.iter().find(|(when, _)| self.holds(when)).map_or(*default, |(_, t)| *t)
            }
        }
    }

    fn enter(&mut self, si: usize) {
        self.path.push(si);
        if !self.started[si] {
            self.started[si] = true;
            self.start_step(si);
        }
        self.settle();
    }

    /// Sets the defaults of a step shown for the first time: what the earlier
    /// run picked when the names match, else the types' suggestions.
    fn start_step(&mut self, si: usize) {
        let name = self.installer.steps[si].name.clone();
        let earlier = self.previous.as_ref().and_then(|p| p.steps.iter().find(|s| s.step == name)).cloned();
        for gi in 0..self.installer.steps[si].groups.len() {
            self.recompute_flags();
            let group = &self.installer.steps[si].groups[gi];
            let earlier = earlier.as_ref().and_then(|s| s.groups.iter().find(|g| g.group == group.name));
            let bits = group
                .plugins
                .iter()
                .map(|p| match earlier {
                    Some(e) => e.plugins.contains(&p.name),
                    None => {
                        group.kind == GroupKind::SelectAll || matches!(self.plugin_type(p), PluginType::Required | PluginType::Recommended)
                    }
                })
                .collect();
            self.picked[si][gi] = bits;
            self.enforce_group(si, gi);
        }
    }

    /// Applies what locks the current step's options (types that changed with
    /// the flags) until it stops changing.
    fn settle(&mut self) {
        self.recompute_flags();
        let Some(si) = self.current() else { return };
        for _ in 0..SETTLE_ROUNDS {
            let before = self.picked[si].clone();
            for gi in 0..before.len() {
                self.recompute_flags();
                self.enforce_group(si, gi);
            }
            self.recompute_flags();
            if before == self.picked[si] {
                break;
            }
        }
    }

    fn enforce_group(&mut self, si: usize, gi: usize) {
        let group = &self.installer.steps[si].groups[gi];
        let kind = group.kind;
        let types: Vec<PluginType> = group.plugins.iter().map(|p| self.plugin_type(p)).collect();
        let picked = &mut self.picked[si][gi];
        for (i, t) in types.iter().enumerate() {
            match t {
                PluginType::Required => picked[i] = true,
                PluginType::NotUsable => picked[i] = false,
                _ if kind == GroupKind::SelectAll => picked[i] = true,
                _ => {}
            }
        }
        if matches!(kind, GroupKind::SelectExactlyOne | GroupKind::SelectAtMostOne) {
            let on: Vec<usize> = (0..picked.len()).filter(|&i| picked[i]).collect();
            if on.len() > 1 {
                let keep = on.iter().copied().find(|&i| types[i] == PluginType::Required).unwrap_or(on[0]);
                picked.iter_mut().enumerate().for_each(|(i, b)| *b = i == keep);
            } else if on.is_empty() && kind == GroupKind::SelectExactlyOne {
                if let Some(first) = types.iter().position(|t| *t != PluginType::NotUsable) {
                    picked[first] = true;
                }
            }
        }
    }

    fn recompute_flags(&mut self) {
        let mut flags = Flags::new();
        for &si in &self.path {
            for (gi, g) in self.installer.steps[si].groups.iter().enumerate() {
                for (pi, p) in g.plugins.iter().enumerate() {
                    if self.picked[si][gi][pi] {
                        flags.extend(p.flags.iter().cloned());
                    }
                }
            }
        }
        self.flags = flags;
    }
}

/// Options the user cannot toggle.
fn is_locked(kind: GroupKind, state: PluginType) -> bool {
    kind == GroupKind::SelectAll || matches!(state, PluginType::Required | PluginType::NotUsable)
}
