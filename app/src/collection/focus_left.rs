//! Left column of the focus panel: the cover, Play (or the Epic progress),
//! the status selector with favourite / metadata / settings, the user's star
//! rating and a facts card (score, library, save backup, Linux). When the
//! panel is narrow the same pieces sit in a row under the title.

use std::time::SystemTime;

use gpui_kit::prelude::*;
use gpui_kit::*;

use super::epic::FocusEpic;
use super::focus::{panel, Layout};
use super::game_facts::{Backup, Linux};
use super::vm::FocusVm;
use super::CollectionView;
use crate::icons::Icon;
use crate::theme;
use crate::ui::{icon, round_button};
use heronite_i18n::tr;

/// Gold of a favourite heart and of the rating stars.
pub(super) const STAR: u32 = 0xf4d35e;
const NARROW_COVER_W: f32 = 120.;

impl CollectionView {
    pub(super) fn focus_left(
        &mut self,
        vm: &FocusVm,
        lay: &Layout,
        epic: Option<FocusEpic>,
        scale: f32,
        vh: f32,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let cover_w = if lay.narrow { NARROW_COVER_W } else { lay.left_w };
        let cover_h = cover_w * self.cover_aspect;
        let cover = self.cover(&vm.cover_key, cover_w, cover_h, scale, 200);
        let cover = div()
            .flex_none()
            .w(px(cover_w))
            .h(px(cover_h))
            .rounded(px(12.))
            .overflow_hidden()
            .bg(theme::ink_a(0.06))
            .shadow(vec![BoxShadow {
                color: theme::shadow_a(0.5),
                offset: point(px(0.), px(12.)),
                blur_radius: px(36.),
                spread_radius: px(0.),
                inset: false,
            }])
            .children(cover.map(|src| img(src).size_full().rounded(px(12.)).object_fit(ObjectFit::Cover)));

        let controls = div()
            .flex()
            .flex_col()
            .gap(px(10.))
            .min_w_0()
            .child(self.play_area(vm, epic, cx))
            .child(self.focus_status_row(vm, vh, cx))
            .children(self.left_off_card(&vm.key, cx))
            .child(self.rating_row(&vm.key, cx))
            .child(self.facts_card(vm, cx));

        if lay.narrow {
            return div().flex().items_start().gap(px(20.)).child(cover).child(controls.flex_1().max_w(px(380.))).into_any_element();
        }
        div().flex().flex_col().gap(px(16.)).w(px(lay.left_w)).child(cover).child(controls).into_any_element()
    }

    /// Play / Stop / Install, or the Epic install progress that replaces it.
    pub(super) fn play_area(&mut self, vm: &FocusVm, epic: Option<FocusEpic>, cx: &mut Context<Self>) -> AnyElement {
        // While the launch prepares, its progress takes the button's place.
        if let Some(progress) = self.launch_progress(&vm.key, cx) {
            return progress;
        }
        let (epic_el, hide_play) = match epic {
            Some(e) => (Some(e.element), e.replaces_play),
            None => (None, false),
        };
        let (label, ic) = if vm.playing {
            (tr!("Stop"), Icon::Stop)
        } else if vm.installed {
            (tr!("Play"), Icon::PlayArrow)
        } else {
            (tr!("Install"), Icon::Download)
        };
        let key = vm.key.clone();
        // The way to start Play uses, when it is not the default.
        let option = (vm.installed && !vm.playing).then(|| self.facts.launch_current.as_ref().map(|o| o.label().to_string())).flatten();
        let play = (!hide_play).then(|| {
            div()
                .id("focus-play")
                .flex()
                .items_center()
                .justify_center()
                .gap(px(8.))
                .w_full()
                .h(px(46.))
                .rounded_full()
                .bg(theme::play_fill())
                .hover(|s| s.bg(theme::play_fill_hover()))
                .text_color(theme::play_text())
                .font_family(theme::font_heading())
                .font_weight(theme::W_BOLD)
                .text_size(px(16.))
                .cursor_pointer()
                .shadow(vec![BoxShadow {
                    color: theme::shadow_a(0.28),
                    offset: point(px(0.), px(8.)),
                    blur_radius: px(24.),
                    spread_radius: px(0.),
                    inset: false,
                }])
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.install_or_play(&key, window, cx);
                    cx.notify();
                }))
                .child(icon(ic, px(20.), theme::play_text()))
                .child(label)
                .children(option.map(|o| {
                    div().min_w_0().truncate().font_weight(theme::W_REGULAR).text_size(px(13.)).text_color(theme::play_note()).child(o)
                }))
        });
        // Installed, with other ways to start or actions: an arrow with them.
        let more = if vm.installed && !vm.playing { self.launch_more_button(&vm.key, cx) } else { None };
        let play = match (play, more) {
            (Some(play), Some(more)) => {
                let row = div()
                    .relative()
                    .flex()
                    .gap(px(8.))
                    .w_full()
                    .child(play.flex_1().w_auto())
                    .child(more)
                    .children(self.launch_menu(&vm.key, cx))
                    .into_any_element();
                return div().flex().flex_col().gap(px(8.)).child(row).children(epic_el).into_any_element();
            }
            (play, _) => play,
        };
        // Not installed: an arrow with the other ways to install next to it.
        let game = self.lib.as_ref().and_then(|l| l.get(&vm.key)).cloned().filter(|_| !vm.playing && !vm.installed);
        let play = match (play, game) {
            (Some(play), Some(game)) => match self.install_more_button(&game, cx) {
                Some(more) => Some(
                    div()
                        .relative()
                        .flex()
                        .gap(px(8.))
                        .w_full()
                        .child(play.flex_1().w_auto())
                        .child(more)
                        .children(self.install_menu(&game, cx))
                        .into_any_element(),
                ),
                None => Some(play.into_any_element()),
            },
            (play, _) => play.map(IntoElement::into_any_element),
        };
        div().flex().flex_col().gap(px(8.)).children(play).children(epic_el).into_any_element()
    }

    /// Status selector, favourite, metadata and game settings.
    pub(super) fn focus_status_row(&mut self, vm: &FocusVm, vh: f32, cx: &mut Context<Self>) -> Div {
        let open = self.facts.status_open;
        let (name, color) = match &vm.status {
            Some(s) => (s.name.clone(), s.color),
            None => (tr!("No status").into(), theme::status_color("not-played")),
        };
        let pill = div()
            .id("focus-status")
            .flex()
            .items_center()
            .gap(px(8.))
            .h(px(36.))
            .pl(px(14.))
            .pr(px(10.))
            .rounded_full()
            .cursor_pointer()
            .bg(theme::round_fill(if open { 0.9 } else { 0.72 }))
            .hover(|s| s.bg(theme::round_fill_hover()))
            .text_size(px(13.5))
            .child(div().size(px(8.)).flex_none().rounded_full().bg(color))
            .child(div().flex_1().min_w_0().truncate().child(name))
            .child(icon(Icon::ExpandMore, px(18.), theme::ink_a(0.6)))
            .on_click(cx.listener(|this, _, _, cx| {
                this.facts.status_open = true;
                cx.notify();
            }));
        let mut wrap = div().relative().flex_1().min_w_0().child(pill);
        if open {
            wrap = wrap.child(self.focus_status_menu(vm, vh, cx));
        }
        div()
            .flex()
            .items_center()
            .gap(px(6.))
            .child(wrap)
            .child(
                round_button("focus-fav", if vm.favourite { Icon::Star } else { Icon::StarBorder }, 36., 0.72)
                    .on_click(cx.listener(|this, _, _, cx| this.toggle_favourite(cx))),
            )
            .child(
                round_button("focus-meta", Icon::TravelExplore, 36., 0.72)
                    .on_click(cx.listener(|this, _, window, cx| this.open_metadata(window, cx))),
            )
            .child(
                round_button("focus-settings", Icon::Settings, 36., 0.72)
                    .on_click(cx.listener(|this, _, window, cx| this.open_game_settings(window, cx))),
            )
    }

    /// The statuses under the pill, over a window-wide catcher that closes it.
    fn focus_status_menu(&self, vm: &FocusVm, vh: f32, cx: &mut Context<Self>) -> AnyElement {
        let Some(lib) = self.lib.clone() else { return div().into_any_element() };
        let current = vm.status.as_ref().map(|s| s.id.clone());
        let mut menu = crate::ui::dropdown::menu_panel(220.).max_h(px((vh - 120.).max(200.)));
        for st in lib.statuses.iter() {
            let (id, key) = (st.id.clone(), vm.key.clone());
            let active = current.as_deref() == Some(st.id.as_str());
            menu = menu.child(
                crate::ui::dropdown::menu_item(SharedString::from(format!("focus-st-{}", st.id)), active)
                    .child(div().size(px(10.)).flex_none().rounded_full().bg(theme::status_color(&st.slug)))
                    .child(heronite_data::status::status_label(&st.name).to_string())
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.facts.status_open = false;
                        this.set_status(&key, id.clone(), cx);
                    })),
            );
        }
        let close = cx.listener(|this, _, _, cx| {
            this.facts.status_open = false;
            cx.notify();
        });
        // Out of the flow: its children are placed from the pill's top-left.
        div()
            .absolute()
            .top_0()
            .left_0()
            .child(
                deferred(
                    anchored().position(point(px(0.), px(0.))).child(
                        div().id("focus-status-catcher").w(px(20000.)).h(px(20000.)).occlude().on_mouse_down(MouseButton::Left, close),
                    ),
                )
                .with_priority(20),
            )
            .child(
                div().absolute().top(px(40.)).left_0().child(
                    deferred(
                        anchored()
                            .snap_to_window_with_margin(px(8.))
                            .child(div().occlude().on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation()).child(menu)),
                    )
                    .with_priority(21),
                ),
            )
            .into_any_element()
    }

    /// The journal's "Where I left off", only once the player wrote it;
    /// clicking it shows the Journal.
    pub(super) fn left_off_card(&self, key: &SharedString, cx: &mut Context<Self>) -> Option<AnyElement> {
        if !self.journal.is_for(key) {
            return None;
        }
        let left_off = self.journal.data.as_ref()?.left_off.clone()?;
        let when = super::data::days_since(&left_off.at).map(super::data::days_ago);
        let caption = div()
            .flex()
            .items_center()
            .gap(px(6.))
            .text_size(px(11.))
            .font_weight(theme::W_BOLD)
            .text_color(theme::accent())
            .child(icon(Icon::Bookmark, px(13.), theme::accent()))
            .child(div().flex_1().min_w_0().truncate().child(tr!("Where I left off").to_uppercase()))
            .children(when.map(|w| div().flex_none().font_weight(theme::W_REGULAR).text_color(theme::ink_a(0.45)).child(w)));
        Some(
            div()
                .id("focus-left-off")
                .flex()
                .flex_col()
                .gap(px(6.))
                .px(px(14.))
                .py(px(10.))
                .rounded(px(10.))
                .bg(theme::accent_wash())
                .border_1()
                .border_color(theme::accent_wash_border())
                .cursor_pointer()
                .hover(|s| s.bg(theme::accent_tint()))
                .on_click(cx.listener(|this, _, _, cx| this.set_journal_view(true, cx)))
                .child(caption)
                .child(
                    div()
                        .text_size(px(13.))
                        .line_height(px(19.))
                        .text_color(theme::ink_a(0.92))
                        .line_clamp(4)
                        .text_ellipsis()
                        .child(left_off.text),
                )
                .into_any_element(),
        )
    }

    /// "Your rating" and five clickable stars (clicking the current rating clears it).
    pub(super) fn rating_row(&self, key: &SharedString, cx: &mut Context<Self>) -> Div {
        let rating = self.facts.rating(key);
        let shown = self.facts.rating_hover.or(rating).unwrap_or(0);
        let mut stars = div().flex().items_center().gap(px(2.));
        for n in 1..=5u8 {
            let key = key.clone();
            let on = n <= shown;
            stars = stars.child(
                div()
                    .id(SharedString::from(format!("focus-star-{n}")))
                    .p(px(1.))
                    .cursor_pointer()
                    .child(icon(
                        if on { Icon::Star } else { Icon::StarBorder },
                        px(20.),
                        if on { theme::readable(rgb(STAR)) } else { theme::ink_a(0.3) },
                    ))
                    .on_hover(cx.listener(move |this, h: &bool, _, cx| {
                        if *h {
                            this.facts.rating_hover = Some(n);
                        } else if this.facts.rating_hover == Some(n) {
                            this.facts.rating_hover = None;
                        }
                        cx.notify();
                    }))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        let next = if this.facts.rating(&key) == Some(n) { None } else { Some(n) };
                        this.set_rating(&key, next, cx);
                    })),
            );
        }
        div()
            .flex()
            .items_center()
            .justify_between()
            .pt(px(4.))
            .text_size(px(13.))
            // Sits on the art, not on a panel.
            .text_color(theme::ink_a(0.66))
            .child(tr!("Your rating"))
            .child(stars)
    }

    /// Score, library, last save backup and Linux compatibility.
    pub(super) fn facts_card(&self, vm: &FocusVm, cx: &mut Context<Self>) -> Div {
        let mut rows: Vec<AnyElement> = Vec::new();
        if let Some(score) = vm.score {
            let color = score_color(score);
            let value = div()
                .flex()
                .items_center()
                .justify_end()
                .gap(px(8.))
                .children(vm.score_source.clone().map(|s| div().text_size(px(12.)).text_color(theme::ink_a(0.45)).child(s)))
                .child(
                    div()
                        .px(px(7.))
                        .rounded(px(5.))
                        .bg(Rgba { a: 0.16, ..color })
                        .font_weight(theme::W_BOLD)
                        .text_color(theme::readable(color))
                        .child(format!("{score:.0}")),
                );
            rows.push(fact_row(tr!("Score"), value).into_any_element());
        }
        // Filters the collection by the store the game comes from.
        let key = vm.key.clone();
        rows.push(
            clickable(fact_row(tr!("Library"), div().child(vm.library.clone())), "focus-library")
                .on_click(cx.listener(move |this, _, _, cx| this.filter_by_store_of(&key, cx)))
                .into_any_element(),
        );
        match &self.facts.backup {
            Some(Backup::At(at)) => {
                let (text, color) = backup_age(*at);
                rows.push(self.backup_row(&vm.key, div().text_color(theme::readable(color)).child(text), cx));
            }
            Some(Backup::Never) => {
                rows.push(self.backup_row(&vm.key, div().text_color(theme::ink_a(0.52)).child(tr!("Never")), cx));
            }
            Some(Backup::Off) | None => {}
        }
        if let Some(linux) = self.facts.linux.as_ref().filter(|l| !l.is_empty()) {
            let mut value = linux_value(linux);
            if linux.steam_app_id.is_some() {
                value = value.child(icon(Icon::OpenInNew, px(13.), theme::ink_a(0.4)));
            }
            let row = fact_row(tr!("On Linux"), value);
            rows.push(match linux.steam_app_id.clone() {
                // ProtonDB page of the game.
                Some(id) => clickable(row, "focus-linux")
                    .on_click(cx.listener(move |this, _, _, cx| this.open_protondb(&id, cx)))
                    .into_any_element(),
                None => row.into_any_element(),
            });
        }
        let mut card = panel(true).pt(px(6.)).pb(px(6.));
        let n = rows.len();
        for (i, r) in rows.into_iter().enumerate() {
            card = card.child(div().when(i + 1 < n, |d| d.border_b_1().border_color(theme::ink_a(0.08))).child(r));
        }
        card
    }

    /// "Save backup": clicking it backs the saves up now.
    fn backup_row(&self, key: &SharedString, value: Div, cx: &mut Context<Self>) -> AnyElement {
        let key = key.clone();
        clickable(fact_row(tr!("Save backup"), value), "focus-backup")
            .on_click(cx.listener(move |this, _, _, cx| this.open_ludusavi_backup(&key, cx)))
            .into_any_element()
    }
}

fn fact_row(label: &'static str, value: Div) -> Div {
    div()
        .flex()
        .items_center()
        .justify_between()
        .gap(px(12.))
        .py(px(7.))
        .text_size(px(13.))
        .child(div().flex_none().text_color(theme::ink_a(0.52)).child(label))
        .child(value.min_w_0().truncate().text_color(theme::ink_a(0.94)))
}

/// A facts row that does something on click: pointer and a faint highlight
/// that reaches a little past the text.
fn clickable(row: Div, id: &'static str) -> Stateful<Div> {
    row.id(id).mx(px(-8.)).px(px(8.)).rounded(px(7.)).cursor_pointer().hover(|s| s.bg(theme::ink_a(0.06)))
}

/// Critic score colour: green from 75, amber from 50, red below.
pub(super) fn score_color(score: f64) -> Rgba {
    match score {
        s if s >= 75. => rgb(0x3ecf8e),
        s if s >= 50. => rgb(0xf0a202),
        _ => rgb(0xe5534b),
    }
}

/// "Today", "Yesterday", "3 days ago"...; green within a week, amber after a month.
fn backup_age(at: SystemTime) -> (String, Hsla) {
    let at: chrono::DateTime<chrono::Local> = at.into();
    let days = (chrono::Local::now().date_naive() - at.date_naive()).num_days().max(0);
    let text = super::data::days_ago(days);
    let color = match days {
        0..=7 => rgb(0x3ecf8e).into(),
        8..=30 => theme::ink_a(0.94),
        _ => rgb(0xf0a202).into(),
    };
    (text, color)
}

/// ProtonDB tier (or "Native") with its colour dot, then the anti-cheat state.
fn linux_value(l: &Linux) -> Div {
    let mut d = div().flex().items_center().justify_end().gap(px(6.));
    let tier = if l.native { Some("native") } else { l.tier.as_deref() };
    if let Some(t) = tier {
        let color = match t {
            "native" => rgb(0x3ecf8e),
            "platinum" => rgb(0xb4c7dc),
            "gold" => rgb(0xcfb53b),
            "silver" => rgb(0xa6a6a6),
            "bronze" => rgb(0xcd7f32),
            "borked" => rgb(0xe5534b),
            _ => rgb(0x9aa0a6),
        };
        let mut name = t.to_string();
        if t == "native" {
            name = tr!("Native").to_string();
        } else if let Some(first) = name.get_mut(0..1) {
            first.make_ascii_uppercase();
        }
        d = d.child(div().size(px(7.)).flex_none().rounded_full().bg(color)).child(name);
    }
    if let Some(ac) = &l.anticheat {
        let names: Vec<String> = ac.anticheats.iter().map(|a| heronite_data::anticheat::short_name(a)).collect();
        let names = if names.is_empty() { tr!("Anti-cheat").to_string() } else { names.join("/") };
        let (state, color): (String, Hsla) = match ac.status.as_str() {
            "Supported" | "Running" => (tr!("{names} ok", names), rgb(0x3ecf8e).into()),
            "Broken" | "Denied" => (tr!("{names} broken", names), rgb(0xe5534b).into()),
            "Planned" => (tr!("{names} planned", names), theme::ink_a(0.7)),
            other => (format!("{names} {}", other.to_lowercase()), theme::ink_a(0.7)),
        };
        if tier.is_some() {
            d = d.child(div().text_color(theme::ink_a(0.4)).child("·"));
        }
        d = d.child(div().text_color(theme::readable(color)).child(state));
    }
    d
}
