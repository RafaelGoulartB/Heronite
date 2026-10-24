//! The focused game's name (or logo) with its facts and overview, as the
//! theme's `hero` options say. Shelves draw it on top; other layouts reuse
//! the parts.

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::console_theme::{FactStyle, Headings, HeroTitle};
use heronite_data::organize::release_year;
use heronite_data::CollectionGame;
use heronite_i18n::tr;

use super::super::data::played_ago;
use super::super::focus_left::score_color;
use super::super::CollectionView;
use crate::console::kit::{self, u};
use crate::console::{look, motion};
use crate::icons::Icon;
use crate::theme;
use crate::ui::icon;

/// Sizes of a hero block, design px.
#[derive(Clone, Copy)]
pub(crate) struct HeroSize {
    pub title: f32,
    /// Box the logo is fitted in.
    pub logo: (f32, f32),
    pub max_w: f32,
    /// Overview characters (0 = none).
    pub overview: usize,
}

impl Default for HeroSize {
    fn default() -> Self {
        Self { title: 46., logo: (460., 120.), max_w: 760., overview: 300 }
    }
}

/// How [`CollectionView::console_hero_with`] brings a game in.
#[derive(Clone, Copy)]
pub(crate) struct HeroMotion {
    /// Where its text slides from, design px (x, y).
    pub slide: (f32, f32),
    /// Part of the slide's id: a new value replays it for the same game.
    pub replay: u64,
    /// "Last played 3 days ago" on the line of facts.
    pub last_played: bool,
    /// A long name shrinks and stays on two lines (the block under it keeps its place).
    pub fit_title: bool,
}

impl Default for HeroMotion {
    fn default() -> Self {
        Self { slide: (14., 0.), replay: 0, last_played: false, fit_title: false }
    }
}

/// The size of a name `chars` long in a hero whose names are `size` px: a
/// long one shrinks a step (at most to 3/4) so it rarely needs a second line.
pub(crate) fn fitted_title(size: f32, chars: usize) -> f32 {
    match chars {
        0..=24 => size,
        25..=34 => size * 0.86,
        _ => size * 0.75,
    }
}

/// The overview shortened to about `max` characters at a word, with "…".
pub(crate) fn clip(text: &str, max: usize) -> String {
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if text.chars().count() <= max {
        return text;
    }
    let cut: String = text.chars().take(max).collect();
    let cut = cut.rfind(' ').map_or(cut.as_str(), |i| &cut[..i]);
    format!("{}\u{2026}", cut.trim_end_matches([',', ';', ':', '.', ' ']))
}

/// A logo fitted in a `w`×`h` design px box at its own proportions (a small
/// one grows up to twice its size), laid out at the size it shows at so the
/// column under it stays in place.
pub(crate) fn logo_element(src: ImageSource, w: f32, h: f32, window: &Window) -> AnyElement {
    let el = img(src.clone()).flex_none();
    match &src {
        ImageSource::Render(r) => {
            let size = r.size(0);
            let scale = window.scale_factor();
            let (lw, lh) = (size.width.0 as f32 / scale, size.height.0 as f32 / scale);
            if lw <= 0. || lh <= 0. {
                return el.into_any_element();
            }
            let (bw, bh) = (f32::from(u(w)), f32::from(u(h)));
            let k = (bw / lw).min(bh / lh).min(2.);
            el.w(px(lw * k)).h(px(lh * k)).into_any_element()
        }
        _ => el.w(u(w)).h(u(h)).object_fit(ObjectFit::Contain).into_any_element(),
    }
}

/// A small outlined box ("2023", "Adventure").
fn outlined(text: impl Into<SharedString>, size: f32) -> Div {
    div()
        .flex_none()
        .px(u(8.))
        .py(u(2.))
        .rounded(u(4.))
        .border_1()
        .border_color(kit::ink(0.55))
        .text_size(u(size))
        .text_color(theme::text_primary())
        .whitespace_nowrap()
        .child(text.into())
}

/// A shelf's title: spaced capitals when the theme asks for them (`count`
/// follows it, "Recently added · 12").
pub(crate) fn shelf_heading(title: &str, count: Option<usize>, size: f32) -> Div {
    let spaced = look::theme().interface.headings == Headings::Spaced;
    let text = kit::heading_text(title);
    let text = count.map_or(text.clone(), |n| format!("{text}  \u{b7}  {n}"));
    let heading = div().text_size(u(if spaced { size * 0.95 } else { size })).whitespace_nowrap();
    if spaced {
        heading.font_weight(theme::W_REGULAR).child(text)
    } else {
        heading.font_weight(theme::W_BOLD).child(text)
    }
}

/// The time `g` was played ("3h 54m"), `None` when it never was.
pub(crate) fn played_time(g: &CollectionGame) -> Option<String> {
    (g.playtime_minutes > 0).then(|| heronite_data::collection::format_playtime_minutes(g.playtime_minutes))
}

/// "3 days ago · 12h 40m"; `long` says "Last played 3 days ago · 12h 40m".
/// "Not played yet" for a game never played.
pub(crate) fn played_line(g: &CollectionGame, long: bool) -> String {
    let when = match g.last_played.as_deref().and_then(played_ago) {
        Some(ago) if long => tr!("Last played {when}", when = ago.to_lowercase()),
        Some(ago) => ago,
        None => tr!("Not played yet").to_string(),
    };
    [Some(when), played_time(g)].into_iter().flatten().collect::<Vec<_>>().join(" \u{b7} ")
}

/// A card's surface: a soft dark panel with a thin edge.
pub(crate) fn card_surface() -> Div {
    div().bg(kit::shade(0.35)).border_1().border_color(kit::ink(0.08))
}

/// Measures of a [`pill_button`], design px.
#[derive(Clone, Copy)]
pub(crate) struct PillSize {
    pub height: f32,
    /// Side padding of a button with a label.
    pub pad: f32,
    pub text: f32,
    pub icon: f32,
    /// Opacity of the ink the idle button is filled with.
    pub fill: f32,
}

impl PillSize {
    pub const SMALL: Self = Self { height: 50., pad: 22., text: 17., icon: 22., fill: 0.1 };
    pub const LARGE: Self = Self { height: 52., pad: 26., text: 18., icon: 24., fill: 0.12 };
}

/// A round (icon only) or pill (icon and label) button; the focused one
/// wears the focus pill. `tint` colours the icon while it is idle.
pub(crate) fn pill_button(focused: bool, ic: Icon, label: Option<SharedString>, tint: Option<Rgba>, size: PillSize) -> Div {
    let colour: Hsla = if focused { theme::console_on_focus().into() } else { tint.unwrap_or(theme::text_primary()).into() };
    let d = div()
        .relative()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .gap(u(10.))
        .h(u(size.height))
        .rounded_full()
        .border_1()
        .text_size(u(size.text))
        .font_weight(theme::W_BOLD)
        .text_color(colour);
    let d = if label.is_some() { d.px(u(size.pad)) } else { d.w(u(size.height)) };
    let d = if focused {
        d.border_color(transparent_black()).child(kit::focus_pill(size.height / 2.))
    } else {
        d.border_color(kit::ink(0.08)).bg(kit::ink(size.fill))
    };
    d.child(icon(ic, u(size.icon), colour)).children(label)
}

impl CollectionView {
    /// "Steam · Installed · 3h 54m played".
    pub(super) fn console_facts(g: &CollectionGame) -> String {
        let mut facts = vec![g.source_label.to_string()];
        facts.push(if g.installed { tr!("Installed").into() } else { tr!("Not installed").into() });
        facts.extend(played_time(g).map(|time| tr!("{time} played", time = time)));
        facts.join(" \u{b7} ")
    }

    /// The game's status: its name and colour.
    pub(super) fn console_status(&self, g: &CollectionGame) -> Option<(String, Rgba)> {
        let s = self.lib.as_ref()?.statuses.iter().find(|s| s.id == g.status_id)?;
        Some((heronite_data::status::status_label(&s.name).to_string(), theme::status_color(&s.slug)))
    }

    /// The one line of facts under a name: `text` and the status pill.
    pub(super) fn console_fact_line(&self, g: &CollectionGame, text: impl Into<SharedString>, size: f32) -> Div {
        div()
            .flex()
            .items_center()
            .gap(u(12.))
            .text_size(u(size))
            .text_color(theme::text_muted())
            .child(text.into())
            .children(self.console_status_badge(g))
    }

    /// The game's status pill.
    pub(super) fn console_status_badge(&self, g: &CollectionGame) -> Option<Div> {
        self.console_status(g).map(|(name, colour)| kit::badge(name, colour))
    }

    /// The facts as chips: the store, the year in a box, the score on its
    /// colour, the time played and the status.
    pub(super) fn console_chips(&self, g: &CollectionGame) -> Div {
        let score = g.details.critic_score.filter(|s| *s > 0.);
        div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap(u(10.))
            .text_size(u(16.))
            .child(div().font_weight(theme::W_BOLD).text_color(theme::text_primary()).child(g.source_label))
            .children(release_year(g).map(|y| outlined(y.to_string(), 14.)))
            .children(score.map(|s| {
                div()
                    .flex_none()
                    .px(u(8.))
                    .py(u(2.))
                    .rounded(u(4.))
                    .bg(score_color(s))
                    .text_size(u(14.))
                    .font_weight(theme::W_BOLD)
                    .text_color(theme::black_a(0.85))
                    .child(format!("{s:.0}"))
            }))
            .children(played_time(g).map(|t| div().text_color(theme::text_muted()).child(t)))
            .children(self.console_status_badge(g))
    }

    /// The first genres as outlined tags.
    fn console_genre_tags(g: &CollectionGame) -> Option<Div> {
        (!g.details.genres.is_empty())
            .then(|| div().flex().flex_wrap().gap(u(8.)).children(g.details.genres.iter().take(4).map(|t| outlined(t.clone(), 13.))))
    }

    /// The genres in one line, thin bars between them ("Adventure | RPG").
    pub(super) fn console_genre_line(g: &CollectionGame) -> Option<Div> {
        (!g.details.genres.is_empty()).then(|| {
            let mut line = div()
                .flex()
                .items_center()
                .gap(u(14.))
                .text_size(u(22.))
                .font_weight(theme::W_BOLD)
                .text_color(theme::text_primary())
                .whitespace_nowrap();
            for (i, t) in g.details.genres.iter().take(3).enumerate() {
                if i > 0 {
                    line = line.child(div().w(px(1.5)).h(u(20.)).bg(kit::ink(0.45)));
                }
                line = line.child(t.clone());
            }
            line
        })
    }

    /// The panel of numbers at the right edge (`hero.stats`): time played,
    /// last played, size on disk, score, genre and status; only what is
    /// known, at most `max` lines. The caller puts it at the edge (it is
    /// `absolute` with its own `right`).
    pub(super) fn console_stats(&self, g: &CollectionGame, max: usize) -> AnyElement {
        let mut lines: Vec<(Icon, String, Option<Rgba>)> = Vec::new();
        if let Some(time) = played_time(g) {
            lines.push((Icon::AccessTime, time, None));
        }
        if let Some(ago) = g.last_played.as_deref().and_then(played_ago) {
            lines.push((Icon::History, ago, None));
        }
        if let Some(size) = g.details.install_size.as_deref().filter(|s| g.installed && !s.is_empty() && *s != "0") {
            lines.push((Icon::FaHardDrive, size.to_string(), None));
        }
        if let Some(s) = g.details.critic_score.filter(|s| *s > 0.) {
            lines.push((Icon::Star, format!("{s:.0}"), Some(score_color(s))));
        }
        if let Some(genre) = g.details.genres.first() {
            lines.push((Icon::Label, genre.clone(), None));
        }
        if let Some((name, colour)) = self.console_status(g) {
            lines.push((Icon::Flag, name, Some(colour)));
        }
        lines.truncate(max);
        let mut fill = theme::console_sidebar();
        fill.a = 0.72;
        let panel = div()
            .relative()
            .flex()
            .flex_col()
            .gap(u(14.))
            .px(u(18.))
            .py(u(16.))
            .rounded_l(u(14.))
            .bg(fill)
            .text_size(u(16.))
            .text_color(theme::text_primary())
            .whitespace_nowrap()
            .children(lines.into_iter().map(|(ic, text, colour)| {
                div()
                    .flex()
                    .items_center()
                    .gap(u(12.))
                    .child(icon(ic, u(18.), colour.map_or_else(theme::text_muted, kit::tint)))
                    .child(text)
            }));
        // Slides in from the edge when the game changes.
        motion::enter_from(SharedString::from(format!("hero-stats-{}", g.key)), panel, 280, 16., 0., 0.3)
    }

    /// The overview of the focused game, when loaded (the Focus data).
    pub(super) fn console_overview(&self, key: &str) -> Option<SharedString> {
        self.focus.as_ref().filter(|f| f.key.as_ref() == key).and_then(|f| f.overview.clone())
    }

    /// The game's name as the theme wants it: its logo when it has one and
    /// the theme asks for logos, else the name in large type.
    pub(super) fn console_name(&mut self, g: &CollectionGame, size: HeroSize, window: &Window) -> AnyElement {
        if look::theme().hero.title == HeroTitle::Logo {
            let key = SharedString::from(g.key.clone());
            if let Some(src) = self.console_logo(&key, size.logo.0, size.logo.1, false, window) {
                return logo_element(src, size.logo.0, size.logo.1, window);
            }
        }
        kit::title(g.title.clone(), size.title).into_any_element()
    }

    /// Name, facts and overview of `key`, sliding in when the game changes.
    pub(super) fn console_hero(&mut self, key: Option<&SharedString>, size: HeroSize, window: &Window) -> Div {
        self.console_hero_with(key, size, HeroMotion::default(), window)
    }

    /// [`Self::console_hero`] brought in as `how` says.
    pub(super) fn console_hero_with(&mut self, key: Option<&SharedString>, size: HeroSize, how: HeroMotion, window: &Window) -> Div {
        let Some(g) = key.and_then(|k| self.lib.as_ref()?.get(k)).cloned() else { return div() };
        let theme = look::theme();
        let name = if how.fit_title && theme.hero.title == HeroTitle::Text {
            let px = fitted_title(size.title, g.title.chars().count());
            kit::title(g.title.clone(), px).line_clamp(2).into_any_element()
        } else {
            self.console_name(&g, size, window)
        };
        let mut line = Self::console_facts(&g);
        if let Some(ago) = g.last_played.as_deref().and_then(played_ago).filter(|_| how.last_played) {
            line = format!("{line} \u{b7} {}", tr!("Last played {when}", when = ago.to_lowercase()));
        }
        let facts = theme.hero.facts.then(|| match theme.hero.fact_style {
            FactStyle::Line => self.console_fact_line(&g, line, 16.),
            FactStyle::Chips => div().flex().flex_col().gap(u(10.)).child(self.console_chips(&g)).children(Self::console_genre_tags(&g)),
        });
        let overview = (theme.hero.overview && size.overview > 0).then(|| self.console_overview(&g.key)).flatten();
        let hero =
            div().relative().flex().flex_col().gap(u(10.)).max_w(u(size.max_w)).child(name).children(facts).children(overview.map(|o| {
                motion::fade_in(
                    SharedString::from(format!("hero-about-{}", g.key)),
                    kit::body_text(clip(&o, size.overview)).line_clamp(3),
                    260,
                )
            }));
        // A new game slides in a little (never from blank: holding a
        // direction keeps the text readable).
        let (dx, dy) = how.slide;
        div().child(motion::enter_from(SharedString::from(format!("hero-{}-{}", g.key, how.replay)), hero, 300, dx, dy, 0.25))
    }
}
