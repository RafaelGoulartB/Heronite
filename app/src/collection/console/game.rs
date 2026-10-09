//! Console game page (A on a cover): the game large over its art
//! (`o2-2.jpg` header: cover, kicker, title, facts), the actions as console
//! rows on the left (Play / Install / Stop, Status ◀ ▶, Favourite, Game
//! settings, Hidden...) and what the desktop focus panel shows on the right
//! (the description scrolls with the right stick).

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::CollectionGame;

use super::super::vm::DetailValue;
use super::super::CollectionView;
use crate::console::kit::{self, u};
use crate::console::list::{ListNav, Outcome, Row, Value};
use crate::console::{hint, Btn, Hint, Pad};
use crate::icons::Icon;
use crate::theme;

#[derive(Default)]
pub(super) struct GameNav {
    list: ListNav,
    about: ScrollHandle,
}

fn detail_text(v: &DetailValue) -> String {
    match v {
        DetailValue::Text(t) => t.to_string(),
        DetailValue::Status { name, .. } => name.to_string(),
        DetailValue::Chips { values, .. } => values.iter().map(|s| s.as_ref()).collect::<Vec<_>>().join(", "),
        DetailValue::Folder { path, size } => match size {
            Some(s) => format!("{path} ({s})"),
            None => path.to_string(),
        },
    }
}

impl CollectionView {
    fn game_rows(&mut self, g: &CollectionGame, cx: &mut Context<Self>) -> Vec<Row> {
        let playing = self.playing.contains_key(&g.app_name);
        let (label, ic) = if playing {
            ("Stop", Icon::Stop)
        } else if g.installed {
            ("Play", Icon::PlayArrow)
        } else {
            ("Install", Icon::Download)
        };
        let mut rows = vec![Row::action("primary", label).icon(ic)];
        if let Some(lib) = &self.lib {
            let names: Vec<SharedString> = lib.statuses.iter().map(|s| SharedString::from(s.name.clone())).collect();
            let index = lib.statuses.iter().position(|s| s.id == g.status_id).unwrap_or(0);
            rows.push(Row::choice("status", "Status", names, index).icon(Icon::Flag));
        }
        rows.push(Row::toggle("favourite", "Favourite", g.favourite).icon(Icon::Favorite));
        if g.installed {
            rows.push(Row::action("settings", "Game settings").icon(Icon::Tune).desc("Wine / Proton, launch options, Gamescope\u{2026}"));
        }
        if !playing {
            rows.push(Row::text("playtime", "Playtime", heronite_data::collection::playtime_label(g.playtime_minutes)).icon(Icon::AccessTime));
        }

        // What the desktop context menu offers besides the above.
        let mut manage = self.epic_console_rows(g, cx);
        let steam_game = matches!(g.launch_kind.as_deref(), Some("steam-uri"));
        if g.installed && !steam_game && g.game.install.is_dlc != Some(true) {
            let (shortcuts, in_steam) = self.probe_shortcuts(g, cx);
            manage.push(Row::toggle("sys:shortcuts", "Desktop and menu shortcuts", shortcuts).icon(Icon::OpenInNew));
            manage.push(Row::toggle("sys:steam", "In Steam", in_steam).icon(Icon::SportsEsports).desc("As a non-Steam game in your Steam library"));
        }
        manage.push(Row::action("x:ludusavi", "Back up saves (Ludusavi)").icon(Icon::Save));
        manage.push(Row::toggle("hidden", "Hidden", g.hidden).icon(Icon::VisibilityOff).desc("Hidden games stay out of the library"));
        if g.recent {
            manage.push(Row::action("recent", "Remove from recent").icon(Icon::History));
        }
        if g.runner == "sideload" && !playing && !matches!(g.launch_kind.as_deref(), Some("steam-uri" | "emulator")) {
            manage.push(Row::action("x:edit", "Edit game").icon(Icon::Edit));
        }
        if g.steam_launch_id.is_some() && g.installed && !playing {
            manage.push(Row::action("x:steam-uninstall", "Uninstall\u{2026}").icon(Icon::DeleteForever).danger());
        }
        if g.runner == "sideload" && !playing {
            manage.push(Row::action("x:remove", "Remove from Collection\u{2026}").icon(Icon::DeleteForever).danger());
        }
        rows.push(Row::header("Manage"));
        rows.extend(manage);
        rows
    }

    pub(super) fn render_console_game(&mut self, key: &SharedString, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let Some(g) = self.lib.as_ref().and_then(|l| l.get(key)).cloned() else {
            return kit::empty(Icon::Warning, "This game is gone", "Press B to go back.").into_any_element();
        };
        self.console_focus_game(Some(key.clone()), cx);
        let rows = self.game_rows(&g, cx);
        let list = self.console.game.list.render("console-game-actions", &rows, true);
        let s = kit::scale();
        let (cw, ch) = (130., 130. * self.cover_aspect);
        let cover = self.cover(key, cw * s, ch * s, window.scale_factor(), 300);
        let focus = self.focus.clone().filter(|f| f.key == *key);

        let mut facts = vec![g.source_label.to_string(), if g.installed { "Installed".into() } else { "Not installed".into() }];
        if g.playtime_minutes > 0 {
            facts.push(format!("{} played", heronite_data::collection::format_playtime_minutes(g.playtime_minutes)));
        }
        let header = div()
            .flex()
            .items_end()
            .gap(u(26.))
            .px(u(46.))
            .pt(u(70.))
            .pb(u(22.))
            .child(
                div()
                    .flex_none()
                    .w(u(cw))
                    .h(u(ch))
                    .rounded(u(12.))
                    .overflow_hidden()
                    .bg(theme::white_a(0.08))
                    .shadow(kit::shadow(0.6, 10., 30.))
                    .children(cover.map(|c| img(c).size_full().object_fit(ObjectFit::Cover))),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(u(6.))
                    .min_w_0()
                    .child(kit::kicker("Game"))
                    .child(kit::title(g.title.clone(), 52.))
                    .child(div().text_size(u(16.)).text_color(theme::text_muted()).child(facts.join(" \u{b7} ")))
                    .children(focus.as_ref().map(|f| f.meta_line.clone()).filter(|m| !m.is_empty()).map(|m| {
                        div().text_size(u(14.)).text_color(theme::text_dim()).child(m)
                    })),
            );

        let mut about = div().id("console-game-about").flex().flex_col().gap(u(14.)).overflow_y_scroll().track_scroll(&self.console.game.about);
        if let Some(f) = &focus {
            let facts = div().flex().flex_col().gap(u(8.)).children(f.details.iter().map(|d| {
                div()
                    .flex()
                    .gap(u(14.))
                    .child(div().flex_none().w(u(150.)).text_size(u(14.)).text_color(theme::text_dim()).child(d.label.clone()))
                    .child(div().flex_1().min_w_0().text_size(u(14.)).text_color(theme::text_primary()).child(detail_text(&d.value)))
            }));
            about = about.child(facts);
            if let Some(h) = &f.hltb {
                let fmt = |v: Option<f32>| v.map_or("\u{2014}".to_string(), |h| format!("{h:.0}h"));
                about = about.child(
                    div()
                        .flex()
                        .gap(u(24.))
                        .text_size(u(14.))
                        .text_color(theme::text_muted())
                        .child(format!("Main story {}", fmt(h.main)))
                        .child(format!("Main + extras {}", fmt(h.extra)))
                        .child(format!("Completionist {}", fmt(h.complete))),
                );
            }
            if let Some(o) = &f.overview {
                about = about.child(kit::body_text(o.clone()));
            }
        }

        div()
            .flex()
            .flex_col()
            .size_full()
            .child(header)
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_h_0()
                    .gap(u(40.))
                    .px(u(46.))
                    .pb(u(20.))
                    .child(div().flex_none().w(u(560.)).flex().flex_col().child(list.flex_1().min_h_0()))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .p(u(22.))
                            .rounded(u(18.))
                            .bg(theme::black_a(0.35))
                            .border_1()
                            .border_color(theme::white_a(0.08))
                            .flex()
                            .flex_col()
                            .child(about.flex_1().min_h_0()),
                    ),
            )
            .into_any_element()
    }

    pub(super) fn game_pad(&mut self, key: &SharedString, pad: Pad, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some(g) = self.lib.as_ref().and_then(|l| l.get(key)).cloned() else { return false };
        if matches!(pad, Pad::ScrollUp | Pad::ScrollDown) {
            let h = &self.console.game.about;
            let off = h.offset();
            let step = u(120.);
            let y = if pad == Pad::ScrollDown { off.y - step } else { off.y + step };
            let max = h.max_offset().y;
            h.set_offset(point(off.x, y.clamp(-max, px(0.))));
            return true;
        }
        if pad == Pad::X {
            self.primary_action(key, cx);
            return true;
        }
        let rows = self.game_rows(&g, cx);
        match self.console.game.list.on_pad(pad, &rows) {
            Outcome::Activate(id) if id.starts_with("epic:") => self.epic_console_activate(&id, key, cx),
            Outcome::Changed(id, _) if id.as_ref() == "epic:eos" => self.epic_console_activate(&id, key, cx),
            Outcome::Changed(id, Value::Toggle(on)) if id.starts_with("sys:") => {
                let (runner, app) = (g.runner.clone(), g.app_name.clone());
                let steam = id.as_ref() == "sys:steam";
                self.system_op(g.key.clone(), cx, move |heroic| {
                    use crate::system::shortcuts as sc;
                    match (steam, on) {
                        (false, true) => sc::add_shortcuts(heroic, &runner, &app),
                        (false, false) => sc::remove_shortcuts(heroic, &runner, &app),
                        (true, true) => sc::add_to_steam(heroic, &runner, &app),
                        (true, false) => sc::remove_from_steam(heroic, &runner, &app),
                    }
                });
            }
            Outcome::Activate(id) => match id.as_ref() {
                "primary" => self.primary_action(key, cx),
                "x:ludusavi" => self.open_ludusavi_backup(key, cx),
                "x:steam-uninstall" => self.open_steam_uninstall(key, window, cx),
                "x:remove" => self.open_remove_game(key, cx),
                "x:edit" => {
                    self.open_edit_game(key, window, cx);
                }
                "settings" => self.console_game_settings(key, window, cx),
                "playtime" => self.console_edit_playtime(key, window, cx),
                "recent" => {
                    if let Err(err) = heronite_data::config_games::remove_recent(&self.heroic, &g.app_name) {
                        self.show_toast(format!("{err:#}"), true, cx);
                    }
                    self.reload(cx);
                }
                _ => {}
            },
            Outcome::Changed(id, value) => match (id.as_ref(), value) {
                ("status", Value::Choice { index, .. }) => {
                    if let Some(st) = self.lib.as_ref().and_then(|l| l.statuses.get(index)).cloned() {
                        self.set_status(key, st.id, cx);
                    }
                }
                ("favourite", _) => {
                    if let Err(err) = heronite_data::config_games::toggle_favourite(&self.heroic, &g.app_name, &g.title) {
                        self.show_toast(format!("{err:#}"), true, cx);
                    }
                    self.reload(cx);
                }
                ("hidden", _) => {
                    if let Err(err) = heronite_data::config_games::toggle_hidden(&self.heroic, &g.app_name, &g.title) {
                        self.show_toast(format!("{err:#}"), true, cx);
                    }
                    self.reload(cx);
                }
                _ => {}
            },
            Outcome::Moved | Outcome::Edge(_) => {}
            Outcome::Ignored => return false,
        }
        true
    }

    pub(super) fn game_hints(&self, key: &SharedString) -> Vec<Hint> {
        let primary = self.lib.as_ref().and_then(|l| l.get(key)).map(|g| self.primary_label(g)).unwrap_or("Play");
        vec![hint(Btn::A, "Select"), hint(Btn::LeftRight, "Change"), hint(Btn::X, primary), hint(Btn::B, "Back"), hint(Btn::Start, "Menu")]
    }
}
