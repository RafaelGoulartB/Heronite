//! The focused game's name (or logo) with its facts and overview, as the
//! theme's `hero` options say. Shelves draw it on top; other layouts reuse
//! the parts.

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::console_theme::HeroTitle;
use heronite_data::CollectionGame;
use heronite_i18n::tr;

use super::super::CollectionView;
use crate::console::kit::{self, u};
use crate::console::{look, motion};
use crate::theme;

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

impl CollectionView {
    /// "Steam · Installed · 3h 54m played".
    pub(super) fn console_facts(g: &CollectionGame) -> String {
        let mut facts = vec![g.source_label.to_string()];
        facts.push(if g.installed { tr!("Installed").into() } else { tr!("Not installed").into() });
        if g.playtime_minutes > 0 {
            facts.push(tr!("{time} played", time = heronite_data::collection::format_playtime_minutes(g.playtime_minutes)));
        }
        facts.join(" \u{b7} ")
    }

    /// The game's status pill.
    pub(super) fn console_status_badge(&self, g: &CollectionGame) -> Option<Div> {
        self.lib
            .as_ref()
            .and_then(|l| l.statuses.iter().find(|s| s.id == g.status_id))
            .map(|s| kit::badge(heronite_data::status::status_label(&s.name).to_string(), theme::status_color(&s.slug)))
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
            if let Some(src) = self.console_logo(&key, size.logo.0, size.logo.1, window) {
                return div()
                    .w(u(size.logo.0))
                    .h(u(size.logo.1))
                    .child(img(src).size_full().object_fit(ObjectFit::Contain))
                    .into_any_element();
            }
        }
        kit::title(g.title.clone(), size.title).into_any_element()
    }

    /// Name, facts and overview of `key`, sliding in when the game changes.
    pub(super) fn console_hero(&mut self, key: Option<&SharedString>, size: HeroSize, window: &Window) -> Div {
        let Some(g) = key.and_then(|k| self.lib.as_ref()?.get(k)).cloned() else { return div() };
        let theme = look::theme();
        let name = self.console_name(&g, size, window);
        let facts = theme.hero.facts.then(|| {
            div()
                .flex()
                .items_center()
                .gap(u(12.))
                .text_size(u(16.))
                .text_color(theme::text_muted())
                .child(Self::console_facts(&g))
                .children(self.console_status_badge(&g))
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
        div().child(motion::enter_from(SharedString::from(format!("hero-{}", g.key)), hero, 300, 14., 0., 0.25))
    }
}
