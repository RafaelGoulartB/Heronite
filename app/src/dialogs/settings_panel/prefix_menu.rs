//! The prefix menu of a game's Wine tab: pick one of the prefixes on this
//! machine (see `heronite_wine::prefixes`) or make a new one for the game,
//! instead of typing a folder.

use std::path::{Path, PathBuf};

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_i18n::tr;
use heronite_wine::prefixes::{self, Prefix, PrefixKind};

use super::SettingsPanel;

fn home() -> String {
    std::env::var("HOME").unwrap_or_else(|_| "/".into())
}

/// A folder name from a game title: no `/`, no leading dots, at most 60 characters.
pub(super) fn folder_name_for(title: &str) -> String {
    let cleaned: String = title.chars().map(|c| if c == '/' || c == '\\' || c.is_control() { ' ' } else { c }).collect();
    let name: String = cleaned.split_whitespace().collect::<Vec<_>>().join(" ").trim_start_matches('.').chars().take(60).collect();
    let name = name.trim().to_string();
    if name.is_empty() {
        "Game".into()
    } else {
        name
    }
}

/// `name`, or `name 2`, `name 3`... when that folder exists in `dir`.
fn free_name(dir: &Path, name: &str) -> String {
    (1..)
        .map(|n| if n == 1 { name.to_string() } else { format!("{name} {n}") })
        .find(|n| !dir.join(n).exists())
        .unwrap_or_else(|| name.to_string())
}

impl SettingsPanel {
    pub(super) fn load_prefix_list(&mut self, cx: &mut Context<Self>) {
        let paths = self.paths.clone();
        if crate::env::flag("HERONITE_DEBUG_DIALOG") {
            self.prefix_list = Some(prefixes::list(&paths, &home()));
            return;
        }
        cx.spawn(async move |this, cx| {
            let list = cx.background_spawn(async move { prefixes::list(&paths, &home()) }).await;
            this.update(cx, |this, cx| {
                this.prefix_list = Some(list);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// The prefixes the menu offers: the ones that exist, the default and the
    /// game's own (even before Wine has created them).
    pub(super) fn prefix_choices(&self) -> Vec<Prefix> {
        let current = self.current_prefix();
        let mut list: Vec<Prefix> = self
            .prefix_list
            .as_deref()
            .unwrap_or_default()
            .iter()
            .filter(|p| p.kind != PrefixKind::NotCreated || p.default || p.path == current)
            .cloned()
            .collect();
        list.sort_by_key(|p| p.name.to_lowercase());
        list
    }

    fn current_prefix(&self) -> PathBuf {
        PathBuf::from(self.text("winePrefix").trim().trim_end_matches('/'))
    }

    /// "Control · /data/Games/Prefixes" for an entry of the menu.
    pub(super) fn prefix_label(p: &Prefix) -> String {
        let parent = p.path.parent().map(|d| d.display().to_string()).unwrap_or_default();
        if p.default {
            tr!("{name} (default) · {folder}", name = p.name, folder = parent)
        } else {
            format!("{} \u{b7} {parent}", p.name)
        }
    }

    pub(super) fn set_prefix(&mut self, path: &Path, window: &mut Window, cx: &mut Context<Self>) {
        let text = path.to_string_lossy().into_owned();
        self.set_input("winePrefix", &text, window, cx);
        self.commit("winePrefix", cx);
    }

    /// Makes an empty prefix named after the game where new prefixes go and
    /// sets the game to it.
    pub(super) fn new_prefix_for_game(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let title = self.facts().map(|f| f.title.clone()).unwrap_or_default();
        let dir = prefixes::new_prefix_dir(&self.paths, &home());
        let name = free_name(&dir, &folder_name_for(&title));
        match prefixes::create(&self.paths, &home(), &name) {
            Ok(path) => {
                self.set_prefix(&path, window, cx);
                self.load_prefix_list(cx);
            }
            Err(e) => {
                self.notice = Some(super::Notice { text: format!("{e:#}").into(), error: true });
                cx.notify();
            }
        }
    }

    pub(super) fn prefix_menu(&mut self, cx: &mut Context<Self>) -> Div {
        let choices = self.prefix_choices();
        let current = self.current_prefix();
        let picked = choices.iter().find(|p| p.path == current);
        let label = match picked {
            Some(p) => Self::prefix_label(p),
            None if self.prefix_list.is_none() => tr!("Looking for prefixes\u{2026}").to_string(),
            None => tr!("Another folder (below)").to_string(),
        };
        let mut options: Vec<(SharedString, bool)> =
            choices.iter().map(|p| (SharedString::from(Self::prefix_label(p)), p.path == current)).collect();
        options.push((SharedString::from(tr!("New prefix for this game")), false));
        let paths: Vec<PathBuf> = choices.iter().map(|p| p.path.clone()).collect();
        self.select_row("prefix-pick", Some(tr!("Prefix")), label, options, self.prefix_list.is_none(), cx, move |this, i, window, cx| {
            match paths.get(i) {
                Some(path) => this.set_prefix(path, window, cx),
                None => this.new_prefix_for_game(window, cx),
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{folder_name_for, free_name};

    #[test]
    fn folder_names_come_from_the_title() {
        assert_eq!(folder_name_for("METAL GEAR SOLID V: THE PHANTOM PAIN"), "METAL GEAR SOLID V: THE PHANTOM PAIN");
        assert_eq!(folder_name_for("AC/DC  Live\n"), "AC DC Live");
        assert_eq!(folder_name_for("...hidden"), "hidden");
        assert_eq!(folder_name_for("  "), "Game");
        assert_eq!(folder_name_for(&"x".repeat(100)).len(), 60);
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("Control")).unwrap();
        std::fs::create_dir(dir.path().join("Control 2")).unwrap();
        assert_eq!(free_name(dir.path(), "Control"), "Control 3");
        assert_eq!(free_name(dir.path(), "Hades"), "Hades");
    }
}
