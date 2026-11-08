//! The Steam pill of the Collection's header, next to the update pill
//! (`services::steam_library`). It shows only while a restart of Steam is
//! what brings in the rest of a library that was never read (Steam was
//! open before Heronite turned on the access it needs), then while Steam
//! restarts; it goes once the library comes in, and for good after
//! "Ignore". Hovering it shows why and the restart. The Steam page has the
//! full status. A Collection without games has the welcome instead.

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_i18n::tr;

use super::downloads_pill::{pill_base, ring};
use super::header::Menu;
use super::CollectionView;
use crate::icons::Icon;
use crate::pages::Page;
use crate::services::steam_library::{SteamLibrary, SteamNotice};
use crate::theme;
use crate::ui::controls::{button_sm, ButtonKind};
use crate::ui::dropdown::Align;
use crate::ui::{glass, icon};

impl CollectionView {
    pub(super) fn steam_pill(&mut self, cx: &mut Context<Self>) -> Option<Stateful<Div>> {
        if self.lib.as_ref().is_some_and(|lib| lib.games.is_empty()) {
            return None;
        }
        let notice = crate::services::steam_library_started(cx)?.read(cx).notice()?;
        let open = self.open_menu == Some(Menu::Steam);
        let label = |text: &str, color: Hsla| {
            div().flex_none().text_size(px(13.)).font_weight(theme::W_BOLD).text_color(color).child(text.to_string())
        };
        let pill = match notice {
            // A faint wash of the warning colour: seen without shouting over
            // the header.
            SteamNotice::Restart => soft_pill(open)
                .child(icon(Icon::BrandSteam, px(16.), theme::warning()))
                .child(label(tr!("Restart Steam"), theme::readable(theme::warning()))),
            SteamNotice::Restarting => pill_base("hdr-steam", open)
                .child(ring(100., theme::accent().into(), Some(Icon::BrandSteam)))
                .child(label(tr!("Restarting Steam"), theme::text_primary().into())),
        };
        Some(self.hover_dropdown(Menu::Steam, pill, Align::Right, move |_, cx| steam_popover(notice, cx), cx))
    }
}

/// The restart pill: a faint wash of the warning colour (about half the
/// update pill's), its border and text.
fn soft_pill(open: bool) -> Stateful<Div> {
    let tint = |a: f32| Hsla { a, ..theme::warning().into() };
    div()
        .id("hdr-steam")
        .flex()
        .flex_none()
        .items_center()
        .gap(px(7.))
        .h(theme::TOOL_BTN)
        .pl(px(10.))
        .pr(px(12.))
        .rounded_full()
        .bg(tint(if open { 0.12 } else { 0.07 }))
        .border_1()
        .border_color(tint(0.25))
        .cursor_pointer()
        .hover(move |s| s.bg(tint(0.12)))
}

/// The pill's popover: why, and the restart.
fn steam_popover(notice: SteamNotice, cx: &mut Context<CollectionView>) -> AnyElement {
    let act = |f: fn(&mut SteamLibrary, &mut Context<SteamLibrary>)| {
        let view = cx.entity().downgrade();
        move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
            crate::services::steam_library(cx).update(cx, f);
            view.update(cx, |this, cx| {
                this.open_menu = None;
                cx.notify();
            })
            .ok();
        }
    };
    let title = |text: &str| {
        div()
            .font_family(theme::font_heading())
            .text_size(px(17.))
            .font_weight(theme::W_BOLD)
            .text_color(theme::text_primary())
            .child(text.to_string())
    };
    let detail = |text: &str| div().text_size(px(13.)).line_height(px(18.)).text_color(theme::text_muted()).child(text.to_string());
    let footer = div().flex().items_center().justify_end().gap(px(8.)).pt(px(6.));
    let panel = glass::glass_panel(theme::R_POPOVER).w(px(360.)).p(px(16.)).flex().flex_col().gap(px(8.));
    let steam_page = {
        let view = cx.entity().downgrade();
        button_sm("steam-page", ButtonKind::Glass, tr!("Steam page")).on_click(move |_, window, cx| {
            view.update(cx, |this, cx| {
                this.open_menu = None;
                this.navigate(Page::Steam, window, cx);
            })
            .ok();
        })
    };
    match notice {
        SteamNotice::Restart => panel
            .child(title(tr!("Restart Steam once")))
            .child(detail(tr!(
                "Only your installed Steam games are in the Collection for now. Restart Steam once and the rest of your Steam library comes in by itself."
            )))
            .child(detail(tr!("Games running from Steam close.")))
            .child(
                footer
                    .child(button_sm("steam-ignore", ButtonKind::Ghost, tr!("Ignore")).on_click(act(SteamLibrary::ignore_notice)))
                    .child(div().flex_1())
                    .child(steam_page)
                    .child(button_sm("steam-restart", ButtonKind::Primary, tr!("Restart Steam")).on_click(act(SteamLibrary::restart))),
            ),
        SteamNotice::Restarting => panel
            .child(title(tr!("Restarting Steam")))
            .child(detail(tr!("Your Steam library comes in as soon as Steam is back. If Steam asks you to sign in, do so.")))
            .child(footer.child(steam_page)),
    }
    .into_any_element()
}
