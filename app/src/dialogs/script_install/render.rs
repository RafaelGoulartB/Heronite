//! The file picker face, the review of a script, and the dialog chrome.

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_i18n::{tr, trn};
use heronite_installer::Issue;

use super::{text_of, BackToForm, ScriptInstallDialog, Select};
use crate::dialogs::settings_panel::logic::{display_wine_name, wine_options};
use crate::dialogs::DialogEvent;
use crate::icons::Icon;
use crate::theme;
use crate::ui::controls::{action_button, button_sm, caption, card, card_box, toggle_row, ButtonKind};
use crate::ui::dialog::{dialog_scrolled, dialog_scrolled_with_back, DialogProps};
use crate::ui::fields::{choose_path, input_box, labeled, meta_text, paragraph, path_field, select_field};
use crate::ui::icon;

/// A line with an icon in front (issues, notes).
pub(super) fn icon_line(ic: Icon, color: Rgba, text: impl Into<SharedString>) -> Div {
    div()
        .flex()
        .items_start()
        .gap(px(8.))
        .child(div().pt(px(2.)).flex_none().child(icon(ic, px(14.), color)))
        .child(div().flex_1().min_w_0().text_size(px(13.)).line_height(px(18.)).text_color(theme::text_primary()).child(text.into()))
}

/// Monospace lines in a scrolling box.
pub(super) fn mono_box(id: &'static str, lines: impl IntoIterator<Item = String>, scroll: Option<&ScrollHandle>) -> Stateful<Div> {
    let mut b = div()
        .id(id)
        .flex()
        .flex_col()
        .max_h(px(220.))
        .overflow_y_scroll()
        .p(px(10.))
        .rounded(px(12.))
        .bg(theme::input_fill())
        .border_1()
        .border_color(theme::input_border());
    if let Some(s) = scroll {
        b = b.track_scroll(s);
    }
    b.children(lines.into_iter().map(|l| {
        div()
            .font_family(theme::FONT_MONO)
            .text_size(px(12.))
            .line_height(px(17.))
            .text_color(theme::text_muted())
            .whitespace_normal()
            .child(l)
    }))
}

/// The spinner icon, turning (still when animations are off).
pub(super) fn spinner(id: &'static str, size: f32, color: Rgba) -> AnyElement {
    let i = icon(Icon::FaSpinner, px(size), color);
    if !theme::animations() {
        return i.into_any_element();
    }
    i.with_animation(id, Animation::new(std::time::Duration::from_secs(1)).repeat(), |s, t| {
        s.with_transformation(Transformation::rotate(percentage(t)))
    })
    .into_any_element()
}

/// "Show …" / "Hide …" disclosure row.
pub(super) fn disclosure(id: &'static str, open: bool, label: &str) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .items_center()
        .gap(px(6.))
        .py(px(6.))
        .cursor_pointer()
        .text_size(px(13.))
        .text_color(theme::accent())
        .child(icon(if open { Icon::ExpandMore } else { Icon::ChevronRight }, px(16.), theme::accent()))
        .child(label.to_string())
}

impl ScriptInstallDialog {
    fn picker_body(&self, cx: &mut Context<Self>) -> Div {
        let mut body = div().flex().flex_col().gap(px(14.));
        if self.loading {
            return body.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .child(spinner("si-reading", 16., theme::text_primary()))
                    .child(paragraph(tr!("Reading the script\u{2026}"))),
            );
        }
        if let Some(err) = &self.load_error {
            body = body.child(icon_line(Icon::Warning, theme::danger_text(), err.clone()));
            if let Some(f) = &self.file {
                body = body.child(meta_text(f.display().to_string()));
            }
        } else {
            body = body.child(paragraph(tr!(
                "Install a game from an install script: a YAML file that says which files to get, how to set the game up (Wine prefix, setup program, extra components, fixes) and how it starts. The game is added to your Collection when it is done."
            )));
        }
        body.child(div().flex().child(action_button(
            "si-choose",
            ButtonKind::Primary,
            tr!("Choose a script file"),
            true,
            cx.listener(|this, _, window, cx| this.choose_file(window, cx)),
        )))
    }

    fn header(&mut self, cx: &mut Context<Self>) -> Div {
        let Some(script) = self.script() else { return div() };
        let mut meta = vec![script.variant()];
        if let Some(y) = &script.year {
            meta.push(y.clone());
        }
        let mut col = div()
            .flex()
            .flex_col()
            .gap(px(6.))
            .child(div().text_size(px(18.)).font_weight(theme::W_BOLD).text_color(theme::text_primary()).child(script.title().to_string()))
            .child(meta_text(meta.join(" \u{b7} ")));
        if !script.description.is_empty() {
            col = col.child(paragraph(script.description.clone()));
        }
        if let Some(f) = &self.file {
            col = col.child(meta_text(f.display().to_string()));
        }
        if self.scripts.len() > 1 {
            let options: Vec<(SharedString, bool)> = self
                .scripts
                .iter()
                .enumerate()
                .map(|(i, (s, _))| (format!("{} ({})", s.title(), s.variant()).into(), i == self.pick))
                .collect();
            let current = options.get(self.pick).map(|o| o.0.clone()).unwrap_or_default();
            col = col.child(select_field(
                "si-script",
                Some(tr!("This file has several scripts")),
                current,
                false,
                self.open_select == Some(Select::Script),
                options,
                cx,
                |this: &mut Self, _, cx| {
                    this.open_select = if this.open_select == Some(Select::Script) { None } else { Some(Select::Script) };
                    cx.notify();
                },
                |this: &mut Self, i, window, cx| {
                    this.open_select = None;
                    this.set_pick(i, window, cx);
                },
            ));
        }
        col
    }

    fn issues(&self) -> Option<Div> {
        let check = self.check()?;
        let script = self.script()?;
        let mut lines: Vec<Div> = check
            .issues
            .iter()
            .map(|Issue { text, blocking }| {
                if *blocking {
                    icon_line(Icon::Cancel, theme::danger_text(), text.clone())
                } else {
                    icon_line(Icon::Warning, rgb(0xebcb8b), text.clone())
                }
            })
            .collect();
        if !script.notes.is_empty() {
            lines.push(icon_line(Icon::Description, theme::text_muted(), script.preview(&script.notes)));
        }
        if check.interactive {
            lines.push(icon_line(Icon::Description, theme::text_muted(), tr!("The script asks questions while it installs.")));
        }
        (!lines.is_empty()).then(|| card_box().gap(px(10.)).py(px(12.)).children(lines))
    }

    fn game_fields(&mut self, cx: &mut Context<Self>) -> Div {
        let has_folder = !text_of(&self.folder, cx).is_empty();
        let mut col = div().flex().flex_col().gap(px(12.)).child(labeled(tr!("Title"), input_box(&self.title, false))).child(path_field(
            "si-folder",
            Some(tr!("Install folder")),
            &self.folder,
            has_folder,
            false,
            cx.listener(move |this, _, window, cx| {
                if has_folder {
                    this.folder.update(cx, |s, cx| s.set_value("", window, cx));
                } else {
                    choose_path(window, cx, true, tr!("Select install path"), |this: &mut Self, p, window, cx| {
                        let name = this.script().map(|s| s.folder_name()).unwrap_or_default();
                        let value = format!("{}/{name}", p.trim_end_matches('/'));
                        this.folder.update(cx, |s, cx| s.set_value(value, window, cx));
                    });
                }
            }),
        ));
        if self.folder_used {
            col = col.child(icon_line(
                Icon::Warning,
                rgb(0xebcb8b),
                tr!("This folder already has files. The script may replace some of them."),
            ));
        }
        col
    }

    /// The sandbox of the game, which the install also runs in.
    fn sandbox_field(&mut self, cx: &mut Context<Self>) -> Div {
        let s = self.sandbox;
        let mut rows = vec![toggle_row(
            "si-sandbox",
            tr!("Install and play in a sandbox"),
            Some(tr!("The script's programs, and later the game, run without network and without your home folder. The game's folder is writable while it installs.")),
            s.enabled,
        )
        .on_click(cx.listener(|this, _, _, cx| this.toggle_sandbox(false, cx)))
        .into_any_element()];
        if s.enabled {
            rows.push(
                toggle_row(
                    "si-sandbox-net",
                    tr!("Allow the Internet"),
                    Some(tr!(
                        "Scripts that install components with Winetricks or sign in need it. A program could send out what it can read."
                    )),
                    s.network,
                )
                .on_click(cx.listener(|this, _, _, cx| this.toggle_sandbox(true, cx)))
                .into_any_element(),
            );
        }
        let mut col = div().flex().flex_col().child(caption(tr!("Sandbox"))).child(card(rows));
        if let Some(why) = self.sandbox_problem.clone().filter(|_| s.enabled) {
            col = col.child(div().mt(px(8.)).child(icon_line(Icon::Warning, rgb(0xebcb8b), why)));
        }
        col
    }

    fn files_to_pick(&mut self, cx: &mut Context<Self>) -> Option<Div> {
        let asks = self.check()?.asks.clone();
        if asks.is_empty() {
            return None;
        }
        let rows = asks.into_iter().enumerate().map(|(i, (id, what))| {
            let chosen = self.picked.get(&id).map(|p| p.display().to_string());
            let label = chosen.clone().unwrap_or_else(|| tr!("Not selected").into());
            let pick_id = id.clone();
            let prompt = what.clone();
            div()
                .flex()
                .items_center()
                .gap(px(12.))
                .py(px(10.))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .min_w_0()
                        .gap(px(2.))
                        .child(div().text_size(px(14.)).text_color(theme::text_primary()).child(what))
                        .child(
                            div()
                                .text_size(px(12.))
                                .line_height(px(16.))
                                .text_color(if chosen.is_some() { theme::text_dim() } else { theme::danger_text() })
                                .child(label),
                        ),
                )
                .child(button_sm(SharedString::from(format!("si-file-{i}")), ButtonKind::Glass, tr!("Select\u{2026}")).on_click(
                    cx.listener(move |_, _, window, cx| {
                        let id = pick_id.clone();
                        choose_path(window, cx, false, &prompt, move |this: &mut Self, p, _, cx| {
                            this.picked.insert(id, p.into());
                            cx.notify();
                        });
                    }),
                ))
                .into_any_element()
        });
        Some(div().flex().flex_col().child(caption(tr!("Files you provide"))).child(card(rows)))
    }

    fn wine_field(&mut self, cx: &mut Context<Self>) -> Option<Div> {
        if !self.check()?.needs_wine() {
            return None;
        }
        let list = self.wine_list.clone();
        let shown = match (&list, &self.wine) {
            (None, _) => tr!("Loading\u{2026}").to_string(),
            (Some(_), Some(w)) => display_wine_name(&w.name),
            (Some(_), None) => crate::dialogs::no_wine_text(cx),
        };
        let options = match (&list, &self.wine) {
            (Some(l), Some(w)) => wine_options(l, w),
            (Some(l), None) => l.clone(),
            _ => Vec::new(),
        };
        let items: Vec<(SharedString, bool)> = options
            .iter()
            .map(|w| (SharedString::from(display_wine_name(&w.name)), self.wine.as_ref().is_some_and(|c| c.name == w.name)))
            .collect();
        let mut col = div().flex().flex_col().gap(px(8.)).child(select_field(
            "si-wine",
            Some(tr!("Wine Version")),
            shown,
            options.is_empty(),
            self.open_select == Some(Select::Wine),
            items,
            cx,
            |this: &mut Self, _, cx| {
                this.open_select = if this.open_select == Some(Select::Wine) { None } else { Some(Select::Wine) };
                cx.notify();
            },
            move |this: &mut Self, i, _, cx| {
                this.wine = options.get(i).cloned();
                this.open_select = None;
                cx.notify();
            },
        ));
        if let Some(wanted) = self.script().and_then(|s| s.runner_config.get("version")).and_then(heronite_installer::text::scalar) {
            if !wanted.is_empty() && self.wine.as_ref().is_some_and(|w| !w.name.eq_ignore_ascii_case(&wanted)) {
                col = col.child(meta_text(tr!("The script was made for Wine {wanted}.", wanted)));
            }
        }
        if list.as_ref().is_some_and(|l| l.is_empty()) {
            col = col.child(crate::dialogs::no_proton_note(cx));
        }
        Some(col)
    }

    fn steps(&mut self, cx: &mut Context<Self>) -> Option<Div> {
        let script = self.script()?;
        let check = self.check()?;
        let mut lines: Vec<String> = Vec::new();
        if !check.downloads.is_empty() {
            let names: Vec<String> = check.downloads.iter().map(|(n, _)| n.clone()).collect();
            lines.push(tr!("Download {files}", files = names.join(", ")));
        }
        lines.extend(script.steps.iter().map(|s| script.preview(&s.summary())));
        let rows = lines.into_iter().enumerate().map(|(i, text)| {
            div()
                .flex()
                .gap(px(10.))
                .py(px(7.))
                .child(div().w(px(22.)).flex_none().text_size(px(12.)).text_color(theme::text_dim()).child(format!("{}.", i + 1)))
                .child(div().flex_1().min_w_0().text_size(px(13.)).line_height(px(18.)).text_color(theme::text_primary()).child(text))
                .into_any_element()
        });
        let mut col = div()
            .flex()
            .flex_col()
            .child(caption(trn!(script.steps.len() as u64, "What the script does ({n} step)", "What the script does ({n} steps)")))
            .child(card(rows));
        if !check.runs.is_empty() {
            let open = self.show_runs;
            col = col.child(disclosure("si-runs", open, tr!("Programs and commands it runs")).on_click(cx.listener(|this, _, _, cx| {
                this.show_runs = !this.show_runs;
                cx.notify();
            })));
            if open {
                col = col.child(mono_box("si-runs-box", check.runs.clone(), None));
            }
        }
        if script.steps.len() > 1 {
            let options: Vec<(SharedString, bool)> = script
                .steps
                .iter()
                .enumerate()
                .map(|(i, s)| (format!("{}. {}", i + 1, script.preview(&s.summary())).into(), i == self.start_at))
                .collect();
            let current = options.get(self.start_at).map(|o| o.0.clone()).unwrap_or_default();
            col = col.child(div().mt(px(10.)).child(select_field(
                "si-start",
                Some(tr!("Start at")),
                current,
                false,
                self.open_select == Some(Select::Start),
                options,
                cx,
                |this: &mut Self, _, cx| {
                    this.open_select = if this.open_select == Some(Select::Start) { None } else { Some(Select::Start) };
                    cx.notify();
                },
                |this: &mut Self, i, _, cx| {
                    this.start_at = i;
                    this.open_select = None;
                    cx.notify();
                },
            )));
            if self.start_at > 0 {
                col = col.child(div().mt(px(6.)).child(meta_text(tr!(
                    "The steps before it are skipped: use this to finish an install whose first steps already ran in this folder."
                ))));
            }
        }
        Some(col)
    }

    fn review_body(&mut self, cx: &mut Context<Self>) -> Div {
        let mut body = div().flex().flex_col().gap(px(16.)).child(self.header(cx));
        body = body.children(self.issues());
        body = body.child(self.game_fields(cx));
        body = body.children(self.wine_field(cx));
        body = body.child(self.sandbox_field(cx));
        body = body.children(self.files_to_pick(cx));
        body = body.children(self.steps(cx));
        body.child(meta_text(tr!(
            "Install scripts run programs and commands on your computer. Only install scripts from sources you trust."
        )))
    }

    fn review_footer(&mut self, cx: &mut Context<Self>) -> Div {
        let hint = if self.error.is_empty() { self.not_ready(cx) } else { Some(self.error.clone()) };
        let ready = self.not_ready(cx).is_none();
        div()
            .flex()
            .items_center()
            .w_full()
            .gap(px(12.))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_size(px(12.5))
                    .line_height(px(17.))
                    .text_color(if self.error.is_empty() { theme::text_muted() } else { theme::danger_text() })
                    .children(hint),
            )
            .child(action_button(
                "si-other",
                ButtonKind::Ghost,
                tr!("Choose another file"),
                true,
                cx.listener(|this, _, window, cx| this.choose_file(window, cx)),
            ))
            .child(action_button(
                "si-install",
                ButtonKind::Primary,
                tr!("Install"),
                ready,
                cx.listener(|this, _, window, cx| this.start(window, cx)),
            ))
    }
}

impl Render for ScriptInstallDialog {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let vw = f32::from(window.viewport_size().width);
        let face_title = if self.in_add_game { tr!("Add game") } else { tr!("Install from script") };
        let (title, body, footer) = if self.running(cx) {
            let (t, b, f) = self.run_view(cx);
            (t, b, Some(f))
        } else if self.scripts.is_empty() {
            (face_title.to_string(), self.picker_body(cx), None)
        } else {
            (face_title.to_string(), self.review_body(cx), Some(self.review_footer(cx)))
        };
        let entity = cx.entity().downgrade();
        // The size of the scroll area is only known after a first layout.
        if self.scroll.bounds().size.height <= px(0.) {
            let view = cx.entity_id();
            cx.defer(move |cx| cx.notify(view));
        }
        let close = move |_: &mut Window, cx: &mut App| {
            entity.update(cx, |_, cx| cx.emit(DialogEvent::Close)).ok();
        };
        let height = crate::dialogs::settings_panel::debug_dialog_height();
        let (body, footer) = (body.into_any_element(), footer.map(IntoElement::into_any_element));
        if self.in_add_game {
            // Same id and width as "Add game": the step replaces its form in place.
            let props = DialogProps { id: "add-game".into(), title: title.into(), width: 860f32.min(vw * 0.96), height };
            let back = cx.entity().downgrade();
            return dialog_scrolled_with_back(
                props,
                &self.scroll,
                body,
                footer,
                move |_, cx| {
                    back.update(cx, |_, cx| cx.emit(BackToForm)).ok();
                },
                close,
            );
        }
        let props = DialogProps { id: "script-install".into(), title: title.into(), width: 720f32.min(vw * 0.96), height };
        dialog_scrolled(props, &self.scroll, body, footer, close)
    }
}
