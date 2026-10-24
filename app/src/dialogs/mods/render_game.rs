//! Game tab: where the game is on the mod sites, where its mods go, where
//! they are stored, and removing them all.

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_i18n::tr;

use super::{format, Confirm, ModsDialog, View};
use crate::theme;
use crate::ui::controls::{button, button_sm, caption, ButtonKind};
use crate::ui::fields::{input_box, labeled, panel};

impl ModsDialog {
    pub(super) fn render_game(&mut self, v: &View, _window: &mut Window, cx: &mut Context<Self>) -> Div {
        let note = |text: SharedString| div().text_size(px(12.5)).line_height(px(17.)).text_color(theme::text_dim()).child(text);
        let found = |name: &Option<String>, linked: bool| match (name, linked) {
            (Some(n), _) => note(tr!("Found: {name}", name = n.clone()).into()),
            (None, true) => note(tr!("Looking it up\u{2026}").into()),
            (None, false) => note(tr!("Not linked: Browse and updates need this").into()),
        };
        let nexus_linked = !self.links.nexus.read(cx).value().trim().is_empty();
        let store_linked = !self.links.store.read(cx).value().trim().is_empty();

        let sites = panel()
            .gap(px(14.))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(5.))
                    .child(labeled(tr!("Nexus Mods game"), input_box(&self.links.nexus, false)))
                    .child(found(&self.links.nexus_name, nexus_linked)),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(5.))
                    .child(labeled(tr!("Thunderstore community"), input_box(&self.links.store, false)))
                    .child(found(&self.links.store_name, store_linked)),
            );

        let default_root =
            if v.g.info.profile.mod_root.is_empty() { tr!("the game folder").to_string() } else { v.g.info.profile.mod_root.clone() };
        let place = panel().gap(px(5.)).child(labeled(tr!("Mods go into"), input_box(&self.links.root, false))).child(note(
            tr!("A folder inside the game's folder. Empty uses the usual one for this game: {folder}.", folder = default_root).into(),
        ));

        let dir = self.store_dir(cx);
        let open_dir = dir.clone();
        let storage = panel().gap(px(10.)).child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .gap(px(12.))
                .child(
                    div().flex().flex_col().min_w_0().child(div().text_size(px(14.)).child(tr!("Mods folder"))).child(
                        div()
                            .mt(px(2.))
                            .truncate()
                            .text_size(px(12.5))
                            .text_color(theme::text_muted())
                            .child(dir.to_string_lossy().to_string()),
                    ),
                )
                .child(
                    button_sm("mods-game-open", ButtonKind::Glass, tr!("Open"))
                        .on_click(cx.listener(move |this, _, _, cx| this.open_dir(open_dir.clone(), cx))),
                ),
        );

        let n = v.g.list.mods.len();
        let danger =
            if self.confirm == Some(Confirm::ForgetGame) {
                let written = v.g.overwrite;
                let overwrite = (!written.is_empty()).then(|| {
                    div().text_size(px(13.)).line_height(px(18.)).text_color(theme::warning()).child(tr!(
                    "The overwrite folder is deleted too: {files}, {size}. It holds what the game wrote while modded (settings, saves).",
                    files = format::files(written.files),
                    size = format::size(written.bytes)
                ))
                });
                panel()
                    .child(div().text_size(px(13.5)).line_height(px(19.)).child(tr!(
                        "Remove all {n} mods of {game}? Their files are deleted; the game is not touched.",
                        n = n,
                        game = self.game.title.clone()
                    )))
                    .children(overwrite)
                    .child(
                        div()
                            .flex()
                            .gap(px(8.))
                            .child(button_sm("mods-forget-no", ButtonKind::Ghost, tr!("Cancel")).on_click(cx.listener(|this, _, _, cx| {
                                this.confirm = None;
                                cx.notify();
                            })))
                            .child(button_sm("mods-forget-yes", ButtonKind::Danger, tr!("Remove all mods")).on_click(cx.listener(
                                |this, _, _, cx| {
                                    this.forget_game(cx);
                                    this.tab = super::Tab::Installed;
                                },
                            ))),
                    )
            } else {
                panel()
                    .flex_row()
                    .items_center()
                    .justify_between()
                    .gap(px(12.))
                    .child(div().min_w_0().text_size(px(13.)).line_height(px(18.)).text_color(theme::text_muted()).child(tr!(
                        "Removes every mod of this game and everything Heronite keeps for it: downloads and the overwrite folder."
                    )))
                    .child(
                        div().flex_none().child(
                            button("mods-forget", ButtonKind::Danger, tr!("Remove all mods"))
                                .on_click(cx.listener(|this, _, _, cx| this.ask(Confirm::ForgetGame, cx))),
                        ),
                    )
            };

        div()
            .flex()
            .flex_col()
            .gap(px(4.))
            .child(caption(tr!("Where the game is on the mod sites")))
            .child(sites)
            .child(caption(tr!("Where mods go")))
            .child(place)
            .child(caption(tr!("Storage")))
            .child(storage)
            .child(caption(tr!("Start over")))
            .child(danger)
    }
}
