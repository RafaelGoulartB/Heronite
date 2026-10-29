//! The update pill of the Collection's header (`services::update`), next to
//! the downloads pill. A new version tints it with the accent so it is seen;
//! then a progress ring while it downloads, green once it is installed (a
//! restart is due), red when the install failed. Hovering it shows the
//! highlights of the release and its actions. Settings → About has the full
//! card with every note. The first start of the installed version opens
//! "What's new" (`dialogs::whats_new`).

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_i18n::tr;

use super::downloads_pill::{pill_base, ring};
use super::header::Menu;
use super::CollectionView;
use crate::icons::Icon;
use crate::pages::settings::Section;
use crate::services::update::{self, notes, Banner, Phase, Updater};
use crate::services::widgets::progress_bar;
use crate::theme;
use crate::ui::controls::{action_button, button_sm, ButtonKind};
use crate::ui::dropdown::Align;
use crate::ui::{glass, icon};

/// Notes the popover lists before "All notes".
const HIGHLIGHTS: usize = 4;

/// What the pill and its popover show, read before they are built.
struct UpdateView {
    kind: Banner,
    phase: Phase,
    version: String,
    current: String,
    size: Option<String>,
    highlights: Vec<String>,
    more: usize,
    can_install: bool,
    manual: Option<String>,
    page: String,
    blocker: Option<String>,
}

/// The first bullets of the notes (the highlights come first) and how many
/// are left.
fn highlights(markdown: &str) -> (Vec<String>, usize) {
    let bullets: Vec<String> = notes::parse(markdown)
        .into_iter()
        .filter_map(|n| match n {
            notes::Note::Bullet(t) => Some(t),
            _ => None,
        })
        .collect();
    let more = bullets.len().saturating_sub(HIGHLIGHTS);
    (bullets.into_iter().take(HIGHLIGHTS).collect(), more)
}

impl CollectionView {
    /// The first start of a version this app installed: its "What's new",
    /// once no other dialog is open (and outside console mode).
    pub(super) fn open_whats_new(&mut self, cx: &mut Context<Self>) {
        if self.dialog.is_some() || crate::console::active() {
            return;
        }
        let Some(updater) = update::get(cx).filter(|u| u.read(cx).has_whats_new()) else { return };
        let Some(release) = updater.update(cx, |u, _| u.take_whats_new()) else { return };
        let view = cx.new(|_| crate::dialogs::whats_new::WhatsNewDialog::new(release));
        self.open_dialog(view, cx);
    }

    pub(super) fn update_pill(&mut self, cx: &mut Context<Self>) -> Option<Stateful<Div>> {
        let updater = update::get(cx)?;
        let u = updater.read(cx);
        let kind = u.banner()?;
        let release = u.release()?;
        let (highlights, more) = highlights(&release.notes);
        let v = UpdateView {
            kind,
            phase: u.phase().clone(),
            version: release.version.clone(),
            current: u.current().to_string(),
            size: u.download_asset().filter(|a| a.size > 0).map(|a| update::size_text(a.size)),
            highlights,
            more,
            can_install: u.can_install(),
            manual: u.manual_command(),
            page: release.page_url.clone(),
            blocker: crate::system::restart_blocker(cx),
        };
        let open = self.open_menu == Some(Menu::Update);
        let label =
            |text: String, color: Hsla| div().flex_none().text_size(px(13.)).font_weight(theme::W_BOLD).text_color(color).child(text);
        let pill = match (kind, &v.phase) {
            // Tinted with the accent: it asks for something, without
            // shouting over the header.
            (Banner::Available, _) => tinted_pill(theme::accent(), open)
                .child(icon(Icon::Update, px(16.), theme::accent()))
                .child(label(tr!("Update to {version}", version = v.version), theme::readable(theme::accent()))),
            (Banner::Ready, _) => tinted_pill(theme::success(), open)
                .child(icon(Icon::Refresh, px(16.), theme::success()))
                .child(label(tr!("Restart to update").into(), theme::readable(theme::success()))),
            (Banner::Downloading, Phase::Downloading(p)) => {
                let percent = p.percent();
                pill_base("hdr-update", open)
                    .child(ring(percent.unwrap_or(0.), theme::accent().into(), None))
                    .child(label(tr!("Updating").into(), theme::text_primary().into()))
                    .children(
                        percent
                            .map(|p| div().flex_none().text_size(px(12.)).text_color(theme::text_dim()).child(format!("{}%", p.round()))),
                    )
            }
            (Banner::Downloading, _) => pill_base("hdr-update", open)
                .child(ring(100., theme::accent().into(), Some(Icon::Update)))
                .child(label(tr!("Installing update").into(), theme::text_primary().into())),
            (Banner::Failed, _) => pill_base("hdr-update", open)
                .border_color(Hsla { a: 0.5, ..theme::danger().into() })
                .child(ring(100., theme::danger_text().into(), Some(Icon::Warning)))
                .child(label(tr!("Update failed").into(), theme::readable(theme::danger_text()))),
        };
        Some(self.hover_dropdown(Menu::Update, pill, Align::Right, move |_, cx| update_popover(v, cx), cx))
    }
}

/// A pill tinted with `color`: a light wash of it, its border and text.
fn tinted_pill(color: Rgba, open: bool) -> Stateful<Div> {
    let tint = |a: f32| Hsla { a, ..color.into() };
    div()
        .id("hdr-update")
        .flex()
        .flex_none()
        .items_center()
        .gap(px(7.))
        .h(theme::TOOL_BTN)
        .pl(px(10.))
        .pr(px(12.))
        .rounded_full()
        .bg(tint(if open { 0.24 } else { 0.14 }))
        .border_1()
        .border_color(tint(0.45))
        .cursor_pointer()
        .hover(move |s| s.bg(tint(0.24)))
}

/// The pill's popover: the release, its highlights and the actions of the
/// current step.
fn update_popover(v: UpdateView, cx: &mut Context<CollectionView>) -> AnyElement {
    let act = |f: fn(&mut Updater, &mut Context<Updater>)| {
        let view = cx.entity().downgrade();
        move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
            if let Some(updater) = update::get(cx) {
                updater.update(cx, f);
            }
            view.update(cx, |this, cx| {
                this.open_menu = None;
                cx.notify();
            })
            .ok();
        }
    };
    let title = |text: String| {
        div().font_family(theme::font_heading()).text_size(px(17.)).font_weight(theme::W_BOLD).text_color(theme::text_primary()).child(text)
    };
    let detail = |text: String| div().text_size(px(13.)).line_height(px(18.)).text_color(theme::text_muted()).child(text);
    let footer = div().flex().items_center().justify_end().gap(px(8.)).pt(px(6.));
    let page = v.page.clone();

    let mut panel = glass::glass_panel(theme::R_POPOVER).w(px(380.)).p(px(16.)).flex().flex_col().gap(px(8.));
    panel = match (v.kind, &v.phase) {
        (Banner::Available, _) => {
            let you_have =
                tr!("You have {current}{size}", current = v.current, size = v.size.map(|s| format!(" \u{b7} {s}")).unwrap_or_default());
            let mut list = div().flex().flex_col().gap(px(6.)).pt(px(4.));
            for note in v.highlights {
                list = list.child(
                    div()
                        .flex()
                        .gap(px(8.))
                        .text_size(px(13.))
                        .line_height(px(18.))
                        .text_color(theme::text_muted())
                        .child(div().flex_none().text_color(theme::text_dim()).child("\u{2022}"))
                        .child(div().flex_1().min_w_0().child(note)),
                );
            }
            if v.more > 0 {
                list = list.child(div().text_size(px(12.)).text_color(theme::text_dim()).child(tr!("and {count} more", count = v.more)));
            }
            let notes = button_sm("upd-notes", ButtonKind::Glass, tr!("All notes")).on_click({
                let view = cx.entity().downgrade();
                move |_, window, cx| {
                    view.update(cx, |this, cx| this.open_settings_section(Section::About, window, cx)).ok();
                }
            });
            let main = if v.can_install {
                button_sm("upd-install", ButtonKind::Primary, tr!("Update")).on_click(act(Updater::install))
            } else {
                button_sm("upd-page", ButtonKind::Primary, tr!("Download")).on_click(move |_, _, _| update::open_page(&page))
            };
            panel
                .child(title(format!("Heronite {}", v.version)))
                .child(detail(you_have))
                .child(
                    div().pt(px(6.)).text_size(px(11.)).font_weight(theme::W_BOLD).text_color(theme::text_dim()).child(tr!("What's new")),
                )
                .child(list)
                .child(footer.child(notes).child(main))
        }
        (Banner::Downloading, Phase::Downloading(p)) => panel
            .child(title(tr!("Downloading Heronite {version}", version = v.version)))
            .child(detail(update::progress_text(p)))
            .child(div().py(px(4.)).child(progress_bar(p.percent(), 4., theme::accent())))
            .child(footer.child(button_sm("upd-cancel", ButtonKind::Ghost, tr!("Cancel")).on_click(act(Updater::cancel)))),
        // The package manager cannot be stopped halfway.
        (Banner::Downloading, _) => panel
            .child(title(tr!("Installing Heronite {version}", version = v.version)))
            .child(detail(tr!("Enter your password if you are asked.").into())),
        (Banner::Ready, _) => {
            let why = match &v.blocker {
                Some(why) => tr!("{why}: restart later, or it starts next time.", why),
                None => tr!("Restart to use it now, or it starts next time.").into(),
            };
            panel.child(title(tr!("Heronite {version} is installed", version = v.version))).child(detail(why)).child(
                footer.child(button_sm("upd-later", ButtonKind::Ghost, tr!("Later")).on_click(act(Updater::dismiss))).child(
                    action_button("upd-restart", ButtonKind::Primary, tr!("Restart now"), v.blocker.is_none(), act(Updater::restart))
                        .h(px(theme::BUTTON_H_SM)),
                ),
            )
        }
        (Banner::Failed, phase) => {
            let error = match phase {
                Phase::InstallFailed(e) => e.clone(),
                _ => String::new(),
            };
            let other = match v.manual {
                Some(cmd) => button_sm("upd-copy", ButtonKind::Ghost, tr!("Copy terminal command"))
                    .on_click(move |_, _, cx| cx.write_to_clipboard(ClipboardItem::new_string(cmd.clone()))),
                None => button_sm("upd-page", ButtonKind::Ghost, tr!("Release page")).on_click(move |_, _, _| update::open_page(&page)),
            };
            panel.child(title(tr!("Could not update to {version}", version = v.version))).child(detail(error)).child(
                footer.child(other).child(button_sm("upd-retry", ButtonKind::Primary, tr!("Try again")).on_click(act(Updater::install))),
            )
        }
    };
    panel.into_any_element()
}

#[cfg(test)]
mod tests {
    use super::highlights;

    #[test]
    fn the_popover_lists_the_first_notes() {
        let md = "**Download:** x\n\n## Highlights\n\n- One\n- Two\n\n## Fixes\n\n- Three\n- Four\n- Five\n\n**Enjoying Heronite?** y";
        let (list, more) = highlights(md);
        assert_eq!(list, ["One", "Two", "Three", "Four"]);
        assert_eq!(more, 1);
        assert_eq!(highlights(""), (Vec::new(), 0));
    }
}
