//! The Metadata menu of a selection: the dock's popover above its Metadata
//! button and the context menu's "Metadata" flyout share its rows. "Fill
//! what is missing" runs at once (the header pill then shows it; Enter does
//! it while the dock's menu is open); "Fix matches…" opens "Review matches"
//! on the selection; "Update metadata…" opens the "Update metadata" dialog on
//! the selection; "Download <family> covers" (one per family a view draws
//! that some of the selection show cropped) downloads them for those games.
//! What "Fill" would look at is counted
//! off the UI thread when the menu opens, once per selection and again after
//! the library reloads (a job, an undo or a settings change was saved).

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::cover_family::{CoverFamilies, CoverFamily, FamilyUser};
use heronite_i18n::{tr, trn};
use heronite_metadata::{BulkArgs, BulkItem, GameCounts};

use super::context_menu::{item, Flyout};
use super::metadata_ui::bulk_item;
use super::selection::DockPop;
use super::CollectionView;
use crate::dialogs::covers::words;
use crate::dialogs::metadata_update::hub::{hub, HubGlobal};
use crate::icons::Icon;
use crate::theme;
use crate::ui::controls::kbd;
use crate::ui::icon;

/// The counts of the selection, kept while it and the data do not change.
#[derive(Default)]
pub(super) struct MetaMenu {
    keys: Vec<SharedString>,
    counts: Option<GameCounts>,
    /// Who draws each cover family, read with the counts.
    families: Option<CoverFamilies>,
    task: Option<Task<()>>,
    /// The data changed since `counts` were read.
    stale: bool,
}

impl MetaMenu {
    /// Whether the counts of the selection `keys` must be read (again).
    fn needs_count(&self, keys: &[SharedString]) -> bool {
        self.stale || self.keys != keys
    }

    /// What was counted no longer holds.
    fn forget(&mut self) {
        self.stale = true;
        self.counts = None;
        self.families = None;
        self.task = None;
    }
}

/// The rows of the menu, in order.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Row {
    FillMissing,
    /// "Download <family> covers": the selection shows some cropped.
    DownloadCovers(CoverFamily),
    FixMatches,
    UpdateMetadata,
}

impl Row {
    fn icon(self) -> Icon {
        match self {
            Self::FillMissing => Icon::AutoAwesome,
            Self::DownloadCovers(_) => Icon::Collections,
            Self::FixMatches => Icon::TravelExplore,
            Self::UpdateMetadata => Icon::Tune,
        }
    }

    fn title(self) -> &'static str {
        match self {
            Self::FillMissing => tr!("Fill what is missing"),
            Self::DownloadCovers(f) => words::download(f),
            Self::FixMatches => tr!("Fix matches\u{2026}"),
            Self::UpdateMetadata => tr!("Update metadata\u{2026}"),
        }
    }

    fn id(self) -> String {
        match self {
            Self::DownloadCovers(f) => format!("covers-{}", f.id()),
            other => format!("{other:?}"),
        }
    }
}

/// "12 show a crop in Console mode": where the cropped covers of `family` show.
fn crop_hint(n: usize, families: Option<&CoverFamilies>, family: CoverFamily) -> String {
    let state = families.map(|f| f.state(family));
    let desktop = state.is_some_and(|s| s.used_by.contains(&FamilyUser::DesktopCovers));
    let console = state.is_some_and(|s| s.used_by.iter().any(|u| matches!(u, FamilyUser::Console { .. })));
    match (desktop, console) {
        (true, true) => {
            trn!(n, "{n} shows a crop on the desktop and in Console mode", "{n} show a crop on the desktop and in Console mode")
        }
        (false, true) => trn!(n, "{n} shows a crop in Console mode", "{n} show a crop in Console mode"),
        _ => trn!(n, "{n} shows a crop on the desktop", "{n} show a crop on the desktop"),
    }
}

impl CollectionView {
    fn selection_items(&self) -> Vec<BulkItem> {
        self.selected_games().iter().map(|g| bulk_item(g)).collect()
    }

    /// Counts the selection's missing fields unless they are counted (or
    /// being counted) already.
    pub(super) fn ensure_meta_counts(&mut self, cx: &mut Context<Self>) {
        if !self.meta_menu.needs_count(&self.sel.keys) || !cx.has_global::<HubGlobal>() {
            return;
        }
        self.meta_menu.stale = false;
        self.meta_menu.keys = self.sel.keys.clone();
        self.meta_menu.counts = None;
        let service = hub(cx).read(cx).service.clone();
        let games = self.selection_items();
        let keys = self.sel.keys.clone();
        self.meta_menu.task = Some(cx.spawn(async move |this, cx| {
            let (counts, families) = cx.background_spawn(async move { (service.bulk_counts(&games), service.cover_families()) }).await;
            this.update(cx, |this, cx| {
                match counts {
                    Ok(c) if this.meta_menu.keys == keys => {
                        this.meta_menu.counts = Some(c);
                        this.meta_menu.families = Some(families);
                    }
                    Ok(_) => {}
                    Err(err) => log::warn!("metadata counts: {err:#}"),
                }
                cx.notify();
            })
            .ok();
        }));
    }

    /// The library is read again: the counts are too, now when the menu shows.
    pub(super) fn forget_meta_counts(&mut self, cx: &mut Context<Self>) {
        self.meta_menu.forget();
        let shown = self.sel.pop == Some(DockPop::Metadata) || self.context_menu.as_ref().is_some_and(|m| m.open == Some(Flyout::Metadata));
        if shown {
            self.ensure_meta_counts(cx);
        }
    }

    /// The dock's Metadata button: opens or closes its menu.
    pub(super) fn toggle_dock_metadata(&mut self, cx: &mut Context<Self>) {
        if self.sel.pop == Some(DockPop::Metadata) {
            self.sel.pop = None;
        } else {
            self.sel.pop = Some(DockPop::Metadata);
            self.ensure_meta_counts(cx);
        }
        cx.notify();
    }

    /// "Fill what is missing" on the selection. False when nothing is missing.
    pub(super) fn fill_selection(&mut self, cx: &mut Context<Self>) -> bool {
        if !self.fill_hint().1 || !cx.has_global::<HubGlobal>() {
            return false;
        }
        let games = self.selection_items();
        if games.is_empty() {
            return false;
        }
        self.sel.pop = None;
        self.context_menu = None;
        hub(cx).update(cx, |h, cx| h.start(BulkArgs::fill_missing(games), cx));
        cx.notify();
        true
    }

    /// The line under "Fill what is missing" and whether it can run.
    fn fill_hint(&self) -> (String, bool) {
        match &self.meta_menu.counts {
            None => (tr!("Counting\u{2026}").to_string(), true),
            Some(c) if c.fields_missing == 0 => (tr!("Nothing is missing").to_string(), false),
            Some(c) => {
                let fields = trn!(c.fields_missing, "{n} field", "{n} fields");
                (trn!(c.games_missing, "{fields} in {n} game", "{fields} in {n} games", fields), true)
            }
        }
    }

    /// The line under "Fix matches…": how many of the games wait for review.
    fn fix_hint(&self) -> String {
        match &self.meta_menu.counts {
            Some(c) if c.to_review > 0 => trn!(c.to_review, "{n} of these waits for review", "{n} of these wait for review"),
            _ => tr!("Check which game each one is").to_string(),
        }
    }

    /// The rows shown: a download per family the selection shows cropped
    /// (counted with the rest) after "Fill what is missing".
    fn rows(&self) -> Vec<Row> {
        let cropped = self.meta_menu.counts.iter().flat_map(|c| c.cropped.iter()).filter(|(f, n)| **n > 0 && **f != CoverFamily::Tall);
        std::iter::once(Row::FillMissing)
            .chain(cropped.map(|(f, _)| Row::DownloadCovers(*f)))
            .chain([Row::FixMatches, Row::UpdateMetadata])
            .collect()
    }

    /// "Download <family> covers" on the selection's games that lack one.
    fn download_selection_covers(&mut self, family: CoverFamily, cx: &mut Context<Self>) {
        if !cx.has_global::<HubGlobal>() {
            return;
        }
        let games = self.selection_items();
        self.sel.pop = None;
        self.context_menu = None;
        hub(cx).update(cx, |h, cx| h.download_family(family, Some(games), cx));
        cx.notify();
    }

    /// The menu's rows; `prefix` keeps the dock's and the context menu's ids apart.
    pub(super) fn metadata_rows(&self, prefix: &str, enter: bool, cx: &mut Context<Self>) -> Vec<Stateful<Div>> {
        self.rows()
            .into_iter()
            .map(|row| {
                let (hint, enabled) = match row {
                    Row::FillMissing => self.fill_hint(),
                    Row::DownloadCovers(f) => {
                        let n = self.meta_menu.counts.as_ref().and_then(|c| c.cropped.get(&f)).copied().unwrap_or(0);
                        (crop_hint(n, self.meta_menu.families.as_ref(), f), true)
                    }
                    Row::FixMatches => (self.fix_hint(), true),
                    Row::UpdateMetadata => (tr!("Pick the games, fields and sources").to_string(), true),
                };
                let text = div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .child(div().truncate().child(row.title()))
                    .child(div().truncate().text_size(px(11.5)).text_color(theme::text_dim()).child(hint));
                let el = item(SharedString::from(format!("{prefix}-meta-{}", row.id())))
                    .py(px(7.))
                    .child(icon(row.icon(), px(18.), if row == Row::FillMissing { theme::accent() } else { theme::text_muted() }))
                    .child(text)
                    .when(enter && row == Row::FillMissing && enabled, |d| d.child(kbd("Enter")));
                if !enabled {
                    return el.opacity(0.5).cursor_default();
                }
                el.on_click(cx.listener(move |this, _, window, cx| match row {
                    Row::FillMissing => {
                        this.fill_selection(cx);
                    }
                    Row::DownloadCovers(f) => this.download_selection_covers(f, cx),
                    Row::FixMatches => this.fix_selection_matches(window, cx),
                    Row::UpdateMetadata => {
                        this.open_metadata_update(true, window, cx);
                    }
                }))
            })
            .collect()
    }

    /// The dock's popover: the selection in a few words, then the rows.
    pub(super) fn dock_metadata_menu(&self, n: usize, cx: &mut Context<Self>) -> Div {
        div()
            .flex()
            .flex_col()
            .gap(px(2.))
            .w(px(300.))
            .child(div().px(px(12.)).pt(px(6.)).pb(px(4.)).text_size(px(12.)).text_color(theme::text_dim()).child(trn!(
                n,
                "{n} game",
                "{n} games"
            )))
            .children(self.metadata_rows("dock", true, cx))
    }
}

#[cfg(test)]
mod tests {
    // Not `super::*`: GPUI's `test` attribute would replace the std one.
    use super::MetaMenu;
    use gpui_kit::SharedString;
    use heronite_metadata::GameCounts;

    #[test]
    fn counts_are_read_again_once_the_data_changed() {
        let keys = vec![SharedString::from("sideload:a")];
        let mut menu = MetaMenu::default();
        assert!(menu.needs_count(&keys));
        menu.keys = keys.clone();
        menu.counts = Some(GameCounts { fields_missing: 12, ..Default::default() });
        assert!(!menu.needs_count(&keys), "the same selection is counted once");
        menu.forget();
        assert!(menu.needs_count(&keys) && menu.counts.is_none());
    }
}
