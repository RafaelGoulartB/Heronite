//! "All achievements" of a game: every achievement of the set the focus
//! panel shows, filtered (all, unlocked, locked) and searched, the latest
//! unlock or the rarest first, with the date, rarity, points and progress of
//! each. Secret ones keep their text hidden until revealed one by one. It
//! follows `services::achievements`, so a refresh shows at once.

use std::collections::HashSet;

use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_achievements::sources::retro::GameHit;
use heronite_achievements::{Achievement, AchievementSet, Kind, Problem, Source};
use heronite_i18n::tr;

use super::art_images::DialogImages;
use super::{DialogCtx, DialogEvent};
use crate::icons::Icon;
use crate::services::achievements as service;
use crate::theme;
use crate::ui::achievements::{badge, bar, long_day, rarity_pill, source_chip, synced_label};
use crate::ui::controls::{button_sm, tabs, ButtonKind};
use crate::ui::dialog::{dialog_scrolled, DialogProps};
use crate::ui::fields::{new_input, search_box};
use crate::ui::icon;

const ICON: f32 = 44.;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Filter {
    All,
    Unlocked,
    Locked,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Sort {
    /// Unlocked ones by date (latest first), then the closest locked ones.
    Recent,
    Rarest,
}

pub struct AchievementsDialog {
    /// `runner_appName`.
    key: String,
    title: String,
    images: Entity<DialogImages>,
    search: Entity<InputState>,
    filter: Filter,
    sort: Sort,
    /// Secret achievements revealed by hand (their ids).
    revealed: HashSet<String>,
    scroll: ScrollHandle,
    /// A ROM no set matched: the RetroAchievements games found for the
    /// search (`None` before the first search).
    hits: Option<Vec<GameHit>>,
    searching: bool,
}

impl EventEmitter<DialogEvent> for AchievementsDialog {}

impl AchievementsDialog {
    pub fn new(ctx: DialogCtx, key: String, title: String, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let images = cx.new(|cx| DialogImages::new(&ctx, cx));
        cx.observe(&images, |_, _, cx| cx.notify()).detach();
        let svc = service::get(cx);
        cx.observe(&svc, |_, _, cx| cx.notify()).detach();
        let search = new_input(window, cx, "", tr!("Search achievements"));
        cx.subscribe_in(&search, window, |this, _, ev: &InputEvent, _, cx| match ev {
            InputEvent::Change => cx.notify(),
            InputEvent::PressEnter { .. } => this.search_games(cx),
            _ => {}
        })
        .detach();
        Self {
            key,
            title,
            images,
            search,
            filter: Filter::All,
            sort: Sort::Recent,
            revealed: HashSet::new(),
            scroll: ScrollHandle::new(),
            hits: None,
            searching: false,
        }
    }

    /// Looks the search up among the RetroAchievements games (picking the
    /// game of a ROM no set matched).
    fn search_games(&mut self, cx: &mut Context<Self>) {
        let query = self.search.read(cx).value().trim().to_string();
        if query.is_empty() || self.searching {
            return;
        }
        let Some(job) = service::get(cx).read(cx).retro_search(&self.key, query) else { return };
        self.searching = true;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let hits = cx.background_spawn(async move { job() }).await;
            this.update(cx, |this, cx| {
                this.searching = false;
                this.hits = Some(hits);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// A ROM no set matched: what it is, a search and the games found.
    fn picker(&mut self, file: &str, hash: Option<&str>, cx: &mut Context<Self>) -> Div {
        let mut col = div()
            .flex()
            .flex_col()
            .gap(px(12.))
            .child(div().text_size(px(13.5)).child(tr!("This ROM doesn't match any RetroAchievements set.")))
            .child(div().text_size(px(12.)).text_color(theme::ink_a(0.55)).child(match hash {
                Some(h) => format!("{file} · MD5 {h}"),
                None => file.to_string(),
            }))
            .child(div().text_size(px(12.5)).text_color(theme::ink_a(0.72)).child(tr!(
                "Sets accept specific dumps: a patched, translated or other-region file has none. Search for the game to use its set anyway (Enter searches)."
            )))
            .child(
                div().flex().gap(px(10.)).child(search_box(&self.search)).child(
                    button_sm("ach-pick-search", ButtonKind::Glass, if self.searching { tr!("Searching…") } else { tr!("Search") })
                        .on_click(cx.listener(|this, _, _, cx| this.search_games(cx))),
                ),
            );
        match &self.hits {
            Some(hits) if hits.is_empty() => {
                col = col.child(div().text_size(px(12.5)).text_color(theme::ink_a(0.5)).child(tr!("No game found.")));
            }
            Some(hits) => {
                for hit in hits.clone() {
                    let key = self.key.clone();
                    col = col.child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(12.))
                            .py(px(8.))
                            .border_t_1()
                            .border_color(theme::ink_a(0.06))
                            .child(div().flex_1().min_w_0().truncate().text_size(px(13.)).child(hit.title.clone()))
                            .child(div().text_size(px(12.)).text_color(theme::ink_a(0.55)).child(hit.console.clone()))
                            .child(
                                button_sm(SharedString::from(format!("ach-pick-{}", hit.id)), ButtonKind::Primary, tr!("Use this game"))
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.hits = None;
                                        service::get(cx).update(cx, |s, cx| s.pin_retro_game(&key, Some(hit.id), cx));
                                    })),
                            ),
                    );
                }
            }
            None => {}
        }
        col
    }

    /// The achievements to list, in order.
    fn listed<'a>(&self, set: &'a AchievementSet, query: &str) -> Vec<&'a Achievement> {
        let query = query.trim().to_lowercase();
        let matches = |a: &Achievement| {
            query.is_empty() || {
                let secret = a.is_secret() && !self.revealed.contains(&a.id);
                a.name.to_lowercase().contains(&query) || (!secret && a.description.to_lowercase().contains(&query))
            }
        };
        let mut list: Vec<&Achievement> = match self.sort {
            Sort::Recent => {
                set.recent().into_iter().chain(set.up_next()).chain(set.achievements.iter().filter(|a| !a.unlocked && a.hidden)).collect()
            }
            Sort::Rarest => {
                let mut v: Vec<&Achievement> = set.achievements.iter().collect();
                v.sort_by(|a, b| a.rarity.unwrap_or(101.).total_cmp(&b.rarity.unwrap_or(101.)));
                v
            }
        };
        list.retain(|a| match self.filter {
            Filter::All => true,
            Filter::Unlocked => a.unlocked,
            Filter::Locked => !a.unlocked,
        });
        list.retain(|a| matches(a));
        list
    }

    fn row(&mut self, a: &Achievement, hide_secrets: bool, cx: &mut Context<Self>) -> Div {
        let secret = hide_secrets && a.is_secret() && !self.revealed.contains(&a.id);
        let url = a.icon_now().to_string();
        let image = (!url.is_empty()).then(|| {
            let chain = if url.starts_with("http") {
                vec![heronite_images::ImageRef::Url(url.clone())]
            } else {
                vec![heronite_images::ImageRef::File(url.clone().into())]
            };
            self.images.update(cx, |im, _| im.get(&format!("ach:{url}"), chain, ICON, ICON))
        });
        let grey = !a.unlocked && a.icon_locked.is_empty();
        let name: SharedString = if secret { tr!("Secret achievement").into() } else { a.name.clone().into() };
        let description: AnyElement = if secret {
            let id = a.id.clone();
            div()
                .flex()
                .gap(px(6.))
                .child(tr!("Hidden until you unlock it."))
                .child(
                    div()
                        .id(SharedString::from(format!("reveal-{id}")))
                        .cursor_pointer()
                        .text_color(theme::accent())
                        .child(tr!("Reveal"))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.revealed.insert(id.clone());
                            cx.notify();
                        })),
                )
                .into_any_element()
        } else {
            div().child(a.description.clone()).into_any_element()
        };
        let mut meta = div().flex().flex_wrap().items_center().gap(px(6.)).mt(px(7.)).text_size(px(11.5)).text_color(theme::ink_a(0.5));
        if let Some(r) = a.rarity {
            let tier = theme::rarity_tier(r);
            meta = meta.child(rarity_pill(r)).child(div().text_color(theme::rarity_colour(tier)).child(tier_label(tier)));
        }
        if let Some(p) = a.points {
            meta = meta.child(pill(tr!("{n} points", n = p)));
        }
        if a.unlocked && a.hardcore {
            meta = meta.child(pill(tr!("Hardcore").to_string()));
        }
        if let Some(k) = a.kind {
            meta = meta.child(pill(kind_label(k).to_string()));
        }
        meta = meta.child(div().flex_1());
        if a.unlocked {
            if let Some(t) = a.unlocked_at {
                let local = t.with_timezone(&chrono::Local);
                meta = meta.child(format!("{} · {}", long_day(local.date_naive()), local.format("%H:%M")));
            }
        } else if let Some(p) = a.progress.filter(|p| p.max > 1) {
            meta = meta.child(div().w(px(90.)).child(bar(p.fraction(), false, 4.))).child(format!("{}/{}", p.current.min(p.max), p.max));
        }
        div()
            .flex()
            .items_start()
            .gap(px(12.))
            .py(px(10.))
            .border_t_1()
            .border_color(theme::ink_a(0.06))
            .child(badge(image.flatten(), a.unlocked, grey, ICON))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .child(div().text_size(px(13.6)).font_weight(theme::W_BOLD).text_color(theme::text_bright()).child(name))
                    .child(div().mt(px(2.)).text_size(px(12.3)).text_color(theme::ink_a(0.62)).child(description))
                    .child(meta),
            )
    }

    fn header(&self, set: &AchievementSet, refreshing: bool, cx: &mut Context<Self>) -> Div {
        let (unlocked, total) = (set.unlocked(), set.total());
        let pct = (unlocked * 100).checked_div(total).unwrap_or(0);
        let mut counts = format!("{unlocked}/{total} · {pct}%");
        if let Some((got, all)) = set.points() {
            counts.push_str(&format!(" · {}", tr!("{got}/{all} points", got, all)));
        }
        let key = self.key.clone();
        div()
            .flex()
            .flex_col()
            .gap(px(12.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .text_size(px(12.))
                    .text_color(theme::ink_a(0.55))
                    .child(source_chip(set.source))
                    .children(set.title.clone().filter(|t| *t != self.title).map(|t| div().truncate().child(t)))
                    .child(if refreshing { tr!("Updating…").to_string() } else { synced_label(set.fetched_at) })
                    .child(div().flex_1())
                    .child(button_sm("ach-dialog-refresh", ButtonKind::Glass, tr!("Refresh")).on_click(cx.listener(move |_, _, _, cx| {
                        service::get(cx).update(cx, |s, cx| s.refresh(&key, cx));
                    }))),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .child(div().flex_1().child(bar(pct as f32 / 100., set.is_complete(), 6.)))
                    .child(div().text_size(px(13.)).font_weight(theme::W_BOLD).child(counts)),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .child(tabs(
                        "ach-filter",
                        &[
                            (Filter::All, tr!("All {n}", n = total)),
                            (Filter::Unlocked, tr!("Unlocked {n}", n = unlocked)),
                            (Filter::Locked, tr!("Locked {n}", n = total - unlocked)),
                        ],
                        self.filter,
                        {
                            let weak = cx.entity().downgrade();
                            move |f, _, cx| {
                                weak.update(cx, |this, cx| {
                                    this.filter = f;
                                    cx.notify();
                                })
                                .ok();
                            }
                        },
                    ))
                    .child(tabs(
                        "ach-sort",
                        &[(Sort::Recent, tr!("Recent first").to_string()), (Sort::Rarest, tr!("Rarest first").to_string())],
                        self.sort,
                        {
                            let weak = cx.entity().downgrade();
                            move |s, _, cx| {
                                weak.update(cx, |this, cx| {
                                    this.sort = s;
                                    cx.notify();
                                })
                                .ok();
                            }
                        },
                    ))
                    .child(search_box(&self.search)),
            )
    }
}

fn pill(text: String) -> Div {
    div().px(px(7.)).rounded_full().bg(theme::ink_a(0.08)).text_color(theme::ink_a(0.78)).child(text)
}

fn tier_label(tier: theme::RarityTier) -> &'static str {
    match tier {
        theme::RarityTier::UltraRare => tr!("Ultra rare"),
        theme::RarityTier::Rare => tr!("Rare"),
        theme::RarityTier::Uncommon => tr!("Uncommon"),
        theme::RarityTier::Common => tr!("Common"),
    }
}

fn kind_label(kind: Kind) -> &'static str {
    match kind {
        Kind::Progression => tr!("Progression"),
        Kind::WinCondition => tr!("Win condition"),
        Kind::Missable => tr!("Missable"),
    }
}

impl Render for AchievementsDialog {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.images.update(cx, |im, _| im.frame(window));
        let (entry, hide_secrets) = {
            let svc = service::get(cx);
            let svc = svc.read(cx);
            (svc.entry(&self.key).cloned(), svc.prefs().hide_secrets)
        };
        let set = entry.as_ref().and_then(|e| e.primary()).cloned();
        let refreshing = entry.as_ref().is_some_and(|e| e.refreshing);
        let mut body = div().flex().flex_col().gap(px(14.)).text_color(theme::text_primary());
        match &set {
            Some(set) => {
                let query = self.search.read(cx).value().to_string();
                body = body.child(self.header(set, refreshing, cx));
                let list = self.listed(set, &query);
                let mut rows = div().flex().flex_col();
                if list.is_empty() {
                    rows = rows.child(
                        div().py(px(24.)).flex().justify_center().text_color(theme::ink_a(0.5)).child(tr!("No achievements match.")),
                    );
                }
                for a in list.into_iter().cloned().collect::<Vec<_>>() {
                    rows = rows.child(self.row(&a, hide_secrets, cx));
                }
                body = body.child(rows);
            }
            None => match entry.as_ref().and_then(|e| e.problem()).cloned() {
                Some((_, Problem::Unmatched { file, hash })) => body = body.child(self.picker(&file, hash.as_deref(), cx)),
                problem => {
                    let text = match problem {
                        Some((Source::RetroAchievements, Problem::NeedsAccount)) => {
                            tr!("Connect your RetroAchievements account in Settings → Achievements to see this game's achievements.")
                        }
                        _ if refreshing => tr!("Reading the achievements…"),
                        _ => tr!("No achievements to show yet."),
                    };
                    body = body.child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(10.))
                            .text_color(theme::ink_a(0.6))
                            .child(icon(Icon::Trophy, px(18.), theme::ink_a(0.6)))
                            .child(text),
                    );
                }
            },
        }
        let pinned = entry.as_ref().is_some_and(|e| e.game.retro_game_id.is_some())
            && set.as_ref().is_some_and(|s| s.source == Source::RetroAchievements);
        if pinned {
            let key = self.key.clone();
            body = body.child(
                div().flex().gap(px(8.)).text_size(px(12.)).text_color(theme::ink_a(0.5)).child(tr!("The game was picked by hand.")).child(
                    div().id("ach-unpin").cursor_pointer().text_color(theme::accent()).child(tr!("Match by the file again")).on_click(
                        cx.listener(move |_, _, _, cx| {
                            service::get(cx).update(cx, |s, cx| s.pin_retro_game(&key, None, cx));
                        }),
                    ),
                ),
            );
        }
        if self.scroll.bounds().size.height <= px(0.) {
            cx.notify();
        }
        let entity = cx.entity().downgrade();
        let title = tr!("{game} — Achievements", game = self.title.as_str());
        dialog_scrolled(
            DialogProps { id: "achievements".into(), title: title.into(), width: 760., height: Some(620.) },
            &self.scroll,
            body.into_any_element(),
            None,
            move |_, cx| {
                entity.update(cx, |_, cx| cx.emit(DialogEvent::Close)).ok();
            },
        )
    }
}
