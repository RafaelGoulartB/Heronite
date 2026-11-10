//! The wizard's faces: the step with its groups, the option's picture and
//! description, and the buttons.

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_i18n::tr;
use heronite_images::ImageRef;
use heronite_mods::fomod::{GroupKind, GroupView, PluginType, PluginView};

use super::ModInstallerDialog;
use crate::dialogs::art_images::picture;
use crate::icons::Icon;
use crate::theme;
use crate::ui::controls::{action_button, button, toggle, ButtonKind};
use crate::ui::dialog::{dialog, DialogProps};
use crate::ui::icon;

/// Height of the step's area; the options scroll inside it.
const BODY_H: f32 = 470.;
/// Width of the picture and description column.
const SIDE_W: f32 = 300.;

impl Render for ModInstallerDialog {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.images.update(cx, |im, _| im.frame(window));
        let entity = cx.entity().downgrade();

        let mut subtitle = Vec::new();
        if !self.version.is_empty() {
            subtitle.push(tr!("Version {version}", version = self.version.clone()));
        }
        if !self.author.is_empty() {
            subtitle.push(tr!("by {author}", author = self.author.clone()));
        }
        let (pos, total) = self.index;
        let head = div()
            .flex()
            .items_end()
            .justify_between()
            .gap(px(16.))
            .pb(px(12.))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .min_w_0()
                    .child(
                        div()
                            .truncate()
                            .font_family(theme::font_heading())
                            .text_size(px(18.))
                            .font_weight(theme::W_BOLD)
                            .child(self.step.name.clone()),
                    )
                    .child(div().truncate().text_size(px(12.5)).text_color(theme::text_dim()).child(subtitle.join(" \u{b7} "))),
            )
            .child(
                div()
                    .flex_none()
                    .px(px(10.))
                    .py(px(3.))
                    .rounded_full()
                    .bg(theme::white_a(0.08))
                    .text_size(px(12.5))
                    .text_color(theme::text_muted())
                    .child(tr!("Step {pos} of {total}", pos, total)),
            );

        let mut options = div().id("mod-installer-options").flex().flex_col().gap(px(14.)).flex_1().min_w_0().h_full().overflow_y_scroll();
        options = options.track_scroll(&self.scroll);
        if !self.missing.is_empty() {
            options = options.child(self.warning(cx));
        }
        for (gi, group) in self.step.groups.iter().enumerate() {
            options = options.child(self.group(gi, group, cx));
        }
        let body =
            div().flex().flex_col().child(head).child(div().flex().gap(px(20.)).h(px(BODY_H - 56.)).child(options).child(self.side(cx)));

        let last = !self.has_next;
        let footer = div()
            .flex()
            .items_center()
            .gap(px(10.))
            .child(
                button("mod-installer-cancel", ButtonKind::Ghost, tr!("Cancel"))
                    .on_click(cx.listener(|this, _, _, cx| this.cancel(false, cx))),
            )
            .child(
                button("mod-installer-plain", ButtonKind::Ghost, tr!("Install without the installer"))
                    .on_click(cx.listener(|this, _, _, cx| this.cancel(true, cx))),
            )
            .child(div().w(px(10.)))
            .when(self.has_back, |d| {
                d.child(button("mod-installer-back", ButtonKind::Glass, tr!("Back")).on_click(cx.listener(|this, _, _, cx| this.back(cx))))
            })
            .child(action_button(
                "mod-installer-next",
                ButtonKind::Primary,
                if last { tr!("Install") } else { tr!("Next") },
                self.can_next,
                cx.listener(|this, _, _, cx| this.next(cx)),
            ));

        dialog(
            DialogProps { id: "mod-installer".into(), title: self.name.clone().into(), width: 960., height: Some(BODY_H) },
            body.into_any_element(),
            Some(footer.into_any_element()),
            move |_, cx| {
                entity.update(cx, |this, cx| this.cancel(false, cx)).ok();
            },
        )
    }
}

impl ModInstallerDialog {
    fn warning(&self, _cx: &mut Context<Self>) -> Div {
        div()
            .flex()
            .gap(px(12.))
            .px(px(14.))
            .py(px(12.))
            .rounded(px(14.))
            .bg(Hsla { a: 0.11, ..theme::warning().into() })
            .border_1()
            .border_color(Hsla { a: 0.28, ..theme::warning().into() })
            .child(icon(Icon::Warning, px(18.), theme::warning()))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .child(
                        div().text_size(px(14.)).font_weight(theme::W_500).child(tr!("This installer expects something that is missing")),
                    )
                    .children(self.missing.iter().map(|m| div().text_size(px(13.)).text_color(theme::text_muted()).child(m.clone()))),
            )
    }

    /// A group: its caption, how many can be picked and the options.
    fn group(&self, gi: usize, group: &GroupView, cx: &mut Context<Self>) -> Div {
        let radio = matches!(group.kind, GroupKind::SelectExactlyOne | GroupKind::SelectAtMostOne);
        let hint = match group.kind {
            GroupKind::SelectExactlyOne => tr!("Pick one"),
            GroupKind::SelectAtMostOne => tr!("Pick one or none"),
            GroupKind::SelectAtLeastOne => tr!("Pick at least one"),
            GroupKind::SelectAny => tr!("Pick any"),
            GroupKind::SelectAll => tr!("All of these are installed"),
        };
        div()
            .flex()
            .flex_col()
            .gap(px(6.))
            .child(
                div()
                    .flex()
                    .items_baseline()
                    .justify_between()
                    .gap(px(10.))
                    .mx(px(4.))
                    .child(div().text_size(px(13.5)).font_weight(theme::W_500).text_color(theme::text_primary()).child(group.name.clone()))
                    .child(div().text_size(px(12.)).text_color(theme::text_dim()).child(hint)),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .rounded(px(theme::R_CARD))
                    .bg(theme::card_fill())
                    .border_1()
                    .border_color(theme::card_border())
                    .p(px(4.))
                    .children(group.plugins.iter().enumerate().map(|(pi, p)| self.option(gi, pi, p, radio, cx))),
            )
    }

    fn option(&self, gi: usize, pi: usize, p: &PluginView, radio: bool, cx: &mut Context<Self>) -> Stateful<Div> {
        let focused = self.focus == Some((gi, pi));
        let on = p.selected;
        let lock = !p.enabled;
        let mark = if radio {
            let ring = if on { theme::accent() } else { rgba(0xffffff59) };
            div()
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .size(px(20.))
                .rounded_full()
                .border_2()
                .border_color(ring)
                .when(on, |d| d.child(div().size(px(10.)).rounded_full().bg(theme::accent())))
                .into_any_element()
        } else {
            toggle(SharedString::from(format!("mod-installer-sw-{gi}-{pi}")), on, lock).into_any_element()
        };
        let pill = |text: &'static str, color: Rgba| {
            div()
                .flex_none()
                .px(px(8.))
                .py(px(1.))
                .rounded_full()
                .bg(Hsla { a: 0.16, ..color.into() })
                .text_size(px(11.5))
                .text_color(color)
                .child(text)
        };
        let state = match p.state {
            PluginType::Required => Some(pill(tr!("Required"), theme::text_muted())),
            PluginType::Recommended => Some(pill(tr!("Recommended"), theme::accent())),
            PluginType::NotUsable => Some(pill(tr!("Not usable"), theme::warning())),
            PluginType::CouldBeUsable => Some(pill(tr!("May not work"), theme::warning())),
            PluginType::Optional => None,
        };
        div()
            .id(SharedString::from(format!("mod-installer-opt-{gi}-{pi}")))
            .flex()
            .items_center()
            .gap(px(12.))
            .px(px(10.))
            .py(px(9.))
            .rounded(px(12.))
            .when(focused, |d| d.bg(theme::white_a(0.06)))
            .when(!lock, |d| d.cursor_pointer().hover(|s| s.bg(theme::white_a(0.06))))
            .when(lock, |d| d.opacity(0.55))
            .on_hover(cx.listener(move |this, h: &bool, _, cx| {
                if *h && this.focus != Some((gi, pi)) {
                    this.focus = Some((gi, pi));
                    cx.notify();
                }
            }))
            .when(!lock, |d| {
                d.on_click(cx.listener(move |this, _, _, cx| {
                    // A radio keeps its pick when clicked again; a switch flips.
                    let to = if radio { true } else { !on };
                    this.select(gi, pi, to, cx);
                }))
            })
            .child(mark)
            .child(div().flex_1().min_w_0().text_size(px(14.)).child(p.name.clone()))
            .children(state)
    }

    /// The hovered option's picture and description.
    fn side(&mut self, cx: &mut Context<Self>) -> Div {
        let plugin = self.focus.and_then(|(g, p)| self.step.groups.get(g)?.plugins.get(p)).cloned();
        let mut col = div().flex().flex_col().flex_none().gap(px(12.)).w(px(SIDE_W)).h_full();
        let Some(p) = plugin else { return col };
        if let Some(path) = p.image.clone() {
            if let Some(&(w, h)) = self.sizes.get(&path) {
                // Fitted into the column, never enlarged past its own size, so nothing is cropped.
                let scale = (SIDE_W / w as f32).min(210. / h as f32).min(1.);
                let (dw, dh) = ((w as f32 * scale).round().max(1.), (h as f32 * scale).round().max(1.));
                let src = self.images.update(cx, |im, _| im.get(&format!("fomod:{}", path.display()), vec![ImageRef::File(path)], dw, dh));
                col = col.child(div().flex().justify_center().child(picture(src).w(px(dw)).h(px(dh)).rounded(px(12.)).flex_none()));
            }
        }
        col.child(div().text_size(px(15.)).font_weight(theme::W_BOLD).child(p.name.clone())).child(
            div()
                .id("mod-installer-desc")
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .text_size(px(13.))
                .line_height(px(19.))
                .text_color(if p.description.trim().is_empty() { theme::text_dim() } else { theme::text_muted() })
                .child(if p.description.trim().is_empty() { tr!("No description.").to_string() } else { p.description.clone() }),
        )
    }
}
