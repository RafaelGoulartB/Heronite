//! The update card in the bottom-right corner (`services::update`): a new
//! version, its download, the restart, or why the install failed. Settings →
//! About has the full card with the release notes.

use gpui_kit::prelude::*;
use gpui_kit::*;

use super::CollectionView;
use crate::icons::Icon;
use crate::pages::settings::Section;
use crate::services::update::{self, Banner, Phase, Updater};
use crate::theme;
use crate::ui::controls::{action_button, button_sm, close_button, kicker, ButtonKind};
use crate::ui::glass::glass_panel;
use crate::ui::icon;
use heronite_i18n::tr;

const WIDTH: f32 = 380.;

impl CollectionView {
    pub(super) fn render_update_banner(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if self.page == crate::pages::Page::Settings || self.dialog.is_some() {
            return None;
        }
        let updater = update::get(cx)?;
        let u = updater.read(cx);
        let kind = u.banner()?;
        let release = u.release()?.clone();
        let version = release.version.clone();
        let current = u.current().to_string();
        let can_install = u.can_install();
        let manual = u.manual_command();
        let installing = *u.phase() == Phase::Installing;
        let blocker = crate::system::restart_blocker(cx);

        let act = |f: fn(&mut Updater, &mut Context<Updater>)| {
            let updater = updater.clone();
            move |_: &ClickEvent, _: &mut Window, cx: &mut App| updater.update(cx, f)
        };
        let (label, title, detail, accent): (&str, String, String, Rgba) = match (kind, u.phase()) {
            (Banner::Downloading, Phase::Downloading(p)) => {
                (tr!("Updating"), tr!("Downloading Heronite {version}", version), update::progress_text(p), theme::accent())
            }
            (Banner::Downloading, _) => (
                tr!("Updating"),
                tr!("Installing Heronite {version}", version),
                tr!("Enter your password if you are asked.").into(),
                theme::accent(),
            ),
            (Banner::Ready, _) => (
                tr!("Update ready"),
                tr!("Heronite {version} is installed", version),
                match &blocker {
                    Some(why) => tr!("{why}: restart later, or it starts next time.", why),
                    None => tr!("Restart to use it now, or it starts next time.").into(),
                },
                theme::success(),
            ),
            (Banner::Failed, Phase::InstallFailed(e)) => {
                (tr!("Update failed"), tr!("Could not update to {version}", version), e.clone(), theme::danger())
            }
            _ => {
                let size = u.download_asset().filter(|a| a.size > 0).map(|a| format!(" \u{b7} {}", update::size_text(a.size)));
                (
                    tr!("Update available"),
                    format!("Heronite {version}"),
                    tr!("You have {current}{size}", current, size = size.unwrap_or_default()),
                    theme::accent(),
                )
            }
        };

        let progress = match u.phase() {
            Phase::Downloading(p) => Some(crate::services::widgets::progress_bar(p.percent(), 4., theme::accent())),
            _ => None,
        };
        let page = release.page_url.clone();
        let mut footer = div().flex().items_center().justify_end().gap(px(8.)).pt(px(4.));
        footer = match kind {
            Banner::Available => {
                footer = footer
                    .child(
                        button_sm("upd-skip", ButtonKind::Ghost, tr!("Skip this version")).on_click(act(|u, cx| u.set_skipped(true, cx))),
                    )
                    .child(
                        button_sm("upd-notes", ButtonKind::Glass, tr!("What's new"))
                            .on_click(cx.listener(|this, _, window, cx| this.open_settings_section(Section::About, window, cx))),
                    );
                if can_install {
                    footer.child(button_sm("upd-install", ButtonKind::Primary, tr!("Update")).on_click(act(Updater::install)))
                } else {
                    footer.child(
                        button_sm("upd-page", ButtonKind::Primary, tr!("Download")).on_click(move |_, _, _| update::open_page(&page)),
                    )
                }
            }
            // The package manager cannot be stopped halfway.
            Banner::Downloading if installing => footer,
            Banner::Downloading => footer.child(button_sm("upd-cancel", ButtonKind::Ghost, tr!("Cancel")).on_click(act(Updater::cancel))),
            Banner::Ready => footer.child(button_sm("upd-later", ButtonKind::Ghost, tr!("Later")).on_click(act(Updater::dismiss))).child(
                action_button("upd-restart", ButtonKind::Primary, tr!("Restart now"), blocker.is_none(), act(Updater::restart))
                    .h(px(theme::BUTTON_H_SM)),
            ),
            Banner::Failed => {
                let footer = match manual {
                    Some(cmd) => footer.child(
                        button_sm("upd-copy", ButtonKind::Ghost, tr!("Copy terminal command"))
                            .on_click(move |_, _, cx| cx.write_to_clipboard(ClipboardItem::new_string(cmd.clone()))),
                    ),
                    None => footer.child(
                        button_sm("upd-page", ButtonKind::Ghost, tr!("Release page")).on_click(move |_, _, _| update::open_page(&page)),
                    ),
                };
                footer.child(button_sm("upd-retry", ButtonKind::Primary, tr!("Try again")).on_click(act(Updater::install)))
            }
        };

        let close = (kind != Banner::Downloading).then(|| close_button("upd-close").size(px(26.)).on_click(act(Updater::dismiss)));
        let head = div()
            .flex()
            .items_center()
            .gap(px(10.))
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_center()
                    .size(px(30.))
                    .rounded(px(9.))
                    .bg(theme::white_a(0.06))
                    .child(icon(Icon::Update, px(17.), accent)),
            )
            .child(div().flex_1().min_w_0().child(kicker(label).text_color(accent)))
            .children(close);
        let body = div()
            .flex()
            .flex_col()
            .gap(px(3.))
            .child(
                div()
                    .font_family(theme::FONT_HEADING)
                    .text_size(px(17.))
                    .font_weight(theme::W_BOLD)
                    .text_color(theme::text_primary())
                    .child(title),
            )
            .child(div().text_size(px(13.)).line_height(px(18.)).text_color(theme::text_muted()).child(detail));
        let card = glass_panel(theme::R_POPOVER)
            .id("update-banner")
            .occlude()
            .w(px(WIDTH))
            .p(px(16.))
            .flex()
            .flex_col()
            .gap(px(12.))
            .text_color(theme::text_primary())
            .font_family(theme::FONT_BODY)
            .child(head)
            .child(body)
            .children(progress)
            .child(footer);
        Some(div().absolute().right(px(20.)).bottom(px(20.)).child(card).into_any_element())
    }
}
