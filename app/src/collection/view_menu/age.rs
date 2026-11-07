//! The View menu's "Ages up to" section: hides the games rated for older
//! players (`heronite_data::age_rating`), and maybe the unrated ones.

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::age_rating::AgeFilter;
use heronite_data::collection::filter_games_where;
use heronite_i18n::tr;

use super::super::CollectionView;
use super::render::{check, count_label, section_head, seg_item, seg_track};
use super::Pick;
use crate::theme;

impl CollectionView {
    pub(super) fn vm_age(&self, cx: &mut Context<Self>) -> Div {
        let d = self.vm_density();
        let age = self.ui.age;
        let counts = self.age_counts();
        let steps = std::iter::once(None).chain(AgeFilter::STEPS.into_iter().map(Some));
        let segments = steps.enumerate().map(|(i, max)| {
            let on = age.max == max;
            let pick = Pick::Age(AgeFilter { max, ..age });
            let label = match max {
                None => tr!("Any").to_string(),
                Some(a) => a.to_string(),
            };
            seg_item(("vm-age", i), on, d)
                .child(label)
                .children(counts.get(i).filter(|_| max.is_some()).map(|n| {
                    div().text_size(px(11.)).text_color(if on { theme::text_muted() } else { theme::text_dim() }).child(n.to_string())
                }))
                .on_click(cx.listener(move |this, _, _, cx| this.apply_pick(&pick, cx)))
        });
        // Which board's ratings it goes by first.
        let board = self
            .lib
            .as_deref()
            .and_then(|l| l.age_order.first())
            .map(|b| div().text_size(px(11.5)).text_color(theme::text_dim()).child(tr!("by {board}", board = b.name())));
        let unrated = age.max.map(|_| {
            let pick = Pick::Age(AgeFilter { unrated: !age.unrated, ..age });
            div()
                .id("vm-age-unrated")
                .flex()
                .items_center()
                .gap(px(10.))
                .px(px(6.))
                .py(px(4.))
                .rounded(px(9.))
                .cursor_pointer()
                .hover(|s| s.bg(theme::menu_item_hover()))
                .text_size(px(d.text))
                .text_color(theme::text_primary())
                .child(check(age.unrated))
                .child(div().flex_1().min_w_0().child(tr!("Also show games with no rating")))
                .child(count_label(self.unrated_count()))
                .on_click(cx.listener(move |this, _, _, cx| this.apply_pick(&pick, cx)))
        });
        div()
            .flex()
            .flex_col()
            .gap(px(d.head_gap))
            .child(section_head(tr!("Ages up to")).children(board))
            .child(seg_track("vm-age", d).children(segments))
            .children(unrated)
    }

    /// Games the list would show at each step (`Any` first), with the
    /// other filters as they are.
    fn age_counts(&self) -> Vec<usize> {
        let Some(lib) = self.lib.as_deref() else { return Vec::new() };
        let base = super::super::data::view_query(&self.ui, &self.search);
        std::iter::once(None)
            .chain(AgeFilter::STEPS.into_iter().map(Some))
            .map(|max| {
                let query = heronite_data::ViewQuery { age: AgeFilter { max, ..self.ui.age }, ..base.clone() };
                filter_games_where(&lib.games, &query, &|g| self.keeps_game(g, &lib.statuses)).len()
            })
            .collect()
    }

    fn unrated_count(&self) -> usize {
        let Some(lib) = self.lib.as_deref() else { return 0 };
        lib.games.iter().filter(|g| g.age_rating.as_ref().and_then(|r| r.min_age()).is_none()).count()
    }
}
