//! "Achievements" section of the focus panel (in the side column, under
//! Details by default; Settings → Appearance → Game sections moves or hides
//! it): the count and a progress bar, the latest three unlocks, the two
//! closest to being unlocked, and "View all". A game at 100% turns gold. For
//! a ROM the panel asks for the RetroAchievements account, or says its file
//! matches no set.

use std::rc::Rc;

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_achievements::{Achievement, AchievementSet, Problem, Source};

use super::focus::{h3, panel};
use crate::icons::Icon;
use crate::theme;
use crate::ui::achievements::{badge, bar, gold_a, long_day, rarity_pill, short_day, source_chip, synced_label};
use crate::ui::controls::{button_sm, ButtonKind};
use crate::ui::icon;
use heronite_i18n::tr;

/// Rows of "Recently unlocked" and of "Up next".
const RECENT: usize = 3;
const NEXT: usize = 2;
pub(super) const ROW_ICON: f32 = 34.;

/// One achievement ready to draw.
pub(super) struct Row {
    pub name: SharedString,
    pub description: SharedString,
    pub unlocked: bool,
    pub image: Option<ImageSource>,
    /// Draw the unlocked icon greyed out (the source has no locked one).
    pub grey: bool,
    /// Unlock day, progress ("7/12") or nothing.
    pub right: Option<SharedString>,
    pub progress: Option<f32>,
    pub rarity: Option<f32>,
}

impl Row {
    /// `hide_secrets`: a locked secret achievement keeps its name hidden.
    pub fn of(a: &Achievement, hide_secrets: bool, image: Option<ImageSource>) -> Row {
        let secret = hide_secrets && a.is_secret();
        let right = if a.unlocked {
            a.unlocked_at.map(|t| short_day(t.with_timezone(&chrono::Local).date_naive()))
        } else {
            a.progress.filter(|p| p.max > 1).map(|p| format!("{}/{}", p.current.min(p.max), p.max))
        };
        Row {
            name: if secret { tr!("Secret achievement").into() } else { a.name.clone().into() },
            description: if secret { tr!("Hidden until you unlock it.").into() } else { a.description.clone().into() },
            unlocked: a.unlocked,
            grey: !a.unlocked && a.icon_locked.is_empty(),
            image,
            right: right.map(Into::into),
            progress: (!a.unlocked).then_some(a.progress.filter(|p| p.max > 1).map(|p| p.fraction())).flatten(),
            rarity: a.rarity,
        }
    }
}

/// What the panel shows.
pub(super) enum Body {
    Set {
        set: Rc<AchievementSet>,
        recent: Vec<Row>,
        next: Vec<Row>,
        refreshing: bool,
    },
    /// The copy is being read for the first time.
    Loading,
    /// A ROM, and no RetroAchievements account yet.
    Connect,
    /// A ROM whose hash no set knows.
    Unmatched {
        file: SharedString,
        hash: Option<SharedString>,
    },
}

pub(super) struct Actions {
    pub view_all: Rc<dyn Fn(&mut Window, &mut App)>,
    pub refresh: Rc<dyn Fn(&mut Window, &mut App)>,
    /// Opens Settings → Achievements.
    pub settings: Rc<dyn Fn(&mut Window, &mut App)>,
}

/// The panel's body for an entry of the service (`None`: nothing to show).
pub(super) fn body_of(
    entry: &crate::services::achievements::GameEntry,
    rows: impl FnOnce(&AchievementSet) -> (Vec<Row>, Vec<Row>),
) -> Option<Body> {
    if let Some(set) = entry.primary() {
        let (recent, next) = rows(set);
        return Some(Body::Set { set: Rc::new(set.clone()), recent, next, refreshing: entry.refreshing });
    }
    match entry.problem() {
        Some((Source::RetroAchievements, Problem::NeedsAccount)) => Some(Body::Connect),
        Some((Source::RetroAchievements, Problem::Unmatched { hash, file })) => {
            Some(Body::Unmatched { file: file.clone().into(), hash: hash.clone().map(Into::into) })
        }
        // Only while a game is read for the first time: one checked before
        // and found without achievements shows nothing.
        _ if entry.refreshing && entry.problems.is_empty() && entry.order.iter().all(|s| entry.game.checked_at(*s).is_none()) => {
            Some(Body::Loading)
        }
        _ => None,
    }
}

/// The achievements of a set the panel lists: the latest unlocks and the
/// closest locked ones.
pub(super) fn picks(set: &AchievementSet) -> (Vec<&Achievement>, Vec<&Achievement>) {
    (set.recent().into_iter().take(RECENT).collect(), set.up_next().into_iter().take(NEXT).collect())
}

pub(super) fn view(body: Body, actions: Actions) -> AnyElement {
    match body {
        Body::Set { set, recent, next, refreshing } => set_view(&set, recent, next, refreshing, actions),
        Body::Loading => section(
            None,
            panel(false).child(div().text_size(px(12.5)).text_color(theme::ink_a(0.52)).child(tr!("Reading the achievements…"))),
        ),
        Body::Connect => notice(
            Source::RetroAchievements,
            tr!("Connect your RetroAchievements account to see and track this game's achievements."),
            None,
            button_sm("ach-connect", ButtonKind::Primary, tr!("Connect RetroAchievements")).on_click({
                let open = actions.settings.clone();
                move |_, w, cx| open(w, cx)
            }),
        ),
        Body::Unmatched { file, hash } => notice(
            Source::RetroAchievements,
            tr!("This ROM doesn't match any RetroAchievements set."),
            Some(match hash {
                Some(h) => format!("{file} · MD5 {h}").into(),
                None => file,
            }),
            button_sm("ach-pick", ButtonKind::Glass, tr!("Pick the game…")).on_click({
                let open = actions.view_all.clone();
                move |_, w, cx| open(w, cx)
            }),
        ),
    }
}

fn set_view(set: &AchievementSet, recent: Vec<Row>, next: Vec<Row>, refreshing: bool, actions: Actions) -> AnyElement {
    let (unlocked, total) = (set.unlocked(), set.total());
    let complete = set.is_complete();
    let pct = (unlocked * 100).checked_div(total).unwrap_or(0);
    let mut summary = vec![format!("{pct}%")];
    if let Some((got, all)) = set.points() {
        summary.push(tr!("{got}/{all} points", got, all));
    }
    let rare = set.rare_unlocked();
    if rare > 0 {
        summary.push(tr!("{n} rare", n = rare));
    }

    let count = div()
        .text_size(px(13.6))
        .font_weight(theme::W_500)
        .text_color(if complete { theme::achievement_gold().into() } else { theme::ink_a(0.62) })
        .child(format!("{unlocked}/{total}"));
    let mut body =
        panel(false).when(complete, |d| d.border_color(theme::achievement_gold())).child(bar(pct as f32 / 100., complete, 6.)).child(
            div()
                .mt(px(8.))
                .flex()
                .items_center()
                .justify_between()
                .gap(px(8.))
                .text_size(px(12.))
                .text_color(theme::ink_a(0.62))
                .child(div().min_w_0().truncate().child(summary.join(" · ")))
                .child(source_chip(set.source)),
        );
    if complete {
        let when = set.last_unlocked_at().map(|t| long_day(t.with_timezone(&chrono::Local).date_naive()));
        body = body.child(
            div()
                .mt(px(10.))
                .flex()
                .items_center()
                .gap(px(8.))
                .px(px(11.))
                .py(px(7.))
                .rounded(px(9.))
                .bg(gold_a(0.14))
                .text_size(px(12.5))
                .font_weight(theme::W_BOLD)
                .text_color(theme::achievement_gold())
                .child(icon(Icon::Trophy, px(15.), theme::achievement_gold()))
                .child(match when {
                    Some(d) => tr!("100% · Completed {date}", date = d),
                    None => tr!("100% · Completed").to_string(),
                }),
        );
    }
    if !recent.is_empty() {
        body = body.child(caption(tr!("Recently unlocked"))).children(recent.into_iter().map(row));
    }
    if !next.is_empty() {
        body = body.child(caption(tr!("Up next"))).children(next.into_iter().map(row));
    }
    let status = if refreshing { tr!("Updating…").to_string() } else { synced_label(set.fetched_at) };
    let body = body.child(
        div()
            .mt(px(12.))
            .pt(px(10.))
            .border_t_1()
            .border_color(theme::ink_a(0.06))
            .flex()
            .items_center()
            .justify_between()
            .gap(px(8.))
            .text_size(px(12.))
            .child(
                div()
                    .id("ach-view-all")
                    .cursor_pointer()
                    .text_color(theme::accent())
                    .hover(|s| s.underline())
                    .child(tr!("View all {n} →", n = total))
                    .on_click({
                        let open = actions.view_all.clone();
                        move |_, w, cx| open(w, cx)
                    }),
            )
            .child(
                div()
                    .id("ach-refresh")
                    .flex()
                    .items_center()
                    .gap(px(5.))
                    .text_color(theme::ink_a(0.45))
                    .when(!refreshing, |d| d.cursor_pointer().hover(|s| s.text_color(theme::text_bright())))
                    .child(status)
                    .when(!refreshing, |d| d.child(icon(Icon::Refresh, px(13.), theme::ink_a(0.45))))
                    .on_click({
                        let refresh = actions.refresh.clone();
                        move |_, w, cx| refresh(w, cx)
                    }),
            ),
    );
    section(Some(count.into_any_element()), body)
}

/// The title above the panel, like the other sections ("Details",
/// "Timeline"), with `meta` on its right.
fn section(meta: Option<AnyElement>, panel: Div) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .child(h3(tr!("Achievements")).flex().justify_between().items_center().gap(px(12.8)).children(meta))
        .child(panel)
        .into_any_element()
}

/// A panel with only a message and a button (no set to show).
fn notice(source: Source, text: &str, detail: Option<SharedString>, button: Stateful<Div>) -> AnyElement {
    let body = panel(false)
        .child(
            div()
                .flex()
                .items_start()
                .gap(px(12.))
                .child(div().size(px(40.)).flex_none().flex().items_center().justify_center().rounded(px(9.)).bg(gold_a(0.16)).child(icon(
                    Icon::Trophy,
                    px(22.),
                    theme::achievement_gold(),
                )))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .gap(px(4.))
                        .child(div().text_size(px(12.8)).text_color(theme::ink_a(0.72)).child(text.to_string()))
                        .children(detail.map(|d| div().text_size(px(11.5)).text_color(theme::ink_a(0.45)).child(d))),
                ),
        )
        .child(div().mt(px(12.)).flex().child(button));
    section(Some(source_chip(source).into_any_element()), body)
}

fn caption(text: &str) -> Div {
    div().mt(px(14.)).mb(px(4.)).text_size(px(11.)).font_weight(theme::W_BOLD).text_color(theme::ink_a(0.45)).child(text.to_uppercase())
}

fn row(r: Row) -> Div {
    let right = div().flex_none().flex().flex_col().items_end().gap(px(4.)).text_size(px(11.5)).text_color(theme::ink_a(0.55));
    let right = match (r.unlocked, r.progress) {
        (false, Some(f)) => right.children(r.right.clone()).child(div().w(px(54.)).child(bar(f, false, 4.))),
        _ => right.children(r.right.clone()).children(r.rarity.map(rarity_pill)),
    };
    div()
        .flex()
        .items_center()
        .gap(px(11.))
        .py(px(6.))
        .min_w_0()
        .child(badge(r.image, r.unlocked, r.grey, ROW_ICON))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .child(div().text_size(px(13.33)).font_weight(theme::W_BOLD).text_color(theme::text_bright()).truncate().child(r.name))
                .child(div().text_size(px(12.)).text_color(theme::ink_a(0.55)).truncate().child(r.description)),
        )
        .child(right)
}
