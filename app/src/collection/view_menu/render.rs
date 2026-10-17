//! The View menu's panel: search box with tokens, then either the results
//! or Presets / Sort / Group by / Show / Only / Browse, and a footer with the
//! count, "Reset all" and "Statuses…".

use gpui_kit::component::input::Input;
use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::{InstallFilter, SortMode};
use heronite_i18n::tr;

use super::super::header::Menu;
use super::super::view_filters::Only;
use super::super::CollectionView;
use super::choices::{self, Kind, Row};
use super::Pick;
use crate::icons::Icon;
use crate::theme;
use crate::ui::dropdown::menu_panel;
use crate::ui::fields::{select_menu_field, SelectOption};
use crate::ui::icon;

/// Sizes of the panel: roomy, or `compact` in a small window (the menu then
/// takes about a quarter less height and width).
#[derive(Clone, Copy)]
struct Density {
    w: f32,
    /// Inner padding of the panel's sections.
    pad: f32,
    /// Between sections.
    gap: f32,
    /// Between a section's title and its content.
    head_gap: f32,
    /// Vertical padding of a segment.
    seg_py: f32,
    text: f32,
    /// Sort direction button.
    dir: f32,
    pill_h: f32,
    pill_px: f32,
    tile_py: f32,
    tile_icon: f32,
}

impl Density {
    const ROOMY: Density = Density {
        w: 404.,
        pad: 12.,
        gap: 16.,
        head_gap: 8.,
        seg_py: 6.,
        text: 13.,
        dir: 36.,
        pill_h: 30.,
        pill_px: 11.,
        tile_py: 9.,
        tile_icon: 19.,
    };
    const COMPACT: Density = Density {
        w: 360.,
        pad: 10.,
        gap: 11.,
        head_gap: 6.,
        seg_py: 4.,
        text: 12.5,
        dir: 30.,
        pill_h: 26.,
        pill_px: 9.,
        tile_py: 6.,
        tile_icon: 16.,
    };
}

/// The actions listed after the views in their select.
#[derive(Clone, Copy)]
enum ViewAction {
    Save,
    Edit,
}

impl CollectionView {
    fn vm_density(&self) -> Density {
        if self.view_menu.compact {
            Density::COMPACT
        } else {
            Density::ROOMY
        }
    }

    pub(in crate::collection) fn view_menu(&self, cx: &mut Context<Self>) -> AnyElement {
        let searching = !self.view_menu.query.trim().is_empty();
        let d = self.vm_density();
        let body = div()
            .id("vm-body")
            .flex()
            .flex_col()
            .gap(px(d.gap))
            .p(px(d.pad))
            .min_h_0()
            .overflow_y_scroll()
            .children(searching.then(|| self.vm_results(cx)))
            .when(!searching, |d| {
                d.child(self.vm_views(cx))
                    .child(self.vm_show(cx))
                    .child(self.vm_group(cx))
                    .child(self.vm_sort(cx))
                    .child(self.vm_only(cx))
                    .child(self.vm_browse(cx))
            });
        menu_panel(d.w)
            .w(px(d.w))
            .p(px(0.))
            .gap(px(0.))
            .max_h(px(self.view_menu.room))
            .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                if this.open_menu == Some(Menu::View) {
                    this.open_menu = None;
                    cx.notify();
                }
            }))
            .child(self.vm_query_box(cx))
            .child(body)
            .child(self.vm_footer(cx))
            .into_any_element()
    }

    fn vm_query_box(&self, cx: &mut Context<Self>) -> Stateful<Div> {
        let tokens = self.view_tokens().into_iter().enumerate().map(|(i, (kind, c))| {
            let pick = c.pick.clone();
            div()
                .id(("vm-tok", i))
                .flex()
                .flex_none()
                .items_center()
                .gap(px(5.))
                .h(px(26.))
                .pl(px(9.))
                .pr(px(5.))
                .max_w(px(260.))
                .rounded(px(8.))
                .bg(theme::accent_tint())
                .text_size(px(12.5))
                .text_color(white())
                .cursor_pointer()
                .hover(|s| s.bg(theme::accent_tint_border()))
                .children(c.color.map(dot))
                .children(kind.map(|k| div().flex_none().text_color(theme::text_muted()).child(format!("{}:", k.to_lowercase()))))
                .child(div().min_w_0().truncate().child(c.label))
                .child(icon(Icon::Close, px(12.), theme::white_a(0.7)))
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.apply_pick(&pick, cx);
                    this.focus_view_search(window, cx);
                }))
        });
        div()
            .id("vm-query")
            .m(px(self.vm_density().pad))
            .mb(px(0.))
            .flex()
            .flex_none()
            .flex_wrap()
            .items_center()
            .gap(px(6.))
            .min_h(px(if self.view_menu.compact { 38. } else { 44. }))
            .px(px(10.))
            .py(px(6.))
            .rounded(px(theme::R_TILE))
            .bg(theme::black_a(0.28))
            .border_1()
            .border_color(if self.view_menu.focused { theme::input_border_focus() } else { theme::white_a(0.08) })
            .cursor_text()
            .on_click(cx.listener(|this, _, window, cx| this.focus_view_search(window, cx)))
            .child(icon(Icon::FaSearch, px(13.), theme::text_dim()))
            .children(tokens)
            .child(
                div()
                    .flex_1()
                    .min_w(px(120.))
                    .text_size(px(14.))
                    .text_color(white())
                    .child(Input::new(&self.view_menu.input).appearance(false)),
            )
    }

    fn vm_results(&self, cx: &mut Context<Self>) -> Div {
        let found = self.view_results();
        let mut list = div().flex().flex_col();
        if found.is_empty() {
            let q = self.view_menu.query.trim().to_string();
            list = list.child(
                div()
                    .py(px(18.))
                    .text_center()
                    .text_size(px(13.))
                    .text_color(theme::text_dim())
                    .child(tr!("No filter matches “{query}”", query = q).to_string()),
            );
            list = list.child(div().flex().justify_center().child(self.vm_results_close(cx)));
        }
        let highlight = self.view_menu.highlight;
        let mut n = 0;
        for (i, f) in found.into_iter().enumerate() {
            list = list.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(7.))
                    .px(px(10.))
                    .pt(px(8.))
                    .pb(px(4.))
                    .text_size(px(11.5))
                    .text_color(theme::text_dim())
                    .child(icon(f.kind.icon(), px(13.), theme::text_dim()))
                    .child(div().flex_1().child(f.kind.label().to_uppercase()))
                    .when(i == 0, |d| d.child(self.vm_results_close(cx))),
            );
            for row in f.rows {
                let lit = n == highlight;
                list = list.child(self.vm_result_row(n, row, lit, cx));
                n += 1;
            }
        }
        if choices::parse_query(&self.view_menu.query).0 == Some(Kind::Category) {
            list = list.child(
                plain_row("vm-manage-cats", false)
                    .child(icon(Icon::Label, px(15.), theme::text_muted()))
                    .child(tr!("Manage categories…"))
                    .on_click(cx.listener(|this, _, window, cx| this.open_manage_categories(window, cx))),
            );
        }
        list
    }

    /// X of the results: empties the box and goes back to the menu.
    fn vm_results_close(&self, cx: &mut Context<Self>) -> Stateful<Div> {
        div()
            .id("vm-results-close")
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .size(px(22.))
            .my(px(-4.))
            .rounded_full()
            .cursor_pointer()
            .hover(|s| s.bg(theme::white_a(0.1)))
            .child(icon(Icon::Close, px(14.), theme::text_muted()))
            .on_click(cx.listener(|this, _, window, cx| this.set_view_query("", window, cx)))
    }

    fn vm_result_row(&self, n: usize, row: Row, lit: bool, cx: &mut Context<Self>) -> Stateful<Div> {
        let el = plain_row(("vm-row", n), lit);
        let el = match &row {
            Row::Choice(c) => el
                .child(check(c.on))
                .children(c.color.map(dot))
                .child(div().flex_1().min_w_0().truncate().when(c.on, |d| d.text_color(white())).child(c.label.clone()))
                .children(c.count.map(count_label))
                .when(c.count == Some(0) && !c.on, |d| d.opacity(0.45)),
            Row::All { kind, matches } => el.text_color(theme::accent()).child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .child(tr!("All {n} in {kind}…", n = matches, kind = kind.label().to_lowercase()).to_string()),
            ),
        };
        el.on_click(cx.listener(move |this, _, window, cx| {
            this.view_menu.highlight = n;
            this.activate_row(row.clone(), window, cx);
        }))
    }

    fn vm_views(&self, cx: &mut Context<Self>) -> Div {
        let editing = self.view_menu.editing;
        let done = editing.then(|| {
            div()
                .id("vm-views-done")
                .cursor_pointer()
                .text_size(px(12.))
                .text_color(theme::accent())
                .hover(|s| s.text_color(white()))
                .child(tr!("Done"))
                .on_click(cx.listener(|this, _, _, cx| {
                    this.view_menu.editing = false;
                    this.view_menu.renaming = None;
                    cx.notify();
                }))
        });
        let header = section_head(tr!("Presets")).children(done);
        let body = if editing { self.vm_views_edit(cx) } else { self.vm_views_select(cx) };
        div().flex().flex_col().gap(px(self.vm_density().head_gap)).child(header).child(body)
    }

    /// The presets in a select (the one shown now, else "Custom"), with "Save
    /// current view…" and "Edit views…" under them; the name box below.
    fn vm_views_select(&self, cx: &mut Context<Self>) -> Div {
        let cards = self.view_cards();
        let current = cards.iter().position(|v| self.ui.shows_view(&v.filters));
        let manage_group: SharedString = tr!("Manage").into();
        let mut options: Vec<SelectOption> = cards
            .iter()
            .enumerate()
            .map(|(i, v)| SelectOption {
                label: v.name.clone().into(),
                selected: current == Some(i),
                dot: Some(v.color),
                ..Default::default()
            })
            .collect();
        let mut actions = Vec::new();
        if current.is_none() {
            actions.push(ViewAction::Save);
            options.push(SelectOption { label: tr!("Save as preset…").into(), group: Some(manage_group.clone()), ..Default::default() });
        }
        if !cards.is_empty() {
            actions.push(ViewAction::Edit);
            options.push(SelectOption { label: tr!("Edit presets…").into(), group: Some(manage_group), ..Default::default() });
        }
        let value: SharedString = match current {
            Some(i) => cards[i].name.clone().into(),
            None => tr!("Custom").into(),
        };
        let filters: Vec<_> = cards.into_iter().map(|v| v.filters).collect();
        let select = select_menu_field(
            "vm-views",
            None,
            value,
            false,
            self.view_menu.select_open,
            options,
            None,
            cx,
            |this, _, cx| {
                this.view_menu.select_open = !this.view_menu.select_open;
                cx.notify();
            },
            move |this, i, window, cx| {
                this.view_menu.select_open = false;
                match filters.get(i) {
                    Some(f) => this.apply_view_filters(f, cx),
                    None => match actions.get(i - filters.len()) {
                        Some(ViewAction::Save) => this.start_naming_view(window, cx),
                        Some(ViewAction::Edit) => this.view_menu.editing = true,
                        None => {}
                    },
                }
                cx.notify();
            },
        );
        let naming = self.view_menu.naming.as_ref().map(|(input, _)| {
            name_row(input)
                .child(
                    crate::ui::controls::button_sm("vm-view-save", crate::ui::controls::ButtonKind::Primary, tr!("Save"))
                        .on_click(cx.listener(|this, _, window, cx| this.save_named_view(window, cx))),
                )
                .child(icon_button("vm-view-cancel", Icon::Close, true).on_click(cx.listener(|this, _, _, cx| this.cancel_view_naming(cx))))
        });
        div().flex().flex_col().gap(px(8.)).child(select).children(naming)
    }

    /// One row per view: rename, store the current state, move, delete.
    fn vm_views_edit(&self, cx: &mut Context<Self>) -> Div {
        let len = self.ui.saved_views.len();
        let renaming = self.view_menu.renaming.as_ref().map(|(i, input, _)| (*i, input.clone()));
        let rows = self.view_cards().into_iter().enumerate().map(|(i, v)| {
            let row = div()
                .id(("vm-view-row", i))
                .flex()
                .items_center()
                .gap(px(8.))
                .h(px(40.))
                .pl(px(10.))
                .pr(px(4.))
                .rounded(px(9.))
                .hover(|s| s.bg(theme::white_a(0.05)))
                .child(dot(v.color));
            match &renaming {
                Some((r, input)) if *r == i => row.child(name_row(input).flex_1()).child(
                    icon_button(("vm-view-ok", i), Icon::Check, true)
                        .on_click(cx.listener(|this, _, window, cx| this.commit_view_rename(window, cx))),
                ),
                _ => row
                    .child(div().flex_1().min_w_0().truncate().text_size(px(13.5)).text_color(theme::text_primary()).child(v.name))
                    .child(
                        icon_button(("vm-view-up", i), Icon::ArrowUpward, i > 0)
                            .on_click(cx.listener(move |this, _, _, cx| this.move_view(i, true, cx))),
                    )
                    .child(
                        icon_button(("vm-view-down", i), Icon::ArrowDownward, i + 1 < len)
                            .on_click(cx.listener(move |this, _, _, cx| this.move_view(i, false, cx))),
                    )
                    .child(
                        icon_button(("vm-view-update", i), Icon::Refresh, true)
                            .on_click(cx.listener(move |this, _, _, cx| this.update_view(i, cx))),
                    )
                    .child(
                        icon_button(("vm-view-rename", i), Icon::Edit, true)
                            .on_click(cx.listener(move |this, _, window, cx| this.start_renaming_view(i, window, cx))),
                    )
                    .child(
                        icon_button(("vm-view-del", i), Icon::DeleteOutline, true)
                            .on_click(cx.listener(move |this, _, _, cx| this.delete_view(i, cx))),
                    ),
            }
        });
        let missing = self.missing_default_views().len();
        let restore = (missing > 0).then(|| {
            div()
                .id("vm-views-restore")
                .flex()
                .items_center()
                .gap(px(6.))
                .px(px(4.))
                .cursor_pointer()
                .text_size(px(12.5))
                .text_color(theme::accent())
                .hover(|s| s.text_color(white()))
                .child(icon(Icon::Replay, px(14.), theme::accent()))
                .child(tr!("Restore default presets"))
                .on_click(cx.listener(|this, _, _, cx| this.restore_default_views(cx)))
        });
        div()
            .flex()
            .flex_col()
            .gap(px(8.))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .p(px(4.))
                    .rounded(px(12.))
                    .bg(theme::white_a(0.035))
                    .children(rows)
                    .when(len == 0, |d| d.child(hint_text(tr!("No presets. Save one from the current filters.")).p(px(8.)))),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(5.))
                    .px(px(4.))
                    .child(icon(Icon::Refresh, px(13.), theme::text_dim()))
                    .child(hint_text(tr!("stores the current filters, sort and grouping in the preset."))),
            )
            .children(restore)
    }

    fn vm_sort(&self, cx: &mut Context<Self>) -> Div {
        let items = [
            (SortMode::Title, tr!("Title"), Icon::Label),
            (SortMode::LastPlayed, tr!("Last played"), Icon::History),
            (SortMode::Playtime, tr!("Playtime"), Icon::AccessTime),
        ];
        let d = self.vm_density();
        let seg = seg_track("vm-sort").children(items.into_iter().map(|(mode, label, ic)| {
            let on = self.ui.sort == mode;
            seg_item(("vm-sort", mode as usize), on, d)
                .child(icon(ic, px(15.), if on { white().into() } else { theme::seg_text() }))
                .child(label)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.ui.sort = mode;
                    this.save_ui();
                    this.refresh_groups();
                    cx.notify();
                }))
        }));
        let descending = self.ui.sort_descending;
        let dir = div()
            .id("vm-sort-dir")
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .size(px(d.dir))
            .rounded_full()
            .bg(theme::seg_track())
            .cursor_pointer()
            .hover(|s| s.bg(theme::white_a(0.12)))
            .child(icon(if descending { Icon::ArrowDownward } else { Icon::ArrowUpward }, px(17.), theme::text_primary()))
            .on_click(cx.listener(|this, _, _, cx| this.toggle_sort_direction(cx)));
        section(tr!("Sort"), d).child(div().flex().items_center().gap(px(6.)).child(seg.flex_1()).child(dir))
    }

    fn vm_group(&self, cx: &mut Context<Self>) -> Div {
        let grouped = self.ui.group_by_status;
        let colors: Vec<Rgba> =
            self.lib.as_deref().map(|l| l.statuses.iter().take(5).map(|s| theme::status_color(&s.slug)).collect()).unwrap_or_default();
        let dots = div().flex().flex_none().items_center().children(colors.into_iter().enumerate().map(|(i, c)| {
            div().size(px(8.)).rounded_full().bg(c).border_1().border_color(theme::rgb_a(20, 22, 30, 0.9)).when(i > 0, |d| d.ml(px(-3.)))
        }));
        let set = |on: bool| {
            cx.listener(move |this: &mut Self, _: &ClickEvent, _: &mut Window, cx: &mut Context<Self>| {
                if this.ui.group_by_status != on {
                    this.ui.group_by_status = on;
                    this.save_ui();
                    this.refresh_groups();
                    cx.notify();
                }
            })
        };
        let d = self.vm_density();
        section(tr!("Group by"), d).child(
            seg_track("vm-group")
                .child(seg_item("vm-group-none", !grouped, d).child(tr!("None")).on_click(set(false)))
                .child(seg_item("vm-group-status", grouped, d).child(dots).child(tr!("Status")).on_click(set(true))),
        )
    }

    fn vm_show(&self, cx: &mut Context<Self>) -> Div {
        let (all, installed) = self.show_counts();
        let items = [
            (InstallFilter::All, tr!("All"), all),
            (InstallFilter::Installed, tr!("Installed"), installed),
            (InstallFilter::Uninstalled, tr!("Not installed"), all - installed),
        ];
        let d = self.vm_density();
        section(tr!("Show"), d).child(seg_track("vm-show").children(items.into_iter().map(|(f, label, n)| {
            let on = self.ui.install_filter == f;
            seg_item(("vm-show", f as usize), on, d)
                .child(label)
                .child(div().text_size(px(11.)).text_color(if on { theme::text_primary() } else { theme::text_dim() }).child(n.to_string()))
                .on_click(cx.listener(move |this, _, _, cx| this.set_install_filter(f, cx)))
        })))
    }

    fn vm_only(&self, cx: &mut Context<Self>) -> Div {
        let tiles = [
            (Pick::Only(Only::Favourites), tr!("Favourites"), Icon::Star, self.ui.only.favourites),
            (Pick::Only(Only::Updates), tr!("Update"), Icon::Update, self.ui.only.updates),
            (Pick::Only(Only::Offline), tr!("Offline"), Icon::WifiOff, self.ui.only.offline),
            (Pick::Hidden, tr!("Hidden"), Icon::VisibilityOff, self.ui.show_hidden),
        ];
        let d = self.vm_density();
        let tile_w = (d.w - d.pad * 2. - 18.) / 4.;
        section(tr!("Only"), d).child(div().flex().gap(px(6.)).children(tiles.into_iter().enumerate().map(|(i, (pick, label, ic, on))| {
            div()
                .id(("vm-only", i))
                .flex()
                .flex_col()
                .items_center()
                .gap(px(d.tile_py - 3.))
                .w(px(tile_w))
                .pt(px(d.tile_py + 1.))
                .pb(px(d.tile_py - 1.))
                .rounded(px(12.))
                .border_1()
                .border_color(if on { theme::accent_tint_border() } else { theme::white_a(0.) })
                .bg(if on { theme::accent_tint() } else { theme::white_a(0.05) })
                .cursor_pointer()
                .hover(|s| s.bg(theme::white_a(0.09)))
                .text_size(px(11.5))
                .text_color(if on { white().into() } else { theme::text_muted() })
                .child(icon(ic, px(d.tile_icon), if on { theme::accent() } else { theme::text_primary() }))
                .child(label)
                .on_click(cx.listener(move |this, _, _, cx| this.apply_pick(&pick, cx)))
        })))
    }

    fn vm_browse(&self, cx: &mut Context<Self>) -> Div {
        let kinds = Kind::ALL.into_iter().filter(|k| *k != Kind::Only && (*k != Kind::Category || !self.categories.is_empty()));
        let d = self.vm_density();
        let fields = div()
            .id("vm-fields")
            .cursor_pointer()
            .text_size(px(12.))
            .text_color(theme::accent())
            .hover(|s| s.text_color(white()))
            .child(tr!("Library fields…"))
            .on_click(cx.listener(|this, _, window, cx| {
                this.open_menu = None;
                this.open_manage_fields(None, window, cx);
            }));
        let header = section_head(tr!("Browse")).child(fields);
        div().flex().flex_col().gap(px(d.head_gap)).child(header).child(div().flex().flex_wrap().gap(px(6.)).children(
            kinds.enumerate().map(|(i, kind)| {
                pill(("vm-browse", i), d).child(icon(kind.icon(), px(14.), theme::text_muted())).child(kind.label()).on_click(cx.listener(
                    move |this, _, window, cx| {
                        let text = choices::scoped_query(kind, "");
                        this.set_view_query(&text, window, cx);
                    },
                ))
            }),
        ))
    }

    fn vm_footer(&self, cx: &mut Context<Self>) -> Div {
        let shown: usize = self.groups.iter().map(|g| g.cards.len()).sum();
        let total = self.lib.as_deref().map_or(0, |l| l.games.len());
        let filtered = self.ui.active_filters() > 0 || self.ui.show_hidden;
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(10.))
            .px(px(16.))
            .py(px(if self.view_menu.compact { 7. } else { 10. }))
            .border_t_1()
            .border_color(theme::white_a(0.08))
            .text_size(px(12.5))
            .text_color(theme::text_muted())
            .child(div().flex_1().child(tr!("{shown} of {total} games", shown, total).to_string()))
            .when(filtered, |d| {
                d.child(
                    div()
                        .id("vm-reset")
                        .cursor_pointer()
                        .text_color(theme::accent())
                        .hover(|s| s.text_color(white()))
                        .child(tr!("Reset all"))
                        .on_click(cx.listener(|this, _, _, cx| this.reset_view_filters(cx))),
                )
            })
            .child(crate::ui::controls::button_sm("vm-statuses", crate::ui::controls::ButtonKind::Glass, tr!("Statuses…")).on_click(
                cx.listener(|this, _, window, cx| {
                    this.open_menu = None;
                    this.open_manage_statuses(window, cx);
                }),
            ))
    }
}

/// Section of the panel: small uppercase label above its content.
fn section(label: &'static str, d: Density) -> Div {
    div().flex().flex_col().gap(px(d.head_gap)).child(section_head(label))
}

/// A section's label row (room for an action on the right).
fn section_head(label: &'static str) -> Div {
    div().flex().items_center().justify_between().px(px(4.)).text_size(px(11.5)).text_color(theme::text_dim()).child(label.to_uppercase())
}

fn hint_text(text: &'static str) -> Div {
    div().text_size(px(12.)).text_color(theme::text_dim()).child(text)
}

/// Rounded name box of a view, followed by the caller's buttons.
fn name_row(input: &Entity<gpui_kit::component::input::InputState>) -> Div {
    div().flex().items_center().gap(px(6.)).child(
        div()
            .flex()
            .flex_1()
            .items_center()
            .h(theme::TOOL_BTN)
            .px(px(12.))
            .rounded_full()
            .bg(theme::search_fill_hover())
            .border_1()
            .border_color(theme::input_border_focus())
            .text_size(px(13.5))
            .child(div().flex_1().child(Input::new(input).appearance(false))),
    )
}

/// 28px round icon button; `enabled: false` greys it out and ignores clicks.
fn icon_button(id: impl Into<ElementId>, ic: Icon, enabled: bool) -> Stateful<Div> {
    let b = div().id(id).flex().flex_none().items_center().justify_center().size(px(28.)).rounded_full();
    if enabled {
        b.cursor_pointer().hover(|s| s.bg(theme::white_a(0.1))).child(icon(ic, px(16.), theme::text_muted()))
    } else {
        b.opacity(0.3).child(icon(ic, px(16.), theme::text_muted()))
    }
}

/// Faint 30px pill of the Views and Browse rows (as the header's chips).
fn pill(id: impl Into<ElementId>, d: Density) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .flex_none()
        .items_center()
        .gap(px(d.pill_px - 5.))
        .h(px(d.pill_h))
        .px(px(d.pill_px))
        .max_w(px(d.w - d.pad * 2.))
        .rounded_full()
        .border_1()
        .border_color(theme::white_a(0.))
        .bg(theme::white_a(0.05))
        .cursor_pointer()
        .hover(|s| s.bg(theme::white_a(0.1)))
        .text_size(px(d.text - 0.5))
        .text_color(theme::text_primary())
}

/// Results row (checkbox, label, count); `lit`: picked by Enter.
fn plain_row(id: impl Into<ElementId>, lit: bool) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .items_center()
        .gap(px(10.))
        .w_full()
        .px(px(10.))
        .py(px(7.))
        .rounded(px(9.))
        .text_size(px(13.5))
        .text_color(theme::text_primary())
        .cursor_pointer()
        .when(lit, |d| d.bg(theme::white_a(0.07)))
        .hover(|s| s.bg(theme::menu_item_hover()))
}

/// Square checkbox of a results row (the row takes the click).
fn check(on: bool) -> Div {
    div()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(px(17.))
        .rounded(px(5.))
        .border_1()
        .when(on, |d| d.bg(theme::accent()).border_color(theme::accent()).child(icon(Icon::Check, px(13.), theme::on_accent())))
        .when(!on, |d| d.border_color(theme::white_a(0.28)))
}

fn count_label(n: usize) -> Div {
    div().flex_none().text_size(px(12.)).text_color(theme::text_dim()).child(n.to_string())
}

fn dot(color: Rgba) -> Div {
    div().flex_none().size(px(8.)).rounded_full().bg(color)
}

/// Segmented control: equal parts on a faint pill.
fn seg_track(id: &'static str) -> Stateful<Div> {
    div().id(id).flex().items_center().gap(px(2.)).p(px(3.)).rounded_full().bg(theme::seg_track())
}

fn seg_item(id: impl Into<ElementId>, on: bool, d: Density) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .flex_1()
        .min_w_0()
        .items_center()
        .justify_center()
        .gap(px(6.))
        .py(px(d.seg_py))
        .px(px(8.))
        .rounded_full()
        .text_size(px(d.text))
        .whitespace_nowrap()
        .cursor_pointer()
        .text_color(if on { white() } else { theme::seg_text().into() })
        .when(on, |d| d.bg(theme::seg_active()))
        .when(!on, |d| d.hover(|s| s.text_color(white())))
}
