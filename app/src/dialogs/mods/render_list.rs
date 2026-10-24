//! Installed tab: notices, jobs, the mods in load order, the overwrite row
//! and the footer.

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_i18n::{tr, trn};

use super::browse::Source;
use super::{format, Confirm, ModsDialog, Tab, View};
use crate::icons::Icon;
use crate::theme;
use crate::ui::controls::{button, button_sm, caption, card_box, ButtonKind};
use crate::ui::fields;
use crate::ui::icon;

impl ModsDialog {
    pub(super) fn render_installed(&mut self, v: &View, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let mut col = div().flex().flex_col().gap(px(12.));
        for n in self.notices(v, cx) {
            col = col.child(n);
        }
        col = col.children(self.jobs_card(v, cx));
        if v.g.list.mods.is_empty() {
            return if v.jobs.is_empty() { col.child(self.empty_state(v, cx)) } else { col.child(self.overwrite_card(v, window, cx)) };
        }
        let hint = tr!("A mod lower in the list wins a file when two have it");
        col = col.child(
            div()
                .flex()
                .items_baseline()
                .justify_between()
                .gap(px(12.))
                .child(caption(tr!("Load order")).m_0().mt(px(4.)))
                .child(div().min_w_0().truncate().text_size(px(12.)).text_color(theme::text_dim()).child(hint)),
        );
        col = col.child(self.mod_list(v, window, cx));
        col.child(self.overwrite_card(v, window, cx))
    }

    fn mod_list(&mut self, v: &View, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let needed = super::render_row::needed_by(&v.g);
        let mut card = card_box().px(px(6.)).py(px(4.));
        let n = v.g.list.mods.len();
        for (i, m) in v.g.list.mods.iter().enumerate() {
            if i > 0 {
                card = card.child(div().h(px(1.)).mx(px(8.)).bg(theme::row_separator()));
            }
            let by = needed.get(&m.id).map(Vec::as_slice).unwrap_or(&[]);
            card = card.child(self.mod_row(v, i, n, m, by, window, cx));
        }
        card
    }

    fn empty_state(&mut self, v: &View, cx: &mut Context<Self>) -> Div {
        let mut sites = Vec::new();
        if v.g.info.nexus_domain.is_some() {
            sites.push(("mods-empty-nexus", tr!("Browse Nexus Mods"), Source::Nexus));
        }
        if v.g.info.thunderstore.is_some() {
            sites.push(("mods-empty-ts", tr!("Browse Thunderstore"), Source::Thunderstore));
        }
        let file_kind = if sites.is_empty() { ButtonKind::Primary } else { ButtonKind::Glass };
        let mut actions = div().flex().flex_wrap().justify_center().gap(px(10.)).mt(px(6.));
        for (i, (id, label, source)) in sites.into_iter().enumerate() {
            let kind = if i == 0 { ButtonKind::Primary } else { ButtonKind::Glass };
            actions = actions.child(button(id, kind, label).on_click(cx.listener(move |this, _, _, cx| {
                this.browse.source = source;
                this.show_tab(Tab::Browse, cx);
            })));
        }
        actions = actions.child(
            button("mods-empty-file", file_kind, tr!("Add from file\u{2026}"))
                .on_click(cx.listener(|this, _, window, cx| this.choose_mod(false, window, cx))),
        );
        div()
            .flex()
            .flex_col()
            .items_center()
            .gap(px(8.))
            .px(px(28.))
            .py(px(36.))
            .rounded(px(theme::R_CARD))
            .bg(theme::card_fill())
            .border_1()
            .border_color(theme::card_border())
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_center()
                    .size(px(56.))
                    .rounded_full()
                    .bg(theme::accent_tint())
                    .child(icon(Icon::Extension, px(28.), theme::accent())),
            )
            .child(div().mt(px(6.)).text_size(px(18.)).font_family(theme::font_heading()).font_weight(theme::W_BOLD).child(tr!("No mods yet")))
            .child(
                div().max_w(px(460.)).text_center().text_size(px(13.5)).line_height(px(19.)).text_color(theme::text_muted()).child(tr!(
                    "Mods are added here and applied only when you play from Heronite: the game's own files are never changed. Find some online, or drop an archive on this window."
                )),
            )
            .child(actions)
    }

    /// The last row: what the game wrote while modded.
    fn overwrite_card(&mut self, v: &View, _window: &mut Window, cx: &mut Context<Self>) -> Div {
        let written = v.g.overwrite;
        if written.is_empty() {
            return div();
        }
        let (files, deleted) = (written.files, written.deleted);
        let info = if files > 0 {
            format!("{} \u{b7} {} \u{b7} {}", tr!("Files the game wrote"), format::files(files), format::size(written.bytes))
        } else {
            tr!("The game wrote no files").to_string()
        };
        let removed = (deleted > 0).then(|| trn!(deleted, "{n} file deleted by the game", "{n} files deleted by the game"));
        let key_dir = self.store_dir(cx).join("overwrite");
        let mut card = card_box().px(px(14.)).py(px(12.)).gap(px(10.));
        card = card.child(
            div()
                .flex()
                .items_center()
                .gap(px(12.))
                .child(
                    div()
                        .flex()
                        .flex_none()
                        .items_center()
                        .justify_center()
                        .size(px(40.))
                        .rounded(px(10.))
                        .bg(theme::white_a(0.07))
                        .child(icon(Icon::Inbox, px(20.), theme::text_muted())),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .min_w_0()
                        .child(div().text_size(px(14.)).font_weight(theme::W_500).child(tr!("Overwrite")))
                        .child(div().text_size(px(12.5)).text_color(theme::text_muted()).child(info))
                        .children(removed.map(|r| div().text_size(px(12.5)).text_color(theme::text_dim()).child(r))),
                ),
        );
        if let Some(input) = self.naming_overwrite.clone() {
            let make = input.clone();
            card = card.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(div().flex_1().child(fields::input_box(&input, false)))
                    .child(button_sm("mods-ow-create", ButtonKind::Primary, tr!("Create mod")).on_click(cx.listener(
                        move |this, _, _, cx| {
                            this.overwrite_into_mod(&make, cx);
                        },
                    )))
                    .child(button_sm("mods-ow-cancel", ButtonKind::Ghost, tr!("Cancel")).on_click(cx.listener(|this, _, _, cx| {
                        this.naming_overwrite = None;
                        cx.notify();
                    }))),
            );
        } else if self.confirm == Some(Confirm::ClearOverwrite) {
            card = card.child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap(px(10.))
                    .child(div().flex_1().text_size(px(13.)).text_color(theme::text_primary()).child(if files > 0 {
                        tr!("Delete these {n} files? The game itself is not touched.", n = files)
                    } else {
                        tr!("Forget what the game deleted? The game itself is not touched.").to_string()
                    }))
                    .child(button_sm("mods-ow-no", ButtonKind::Ghost, tr!("Cancel")).on_click(cx.listener(|this, _, _, cx| {
                        this.confirm = None;
                        cx.notify();
                    })))
                    .child(
                        button_sm("mods-ow-yes", ButtonKind::Danger, tr!("Empty"))
                            .on_click(cx.listener(|this, _, _, cx| this.clear_overwrite(cx))),
                    ),
            );
        } else {
            card = card.child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap(px(8.))
                    .child(button_sm("mods-ow-open", ButtonKind::Glass, tr!("Open")).on_click(cx.listener(move |this, _, _, cx| {
                        this.open_dir(key_dir.clone(), cx);
                    })))
                    .children((files > 0).then(|| {
                        button_sm("mods-ow-mod", ButtonKind::Glass, tr!("Turn into a mod"))
                            .on_click(cx.listener(|this, _, window, cx| this.start_naming_overwrite(window, cx)))
                    }))
                    .child(
                        button_sm("mods-ow-empty", ButtonKind::Ghost, tr!("Empty"))
                            .on_click(cx.listener(|this, _, _, cx| this.ask(Confirm::ClearOverwrite, cx))),
                    ),
            );
        }
        card
    }

    pub(super) fn render_footer(&mut self, v: Option<&View>, cx: &mut Context<Self>) -> Div {
        let loaded = v.is_some();
        let mods_dir = self.store_dir(cx).join("mods");
        div()
            .flex()
            .flex_wrap()
            .items_center()
            .justify_between()
            .gap(px(8.))
            .w_full()
            .child(
                div()
                    .flex()
                    .gap(px(8.))
                    .child(
                        button_sm("mods-add-file", ButtonKind::Primary, tr!("Add from file\u{2026}"))
                            .on_click(cx.listener(|this, _, window, cx| this.choose_mod(false, window, cx))),
                    )
                    .child(
                        button_sm("mods-add-folder", ButtonKind::Glass, tr!("Add a folder\u{2026}"))
                            .on_click(cx.listener(|this, _, window, cx| this.choose_mod(true, window, cx))),
                    ),
            )
            .child(
                div()
                    .flex()
                    .gap(px(4.))
                    .child(crate::ui::controls::icon_button_sm(
                        "mods-check-updates",
                        ButtonKind::Ghost,
                        Icon::Refresh,
                        tr!("Check for updates"),
                        loaded,
                        cx.listener(|this, _, _, cx| this.check_updates(cx)),
                    ))
                    .child(crate::ui::controls::icon_button_sm(
                        "mods-open-folder",
                        ButtonKind::Ghost,
                        Icon::FolderOpen,
                        tr!("Open mods folder"),
                        true,
                        cx.listener(move |this, _, _, cx| this.open_dir(mods_dir.clone(), cx)),
                    )),
            )
    }
}
