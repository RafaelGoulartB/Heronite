//! Short notices in a top corner of the screen (screenshot saved, the
//! volume level), shown by the shortcuts that do not open the panel. A new
//! notice replaces the one on screen and restarts its timer.
//!
//! Achievement notices wait their turn instead: they queue (a few, then one
//! "N more achievements"), each stays longer, and one that another notice
//! interrupts shows again after it. Besides the name, an achievement's
//! notice shows what Settings → Achievements asks for and the source gives
//! ([`Details`]): its description, then badges for its rarity, points,
//! set, type and hardcore mode, on a taller card. "Achievement unlocked"
//! sits above its name.

use std::collections::{BTreeMap, VecDeque};
use std::sync::Arc;
use std::time::Duration;

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_achievements::prefs::NoticeFields;
use heronite_achievements::{Achievement, Source};
use heronite_i18n::{tr, trn};

use super::{window, OverlayService};
use crate::console::kit::{self, u};
use crate::icons::Icon;
use crate::theme;
use crate::ui::icon;

/// How long a notice stays.
const SHOWN_FOR: Duration = Duration::from_millis(1800);
/// How long an achievement's notice stays.
const ACHIEVEMENT_FOR: Duration = Duration::from_secs(5);
/// Achievement notices waiting at most; more are summed up in one.
const QUEUE_MAX: usize = 4;
/// What a description line adds to the card's height (design px).
const DESCRIPTION_H: f32 = 20.;
/// What the line of badges adds.
const CHIPS_H: f32 = 28.;

#[derive(Clone)]
pub struct Notice {
    pub icon: Icon,
    pub title: String,
    pub sub: Option<String>,
    pub error: bool,
    /// Draws a level bar (0..=100+).
    pub volume: Option<u16>,
    /// A picture in place of the icon (an achievement's).
    pub image: Option<Arc<RenderImage>>,
    /// Colour of the sub line and the border (a rare achievement's tier).
    pub accent: Option<Hsla>,
    pub duration: Duration,
    /// An achievement: it queues instead of replacing.
    pub achievement: bool,
    /// An achievement's description, on a line of its own.
    pub description: Option<String>,
    /// An achievement's badges (points, type, hardcore, set), on the last line.
    pub chips: Vec<Chip>,
}

/// A new unlock to show over the game.
#[derive(Debug, Clone)]
pub struct Unlock {
    pub achievement: Achievement,
    pub source: Source,
    /// The name of its Steam Achievement Set, when the game has more than one.
    pub group: Option<String>,
}

/// A badge on an achievement's notice.
#[derive(Debug, Clone, PartialEq)]
pub enum Chip {
    /// The share of players that have it, in its tier's colour when rare.
    Rarity(f32),
    /// "10 points", "250 XP".
    Points(String),
    /// RetroAchievements' type.
    Kind(String),
    Hardcore,
    /// The Achievement Set's name.
    Group(String),
}

impl Chip {
    pub fn text(&self) -> String {
        match self {
            Chip::Points(t) | Chip::Kind(t) | Chip::Group(t) => t.clone(),
            Chip::Hardcore => tr!("Hardcore").to_string(),
            Chip::Rarity(r) => crate::ui::achievements::percent_label(*r),
        }
    }
}

/// What an achievement's notice shows under its name: the lines `fields`
/// ask for that `unlock`'s source gives.
#[derive(Debug, Clone, PartialEq)]
pub struct Details {
    /// The rarity shown (its badge comes first), which colours a rare one.
    pub rarity: Option<f32>,
    pub description: Option<String>,
    pub chips: Vec<Chip>,
}

impl Details {
    pub fn of(unlock: &Unlock, fields: NoticeFields) -> Self {
        let a = &unlock.achievement;
        let retro = unlock.source == Source::RetroAchievements;
        let rarity = a.rarity.filter(|_| fields.rarity);
        let description = Some(a.description.trim()).filter(|d| fields.description && !d.is_empty()).map(str::to_string);
        let mut chips: Vec<Chip> = rarity.map(Chip::Rarity).into_iter().collect();
        // Only RetroAchievements (points) and Epic Games (XP) give them.
        match (a.points.filter(|_| fields.points), unlock.source) {
            (Some(n), Source::RetroAchievements) => chips.push(Chip::Points(trn!(n, "{n} point", "{n} points"))),
            (Some(n), Source::Epic) => chips.push(Chip::Points(tr!("{n} XP", n = n))),
            _ => {}
        }
        if let Some(kind) = a.kind.filter(|_| fields.kind && retro) {
            chips.push(Chip::Kind(crate::dialogs::achievements::kind_label(kind).to_string()));
        }
        if fields.hardcore && retro && a.hardcore {
            chips.push(Chip::Hardcore);
        }
        if let Some(group) = unlock.group.as_ref().filter(|g| fields.group && !g.is_empty()) {
            chips.push(Chip::Group(group.clone()));
        }
        Self { rarity, description, chips }
    }
}

impl std::fmt::Debug for Notice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Notice").field("title", &self.title).field("sub", &self.sub).field("achievement", &self.achievement).finish()
    }
}

impl Notice {
    pub fn text(icon: Icon, title: impl Into<String>, error: bool) -> Self {
        Self {
            icon,
            title: title.into(),
            sub: None,
            error,
            volume: None,
            image: None,
            accent: None,
            duration: SHOWN_FOR,
            achievement: false,
            description: None,
            chips: Vec::new(),
        }
    }

    /// Height of the card, in design px: an achievement's grows with its lines.
    pub fn height(&self) -> f32 {
        window::NOTICE_H + if self.description.is_some() { DESCRIPTION_H } else { 0. } + if self.chips.is_empty() { 0. } else { CHIPS_H }
    }

    pub fn sub(mut self, sub: impl Into<String>) -> Self {
        let sub = sub.into();
        self.sub = (!sub.is_empty()).then_some(sub);
        self
    }

    pub fn volume(level: u16) -> Self {
        let icon = if level == 0 { Icon::VolumeOff } else { Icon::VolumeUp };
        Self { volume: Some(level), ..Self::text(icon, tr!("Volume"), false) }
    }

    /// An unlock's notice: "Achievement unlocked" above its name, then the
    /// lines of [`Details`]; the border takes the rarity's colour when it is
    /// rare.
    pub fn achievement(unlock: &Unlock, fields: NoticeFields, image: Option<Arc<RenderImage>>) -> Self {
        let d = Details::of(unlock, fields);
        let accent = d
            .rarity
            .filter(|r| *r < heronite_achievements::AchievementSet::RARE_BELOW)
            .map(|r| Hsla::from(theme::rarity_colour(theme::rarity_tier(r))));
        Self {
            image,
            accent,
            duration: ACHIEVEMENT_FOR,
            achievement: true,
            description: d.description,
            chips: d.chips,
            ..Self::text(Icon::Trophy, unlock.achievement.name.clone(), false)
        }
        .sub(tr!("Achievement unlocked"))
    }

    /// The last of a long run of unlocks.
    fn more_achievements(n: usize) -> Self {
        Self {
            duration: ACHIEVEMENT_FOR,
            achievement: true,
            ..Self::text(Icon::Trophy, trn!(n, "{n} more achievement", "{n} more achievements"), false)
        }
    }
}

/// Made-up unlocks for the previews (Settings’ "Over the game" shows the
/// first two, the debug notice one or all): from Steam with a set, from
/// RetroAchievements in hardcore, from Epic Games, then plainer ones.
pub fn made_up_unlocks(icon: &str) -> Vec<Unlock> {
    let one = |source, name: &str, description: &str, rarity, points, group: Option<&str>| Unlock {
        achievement: Achievement {
            id: name.into(),
            name: name.into(),
            description: description.into(),
            icon: icon.into(),
            unlocked: true,
            unlocked_at: Some(chrono::Utc::now()),
            rarity,
            points,
            ..Default::default()
        },
        source,
        group: group.map(str::to_string),
    };
    let mut retro =
        one(Source::RetroAchievements, tr!("Untouchable"), tr!("Beat the first boss without taking a hit."), Some(1.2), Some(25), None);
    retro.achievement.kind = Some(heronite_achievements::Kind::Missable);
    retro.achievement.hardcore = true;
    vec![
        one(Source::Steam, tr!("Treasure Hunter"), tr!("Open every chest in the old harbour."), Some(9.7), None, Some("The Pale Reach")),
        retro,
        one(Source::Epic, tr!("First Steps"), tr!("Finish the tutorial."), Some(68.4), Some(250), None),
        one(Source::Steam, tr!("Speed Demon"), "", None, None, None),
        one(Source::Steam, tr!("First Steps"), "", Some(40.), None, None),
        one(Source::Steam, tr!("Treasure Hunter"), "", Some(30.), None, None),
    ]
}

pub struct NoticeWindow {
    handle: WindowHandle<NoticeView>,
    /// Bumped by every notice; the timer of an older one does nothing.
    generation: u64,
    /// Windows: the monitor it was placed on.
    #[cfg(windows)]
    screen: Option<heronite_platform::overlay_window::Screen>,
    /// The notice on screen.
    shown: Notice,
}

/// Achievement notices waiting for the screen.
#[derive(Default)]
pub struct Queue {
    waiting: VecDeque<Notice>,
    /// Unlocks past [`QUEUE_MAX`], shown as one notice at the end.
    more: usize,
    /// The last of them: when it is the only one, it shows as itself.
    last: Option<Notice>,
    /// Tickets handed out as unlocks come in, so the ones whose icon loads
    /// sooner still wait for those before them.
    next_ticket: u64,
    /// The ticket that goes in next.
    due: u64,
    /// Ready, waiting for an earlier ticket.
    ready: BTreeMap<u64, Notice>,
}

impl Queue {
    /// Holds a place for an unlock whose notice is being made.
    pub fn ticket(&mut self) -> u64 {
        self.next_ticket += 1;
        self.next_ticket - 1
    }

    /// The notice of `ticket` is ready: it goes in after the earlier ones.
    fn arrive(&mut self, ticket: u64, notice: Notice) {
        if ticket < self.due {
            // Cleared since.
            return;
        }
        self.ready.insert(ticket, notice);
        while let Some(n) = self.ready.remove(&self.due) {
            self.push(n);
            self.due += 1;
        }
    }

    fn push(&mut self, notice: Notice) {
        if self.waiting.len() < QUEUE_MAX {
            self.waiting.push_back(notice);
        } else {
            self.more += 1;
            self.last = Some(notice);
        }
    }

    fn next(&mut self) -> Option<Notice> {
        if let Some(n) = self.waiting.pop_front() {
            return Some(n);
        }
        let last = self.last.take();
        match std::mem::take(&mut self.more) {
            0 => None,
            1 => last,
            more => Some(Notice::more_achievements(more)),
        }
    }

    pub fn clear(&mut self) {
        self.waiting.clear();
        self.more = 0;
        self.last = None;
        // Notices still being made are dropped when they arrive.
        self.ready.clear();
        self.due = self.next_ticket;
    }
}

impl OverlayService {
    /// Queues an achievement's notice (in the order of `ticket`): it shows
    /// when the screen is free.
    pub(super) fn queue_achievement(&mut self, ticket: u64, notice: Notice, layer: bool, cx: &mut Context<Self>) {
        self.achievement_queue.arrive(ticket, notice);
        if self.notice.is_none() {
            if let Some(next) = self.achievement_queue.next() {
                self.show_notice(next, layer, cx);
            }
        }
    }

    pub(super) fn show_notice(&mut self, notice: Notice, layer: bool, cx: &mut Context<Self>) {
        // Windows: a topmost window would minimise a game in exclusive
        // fullscreen. Nothing shows, and the achievements waiting are dropped
        // (the one on screen goes too, or its timer would leave it there).
        if layer && window::exclusive_fullscreen() {
            self.achievement_queue.clear();
            if let Some(old) = self.notice.take() {
                let _ = old.handle.update(cx, |_, window, _| window.remove_window());
            }
            return;
        }
        // An achievement cut short by another notice shows again after it.
        if !notice.achievement {
            if let Some(shown) = self.notice.as_ref().map(|n| n.shown.clone()).filter(|n| n.achievement) {
                self.achievement_queue.waiting.push_front(shown);
            }
        }
        let generation = self.notice.as_ref().map_or(0, |n| n.generation + 1);
        let display = window::Display::current(cx);
        // A Windows notice stays where it was placed: one for another
        // monitor (the foreground window's moved) replaces it.
        #[cfg(windows)]
        if layer && self.notice.as_ref().is_some_and(|n| display.screen().is_some_and(|now| n.screen != Some(now))) {
            if let Some(old) = self.notice.take() {
                let _ = old.handle.update(cx, |_, window, _| window.remove_window());
            }
        }
        // A card of another height needs a window of its own.
        if self.notice.as_ref().is_some_and(|n| n.shown.height() != notice.height()) {
            if let Some(old) = self.notice.take() {
                let _ = old.handle.update(cx, |_, window, _| window.remove_window());
            }
        }
        let duration = notice.duration;
        let reused = self.notice.as_ref().is_some_and(|n| {
            let notice = notice.clone();
            n.handle
                .update(cx, |v, _, cx| {
                    v.notice = notice;
                    cx.notify();
                })
                .is_ok()
        });
        if reused {
            if let Some(n) = &mut self.notice {
                n.generation = generation;
                n.shown = notice;
            }
        } else {
            let scale = display.scale();
            let view = notice.clone();
            #[cfg(windows)]
            let screen = display.screen();
            match window::open_notice(cx, display, self.prefs.side, layer, notice.height(), |_, cx| {
                cx.new(|_| NoticeView { notice: view, scale })
            }) {
                Ok(handle) => {
                    self.notice = Some(NoticeWindow {
                        handle,
                        generation,
                        shown: notice,
                        #[cfg(windows)]
                        screen,
                    })
                }
                Err(e) => {
                    log::warn!("overlay: cannot show a notice: {e:#}");
                    // Its window is gone: the next notice opens a new one
                    // (else the achievements would wait for it forever).
                    self.notice = None;
                    return;
                }
            }
        }
        // Debug captures need the time to find the window (on Windows, the
        // native one of `HERONITE_DEBUG_OVERLAY=layer` too).
        let debug = self.debug_window || (cfg!(windows) && crate::env::value("HERONITE_DEBUG_OVERLAY").is_some());
        let shown = if debug { duration * 6 } else { duration };
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(shown).await;
            this.update(cx, |this, cx| {
                if this.notice.as_ref().is_some_and(|n| n.generation == generation) {
                    match this.achievement_queue.next() {
                        Some(next) => this.show_notice(next, layer, cx),
                        None => {
                            if let Some(n) = this.notice.take() {
                                let _ = n.handle.update(cx, |_, window, _| window.remove_window());
                            }
                        }
                    }
                }
            })
            .ok();
        })
        .detach();
    }
}

pub struct NoticeView {
    notice: Notice,
    /// Console scale of the display (the window itself is small).
    scale: f32,
}

impl Render for NoticeView {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        // Over the game, whatever the theme.
        theme::set_dark_only(true);
        kit::set_scale_value(self.scale);
        let n = &self.notice;
        let accent: Hsla = if n.error { theme::danger_text().into() } else { theme::text_primary().into() };
        let sub_colour: Hsla = n.accent.unwrap_or_else(|| theme::text_muted().into());
        let gold: Hsla = theme::achievement_gold().into();
        let text = div()
            .flex()
            .flex_col()
            .flex_1()
            .min_w_0()
            .gap(u(6.))
            // An achievement's "Achievement unlocked" goes above its name.
            .when(n.achievement, |d| {
                d.children(n.sub.clone().map(|s| div().text_size(u(12.)).font_weight(theme::W_BOLD).text_color(gold).truncate().child(s)))
            })
            .child(div().text_size(u(16.)).font_weight(theme::W_BOLD).text_color(accent).truncate().child(n.title.clone()))
            .when(!n.achievement, |d| d.children(n.sub.clone().map(|s| div().text_size(u(13.)).text_color(sub_colour).truncate().child(s))))
            .children(n.description.clone().map(|d| div().text_size(u(13.)).text_color(theme::white_a(0.78)).truncate().child(d)))
            .when(!n.chips.is_empty(), |d| d.child(chips(&n.chips)))
            .children(n.volume.map(|v| {
                let fill = (f32::from(v.min(100)) / 100.).clamp(0., 1.);
                div()
                    .flex()
                    .items_center()
                    .gap(u(10.))
                    .child(
                        div()
                            .flex_1()
                            .h(u(6.))
                            .rounded_full()
                            .bg(theme::white_a(0.14))
                            .child(div().h_full().w(relative(fill)).rounded_full().bg(theme::console_btn_x())),
                    )
                    .child(div().w(u(40.)).text_size(u(13.)).text_color(theme::text_muted()).child(format!("{v}%")))
            }));
        let lead = match (&n.image, n.achievement) {
            (Some(image), _) => div()
                .flex_none()
                .size(u(44.))
                .rounded(u(10.))
                .overflow_hidden()
                .child(img(ImageSource::Render(image.clone())).size_full().object_fit(ObjectFit::Cover)),
            (None, true) => {
                div().flex_none().size(u(44.)).rounded(u(10.)).flex().items_center().justify_center().bg(gold.opacity(0.16)).child(icon(
                    n.icon,
                    u(26.),
                    gold,
                ))
            }
            (None, false) => div().flex_none().child(icon(n.icon, u(26.), accent)),
        };
        let border = match (n.accent, n.achievement) {
            (Some(c), _) => c.opacity(0.7),
            (None, true) => gold.opacity(0.45),
            (None, false) => theme::white_a(0.1),
        };
        div()
            .size_full()
            .font_family(theme::font_body())
            .flex()
            .items_center()
            .gap(u(14.))
            .px(u(18.))
            .rounded(u(18.))
            .bg(theme::overlay_fill())
            .border_1()
            .border_color(border)
            .child(lead)
            .child(text)
    }
}

/// The line of badges: the rarity in its tier's colour when rare, points in
/// gold, hardcore in the danger colour, the rest muted; the set's name is
/// cut when the line is full.
fn chips(chips: &[Chip]) -> Div {
    let gold: Hsla = theme::achievement_gold().into();
    let danger: Hsla = theme::danger_text().into();
    let mut line = div().flex().items_center().gap(u(6.)).overflow_hidden();
    for c in chips {
        let (fg, bg): (Hsla, Hsla) = match c {
            Chip::Rarity(r) if *r < heronite_achievements::AchievementSet::RARE_BELOW => {
                let tier: Hsla = theme::rarity_colour(theme::rarity_tier(*r)).into();
                (tier, tier.opacity(0.16))
            }
            Chip::Points(_) => (gold, gold.opacity(0.16)),
            Chip::Hardcore => (danger, danger.opacity(0.16)),
            Chip::Rarity(_) | Chip::Kind(_) | Chip::Group(_) => (theme::white_a(0.78), theme::white_a(0.1)),
        };
        let chip = div().px(u(8.)).py(u(2.)).rounded_full().bg(bg).text_size(u(12.)).text_color(fg).font_weight(theme::W_BOLD).truncate();
        let chip = if matches!(c, Chip::Group(_)) { chip.min_w_0() } else { chip.flex_none() };
        line = line.child(chip.child(c.text()));
    }
    line
}

#[cfg(test)]
mod tests {
    // Explicit imports: `use super::*` would bring in gpui's `test` macro and shadow `#[test]`.
    use super::{Chip, Details, Notice, Queue, Unlock, QUEUE_MAX};
    use heronite_achievements::prefs::NoticeFields;
    use heronite_achievements::{Achievement, Kind, Source};

    fn unlock(source: Source, name: &str, rarity: Option<f32>) -> Unlock {
        Unlock { achievement: Achievement { name: name.into(), rarity, unlocked: true, ..Default::default() }, source, group: None }
    }

    fn ach(name: impl AsRef<str>) -> Notice {
        Notice::achievement(&unlock(Source::Local, name.as_ref(), None), NoticeFields::default(), None)
    }

    /// Everything a source could give.
    fn full(source: Source) -> Unlock {
        let mut u = unlock(source, "Untouchable", Some(1.2));
        u.achievement.description = "  Beat the first boss without taking a hit. ".into();
        u.achievement.points = Some(25);
        u.achievement.kind = Some(Kind::WinCondition);
        u.achievement.hardcore = true;
        u.group = Some("The Pale Reach".into());
        u
    }

    fn chips(d: &Details) -> Vec<String> {
        d.chips.iter().map(Chip::text).collect()
    }

    #[test]
    fn the_queue_keeps_a_few_and_sums_up_the_rest() {
        let mut q = Queue::default();
        for i in 0..7 {
            q.push(ach(format!("a{i}")));
        }
        let titles: Vec<String> = std::iter::from_fn(|| q.next()).map(|n| n.title).collect();
        assert_eq!(titles.len(), QUEUE_MAX + 1);
        assert_eq!(titles[..QUEUE_MAX], ["a0", "a1", "a2", "a3"]);
        assert_eq!(titles[QUEUE_MAX], "3 more achievements");
        assert!(q.next().is_none());
    }

    #[test]
    fn notices_keep_the_order_of_the_unlocks() {
        let mut q = Queue::default();
        let (a, b, c) = (q.ticket(), q.ticket(), q.ticket());
        q.arrive(c, ach("c"));
        q.arrive(a, ach("a"));
        assert_eq!(q.next().map(|n| n.title).as_deref(), Some("a"));
        // "c" waits for "b", whose icon is still loading.
        assert!(q.next().is_none());
        q.arrive(b, ach("b"));
        let titles: Vec<String> = std::iter::from_fn(|| q.next()).map(|n| n.title).collect();
        assert_eq!(titles, ["b", "c"]);
        // One made before a clear is dropped when it arrives.
        let late = q.ticket();
        q.clear();
        q.arrive(late, ach("late"));
        assert!(q.next().is_none());
        let after = q.ticket();
        q.arrive(after, ach("after"));
        assert_eq!(q.next().map(|n| n.title).as_deref(), Some("after"));
    }

    #[test]
    fn a_single_one_past_the_queue_shows_as_itself() {
        let mut q = Queue::default();
        for i in 0..=QUEUE_MAX {
            q.push(ach(format!("a{i}")));
        }
        let titles: Vec<String> = std::iter::from_fn(|| q.next()).map(|n| n.title).collect();
        assert_eq!(titles, ["a0", "a1", "a2", "a3", "a4"]);
    }

    #[test]
    fn a_rare_achievement_takes_its_tier_colour() {
        let all = NoticeFields::default();
        let rare = Notice::achievement(&unlock(Source::Steam, "x", Some(2.)), all, None);
        assert!(rare.accent.is_some());
        assert_eq!(rare.sub.as_deref(), Some("Achievement unlocked"));
        assert_eq!(rare.chips, [Chip::Rarity(2.)]);
        assert_eq!(rare.chips[0].text(), "2.0%");
        let common = Notice::achievement(&unlock(Source::Steam, "y", Some(60.)), all, None);
        assert!(common.accent.is_none());
        assert_eq!(ach("z").sub.as_deref(), Some("Achievement unlocked"));
        // The rarity turned off takes its colour with it.
        let hidden = Notice::achievement(&unlock(Source::Steam, "x", Some(2.)), NoticeFields { rarity: false, ..all }, None);
        assert!(hidden.accent.is_none());
        assert!(hidden.chips.is_empty());
    }

    #[test]
    fn each_source_shows_what_it_gives() {
        let all = NoticeFields::default();
        // RetroAchievements: points, type and hardcore (and a set, were it given).
        let retro = Details::of(&full(Source::RetroAchievements), all);
        assert_eq!(retro.description.as_deref(), Some("Beat the first boss without taking a hit."));
        assert_eq!(chips(&retro), ["1.2%", "25 points", "Win condition", "Hardcore", "The Pale Reach"]);
        // Epic Games: its points are XP; types and hardcore are RetroAchievements'.
        assert_eq!(chips(&Details::of(&full(Source::Epic), all)), ["1.2%", "25 XP", "The Pale Reach"]);
        // Steam and game files: the set only.
        assert_eq!(chips(&Details::of(&full(Source::Steam), all)), ["1.2%", "The Pale Reach"]);
        assert_eq!(chips(&Details::of(&full(Source::Local), all)), ["1.2%", "The Pale Reach"]);
        // GOG: description and rarity.
        let mut gog = full(Source::Gog);
        gog.group = None;
        let gog = Details::of(&gog, all);
        assert_eq!(chips(&gog), ["1.2%"]);
        assert_eq!(gog.rarity, Some(1.2));
        assert!(gog.description.is_some());
        // One point is singular; hardcore shows only for a hardcore unlock.
        let mut one = full(Source::RetroAchievements);
        one.achievement.points = Some(1);
        one.achievement.hardcore = false;
        one.group = None;
        assert_eq!(chips(&Details::of(&one, all)), ["1.2%", "1 point", "Win condition"]);
    }

    #[test]
    fn each_switch_hides_its_line() {
        let all = NoticeFields::default();
        let u = full(Source::RetroAchievements);
        let off = |change: fn(&mut NoticeFields)| {
            let mut f = all;
            change(&mut f);
            Details::of(&u, f)
        };
        assert_eq!(off(|f| f.description = false).description, None);
        assert_eq!(off(|f| f.rarity = false).rarity, None);
        assert_eq!(chips(&off(|f| f.rarity = false)), ["25 points", "Win condition", "Hardcore", "The Pale Reach"]);
        assert_eq!(chips(&off(|f| f.points = false)), ["1.2%", "Win condition", "Hardcore", "The Pale Reach"]);
        assert_eq!(chips(&off(|f| f.kind = false)), ["1.2%", "25 points", "Hardcore", "The Pale Reach"]);
        assert_eq!(chips(&off(|f| f.hardcore = false)), ["1.2%", "25 points", "Win condition", "The Pale Reach"]);
        assert_eq!(chips(&off(|f| f.group = false)), ["1.2%", "25 points", "Win condition", "Hardcore"]);
        let none = NoticeFields { description: false, rarity: false, points: false, group: false, kind: false, hardcore: false };
        let bare = Details::of(&u, none);
        assert_eq!((bare.description, bare.rarity, bare.chips), (None, None, Vec::new()));
        assert_eq!(Notice::achievement(&u, none, None).sub.as_deref(), Some("Achievement unlocked"));
    }

    #[test]
    fn the_card_grows_with_its_lines() {
        let all = NoticeFields::default();
        let plain = ach("a");
        assert_eq!(plain.height(), super::window::NOTICE_H);
        let rich = Notice::achievement(&full(Source::RetroAchievements), all, None);
        assert!(rich.height() > plain.height());
        let no_badges = NoticeFields { rarity: false, points: false, group: false, kind: false, hardcore: false, ..all };
        let middle = Notice::achievement(&full(Source::RetroAchievements), no_badges, None);
        assert!(plain.height() < middle.height() && middle.height() < rich.height());
        // An empty description takes no line, nor does an unknown rarity.
        let mut blank = full(Source::Gog);
        blank.achievement.description = "   ".into();
        blank.achievement.rarity = None;
        blank.group = None;
        assert_eq!(Notice::achievement(&blank, all, None).height(), plain.height());
        // The summing-up card stays as it was.
        assert_eq!(Notice::more_achievements(3).height(), plain.height());
    }

    #[test]
    fn the_previews_cover_every_line() {
        let made_up = super::made_up_unlocks("");
        assert_eq!(made_up.len(), 6);
        let all = NoticeFields::default();
        let first_two: Vec<Details> = made_up.iter().take(2).map(|u| Details::of(u, all)).collect();
        assert!(first_two.iter().all(|d| d.description.is_some() && d.rarity.is_some()));
        let badges: Vec<String> = first_two.iter().flat_map(chips).collect();
        assert_eq!(badges, ["9.7%", "The Pale Reach", "1.2%", "25 points", "Missable", "Hardcore"]);
    }
}
