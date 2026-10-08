//! `MergePlayniteDialog`: previews what merging the saved Playnite library
//! into Heronite would change (new games, playtime, status, conflicts,
//! sessions) and applies it. Backend: `Playnite::{preview_merge, merge}`.

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_playnite::{Playnite, PlayniteImportResult, PlayniteMergePreview, PlayniteMergePreviewItem};

use super::playnite_import::{action, bullet_list, stat_card};
use super::{DialogCtx, DialogEvent};
use crate::theme;
use crate::ui::controls::{button, section_title, ButtonKind};
use crate::ui::dialog::{dialog, DialogProps};
use crate::ui::fields::{paragraph, paragraph_secondary};

/// Rows shown per section before "+N".
const VISIBLE: usize = 12;

pub struct MergePlayniteDialog {
    ctx: DialogCtx,
    preview: Option<PlayniteMergePreview>,
    loading: bool,
    merging: bool,
    result: Option<PlayniteImportResult>,
    error: Option<SharedString>,
}

impl EventEmitter<DialogEvent> for MergePlayniteDialog {}

fn minutes(v: Option<heronite_data::JsNum>) -> String {
    format!("{}m", v.map(|n| n.get()).unwrap_or(0.0).round())
}

fn section(title: &str, items: &[PlayniteMergePreviewItem], line: impl Fn(&PlayniteMergePreviewItem) -> String) -> Div {
    let mut s = div().flex().flex_col().child(section_title(title.to_string()));
    if items.is_empty() {
        return s.child(paragraph_secondary("None").mx(px(6.)));
    }
    s = s.child(bullet_list(items.iter().take(VISIBLE).map(line).collect()));
    if items.len() > VISIBLE {
        s = s.child(paragraph_secondary(format!("+{}", items.len() - VISIBLE)).mx(px(6.)).mt(px(6.)));
    }
    s
}

impl MergePlayniteDialog {
    pub fn new(ctx: DialogCtx, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let heroic = ctx.heroic.clone();
        cx.spawn_in(window, async move |this, cx| {
            let res = cx.background_executor().spawn(async move { Playnite::new(heroic).preview_merge() }).await;
            this.update(cx, |this, cx| {
                this.loading = false;
                match res {
                    Ok(p) => {
                        if !p.errors.is_empty() {
                            this.error = Some(p.errors.join("\n").into());
                        }
                        this.preview = Some(p);
                    }
                    Err(err) => this.error = Some(format!("{err:#}").into()),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        Self { ctx, preview: None, loading: true, merging: false, result: None, error: None }
    }

    fn run_merge(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.merging {
            return;
        }
        self.merging = true;
        self.error = None;
        let heroic = self.ctx.heroic.clone();
        cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            let res = cx.background_executor().spawn(async move { Playnite::new(heroic).merge() }).await;
            this.update(cx, |this, cx| {
                this.merging = false;
                match res {
                    Ok(r) => {
                        let failed = !r.errors.is_empty() && r.imported + r.updated == 0;
                        if failed {
                            this.error = Some(r.errors.join("\n").into());
                        } else {
                            this.result = Some(r);
                            cx.emit(DialogEvent::Changed);
                        }
                    }
                    Err(err) => this.error = Some(format!("{err:#}").into()),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }
}

impl Render for MergePlayniteDialog {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut body = div().flex().flex_col().gap(px(8.));
        if self.loading {
            body = body.child(paragraph("Reading Playnite library\u{2026}"));
        } else {
            body = body.child(paragraph(
                "This merges Playnite into Heroic. Status you changed here is kept. Playtime takes the higher value. \
                 New Playnite games are added.",
            ));
            if let Some(p) = self.preview.as_ref().filter(|p| p.errors.is_empty()) {
                body = body
                    .child(stat_card(vec![
                        ("New games".into(), p.new_games.len().to_string()),
                        ("Playtime increases".into(), p.playtime_updates.len().to_string()),
                        ("Status from Playnite".into(), p.status_from_playnite.len().to_string()),
                        ("Status conflicts (keep Heroic)".into(), p.status_conflicts.len().to_string()),
                        ("New sessions".into(), p.new_sessions.to_string()),
                        ("Unchanged".into(), p.unchanged.to_string()),
                        ("Skipped".into(), p.skipped.to_string()),
                    ]))
                    .child(section("New games", &p.new_games, |i| i.title.clone()))
                    .child(section("Playtime increases", &p.playtime_updates, |i| {
                        format!("{}: {} \u{2192} {}", i.title, minutes(i.heroic_minutes), minutes(i.playnite_minutes))
                    }))
                    .child(section("Status from Playnite", &p.status_from_playnite, |i| {
                        format!(
                            "{}: {} \u{2192} {}",
                            i.title,
                            i.heroic_status.clone().unwrap_or_default(),
                            i.playnite_status.clone().unwrap_or_default()
                        )
                    }))
                    .child(section("Status conflicts (keep Heroic)", &p.status_conflicts, |i| {
                        format!(
                            "{}: {} / {}",
                            i.title,
                            i.heroic_status.clone().unwrap_or_default(),
                            i.playnite_status.clone().unwrap_or_default()
                        )
                    }));
            }
        }
        if let Some(r) = &self.result {
            body = body.child(div().text_size(px(14.)).text_color(theme::success()).child(format!(
                "Merged without overwriting your Heroic status. New games {}, updated {}, sessions {}, skipped {}.",
                r.imported, r.updated, r.sessions_imported, r.skipped
            )));
            if !r.errors.is_empty() {
                body = body.child(
                    div().text_size(px(13.33)).text_color(theme::danger_text()).child(
                        r.errors.iter().take(4).cloned().collect::<Vec<_>>().join("\n"),
                    ),
                );
            }
        }
        if let Some(e) = self.error.clone() {
            body = body.child(div().text_size(px(14.)).text_color(theme::danger_text()).child(e));
        }

        let blocked = self.preview.as_ref().map_or(true, |p| p.library_path.is_empty() || !p.errors.is_empty());
        let done = self.result.is_some();
        let footer = div()
            .flex()
            .gap(px(10.))
            .child(
                button("pm-close", ButtonKind::Ghost, "Close").on_click(cx.listener(move |_, _, _, cx| {
                    cx.emit(if done { DialogEvent::ChangedAndClose } else { DialogEvent::Close })
                })),
            )
            .when(!done, |d| {
                d.child(action(
                    "pm-merge",
                    ButtonKind::Primary,
                    if self.merging { "Merging\u{2026}" } else { "Merge" },
                    !self.loading && !self.merging && !blocked,
                    cx.listener(|this, _, w, cx| this.run_merge(w, cx)),
                ))
            })
            .into_any_element();

        let entity = cx.entity().downgrade();
        let vw = f32::from(window.viewport_size().width);
        dialog(
            DialogProps {
                id: "merge-playnite".into(),
                title: "Playnite merge".into(),
                width: 560f32.min(vw * 0.9) + 64.,
                height: None,
            },
            body.into_any_element(),
            Some(footer),
            move |_, cx| {
                entity
                    .update(cx, |this, cx| {
                        cx.emit(if this.result.is_some() { DialogEvent::ChangedAndClose } else { DialogEvent::Close })
                    })
                    .ok();
            },
        )
    }
}
