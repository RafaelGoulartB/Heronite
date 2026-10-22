//! The games planned for the year's goal (Backlog tab). The slots the goal
//! still asks for on the year's line take games you mean to beat next: a
//! slot opens a picker (the queue, the games in progress, or any game by
//! name) and the pick sits there as an outlined cover until it is beaten,
//! when it becomes one of the year's beaten games. Plans live in
//! `ui-state.json` ([`heronite_data::stats::StatsPrefs::plans`]), by game key.

use gpui_kit::component::input::InputState;
use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_i18n::{tr, trn};

use super::desktop::hint;
use super::StatsPage;
use crate::icons::Icon;
use crate::theme;
use crate::ui::controls::{button_sm, close_button, ButtonKind};
use crate::ui::fields::{input_box, new_input};
use crate::ui::icon;

/// Rows of each list of the picker.
const PICK_MAX: usize = 40;

/// The picker over the page: which year's slot it fills, and the search.
pub(super) struct Planner {
    year: i32,
    slot: usize,
    query: Entity<InputState>,
}

impl StatsPage {
    /// The games planned for `year` that are not beaten yet, in order
    /// (library indexes).
    pub(super) fn plans_of(&self, year: i32) -> Vec<usize> {
        let Some(keys) = self.prefs.plans.get(&year) else { return Vec::new() };
        keys.iter().filter_map(|k| self.facts.index_of(k)).filter(|g| self.facts.beaten_on(*g).is_none()).collect()
    }

    pub(super) fn open_planner(&mut self, year: i32, slot: usize, window: &mut Window, cx: &mut Context<Self>) {
        let query = new_input(window, cx, "", tr!("Search the Collection"));
        cx.observe(&query, |_, _, cx| cx.notify()).detach();
        query.update(cx, |q, cx| q.focus(window, cx));
        self.planner = Some(Planner { year, slot, query });
        self.hover = None;
        cx.notify();
    }

    pub(super) fn close_planner(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.planner = None;
        self.focus.focus(window, cx);
        cx.notify();
    }

    /// Keys of `year`'s open plans (beaten ones dropped), for editing.
    fn open_plan_keys(&self, year: i32) -> Vec<String> {
        let lib = self.lib.clone();
        self.plans_of(year).into_iter().filter_map(|g| Some(lib.as_ref()?.games.get(g)?.key.clone())).collect()
    }

    /// Puts `game` in slot `slot` of `year` (out of any other slot).
    fn plan(&mut self, year: i32, slot: usize, game: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(key) = self.game_key(game).map(|k| k.to_string()) else { return };
        let mut keys = self.open_plan_keys(year);
        // A filled slot changes its game; an empty one adds after the others.
        let at = if slot < keys.len() {
            keys[slot] = key.clone();
            slot
        } else {
            keys.push(key.clone());
            keys.len() - 1
        };
        let keys: Vec<String> = keys.into_iter().enumerate().filter(|(i, k)| *i == at || *k != key).map(|(_, k)| k).collect();
        self.prefs.plans.insert(year, keys);
        self.save_prefs(cx);
        self.close_planner(window, cx);
    }

    fn unplan(&mut self, year: i32, slot: usize, window: &mut Window, cx: &mut Context<Self>) {
        let mut keys = self.open_plan_keys(year);
        if slot < keys.len() {
            keys.remove(slot);
        }
        self.prefs.plans.insert(year, keys);
        self.save_prefs(cx);
        self.close_planner(window, cx);
    }

    /// A slot of the goal on the year's line: the planned game's cover, or a
    /// "+" that opens the picker.
    pub(super) fn plan_slot(
        &mut self,
        year: i32,
        slot: usize,
        planned: Option<usize>,
        w: f32,
        h: f32,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let accent: Hsla = theme::accent().into();
        let tile = div()
            .id(("plan-slot", slot))
            .w(px(w))
            .h(px(h))
            .rounded(px(8.))
            .overflow_hidden()
            .cursor_pointer()
            .on_click(cx.listener(move |this, _, window, cx| this.open_planner(year, slot, window, cx)));
        match planned {
            Some(g) => {
                let src = self.cover(g, w, h, window);
                tile.relative()
                    .border_2()
                    .border_color(Hsla { a: 0.75, ..accent })
                    .bg(theme::white_a(0.06))
                    .hover(|s| s.border_color(accent))
                    .children(src.map(|s| img(s).size_full().object_fit(ObjectFit::Cover).opacity(0.8)))
                    .child(
                        div()
                            .absolute()
                            .top(px(3.))
                            .right(px(3.))
                            .size(px(16.))
                            .rounded_full()
                            .flex()
                            .items_center()
                            .justify_center()
                            .bg(theme::accent())
                            .child(icon(Icon::Flag, px(10.), theme::on_accent())),
                    )
            }
            None => tile
                .flex()
                .items_center()
                .justify_center()
                .border_1()
                .border_color(theme::white_a(0.18))
                .bg(theme::white_a(0.03))
                .hover(|s| s.border_color(accent).bg(Hsla { a: 0.08, ..accent }))
                .child(icon(Icon::Add, px(18.), theme::text_dim())),
        }
    }

    /// The picker: the queue and the games in progress, or the search's
    /// matches; a click puts the game in the slot.
    pub(super) fn render_planner(&mut self, window: &Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        let p = self.planner.as_ref()?;
        let (year, slot, query) = (p.year, p.slot, p.query.clone());
        let text = query.read(cx).value().trim().to_lowercase();
        let lib = self.lib.clone()?;
        let planned = self.plans_of(year);
        let current = planned.get(slot).copied();
        let facts = self.facts.clone();
        let open = move |g: usize| facts.beaten_on(g).is_none();
        let mut lists: Vec<(String, Vec<usize>)> = Vec::new();
        if text.is_empty() {
            let mut queue = self.facts.with(&["plan-to-play"]);
            let hours = |g: &usize| self.backlog.hltb.get(g).map(|h| h.main_story).filter(|h| *h > 0.).unwrap_or(f64::MAX);
            queue.sort_by(|a, b| hours(a).total_cmp(&hours(b)));
            lists.push((tr!("Your queue").into(), queue));
            lists.push((tr!("In progress").into(), self.facts.with(&["playing", "on-going", "on-hold", "played"])));
        } else {
            let found = (0..lib.games.len()).filter(|g| open(*g) && lib.games[*g].title_lower.contains(&text)).collect();
            lists.push((tr!("Results").into(), found));
        }
        let mut body = div().flex().flex_col().gap(px(18.));
        let mut any = false;
        for (k, (title, games)) in lists.into_iter().enumerate() {
            let games: Vec<usize> = games.into_iter().filter(|g| open(*g)).take(PICK_MAX).collect();
            if games.is_empty() {
                continue;
            }
            any = true;
            let mut grid = div().grid().grid_cols(2).gap(px(6.));
            for g in games {
                grid = grid.child(self.pick_row(k, g, planned.contains(&g), year, slot, window, cx));
            }
            body = body.child(div().flex().flex_col().gap(px(8.)).child(hint(title.to_uppercase())).child(grid));
        }
        if !any {
            body = body.child(div().py(px(30.)).text_size(px(13.)).text_color(theme::text_muted()).child(tr!("No game found.")));
        }
        let remove = current.map(|_| {
            button_sm("plan-remove", ButtonKind::Ghost, tr!("Remove from the plan"))
                .on_click(cx.listener(move |this, _, window, cx| this.unplan(year, slot, window, cx)))
        });
        let close = close_button("plan-close").on_click(cx.listener(|this, _, window, cx| this.close_planner(window, cx)));
        let card = div()
            .id("planner-card")
            .occlude()
            .w(px(780.))
            .max_h(relative(0.86))
            .flex()
            .flex_col()
            .gap(px(14.))
            .p(px(24.))
            .rounded(px(22.))
            .bg(rgb(0x161b24))
            .border_1()
            .border_color(theme::white_a(0.1))
            .on_click(|_, _, cx| cx.stop_propagation())
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .child(
                        div()
                            .flex_1()
                            .font_family(theme::font_heading())
                            .font_weight(theme::W_BOLD)
                            .text_size(px(20.))
                            .child(tr!("Plan a game for {year}", year)),
                    )
                    .children(remove)
                    .child(close),
            )
            .child(
                div()
                    .mt(px(-6.))
                    .text_size(px(13.))
                    .text_color(theme::text_muted())
                    .child(tr!("Pick a game you want to beat. It takes this slot of the goal on the year's line.")),
            )
            .child(input_box(&query, false))
            .child(div().id("planner-list").flex_1().min_h_0().overflow_y_scroll().child(body));
        Some(
            div()
                .id("stats-planner")
                .occlude()
                .absolute()
                .inset_0()
                .flex()
                .items_center()
                .justify_center()
                .bg(theme::black_a(0.6))
                .on_click(cx.listener(|this, _, window, cx| this.close_planner(window, cx)))
                .child(card)
                .into_any_element(),
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn pick_row(
        &mut self,
        list: usize,
        g: usize,
        planned: bool,
        year: i32,
        slot: usize,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let src = self.cover(g, 30., 40., window);
        let to_beat = self.backlog.hltb.get(&g).map(|h| h.main_story).filter(|h| *h > 0.);
        let status = self.game_status(g);
        div()
            .id(SharedString::from(format!("pick-{list}-{g}")))
            .flex()
            .items_center()
            .gap(px(10.))
            .p(px(6.))
            .rounded(px(10.))
            .cursor_pointer()
            .hover(|s| s.bg(theme::white_a(0.06)))
            .on_click(cx.listener(move |this, _, window, cx| this.plan(year, slot, g, window, cx)))
            .child(
                div()
                    .flex_none()
                    .w(px(30.))
                    .h(px(40.))
                    .rounded(px(5.))
                    .overflow_hidden()
                    .bg(theme::white_a(0.06))
                    .children(src.map(|s| img(s).size_full().object_fit(ObjectFit::Cover))),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .child(div().text_size(px(13.)).font_weight(theme::W_BOLD).truncate().child(self.game_title(g)))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .text_size(px(11.5))
                            .text_color(theme::text_dim())
                            .children(status.map(|(name, c)| {
                                div().flex().items_center().gap(px(5.)).child(div().size(px(7.)).rounded_full().bg(c)).child(name)
                            }))
                            .children(to_beat.map(|h| tr!("~{h}h to beat", h = h.round()))),
                    ),
            )
            .when(planned, |d| d.child(super::super::status_pill(tr!("Planned"), theme::accent())))
    }

    /// "**2 games planned** for the rest of the year" (goal card), if any.
    pub(super) fn plans_line(&self, year: i32) -> Option<String> {
        let n = self.plans_of(year).len();
        (n > 0).then(|| trn!(n, "**{n} game planned** for the rest of the year", "**{n} games planned** for the rest of the year"))
    }
}
