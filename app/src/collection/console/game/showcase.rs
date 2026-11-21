//! The game page as Showcase: the game full screen over the theme's
//! background, its logo (or name), Play / Options / ♥, its facts and genres,
//! and under them the sections of Spotlight's page (`layouts/spotlight`:
//! the store's and the player's screenshots, About this game, Achievements,
//! Details, How Long To Beat, More like this...) on panels of frosted glass,
//! with the same focus, the same viewers and the same section switches
//! (`layouts.spotlightSections`). Options lists the game's actions in a
//! sheet; B goes back to the screen it was opened from, as it was.

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_i18n::tr;

use super::super::super::CollectionView;
use super::super::layouts::SpotHost;
use super::sheet::side_sheet;
use crate::console::kit::{self, u};
use crate::console::list::{ListNav, Outcome};
use crate::console::{hint, motion, Btn, Hint, Pad};

impl CollectionView {
    /// Options on the page: the sheet of the game's actions.
    pub(in crate::collection::console) fn open_showcase_options(&mut self) {
        self.console.game.showcase_more = Some(ListNav::default());
    }

    pub(super) fn render_game_showcase(&mut self, key: &SharedString, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        if self.lib.as_ref().and_then(|l| l.get(key)).is_none() {
            return self.render_game_classic(key, window, cx);
        }
        let page = self.render_spot_page(SpotHost::Game, std::slice::from_ref(key), window, cx);
        let more = match self.console.game.showcase_more.is_some() {
            true => {
                let g = self.lib.as_ref().and_then(|l| l.get(key)).cloned();
                g.map(|g| {
                    let rows = self.game_rows(&g, cx);
                    let list = self.console.game.showcase_more.as_mut().map(|l| l.render("console-showcase-more", &rows, true));
                    let body = div()
                        .flex()
                        .flex_col()
                        .size_full()
                        .child(div().px(u(34.)).pt(u(30.)).pb(u(10.)).child(kit::title(tr!("Options"), 26.)))
                        .child(div().flex_1().min_h_0().px(u(12.)).pb(u(10.)).children(list));
                    side_sheet("console-showcase-more", body)
                })
            }
            false => None,
        };
        // The page comes in over the screen it opens from: a short fade as it
        // rises a little into place.
        let page =
            motion::enter(SharedString::from(format!("console-showcase-{key}")), div().relative().size_full().child(page), 320, 0., 18.);
        div().relative().size_full().child(page).children(more.map(|m| div().absolute().inset_0().occlude().child(m))).into_any_element()
    }

    pub(super) fn game_pad_showcase(&mut self, key: &SharedString, pad: Pad, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some(g) = self.lib.as_ref().and_then(|l| l.get(key)).cloned() else { return false };
        if self.console.game.showcase_more.is_some() {
            let rows = self.game_rows(&g, cx);
            let Some(list) = self.console.game.showcase_more.as_mut() else { return true };
            match list.on_pad(pad, &rows) {
                Outcome::Activate(id) | Outcome::Changed(id, _) => self.game_press_row(key, &id, pad, window, cx),
                Outcome::Ignored if pad == Pad::B => self.console.game.showcase_more = None,
                _ => {}
            }
            return true;
        }
        if pad == Pad::X {
            self.primary_action(key, cx);
            return true;
        }
        self.spot_page_pad(SpotHost::Game, std::slice::from_ref(key), pad, window, cx)
    }

    pub(super) fn game_hints_showcase(&self, key: &SharedString) -> Vec<Hint> {
        if self.console.game.showcase_more.is_some() {
            return vec![hint(Btn::A, tr!("Select")), hint(Btn::LeftRight, tr!("Change")), hint(Btn::B, tr!("Close"))];
        }
        self.spot_page_hints(SpotHost::Game, std::slice::from_ref(key))
    }
}
