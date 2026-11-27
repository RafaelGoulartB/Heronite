//! The media row of Home and Audio: what a media player plays now (its
//! cover, the track, the artist and the app) and a round button each for the
//! previous track, play / pause and the next one. ◀ ▶ choose a button and A
//! runs it, as on Home's "Capture" row. With several players, dots under the
//! row show which one it is: ◀ ▶ past the first or last button, the right
//! stick ← → and a click on a dot go to another, Y to the next (round).
//! The one playing comes first, else the one used last. The players are read
//! when the panel opens and every couple of seconds while Home or Audio shows
//! (`heronite_overlay::media`: MPRIS on Linux, the system media controls on
//! Windows).

use std::path::PathBuf;
use std::time::Duration;

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::{AppPaths, DataPaths};
use heronite_i18n::{tr, trc};
use heronite_images::{ImageRef, Variant};
use heronite_overlay::media::{self, Action, Art, Playback, Player};

use super::{OverlayPanel, Tab};
use crate::console::kit::{self, u};
use crate::console::list::Row;
use crate::console::pointer::{self, Clickable};
use crate::console::{hint, Btn, Hint, Pad};
use crate::icons::Icon;
use crate::theme;
use crate::ui::icon;

pub(super) const MEDIA: &str = "media";
/// How often the players are read again while Home or Audio shows.
const EVERY: Duration = Duration::from_secs(2);
/// After a command, a player takes a moment to say what it plays now.
const SETTLE: Duration = Duration::from_millis(400);
/// The cover's side, in design px.
const ART: f32 = 44.;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Button {
    Previous,
    PlayPause,
    Next,
}

const BUTTONS: [Button; 3] = [Button::Previous, Button::PlayPause, Button::Next];
/// Play / pause: what ◀ ▶ start from.
const MIDDLE: usize = 1;

impl Button {
    fn action(self) -> Action {
        match self {
            Button::Previous => Action::Previous,
            Button::PlayPause => Action::PlayPause,
            Button::Next => Action::Next,
        }
    }

    fn enabled(self, p: &Player) -> bool {
        match self {
            Button::Previous => p.can_previous,
            Button::PlayPause => p.can_play_pause,
            Button::Next => p.can_next,
        }
    }

    fn icon(self, p: &Player) -> Icon {
        match self {
            Button::Previous => Icon::SkipPrevious,
            Button::PlayPause if p.playback == Playback::Playing => Icon::Pause,
            Button::PlayPause => Icon::PlayArrow,
            Button::Next => Icon::SkipNext,
        }
    }

    fn label(self, p: &Player) -> &'static str {
        match self {
            Button::Previous => trc!("media", "Previous"),
            Button::PlayPause if p.playback == Playback::Playing => trc!("media", "Pause"),
            Button::PlayPause => trc!("media", "Play"),
            Button::Next => trc!("media", "Next"),
        }
    }
}

/// The panel's players and what the media row has chosen.
pub(super) struct Media {
    players: Vec<Player>,
    /// The player chosen (another page, a command), while it is there; else
    /// the first.
    chosen: Option<String>,
    /// The button ◀ ▶ chose (in [`BUTTONS`]).
    button: usize,
    reading: bool,
    /// Where Windows' thumbnails are kept.
    art_dir: PathBuf,
    /// Made-up players (`HERONITE_DEBUG_OVERLAY_MEDIA`): never read again
    /// nor sent a command.
    made_up: bool,
    /// Commands reach the players under the other debug hooks too.
    live: bool,
}

impl Media {
    /// `HERONITE_DEBUG_OVERLAY_MEDIA`: `live` reads the real players and
    /// sends them the panel's presses (under the other debug hooks nothing is
    /// sent, as with the volume); `playing`, `paused`, `browser`, `two`,
    /// `three` or `none` show made-up players instead (the cover from
    /// `HERONITE_DEBUG_OVERLAY_MEDIA_ART`, a file or a URL), changed in place.
    pub(super) fn new(paths: &DataPaths) -> Self {
        let spec = crate::env::value("HERONITE_DEBUG_OVERLAY_MEDIA").unwrap_or_default();
        let made_up = made_up(&spec);
        Media {
            made_up: made_up.is_some(),
            players: made_up.unwrap_or_default(),
            chosen: None,
            button: MIDDLE,
            reading: false,
            art_dir: AppPaths::for_data(paths).cache.join("overlay-media"),
            live: spec == "live",
        }
    }
}

/// The made-up tracks a made-up player goes through.
const TRACKS: [(&str, &str); 3] =
    [("Neon Skyline", "The Pixel Cartographers"), ("Save Point", "Lena & the Respawns"), ("Final Boss (Lo-fi Mix)", "Cozy Tavern")];

fn made_up(spec: &str) -> Option<Vec<Player>> {
    let art =
        crate::env::value("HERONITE_DEBUG_OVERLAY_MEDIA_ART").map(
            |a| {
                if a.starts_with("http") {
                    Art::Url(a)
                } else {
                    Art::File(a.into())
                }
            },
        );
    let music = |playback| Player {
        id: "debug.music".into(),
        app: "Spotify".into(),
        title: TRACKS[0].0.into(),
        artist: TRACKS[0].1.into(),
        album: "Overworld Sessions".into(),
        playback,
        can_play_pause: true,
        can_next: true,
        can_previous: true,
        art: art.clone(),
    };
    let browser = Player {
        id: "debug.browser".into(),
        app: "Firefox".into(),
        title: "Speedrun commentary, part 3".into(),
        artist: "Any% Club".into(),
        album: String::new(),
        playback: Playback::Playing,
        can_play_pause: true,
        can_next: false,
        can_previous: false,
        art: None,
    };
    match spec {
        "playing" => Some(vec![music(Playback::Playing)]),
        "paused" => Some(vec![music(Playback::Paused)]),
        "browser" => Some(vec![browser]),
        "two" => Some(vec![music(Playback::Playing), Player { playback: Playback::Paused, ..browser }]),
        "three" => Some(vec![
            music(Playback::Playing),
            Player { playback: Playback::Paused, ..browser.clone() },
            Player {
                id: "debug.podcast".into(),
                app: "VLC".into(),
                title: "Patch notes, episode 12".into(),
                artist: "Respawn Radio".into(),
                playback: Playback::Paused,
                can_next: true,
                can_previous: true,
                ..browser
            },
        ]),
        "none" => Some(Vec::new()),
        _ => None,
    }
}

impl OverlayPanel {
    /// The player the media row shows.
    pub(super) fn media_player(&self) -> Option<&Player> {
        let players = &self.media.players;
        self.media.chosen.as_ref().and_then(|id| players.iter().find(|p| &p.id == id)).or(players.first())
    }

    /// The media row, while a player is there.
    pub(super) fn media_row(&self) -> Option<Row> {
        let p = self.media_player()?;
        let label = [&p.title, &p.app].into_iter().find(|s| !s.is_empty()).cloned().unwrap_or_else(|| tr!("Media player").to_string());
        let mut line: Vec<&str> = Vec::new();
        if p.playback == Playback::Paused {
            line.push(tr!("Paused"));
        }
        if !p.artist.is_empty() {
            line.push(&p.artist);
        }
        // Without a title the app's name is the label.
        if !p.title.is_empty() && !p.app.is_empty() {
            line.push(&p.app);
        }
        Some(Row::action(MEDIA, label).icon(Icon::MusicNote).desc(line.join(" · ")))
    }

    /// The button chosen, or the nearest one the player takes.
    fn media_button(&self, p: &Player) -> usize {
        let at = self.media.button.min(BUTTONS.len() - 1);
        if BUTTONS[at].enabled(p) {
            return at;
        }
        [MIDDLE, 0, 2].into_iter().find(|i| BUTTONS[*i].enabled(p)).unwrap_or(at)
    }

    /// The next button the player takes after (or before) the chosen one:
    /// ◀ ▶ skip the others.
    fn media_step(&self, p: &Player, forward: bool) -> usize {
        let at = self.media_button(p);
        let mut i = at;
        loop {
            i = match (forward, i) {
                (true, i) if i + 1 < BUTTONS.len() => i + 1,
                (false, i) if i > 0 => i - 1,
                _ => return at,
            };
            if BUTTONS[i].enabled(p) {
                return i;
            }
        }
    }

    /// A press on the media row: ◀ ▶ choose (past the first or last button,
    /// the player before or after), A runs the chosen button, the right stick
    /// ← → goes to the player before or after, Y to the next (round).
    pub(super) fn media_pad(&mut self, pad: Pad, cx: &mut Context<Self>) -> bool {
        let Some(player) = self.media_player().cloned() else { return false };
        match pad {
            Pad::Left | Pad::Right => {
                let forward = pad == Pad::Right;
                let (at, to) = (self.media_button(&player), self.media_step(&player, forward));
                if to != at {
                    self.media.button = to;
                } else {
                    // From the edge it leaves by: ▶ past ⏭ lands on the next one's ⏮.
                    self.media_turn(forward, if forward { 0 } else { BUTTONS.len() - 1 });
                }
            }
            Pad::ScrollLeft | Pad::ScrollRight => self.media_turn(pad == Pad::ScrollRight, MIDDLE),
            Pad::A => {
                let b = BUTTONS[self.media_button(&player)];
                if b.enabled(&player) {
                    self.run_media(&player, b.action(), cx);
                }
            }
            Pad::Y if self.media.players.len() > 1 => {
                let i = self.media.players.iter().position(|p| p.id == player.id).unwrap_or(0);
                self.media.chosen = Some(self.media.players[(i + 1) % self.media.players.len()].id.clone());
            }
            _ => return false,
        }
        true
    }

    /// Shows the player after (or before) the one shown, with `button`
    /// chosen; nothing past the last (or the first).
    fn media_turn(&mut self, forward: bool, button: usize) {
        let Some(id) = self.media_player().map(|p| p.id.clone()) else { return };
        let at = self.media.players.iter().position(|p| p.id == id).unwrap_or(0);
        let to = if forward { at.checked_add(1) } else { at.checked_sub(1) };
        if let Some(next) = to.and_then(|i| self.media.players.get(i)) {
            self.media.chosen = Some(next.id.clone());
            self.media.button = button;
        }
    }

    pub(super) fn media_hints(&self) -> Vec<Hint> {
        let Some(p) = self.media_player() else { return Vec::new() };
        let b = BUTTONS[self.media_button(p)];
        let mut h = vec![hint(Btn::LeftRight, tr!("Choose")), hint(Btn::A, b.label(p))];
        // Five hints would take two lines.
        if self.media.players.len() > 1 {
            h.push(hint(Btn::Y, tr!("Other player")));
            h.push(hint(Btn::B, tr!("Close")));
        } else {
            h.push(hint(Btn::B, tr!("Close")));
            h.push(hint(Btn::Bumpers, tr!("Tabs")));
        }
        h
    }

    fn run_media(&mut self, player: &Player, action: Action, cx: &mut Context<Self>) {
        self.error = None;
        // Kept on screen even when another player then plays first.
        self.media.chosen = Some(player.id.clone());
        let made_up = self.media.made_up;
        if let Some(p) = self.media.players.iter_mut().find(|p| p.id == player.id) {
            // Shown at once; the next read says what the player did.
            if action == Action::PlayPause {
                p.playback = if p.playback == Playback::Playing { Playback::Paused } else { Playback::Playing };
            } else if made_up {
                let at = TRACKS.iter().position(|t| t.0 == p.title).unwrap_or(0);
                let next = if action == Action::Next { (at + 1) % TRACKS.len() } else { (at + TRACKS.len() - 1) % TRACKS.len() };
                (p.title, p.artist) = (TRACKS[next].0.into(), TRACKS[next].1.into());
            }
        }
        if made_up || (crate::env::value("HERONITE_DEBUG_OVERLAY").is_some() && !self.media.live) {
            log::info!("overlay debug: media {action:?} skipped");
            return;
        }
        media::used(player);
        let player = player.clone();
        cx.spawn(async move |this, cx| {
            let res = cx.background_spawn(async move { media::control(&player, action) }).await;
            cx.background_executor().timer(SETTLE).await;
            this.update(cx, |this, cx| {
                if let Err(e) = res {
                    log::warn!("overlay: media {action:?}: {e}");
                    this.error = Some(tr!("Could not control the player: {err}", err = e).into());
                }
                this.load_media(cx);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Reads the players now and again every [`EVERY`] while Home or Audio
    /// shows, until the panel closes.
    pub(super) fn watch_media(&mut self, cx: &mut Context<Self>) {
        if self.media.made_up {
            return;
        }
        self.load_media(cx);
        cx.spawn(async move |this, cx| loop {
            cx.background_executor().timer(EVERY).await;
            let alive = this
                .update(cx, |this, cx| {
                    if matches!(this.tab(), Tab::Home | Tab::Audio) {
                        this.load_media(cx);
                    }
                })
                .is_ok();
            if !alive {
                break;
            }
        })
        .detach();
    }

    /// One read of the players (none while another runs). The focus stays on
    /// its row when the media row comes or goes above it.
    pub(super) fn load_media(&mut self, cx: &mut Context<Self>) {
        if self.media.reading || self.media.made_up {
            return;
        }
        self.media.reading = true;
        let dir = self.media.art_dir.clone();
        cx.spawn(async move |this, cx| {
            let players = cx.background_spawn(async move { media::read(&dir).unwrap_or_default() }).await;
            this.update(cx, |this, cx| {
                this.media.reading = false;
                if this.media.players == players {
                    return;
                }
                let focused = this.nav.focused(&this.rows()).map(|r| r.id.clone());
                this.media.players = players;
                if let Some(id) = focused {
                    if let Some(i) = this.rows().iter().position(|r| r.id == id) {
                        this.nav.focus = i;
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// The shown player's cover, once loaded (made before the rows are
    /// drawn: it needs the image store).
    pub(super) fn media_art(&mut self, window: &Window, cx: &mut Context<Self>) -> Option<ImageSource> {
        let p = self.media_player()?;
        let art = p.art.clone()?;
        let px = (ART * kit::scale() * window.scale_factor()).round() as u32;
        let (key, chain) = match art {
            Art::File(f) => (f.display().to_string(), ImageRef::File(f)),
            // Any page can name it (a browser passes the tab's artwork on).
            Art::Url(url) => (url.clone(), ImageRef::Url(heronite_images::public::marked(&url))),
        };
        // With the title: a browser writes each track's artwork to the same file.
        let id: SharedString = format!("media:{key}:{}:{px}", p.title).into();
        self.icons(cx).get_or_request(&id, || vec![chain], px, px, Variant::Cover, 30).map(ImageSource::Render)
    }

    /// The focus goes to the media row (a click on it).
    fn media_focus(&mut self) {
        if let Some(i) = self.rows().iter().position(|r| r.id.as_ref() == MEDIA) {
            self.nav.focus = i;
        }
    }

    /// A click on button `i`: the focus goes to the row and the button runs.
    fn media_click(&mut self, i: usize, cx: &mut Context<Self>) {
        self.media_focus();
        self.media.button = i;
        let Some(p) = self.media_player().cloned() else { return };
        if BUTTONS[i].enabled(&p) {
            self.run_media(&p, BUTTONS[i].action(), cx);
        }
    }

    /// With several players, a dot each under the row (the one shown is
    /// longer); a click on one shows that player.
    fn media_dots(&self, shown: &Player, fg: Hsla, clickable: bool) -> Option<Div> {
        let players = &self.media.players;
        if players.len() < 2 {
            return None;
        }
        Some(div().flex().justify_center().pb(u(6.)).children(players.iter().map(|p| {
            let on = p.id == shown.id;
            let id = p.id.clone();
            let pick = clickable.then(|| {
                self.click(move |this, _, _| {
                    this.media_focus();
                    this.media.chosen = Some(id.clone());
                    this.media.button = MIDDLE;
                })
            });
            // The room around it takes the click too.
            div()
                .px(u(3.))
                .py(u(3.))
                .when_some(pick, |d, pick| d.on_mouse_down(MouseButton::Left, pick))
                .child(div().h(u(6.)).w(u(if on { 18. } else { 6. })).rounded_full().bg(Hsla { a: if on { 0.9 } else { 0.3 }, ..fg }))
        })))
    }

    /// The media row: the cover, the track and its line, then the buttons.
    /// `route` brings the focus to the row (`None`: the rows take no clicks).
    pub(super) fn render_media(&self, r: &Row, focused: bool, route: Option<&[Pad]>, art: Option<ImageSource>) -> Div {
        let Some(p) = self.media_player() else { return div() };
        let at = self.media_button(p);
        let fg: Hsla = if focused { theme::console_on_focus().into() } else { theme::text_primary().into() };
        let buttons = BUTTONS.iter().enumerate().map(|(i, b)| {
            let enabled = b.enabled(p);
            // The presses to it: ◀ ▶ skip the buttons the player does not take.
            let between = if i > at { at + 1..i + 1 } else { i..at };
            let steps = between.filter(|j| BUTTONS[*j].enabled(p)).count();
            let to: Vec<Pad> = route
                .filter(|_| enabled)
                .map(|route| route.iter().copied().chain(pointer::repeat(if i > at { Pad::Right } else { Pad::Left }, steps)).collect())
                .unwrap_or_default();
            // Run straight away, not as ▶ presses from the button chosen: a
            // hover's presses may not be drawn yet, and ▶ past ⏭ turns the page.
            let click = route.filter(|_| enabled).map(|_| self.click(move |this, _, cx| this.media_click(i, cx)));
            let (bg, ink): (Hsla, Hsla) = match (focused, i == at && enabled) {
                (true, true) => (fg, theme::console_focus().into()),
                (true, false) => (theme::black_a(0.08), fg),
                (false, _) => (theme::white_a(0.1), fg),
            };
            div()
                .flex_none()
                .size(u(40.))
                .rounded_full()
                .flex()
                .items_center()
                .justify_center()
                .bg(bg)
                // Faded, and its clicks do not reach the row (which runs the chosen one).
                .when(!enabled, |d| d.opacity(0.35).on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation()))
                .on_hover_pads(to)
                .when_some(click, |d, click| d.on_mouse_down(MouseButton::Left, click))
                .child(icon(b.icon(p), u(20.), ink))
        });
        let tile = div().flex_none().size(u(ART)).rounded(u(10.)).overflow_hidden().flex().items_center().justify_center().bg(if focused {
            theme::black_a(0.1)
        } else {
            theme::white_a(0.08)
        });
        let tile = match art {
            Some(src) => tile.child(img(src).size_full().object_fit(ObjectFit::Cover)),
            None => tile.child(icon(if p.is_browser() { Icon::Language } else { Icon::MusicNote }, u(22.), fg)),
        };
        let sub: Hsla = if focused { Hsla { a: 0.7, ..fg } } else { theme::text_muted().into() };
        let line = div()
            .flex()
            .items_center()
            .gap(u(12.))
            .child(tile)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .py(u(10.))
                    .child(div().text_size(u(18.)).text_color(fg).truncate().child(r.label.clone()))
                    .children(
                        r.desc
                            .clone()
                            .filter(|d| !d.is_empty())
                            .map(|d| div().text_size(u(13.)).line_height(u(18.)).truncate().text_color(sub).child(d)),
                    ),
            )
            .child(div().flex().flex_none().gap(u(8.)).children(buttons));
        let row = kit::row_base(focused)
            .flex_col()
            .items_stretch()
            .justify_center()
            .gap(u(0.))
            // The cover starts where the rows' icons do; the buttons end
            // where the "Capture" row's do.
            .pl(u(12.))
            .child(line)
            .children(self.media_dots(p, fg, route.is_some()));
        // A click on the row itself runs the chosen button.
        let click: Vec<Pad> = route.map(|route| route.iter().copied().chain([Pad::A]).collect()).unwrap_or_default();
        div().on_hover_pads(route.map(<[Pad]>::to_vec).unwrap_or_default()).on_pads(click).child(row)
    }
}

#[cfg(test)]
mod tests {
    // Explicit imports: `use super::*` would bring in gpui's `test` macro and shadow `#[test]`.
    use heronite_overlay::media::Playback;

    use super::made_up;

    #[test]
    fn made_up_players() {
        assert_eq!(made_up("playing").unwrap()[0].playback, Playback::Playing);
        assert_eq!(made_up("two").unwrap().len(), 2);
        assert!(made_up("none").unwrap().is_empty());
        assert!(made_up("").is_none() && made_up("live").is_none());
        let browser = &made_up("browser").unwrap()[0];
        assert!(browser.is_browser() && !browser.can_next && !browser.can_previous);
    }
}
