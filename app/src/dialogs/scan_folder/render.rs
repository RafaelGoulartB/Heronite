//! The folder field and the list of games found.

use std::path::Path;

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_i18n::tr;

use super::scan::{FoundGame, Kind, Known};
use super::ScanFolderDialog;
use crate::icons::Icon;
use crate::pages::tag_pill;
use crate::theme;
use crate::ui::controls::{button, card_box, checkbox, icon_button, mini_checkbox, ButtonKind};
use crate::ui::dropdown::{menu_item, menu_panel};
use crate::ui::fields::{choose_path, input_box, meta_text, panel, path_field};
use crate::ui::icon;

impl ScanFolderDialog {
    pub(super) fn render_folder(&self, dir_value: &str, cx: &mut Context<Self>) -> Div {
        let has = !dir_value.is_empty();
        path_field(
            "sf-folder-pick",
            Some(tr!("Games folder")),
            &self.directory,
            has,
            self.importing,
            cx.listener(move |this, _, window, cx| {
                if has {
                    this.directory.update(cx, |s, cx| s.set_value("", window, cx));
                    this.clear_results();
                    cx.notify();
                } else {
                    choose_path(window, cx, true, tr!("Games folder"), |this: &mut Self, p, window, cx| {
                        this.directory.update(cx, |s, cx| s.set_value(p, window, cx));
                        this.scan(cx);
                    });
                }
            }),
        )
    }

    /// Linux: whether the Windows games get a Wine prefix each, when some
    /// are chosen.
    pub(super) fn render_prefix_option(&self, cx: &mut Context<Self>) -> Option<Stateful<Div>> {
        let windows = self.scan.as_ref()?.games.iter().any(|g| g.import && g.program().kind == Kind::Windows);
        (windows && heronite_platform::supported(heronite_platform::Feature::Wine)).then(|| {
            checkbox("sf-own-prefix", self.own_prefixes, tr!("Give each Windows game its own Wine prefix")).on_click(cx.listener(
                |this, _, _, cx| {
                    this.own_prefixes = !this.own_prefixes;
                    cx.notify();
                },
            ))
        })
    }

    pub(super) fn render_results(&self, cx: &mut Context<Self>) -> Option<Div> {
        let scan = self.scan.as_ref()?;
        if scan.games.is_empty() {
            return Some(
                panel()
                    .child(div().text_size(px(14.)).text_color(theme::text_primary()).child(tr!("No games were found in this folder.")))
                    .child(meta_text(if heronite_platform::supported(heronite_platform::Feature::Wine) {
                        tr!("Games are found by the programs that start them: Windows programs (.exe), AppImages, start scripts and Linux programs. Try the folder of one game, or the one that holds them.")
                    } else {
                        tr!("Games are found by the programs that start them (.exe). Try the folder of one game, or the one that holds them.")
                    })),
            );
        }
        let root = Path::new(&self.scanned);
        let mut list = div().id("sf-games").flex().flex_col().gap(px(2.)).max_h(px(420.)).overflow_y_scroll();
        for (i, g) in scan.games.iter().enumerate() {
            list = list.child(self.render_game(i, g, root, cx));
        }
        let header = div()
            .flex()
            .justify_between()
            .items_center()
            .gap(px(12.))
            .child(div().text_size(px(13.)).text_color(theme::text_muted()).children(self.summary()))
            .child(
                div()
                    .flex()
                    .gap(px(8.))
                    .child(button("sf-all", ButtonKind::Glass, tr!("Select all")).on_click(cx.listener(|this, _, _, cx| {
                        this.select_all(true);
                        cx.notify();
                    })))
                    .child(button("sf-none", ButtonKind::Glass, tr!("Select none")).on_click(cx.listener(|this, _, _, cx| {
                        this.select_all(false);
                        cx.notify();
                    }))),
            );
        Some(div().flex().flex_col().gap(px(8.)).child(header).child(card_box().py(px(6.)).child(list)))
    }

    fn render_game(&self, i: usize, g: &FoundGame, root: &Path, cx: &mut Context<Self>) -> Stateful<Div> {
        let locked = g.known == Known::Path;
        let renaming = self.renaming == Some(i);
        let relative = |p: &Path| p.strip_prefix(root).unwrap_or(p).display().to_string();
        let title = if renaming {
            div().w_full().max_w(px(360.)).child(input_box(&self.rename, false).h(px(32.))).into_any_element()
        } else {
            div()
                .flex()
                .items_center()
                .gap(px(4.))
                .min_w_0()
                .child(
                    div()
                        .min_w_0()
                        .truncate()
                        .font_weight(theme::W_BOLD)
                        .text_size(px(14.))
                        .text_color(theme::text_primary())
                        .child(g.title.clone()),
                )
                .when(!locked, |d| {
                    d.child(
                        icon_button(SharedString::from(format!("sf-rename-{i}")), Icon::Edit, 14., true)
                            .size(px(24.))
                            .tooltip(crate::ui::tooltip(tr!("Rename")))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                cx.stop_propagation();
                                this.start_rename(i, window, cx);
                            })),
                    )
                })
                .into_any_element()
        };
        let note = match g.known {
            Known::Path => Some(tr!("Already in Collection")),
            Known::Title => Some(tr!("A game with this name is in your Collection")),
            Known::No => None,
        };
        let more = g.programs.len() > 1 && !locked;
        let program = div()
            .id(SharedString::from(format!("sf-program-{i}")))
            .flex()
            .items_center()
            .gap(px(4.))
            .min_w_0()
            .text_size(px(12.))
            .text_color(theme::text_dim())
            .child(div().min_w_0().truncate().child(relative(&g.program().path)))
            .when(more, |d| {
                d.cursor_pointer()
                    .hover(|s| s.text_color(theme::text_primary()))
                    .child(icon(Icon::ExpandMore, px(16.), theme::text_muted()))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        cx.stop_propagation();
                        this.programs_open = if this.programs_open == Some(i) { None } else { Some(i) };
                        cx.notify();
                    }))
            });
        let menu = (self.programs_open == Some(i) && more).then(|| self.render_programs(i, g, root, cx));
        div()
            .id(SharedString::from(format!("sf-game-{i}")))
            .relative()
            .flex()
            .gap(px(12.))
            .items_start()
            .px(px(8.))
            .py(px(8.))
            .rounded(px(theme::R_ITEM))
            .when(locked, |d| d.opacity(0.6))
            .when(!locked && !renaming, |d| {
                d.cursor_pointer().hover(|s| s.bg(theme::menu_item_hover())).on_click(cx.listener(move |this, _, _, cx| {
                    if let Some(g) = this.scan.as_mut().and_then(|s| s.games.get_mut(i)) {
                        g.import = !g.import;
                    }
                    cx.notify();
                }))
            })
            .child(div().pt(px(2.)).child(mini_checkbox(SharedString::from(format!("sf-cb-{i}")), g.import, locked)))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .flex_1()
                    .min_w_0()
                    .child(title)
                    .child(program)
                    .children(note.map(|n| div().text_size(px(12.)).text_color(theme::text_muted()).child(n))),
            )
            .child(div().pt(px(2.)).child(tag_pill(match g.program().kind {
                Kind::Windows => "Windows",
                Kind::Linux => "Linux",
            })))
            .children(menu)
    }

    /// The programs of game `i`, to pick the one that starts it.
    fn render_programs(&self, i: usize, g: &FoundGame, root: &Path, cx: &mut Context<Self>) -> impl IntoElement {
        let mut menu = menu_panel(280.).max_w(px(560.)).on_mouse_down_out(cx.listener(|this, _, _, cx| {
            this.programs_open = None;
            cx.notify();
        }));
        for (j, p) in g.programs.iter().enumerate() {
            let label = p.path.strip_prefix(&g.folder).or_else(|_| p.path.strip_prefix(root)).unwrap_or(&p.path).display().to_string();
            menu = menu.child(
                menu_item(SharedString::from(format!("sf-program-{i}-{j}")), j == g.chosen)
                    .child(div().flex_1().min_w_0().truncate().child(label))
                    .child(tag_pill(match p.kind {
                        Kind::Windows => "Windows",
                        Kind::Linux => "Linux",
                    }))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        cx.stop_propagation();
                        if let Some(g) = this.scan.as_mut().and_then(|s| s.games.get_mut(i)) {
                            g.chosen = j;
                        }
                        this.programs_open = None;
                        cx.notify();
                    })),
            );
        }
        deferred(div().absolute().top(px(52.)).left(px(36.)).occlude().child(menu)).with_priority(40)
    }
}
