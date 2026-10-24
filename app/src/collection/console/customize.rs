//! ☰ → Customize: the console theme's options as a side panel over the live
//! screen (each change shows at once behind it). The rows are the ones of
//! Settings → Console mode (`console::theme_rows`); LB / RB switch the group,
//! Y hides the panel to see the whole screen, B closes it.

use std::path::PathBuf;

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::console_theme::store;
use heronite_data::organize::Organize;
use heronite_data::paths::AppPaths;
use heronite_i18n::{tr, tr as tr_};

use super::super::CollectionView;
use crate::console::browser::{Browser, BrowserEvent, PickKind};
use crate::console::kit::{self, u};
use crate::console::list::{ListNav, Outcome, Row};
use crate::console::motion;
use crate::console::osk::{Osk, OskEvent};
use crate::console::sheet::{Sheet, SheetEvent};
use crate::console::theme_rows::{self, Extra, Facts, Host, ThemeState, PANEL};
use crate::console::{hint, Btn, Hint, Pad};
use crate::theme;

/// The panel's focus.
pub(super) struct CustomizeNav {
    group: usize,
    list: ListNav,
    state: ThemeState,
    /// Hidden with Y, to see the screen.
    peek: bool,
}

impl Host for CollectionView {
    fn app(&self) -> AppPaths {
        self.app_paths.clone()
    }

    fn theme_state(&mut self) -> Option<&mut ThemeState> {
        self.console.customize.as_mut().map(|c| &mut c.state)
    }

    fn open_osk(
        &mut self,
        title: &str,
        initial: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
        done: impl Fn(&mut Self, String, &mut Window, &mut Context<Self>) + 'static,
    ) {
        let osk = cx.new(|_| Osk::new(title.to_string(), initial));
        let sub = cx.subscribe_in(&osk, window, move |this, _, ev: &OskEvent, window, cx| {
            this.close_console_overlay(cx);
            if let OskEvent::Done(text) = ev {
                done(this, text.clone(), window, cx);
            }
        });
        self.open_console_overlay(&osk, sub, cx);
    }

    fn open_browser(
        &mut self,
        title: &str,
        kind: PickKind,
        start: Option<PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
        picked: impl Fn(&mut Self, PathBuf, &mut Window, &mut Context<Self>) + 'static,
    ) {
        let browser = cx.new(|cx| Browser::new(title.to_string(), kind, start.as_deref(), cx));
        let sub = cx.subscribe_in(&browser, window, move |this, _, ev: &BrowserEvent, window, cx| {
            this.close_console_overlay(cx);
            if let BrowserEvent::Picked(p) = ev {
                picked(this, p.clone(), window, cx);
            }
        });
        self.open_console_overlay(&browser, sub, cx);
    }

    fn open_sheet(
        &mut self,
        title: &str,
        message: Option<String>,
        rows: Vec<Row>,
        window: &mut Window,
        cx: &mut Context<Self>,
        chosen: impl Fn(&mut Self, &str, &mut Window, &mut Context<Self>) + 'static,
    ) {
        let sheet = cx.new(|_| Sheet::new(title.to_string(), message, rows));
        let sub = cx.subscribe_in(&sheet, window, move |this, _, ev: &SheetEvent, window, cx| {
            let SheetEvent::Activate(id) = ev;
            this.close_console_overlay(cx);
            chosen(this, id, window, cx);
        });
        self.open_console_overlay(&sheet, sub, cx);
    }

    fn start_options_changed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.console_start_options_changed(window, cx);
    }
}

impl CollectionView {
    /// Opens the panel: reads the saved themes (a few small files) and takes
    /// the names the pickers offer from the loaded library.
    pub(super) fn open_console_customize(&mut self) {
        let mut state = ThemeState::new(&self.app_paths);
        state.data.saved = store::list(&self.app_paths);
        if let Some(lib) = self.lib.as_ref() {
            let org = Organize { extras: Some(&self.game_extras), ..Default::default() };
            state.data.facts = Facts::build(lib, &self.categories, |g| org.platform(g));
        }
        self.console.customize = Some(CustomizeNav { group: 0, list: ListNav::default(), state, peek: false });
    }

    pub(super) fn render_console_customize(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> Option<AnyElement> {
        let nav = self.console.customize.as_mut().filter(|n| !n.peek)?;
        let rows = theme_rows::group_rows(nav.group, &nav.state.ctx());
        let groups: Vec<SharedString> = PANEL.iter().map(|(g, _)| SharedString::from(tr_(g))).collect();
        let mut strip = div().flex().items_center().gap(u(14.)).child(kit::glyph(Btn::LB));
        for (i, g) in groups.iter().enumerate() {
            let on = i == nav.group;
            strip = strip.child(
                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap(u(3.))
                    .child(
                        div()
                            .text_size(u(14.))
                            .whitespace_nowrap()
                            .text_color(if on { theme::text_primary() } else { theme::text_muted() })
                            .font_weight(if on { theme::W_BOLD } else { theme::W_REGULAR })
                            .child(g.clone()),
                    )
                    .child(div().h(u(2.)).w(u(if on { 22. } else { 0. })).rounded_full().bg(theme::console_focus())),
            );
        }
        strip = strip.child(kit::glyph(Btn::RB));
        let notice = nav
            .state
            .notice
            .clone()
            .map(|(text, error)| kit::body_text(text).text_color(if error { theme::danger_text() } else { theme::text_muted() }));
        let list = nav.list.render("console-customize-rows", &rows, true).flex_1().min_h_0();
        let panel = div()
            .absolute()
            .top_0()
            .bottom_0()
            .right_0()
            .w(u(470.))
            .flex()
            .flex_col()
            .gap(u(10.))
            .px(u(18.))
            .pt(u(26.))
            .pb(u(70.))
            .bg(theme::console_sidebar())
            .border_l_1()
            .border_color(kit::ink(0.08))
            .shadow(kit::shadow(0.5, 0., 40.))
            .child(div().px(u(8.)).child(kit::title(tr!("Customize"), 30.)))
            .child(div().px(u(8.)).child(strip))
            .child(list)
            .children(notice);
        Some(motion::enter("console-customize", panel, 280, 40., 0.))
    }

    /// `false` lets the shell close it on B.
    pub(super) fn customize_pad(&mut self, pad: Pad, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some(nav) = self.console.customize.as_mut() else { return false };
        if nav.peek {
            nav.peek = false;
            return true;
        }
        match pad {
            Pad::Y => {
                nav.peek = true;
                return true;
            }
            Pad::LB | Pad::RB => {
                let n = PANEL.len();
                nav.group = if pad == Pad::RB { (nav.group + 1) % n } else { (nav.group + n - 1) % n };
                nav.state.moving = false;
                nav.list.reset();
                return true;
            }
            _ => {}
        }
        let rows = theme_rows::group_rows(nav.group, &nav.state.ctx());
        let focused = nav.list.focused(&rows).map(|r| r.id.to_string());
        match theme_rows::extra_pad(&mut nav.state, pad, focused.as_deref()) {
            Extra::Handled => return true,
            Extra::Run(cmd, delta) => {
                if theme_rows::change(self, &cmd, cx) && delta != 0 {
                    if let Some(nav) = self.console.customize.as_mut() {
                        nav.list.focus = nav.list.focus.saturating_add_signed(delta as isize);
                    }
                }
                return true;
            }
            Extra::Pass => {}
        }
        if matches!(pad, Pad::B | Pad::Start) {
            return false;
        }
        if let outcome @ (Outcome::Changed(..) | Outcome::Activate(_)) = nav.list.on_pad(pad, &rows) {
            theme_rows::on_outcome(self, outcome, window, cx);
        }
        true
    }

    pub(super) fn customize_hints(&self) -> Vec<Hint> {
        let Some(nav) = self.console.customize.as_ref() else { return Vec::new() };
        if nav.peek {
            return vec![hint(Btn::Y, tr!("Show the panel")), hint(Btn::B, tr!("Close"))];
        }
        let rows = theme_rows::group_rows(nav.group, &nav.state.ctx());
        let focused = nav.list.focused(&rows).map(|r| r.id.to_string());
        let extra = theme_rows::extra_hints(&nav.state, focused.as_deref());
        if nav.state.moving {
            return extra;
        }
        let mut h = vec![hint(Btn::LeftRight, tr!("Change")), hint(Btn::Bumpers, tr!("Group"))];
        h.extend(extra);
        h.push(hint(Btn::Y, tr!("See the screen")));
        h.push(hint(Btn::B, tr!("Close")));
        h
    }
}
