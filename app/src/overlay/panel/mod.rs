//! The overlay panel: the game and the session on top, tabs switched with
//! LB / RB (Home, Screenshots, Notes, Audio, Controller), rows in console mode's
//! look and the button hints at the bottom. It only takes controller presses
//! (the service forwards them: the window never has the keyboard, so the
//! game keeps it).

mod data;
mod mixer;
mod notes;
mod rows;
mod shots;

use std::time::Duration;

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::DataPaths;
use heronite_i18n::tr;
use heronite_overlay::audio::AudioState;
use heronite_overlay::input::{Button, Controller};

use self::data::Data;
use super::{GameNow, OverlayService};
use crate::console::kit::{self, u};
use crate::console::list::{self, ListNav, Row};
use crate::console::osk::{self, Osk};
use crate::console::{hint, Btn, Hint, Pad, PadTarget};
use crate::icons::Icon;
use crate::theme;
use crate::ui::icon;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tab {
    Home,
    Shots,
    Notes,
    Audio,
    Controller,
}

const TABS: [Tab; 5] = [Tab::Home, Tab::Shots, Tab::Notes, Tab::Audio, Tab::Controller];
/// Without a game there is nothing to capture.
const TABS_NO_GAME: [Tab; 3] = [Tab::Home, Tab::Audio, Tab::Controller];

impl Tab {
    fn label(self) -> String {
        match self {
            Tab::Home => tr!("Home").into(),
            Tab::Shots => tr!("Screenshots").into(),
            Tab::Notes => tr!("Notes").into(),
            Tab::Audio => tr!("Audio").into(),
            Tab::Controller => tr!("Controller").into(),
        }
    }

    fn icon(self) -> Icon {
        match self {
            Tab::Home => Icon::Home,
            Tab::Shots => Icon::Collections,
            Tab::Notes => Icon::Notes,
            Tab::Audio => Icon::VolumeUp,
            Tab::Controller => Icon::SportsEsports,
        }
    }
}

/// A question before something that cannot be undone from here.
#[derive(Clone, Debug, PartialEq)]
enum Confirm {
    Quit,
    /// Bluetooth address and name of the controller.
    Disconnect(String, String),
}

pub struct OverlayPanel {
    /// Never read while it calls in (it is often updating): actions go
    /// through [`OverlayPanel::with_service`], deferred.
    service: WeakEntity<OverlayService>,
    paths: DataPaths,
    /// Percent per press of ◀ ▶ on the volume rows.
    volume_step: i64,
    /// `None`: opened without a game running (Settings → Overlay allows it).
    game: Option<GameNow>,
    controllers: Vec<Controller>,
    scale: f32,
    tab: usize,
    nav: ListNav,
    data: Data,
    confirm: Option<Confirm>,
    confirm_nav: ListNav,
    /// Focused capture in the Screenshots grid (`None` = the rows have it).
    grid: Option<usize>,
    grid_scroll: ScrollHandle,
    /// Capture shown full size.
    viewer: Option<usize>,
    /// Shown under the rows after a failed action.
    error: Option<SharedString>,
    /// Opened from the keyboard: it has the keyboard and shows keys.
    keyboard: bool,
    focus: FocusHandle,
    /// The on-screen keyboard over the panel, and what it writes.
    osk: Option<(Entity<Osk>, Subscription)>,
    writing: Option<notes::Writing>,
    /// Seconds a replay keeps, when the replay is on and a game runs.
    replay: Option<u32>,
}

/// Controller button to console mode's press.
pub fn pad_of(b: Button) -> Pad {
    match b {
        Button::A => Pad::A,
        Button::B => Pad::B,
        Button::X => Pad::X,
        Button::Y => Pad::Y,
        Button::LB => Pad::LB,
        Button::RB => Pad::RB,
        Button::LT => Pad::LT,
        Button::RT => Pad::RT,
        Button::Start | Button::Guide => Pad::Start,
        Button::Select => Pad::Select,
        Button::Up => Pad::Up,
        Button::Down => Pad::Down,
        Button::Left => Pad::Left,
        Button::Right => Pad::Right,
    }
}

impl OverlayPanel {
    pub fn new(
        service: WeakEntity<OverlayService>,
        paths: DataPaths,
        volume_step: u8,
        game: Option<GameNow>,
        controllers: Vec<Controller>,
        keyboard: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let focus = cx.focus_handle();
        if keyboard {
            window.focus(&focus, cx);
        }
        let replay = game.as_ref().and_then(|_| OverlayService::replay_seconds(cx));
        let mut this = Self {
            service,
            paths: paths.clone(),
            volume_step: i64::from(volume_step),
            game,
            controllers,
            scale: super::window::scale(cx),
            tab: 0,
            nav: ListNav::default(),
            data: Data::default(),
            confirm: None,
            confirm_nav: ListNav::default(),
            grid: None,
            grid_scroll: ScrollHandle::new(),
            viewer: None,
            error: None,
            keyboard,
            focus,
            osk: None,
            writing: None,
            replay,
        };
        this.load_all(paths, cx);
        // The clock and the session time.
        cx.spawn(async move |this, cx| loop {
            cx.background_executor().timer(Duration::from_secs(20)).await;
            if this.update(cx, |_, cx| cx.notify()).is_err() {
                break;
            }
        })
        .detach();
        this
    }

    pub fn set_controllers(&mut self, list: Vec<Controller>, cx: &mut Context<Self>) {
        self.controllers = list;
        self.load_batteries(cx);
    }

    fn tabs(&self) -> &'static [Tab] {
        if self.game.is_some() {
            &TABS
        } else {
            &TABS_NO_GAME
        }
    }

    fn tab(&self) -> Tab {
        self.tabs()[self.tab]
    }

    fn with_service(&self, cx: &mut Context<Self>, f: impl FnOnce(&mut OverlayService, &mut Context<OverlayService>) + 'static) {
        if let Some(s) = self.service.upgrade() {
            // Deferred: the service may be closing this very window.
            cx.defer(move |cx| s.update(cx, f));
        }
    }

    fn close(&self, cx: &mut Context<Self>) {
        self.with_service(cx, |s, cx| s.close_panel(cx));
    }

    pub fn on_pad(&mut self, pad: Pad, window: &mut Window, cx: &mut Context<Self>) -> bool {
        crate::console::motion::press();
        cx.notify();
        if let Some(osk) = self.osk.as_ref().map(|(k, _)| k.clone()) {
            osk.update(cx, |k, cx| k.on_pad(pad, window, cx));
            return true;
        }
        if let Some(i) = self.viewer {
            if pad == Pad::Y {
                self.capture_note(i, cx);
            } else {
                self.viewer_pad(pad);
            }
            return true;
        }
        if let Some(confirm) = self.confirm.clone() {
            self.confirm_pad(confirm, pad, cx);
            return true;
        }
        match pad {
            Pad::LB | Pad::RB => {
                let n = self.tabs().len();
                self.tab = if pad == Pad::RB { (self.tab + 1) % n } else { (self.tab + n - 1) % n };
                self.nav.reset();
                self.grid = None;
                self.error = None;
                return true;
            }
            Pad::B => {
                self.close(cx);
                return true;
            }
            _ => {}
        }
        if let Some(i) = self.grid {
            if pad == Pad::Y {
                self.capture_note(i, cx);
            } else {
                self.grid_pad(pad);
            }
            return true;
        }
        let rows = self.rows();
        if let Some(id) = self.nav.focused(&rows).map(|r| r.id.clone()) {
            if mixer::is_mixer(&id) && self.mixer_pad(&id, pad, cx) {
                return true;
            }
        }
        if pad == Pad::Y && self.tab() == Tab::Notes && self.data.journal.is_some() {
            self.with_service(cx, |s, cx| s.mark_moment(cx));
            return true;
        }
        let outcome = self.nav.on_pad(pad, &rows);
        self.apply(outcome, cx);
        true
    }

    fn confirm_pad(&mut self, confirm: Confirm, pad: Pad, cx: &mut Context<Self>) {
        if pad == Pad::B {
            self.confirm = None;
            return;
        }
        let rows = confirm_rows(&confirm);
        if let list::Outcome::Activate(id) = self.confirm_nav.on_pad(pad, &rows) {
            self.confirm = None;
            if id.as_ref() != "yes" {
                return;
            }
            match confirm {
                Confirm::Quit => {
                    if let Some(app_name) = self.game.as_ref().map(|g| g.game.app_name.clone()) {
                        self.with_service(cx, move |s, cx| s.stop_game(&app_name, cx));
                    }
                }
                Confirm::Disconnect(address, _) => self.disconnect(address, cx),
            }
        }
    }

    fn ask(&mut self, confirm: Confirm) {
        self.confirm = Some(confirm);
        // Cancel has the focus: A twice in a row does not quit by accident.
        self.confirm_nav = ListNav::default();
        self.confirm_nav.focus = 1;
    }

    fn hints(&self) -> Vec<Hint> {
        if self.osk.is_some() {
            return osk::keyboard_hints();
        }
        let notes = self.data.journal.is_some();
        if self.viewer.is_some() {
            let mut h = vec![hint(Btn::LeftRight, tr!("Previous / next"))];
            if notes {
                h.push(hint(Btn::Y, tr!("Note")));
            }
            h.push(hint(Btn::B, tr!("Back")));
            return h;
        }
        if self.confirm.is_some() {
            return vec![hint(Btn::A, tr!("Select")), hint(Btn::B, tr!("Cancel"))];
        }
        let rows = self.rows();
        if self.grid.is_none() && self.nav.focused(&rows).is_some_and(|r| mixer::is_mixer(&r.id)) {
            let outputs = self.data.audio.as_ref().and_then(|a| a.as_ref()).map_or(0, |a| a.outputs.len());
            let mut h = vec![hint(Btn::LeftRight, tr!("Adjust")), hint(Btn::A, tr!("Mute"))];
            if outputs > 1 {
                h.push(hint(Btn::Y, tr!("Output")));
            }
            h.push(hint(Btn::B, tr!("Close")));
            return h;
        }
        if self.grid.is_some() && notes {
            return vec![hint(Btn::A, tr!("View")), hint(Btn::Y, tr!("Note")), hint(Btn::B, tr!("Close")), hint(Btn::Bumpers, tr!("Tabs"))];
        }
        if self.tab() == Tab::Notes && notes {
            return vec![
                hint(Btn::A, tr!("Select")),
                hint(Btn::Y, tr!("Mark a moment")),
                hint(Btn::B, tr!("Close")),
                hint(Btn::Bumpers, tr!("Tabs")),
            ];
        }
        vec![hint(Btn::A, tr!("Select")), hint(Btn::B, tr!("Close")), hint(Btn::Bumpers, tr!("Tabs"))]
    }

    fn render_header(&self) -> Div {
        let (title, line) = match &self.game {
            Some(game) => {
                let session = duration_label(game.game.started.elapsed().as_secs() / 60);
                let line = match self.data.total_minutes {
                    Some(total) => {
                        let total = duration_label(total);
                        tr!("Session {session} · Total {total}", session, total)
                    }
                    None => tr!("Session {session}", session),
                };
                (game.title(), line)
            }
            None => ("Heronite".to_string(), tr!("No game running").to_string()),
        };
        let battery = self.data.controllers.iter().find_map(|(_, b)| *b).map(|b| format!("{}%", b.percent));
        let info = |ic: Icon, text: String| {
            div()
                .flex()
                .items_center()
                .gap(u(6.))
                .min_w_0()
                .child(icon(ic, u(15.), theme::text_muted()))
                .child(div().truncate().child(text))
        };
        div()
            .flex()
            .flex_col()
            .gap(u(8.))
            .px(u(24.))
            .pt(u(14.))
            .pb(u(16.))
            .border_b_1()
            .border_color(theme::white_a(0.08))
            .child(kit::title(title, 24.).truncate())
            .child(div().text_size(u(14.)).text_color(theme::text_muted()).child(line))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(u(16.))
                    .text_size(u(13.))
                    .text_color(theme::text_muted())
                    .child(info(Icon::AccessTime, chrono::Local::now().format("%H:%M").to_string()))
                    .children(battery.map(|b| info(Icon::SportsEsports, b))),
            )
    }

    fn render_tabs(&self) -> Div {
        div()
            .flex()
            .items_center()
            .gap(u(6.))
            .mx(u(16.))
            .pt(u(16.))
            .child(self.glyph(Btn::LB))
            .children(self.tabs().iter().enumerate().map(|(i, t)| {
                let on = i == self.tab;
                let color: Hsla = if on { theme::text_primary().into() } else { theme::text_muted().into() };
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap(u(4.))
                    .py(u(8.))
                    .rounded(u(10.))
                    .when(on, |d| d.bg(theme::white_a(0.1)))
                    .child(icon(t.icon(), u(20.), color))
                    .child(div().text_size(u(12.)).text_color(color).truncate().child(t.label()))
            }))
            .child(self.glyph(Btn::RB))
    }

    fn apps_playing(&self) -> bool {
        self.data.audio.as_ref().and_then(|a| a.as_ref()).is_some_and(|a| !a.apps.is_empty())
    }

    /// The button's glyph, or its key when opened from the keyboard.
    fn glyph(&self, btn: Btn) -> Div {
        if self.keyboard {
            kit::key_glyph(btn)
        } else {
            kit::glyph(btn)
        }
    }

    fn on_key(&mut self, ev: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(osk) = self.osk.as_ref().map(|(k, _)| k.clone()) {
            if osk.update(cx, |k, cx| k.type_key(&ev.keystroke, cx)) {
                cx.stop_propagation();
                return;
            }
        }
        if let Some(pad) = crate::console::input::pad_from_key(&ev.keystroke) {
            cx.stop_propagation();
            self.on_pad(pad, window, cx);
        }
    }

    fn render_hints(&self) -> Div {
        div()
            .flex()
            .flex_none()
            .flex_wrap()
            .items_center()
            .gap(u(18.))
            .px(u(24.))
            .py(u(16.))
            .border_t_1()
            .border_color(theme::white_a(0.08))
            .children(self.hints().into_iter().map(|h| {
                div()
                    .flex()
                    .items_center()
                    .gap(u(7.))
                    .text_size(u(13.))
                    .text_color(theme::text_muted())
                    .child(self.glyph(h.btn))
                    .child(h.label)
            }))
    }

    fn render_body(&mut self) -> Div {
        let rows = self.rows();
        let list_active = self.grid.is_none() && self.confirm.is_none();
        let placeholder = AudioState::default();
        let has_mixer = rows.iter().any(|r| r.id.as_ref() == mixer::SYSTEM);
        // While the audio is read, the mixer shows empty at its full size.
        let audio = match &self.data.audio {
            Some(Some(a)) => Some((a, false)),
            None => Some((&placeholder, true)),
            Some(None) => None,
        };
        let list = match audio.filter(|_| has_mixer) {
            // With the mixer: the rows are drawn here (short lists, no scrolling).
            Some((audio, loading)) => {
                self.nav.clamp(&rows);
                let focus = list_active.then(|| rows.get(self.nav.focus).map(|r| r.id.clone())).flatten();
                let mixer_focus = focus.as_ref().filter(|id| mixer::is_mixer(id)).map(|id| id.to_string());
                let mut list = div().flex().flex_col().gap(u(4.));
                for (i, r) in rows.iter().enumerate() {
                    match r.id.as_ref() {
                        mixer::SYSTEM => {
                            let apps = if self.tab() == Tab::Home { mixer::HOME_APPS } else { mixer::AUDIO_APPS };
                            list = list.child(div().py(u(4.)).child(self.render_mixer(audio, mixer_focus.as_deref(), loading, apps)))
                        }
                        id if mixer::is_mixer(id) => {}
                        _ => list = list.child(list::render_row(r, list_active && i == self.nav.focus)),
                    }
                }
                list.into_any_element()
            }
            // Long lists (several controllers) scroll inside the panel.
            None if self.tab() == Tab::Controller => {
                self.nav.render(("overlay-rows", self.tab), &rows, list_active).flex_1().min_h_0().into_any_element()
            }
            None => self.nav.render(("overlay-rows", self.tab), &rows, list_active).flex_none().into_any_element(),
        };
        let mut body = div().flex().flex_col().flex_1().min_h_0().overflow_hidden().px(u(12.)).pt(u(10.)).pb(u(6.)).gap(u(8.)).child(list);
        if let Some(e) = &self.error {
            body = body.child(div().px(u(22.)).text_size(u(13.)).text_color(theme::danger_text()).child(e.clone()));
        }
        match self.tab() {
            // Apps playing take the room of the recent captures (the panel's
            // height is fixed; the Screenshots tab has them).
            Tab::Home if !self.apps_playing() => body.child(self.render_recent()),
            Tab::Home => body,
            Tab::Shots => body.child(self.render_grid()),
            Tab::Notes | Tab::Audio | Tab::Controller => body,
        }
    }
}

fn confirm_rows(confirm: &Confirm) -> Vec<Row> {
    let yes = match confirm {
        Confirm::Quit => tr!("Quit game"),
        Confirm::Disconnect(..) => tr!("Disconnect"),
    };
    vec![Row::action("yes", yes).danger(), Row::action("no", tr!("Cancel"))]
}

/// "1h 12m", "12 min".
fn duration_label(minutes: u64) -> String {
    let (h, m) = heronite_data::playtime::split_minutes(minutes);
    if h == 0 {
        tr!("{m} min", m)
    } else {
        tr!("{h}h {m}m", h, m)
    }
}

impl Render for OverlayPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        kit::set_scale_value(self.scale);
        // The window has the panel's fixed size (`window::PANEL_H`): the panel
        // fills it and the tab's content takes what the header and hints leave.
        let mut panel = div()
            .relative()
            .size_full()
            .font_family(theme::font_body())
            .text_color(theme::text_primary())
            .flex()
            .flex_col()
            .rounded(u(24.))
            .bg(theme::overlay_fill())
            .border_1()
            .border_color(theme::white_a(0.1))
            .overflow_hidden()
            .child(self.render_tabs())
            .child(self.render_header())
            .child(self.render_body())
            .child(self.render_hints())
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::on_key));
        if let Some(confirm) = &self.confirm {
            let (title, message) = match confirm {
                Confirm::Quit => (
                    tr!("Quit {game}?", game = self.game.as_ref().map(|g| g.title()).unwrap_or_default()),
                    tr!("Progress you have not saved in the game is lost."),
                ),
                Confirm::Disconnect(_, name) => {
                    (tr!("Disconnect {name}?", name = name.clone()), tr!("Press its Guide button to connect it again."))
                }
            };
            let rows = confirm_rows(confirm);
            panel = panel.child(list::simple_dialog(&mut self.confirm_nav, "overlay-confirm", title, message, None, &rows));
        }
        if let Some(viewer) = self.render_viewer() {
            panel = panel.child(viewer);
        }
        if let Some((osk, _)) = &self.osk {
            panel = panel.child(osk.clone());
        }
        panel
    }
}
