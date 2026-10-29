//! "What's new in Heronite X": opens once, on the first start of a version
//! this app installed (`services::update`), with the release's notes and an
//! invitation to star the project and give feedback in its Discussions.

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_i18n::tr;
use heronite_update::{Release, REPO_PAGE};

use super::DialogEvent;
use crate::icons::Icon;
use crate::services::update;
use crate::theme;
use crate::ui::controls::{button, icon_action_button, ButtonKind};
use crate::ui::dialog::{dialog_scrolled, DialogProps};
use crate::ui::icon;

pub struct WhatsNewDialog {
    release: Release,
    scroll: ScrollHandle,
}

impl EventEmitter<DialogEvent> for WhatsNewDialog {}

impl WhatsNewDialog {
    pub fn new(release: Release) -> Self {
        Self { release, scroll: ScrollHandle::new() }
    }
}

/// About the height of the body for `notes` (lines of the dialog's width),
/// so a short release does not leave a blank dialog.
fn body_height(notes: &str) -> f32 {
    use crate::services::update::notes::{parse, Note};
    let lines: f32 = parse(notes)
        .iter()
        .map(|n| match n {
            Note::Heading(_) => 28.,
            Note::Bullet(t) => 22. * (t.chars().count() as f32 / 80.).ceil().max(1.),
            Note::Text(_) => 0.,
        })
        .sum();
    // The "Updated" line, the list's padding and the invitation.
    30. + 12. + lines + 120.
}

impl Render for WhatsNewDialog {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // `2026-10-11T12:00:00Z` → `2026-10-11`.
        let date = self.release.published_at.get(..10).unwrap_or_default().to_string();
        let updated = div()
            .flex()
            .items_center()
            .gap(px(6.))
            .text_size(px(13.))
            .child(icon(Icon::Check, px(15.), theme::success()))
            .child(div().font_weight(theme::W_BOLD).text_color(theme::readable(theme::success())).child(tr!("Updated")))
            .when(!date.is_empty(), |d| d.child(div().text_color(theme::text_dim()).child(tr!("released {date}", date))));
        let invite = div()
            .flex()
            .items_center()
            .gap(px(14.))
            .mt(px(6.))
            .p(px(14.))
            .rounded(px(theme::R_CARD))
            .bg(theme::card_fill())
            .border_1()
            .border_color(theme::card_border())
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_center()
                    .size(px(34.))
                    .rounded(px(10.))
                    .bg(theme::accent_alpha(0.16))
                    .child(icon(Icon::Star, px(17.), theme::accent())),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .gap(px(2.))
                    .child(
                        div()
                            .text_size(px(14.))
                            .font_weight(theme::W_BOLD)
                            .text_color(theme::text_primary())
                            .child(tr!("Enjoying Heronite?")),
                    )
                    .child(
                        div()
                            .text_size(px(13.))
                            .line_height(px(18.))
                            .text_color(theme::text_muted())
                            .child(tr!("A star on GitHub helps more people find it. Ideas and feedback go in Discussions.")),
                    ),
            );
        let body = div()
            .flex()
            .flex_col()
            .child(updated)
            // The closing line of the notes asks the same as `invite`.
            .child(crate::ui::release_notes::list(&self.release.notes, false))
            .child(invite);

        let close = cx.listener(|_, _, _, cx| cx.emit(DialogEvent::Close));
        let footer = div()
            .flex()
            .items_center()
            .gap(px(10.))
            .child(icon_action_button("whatsnew-discuss", ButtonKind::Glass, Icon::Notes, tr!("Discussions"), |_, _, _| {
                update::open_page(&format!("{REPO_PAGE}/discussions"))
            }))
            .child(icon_action_button("whatsnew-star", ButtonKind::Glass, Icon::Star, tr!("Star on GitHub"), |_, _, _| {
                update::open_page(REPO_PAGE)
            }))
            .child(button("whatsnew-ok", ButtonKind::Primary, tr!("Got it")).on_click(close));

        // The scroll's size is known after a first layout.
        if self.scroll.bounds().size.height <= px(0.) {
            let view = cx.entity_id();
            cx.defer(move |cx| cx.notify(view));
        }
        let entity = cx.entity().downgrade();
        let height = body_height(&self.release.notes).min(f32::from(window.viewport_size().height) * 0.78 - 140.);
        dialog_scrolled(
            DialogProps {
                id: "whats-new".into(),
                title: tr!("What's new in Heronite {version}", version = self.release.version).into(),
                width: 600f32.min(f32::from(window.viewport_size().width) * 0.92),
                height: Some(height),
            },
            &self.scroll,
            body.into_any_element(),
            Some(footer.into_any_element()),
            move |_, cx| {
                entity.update(cx, |_, cx| cx.emit(DialogEvent::Close)).ok();
            },
        )
    }
}
