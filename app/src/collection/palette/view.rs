//! The palette overlay: input, grouped results, footer.
//!
//! Two levels: the main list, and the actions of one game (→ or Ctrl Enter
//! on a game, its chevron, or "All actions"). Esc, or Backspace in the empty
//! box, goes back to the main list with its text as it was.

use std::rc::Rc;
use std::time::Duration;

use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_i18n::tr;

use super::rank::{actions_list, search, Filter, ItemRef, Results};
use super::sources::{Act, Entry};
use crate::icons::Icon;
use crate::theme;
use crate::ui::controls::kbd;
use crate::ui::glass::glass_panel;
use crate::ui::icon;

#[derive(Clone, Debug)]
pub(super) enum PaletteEvent {
    /// Run `act`; `id` goes to the history.
    Run {
        act: Act,
        id: SharedString,
    },
    /// List every action of this game (the host answers with
    /// [`PaletteView::show_game`]).
    OpenGame(String),
    Close,
}

const ROW_H: f32 = 38.;
const CAPTION_H: f32 = 30.;
const LIST_PAD: f32 = 6.;
/// Room for about nine rows (and a caption); the rest scrolls.
const LIST_MAX_H: f32 = 9. * ROW_H + CAPTION_H;
const PANEL_W: f32 = 620.;
const TOP: f32 = 90.;
fn root_placeholder() -> &'static str {
    tr!("Type a command, page or game\u{2026}")
}

/// The second level: one game's actions.
struct GameLevel {
    title: SharedString,
    entries: Rc<Vec<Entry>>,
    /// The main list as it was, restored by [`PaletteView::back`].
    root_query: String,
    root_selected: usize,
}

pub(super) struct PaletteView {
    entries: Rc<Vec<Entry>>,
    /// Ids run lately, most recent first.
    recent: Rc<Vec<String>>,
    game: Option<GameLevel>,
    input: Entity<InputState>,
    query: String,
    filter: Filter,
    results: Results,
    selected: usize,
    scroll: ScrollHandle,
    _sub: Subscription,
}

impl EventEmitter<PaletteEvent> for PaletteView {}

impl PaletteView {
    pub fn new(entries: Rc<Vec<Entry>>, recent: Rc<Vec<String>>, query: &str, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input = cx.new(|cx| {
            let mut s = InputState::new(window, cx);
            s.set_placeholder(root_placeholder(), window, cx);
            s.set_value(query.to_string(), window, cx);
            s
        });
        let sub = cx.subscribe_in(&input, window, |this, input, ev: &InputEvent, _window, cx| {
            if matches!(ev, InputEvent::Change) {
                let value = input.read(cx).value().to_string();
                if value != this.query {
                    this.query = value;
                    this.recompute(cx);
                }
            }
        });
        input.update(cx, |s, cx| s.focus(window, cx));
        let mut this = Self {
            entries,
            recent,
            game: None,
            input,
            query: query.to_string(),
            filter: Filter::All,
            results: Results::default(),
            selected: 0,
            scroll: ScrollHandle::new(),
            _sub: sub,
        };
        this.results = this.rank();
        this
    }

    fn rank(&self) -> Results {
        match &self.game {
            Some(g) => actions_list(&g.entries, &self.query, &self.recent, &g.title),
            None => search(&self.entries, &self.query, self.filter, &self.recent),
        }
    }

    fn recompute(&mut self, cx: &mut Context<Self>) {
        self.results = self.rank();
        self.selected = 0;
        self.scroll.set_offset(point(px(0.), px(0.)));
        cx.notify();
    }

    fn entry(&self, r: ItemRef) -> &Entry {
        match (r, &self.game) {
            (ItemRef::Entry(i), Some(g)) => &g.entries[i],
            (ItemRef::Entry(i), None) => &self.entries[i],
            (ItemRef::Verb(i), _) => &self.results.verbs[i],
        }
    }

    fn selected_entry(&self) -> Option<&Entry> {
        self.results.items().nth(self.selected).map(|i| self.entry(i))
    }

    /// Top and bottom of the selected row inside the list content (a group's
    /// first row includes its caption).
    fn selected_span(&self) -> Option<(f32, f32)> {
        let mut y = LIST_PAD;
        let mut n = 0;
        for g in &self.results.groups {
            let cap_top = y;
            y += CAPTION_H;
            for k in 0..g.items.len() {
                if n == self.selected {
                    return Some((if k == 0 { cap_top } else { y }, y + ROW_H));
                }
                y += ROW_H;
                n += 1;
            }
        }
        None
    }

    fn content_h(&self) -> f32 {
        let mut h = 2. * LIST_PAD;
        for g in &self.results.groups {
            h += CAPTION_H + ROW_H * g.items.len() as f32;
        }
        h
    }

    fn reveal_selected(&mut self) {
        let Some((top, bottom)) = self.selected_span() else { return };
        let view_h = self.content_h().min(LIST_MAX_H);
        let shown = f32::from(-self.scroll.offset().y);
        let new = if top < shown {
            top
        } else if bottom > shown + view_h {
            bottom - view_h
        } else {
            return;
        };
        self.scroll.set_offset(point(px(0.), px(-new)));
    }

    /// Moves the selection by `delta` rows, wrapping around.
    pub(super) fn move_selection(&mut self, delta: isize, cx: &mut Context<Self>) {
        let n = self.results.count() as isize;
        if n == 0 {
            return;
        }
        self.selected = (self.selected as isize + delta).rem_euclid(n) as usize;
        self.reveal_selected();
        cx.notify();
    }

    /// Next / previous filter (the main list only).
    pub(super) fn cycle_filter(&mut self, back: bool, cx: &mut Context<Self>) {
        if self.game.is_some() {
            return;
        }
        self.filter = if back { self.filter.prev() } else { self.filter.next() };
        self.recompute(cx);
    }

    fn set_filter(&mut self, f: Filter, cx: &mut Context<Self>) {
        if self.filter != f {
            self.filter = f;
            self.recompute(cx);
        }
    }

    /// Runs the selected item. A disabled one only tells why (it is shown).
    pub fn activate(&mut self, cx: &mut Context<Self>) {
        let Some(e) = self.selected_entry() else { return };
        if e.disabled.is_some() {
            return;
        }
        let ev = match &e.act {
            Act::GameActions(key) => PaletteEvent::OpenGame(key.clone()),
            act => PaletteEvent::Run { act: act.clone(), id: e.id.clone() },
        };
        cx.emit(ev);
    }

    /// Asks for the actions of the selected game (or of the game the
    /// selected verb is about). Returns whether there was one.
    pub fn open_selected_game(&mut self, cx: &mut Context<Self>) -> bool {
        if self.game.is_some() {
            return false;
        }
        let Some(key) = self.selected_entry().and_then(|e| e.game_key()).map(str::to_string) else { return false };
        cx.emit(PaletteEvent::OpenGame(key));
        true
    }

    /// The second level, for the game `title` (entries from `action_entries`).
    pub fn show_game(&mut self, title: SharedString, entries: Vec<Entry>, window: &mut Window, cx: &mut Context<Self>) {
        let root_query = std::mem::take(&mut self.query);
        let root_selected = if self.game.is_some() { 0 } else { self.selected };
        let placeholder = tr!("Search the actions of {title}\u{2026}", title);
        self.game = Some(GameLevel { title, entries: Rc::new(entries), root_query, root_selected });
        self.input.update(cx, |s, cx| {
            s.set_value(String::new(), window, cx);
            s.set_placeholder(placeholder, window, cx);
        });
        self.recompute(cx);
    }

    /// Back to the main list from a game's actions; `false` when on it.
    pub fn back(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some(g) = self.game.take() else { return false };
        self.query = g.root_query.clone();
        self.input.update(cx, |s, cx| {
            s.set_value(g.root_query, window, cx);
            s.set_placeholder(root_placeholder(), window, cx);
        });
        self.results = self.rank();
        self.selected = g.root_selected.min(self.results.count().saturating_sub(1));
        self.reveal_selected();
        cx.notify();
        true
    }

    pub fn in_game(&self) -> bool {
        self.game.is_some()
    }

    pub fn query_is_empty(&self) -> bool {
        self.query.is_empty()
    }

    /// Currently listed titles (tests and debugging).
    #[allow(dead_code)]
    pub fn listed(&self) -> Vec<String> {
        self.results.items().map(|i| self.entry(i).title.to_string()).collect()
    }

    fn row(&self, n: usize, item: ItemRef, cx: &mut Context<Self>) -> Stateful<Div> {
        let e = self.entry(item);
        let sel = n == self.selected;
        let off = e.disabled.is_some();
        // A game of the main list opens its actions from the chevron too.
        let chevron = (self.game.is_none() && matches!(e.act, Act::SelectGame(_))).then(|| {
            div()
                .id(SharedString::from(format!("pal-more-{n}")))
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .size(px(24.))
                .rounded(px(6.))
                .cursor_pointer()
                .hover(|s| s.bg(theme::ink_a(0.1)))
                .child(icon(Icon::ChevronRight, px(16.), if sel { theme::text_primary() } else { theme::text_dim() }))
                .on_click(cx.listener(move |this, _, _, cx| {
                    cx.stop_propagation();
                    this.selected = n;
                    this.open_selected_game(cx);
                }))
        });
        let icon_color = if off {
            theme::text_dim()
        } else if sel {
            theme::accent()
        } else {
            theme::text_muted()
        };
        div()
            .id(SharedString::from(format!("pal-row-{n}")))
            .relative()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(12.))
            .h(px(ROW_H))
            .px(px(12.))
            .rounded(px(theme::R_ITEM))
            .when(!off, |d| d.cursor_pointer())
            .when(sel, |d| d.bg(if off { theme::menu_item_hover() } else { theme::accent_tint() }))
            .when(!sel, |d| d.hover(|s| s.bg(theme::menu_item_hover())))
            .on_mouse_move(cx.listener(move |this, _, _, cx| {
                if this.selected != n {
                    this.selected = n;
                    cx.notify();
                }
            }))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.selected = n;
                this.activate(cx);
            }))
            .when(sel && !off, |d| {
                d.child(div().absolute().left(px(0.)).top(px(8.)).bottom(px(8.)).w(px(2.)).rounded_full().bg(theme::accent()))
            })
            .child(icon(e.icon, px(16.), icon_color))
            .child(
                div()
                    .flex_none()
                    .max_w(px(340.))
                    .truncate()
                    .text_size(px(14.))
                    .text_color(if off { theme::text_dim() } else { theme::text_primary() })
                    .child(e.title.clone()),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_size(px(12.))
                    .text_color(if off { theme::text_muted() } else { theme::text_dim() })
                    .child(e.disabled.clone().unwrap_or_else(|| e.subtitle.clone())),
            )
            .children(e.shortcut.map(kbd))
            .children(chevron)
    }

    fn filter_pills(&self, cx: &mut Context<Self>) -> Div {
        let mut bar = div().flex().flex_none().items_center().gap(px(2.)).p(px(2.)).rounded_full().bg(theme::seg_track());
        for f in [Filter::All, Filter::Pages, Filter::Actions, Filter::Games] {
            let on = f == self.filter;
            bar = bar.child(
                div()
                    .id(SharedString::from(format!("pal-filter-{}", f.label())))
                    .px(px(9.))
                    .py(px(2.))
                    .rounded_full()
                    .cursor_pointer()
                    .text_size(px(11.5))
                    .text_color(if on { theme::text_bright() } else { Hsla::from(theme::seg_text()) })
                    .when(on, |d| d.bg(theme::seg_active()))
                    .when(!on, |d| d.hover(|s| s.text_color(theme::text_bright())))
                    .on_click(cx.listener(move |this, _, _, cx| this.set_filter(f, cx)))
                    .child(f.label()),
            );
        }
        bar
    }

    /// The game whose actions are listed, before the input; a click goes back.
    fn crumb(&self, cx: &mut Context<Self>) -> Option<Stateful<Div>> {
        let g = self.game.as_ref()?;
        Some(
            div()
                .id("pal-crumb")
                .flex()
                .flex_none()
                .items_center()
                .gap(px(4.))
                .max_w(px(220.))
                .pl(px(10.))
                .pr(px(6.))
                .py(px(3.))
                .rounded_full()
                .cursor_pointer()
                .bg(theme::accent_tint())
                .hover(|s| s.bg(theme::menu_item_hover()))
                .text_size(px(12.5))
                .text_color(theme::text_primary())
                .child(div().min_w_0().truncate().child(g.title.clone()))
                .child(icon(Icon::ChevronRight, px(14.), theme::text_muted()))
                .on_click(cx.listener(|this, _, window, cx| {
                    this.back(window, cx);
                })),
        )
    }
}

fn hint(keys: Vec<&'static str>, label: &'static str) -> Div {
    div().flex().items_center().gap(px(6.)).child(div().flex().gap(px(3.)).children(keys.into_iter().map(kbd))).child(label)
}

impl Render for PaletteView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut list = div()
            .id("palette-list")
            .flex()
            .flex_col()
            .px(px(8.))
            .py(px(LIST_PAD))
            .max_h(px(LIST_MAX_H))
            .overflow_y_scroll()
            .track_scroll(&self.scroll);
        if self.results.count() == 0 {
            list = list.child(
                div()
                    .flex()
                    .items_center()
                    .justify_center()
                    .h(px(64.))
                    .text_size(px(13.))
                    .text_color(theme::text_dim())
                    .child(if self.query.trim().is_empty() { tr!("Nothing to show here") } else { tr!("No results") }),
            );
        }
        let mut n = 0;
        let groups: Vec<_> = self.results.groups.iter().map(|g| (g.caption.clone(), g.items.clone())).collect();
        for (caption, items) in groups {
            list = list.child(
                div()
                    .flex()
                    .flex_none()
                    .items_end()
                    .h(px(CAPTION_H))
                    .px(px(12.))
                    .pb(px(4.))
                    .text_size(px(12.))
                    .text_color(theme::text_muted())
                    .child(caption),
            );
            for item in items {
                list = list.child(self.row(n, item, cx));
                n += 1;
            }
        }

        let input_row = div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(10.))
            .h(px(54.))
            .px(px(18.))
            .border_b_1()
            .border_color(theme::ink_a(0.08))
            .child(icon(Icon::FaSearch, px(15.), theme::text_muted()))
            .children(self.crumb(cx))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_size(px(15.))
                    .text_color(theme::text_primary())
                    .child(Input::new(&self.input).appearance(false)),
            )
            .child(kbd("Esc"));

        let has_game = self.game.is_none() && self.selected_entry().is_some_and(|e| e.game_key().is_some());
        let mut footer = div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(18.))
            .px(px(16.))
            .py(px(9.))
            .border_t_1()
            .border_color(theme::ink_a(0.08))
            .text_size(px(12.))
            .text_color(theme::text_dim())
            .child(hint(vec!["\u{2191}", "\u{2193}"], tr!("move")))
            .child(hint(vec!["\u{21b5}"], if self.game.is_some() { tr!("run") } else { tr!("open") }));
        footer = if self.game.is_some() {
            footer.child(hint(vec!["Esc"], tr!("back"))).child(div().flex_1())
        } else {
            footer
                .when(has_game, |f| f.child(hint(vec!["\u{2192}"], tr!("actions"))))
                .child(hint(vec!["Tab"], tr!("filter")))
                .child(div().flex_1())
                .child(self.filter_pills(cx))
        };

        let panel = glass_panel(theme::R_POPOVER)
            .id("palette")
            .occlude()
            .flex()
            .flex_col()
            .w(px(PANEL_W))
            .max_h(relative(0.9))
            .font_family(theme::font_body())
            .text_color(theme::text_primary())
            // Clicks on the panel keep the input focused and do not close it.
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, window, cx| {
                    cx.stop_propagation();
                    this.input.update(cx, |s, cx| s.focus(window, cx));
                }),
            )
            .child(input_row)
            .child(list)
            .child(footer);

        let panel = if theme::animations() {
            panel
                .with_animation("palette-open", Animation::new(Duration::from_millis(140)).with_easing(ease_out_quint()), |el, t| {
                    el.opacity(t).mt(px(-10. * (1. - t)))
                })
                .into_any_element()
        } else {
            panel.into_any_element()
        };

        deferred(
            div()
                .id("palette-backdrop")
                .absolute()
                .inset_0()
                .occlude()
                .flex()
                .flex_col()
                .items_center()
                .pt(px(TOP))
                .bg(theme::glass_backdrop_veil())
                .on_mouse_down(MouseButton::Left, cx.listener(|_, _, _, cx| cx.emit(PaletteEvent::Close)))
                .child(panel),
        )
        .with_priority(40)
    }
}
