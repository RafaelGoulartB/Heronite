//! The question a drive gets once (Windows): keep the mods of its games on
//! it, so a launch links them, or keep them in Heronite's folder and copy
//! them at each launch (`heronite_mods::game_drive`).

use std::path::Path;

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_i18n::tr;
use heronite_mods::game_drive::{Choice, Placement};

use super::render_jobs::{tinted, Tone};
use super::{ModsDialog, View};
use crate::icons::Icon;
use crate::theme;
use crate::ui::controls::{button_sm, ButtonKind};

/// `D:` for `D:\`; a mounted folder as it is.
pub(crate) fn drive_name(volume: &Path) -> String {
    let s = volume.display().to_string();
    let trimmed = s.trim_end_matches(['\\', '/']);
    if trimmed.is_empty() {
        s
    } else {
        trimmed.to_string()
    }
}

impl ModsDialog {
    pub(super) fn drive_notice(&self, v: &View, cx: &mut Context<Self>) -> Option<AnyElement> {
        let Placement::Ask { volume, store } = &v.g.info.placement else { return None };
        let drive = drive_name(volume);
        let title = tr!("Keep this game's mods on {drive}?", drive = drive.clone());
        let body = tr!(
            "The game is on {drive} and its mods are not, so they are copied into the game's folder at every launch. Kept in {folder}, they are linked instead and the game starts at once. The answer counts for every game on {drive}; Settings → Mods can change it.",
            drive = drive.clone(),
            folder = store.display()
        );
        let text = div()
            .flex()
            .flex_col()
            .flex_1()
            .min_w_0()
            .gap(px(2.))
            .child(div().text_size(px(14.)).font_weight(theme::W_500).text_color(theme::text_primary()).child(title))
            .child(div().text_size(px(13.)).line_height(px(18.)).text_color(theme::text_muted()).child(body));
        let answer = |choice: Choice| {
            let volume = volume.clone();
            cx.listener(move |this, _, _, cx| {
                let volume = volume.clone();
                this.service.update(cx, |s, cx| s.answer_drive(volume, choice, cx));
            })
        };
        let buttons = div()
            .flex()
            .flex_col()
            .flex_none()
            .gap(px(6.))
            .self_center()
            .child(
                button_sm("mods-drive-store", ButtonKind::Primary, tr!("Keep them on {drive}", drive = drive))
                    .on_click(answer(Choice::Store)),
            )
            .child(button_sm("mods-drive-copy", ButtonKind::Ghost, tr!("Copy at each launch")).on_click(answer(Choice::Copy)));
        Some(tinted(Tone::Info, Icon::FaHardDrive).child(text).child(buttons).into_any_element())
    }
}
