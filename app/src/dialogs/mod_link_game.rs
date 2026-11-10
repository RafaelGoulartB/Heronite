//! "Which game is this mod for?": asked when an `nxm://` or `ror2mm://` link
//! names a game that cannot be told from the Collection. The answer is kept
//! for the site's game, so it is asked once.

use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_i18n::tr;

use super::{DialogCtx, DialogEvent};
use crate::services::mods::{self, LinkRequest};
use crate::theme;
use crate::ui::controls::{button, ButtonKind};
use crate::ui::dialog::{dialog, DialogProps};
use crate::ui::fields;

struct Choice {
    key: String,
    title: String,
    store: &'static str,
}

pub struct ModLinkGameDialog {
    req: LinkRequest,
    /// The suggested games first, then the other installed PC games.
    games: Vec<Choice>,
    suggested: usize,
    search: Entity<InputState>,
    query: String,
    answered: bool,
}

impl EventEmitter<DialogEvent> for ModLinkGameDialog {}

impl ModLinkGameDialog {
    pub fn new(ctx: DialogCtx, req: LinkRequest, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut pc: Vec<Choice> = ctx
            .lib
            .games
            .iter()
            .filter(|g| g.installed && g.launch_kind.as_deref() != Some("emulator"))
            .map(|g| Choice { key: g.key.clone(), title: g.title.clone(), store: g.source_label })
            .collect();
        pc.sort_by_key(|c| c.title.to_lowercase());
        // `candidates` are game keys (titles are accepted too).
        let mut games = Vec::new();
        for want in &req.candidates {
            if let Some(i) = pc.iter().position(|c| c.key == *want || c.title.eq_ignore_ascii_case(want)) {
                games.push(pc.remove(i));
            }
        }
        let suggested = games.len();
        games.extend(pc);
        let search = fields::new_input(window, cx, "", tr!("Search your games"));
        cx.subscribe_in(&search, window, |this, state, ev: &InputEvent, _, cx| {
            if matches!(ev, InputEvent::Change) {
                this.query = state.read(cx).value().trim().to_lowercase();
                cx.notify();
            }
        })
        .detach();
        Self { req, games, suggested, search, query: String::new(), answered: false }
    }

    fn answer(&mut self, key: Option<String>, cx: &mut Context<Self>) {
        if std::mem::replace(&mut self.answered, true) {
            return;
        }
        let id = self.req.id;
        mods::service(cx).update(cx, |s, cx| s.answer_link(id, key, cx));
        cx.emit(DialogEvent::Close);
    }

    /// Esc is "not now": the link is dropped.
    pub fn on_escape(&mut self, cx: &mut Context<Self>) -> bool {
        self.answer(None, cx);
        true
    }
}

impl Render for ModLinkGameDialog {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let entity = cx.entity().downgrade();
        let intro = match &self.req.site_game {
            Some(site) => {
                tr!("The link is for {game} on the mod site. Pick the game it belongs to here; Heronite remembers it.", game = site.clone())
            }
            None => tr!("Pick the game this link belongs to; Heronite remembers it.").to_string(),
        };
        let mut list = div().id("mod-link-list").flex().flex_col().gap(px(2.)).h(px(280.)).overflow_y_scroll();
        let mut shown = 0;
        for (i, c) in self.games.iter().enumerate() {
            let suggested = i < self.suggested;
            if !suggested && !self.query.is_empty() && !c.title.to_lowercase().contains(&self.query) {
                continue;
            }
            if i == 0 && suggested {
                list = list.child(heading(tr!("Likely")));
            }
            if i == self.suggested && self.suggested > 0 {
                list = list.child(heading(tr!("Your games")));
            }
            shown += 1;
            let key = c.key.clone();
            list = list.child(
                div()
                    .id(SharedString::from(format!("mod-link-{}", c.key)))
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap(px(10.))
                    .px(px(12.))
                    .py(px(9.))
                    .rounded(px(theme::R_ITEM))
                    .cursor_pointer()
                    .when(suggested, |d| d.bg(theme::accent_tint()))
                    .hover(|s| s.bg(theme::white_a(0.08)))
                    .child(div().min_w_0().truncate().text_size(px(14.)).child(c.title.clone()))
                    .child(div().flex_none().text_size(px(12.)).text_color(theme::text_dim()).child(c.store))
                    .on_click(cx.listener(move |this, _, _, cx| this.answer(Some(key.clone()), cx))),
            );
        }
        if shown == 0 {
            list =
                list.child(div().py(px(30.)).text_center().text_size(px(13.)).text_color(theme::text_dim()).child(tr!("No game matches.")));
        }
        let body = div()
            .flex()
            .flex_col()
            .gap(px(12.))
            .child(div().text_size(px(13.5)).line_height(px(19.)).text_color(theme::text_muted()).child(intro))
            .child(div().flex().child(fields::search_box(&self.search)))
            .child(list);
        let footer =
            button("mod-link-cancel", ButtonKind::Ghost, tr!("Cancel")).on_click(cx.listener(|this, _, _, cx| this.answer(None, cx)));
        dialog(
            DialogProps { id: "mod-link-game".into(), title: tr!("Which game is this mod for?").into(), width: 520., height: None },
            body.into_any_element(),
            Some(footer.into_any_element()),
            move |_, cx| {
                entity.update(cx, |this, cx| this.answer(None, cx)).ok();
            },
        )
    }
}

fn heading(text: &'static str) -> Div {
    div().mx(px(6.)).mt(px(6.)).mb(px(2.)).text_size(px(12.)).text_color(theme::text_muted()).child(text)
}
