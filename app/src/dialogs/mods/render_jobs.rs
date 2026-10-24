//! Installed tab, top: the notices that matter now and the downloads and
//! installs of this game.

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_i18n::tr;

use super::{format, ModsDialog, View};
use crate::icons::Icon;
use crate::services::mods::{HandlerState, JobStage};
use crate::services::widgets::{busy_bar, progress_bar};
use crate::theme;
use crate::ui::controls::{button_sm, ButtonKind};
use crate::ui::icon;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Tone {
    Info,
    Warn,
    Error,
}

impl Tone {
    pub fn color(self) -> Rgba {
        match self {
            Tone::Info => theme::accent(),
            Tone::Warn => theme::warning(),
            Tone::Error => theme::danger_text(),
        }
    }
}

/// A tinted card with a leading icon; the caller adds the content.
pub(super) fn tinted(tone: Tone, ic: Icon) -> Div {
    let c: Hsla = tone.color().into();
    div()
        .flex()
        .gap(px(12.))
        .px(px(14.))
        .py(px(12.))
        .rounded(px(14.))
        .bg(Hsla { a: if tone == Tone::Info { 0.08 } else { 0.11 }, ..c })
        .border_1()
        .border_color(Hsla { a: 0.28, ..c })
        .child(div().flex_none().pt(px(1.)).child(icon(ic, px(18.), tone.color())))
}

/// Text of a notice: an optional title and a muted body.
fn words(title: Option<&'static str>, text: impl Into<SharedString>) -> Div {
    div()
        .flex()
        .flex_col()
        .flex_1()
        .min_w_0()
        .gap(px(2.))
        .children(title.map(|t| div().text_size(px(14.)).font_weight(theme::W_500).text_color(theme::text_primary()).child(t)))
        .child(div().text_size(px(13.)).line_height(px(18.)).text_color(theme::text_muted()).child(text.into()))
}

impl ModsDialog {
    /// The notices of the Installed tab, most urgent first.
    pub(super) fn notices(&mut self, v: &View, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let mut out = Vec::new();
        let g = &v.g;
        if g.info.game_dir.is_none() {
            out.push(
                tinted(Tone::Warn, Icon::Warning)
                    .child(words(
                        Some(tr!("Game folder not found")),
                        tr!("Heronite cannot find where this game is installed, so it cannot tell where a mod's files go. Install or locate the game first."),
                    ))
                    .into_any_element(),
            );
        }
        if let Some(problem) = v.support_problem.clone() {
            out.push(
                tinted(Tone::Warn, Icon::Warning)
                    .child(words(Some(tr!("Mods cannot be applied yet")), problem))
                    .child(
                        button_sm("mods-open-settings", ButtonKind::Glass, tr!("Open Settings \u{2192} Mods"))
                            .self_center()
                            .on_click(cx.listener(|this, _, _, cx| this.open_settings(cx))),
                    )
                    .into_any_element(),
            );
        }
        if self.running {
            out.push(
                tinted(Tone::Info, Icon::InfoOutlined)
                    .child(words(None, tr!("The game is running. Changes apply the next time you play.")))
                    .into_any_element(),
            );
        }
        out.extend(self.limit_notices(v));
        out.extend(self.loader_notices(v, cx));
        if let Some(value) = g.steam_overrides_missing.clone().filter(|_| !g.info.native) {
            out.push(
                tinted(Tone::Warn, Icon::Build)
                    .child(words(
                        Some(tr!("One setting for Steam")),
                        tr!(
                            "This game starts through Steam, so Heronite cannot set {value} itself. Add it to the game's launch options in Steam.",
                            value
                        ),
                    ))
                    .child(
                        button_sm("mods-steam-overrides", ButtonKind::Glass, tr!("Add to Steam launch options")).self_center().on_click(
                            cx.listener(|this, _, _, cx| {
                                let key = this.key.clone();
                                this.service.update(cx, |s, cx| s.add_steam_overrides(&key, cx));
                            }),
                        ),
                    )
                    .into_any_element(),
            );
        }
        // A Thunderstore game asks nothing of Nexus Mods until it has mods from there.
        let nexus_mods = g.list.mods.iter().any(|m| matches!(m.source, heronite_mods::ModSource::Nexus { .. }));
        if g.info.nexus_domain.is_some() && (nexus_mods || g.info.thunderstore.is_none()) {
            out.extend(self.nexus_notice(v, cx));
        }
        out
    }

    /// No account, or `nxm://` links that do not open Heronite.
    fn nexus_notice(&mut self, v: &View, cx: &mut Context<Self>) -> Option<AnyElement> {
        if v.account.is_none() {
            return Some(
                tinted(Tone::Info, Icon::AccountCircle)
                    .child(words(
                        Some(tr!("Connect Nexus Mods")),
                        tr!("Add your Nexus Mods API key to check these mods for updates and, with Premium, install straight from Browse."),
                    ))
                    .child(
                        button_sm("mods-nexus-account", ButtonKind::Glass, tr!("Open Settings \u{2192} Mods"))
                            .self_center()
                            .on_click(cx.listener(|this, _, _, cx| this.open_settings(cx))),
                    )
                    .into_any_element(),
            );
        }
        if matches!(v.handlers, HandlerState::None | HandlerState::Other) {
            let text = if v.handlers == HandlerState::Other {
                tr!("Another program opens the \"Mod manager download\" links of Nexus Mods. Use Heronite to install them here.")
            } else {
                tr!("Let \"Mod manager download\" on Nexus Mods open Heronite, and the mod lands in this list.")
            };
            return Some(
                tinted(Tone::Info, Icon::Link)
                    .child(words(Some(tr!("Install from the Nexus Mods site")), text))
                    .child(button_sm("mods-nxm", ButtonKind::Glass, tr!("Use Heronite")).self_center().on_click(cx.listener(
                        |this, _, _, cx| {
                            this.service.update(cx, |s, cx| s.register_handlers(cx));
                        },
                    )))
                    .into_any_element(),
            );
        }
        None
    }

    /// Downloads and installs of this game: progress, cancel, dismiss.
    pub(super) fn jobs_card(&mut self, v: &View, cx: &mut Context<Self>) -> Option<Div> {
        if v.jobs.is_empty() {
            return None;
        }
        let mut col = div().flex().flex_col().gap(px(8.));
        for job in &v.jobs {
            let id = job.id;
            let failed = job.stage == JobStage::Failed;
            let active = format::is_active(job.stage);
            let (ic, color) = match job.stage {
                JobStage::Failed => (Icon::Warning, theme::danger_text()),
                JobStage::Done => (Icon::Check, theme::success()),
                JobStage::Cancelled => (Icon::Close, theme::text_dim()),
                _ => (Icon::Downloading, theme::accent()),
            };
            let bar = match (active, job.progress.fraction()) {
                (true, Some(f)) => Some(progress_bar(Some(f * 100.), 5., theme::accent()).into_any_element()),
                (true, None) if job.stage != JobStage::NeedsChoices => Some(busy_bar(format!("mods-job-bar-{id}"), 5.)),
                _ => None,
            };
            let action = if job.stage == JobStage::NeedsChoices {
                button_sm(format!("mods-job-choose-{id}"), ButtonKind::Glass, tr!("Choose options")).on_click(cx.listener(|_, _, _, cx| {
                    cx.emit(super::ModsDialogEvent::OpenInstaller);
                }))
            } else if active {
                button_sm(format!("mods-job-cancel-{id}"), ButtonKind::Ghost, tr!("Cancel")).on_click(cx.listener(move |this, _, _, cx| {
                    this.service.update(cx, |s, cx| s.cancel_job(id, cx));
                }))
            } else {
                button_sm(format!("mods-job-dismiss-{id}"), ButtonKind::Ghost, tr!("Dismiss")).on_click(cx.listener(
                    move |this, _, _, cx| {
                        this.service.update(cx, |s, cx| s.dismiss_job(id, cx));
                    },
                ))
            };
            col = col.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .px(px(14.))
                    .py(px(11.))
                    .rounded(px(14.))
                    .bg(if failed { theme::danger_fill() } else { theme::card_fill() })
                    .border_1()
                    .border_color(if failed { Hsla { a: 0.3, ..theme::danger().into() } } else { theme::card_border() })
                    .child(icon(ic, px(20.), color))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .flex_1()
                            .min_w_0()
                            .gap(px(3.))
                            .child(div().text_size(px(14.)).font_weight(theme::W_500).truncate().child(job.name.clone()))
                            .child(
                                div()
                                    .text_size(px(12.5))
                                    .line_height(px(17.))
                                    .text_color(if failed { theme::danger_text() } else { theme::text_muted() })
                                    .child(format::stage_text(job)),
                            )
                            .children(bar.map(|b| div().mt(px(3.)).child(b))),
                    )
                    .child(action),
            );
        }
        Some(col)
    }
}
