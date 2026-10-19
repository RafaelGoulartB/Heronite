//! Rendering of the log list and the pane.

use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::SystemTime;

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::maintenance::human_size;
use heronite_i18n::tr;
use heronite_launch::log_reader::LogEntry;

use super::LogViewer;
use crate::theme;
use crate::ui::controls::{action_button, ButtonKind};
use crate::ui::fields::meta_text;

fn when(t: SystemTime) -> String {
    chrono::DateTime::<chrono::Local>::from(t).format("%Y-%m-%d %H:%M").to_string()
}

/// Colour of a log line by its level.
fn line_color(line: &str) -> Hsla {
    if line.starts_with("===") {
        theme::white_a(0.45)
    } else if line.contains("[ERROR]") {
        rgb(0xe58a93).into()
    } else if line.contains("[WARNING]") {
        rgb(0xebcb8b).into()
    } else {
        rgb(0xd8dee9).into()
    }
}

impl LogViewer {
    pub(super) fn render_list(&self, height: f32, cx: &mut Context<Self>) -> Stateful<Div> {
        let mut col = div().flex().flex_col().gap(px(2.)).p(px(6.));
        let mut last_group = None;
        if self.entries.is_empty() {
            col = col.child(div().p(px(8.)).child(meta_text(if self.entries_loaded {
                tr!("No logs yet.")
            } else {
                tr!("Looking for logs...")
            })));
        }
        for (i, e) in self.entries.iter().enumerate() {
            let (g, header) = Self::group_of(e);
            if last_group != Some(g) {
                last_group = Some(g);
                col = col.child(
                    div()
                        .px(px(8.))
                        .pt(px(if i == 0 { 2. } else { 10. }))
                        .pb(px(2.))
                        .text_size(px(12.))
                        .text_color(theme::text_muted())
                        .child(header),
                );
            }
            let active = self.selected == Some(i);
            let (name, detail) = self.label(e);
            col = col.child(
                div()
                    .id(SharedString::from(format!("log-entry-{i}")))
                    .flex()
                    .flex_col()
                    .px(px(8.))
                    .py(px(5.))
                    .rounded(px(theme::R_ITEM))
                    .cursor_pointer()
                    .when(active, |d| d.bg(theme::accent_tint()))
                    .when(!active, |d| d.hover(|d| d.bg(theme::menu_item_hover())))
                    .on_click(cx.listener(move |this, _, _, cx| this.select(i, cx)))
                    .child(
                        div()
                            .text_size(px(14.))
                            .overflow_hidden()
                            .text_ellipsis()
                            .whitespace_nowrap()
                            .text_color(if active { white() } else { crate::theme::text_primary().into() })
                            .child(name),
                    )
                    .child(
                        div()
                            .text_size(px(11.5))
                            .overflow_hidden()
                            .text_ellipsis()
                            .whitespace_nowrap()
                            .text_color(theme::text_dim())
                            .child(format!("{detail} \u{b7} {}", human_size(e.size))),
                    ),
            );
        }
        div()
            .id("log-entries")
            .w(px(240.))
            .flex_none()
            .h(px(height))
            .overflow_y_scroll()
            .rounded(px(theme::R_CARD))
            .bg(theme::card_fill())
            .border_1()
            .border_color(theme::card_border())
            .child(col)
    }

    pub(super) fn render_pane(&self, cx: &mut Context<Self>) -> Div {
        let entry = self.selected.and_then(|i| self.entries.get(i)).cloned();
        let (name, _) = entry.as_ref().map(|e| self.label(e)).unwrap_or_default();
        let path = self.current_path();
        let has_previous = entry.as_ref().is_some_and(|e| e.previous.is_some());
        let showing_previous = self.previous && has_previous;

        let bar = self.render_bar(name, &path, has_previous, showing_previous, cx);
        let info = self.info_line(&path, entry.as_ref(), showing_previous);
        let viewer = self.render_log_view(cx);
        let footer = self.render_footer(cx);

        div()
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col()
            .gap(px(8.))
            .child(bar)
            .child(div().text_size(px(12.)).text_color(theme::text_dim()).overflow_hidden().text_ellipsis().whitespace_nowrap().child(info))
            .child(viewer)
            .child(footer)
    }

    fn render_bar(&self, name: String, path: &Option<PathBuf>, has_previous: bool, showing_previous: bool, cx: &mut Context<Self>) -> Div {
        let mut bar = div().flex().flex_wrap().items_center().gap(px(8.));
        bar = bar.child(
            div()
                .flex_1()
                .min_w(px(120.))
                .font_family(theme::font_heading())
                .text_size(px(17.))
                .font_weight(theme::W_BOLD)
                .text_color(white())
                .overflow_hidden()
                .text_ellipsis()
                .whitespace_nowrap()
                .child(if name.is_empty() { tr!("Logs").to_string() } else { name }),
        );
        if has_previous {
            let this = cx.entity();
            bar = bar.child(crate::ui::controls::tabs(
                "log-run",
                &[(false, tr!("Latest")), (true, tr!("Previous run"))],
                showing_previous,
                move |previous, _, cx| {
                    this.update(cx, |this, cx| this.set_previous(previous, cx));
                },
            ));
        }
        let has_path = path.is_some();
        let (p1, p2) = (path.clone(), path.clone());
        bar = bar
            .child(action_button("log-refresh", ButtonKind::Glass, tr!("Refresh"), true, cx.listener(|this, _, _, cx| this.refresh(cx))))
            .child(action_button(
                "log-copy",
                ButtonKind::Glass,
                if self.copied { tr!("Copied!") } else { tr!("Copy") },
                has_path && !self.content.lines.is_empty(),
                cx.listener(|this, _, _, cx| this.copy(cx)),
            ))
            .child(action_button(
                "log-folder",
                ButtonKind::Glass,
                tr!("Open folder"),
                has_path,
                cx.listener(move |this, _, _, cx| {
                    if let Some(dir) = p1.as_deref().and_then(Path::parent) {
                        this.open(dir.to_path_buf(), cx);
                    }
                }),
            ))
            .child(action_button(
                "log-open",
                ButtonKind::Glass,
                tr!("Open file"),
                has_path,
                cx.listener(move |this, _, _, cx| {
                    if let Some(file) = p2.clone() {
                        this.open(file, cx);
                    }
                }),
            ));
        bar
    }

    fn info_line(&self, path: &Option<PathBuf>, entry: Option<&LogEntry>, showing_previous: bool) -> String {
        let c = &self.content;
        let mut info = String::new();
        if let Some(p) = &path {
            info.push_str(&p.display().to_string());
        }
        if !c.lines.is_empty() || c.total > 0 {
            let total = human_size(c.total);
            let size = if c.skipped > 0 {
                tr!("{total}, showing the last {shown}", total, shown = human_size(c.total - c.skipped))
            } else {
                total
            };
            info.push_str(&format!("  \u{b7}  {size}"));
            if let Some(t) = entry.and_then(|e| e.modified) {
                if !showing_previous {
                    info.push_str(&format!("  \u{b7}  {}", when(t)));
                }
            }
        }
        info
    }

    fn render_log_view(&self, cx: &mut Context<Self>) -> Div {
        let c = &self.content;
        let lines = c.lines.clone();
        let mono = self.mono.clone();
        let body = if let Some(err) = self.error.clone() {
            div().p(px(12.)).text_color(theme::danger_text()).child(err).into_any_element()
        } else if lines.is_empty() {
            let text = if self.busy {
                tr!("Reading...")
            } else if self.selected.is_none() {
                tr!("Nothing to show. Logs appear here after a game is launched.")
            } else {
                tr!("This log is empty.")
            };
            div().p(px(12.)).child(meta_text(text)).into_any_element()
        } else {
            list(self.list_state.clone(), move |ix, _, _| {
                let line = lines[ix].clone();
                let color = line_color(&line);
                div()
                    .w_full()
                    .px(px(10.))
                    .font_family(mono.clone())
                    .text_size(px(12.5))
                    .line_height(px(17.))
                    .text_color(color)
                    .min_h(px(17.))
                    .child(line)
                    .into_any_element()
            })
            .size_full()
            .into_any_element()
        };
        let scrollbar = crate::ui::scrollbar_draggable(
            "log-scrollbar",
            self.list_state.scroll_px_offset_for_scrollbar().y,
            self.list_state.max_offset_for_scrollbar().y,
            self.list_state.viewport_bounds().size.height,
            {
                let (state, weak) = (self.list_state.clone(), cx.entity().downgrade());
                move |distance, _, cx| {
                    state.set_offset_from_scrollbar(point(px(0.), px(-distance)));
                    weak.update(cx, |_, cx| cx.notify()).ok();
                }
            },
            Some(Rc::new({
                let state = self.list_state.clone();
                move |started| {
                    if started {
                        state.scrollbar_drag_started();
                    } else {
                        state.scrollbar_drag_ended();
                    }
                }
            })),
        );
        div()
            .relative()
            .flex_1()
            .min_h_0()
            .rounded(px(theme::R_INPUT))
            .bg(theme::input_fill())
            .border_1()
            .border_color(theme::input_border())
            .overflow_hidden()
            .child(body)
            .children(scrollbar)
    }

    fn render_footer(&self, cx: &mut Context<Self>) -> Div {
        let c = &self.content;
        let mut footer = div().flex().items_center().gap(px(12.)).min_h(px(22.));
        if self.can_load_earlier() {
            footer = footer.child(action_button(
                "log-earlier",
                ButtonKind::Glass,
                tr!("Load earlier"),
                true,
                cx.listener(|this, _, _, cx| this.load_earlier(cx)),
            ));
        } else if c.skipped > 0 {
            footer = footer.child(meta_text(tr!("Only the end of this big log is shown here. Use Open file to see all of it.")));
        }
        if let Some(n) = self.notice.clone() {
            footer = footer.child(div().text_size(px(13.)).text_color(theme::danger_text()).child(n));
        }
        footer
    }
}

#[cfg(test)]
mod tests {
    // Not `super::*`: the gpui prelude has its own `test` attribute.
    use super::line_color;
    use crate::theme;

    #[test]
    fn lines_get_a_colour_by_level() {
        assert_ne!(line_color("(10:00:00) [ERROR]:   boom"), line_color("plain"));
        assert_ne!(line_color("(10:00:00) [WARNING]: hmm"), line_color("plain"));
        assert_eq!(line_color("(10:00:00) [INFO]:    ok"), line_color("plain"));
        assert_eq!(line_color("============= End of log ============="), theme::white_a(0.45));
    }
}
