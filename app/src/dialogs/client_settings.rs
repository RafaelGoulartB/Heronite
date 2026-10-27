//! "Wine settings" of a store client (Battle.net, the EA app, Ubisoft
//! Connect, the Rockstar Games Launcher): the settings panel bound to the
//! client's own `GamesConfig` file (`Client::config_app`), as a Windows
//! program run by Wine. It shows what makes sense for a program that is not
//! a game: Wine and the prefix, the environment and scripts, the graphics
//! card. The launch preview needs a game, so it is left out.

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::clients::Client;
use heronite_data::DataPaths;
use heronite_i18n::tr;

use super::settings_panel::logic::RunKind;
use super::settings_panel::{GameFacts, Section, SettingsPanel, Target};
use super::DialogEvent;
use crate::console::kit::{self, u};
use crate::console::{hint, Btn, Hint, Pad, PadTarget};
use crate::theme;
use crate::ui::controls::{action_button, button, close_button, dialog_title, kicker, tabs, ButtonKind};
use crate::ui::dialog::{drawer, DrawerProps};

/// The panel's tabs, in order: Wine first.
const SECTIONS: [Section; 3] = [Section::Compatibility, Section::Launch, Section::Gpu];

const DRAWER_W: f32 = 580.;

pub struct ClientSettingsDialog {
    client: Client,
    panel: Entity<SettingsPanel>,
    section: Section,
}

impl EventEmitter<DialogEvent> for ClientSettingsDialog {}

impl ClientSettingsDialog {
    pub fn new(data_paths: DataPaths, client: Client, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let facts = GameFacts {
            app_name: client.config_app().to_string(),
            title: client.name().to_string(),
            runner: "sideload".into(),
            run_kind: RunKind::Wine,
            cloud_saves: false,
            steam_app_id: None,
            protondb_id: None,
            game: None,
        };
        let panel = cx.new(|cx| SettingsPanel::new(data_paths, Target::Game(facts), SECTIONS[0], window, cx));
        cx.observe(&panel, |_, _, cx| cx.notify()).detach();
        Self { client, panel, section: SECTIONS[0] }
    }

    fn set_section(&mut self, section: Section, window: &mut Window, cx: &mut Context<Self>) {
        self.section = section;
        self.panel.update(cx, |p, cx| {
            p.set_section(section, window, cx);
            p.console_reset();
        });
        cx.notify();
    }

    /// The tab LB / RB lands on from the current one.
    fn neighbour(&self, pad: Pad) -> Section {
        let i = SECTIONS.iter().position(|s| *s == self.section).unwrap_or(0);
        let n = SECTIONS.len();
        SECTIONS[if pad == Pad::RB { (i + 1) % n } else { (i + n - 1) % n }]
    }

    fn render_console(&mut self) -> AnyElement {
        let labels: Vec<SharedString> = SECTIONS.iter().map(|s| SharedString::from(s.label())).collect();
        let active = SECTIONS.iter().position(|s| *s == self.section).unwrap_or(0);
        div()
            .absolute()
            .inset_0()
            .bg(theme::console_bg())
            .flex()
            .flex_col()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_none()
                    .gap(u(4.))
                    .px(u(46.))
                    .pt(u(36.))
                    .child(kit::kicker(tr!("Wine settings")))
                    .child(kit::title(self.client.name(), 40.)),
            )
            .child(div().flex_none().px(u(46.)).py(u(12.)).child(kit::tabs(&labels, active)))
            .child(
                div().flex().flex_col().flex_1().min_h_0().px(u(46.)).pb(u(70.)).child(
                    div()
                        .w(u(900.))
                        .max_w_full()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .min_h_0()
                        .child(div().flex().flex_col().size_full().child(self.panel.clone())),
                ),
            )
            .into_any_element()
    }
}

impl Render for ClientSettingsDialog {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if crate::console::active() {
            return self.render_console();
        }
        let items: Vec<(Section, &'static str)> = SECTIONS.iter().map(|s| (*s, s.label())).collect();
        let entity = cx.entity().downgrade();
        let tab_bar = tabs("cs", &items, self.section, {
            let entity = entity.clone();
            move |s, window, cx| {
                entity.update(cx, |this, cx| this.set_section(s, window, cx)).ok();
            }
        });
        let header = div()
            .flex()
            .flex_col()
            .gap(px(14.))
            .pb(px(10.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(14.))
                    .px(px(20.))
                    .pt(px(20.))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .flex_1()
                            .min_w_0()
                            .gap(px(1.))
                            .child(kicker(tr!("Wine settings")))
                            .child(dialog_title(self.client.name()).truncate()),
                    )
                    .child(
                        div()
                            .self_start()
                            .child(close_button("cs-close-x").on_click(cx.listener(|_, _, _, cx| cx.emit(DialogEvent::Close)))),
                    ),
            )
            .child(div().px(px(20.)).child(tab_bar));
        let changed = self.panel.read(cx).tab_changed();
        let panel = self.panel.clone();
        let footer = div()
            .flex()
            .w_full()
            .items_center()
            .justify_end()
            .gap(px(8.))
            .child(action_button("cs-reset-tab", ButtonKind::Ghost, tr!("Reset this tab"), changed, move |_, _, cx| {
                panel.update(cx, |p, cx| p.reset_tab(cx))
            }))
            .child(button("cs-close", ButtonKind::Primary, tr!("Done")).on_click(cx.listener(|_, _, _, cx| cx.emit(DialogEvent::Close))));
        let vw = f32::from(window.viewport_size().width);
        drawer(
            DrawerProps { id: "client-settings".into(), width: DRAWER_W.min(vw * 0.96) },
            header.into_any_element(),
            div().child(self.panel.clone()).into_any_element(),
            Some(footer.into_any_element()),
            move |_, cx| {
                entity.update(cx, |_, cx| cx.emit(DialogEvent::Close)).ok();
            },
        )
        .into_any_element()
    }
}

impl PadTarget for ClientSettingsDialog {
    fn on_pad(&mut self, pad: Pad, window: &mut Window, cx: &mut Context<Self>) -> bool {
        match pad {
            Pad::LB | Pad::RB => {
                let next = self.neighbour(pad);
                self.set_section(next, window, cx);
                true
            }
            // The panel keeps B while it has a keyboard or a picker open.
            Pad::B => {
                if !self.panel.update(cx, |p, cx| p.on_pad(pad, window, cx)) {
                    cx.emit(DialogEvent::Close);
                }
                true
            }
            _ => self.panel.update(cx, |p, cx| p.on_pad(pad, window, cx)),
        }
    }

    fn hints(&self, cx: &App) -> Vec<Hint> {
        let mut h = self.panel.read(cx).hints(cx);
        h.push(hint(Btn::Bumpers, tr!("Tabs")));
        h.push(hint(Btn::B, tr!("Done")));
        h
    }
}

#[cfg(test)]
mod tests {
    use super::{Section, SECTIONS};

    #[test]
    fn wine_comes_first_and_no_tab_needs_a_game() {
        assert_eq!(SECTIONS[0], Section::Compatibility);
        assert!(!SECTIONS.contains(&Section::Sandbox));
    }
}
