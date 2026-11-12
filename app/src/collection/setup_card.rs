//! The game page's Setup card: one row per group of the game's settings,
//! saying in words what this game sets (`summary.rs`), each opening the game
//! settings on its group; then the Mods and Save backup rows. It replaces
//! the gear next to the status. The rows are read off the UI thread when the
//! focus moves and again when a dialog closes.

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_i18n::tr;

use super::focus::panel;
use super::focus_left::{clickable, fact_row};
use super::vm::FocusVm;
use super::CollectionView;
use crate::dialogs::game_settings::Tab;
use crate::dialogs::settings_panel::summary::{self, SetupRow};
use crate::dialogs::settings_panel::Section;
use crate::icons::Icon;
use crate::theme;
use crate::ui::icon;

/// Below this window height the card shows only the groups the game changed
/// (and "Show all").
const SHORT_VH: f32 = 900.;

fn row_id(section: Section) -> &'static str {
    match section {
        Section::Start => "setup-start",
        Section::Compatibility => "setup-compat",
        Section::Graphics => "setup-graphics",
        Section::Performance => "setup-perf",
        Section::Saves => "setup-saves",
        Section::General => "setup-general",
    }
}

impl CollectionView {
    /// Reads the Setup rows of the selected game (the copy Play starts).
    pub(super) fn load_setup(&mut self, cx: &mut Context<Self>) {
        let Some(key) = self.selected.clone() else {
            self.facts.setup = None;
            return;
        };
        // Read again for the same game: the old rows stay until then.
        if self.facts.setup.as_ref().is_some_and(|(k, _)| *k != key) {
            self.facts.setup = None;
        }
        let Some(game) = self.acting_copy(&key) else { return };
        let paths = self.data_paths.clone();
        cx.spawn(async move |this, cx| {
            let rows = cx.background_spawn(async move { summary::read(&paths, &game) }).await;
            this.update(cx, |this, cx| {
                if this.selected.as_ref() == Some(&key) {
                    this.facts.setup = Some((key, rows));
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    /// The Setup rows of `key`, once read.
    pub(super) fn setup_rows(&self, key: &SharedString) -> &[SetupRow] {
        self.facts.setup.as_ref().filter(|(k, _)| k == key).map_or(&[], |(_, rows)| rows.as_slice())
    }

    /// "Setup" and its card; nothing when the game has no row.
    pub(super) fn setup_card(&self, vm: &FocusVm, vh: f32, cx: &mut Context<Self>) -> Option<AnyElement> {
        let setup = self.setup_rows(&vm.key);
        let (shown, hidden) = summary::folded(setup, vh < SHORT_VH, self.facts.setup_all);
        // The group the settings in the page show.
        let open = self.page_settings_tab(&vm.key, cx);
        let mut rows: Vec<AnyElement> = Vec::new();
        for r in shown {
            let tab = Tab::of_section(r.section);
            let current = open == Some(tab);
            let value = div()
                .flex()
                .items_center()
                .justify_end()
                .gap(px(4.))
                .child(div().min_w_0().truncate().text_color(theme::ink_a(if r.changed { 0.94 } else { 0.5 })).child(r.value.clone()))
                .child(icon(Icon::ChevronRight, px(14.), theme::ink_a(0.4)));
            rows.push(
                clickable(fact_row(r.label, value), row_id(r.section))
                    .when(current, |d| d.bg(theme::ink_a(0.08)).rounded(px(8.)))
                    .on_click(cx.listener(move |this, _, window, cx| this.open_game_settings_tab(tab, window, cx)))
                    .into_any_element(),
            );
        }
        if hidden {
            rows.push(
                clickable(fact_row(tr!("Show all"), div().child(icon(Icon::ExpandMore, px(14.), theme::ink_a(0.5)))), "setup-all")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.facts.setup_all = true;
                        cx.notify();
                    }))
                    .into_any_element(),
            );
        }
        rows.extend(self.mods_and_backup_rows(vm, cx));
        if rows.is_empty() {
            return None;
        }
        let mut card = panel(true).pt(px(6.)).pb(px(6.));
        let n = rows.len();
        for (i, r) in rows.into_iter().enumerate() {
            card = card.child(div().when(i + 1 < n, |d| d.border_b_1().border_color(theme::ink_a(0.08))).child(r));
        }
        let all = (!setup.is_empty()).then(|| {
            div()
                .id("setup-all-settings")
                .text_size(px(12.))
                .text_color(theme::accent())
                .cursor_pointer()
                .hover(|s| s.text_color(theme::accent_overlay()))
                .child(tr!("All settings"))
                .on_click(cx.listener(|this, _, window, cx| this.open_game_settings(window, cx)))
        });
        let head = div()
            .flex()
            .items_center()
            .justify_between()
            .px(px(4.))
            .child(div().text_size(px(11.)).font_weight(theme::W_BOLD).text_color(theme::ink_a(0.55)).child(tr!("Setup").to_uppercase()))
            .children(all);
        Some(div().flex().flex_col().gap(px(6.)).child(head).child(card).into_any_element())
    }
}
