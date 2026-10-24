//! The game page as Sheet: a side sheet over the tab with the essentials
//! (cover, name, facts, status, Play, Status, Favourite, Game settings);
//! "Full page" shows the Classic page until the game page closes. The sheet
//! shell ([`side_sheet`]) also holds Hero's "More".

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_i18n::tr;

use super::super::super::CollectionView;
use super::super::hero::HeroSize;
use crate::console::kit::{self, u};
use crate::console::list::{ListNav, Outcome, Row};
use crate::console::motion;
use crate::console::{hint, Btn, Hint, Pad};
use crate::icons::Icon;
use crate::theme;

#[derive(Default)]
pub(super) struct Nav {
    list: ListNav,
    /// "Full page" was picked: the Classic page takes over.
    full: bool,
}

impl Nav {
    /// "Full page" took over (the Classic page shows instead).
    pub(super) fn full(&self) -> bool {
        self.full
    }
}

/// Design width of the sheet.
const WIDTH: f32 = 440.;

/// The rows the sheet shows: the main ones of the game's actions, then "Full page".
fn sheet_rows(all: Vec<Row>) -> Vec<Row> {
    let mut rows: Vec<Row> = ["primary", "status", "favourite", "settings"]
        .into_iter()
        .filter_map(|id| all.iter().find(|r| r.id.as_ref() == id).cloned())
        .collect();
    rows.push(Row::action("full", tr!("Full page")).icon(Icon::OpenInNew));
    rows
}

/// A panel on the right over a light dim, sliding in. `id` restarts the slide.
pub(super) fn side_sheet(id: &'static str, body: impl IntoElement) -> AnyElement {
    let panel = div()
        .absolute()
        .top_0()
        .bottom_0()
        .right_0()
        .w(u(WIDTH))
        .flex()
        .flex_col()
        .bg(theme::console_sidebar())
        .border_l_1()
        .border_color(kit::ink(0.08))
        .shadow(kit::shadow(0.5, 0., 40.))
        .child(body);
    div()
        .absolute()
        .inset_0()
        .child(motion::fade_in("console-sheet-dim", div().absolute().inset_0().bg(kit::dim(0.4)), 180))
        .child(motion::enter(id, panel, 280, 40., 0.))
        .into_any_element()
}

impl CollectionView {
    /// Presses `pad` on the row `id` of the game's actions as the Classic page
    /// does, so every page runs an action the same way.
    pub(super) fn game_press_row(&mut self, key: &SharedString, id: &str, pad: Pad, window: &mut Window, cx: &mut Context<Self>) {
        let Some(g) = self.lib.as_ref().and_then(|l| l.get(key)).cloned() else { return };
        let rows = self.game_rows(&g, cx);
        let Some(ix) = rows.iter().position(|r| r.id.as_ref() == id) else { return };
        self.console.game.list.focus = ix;
        self.game_pad_classic(key, pad, window, cx);
    }

    pub(super) fn render_game_sheet(&mut self, key: &SharedString, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let g = self.lib.as_ref().and_then(|l| l.get(key)).cloned();
        let Some(g) = g.filter(|_| !self.console.game.sheet.full) else { return self.render_game_classic(key, window, cx) };
        // The tab stays under the sheet, as it was.
        let (tab, spec) = self.console_tab_spec();
        let under = self.render_layout(tab, &spec, window, cx);
        let rows = sheet_rows(self.game_rows(&g, cx));
        let list = self.console.game.sheet.list.render("console-game-sheet", &rows, true);

        let s = kit::scale();
        let (cw, ch) = (96., 96. * self.cover_aspect);
        let cover = self.cover(key, cw * s, ch * s, window.scale_factor(), 300);
        let size = HeroSize { title: 24., logo: (240., 70.), max_w: 280., overview: 0 };
        let name = self.console_name(&g, size, window);
        let head = div()
            .flex()
            .items_end()
            .gap(u(16.))
            .px(u(22.))
            .pt(u(24.))
            .pb(u(14.))
            .child(
                div()
                    .flex_none()
                    .w(u(cw))
                    .h(u(ch))
                    .rounded(u(10.))
                    .overflow_hidden()
                    .bg(kit::ink(0.08))
                    .shadow(kit::shadow(0.5, 10., 26.))
                    .children(cover.map(|c| img(c).size_full().object_fit(ObjectFit::Cover))),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(u(6.))
                    .min_w_0()
                    .child(name)
                    .child(div().text_size(u(13.5)).text_color(theme::text_muted()).child(Self::console_facts(&g)))
                    .children(self.console_status_badge(&g).map(|b| div().flex().child(b))),
            );
        let body = div().flex().flex_col().size_full().child(head).child(div().flex_1().min_h_0().px(u(12.)).py(u(6.)).child(list));
        div().relative().size_full().child(under).child(side_sheet("console-game-sheet-panel", body)).into_any_element()
    }

    pub(super) fn game_pad_sheet(&mut self, key: &SharedString, pad: Pad, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if self.console.game.sheet.full {
            return self.game_pad_classic(key, pad, window, cx);
        }
        let Some(g) = self.lib.as_ref().and_then(|l| l.get(key)).cloned() else { return false };
        if pad == Pad::X {
            self.primary_action(key, cx);
            return true;
        }
        let rows = sheet_rows(self.game_rows(&g, cx));
        match self.console.game.sheet.list.on_pad(pad, &rows) {
            Outcome::Activate(id) if id.as_ref() == "full" => self.console.game.sheet.full = true,
            Outcome::Activate(id) | Outcome::Changed(id, _) => self.game_press_row(key, &id, pad, window, cx),
            Outcome::Moved | Outcome::Edge(_) => {}
            Outcome::Ignored => return false,
        }
        true
    }

    pub(super) fn game_hints_sheet(&self, key: &SharedString) -> Vec<Hint> {
        if self.console.game.sheet.full {
            return self.game_hints_classic(key);
        }
        let primary = self.lib.as_ref().and_then(|l| l.get(key)).map(|g| self.primary_label(g)).unwrap_or(tr!("Play"));
        vec![hint(Btn::A, tr!("Select")), hint(Btn::LeftRight, tr!("Change")), hint(Btn::X, primary), hint(Btn::B, tr!("Close"))]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    // `gpui_kit::*` brings gpui's `test` macro into scope.
    use core::prelude::v1::test;

    #[test]
    fn the_sheet_keeps_the_main_rows_and_ends_with_the_full_page() {
        let all = vec![
            Row::action("primary", "Play"),
            Row::action("ua:1", "Mod manager"),
            Row::toggle("favourite", "Favourite", false),
            Row::header("Manage"),
            Row::action("x:ludusavi", "Back up"),
        ];
        let ids: Vec<_> = sheet_rows(all).iter().map(|r| r.id.to_string()).collect();
        assert_eq!(ids, ["primary", "favourite", "full"]);
    }
}
