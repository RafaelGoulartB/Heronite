//! Y on any tab: the Filters panel along the right edge, the search field
//! on top (focused when it opens), then what the Library grid shows: Show,
//! Sort, Favourites only, Status and Category. Every change shows at once
//! behind it, in the tab's grid, else the theme's first Grid tab, else (a
//! theme without one, like Minimal) a grid of the results over the tab that
//! B closes (`ConsoleUi::results`). A types the search on the on-screen
//! keyboard, a physical keyboard types into the field directly; X resets
//! everything; B or Y closes.

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::console_theme::{Layout, TabSpec};
use heronite_i18n::{n_, tr};

use super::super::CollectionView;
use super::layouts::{self, category_options, status_options, Filters, SHOW};
use crate::console::kit::{self, u};
use crate::console::osk::{Osk, OskEvent};
use crate::console::{hint, look, motion, Btn, Hint, Pad};
use crate::icons::Icon;
use crate::theme;
use crate::ui::icon;

/// The labels of the Grid's `SHOW` and `SORT` (A–Z, last played, most
/// played), short enough for a third of the panel.
const SHOW_LABELS: [&str; 3] = [n_!("All"), n_!("Installed"), n_!("Not installed")];
const SORT: [&str; 3] = [n_!("Title"), n_!("Last played"), n_!("Playtime")];

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Item {
    Search,
    Show,
    Sort,
    Favourites,
    Status,
    Category,
}

/// The panel's focus (index in [`CollectionView::filter_items`]).
#[derive(Default)]
pub(super) struct FiltersNav {
    focus: usize,
}

impl CollectionView {
    /// The Grid the filters drive: the tab's own, the theme's first Grid
    /// tab, else one past the theme's tabs (the results grid over the tab).
    pub(super) fn filters_target(&self) -> (usize, TabSpec) {
        let (tab, spec) = self.console_tab_spec();
        if spec.layout == Layout::Grid {
            return (tab, spec);
        }
        let theme = look::theme();
        match theme.tabs.iter().position(|t| t.layout == Layout::Grid) {
            Some(i) => (i, theme.tabs[i].clone()),
            // The tab's games, in a grid.
            None => (theme.tabs.len(), TabSpec { layout: Layout::Grid, ..spec }),
        }
    }

    /// Opens the panel with the search field focused.
    pub(super) fn open_console_filters(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        self.console.filters = Some(FiltersNav::default());
        cx.notify();
    }

    /// The filters changed: the target grid from its first game, in sight.
    pub(super) fn console_show_filtered(&mut self) {
        let (target, _) = self.filters_target();
        let nav = &mut self.tab_nav(target).grid;
        nav.index = 0;
        nav.toolbar = None;
        nav.restart = true;
        if target >= look::theme().tabs.len() {
            self.console.results = true;
        } else if target != self.console.tab {
            self.console.tab = target;
            self.console.tab_dir = 1.;
        }
    }

    fn filter_items(&self) -> Vec<Item> {
        let mut v = vec![Item::Search, Item::Show, Item::Sort, Item::Favourites];
        if self.lib.as_ref().is_some_and(|l| !l.statuses.is_empty()) {
            v.push(Item::Status);
        }
        if category_options(&self.categories).len() > 1 {
            v.push(Item::Category);
        }
        v
    }

    fn focused_filter(&self) -> Option<Item> {
        let items = self.filter_items();
        self.console.filters.as_ref().map(|n| items[n.focus.min(items.len() - 1)])
    }

    /// `false` closes the panel.
    pub(super) fn filters_pad(&mut self, pad: Pad, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let items = self.filter_items();
        let Some(nav) = self.console.filters.as_mut() else { return false };
        let i = nav.focus.min(items.len() - 1);
        match pad {
            Pad::B | Pad::Y | Pad::Start => return false,
            Pad::Up | Pad::Down => {
                nav.focus = layouts::step(i, items.len(), pad.dir(), false);
                return true;
            }
            Pad::A if items[i] == Item::Search => {
                self.filters_keyboard(window, cx);
                return true;
            }
            _ => {}
        }
        let status_n = self.lib.as_ref().map_or(0, |l| l.statuses.len()) + 1;
        let category_n = category_options(&self.categories).len();
        let (target, _) = self.filters_target();
        let before = (self.tab_nav(target).grid.filters.clone(), self.console.search.clone());
        if pad == Pad::X {
            self.console.search.clear();
        }
        let f = &mut self.tab_nav(target).grid.filters;
        // ◀ ▶ stop at the ends; A goes round.
        let turn = |v: &mut usize, n: usize| {
            *v = match pad {
                Pad::A => (*v + 1) % n,
                _ => layouts::step(*v, n, pad.dir(), false),
            }
        };
        match (pad, items[i]) {
            (Pad::X, _) => *f = Filters::default(),
            (Pad::Left | Pad::Right | Pad::A, Item::Show) => turn(&mut f.show, SHOW.len()),
            (Pad::Left | Pad::Right | Pad::A, Item::Sort) => turn(&mut f.sort, SORT.len()),
            (Pad::Left | Pad::Right | Pad::A, Item::Status) => turn(&mut f.status, status_n),
            (Pad::Left | Pad::Right | Pad::A, Item::Category) => turn(&mut f.category, category_n),
            (Pad::Left | Pad::Right | Pad::A, Item::Favourites) => f.favourites = !f.favourites,
            _ => return true,
        }
        if (self.tab_nav(target).grid.filters.clone(), self.console.search.clone()) != before {
            self.console_show_filtered();
        }
        true
    }

    /// The on-screen keyboard over the panel for the search.
    fn filters_keyboard(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let osk = cx.new(|_| Osk::new(tr!("Search games"), &self.console.search));
        let sub = cx.subscribe_in(&osk, window, |this, _, ev: &OskEvent, _, cx| {
            if let OskEvent::Done(text) = ev {
                let text = text.trim().to_string();
                if text != this.console.search {
                    this.console.search = text;
                    this.console_show_filtered();
                }
            }
            this.close_console_overlay(cx);
        });
        self.open_console_overlay(&osk, sub, cx);
    }

    /// A physical key while the search field has the focus types into it
    /// (Backspace on an empty field, Esc, Enter and the arrows stay presses).
    pub(super) fn filters_type(&mut self, ks: &Keystroke, cx: &mut Context<Self>) -> bool {
        let m = &ks.modifiers;
        if self.console.overlay.is_some() || self.focused_filter() != Some(Item::Search) || m.control || m.alt || m.platform {
            return false;
        }
        match ks.key.as_str() {
            "backspace" if !self.console.search.is_empty() => {
                self.console.search.pop();
            }
            "space" => self.console.search.push(' '),
            "backspace" | "enter" | "escape" | "tab" => return false,
            _ => match ks.key_char.as_deref() {
                Some(s) if !s.is_empty() && s.chars().all(|c| !c.is_control()) => self.console.search.push_str(s),
                _ => return false,
            },
        }
        self.console_show_filtered();
        cx.notify();
        true
    }

    pub(super) fn filters_hints(&self) -> Vec<Hint> {
        let mut h = match self.focused_filter() {
            Some(Item::Search) => vec![hint(Btn::A, tr!("Type"))],
            Some(Item::Favourites) => vec![hint(Btn::A, tr!("Change"))],
            _ => vec![hint(Btn::LeftRight, tr!("Change"))],
        };
        h.push(hint(Btn::X, tr!("Reset all")));
        h.push(hint(Btn::B, tr!("Done")));
        h
    }

    pub(super) fn render_console_filters(&mut self) -> Option<AnyElement> {
        let focus = self.focused_filter()?;
        let lib = self.lib.clone()?;
        let (target, spec) = self.filters_target();
        let games = self.console_games(&spec.source);
        let shown = self.grid_keys(target, &spec).len();
        let f = self.tab_nav(target).grid.filters.clone();
        let search = self.console.search.clone();
        let installed = games.iter().filter(|g| g.installed).count();
        let counts = [games.len(), installed, games.len() - installed];

        let tint = |on: bool| -> Hsla {
            if on {
                theme::console_on_focus().into()
            } else {
                theme::text_muted().into()
            }
        };
        let field =
            kit::row_base(focus == Item::Search)
                .when(focus != Item::Search, |d| d.bg(kit::ink(0.07)))
                .child(div().relative().child(icon(Icon::FaSearch, u(18.), tint(focus == Item::Search))))
                .child(
                    div().relative().flex_1().min_w_0().truncate().text_size(u(17.)).when(search.is_empty(), |d| d.opacity(0.6)).child(
                        if search.is_empty() { SharedString::from(tr!("Search games")) } else { format!("{search}\u{2502}").into() },
                    ),
                );
        let show: Vec<(SharedString, Option<usize>)> =
            SHOW_LABELS.iter().zip(counts).map(|(l, n)| (SharedString::from(heronite_i18n::tr(l)), Some(n))).collect();
        let sort: Vec<(SharedString, Option<usize>)> = SORT.iter().map(|l| (SharedString::from(heronite_i18n::tr(l)), None)).collect();
        let statuses = status_options(&lib);
        let categories = category_options(&self.categories);
        let category = match categories.get(f.category).cloned().flatten() {
            None => tr!("Any category").to_string(),
            Some(name) => crate::collection::view_filters::category_label(&name),
        };
        let items = self.filter_items();
        let row = |item: Item, label: String, ic: Icon, value: AnyElement| {
            let on = focus == item;
            kit::row_base(on)
                .child(div().relative().child(icon(ic, u(20.), tint(on))))
                .child(div().relative().flex_1().min_w_0().truncate().text_size(u(17.)).child(label))
                .child(div().relative().child(value))
        };
        let mut body = div()
            .flex()
            .flex_col()
            .gap(u(6.))
            .child(field)
            .child(kit::caption(tr!("Show")))
            .child(segments(&show, f.show, focus == Item::Show))
            .child(kit::caption(tr!("Sort")))
            .child(segments(&sort, f.sort, focus == Item::Sort))
            .child(kit::caption(tr!("Only")))
            .child(row(
                Item::Favourites,
                tr!("Favourites").to_string(),
                Icon::Star,
                kit::toggle_pill(f.favourites, focus == Item::Favourites).into_any_element(),
            ));
        if items.contains(&Item::Status) {
            let value = statuses.get(f.status).cloned().unwrap_or_default();
            let on = focus == Item::Status;
            body = body.child(row(
                Item::Status,
                tr!("Status").to_string(),
                Icon::Flag,
                kit::choice_sized(value, on, f.status > 0, f.status + 1 < statuses.len(), 120.).into_any_element(),
            ));
        }
        if items.contains(&Item::Category) {
            let on = focus == Item::Category;
            body = body.child(row(
                Item::Category,
                tr!("Category").to_string(),
                Icon::Folder,
                kit::choice_sized(category, on, f.category > 0, f.category + 1 < categories.len(), 120.).into_any_element(),
            ));
        }
        let footer = div()
            .flex()
            .items_center()
            .px(u(8.))
            .pt(u(12.))
            .border_t_1()
            .border_color(kit::ink(0.08))
            .text_size(u(15.))
            .text_color(theme::text_muted())
            .child(tr!("{shown} of {total} games", shown, total = games.len()));
        let panel = div()
            .absolute()
            .top_0()
            .bottom_0()
            .right_0()
            .w(u(470.))
            .flex()
            .flex_col()
            .gap(u(14.))
            .px(u(18.))
            .pt(u(26.))
            .pb(u(70.))
            .bg(theme::console_sidebar())
            .border_l_1()
            .border_color(kit::ink(0.08))
            .shadow(kit::shadow(0.5, 0., 40.))
            .child(div().px(u(8.)).child(kit::title(tr!("Filters"), 30.)))
            .child(body)
            .child(div().flex_1())
            .child(footer);
        Some(motion::enter("console-filters", panel, 280, 40., 0.))
    }
}

/// One choice among a few side by side (Show, Sort), with an optional
/// count each; the chosen one is lit, with the focus pill while its row has
/// the focus.
fn segments(options: &[(SharedString, Option<usize>)], current: usize, focused: bool) -> Div {
    div().flex().gap(u(4.)).p(u(4.)).rounded(u(16.)).bg(kit::ink(0.06)).children(options.iter().enumerate().map(|(i, (label, count))| {
        let on = i == current;
        let lit = on && focused;
        let color: Hsla = if lit {
            theme::console_on_focus().into()
        } else if on {
            kit::strong()
        } else {
            theme::text_muted().into()
        };
        div()
            .relative()
            .flex_1()
            .min_w_0()
            .flex()
            .items_center()
            .justify_center()
            .gap(u(6.))
            .h(u(46.))
            .px(u(8.))
            .rounded(u(12.))
            .when(lit, |d| d.child(kit::focus_pill(12.)))
            .when(on && !focused, |d| d.bg(kit::ink(0.14)))
            .child(
                div()
                    .relative()
                    .min_w_0()
                    .truncate()
                    .text_size(u(15.))
                    .font_weight(if on { theme::W_BOLD } else { theme::W_REGULAR })
                    .text_color(color)
                    .child(label.clone()),
            )
            .children(count.map(|n| {
                div().relative().flex_none().text_size(u(13.)).text_color(Hsla { a: color.a * 0.65, ..color }).child(n.to_string())
            }))
    }))
}
