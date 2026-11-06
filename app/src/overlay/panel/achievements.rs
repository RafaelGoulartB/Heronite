//! The Achievements tab: the game's progress, what this session unlocked,
//! the closest locked ones and the latest unlocks, from the achievements
//! service (whatever the source). It shows only when the game has a set;
//! showing it reads the sources again (at most once a minute), so unlocks
//! of the session show up.
//!
//! Each achievement is drawn with its icon (loaded off the UI thread into a
//! small image store the tab creates when first shown), its description,
//! its rarity and its progress when the source counts it. Secret ones stay
//! hidden: they are only counted.

use std::sync::Arc;

use chrono::{DateTime, Utc};
use futures::StreamExt;
use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_achievements::{Achievement, AchievementSet, GameAchievements, Source};
use heronite_data::AppPaths;
use heronite_i18n::{tr, trn};
use heronite_images::{ImageRef, Variant};

use super::{OverlayPanel, Tab};
use crate::console::kit::{self, u};
use crate::console::list::{self, Row};
use crate::console::motion;
use crate::icons::Icon;
use crate::images::ImageStore;
use crate::services::achievements;
use crate::theme;
use crate::ui::achievements::{badge, bar, percent_label, short_day};
use crate::ui::icon;

/// Rows per group.
const PER_GROUP: usize = 5;
/// Size of an achievement's icon, in design px.
const BADGE: f32 = 48.;

/// The game's achievements as the service has them.
pub struct Shown {
    game: Arc<GameAchievements>,
    order: Vec<Source>,
}

impl Shown {
    fn set(&self) -> Option<&AchievementSet> {
        self.game.primary(&self.order)
    }
}

/// The achievements' icons, made when the tab is first drawn.
pub struct Icons {
    store: ImageStore,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Group {
    Session,
    Next,
    Recent,
}

/// What a row of the tab draws.
#[derive(Clone, Debug)]
pub(super) enum Item {
    Summary,
    /// A caption, or the line saying there is nothing yet.
    Plain,
    Achievement(Achievement, Group),
    /// How many secret ones are still locked.
    Secret(usize),
}

impl OverlayPanel {
    /// Reads the game's copy and follows the service's changes.
    pub(super) fn watch_achievements(&mut self, cx: &mut Context<Self>) {
        let Some(app_name) = self.game.as_ref().map(|g| g.game.app_name.clone()) else { return };
        if !achievements::enabled() {
            return;
        }
        let service = achievements::get(cx);
        service.update(cx, |s, cx| s.want_playing(&app_name, false, cx));
        self._achievements_sub = Some(cx.observe(&service, |this, service, cx| this.take_achievements(&service, cx)));
        self.take_achievements(&service, cx);
    }

    fn take_achievements(&mut self, service: &Entity<achievements::Achievements>, cx: &mut Context<Self>) {
        let Some(app_name) = self.game.as_ref().map(|g| g.game.app_name.as_str()) else { return };
        let entry = service.read(cx).entry_of_app(app_name).filter(|e| e.primary().is_some());
        let same = match (&self.achievements, entry) {
            (Some(a), Some(e)) => Arc::ptr_eq(&a.game, &e.game),
            (None, None) => true,
            _ => false,
        };
        if same {
            return;
        }
        // Stay on the tab shown (its index moves only if Achievements goes away).
        let tab = self.tab();
        self.achievements = entry.map(|e| Shown { game: e.game.clone(), order: e.order.clone() });
        self.tab = self.tabs().iter().position(|t| *t == tab).unwrap_or(0);
        cx.notify();
    }

    pub(super) fn refresh_achievements(&mut self, cx: &mut Context<Self>) {
        let Some(app_name) = self.game.as_ref().map(|g| g.game.app_name.clone()) else { return };
        achievements::get(cx).update(cx, |s, cx| s.want_playing(&app_name, true, cx));
    }

    /// When the session began, on the wall clock.
    fn session_start(&self) -> Option<DateTime<Utc>> {
        let elapsed = self.game.as_ref()?.game.started.elapsed();
        Some(Utc::now() - chrono::Duration::from_std(elapsed).ok()?)
    }

    /// Home's row that leads to the tab.
    pub(super) fn achievements_home_row(&self) -> Option<Row> {
        let set = self.achievements.as_ref()?.set()?;
        Some(Row::action("achievements", tr!("Achievements")).icon(Icon::Trophy).desc(progress_line(set)))
    }

    pub(super) fn achievements_activate(&mut self, id: &str, cx: &mut Context<Self>) -> bool {
        if id != "achievements" {
            return false;
        }
        self.show_tab(Tab::Achievements, cx);
        true
    }

    fn achievement_items(&self) -> Vec<(Row, Item)> {
        match self.achievements.as_ref().and_then(|a| a.set()) {
            Some(set) => items(set, self.session_start()),
            None => vec![(Row::info("none", tr!("No achievements to show yet."), "").enabled(false), Item::Plain)],
        }
    }

    pub(super) fn achievement_rows(&self) -> Vec<Row> {
        self.achievement_items().into_iter().map(|(r, _)| r).collect()
    }

    /// The image store of the icons, made on first use; what it loads lands
    /// here and redraws.
    fn icons(&mut self, cx: &mut Context<Self>) -> &mut ImageStore {
        if self.icons.is_none() {
            let (store, mut rx) = ImageStore::new(self.paths.images_cache_dir(), &AppPaths::for_data(&self.paths));
            cx.spawn(async move |this, cx| {
                while let Some(first) = rx.next().await {
                    let mut batch = vec![first];
                    while let Ok(more) = rx.try_recv() {
                        batch.push(more);
                    }
                    let alive = this
                        .update(cx, |this, cx| {
                            if let Some(icons) = &mut this.icons {
                                for item in batch {
                                    icons.store.on_loaded(item);
                                }
                            }
                            cx.notify();
                        })
                        .is_ok();
                    if !alive {
                        break;
                    }
                }
            })
            .detach();
            self.icons = Some(Icons { store });
        }
        &mut self.icons.as_mut().expect("just made").store
    }

    /// Frees the GPU memory of icons the store let go (from `render`).
    pub(super) fn icons_frame(&mut self, window: &mut Window) {
        if let Some(icons) = &mut self.icons {
            icons.store.drop_evicted(window);
        }
    }

    /// The tab's list, each achievement with its icon, rarity and progress.
    pub(super) fn render_achievements(&mut self, active: bool, window: &Window, cx: &mut Context<Self>) -> Stateful<Div> {
        let items = self.achievement_items();
        let rows: Vec<Row> = items.iter().map(|(r, _)| r.clone()).collect();
        let px_size = (BADGE * kit::scale() * window.scale_factor()).round() as u32;
        let images: Vec<Option<ImageSource>> = items
            .iter()
            .map(|(_, item)| match item {
                Item::Achievement(a, _) => icon_of(self.icons(cx), a.icon_now(), px_size),
                _ => None,
            })
            .collect();
        let set = self.achievements.as_ref().and_then(|a| a.set()).cloned();
        self.nav.render_with(("overlay-rows", self.tab), &rows, active, |i, row, focused| match (&items[i].1, &set) {
            (Item::Summary, Some(set)) => summary_card(set, focused).into_any_element(),
            (Item::Achievement(a, group), _) => achievement_row(a, *group, images[i].clone(), focused).into_any_element(),
            (Item::Secret(n), _) => secret_row(*n, focused).into_any_element(),
            _ => list::render_row(row, focused).into_any_element(),
        })
    }
}

/// An icon (URL or file) at `px_size` device px, once loaded.
fn icon_of(store: &mut ImageStore, url: &str, px_size: u32) -> Option<ImageSource> {
    if url.is_empty() {
        return None;
    }
    let id: SharedString = format!("ach:{url}:{px_size}").into();
    let chain = if url.starts_with("http") { ImageRef::Url(url.to_string()) } else { ImageRef::File(std::path::PathBuf::from(url)) };
    store.get_or_request(&id, || vec![chain], px_size, px_size, Variant::Cover, 30).map(ImageSource::Render)
}

/// "12/40 · 30% · 120/400 points".
fn progress_line(set: &AchievementSet) -> String {
    let (unlocked, total) = (set.unlocked(), set.total());
    let pct = (unlocked * 100).checked_div(total).unwrap_or(0);
    let mut line = format!("{unlocked}/{total} · {pct}%");
    if let Some((got, all)) = set.points() {
        line.push_str(" · ");
        line.push_str(&tr!("{got}/{all} points", got, all));
    }
    line
}

/// The rows of the tab and what each draws: the progress, then this
/// session's unlocks, the closest locked ones and the latest unlocks (5
/// each), then how many are secret.
pub(super) fn items(set: &AchievementSet, session_start: Option<DateTime<Utc>>) -> Vec<(Row, Item)> {
    let mut items = Vec::new();
    let summary = if set.is_complete() { tr!("100% · Completed").to_string() } else { progress_line(set) };
    items.push((Row::info("progress", set.source.label(), summary).icon(Icon::Trophy), Item::Summary));

    let this_session = |a: &&Achievement| session_start.is_some_and(|s| a.unlocked_at.is_some_and(|t| t >= s));
    let group = |items: &mut Vec<(Row, Item)>, title: String, list: Vec<&Achievement>, g: Group, prefix: &str| {
        if list.is_empty() {
            return;
        }
        items.push((Row::header(title), Item::Plain));
        for (i, a) in list.into_iter().enumerate() {
            let row = Row::info(SharedString::from(format!("{prefix}-{i}")), a.name.clone(), value_of(a, g)).desc(a.description.clone());
            items.push((row, Item::Achievement(a.clone(), g)));
        }
    };
    let session: Vec<&Achievement> = set.recent().into_iter().filter(this_session).take(PER_GROUP).collect();
    group(&mut items, tr!("This session").into(), session, Group::Session, "session");
    let next: Vec<&Achievement> = set.up_next().into_iter().take(PER_GROUP).collect();
    group(&mut items, tr!("Up next").into(), next, Group::Next, "next");
    let recent: Vec<&Achievement> = set.recent().into_iter().filter(|a| !this_session(a)).take(PER_GROUP).collect();
    group(&mut items, tr!("Recently unlocked").into(), recent, Group::Recent, "recent");

    let secret = set.achievements.iter().filter(|a| a.is_secret()).count();
    if secret > 0 {
        let row = Row::info("secret", trn!(secret, "{n} secret achievement", "{n} secret achievements"), "").desc(secret_hint(secret));
        items.push((row, Item::Secret(secret)));
    }
    items
}

/// The short value of a row (what a screen reader of the list would get):
/// counted progress of a locked one, else the rarity, else when it was
/// unlocked.
fn value_of(a: &Achievement, group: Group) -> String {
    match (group, a.progress.filter(|p| p.max > 0), a.rarity) {
        (Group::Next, Some(p), _) => format!("{}/{}", p.current.min(p.max), p.max),
        (_, _, Some(r)) => percent_label(r),
        _ => when_unlocked(a, group),
    }
}

/// "20:41" for this session's unlocks, the day for older ones.
fn when_unlocked(a: &Achievement, group: Group) -> String {
    let Some(at) = a.unlocked_at.map(|t| t.with_timezone(&chrono::Local)) else { return String::new() };
    match group {
        Group::Session => at.format("%H:%M").to_string(),
        _ => short_day(at.date_naive()),
    }
}

/// Text over the row: the focus text on the pill, else `idle`.
fn ink(focused: bool, idle: impl Into<Hsla>) -> Hsla {
    if focused {
        theme::console_on_focus().into()
    } else {
        idle.into()
    }
}

/// A rarity colour that reads on the row (darker on the light focus pill).
fn tier_colour(percent: f32, focused: bool) -> Rgba {
    let c = theme::rarity_colour(theme::rarity_tier(percent));
    if focused {
        motion::mix(c, theme::console_on_focus(), 0.45)
    } else {
        c
    }
}

/// "2.1%" in the colour of its rarity tier, sized for the overlay.
fn rarity_tag(percent: f32, focused: bool) -> Div {
    let c = tier_colour(percent, focused);
    div()
        .flex_none()
        .px(u(8.))
        .py(u(2.))
        .rounded_full()
        .bg(Hsla::from(c).opacity(if focused { 0.2 } else { 0.16 }))
        .text_size(u(12.))
        .font_weight(theme::W_BOLD)
        .text_color(c)
        .child(percent_label(percent))
}

/// A centred icon over a badge without a picture.
fn badge_icon(ic: Icon, colour: Hsla) -> Div {
    div().absolute().top_0().left_0().size_full().flex().items_center().justify_center().child(icon(ic, u(24.), colour))
}

/// The achievement's icon, or a trophy / lock when it has none; this
/// session's unlocks get a gold ring.
fn achievement_badge(image: Option<ImageSource>, a: &Achievement, ring: bool) -> Div {
    let fallback = image.is_none();
    let grey = !a.unlocked && a.icon_locked.is_empty();
    let mut b = badge(image, a.unlocked, grey, BADGE * kit::scale());
    if fallback {
        b = b.child(if a.unlocked {
            badge_icon(Icon::Trophy, theme::achievement_gold().into())
        } else {
            badge_icon(Icon::Lock, kit::ink(0.55))
        });
    }
    if ring {
        b = b.border_2().border_color(theme::achievement_gold());
    }
    b
}

/// A progress bar whose track also reads on the light focus pill.
fn progress_bar(fraction: f32, gold: bool, height: f32, focused: bool) -> Div {
    if !focused {
        return bar(fraction, gold, height * kit::scale());
    }
    let fill: Hsla = if gold { theme::achievement_gold().into() } else { theme::accent().into() };
    div()
        .w_full()
        .h(u(height))
        .rounded_full()
        .bg(theme::black_a(0.14))
        .overflow_hidden()
        .child(div().h_full().rounded_full().bg(fill).w(relative(fraction.clamp(0., 1.))))
}

fn achievement_row(a: &Achievement, group: Group, image: Option<ImageSource>, focused: bool) -> Div {
    let muted = ink(focused, theme::text_muted());
    let progress = a.progress.filter(|p| p.max > 0 && !a.unlocked);
    let when = when_unlocked(a, group);
    // Name and rarity on top, so every bar below has the full width.
    let name = div().flex_1().min_w_0().text_size(u(17.)).text_color(ink(focused, theme::text_primary())).truncate().child(a.name.clone());
    let top = div().flex().items_center().gap(u(10.)).child(name).children(a.rarity.map(|r| rarity_tag(r, focused)));
    let mut text = div().flex().flex_col().flex_1().min_w_0().gap(u(3.)).py(u(10.)).child(top);
    if !a.description.is_empty() {
        let desc = div().text_size(u(13.)).line_height(u(18.)).text_color(muted).child(a.description.clone());
        // The focused one reads in full (up to three lines); the others keep one.
        text = text.child(if focused { desc.line_clamp(3).text_ellipsis() } else { desc.truncate() });
    }
    if let Some(p) = progress {
        let count = div().flex_none().text_size(u(12.)).text_color(muted).child(format!("{}/{}", p.current.min(p.max), p.max));
        text = text.child(
            div()
                .flex()
                .items_center()
                .gap(u(10.))
                .pt(u(3.))
                .child(div().flex_1().child(progress_bar(p.fraction(), false, 5., focused)))
                .child(count),
        );
    } else if !when.is_empty() {
        let (colour, line) = match group {
            Group::Session => (ink(focused, theme::achievement_gold()), tr!("Unlocked at {time}", time = when)),
            _ => (muted, tr!("Unlocked {date}", date = when)),
        };
        text = text.child(div().text_size(u(12.)).text_color(colour).child(line));
    }
    kit::row_base(focused).min_h(u(72.)).gap(u(14.)).child(achievement_badge(image, a, group == Group::Session)).child(text)
}

fn secret_row(n: usize, focused: bool) -> Div {
    let lock_colour = if focused { theme::black_a(0.45) } else { kit::ink(0.55) };
    let lock = badge(None, false, true, BADGE * kit::scale()).child(badge_icon(Icon::Lock, lock_colour));
    kit::row_base(focused).min_h(u(68.)).gap(u(14.)).child(lock).child(
        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_w_0()
            .gap(u(3.))
            .child(div().text_size(u(17.)).text_color(ink(focused, theme::text_primary())).child(trn!(
                n,
                "{n} secret achievement",
                "{n} secret achievements"
            )))
            .child(div().text_size(u(13.)).text_color(ink(focused, theme::text_muted())).child(secret_hint(n))),
    )
}

/// "Hidden until you unlock it", or "them" for several.
fn secret_hint(n: usize) -> String {
    trn!(n, "Hidden until you unlock it.", "Hidden until you unlock them.")
}

/// The game's progress: the count, the bar (gold at 100%), the source,
/// points and rare unlocks.
fn summary_card(set: &AchievementSet, focused: bool) -> Div {
    let (unlocked, total) = (set.unlocked(), set.total());
    let complete = set.is_complete();
    let pct = (unlocked * 100).checked_div(total).unwrap_or(0);
    let gold: Hsla = theme::achievement_gold().into();
    let strong = if complete { gold } else { ink(focused, theme::text_primary()) };
    let mut facts = vec![set.source.label().to_string()];
    if let Some((got, all)) = set.points() {
        facts.push(tr!("{got}/{all} points", got, all));
    }
    let rare = set.rare_unlocked();
    if rare > 0 {
        facts.push(tr!("{n} rare", n = rare));
    }
    let headline = if complete { tr!("100% · Completed").to_string() } else { format!("{unlocked}/{total}") };
    let trophy = div()
        .flex_none()
        .size(u(52.))
        .rounded(u(14.))
        .flex()
        .items_center()
        .justify_center()
        .bg(if complete {
            gold.opacity(0.18)
        } else if focused {
            theme::black_a(0.08)
        } else {
            kit::ink(0.08)
        })
        .child(icon(Icon::Trophy, u(28.), strong));
    let count = div()
        .flex()
        .items_baseline()
        .justify_between()
        .gap(u(10.))
        .child(div().text_size(u(22.)).font_weight(theme::W_BOLD).text_color(strong).child(headline))
        .when(!complete, |d| d.child(div().text_size(u(15.)).text_color(ink(focused, theme::text_muted())).child(format!("{pct}%"))));
    let facts = div().text_size(u(13.)).text_color(ink(focused, theme::text_muted())).truncate().child(facts.join(" · "));
    kit::row_base(focused).min_h(u(88.)).gap(u(16.)).child(trophy).child(
        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_w_0()
            .gap(u(6.))
            .py(u(12.))
            .child(count)
            .child(progress_bar(pct as f32 / 100., complete, 7., focused))
            .child(facts),
    )
}

#[cfg(test)]
mod tests {
    // Explicit imports: `use super::*` would bring in gpui's `test` macro and shadow `#[test]`.
    use chrono::{DateTime, TimeZone, Utc};
    use heronite_achievements::{Achievement, AchievementSet, Progress, Source};

    use super::{items, value_of, Group, Item};
    use crate::console::list::{Row, Value};

    fn a(id: &str, unlocked_at: Option<DateTime<Utc>>) -> Achievement {
        Achievement { id: id.into(), name: id.into(), unlocked: unlocked_at.is_some(), unlocked_at, ..Default::default() }
    }

    fn set(achievements: Vec<Achievement>) -> AchievementSet {
        AchievementSet { source: Source::Steam, external_id: "1".into(), title: None, fetched_at: Utc::now(), achievements }
    }

    fn labels(items: &[(Row, Item)]) -> Vec<String> {
        items.iter().skip(1).map(|(r, _)| r.label.to_string()).collect()
    }

    #[test]
    fn groups_this_session_up_next_recent_and_secrets() {
        let start = Utc.with_ymd_and_hms(2026, 11, 6, 20, 0, 0).unwrap();
        let mut counted = a("counted", None);
        counted.progress = Some(Progress { current: 3, max: 10 });
        counted.rarity = Some(4.2);
        let mut secret = a("secret", None);
        secret.hidden = true;
        secret.name = "Spoiler".into();
        let mut now = a("now", Some(start + chrono::Duration::minutes(30)));
        now.rarity = Some(1.5);
        let s = set(vec![a("old", Some(start - chrono::Duration::days(2))), now, counted, a("plain", None), secret]);
        let items = items(&s, Some(start));
        assert_eq!(items[0].0.value, Value::Info("2/5 · 40%".into()));
        assert_eq!(
            labels(&items),
            ["This session", "now", "Up next", "counted", "plain", "Recently unlocked", "old", "1 secret achievement"]
        );
        // The secret one is only counted: its name shows nowhere.
        assert!(matches!(items.last().unwrap().1, Item::Secret(1)));
        assert!(!items.iter().any(|(r, i)| r.label.contains("Spoiler") || matches!(i, Item::Achievement(a, _) if a.id == "secret")));
        assert!(matches!(&items[2].1, Item::Achievement(a, Group::Session) if a.id == "now"));
        assert_eq!(items[2].0.value, Value::Info("1.5%".into()));
        // A counted one keeps its progress as the value; its rarity is drawn next to it.
        assert_eq!(items[4].0.value, Value::Info("3/10".into()));
        assert!(matches!(&items[4].1, Item::Achievement(a, Group::Next) if a.rarity == Some(4.2)));
    }

    #[test]
    fn without_a_session_every_unlock_is_recent() {
        let items = items(&set(vec![a("x", Some(Utc::now()))]), None);
        assert_eq!(labels(&items), ["Recently unlocked", "x"]);
        assert_eq!(items[0].0.value, Value::Info("100% · Completed".into()));
    }

    #[test]
    fn missing_data_gives_empty_values() {
        // No rarity, no time, no description, a broken counter: nothing is invented.
        let mut x = a("x", None);
        x.progress = Some(Progress { current: 5, max: 0 });
        assert_eq!(value_of(&x, Group::Next), "");
        let mut y = a("y", None);
        y.unlocked = true;
        assert_eq!(value_of(&y, Group::Recent), "");
        let items = items(&set(vec![x, y]), None);
        assert!(items.iter().all(|(r, _)| r.desc.is_none()));
    }
}
