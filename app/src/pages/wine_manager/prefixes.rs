//! "Prefixes" in the Wine Manager: the Wine prefixes on this machine with
//! their size and the games that use them; create, duplicate, move to the
//! trash (see `heronite_wine::prefixes`). Shown in place of a source's builds.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use gpui_kit::component::input::InputState;
use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::clients::Client;
use heronite_data::proc::Cancel;
use heronite_i18n::tr;
use heronite_wine::prefixes::{self, Prefix, PrefixKind};

use super::render::fmt_size;
use super::WineManagerPage;
use crate::icons::Icon;
use crate::pages::{icon_action, status_pill};
use crate::theme;
use crate::ui::controls::{button_sm, card_box, ButtonKind};
use crate::ui::fields;
use crate::ui::icon;

pub(super) struct PrefixesState {
    /// `None` until the first list is read.
    pub(super) list: Option<Vec<Prefix>>,
    /// App name -> (title, Collection key) of the library's games that run
    /// through Wine: only they use a prefix.
    titles: HashMap<String, (String, String)>,
    /// Bytes on disk, filled in one by one after the list.
    sizes: HashMap<PathBuf, u64>,
    pub(super) new_name: Entity<InputState>,
    /// Prefix whose "Duplicate" form is open, with the name input.
    duplicate: Option<(PathBuf, Entity<InputState>)>,
    /// Prefix being copied (one at a time).
    copying: Option<(PathBuf, Arc<Cancel>)>,
    confirm_trash: Option<PathBuf>,
    loading: bool,
}

impl PrefixesState {
    pub(super) fn new(window: &mut Window, cx: &mut App) -> Self {
        Self {
            list: None,
            titles: HashMap::new(),
            sizes: HashMap::new(),
            new_name: fields::new_input(window, cx, "", tr!("New prefix name")),
            duplicate: None,
            copying: None,
            confirm_trash: None,
            loading: false,
        }
    }

    /// Esc: drops the open question or Duplicate form. True when one was open.
    pub(super) fn close_question(&mut self) -> bool {
        self.confirm_trash.take().is_some() || self.duplicate.take().is_some()
    }
}

/// Whether a game of the library runs through Wine in a prefix of its own
/// (emulated games, Steam games and the ones another launcher or a store
/// client starts do not, whatever their platform).
fn runs_through_wine(g: &heronite_data::library::CollectionGame) -> bool {
    let windows = g.game.install.platform.as_deref().is_some_and(|p| p.eq_ignore_ascii_case("windows"));
    windows
        && !matches!(
            g.launch_kind.as_deref(),
            Some("emulator" | "steam-uri" | heronite_data::launchers::LAUNCH_KIND | heronite_data::clients::LAUNCH_KIND)
        )
}

/// The store client whose prefix `p` is (its settings file is what sets it).
fn client_of(p: &Prefix) -> Option<Client> {
    p.games.iter().find_map(|app| Client::from_config_app(app))
}

fn home() -> String {
    std::env::var("HOME").unwrap_or_else(|_| "/".into())
}

fn open_folder(path: &std::path::Path) {
    if let Err(e) = std::process::Command::new("xdg-open").arg(path).spawn() {
        log::warn!("xdg-open {}: {e}", path.display());
    }
}

impl WineManagerPage {
    /// Reads the prefixes (and the game titles) off the UI thread, then their sizes.
    pub(super) fn load_prefixes(&mut self, cx: &mut Context<Self>) {
        if self.prefixes.loading {
            return;
        }
        self.prefixes.loading = true;
        let data_paths = self.ctx.data_paths.clone();
        let need_titles = self.prefixes.titles.is_empty();
        cx.spawn(async move |this, cx| {
            let (list, titles) = cx
                .background_spawn(async move {
                    let list = prefixes::list(&data_paths, &home());
                    let titles: HashMap<String, (String, String)> = if need_titles {
                        heronite_data::library::Library::load(&data_paths)
                            .map(|lib| {
                                lib.games
                                    .iter()
                                    .filter(|g| runs_through_wine(g))
                                    .map(|g| (g.app_name.clone(), (g.title.clone(), g.key.clone())))
                                    .collect()
                            })
                            .unwrap_or_default()
                    } else {
                        HashMap::new()
                    };
                    (list, titles)
                })
                .await;
            let paths: Vec<PathBuf> = list.iter().filter(|p| p.kind != PrefixKind::NotCreated).map(|p| p.path.clone()).collect();
            let ok = this.update(cx, |this, cx| {
                let st = &mut this.prefixes;
                st.loading = false;
                st.sizes.retain(|p, _| paths.contains(p));
                st.list = Some(list);
                if !titles.is_empty() {
                    st.titles = titles;
                }
                cx.notify();
            });
            if ok.is_err() {
                return;
            }
            for path in paths {
                let p = path.clone();
                let size = cx.background_spawn(async move { prefixes::size_of(&p) }).await;
                if this
                    .update(cx, |this, cx| {
                        this.prefixes.sizes.insert(path, size);
                        cx.notify();
                    })
                    .is_err()
                {
                    return;
                }
            }
        })
        .detach();
    }

    pub(super) fn create_prefix(&mut self, name: String, cx: &mut Context<Self>) {
        let data_paths = self.ctx.data_paths.clone();
        cx.spawn(async move |this, cx| {
            let res = cx.background_spawn(async move { prefixes::create(&data_paths, &home(), &name) }).await;
            this.update(cx, |this, cx| {
                match res {
                    Ok(path) => {
                        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                        this.toast(tr!("Prefix {name} created. Choose it in a game's Wine settings.", name), false, cx);
                    }
                    Err(e) => this.toast(format!("{e:#}"), true, cx),
                }
                this.load_prefixes(cx);
            })
            .ok();
        })
        .detach();
    }

    pub(super) fn duplicate_prefix(&mut self, source: PathBuf, name: String, cx: &mut Context<Self>) {
        if self.prefixes.copying.is_some() {
            return;
        }
        let cancel = Arc::new(Cancel::default());
        self.prefixes.copying = Some((source.clone(), cancel.clone()));
        self.prefixes.duplicate = None;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let res = cx.background_spawn(async move { prefixes::duplicate(&source, &name, &cancel) }).await;
            this.update(cx, |this, cx| {
                this.prefixes.copying = None;
                match res {
                    Ok(path) => {
                        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                        this.toast(tr!("Prefix copied to {name}", name), false, cx);
                    }
                    Err(e) => this.toast(format!("{e:#}"), true, cx),
                }
                this.load_prefixes(cx);
            })
            .ok();
        })
        .detach();
    }

    pub(super) fn trash_prefix(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        self.prefixes.confirm_trash = None;
        let data_paths = self.ctx.data_paths.clone();
        cx.spawn(async move |this, cx| {
            let p = path.clone();
            let res = cx.background_spawn(async move { prefixes::trash(&data_paths, &home(), &p) }).await;
            this.update(cx, |this, cx| {
                match res {
                    Ok(()) => {
                        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                        this.toast(tr!("{name} moved to the trash", name), false, cx);
                    }
                    Err(e) => this.toast(format!("{e:#}"), true, cx),
                }
                this.load_prefixes(cx);
            })
            .ok();
        })
        .detach();
    }

    /// The prefix being copied.
    pub(super) fn prefixes_copying(&self) -> Option<PathBuf> {
        self.prefixes.copying.as_ref().map(|(p, _)| p.clone())
    }

    pub(super) fn cancel_prefix_copy(&self) {
        if let Some((_, c)) = &self.prefixes.copying {
            c.request();
        }
    }

    /// Title of a game (its app name when it is not in the library); a store
    /// client's settings file is named after the client.
    pub(super) fn game_title(&self, app_name: &str) -> String {
        if let Some(client) = Client::from_config_app(app_name) {
            return client.name().to_string();
        }
        self.prefixes.titles.get(app_name).map(|(t, _)| t.clone()).unwrap_or_else(|| app_name.to_string())
    }

    /// Size text of a prefix: on disk, measuring, or not created.
    pub(super) fn prefix_size(&self, p: &Prefix) -> String {
        match (p.kind, self.prefixes.sizes.get(&p.path)) {
            (PrefixKind::NotCreated, _) => tr!("Created on the next launch").to_string(),
            (_, Some(size)) => fmt_size(*size),
            (_, None) => tr!("Measuring\u{2026}").to_string(),
        }
    }

    /// The games of the library set to `p`, and the store client it is the
    /// prefix of (settings of removed games and Linux games do not count).
    fn users(&self, p: &Prefix) -> Vec<String> {
        p.games
            .iter()
            .filter(|g| self.prefixes.titles.contains_key(*g) || Client::from_config_app(g).is_some())
            .map(|g| self.game_title(g))
            .collect()
    }

    /// "Used by Control, Hollow Knight and 3 more" or why none is listed.
    pub(super) fn prefix_users(&self, p: &Prefix) -> String {
        let names = self.users(p);
        match names.len() {
            0 if p.default => tr!("Default for new games").to_string(),
            0 => tr!("No game uses it").to_string(),
            1..=3 => tr!("Used by {games}", games = names.join(", ")),
            n => tr!("Used by {games} and {count} more", games = names[..3].join(", "), count = n - 3),
        }
    }

    /// The prefixes matching the search.
    pub(super) fn visible_prefixes(&self) -> Vec<Prefix> {
        let q = &self.query;
        let list = self.prefixes.list.as_deref().unwrap_or_default();
        list.iter()
            // A prefix not created yet only matters when a game will create it.
            .filter(|p| p.kind != PrefixKind::NotCreated || p.default || !self.users(p).is_empty())
            .filter(|p| {
                q.is_empty()
                    || p.name.to_lowercase().contains(q)
                    || p.path.to_string_lossy().to_lowercase().contains(q)
                    || p.games.iter().any(|g| self.game_title(g).to_lowercase().contains(q))
            })
            .cloned()
            .collect()
    }

    pub(super) fn render_prefixes(&self, cx: &mut Context<Self>) -> Div {
        let new_row = div()
            .flex()
            .items_center()
            .gap(px(10.))
            .child(div().flex_1().max_w(px(460.)).child(fields::input_box(&self.prefixes.new_name, false)))
            .child(button_sm("prefix-create", ButtonKind::Glass, tr!("Create prefix")).on_click(cx.listener(|this, _, window, cx| {
                let name = this.prefixes.new_name.read(cx).value().trim().to_string();
                if name.is_empty() {
                    return;
                }
                this.prefixes.new_name.update(cx, |s, cx| s.set_value("", window, cx));
                this.create_prefix(name, cx);
            })));
        let dir = prefixes::new_prefix_dir(&self.ctx.data_paths, &home());
        let note = div()
            .px(px(6.))
            .text_size(px(12.))
            .line_height(px(17.))
            .text_color(theme::text_muted())
            .child(tr!("New prefixes go to {folder}. Wine fills a prefix the first time a game runs in it. Choose the prefix of a game in its Wine settings.", folder = dir.display()));
        let body = match &self.prefixes.list {
            None => message(tr!("Looking for prefixes\u{2026}")),
            Some(_) => {
                let rows = self.visible_prefixes();
                if rows.is_empty() {
                    message(tr!("No prefix matches your search."))
                } else {
                    let mut list = card_box();
                    for (i, p) in rows.iter().enumerate() {
                        if i > 0 {
                            list = list.child(div().h(px(1.)).w_full().bg(theme::row_separator()));
                        }
                        list = list.child(self.render_prefix(i, p, cx));
                    }
                    list
                }
            }
        };
        div().flex().flex_col().gap(px(12.)).w_full().child(new_row).child(note).child(body)
    }

    fn render_prefix(&self, i: usize, p: &Prefix, cx: &mut Context<Self>) -> Div {
        let st = &self.prefixes;
        let copying = st.copying.as_ref().is_some_and(|(c, _)| c == &p.path);
        let busy = st.copying.is_some();
        let kind = match p.kind {
            PrefixKind::Proton => status_pill("Proton", theme::accent()),
            PrefixKind::Wine => status_pill("Wine", theme::accent()),
            PrefixKind::NotCreated => status_pill(tr!("Not created yet"), theme::text_muted()),
        };
        let path = p.path.clone();
        let mut actions = div().flex().flex_none().items_center().gap(px(6.));
        if p.kind != PrefixKind::NotCreated {
            let open = path.clone();
            actions = actions.child(
                icon_action(SharedString::from(format!("prefix-open-{i}")), Icon::FolderOpen, true)
                    .on_click(move |_, _, _| open_folder(&open)),
            );
            let (src, name) = (path.clone(), tr!("{name} copy", name = p.name));
            actions = actions.child(icon_action(SharedString::from(format!("prefix-dup-{i}")), Icon::ContentCopy, !busy).on_click(
                cx.listener(move |this, _, window, cx| {
                    if this.prefixes.copying.is_none() {
                        let input = fields::new_input(window, cx, &name, tr!("Name of the copy"));
                        this.prefixes.duplicate = Some((src.clone(), input));
                        this.prefixes.confirm_trash = None;
                        cx.notify();
                    }
                }),
            ));
            // A store client keeps its prefix: its games are uninstalled in it.
            if !p.default && client_of(p).is_none() {
                let target = path.clone();
                actions = actions.child(icon_action(SharedString::from(format!("prefix-trash-{i}")), Icon::FaTrash, !copying).on_click(
                    cx.listener(move |this, _, _, cx| {
                        this.prefixes.confirm_trash = Some(target.clone());
                        this.prefixes.duplicate = None;
                        cx.notify();
                    }),
                ));
            }
        }
        let mut games = div().flex().flex_wrap().items_center().gap(px(6.)).text_size(px(12.)).text_color(theme::text_dim());
        games = games.child(self.prefix_users(p));
        let row = div()
            .flex()
            .items_center()
            .gap(px(12.))
            .py(px(12.))
            .child(icon(Icon::Folder, px(20.), theme::text_muted()))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(3.))
                    .flex_1()
                    .min_w_0()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .child(
                                div()
                                    .truncate()
                                    .text_size(px(14.))
                                    .font_weight(theme::W_BOLD)
                                    .text_color(theme::text_primary())
                                    .child(p.name.clone()),
                            )
                            .child(kind)
                            .when(p.default, |d| d.child(status_pill(tr!("Default"), theme::success()))),
                    )
                    .child(div().truncate().text_size(px(12.)).text_color(theme::text_muted()).child(p.path.display().to_string()))
                    .child(games),
            )
            .child(div().w(px(150.)).flex_none().text_right().text_size(px(13.)).text_color(theme::text_muted()).child(if copying {
                tr!("Copying\u{2026}").to_string()
            } else {
                self.prefix_size(p)
            }))
            .child(actions);

        let mut block = div().flex().flex_col().child(row);
        if copying {
            block = block.child(div().flex().justify_end().pb(px(10.)).child(
                button_sm("prefix-copy-cancel", ButtonKind::Ghost, tr!("Cancel")).on_click(cx.listener(|this, _, _, cx| {
                    this.cancel_prefix_copy();
                    cx.notify();
                })),
            ));
        }
        if let Some((src, input)) = st.duplicate.as_ref().filter(|(src, _)| src == &p.path) {
            let src = src.clone();
            block = block.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .pb(px(12.))
                    .child(div().text_size(px(13.)).text_color(theme::text_muted()).child(tr!("Copy as")))
                    .child(div().flex_1().child(fields::input_box(input, false)))
                    .child(button_sm("prefix-dup-go", ButtonKind::Primary, tr!("Duplicate")).on_click(cx.listener(
                        move |this, _, _, cx| {
                            let name =
                                this.prefixes.duplicate.as_ref().map(|(_, i)| i.read(cx).value().trim().to_string()).unwrap_or_default();
                            if !name.is_empty() {
                                this.duplicate_prefix(src.clone(), name, cx);
                            }
                        },
                    )))
                    .child(button_sm("prefix-dup-no", ButtonKind::Ghost, tr!("Cancel")).on_click(cx.listener(|this, _, _, cx| {
                        this.prefixes.duplicate = None;
                        cx.notify();
                    }))),
            );
        }
        if st.confirm_trash.as_ref() == Some(&p.path) {
            let warning = match self.users(p).len() {
                0 => tr!("Move {name} to the trash? You can restore it from there.", name = p.name),
                n => tr!(
                    "Move {name} to the trash? {count} games use it and get a new, empty prefix the next time they start. Their saves stored in the prefix go with it.",
                    name = p.name,
                    count = n
                ),
            };
            let target = path.clone();
            block = block.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .pb(px(12.))
                    .child(div().flex_1().text_size(px(13.)).line_height(px(18.)).text_color(theme::danger_text()).child(warning))
                    .child(
                        button_sm("prefix-trash-yes", ButtonKind::Danger, tr!("Move to trash"))
                            .on_click(cx.listener(move |this, _, _, cx| this.trash_prefix(target.clone(), cx))),
                    )
                    .child(button_sm("prefix-trash-no", ButtonKind::Ghost, tr!("Keep it")).on_click(cx.listener(|this, _, _, cx| {
                        this.prefixes.confirm_trash = None;
                        cx.notify();
                    }))),
            );
        }
        block
    }
}

/// Full-width card with a centred message.
fn message(text: &str) -> Div {
    card_box().items_center().py(px(24.)).text_size(px(14.)).text_color(theme::text_muted()).child(text.to_string())
}
