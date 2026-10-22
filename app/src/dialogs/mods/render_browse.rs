//! Browse tab: the site switch, search box, sort and the results.

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_i18n::{grouped, tr};
use heronite_images::ImageRef;
use heronite_mods::nexus::{SearchMod, SearchSort};
use heronite_mods::thunderstore::{Listing, Ordering};
use heronite_mods::{ModEntry, ModSource};

use super::browse::Source;
use super::{format, ModsDialog, Tab, View};
use crate::dialogs::art_images::picture;
use crate::icons::Icon;
use crate::services::widgets::busy_bar;
use crate::theme;
use crate::ui::controls::{button, button_sm, card_box, tabs, ButtonKind};
use crate::ui::fields;
use crate::ui::icon;

/// A result's state against the installed list.
enum Have {
    No,
    Installed,
    Update,
    /// Asked for here and not in the list yet.
    Queued,
}

impl ModsDialog {
    pub(super) fn render_browse(&mut self, v: Option<&View>, _window: &mut Window, cx: &mut Context<Self>) -> Div {
        let Some(v) = v else { return self.render_loading() };
        let sources = Self::sources(v);
        if sources.is_empty() {
            return self.unlinked(cx);
        }
        let mut col = div().flex().flex_col().gap(px(12.));

        let mut top = div().flex().items_center().gap(px(10.));
        if sources.len() > 1 {
            top = top.child(tabs(
                "mods-src",
                &[(Source::Nexus, tr!("Nexus Mods")), (Source::Thunderstore, tr!("Thunderstore"))],
                self.browse.source,
                {
                    let entity = cx.entity().downgrade();
                    move |s, _, cx| {
                        entity.update(cx, |this, cx| this.set_source(s, cx)).ok();
                    }
                },
            ));
        }
        top = top.child(fields::search_box(&self.browse.input));
        col = col.child(top).child(self.sort_pills(cx));

        if self.browse.source == Source::Nexus && !v.premium() {
            col = col.child(
                div()
                    .flex()
                    .items_start()
                    .gap(px(10.))
                    .px(px(12.))
                    .py(px(9.))
                    .rounded(px(12.))
                    .bg(theme::white_a(0.05))
                    .text_size(px(12.5))
                    .line_height(px(17.))
                    .text_color(theme::text_muted())
                    .child(icon(Icon::InfoOutlined, px(16.), theme::text_muted()))
                    .child(
                        div().flex_1().child(tr!("Free accounts: click \"Mod manager download\" on the site and Heronite installs it.")),
                    ),
            );
        }
        match self.browse.source {
            Source::Nexus => col.child(self.nexus_results(v, cx)),
            Source::Thunderstore => col.child(self.ts_results(v, cx)),
        }
    }

    fn unlinked(&mut self, cx: &mut Context<Self>) -> Div {
        div()
            .flex()
            .flex_col()
            .items_center()
            .gap(px(12.))
            .px(px(30.))
            .py(px(40.))
            .child(
                div().max_w(px(420.)).text_center().text_size(px(14.)).line_height(px(20.)).text_color(theme::text_muted()).child(tr!(
                    "This game is not linked to Nexus Mods or Thunderstore yet. Link it in the Game tab to search for mods here."
                )),
            )
            .child(button("mods-browse-link", ButtonKind::Glass, tr!("Open the Game tab")).on_click(cx.listener(|this, _, _, cx| {
                this.show_tab(Tab::Game, cx);
            })))
    }

    fn sort_pills(&mut self, cx: &mut Context<Self>) -> Div {
        let entity = cx.entity().downgrade();
        match self.browse.source {
            Source::Nexus => tabs(
                "mods-sort-n",
                &[
                    (SearchSort::Endorsements, tr!("Most endorsed")),
                    (SearchSort::Downloads, tr!("Most downloaded")),
                    (SearchSort::Updated, tr!("Recently updated")),
                ],
                self.browse.nexus_sort,
                move |s, _, cx| {
                    entity
                        .update(cx, |this, cx| {
                            this.browse.nexus_sort = s;
                            this.restart_search(cx);
                        })
                        .ok();
                },
            ),
            Source::Thunderstore => tabs(
                "mods-sort-t",
                &[
                    (Ordering::MostDownloaded, tr!("Most downloaded")),
                    (Ordering::TopRated, tr!("Top rated")),
                    (Ordering::LastUpdated, tr!("Last updated")),
                    (Ordering::Newest, tr!("Newest")),
                ],
                self.browse.ts_order,
                move |s, _, cx| {
                    entity
                        .update(cx, |this, cx| {
                            this.browse.ts_order = s;
                            this.restart_search(cx);
                        })
                        .ok();
                },
            ),
        }
    }

    fn nexus_results(&mut self, v: &View, cx: &mut Context<Self>) -> Div {
        let r = &self.browse.nexus;
        let (loading, error, more, total, loaded) = (r.loading, r.error.clone(), r.more, r.total, r.loaded);
        let items: Vec<SearchMod> = r.items.clone();
        let mut list = card_box().px(px(8.)).py(px(4.));
        for (i, m) in items.iter().enumerate() {
            if i > 0 {
                list = list.child(div().h(px(1.)).mx(px(8.)).bg(theme::row_separator()));
            }
            let have = have_nexus(v, &self.browse.queued, m.mod_id);
            let domain = v.g.info.nexus_domain.clone().unwrap_or_default();
            let (id, premium) = (m.mod_id, v.premium());
            let action = match have {
                Have::No if premium => action_button(&format!("mods-get-n{id}"), tr!("Install"), ButtonKind::Primary).on_click(
                    cx.listener(move |this, _, _, cx| {
                        this.browse.queued.insert(format!("n:{id}"));
                        let key = this.key.clone();
                        this.service.update(cx, |s, cx| s.install_nexus(&key, id, None, cx));
                        cx.notify();
                    }),
                ),
                Have::No => action_button(&format!("mods-get-n{id}"), tr!("Download on Nexus Mods"), ButtonKind::Glass).on_click(
                    cx.listener(move |this, _, _, cx| {
                        this.open_url(format!("https://www.nexusmods.com/{domain}/mods/{id}?tab=files"), cx);
                    }),
                ),
                other => state_button(&format!("mods-get-n{id}"), &other, cx, move |this, cx| {
                    let key = this.key.clone();
                    let entry = this.service.read(cx).game(&key).and_then(|g| {
                        g.list
                            .mods
                            .iter()
                            .find(|m| matches!(&m.source, ModSource::Nexus { mod_id, .. } if *mod_id == id))
                            .map(|m| m.id.clone())
                    });
                    if let Some(entry) = entry {
                        this.service.update(cx, |s, cx| s.update(&key, &entry, cx));
                    }
                }),
            };
            let meta = format!(
                "{} \u{b7} {} \u{b7} {}",
                tr!("by {author}", author = m.author.clone()),
                tr!("{n} downloads", n = format::compact(m.downloads)),
                tr!("{n} endorsements", n = format::compact(m.endorsements)),
            );
            let thumb = m.thumbnail.clone().or(m.picture.clone());
            list = list.child(self.result_row(&format!("n{id}"), thumb, &m.name, meta, &m.summary, action, cx));
        }
        results_frame(list, items.is_empty(), loading, error, loaded, more, total, cx)
    }

    fn ts_results(&mut self, v: &View, cx: &mut Context<Self>) -> Div {
        let r = &self.browse.ts;
        let (loading, error, more, total, loaded) = (r.loading, r.error.clone(), r.more, r.total, r.loaded);
        let items: Vec<Listing> = r.items.clone();
        let mut list = card_box().px(px(8.)).py(px(4.));
        for (i, l) in items.iter().enumerate() {
            if i > 0 {
                list = list.child(div().h(px(1.)).mx(px(8.)).bg(theme::row_separator()));
            }
            let (ns, name) = (l.namespace.clone(), l.name.clone());
            let key = format!("{}-{}", l.namespace, l.name);
            let have = have_ts(v, &self.browse.queued, &l.namespace, &l.name);
            let id = format!("mods-get-t{key}");
            let action = match have {
                Have::No => action_button(&id, tr!("Install"), ButtonKind::Primary).on_click(cx.listener({
                    let (ns, name, key) = (ns.clone(), name.clone(), key.clone());
                    move |this, _, _, cx| {
                        this.browse.queued.insert(format!("t:{key}"));
                        let game = this.key.clone();
                        this.service.update(cx, |s, cx| s.install_thunderstore(&game, &ns, &name, cx));
                        cx.notify();
                    }
                })),
                other => state_button(&id, &other, cx, move |this, cx| {
                    let game = this.key.clone();
                    let entry = this.service.read(cx).game(&game).and_then(|g| {
                        g.list
                            .mods
                            .iter()
                            .find(|m| matches!(&m.source, ModSource::Thunderstore { namespace, name: n, .. } if *namespace == ns && *n == name))
                            .map(|m| m.id.clone())
                    });
                    if let Some(entry) = entry {
                        this.service.update(cx, |s, cx| s.update(&game, &entry, cx));
                    }
                }),
            };
            let meta = format!(
                "{} \u{b7} {} \u{b7} {}",
                tr!("by {author}", author = l.namespace.clone()),
                tr!("{n} downloads", n = format::compact(l.download_count)),
                tr!("{n} likes", n = format::compact(l.rating_count)),
            );
            let thumb = (!l.icon_url.is_empty()).then(|| l.icon_url.clone());
            list = list.child(self.result_row(&format!("t{key}"), thumb, &l.name.replace('_', " "), meta, &l.description, action, cx));
        }
        results_frame(list, items.is_empty(), loading, error, loaded, more, total, cx)
    }

    fn result_row(
        &mut self,
        id: &str,
        thumb: Option<String>,
        name: &str,
        meta: String,
        summary: &str,
        action: Stateful<Div>,
        cx: &mut Context<Self>,
    ) -> Div {
        let src = thumb.and_then(|u| self.images.update(cx, |im, _| im.get(&format!("br:{id}"), vec![ImageRef::Url(u)], 60., 60.)));
        div()
            .flex()
            .items_start()
            .gap(px(14.))
            .px(px(6.))
            .py(px(11.))
            .child(picture(src).size(px(60.)).flex_none().rounded(px(12.)))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .gap(px(2.))
                    .child(div().truncate().text_size(px(14.5)).font_weight(theme::W_500).child(name.to_string()))
                    .child(div().truncate().text_size(px(12.)).text_color(theme::text_muted()).child(meta))
                    .child(
                        div()
                            .mt(px(2.))
                            .line_clamp(2)
                            .text_ellipsis()
                            .text_size(px(12.5))
                            .line_height(px(17.))
                            .text_color(theme::text_dim())
                            .child(summary.to_string()),
                    ),
            )
            .child(div().flex_none().pt(px(4.)).child(action))
    }
}

fn have_nexus(v: &View, queued: &std::collections::HashSet<String>, mod_id: u64) -> Have {
    let found = v.g.list.mods.iter().find(|m| matches!(&m.source, ModSource::Nexus { mod_id: id, .. } if *id == mod_id));
    have(found, queued.contains(&format!("n:{mod_id}")) && ModsDialog::busy(v))
}

fn have_ts(v: &View, queued: &std::collections::HashSet<String>, ns: &str, name: &str) -> Have {
    let found =
        v.g.list
            .mods
            .iter()
            .find(|m| matches!(&m.source, ModSource::Thunderstore { namespace, name: n, .. } if namespace == ns && n == name));
    have(found, queued.contains(&format!("t:{ns}-{name}")) && ModsDialog::busy(v))
}

fn have(found: Option<&ModEntry>, queued: bool) -> Have {
    match found {
        Some(m) if m.update.is_some() => Have::Update,
        Some(_) => Have::Installed,
        None if queued => Have::Queued,
        None => Have::No,
    }
}

fn action_button(id: &str, label: &'static str, kind: ButtonKind) -> Stateful<Div> {
    button_sm(SharedString::from(id.to_string()), kind, label)
}

/// "Installed" (quiet), "Installing…" or "Update" for a mod we already have.
fn state_button(
    id: &str,
    have: &Have,
    cx: &mut Context<ModsDialog>,
    update: impl Fn(&mut ModsDialog, &mut Context<ModsDialog>) + 'static,
) -> Stateful<Div> {
    let id = SharedString::from(id.to_string());
    match have {
        Have::Update => button_sm(id, ButtonKind::Primary, tr!("Update")).on_click(cx.listener(move |this, _, _, cx| update(this, cx))),
        Have::Queued => button_sm(id, ButtonKind::Ghost, tr!("Installing\u{2026}")).opacity(0.7),
        _ => button_sm(id, ButtonKind::Ghost, tr!("Installed")).opacity(0.7),
    }
}

/// The list with what goes around it: loading, errors, no results, "Load more".
fn results_frame(
    list: Div,
    empty: bool,
    loading: bool,
    error: Option<String>,
    loaded: bool,
    more: bool,
    total: u64,
    cx: &mut Context<ModsDialog>,
) -> Div {
    let mut col = div().flex().flex_col().gap(px(10.));
    if let Some(e) = error {
        return col.child(
            super::render_jobs::tinted(super::render_jobs::Tone::Error, Icon::Warning)
                .child(div().flex_1().min_w_0().text_size(px(13.)).line_height(px(18.)).text_color(theme::text_primary()).child(e))
                .child(button_sm("mods-browse-retry", ButtonKind::Glass, tr!("Try again")).self_center().on_click(cx.listener(
                    |this, _, _, cx| {
                        this.restart_search(cx);
                    },
                ))),
        );
    }
    if empty && loaded && !loading {
        return col.child(
            div().py(px(40.)).text_center().text_size(px(14.)).text_color(theme::text_dim()).child(tr!("No mods found. Try other words.")),
        );
    }
    if !empty {
        col = col
            .child(div().mx(px(6.)).text_size(px(12.)).text_color(theme::text_dim()).child(tr!("{n} results", n = grouped(total as i64))));
        col = col.child(list);
    }
    if loading {
        col = col.child(
            div()
                .flex()
                .flex_col()
                .items_center()
                .gap(px(10.))
                .py(px(if empty { 40. } else { 10. }))
                .child(div().text_size(px(13.)).text_color(theme::text_dim()).child(tr!("Searching\u{2026}")))
                .child(div().w(px(160.)).child(busy_bar("mods-browse-busy", 4.))),
        );
    } else if more {
        col = col.child(div().flex().justify_center().child(button("mods-browse-more", ButtonKind::Glass, tr!("Load more")).on_click(
            cx.listener(|this, _, _, cx| {
                this.run_search(true, cx);
            }),
        )));
    }
    col
}
