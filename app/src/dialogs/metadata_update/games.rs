//! Step 1, which games: a radio card per set with its count, then a strip of
//! the first covers.

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_i18n::{tr, trn};

use super::{footer_note, MetadataUpdate, Scope, Step};
use crate::dialogs::pick_rom::radio;
use crate::theme;
use crate::ui::controls::{action_button, button, ButtonKind};

/// Covers in the strip under the cards.
const STRIP: usize = 10;

impl MetadataUpdate {
    fn scope_card(
        &self,
        scope: Scope,
        title: SharedString,
        line: SharedString,
        extra: Option<Div>,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let on = self.scope == scope;
        let count = match scope {
            Scope::Attention => self.attention.as_ref().map(|a| a.len().to_string()).unwrap_or_else(|| "\u{2026}".into()),
            _ => self.games_of(scope).len().to_string(),
        };
        div()
            .id(SharedString::from(format!("mu-scope-{scope:?}")))
            .flex()
            .gap(px(12.))
            .px(px(16.))
            .py(px(12.))
            .rounded(px(theme::R_TILE))
            .border_1()
            .cursor_pointer()
            .when(on, |d| d.bg(theme::accent_tint()).border_color(theme::accent_tint_border()))
            .when(!on, |d| d.bg(theme::card_fill()).border_color(theme::card_border()).hover(|s| s.bg(theme::ink_a(0.06))))
            .child(div().pt(px(1.)).child(radio(on)))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .gap(px(2.))
                    .child(div().text_size(px(14.)).text_color(theme::text_primary()).child(title))
                    .child(div().text_size(px(12.)).line_height(px(16.)).text_color(theme::text_dim()).child(line))
                    .children(extra),
            )
            .child(div().flex_none().text_size(px(15.)).font_weight(theme::W_BOLD).text_color(theme::text_primary()).child(count))
            .on_click(cx.listener(move |this, _, _, cx| this.set_scope(scope, cx)))
    }

    /// What "needs attention" counts, as chips ("No details 14").
    fn attention_chips(&self) -> Option<Div> {
        let c = self.all_counts.as_ref()?;
        let chip = |label: String| {
            div()
                .px(px(9.))
                .py(px(3.))
                .rounded_full()
                .bg(theme::chip_fill())
                .text_size(px(11.5))
                .text_color(theme::text_muted())
                .child(label)
        };
        let chips = [
            (c.to_review, tr!("To review {n}", n = c.to_review)),
            (c.no_details, tr!("No details {n}", n = c.no_details)),
            (c.failed_last_time, tr!("Failed last time {n}", n = c.failed_last_time)),
            (c.no_description, tr!("No description {n}", n = c.no_description)),
            (c.no_cover, tr!("No cover {n}", n = c.no_cover)),
        ];
        let row = div().flex().flex_wrap().gap(px(6.)).mt(px(8.));
        (c.needs_attention > 0).then(|| row.children(chips.into_iter().filter(|(n, _)| *n > 0).map(|(_, label)| chip(label))))
    }

    /// The first covers of the chosen games and how many there are.
    fn strip(&mut self, window: &Window) -> Div {
        let scale = window.scale_factor();
        let keys: Vec<String> =
            self.chosen().iter().take(STRIP).map(|g| heronite_data::library::metadata_key(&g.runner, &g.app_name)).collect();
        let mut row = div().flex().items_center().gap(px(5.));
        for key in keys {
            let src = self.games.covers.get(&key).and_then(|chain| self.covers.get_chain(&format!("mu:{key}"), chain, 24., 32., scale));
            row = row.child(crate::services::widgets::cover_box(src, 24., 32.).rounded(px(4.)));
        }
        let n = self.chosen().len();
        div()
            .flex()
            .items_center()
            .gap(px(12.))
            .mt(px(14.))
            .min_h(px(32.))
            .child(row)
            .child(div().text_size(px(12.5)).text_color(theme::text_muted()).child(trn!(n, "{n} game", "{n} games")))
    }

    pub(super) fn render_games(&mut self, window: &Window, cx: &mut Context<Self>) -> Div {
        let mut cards = div().flex().flex_col().gap(px(8.));
        if self.games.selected.is_some() {
            cards = cards.child(self.scope_card(
                Scope::Selected,
                tr!("Selected games").into(),
                tr!("The games ticked in the Collection").into(),
                None,
                cx,
            ));
        }
        let shown_line = match &self.games.filters {
            Some(filters) => tr!("What the list shows now, with its filters: {filters}", filters = filters.as_str()),
            None => tr!("What the list shows now").to_string(),
        };
        cards = cards
            .child(self.scope_card(Scope::Shown, tr!("Games on screen").into(), shown_line.into(), None, cx))
            .child(self.scope_card(Scope::All, tr!("Whole Collection").into(), tr!("Every game in the Collection").into(), None, cx));
        let chips = self.attention_chips();
        cards = cards.child(self.scope_card(
            Scope::Attention,
            tr!("Games that need attention").into(),
            tr!("Waiting for review, or missing details, a description or a cover").into(),
            chips,
            cx,
        ));
        div().flex().flex_col().child(cards).child(self.strip(window))
    }

    pub(super) fn games_footer(&self, cx: &mut Context<Self>) -> Div {
        let n = self.chosen().len();
        div()
            .flex()
            .flex_1()
            .items_center()
            .gap(px(10.))
            .child(footer_note(trn!(n, "{n} game", "{n} games")))
            .child(button("mu-cancel", ButtonKind::Glass, tr!("Cancel")).on_click(cx.listener(|this, _, _, cx| this.hide(cx))))
            .child(action_button("mu-next", ButtonKind::Primary, tr!("Next"), n > 0, cx.listener(|this, _, _, cx| this.go(Step::What, cx))))
    }
}
