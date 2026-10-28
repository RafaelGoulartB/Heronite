//! Rendering of Settings → Library: the "Show hidden games" switch, the
//! hidden and favourite lists with their pagers, and the recent games.

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::config_games;
use heronite_i18n::{tr, trn};

use super::pager::{ListKind, Pager, COLLAPSED_ROWS};
use super::{Clear, SettingsPage, Snapshot};
use crate::icons::Icon;
use crate::pages::{caption_with, empty_row, icon_action};
use crate::theme;
use crate::ui::controls::{action_button, caption, card, row, toggle_row, ButtonKind};
use crate::ui::fields::{meta_text, select_field};
use crate::ui::icon;

impl SettingsPage {
    pub(in super::super) fn library_section(&mut self, cx: &mut Context<Self>) -> Div {
        let snap = self.library.snap.clone();
        let show = self.render_show_hidden(&snap, cx);
        self.library.hidden_pg.clamp(snap.hidden.len());
        self.library.fav_pg.clamp(snap.favourites.len());
        let hidden = self.render_hidden(&snap, cx);
        let favourites = self.render_favourites(&snap, cx);
        let recent = self.render_recent(&snap, cx);

        let mut col =
            div().flex().flex_col().child(show).children(hidden).children(favourites).child(caption(tr!("Recent games"))).child(recent);
        if let Some((text, error)) = self.library.notice.clone() {
            col = col.child(notice_line(text, error));
        }
        col
    }

    fn render_show_hidden(&self, snap: &Snapshot, cx: &mut Context<Self>) -> Div {
        card([toggle_row(
            "lib-show-hidden",
            tr!("Show hidden games"),
            Some(tr!("Hidden games appear in the Collection, greyed with a \"Hidden\" tag, and can be unhidden from their menu. The same switch is in the filter menu of the header.")),
            snap.show_hidden,
        )
        .on_click(cx.listener(|this, _, _, cx| {
            let on = !this.library.snap.show_hidden;
            this.set_show_hidden(on, cx);
        }))
        .into_any_element()])
    }

    /// Caption with "Unhide all", the card of hidden games and its pager.
    fn render_hidden(&mut self, snap: &Snapshot, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let mut rows: Vec<AnyElement> = Vec::new();
        let range = self.library.hidden_pg.range(snap.hidden.len());
        for (i, (app, title)) in snap.hidden.iter().enumerate().skip(range.start).take(range.len()) {
            let app_name = app.clone();
            rows.push(self.entry_row(format!("lib-unhide-{i}"), title, app, tr!("Unhide"), cx, move |this, cx| {
                this.library_unhide(app_name.clone(), cx)
            }));
        }
        if rows.is_empty() {
            rows.push(empty_row(tr!("No hidden games. Use \"Hide\" in a game's context menu.")).into_any_element());
        }
        let asking = self.library.confirm == Some(Clear::Hidden) && !snap.hidden.is_empty();
        let head = list_caption(
            tr!("Hidden games ({count})", count = snap.hidden.len()),
            self.clear_button(
                asking,
                Clear::Hidden,
                "lib-unhide-all",
                tr!("Unhide all"),
                !self.library.busy && !snap.hidden.is_empty(),
                cx,
            ),
        );
        let question = asking.then(|| {
            let n = snap.hidden.len();
            let text = trn!(
                n,
                "Unhide {n} game? It shows normally in the Collection again.",
                "Unhide all {n} games? They show normally in the Collection again."
            );
            self.confirm_card("lib-unhide", text, tr!("Unhide all"), Clear::Hidden, cx)
        });
        let pager = self.pager_row(ListKind::Hidden, snap.hidden.len(), cx);
        [Some(head.into_any_element()), question, Some(card(rows).into_any_element()), pager].into_iter().flatten().collect()
    }

    /// Caption with "Clear favourites", the card of favourites and its pager.
    fn render_favourites(&mut self, snap: &Snapshot, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let mut rows: Vec<AnyElement> = Vec::new();
        let range = self.library.fav_pg.range(snap.favourites.len());
        for (i, (app, title)) in snap.favourites.iter().enumerate().skip(range.start).take(range.len()) {
            let app_name = app.clone();
            rows.push(self.entry_row(format!("lib-fav-{i}"), title, app, tr!("Remove"), cx, move |this, cx| {
                let app = app_name.clone();
                this.library_job(
                    Box::new(move |data_paths, _| {
                        config_games::remove_favourite(data_paths, &app)?;
                        Ok(String::new())
                    }),
                    cx,
                );
            }));
        }
        if rows.is_empty() {
            rows.push(empty_row(tr!("No favourites. Use \"Add to favourites\" in a game's context menu.")).into_any_element());
        }
        let asking = self.library.confirm == Some(Clear::Favourites) && !snap.favourites.is_empty();
        let head = list_caption(
            tr!("Favourites ({count})", count = snap.favourites.len()),
            self.clear_button(
                asking,
                Clear::Favourites,
                "lib-fav-clear",
                tr!("Clear favourites"),
                !self.library.busy && !snap.favourites.is_empty(),
                cx,
            ),
        );
        let question = asking.then(|| {
            let n = snap.favourites.len();
            let text = trn!(
                n,
                "Remove {n} game from your favourites? It stays in the Collection.",
                "Remove all {n} games from your favourites? They stay in the Collection."
            );
            self.confirm_card("lib-fav", text, tr!("Clear favourites"), Clear::Favourites, cx)
        });
        let pager = self.pager_row(ListKind::Favourites, snap.favourites.len(), cx);
        [Some(head.into_any_element()), question, Some(card(rows).into_any_element()), pager].into_iter().flatten().collect()
    }

    /// Card with the "Recent games to show" select and "Clear recent".
    fn render_recent(&self, snap: &Snapshot, cx: &mut Context<Self>) -> Div {
        let options: Vec<(SharedString, bool)> = (1..=10).map(|n| (SharedString::from(n.to_string()), n == snap.max_recent)).collect();
        let select = select_field(
            "lib-max-recent",
            None,
            snap.max_recent.to_string(),
            false,
            self.library.open_select,
            options,
            cx,
            |this: &mut Self, _, cx| {
                this.library.open_select = !this.library.open_select;
                cx.notify();
            },
            |this: &mut Self, i, _, cx| {
                this.library.open_select = false;
                this.set_max_recent(i as i64 + 1, cx);
            },
        );
        card([
            row(
                tr!("Recent games to show"),
                Some(tr!("How many recently played games the tray menu lists.")),
                div().w(px(120.)).child(select),
            )
            .into_any_element(),
            self.recent_clear_row(snap, cx),
        ])
    }

    /// "Clear recent games" with its count, or the question that replaces it.
    fn recent_clear_row(&self, snap: &Snapshot, cx: &mut Context<Self>) -> AnyElement {
        let n = snap.recent.len();
        if self.library.confirm == Some(Clear::Recent) && n > 0 {
            let text = trn!(
                n,
                "Clear {n} game from the recent list? It stays in the Collection.",
                "Clear {n} games from the recent list? They stay in the Collection."
            );
            return confirm_row("lib-recent", text, tr!("Clear recent"), Clear::Recent, self.library.busy, cx);
        }
        row(
            tr!("Clear recent games"),
            Some(&trn!(n, "{n} game in the recent list", "{n} games in the recent list")),
            self.clear_button(false, Clear::Recent, "lib-recent-clear", tr!("Clear recent"), !self.library.busy && n > 0, cx),
        )
        .into_any_element()
    }

    /// The button that opens the question for `which`. Nothing while that
    /// question is open: it sits in the card under the caption.
    fn clear_button(
        &self,
        asking: bool,
        which: Clear,
        id: &'static str,
        label: &'static str,
        enabled: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if asking {
            return div().into_any_element();
        }
        action_button(
            id,
            ButtonKind::Glass,
            label,
            enabled,
            cx.listener(move |this, _, _, cx| {
                this.library.confirm = Some(which);
                cx.notify();
            }),
        )
        .into_any_element()
    }

    /// A card with the question for `which` under a list's caption.
    fn confirm_card(&self, id: &'static str, text: String, yes: &'static str, which: Clear, cx: &mut Context<Self>) -> AnyElement {
        card([confirm_row(id, text, yes, which, self.library.busy, cx)]).mb(px(8.)).into_any_element()
    }

    /// One game of a list card: title, app name below, pill action on the right.
    fn entry_row(
        &self,
        id: String,
        title: &str,
        app: &str,
        label: &'static str,
        cx: &mut Context<Self>,
        on_click: impl Fn(&mut Self, &mut Context<Self>) + 'static,
    ) -> AnyElement {
        let shown = if title.is_empty() { app.to_string() } else { title.to_string() };
        let desc = (!title.is_empty()).then_some(app);
        row(
            shown,
            desc,
            action_button(
                SharedString::from(id),
                ButtonKind::Glass,
                label,
                !self.library.busy,
                cx.listener(move |this, _, _, cx| on_click(this, cx)),
            ),
        )
        .into_any_element()
    }

    fn pager_mut(&mut self, kind: ListKind) -> &mut Pager {
        match kind {
            ListKind::Hidden => &mut self.library.hidden_pg,
            ListKind::Favourites => &mut self.library.fav_pg,
        }
    }

    /// Below a list longer than 5 rows: "Expand", or once expanded
    /// "Collapse" + "11-20 of 27" with previous / next buttons.
    fn pager_row(&self, kind: ListKind, total: usize, cx: &mut Context<Self>) -> Option<AnyElement> {
        if total <= COLLAPSED_ROWS {
            return None;
        }
        let pager = match kind {
            ListKind::Hidden => self.library.hidden_pg,
            ListKind::Favourites => self.library.fav_pg,
        };
        let tag = if kind == ListKind::Hidden { "hidden" } else { "fav" };
        let toggle = |label: &'static str, expand: bool, cx: &mut Context<Self>| {
            action_button(
                SharedString::from(format!("lib-{tag}-toggle")),
                ButtonKind::Glass,
                label,
                true,
                cx.listener(move |this, _, _, cx| {
                    let pg = this.pager_mut(kind);
                    pg.expanded = expand;
                    pg.page = 0;
                    cx.notify();
                }),
            )
        };
        let row = div().flex().items_center().justify_between().gap(px(12.)).mt(px(8.));
        if !pager.expanded {
            return Some(
                row.child(toggle(tr!("Expand"), true, cx))
                    .child(meta_text(tr!("Showing {shown} of {total}", shown = COLLAPSED_ROWS, total)))
                    .into_any_element(),
            );
        }
        let range = pager.range(total);
        let pages = Pager::pages(total);
        let page = pager.page.min(pages - 1);
        let step = |id: &str, ic: Icon, enabled: bool, delta: isize, cx: &mut Context<Self>| {
            let b = icon_action(SharedString::from(format!("lib-{tag}-{id}")), ic, enabled);
            if enabled {
                b.on_click(cx.listener(move |this, _, _, cx| {
                    let pg = this.pager_mut(kind);
                    pg.page = (page as isize + delta).max(0) as usize;
                    cx.notify();
                }))
                .into_any_element()
            } else {
                b.into_any_element()
            }
        };
        let nav = div()
            .flex()
            .items_center()
            .gap(px(10.))
            .child(step("prev", Icon::ChevronLeft, page > 0, -1, cx))
            .child(meta_text(tr!(
                "{start}\u{2013}{end} of {total} \u{b7} Page {page} of {pages}",
                start = range.start + 1,
                end = range.end,
                total,
                page = page + 1,
                pages
            )))
            .child(step("next", Icon::ChevronRight, page + 1 < pages, 1, cx));
        Some(row.child(toggle(tr!("Collapse"), false, cx)).child(nav).into_any_element())
    }
}

/// The inline question before emptying a list: what will happen, Cancel and
/// the danger button that does it.
fn confirm_row(id: &'static str, text: String, yes: &'static str, which: Clear, busy: bool, cx: &mut Context<SettingsPage>) -> AnyElement {
    let buttons = div()
        .flex()
        .items_center()
        .gap(px(8.))
        .child(action_button(
            SharedString::from(format!("{id}-no")),
            ButtonKind::Ghost,
            tr!("Cancel"),
            true,
            cx.listener(|this, _, _, cx| {
                this.library.confirm = None;
                cx.notify();
            }),
        ))
        .child(action_button(
            SharedString::from(format!("{id}-yes")),
            ButtonKind::Danger,
            yes,
            !busy,
            cx.listener(move |this, _, _, cx| this.library_clear(which.key(), cx)),
        ));
    row(text, None, buttons).into_any_element()
}

/// [`caption_with`] that grows to its button: this page is one column, so
/// nothing needs it as short as a plain caption, and the full-size button
/// would otherwise overflow onto the card below.
fn list_caption(text: String, action: impl IntoElement) -> Div {
    caption_with(text, action).h_auto().mt(px(8.))
}

/// Result line under the cards (success / error).
pub(in super::super) fn notice_line(text: SharedString, error: bool) -> Div {
    let color = if error { theme::danger() } else { theme::success() };
    div()
        .flex()
        .items_center()
        .gap(px(8.))
        .mt(px(14.))
        .mx(px(6.))
        .text_size(px(13.))
        .text_color(color)
        .child(icon(if error { Icon::Warning } else { Icon::Check }, px(16.), color))
        .child(text)
}
