//! The Winetricks page as console rows: what the prefix has, a search typed
//! on the on-screen keyboard, the kind of components, and one row per
//! component (A installs it). B goes back to the tab.

use std::rc::Rc;

use gpui_kit::*;
use heronite_launch::wine_tools::WineTool;
use heronite_launch::winetricks::Category;

use super::super::winetricks::category_label;
use super::{item, Act, Item};
use crate::console::list::Row;
use crate::dialogs::settings_panel::SettingsPanel;
use crate::icons::Icon;
use heronite_i18n::tr;

/// Component rows drawn at once (the search narrows the rest).
const SHOWN: usize = 60;

impl SettingsPanel {
    pub(super) fn c_winetricks(&self, cx: &App) -> Vec<Item> {
        let Some(page) = self.winetricks_page() else { return Vec::new() };
        let busy = page.run.is_some();
        let mut v = vec![Self::c_header("Winetricks")];
        if let Some(run) = &page.run {
            let last = run.log.last().cloned().unwrap_or_default();
            v.push(Self::c_info("wt-run", &tr!("Installing {verb}\u{2026}", verb = run.verb), last));
            let mut cancel = Self::c_do("wt-cancel", tr!("Cancel"), Icon::Close, true, |this, _, cx| this.cancel_winetricks(cx));
            cancel.row = cancel.row.danger();
            v.push(cancel);
        }
        if let Some(n) = &self.notice {
            v.push(Self::c_info("notice", if n.error { tr!("Problem") } else { tr!("Done") }, n.text.clone()));
        }
        let have = if page.installed.is_empty() { tr!("Nothing yet").to_string() } else { page.installed.join(", ") };
        v.push(Self::c_info("wt-have", tr!("Installed in this prefix"), have));
        v.push(Self::c_do("wt-window", tr!("Open the winetricks window"), Icon::OpenInNew, !busy, |this, _, cx| {
            this.run_tool("winetricks", WineTool::Winetricks, cx)
        }));

        v.push(Self::c_header(tr!("Available")));
        v.push(self.c_text("wt.search", tr!("Search"), cx));
        let searching = !self.input_text("wt.search", cx).trim().is_empty();
        if !searching {
            let names = Category::ALL.iter().map(|c| SharedString::from(category_label(*c))).collect();
            let index = Category::ALL.iter().position(|c| *c == page.category).unwrap_or(0);
            v.push(item(
                Row::choice("c:wt-kind", tr!("Kind"), names, index).icon(Icon::Tune),
                Act::Choice(Rc::new(|this, i, cx| this.set_winetricks_category(Category::ALL[i.min(2)], cx))),
            ));
        }
        if page.catalog.is_none() {
            v.push(Self::c_info("wt-loading", tr!("Loading\u{2026}"), ""));
        }
        let (list, custom) = self.winetricks_matches(cx);
        if let Some(name) = custom {
            let verb = name.clone();
            let mut it =
                Self::c_do(&format!("wt-x-{name}"), &tr!("Install {verb}", verb = name), Icon::Download, !busy, move |this, _, cx| {
                    this.winetricks_install(verb.clone(), cx)
                });
            it.row = it.row.desc(tr!("Not in the list: install it by name"));
            v.push(it);
        }
        for verb in list.into_iter().take(SHOWN) {
            let have = page.installed.contains(&verb.name);
            let name = verb.name.clone();
            let ic = if have { Icon::Check } else { Icon::Download };
            let mut it = Self::c_do(&format!("wt-{name}"), &verb.name, ic, !busy && !have, move |this, _, cx| {
                this.winetricks_install(name.clone(), cx)
            });
            let desc = if have { format!("{} \u{b7} {}", tr!("Installed"), verb.title) } else { verb.title.clone() };
            it.row = it.row.desc(desc);
            v.push(it);
        }
        v
    }
}
