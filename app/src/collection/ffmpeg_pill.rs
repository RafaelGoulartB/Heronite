//! The ffmpeg pill of the Collection's header, next to the Steam pill: with
//! Settings → Appearance → "Animated backgrounds" on, a background that
//! moves is shown and ffmpeg, which plays it, is missing (it then stays
//! still). A faint wash of the warning colour, as the Steam pill; hovering
//! it says the setting is why, and how to fix it: install ffmpeg (Windows:
//! downloaded here, `heronite_launch::ffmpeg_install`, a ring while it
//! comes; elsewhere from the system's packages, then "Check again") or turn
//! the setting off. It goes once ffmpeg is there or the setting is off.
//!
//! `HERONITE_DEBUG_NO_FFMPEG=1` acts as if ffmpeg were missing (`download`:
//! and could be downloaded, as on Windows) for captures.

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::ffmpeg::Tool;
use heronite_i18n::tr;

use super::downloads_pill::{pill_base, ring};
use super::header::Menu;
use super::steam_pill::soft_pill;
use super::CollectionView;
use crate::icons::Icon;
use crate::theme;
use crate::ui::controls::{button_sm, ButtonKind};
use crate::ui::dropdown::Align;
use crate::ui::{glass, icon};

/// Whether ffmpeg is downloaded here when it is missing (Windows).
fn can_download() -> bool {
    heronite_launch::ffmpeg_install::can_download() || crate::env::value("HERONITE_DEBUG_NO_FFMPEG").as_deref() == Some("download")
}

/// `HERONITE_DEBUG_NO_FFMPEG`: ffmpeg counts as missing.
pub(super) fn debug_no_ffmpeg() -> bool {
    crate::env::flag("HERONITE_DEBUG_NO_FFMPEG")
}

impl CollectionView {
    pub(super) fn ffmpeg_pill(&mut self, cx: &mut Context<Self>) -> Option<Stateful<Div>> {
        let getting = self.live.getting_ffmpeg;
        if !self.live.wants_ffmpeg && !getting {
            return None;
        }
        let open = self.open_menu == Some(Menu::Ffmpeg);
        let label = |text: &str, color: Hsla| {
            div().flex_none().text_size(px(13.)).font_weight(theme::W_BOLD).text_color(color).child(text.to_string())
        };
        let pill = if getting {
            pill_base("hdr-ffmpeg", open)
                .child(ring(100., theme::accent().into(), Some(Icon::Animation)))
                .child(label(tr!("Installing ffmpeg"), theme::text_primary().into()))
        } else {
            soft_pill("hdr-ffmpeg", open)
                .child(icon(Icon::Animation, px(16.), theme::warning()))
                .child(label(tr!("ffmpeg missing"), theme::readable(theme::warning())))
        };
        Some(self.hover_dropdown(Menu::Ffmpeg, pill, Align::Right, |this, cx| this.ffmpeg_popover(cx), cx))
    }

    /// Why, and the two ways out: install ffmpeg or turn the setting off.
    fn ffmpeg_popover(&self, cx: &mut Context<Self>) -> AnyElement {
        let title = |text: &str| {
            div()
                .font_family(theme::font_heading())
                .text_size(px(17.))
                .font_weight(theme::W_BOLD)
                .text_color(theme::text_primary())
                .child(text.to_string())
        };
        let detail = |text: &str| div().text_size(px(13.)).line_height(px(18.)).text_color(theme::text_muted()).child(text.to_string());
        let panel = glass::glass_panel(theme::R_POPOVER).w(px(360.)).p(px(16.)).flex().flex_col().gap(px(8.));
        if self.live.getting_ffmpeg {
            return panel
                .child(title(tr!("Installing ffmpeg")))
                .child(detail(tr!("Downloading about 55 MB. The background starts moving once it is done.")))
                .into_any_element();
        }
        let act = |f: fn(&mut CollectionView, &mut Context<CollectionView>)| {
            let view = cx.entity().downgrade();
            move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                view.update(cx, |this, cx| {
                    this.open_menu = None;
                    f(this, cx);
                    cx.notify();
                })
                .ok();
            }
        };
        let how = if can_download() {
            tr!("Install it here (about 55 MB), or turn the setting off.")
        } else {
            tr!("Install ffmpeg from your system's packages and check again, or turn the setting off.")
        };
        let fix = if can_download() {
            button_sm("ffmpeg-install", ButtonKind::Primary, tr!("Install ffmpeg")).on_click(act(Self::live_install_ffmpeg))
        } else {
            button_sm("ffmpeg-check", ButtonKind::Primary, tr!("Check again")).on_click(act(Self::live_check_ffmpeg))
        };
        let error =
            self.live.ffmpeg_error.clone().map(|e| div().text_size(px(12.)).line_height(px(16.)).text_color(theme::danger_text()).child(e));
        panel
            .child(title(tr!("Animated backgrounds need ffmpeg")))
            .child(detail(tr!(
                "This shows because Animated backgrounds is on (Settings → Appearance). ffmpeg plays them; without it, backgrounds stay still."
            )))
            .child(detail(how))
            .children(error)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .pt(px(6.))
                    .child(button_sm("ffmpeg-off", ButtonKind::Ghost, tr!("Turn off")).on_click(act(Self::live_turn_off)))
                    .child(div().flex_1())
                    .child(fix),
            )
            .into_any_element()
    }

    /// Downloads ffmpeg (Windows), then plays the background.
    fn live_install_ffmpeg(&mut self, cx: &mut Context<Self>) {
        if std::mem::replace(&mut self.live.getting_ffmpeg, true) {
            return;
        }
        self.live.ffmpeg_error = None;
        let paths = self.data_paths.clone();
        cx.spawn(async move |this, cx| {
            let got = cx
                .background_spawn(async move {
                    heronite_launch::ffmpeg_install::ensure(&paths, &[Tool::Ffmpeg], &heronite_launch::net::SystemNet, &|| false)?;
                    if heronite_media::stream::available() {
                        Ok(())
                    } else {
                        Err("ffmpeg was downloaded but is not found".to_string())
                    }
                })
                .await;
            this.update(cx, |this, cx| {
                this.live.getting_ffmpeg = false;
                match got {
                    Ok(()) => this.live.ffmpeg = Some(true),
                    Err(e) => {
                        log::warn!("ffmpeg download: {e}");
                        this.live.ffmpeg_error = Some(tr!("Could not install it: {error}", error = e));
                        // Said where it was asked for, without another hover.
                        this.reopen_ffmpeg_pill();
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Looks for ffmpeg again (installed from the system's packages since).
    fn live_check_ffmpeg(&mut self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            let found = cx
                .background_spawn(async move {
                    heronite_data::ffmpeg::forget();
                    heronite_media::stream::available()
                })
                .await;
            this.update(cx, |this, cx| {
                this.live.ffmpeg = Some(found);
                this.live.ffmpeg_error = (!found).then(|| tr!("ffmpeg is still not found.").to_string());
                // The answer, where it was asked for.
                if !found {
                    this.reopen_ffmpeg_pill();
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Opens the pill's popover again with its answer, while the pill shows
    /// (else the answer waits for the next hover: a menu with nothing on
    /// screen would hold the list's keys).
    fn reopen_ffmpeg_pill(&mut self) {
        let shown = self.live.wants_ffmpeg
            && self.page == crate::pages::Page::Collection
            && !crate::console::active()
            && self.dialog.is_none()
            && self.open_menu.is_none();
        if shown {
            self.open_menu = Some(Menu::Ffmpeg);
        }
    }

    /// Turns "Animated backgrounds" off: nothing needs ffmpeg any more.
    fn live_turn_off(&mut self, cx: &mut Context<Self>) {
        self.live.ffmpeg_error = None;
        crate::dialogs::animated_art::set(&self.app_paths, false, cx);
    }
}
