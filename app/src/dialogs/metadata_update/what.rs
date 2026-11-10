//! Step 2, what to update: how (fill / refresh / replace), the fields in
//! three columns and the sources to ask, then a summary and "Update N games".
//! Replace everything asks once before it starts: it overwrites hand edits.

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::cover_family::CoverFamily;
use heronite_i18n::{tr, trn};
use heronite_metadata::{bulk_fields, BulkArgs, UpdateMode};

use super::{footer_note, tick, MetadataUpdate, Step};
use crate::icons::Icon;
use crate::pages::settings::metadata_labels as labels;
use crate::theme;
use crate::ui::controls::{action_button, button, icon_action_button, ButtonKind};
use crate::ui::icon;

/// The columns of fields, in the order shown. A field of `bulk_fields` in
/// none of them goes to the last one.
const DETAILS: [&str; 10] =
    ["description", "releaseDate", "developers", "publishers", "genres", "themes", "gameModes", "platforms", "series", "features"];
/// The pictures: a cover per family (those not kept are left out), then the background.
const PICTURES: [&str; 4] = ["cover", "coverSquare", "coverWide", "hero"];

/// `kept`: the cover families this installation keeps.
fn columns(kept: &[CoverFamily]) -> [(&'static str, Vec<String>); 3] {
    let all: Vec<String> = bulk_fields().into_iter().filter(|f| CoverFamily::from_field(f).is_none_or(|c| kept.contains(&c))).collect();
    let pick = |list: &[&str]| list.iter().filter(|f| all.iter().any(|a| a == *f)).map(|f| f.to_string()).collect::<Vec<_>>();
    let extras = all.iter().filter(|f| !DETAILS.contains(&f.as_str()) && !PICTURES.contains(&f.as_str())).cloned().collect();
    [(tr!("Details"), pick(&DETAILS)), (tr!("Pictures"), pick(&PICTURES)), (tr!("Extras"), extras)]
}

fn mode_title(m: UpdateMode) -> &'static str {
    match m {
        UpdateMode::Fill => tr!("Fill what is empty"),
        UpdateMode::Refresh => tr!("Refresh"),
        UpdateMode::Replace => tr!("Replace everything"),
    }
}

fn mode_line(m: UpdateMode) -> &'static str {
    match m {
        UpdateMode::Fill => tr!("Nothing you have changes"),
        UpdateMode::Refresh => tr!("What you edited by hand stays"),
        UpdateMode::Replace => tr!("Your edits too"),
    }
}

/// The mode in the footer's summary.
fn mode_short(m: UpdateMode) -> &'static str {
    match m {
        UpdateMode::Fill => tr!("empty fields only"),
        UpdateMode::Refresh => tr!("keeps hand edits"),
        UpdateMode::Replace => tr!("replaces hand edits"),
    }
}

impl MetadataUpdate {
    /// The fields ticked; the covers of a family that is not kept have no
    /// tick and are left out (a job would not fill them anyway).
    fn fields_on(&self) -> Vec<String> {
        let kept = |f: &String| CoverFamily::from_field(f).is_none_or(|c| self.kept.contains(&c));
        bulk_fields().into_iter().filter(|f| !self.fields_off.contains(f) && kept(f)).collect()
    }

    /// The sources this run asks: on in the dialog and with something for
    /// these games (once counted).
    fn sources_on(&self) -> Vec<String> {
        let reach = self.counts.as_ref().filter(|(s, _)| *s == self.scope).map(|(_, c)| &c.reach);
        self.sources
            .iter()
            .flatten()
            .filter(|d| !self.sources_off.contains(&d.id))
            .filter(|d| reach.and_then(|r| r.get(&d.id)).is_none_or(|n| *n > 0))
            .map(|d| d.id.clone())
            .collect()
    }

    fn args(&self) -> BulkArgs {
        let fields = self.fields_on();
        let fields = (fields.len() < bulk_fields().len()).then_some(fields);
        // Every source that is on: `None`, so the job may mark a game no
        // source has as failed.
        let sources = (!self.sources_off.is_empty()).then(|| self.sources_on());
        BulkArgs::update(self.chosen().to_vec(), self.mode, fields, sources)
    }

    fn start(&mut self, cx: &mut Context<Self>) {
        if self.mode == UpdateMode::Replace && !self.replace_ok {
            self.confirm_replace = true;
            cx.notify();
            return;
        }
        let args = self.args();
        let id = self.hub.update(cx, |h, cx| h.start(args, cx));
        self.started = Some(id);
        self.filter = None;
        self.confirm_replace = false;
        self.list.scroll_to_item(0, ScrollStrategy::Top);
        self.go(Step::Update, cx);
    }

    fn render_modes(&self, cx: &mut Context<Self>) -> Div {
        let mut row = div().flex().gap(px(8.));
        for m in [UpdateMode::Fill, UpdateMode::Refresh, UpdateMode::Replace] {
            let on = self.mode == m;
            row = row.child(
                div()
                    .id(SharedString::from(format!("mu-mode-{m:?}")))
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .gap(px(2.))
                    .px(px(14.))
                    .py(px(10.))
                    .rounded(px(theme::R_TILE))
                    .border_1()
                    .cursor_pointer()
                    .when(on, |d| d.bg(theme::accent_tint()).border_color(theme::accent_tint_border()))
                    .when(!on, |d| d.bg(theme::card_fill()).border_color(theme::card_border()).hover(|s| s.bg(theme::ink_a(0.06))))
                    .child(div().text_size(px(13.5)).text_color(theme::text_primary()).child(mode_title(m)))
                    .child(div().truncate().text_size(px(12.)).text_color(theme::text_dim()).child(mode_line(m)))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.mode = m;
                        this.confirm_replace = false;
                        cx.notify();
                    })),
            );
        }
        row
    }

    fn render_fields(&self, cx: &mut Context<Self>) -> Div {
        let mut grid = div()
            .flex()
            .gap(px(16.))
            .px(px(16.))
            .py(px(12.))
            .rounded(px(theme::R_CARD))
            .bg(theme::card_fill())
            .border_1()
            .border_color(theme::card_border());
        for (c, (title, fields)) in columns(&self.kept).into_iter().enumerate() {
            if fields.is_empty() {
                continue;
            }
            let all_on = fields.iter().all(|f| !self.fields_off.contains(f));
            let toggle_all = fields.clone();
            let head = div()
                .flex()
                .items_center()
                .justify_between()
                .mb(px(6.))
                .child(div().text_size(px(12.)).text_color(theme::text_muted()).child(title))
                .child(
                    div()
                        .id(("mu-all", c))
                        .text_size(px(12.))
                        .text_color(theme::accent())
                        .cursor_pointer()
                        .hover(|s| s.text_color(theme::text_bright()))
                        .child(tr!("All"))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            for f in &toggle_all {
                                if all_on {
                                    this.fields_off.insert(f.clone());
                                } else {
                                    this.fields_off.remove(f);
                                }
                            }
                            cx.notify();
                        })),
                );
            // The long Details column lays its ticks out in two.
            let wide = fields.len() > 6 && c == 0;
            let mut ticks = div().flex().flex_wrap();
            for f in fields {
                let on = !self.fields_off.contains(&f);
                let label = labels::field_name(&f).to_string();
                ticks = ticks.child(
                    div()
                        .id(SharedString::from(format!("mu-field-{f}")))
                        .w(if wide { relative(0.5) } else { relative(1.) })
                        .flex()
                        .items_center()
                        .gap(px(10.))
                        .py(px(4.))
                        .cursor_pointer()
                        .text_size(px(13.5))
                        .text_color(if on { theme::text_primary() } else { theme::text_muted() })
                        .child(tick(on))
                        .child(div().min_w_0().truncate().child(label))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if !this.fields_off.remove(&f) {
                                this.fields_off.insert(f.clone());
                            }
                            cx.notify();
                        })),
                );
            }
            let col = div().flex().flex_col().min_w_0().child(head).child(ticks);
            grid = grid.child(if wide { col.flex_grow(2.).flex_shrink(1.).flex_basis(px(0.)) } else { col.flex_1() });
        }
        grid
    }

    fn render_sources(&self, cx: &mut Context<Self>) -> Div {
        let counts = self.counts.as_ref().filter(|(s, _)| *s == self.scope).map(|(_, c)| c);
        let reach = counts.map(|c| c.reach.clone());
        let waiting: Vec<String> = counts.map(|c| c.needs_download.clone()).unwrap_or_default();
        let mut chips = div().flex().flex_wrap().gap(px(6.));
        let mut empty = Vec::new();
        let mut not_downloaded = Vec::new();
        for d in self.sources.iter().flatten() {
            let name = labels::name(&d.name).to_string();
            let useless = reach.as_ref().and_then(|r| r.get(&d.id)).is_some_and(|n| *n == 0);
            if waiting.contains(&d.id) {
                not_downloaded.push(name.clone());
            } else if useless {
                empty.push(name.clone());
            }
            let on = !self.sources_off.contains(&d.id) && !useless;
            let id = d.id.clone();
            chips = chips.child(
                div()
                    .id(SharedString::from(format!("mu-src-{}", d.id)))
                    .flex()
                    .items_center()
                    .gap(px(7.))
                    .h(px(30.))
                    .pl(px(5.))
                    .pr(px(11.))
                    .rounded_full()
                    .border_1()
                    .text_size(px(12.5))
                    .when(on, |el| el.bg(theme::chip_fill()).border_color(theme::ink_a(0.12)).text_color(theme::text_primary()))
                    .when(!on, |el| el.border_color(theme::ink_a(0.08)).text_color(theme::text_dim()).line_through())
                    .when(useless, |el| el.opacity(0.6))
                    .when(!useless, |el| {
                        el.cursor_pointer().hover(|s| s.bg(theme::ink_a(0.1))).on_click(cx.listener(move |this, _, _, cx| {
                            if !this.sources_off.remove(&id) {
                                this.sources_off.insert(id.clone());
                            }
                            cx.notify();
                        }))
                    })
                    .child(labels::mark(&d.id, &name, 20.).when(!on, |m| m.opacity(0.5)))
                    .child(name),
            );
        }
        let mut note = tr!("In your order. Click one to leave it out of this update.").to_string();
        if !empty.is_empty() {
            note = format!("{note} {}", tr!("Nothing for these games: {sources}.", sources = empty.join(", ")));
        }
        if !not_downloaded.is_empty() {
            let sources = not_downloaded.join(", ");
            note = format!("{note} {}", tr!("Download its database in Settings → Metadata first: {sources}.", sources));
        }
        div()
            .flex()
            .flex_col()
            .gap(px(8.))
            .child(
                div()
                    .flex()
                    .items_baseline()
                    .gap(px(10.))
                    .child(div().flex_none().text_size(px(12.)).text_color(theme::text_muted()).child(tr!("From")))
                    .child(div().min_w_0().text_size(px(12.)).text_color(theme::text_dim()).child(note)),
            )
            .child(chips)
    }

    pub(super) fn render_what(&mut self, cx: &mut Context<Self>) -> Div {
        div()
            .flex()
            .flex_col()
            .gap(px(14.))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(6.))
                    .child(div().text_size(px(12.)).text_color(theme::text_muted()).child(tr!("How")))
                    .child(self.render_modes(cx)),
            )
            .child(self.render_fields(cx))
            .child(self.render_sources(cx))
    }

    /// "7 games · empty fields only · 15 fields · 5 sources".
    fn summary(&self) -> String {
        let (games, fields, sources) = (self.chosen().len(), self.fields_on().len(), self.sources_on().len());
        [
            trn!(games, "{n} game", "{n} games"),
            mode_short(self.mode).to_string(),
            trn!(fields, "{n} field", "{n} fields"),
            trn!(sources, "{n} source", "{n} sources"),
        ]
        .join(" \u{b7} ")
    }

    pub(super) fn what_footer(&self, cx: &mut Context<Self>) -> Div {
        let n = self.chosen().len();
        let row = div().flex().flex_1().items_center().gap(px(10.));
        if self.confirm_replace {
            return row
                .child(
                    div()
                        .flex()
                        .flex_1()
                        .min_w_0()
                        .items_center()
                        .gap(px(8.))
                        .child(icon(Icon::Warning, px(16.), theme::caution_text()))
                        .child(div().min_w_0().text_size(px(12.5)).text_color(theme::caution_text()).child(trn!(
                            n,
                            "This replaces what you edited by hand in this game.",
                            "This replaces what you edited by hand in these {n} games."
                        ))),
                )
                .child(button("mu-replace-no", ButtonKind::Ghost, tr!("Cancel")).on_click(cx.listener(|this, _, _, cx| {
                    this.confirm_replace = false;
                    cx.notify();
                })))
                .child(button("mu-replace-yes", ButtonKind::Danger, trn!(n, "Replace in {n} game", "Replace in {n} games")).on_click(
                    cx.listener(|this, _, _, cx| {
                        this.replace_ok = true;
                        this.start(cx);
                    }),
                ));
        }
        let ready = n > 0 && !self.fields_on().is_empty() && !self.sources_on().is_empty() && self.sources.is_some();
        let waits = self.hub.read(cx).running();
        let update = if ready {
            icon_action_button(
                "mu-start",
                ButtonKind::Primary,
                Icon::CloudDownload,
                trn!(n, "Update {n} game", "Update {n} games"),
                cx.listener(|this, _, _, cx| this.start(cx)),
            )
        } else {
            action_button("mu-start", ButtonKind::Primary, trn!(n, "Update {n} game", "Update {n} games"), false, |_, _, _| {})
        };
        row.child(footer_note(if waits { tr!("It starts after the update running now").to_string() } else { self.summary() }))
            .child(button("mu-back", ButtonKind::Glass, tr!("Back")).on_click(cx.listener(|this, _, _, cx| this.go(Step::Games, cx))))
            .child(update)
    }
}
