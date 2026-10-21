//! Rendering of the form, the Wine section and the remove confirmation.

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::sideload::SideloadPlatform;
use heronite_i18n::tr;

use super::logic::*;
use super::{text_of, AddGameDialog, Mode, Select, STORE_NOTE};
use crate::dialogs::settings_panel::logic::{default_wine, display_wine_name, wine_of, wine_options};
use crate::icons::Icon;
use crate::pages::{status_pill, tag_pill};
use crate::theme;
use crate::ui::controls::{button_sm, card, card_box, checkbox, icon_button_sm, mini_checkbox, tabs, toggle_row, ButtonKind};
use crate::ui::fields::{choose_path, field_label, input_box, input_box_with, labeled, meta_text, panel, path_field, select_field};
use crate::ui::{icon, tooltip};

impl AddGameDialog {
    fn platform_tabs(&self, cx: &mut Context<Self>) -> Div {
        let items: Vec<(SideloadPlatform, &'static str)> = PLATFORMS.iter().map(|p| (*p, platform_label(*p))).collect();
        let weak = cx.entity().downgrade();
        labeled(
            tr!("Platform"),
            tabs("ag-platform", &items, self.platform, move |platform, _, cx| {
                weak.update(cx, |this, cx| this.set_platform(platform, cx)).ok();
            }),
        )
    }

    /// The note above the form: what to do instead for store games (until
    /// "Got it"), or how installing works.
    fn note(&self, cx: &mut Context<Self>) -> Option<Div> {
        let (text, dismiss) = match self.mode {
            Mode::Add if self.store_note => {
                (tr!("A game from Epic, GOG or Amazon? Do not add it here: install or import it from its store, in the menu."), true)
            }
            Mode::Add => return None,
            _ => (self.top_note()?, false),
        };
        let ok = dismiss.then(|| {
            button_sm("ag-note-ok", ButtonKind::Ghost, tr!("Got it")).on_click(cx.listener(|this, _, _, cx| {
                this.store_note = false;
                let app = this.ctx.app.clone();
                cx.background_spawn(async move {
                    if let Err(err) = heronite_data::ui_state::dismiss_note(&app, STORE_NOTE) {
                        log::warn!("saving the dismissed note: {err:#}");
                    }
                })
                .detach();
                cx.notify();
            }))
        });
        Some(
            panel()
                .flex_row()
                .items_center()
                .gap(px(12.))
                .py(px(10.))
                .pr(px(if dismiss { 8. } else { 14. }))
                .child(icon(Icon::InfoOutlined, px(18.), theme::accent()))
                .child(div().flex_1().min_w_0().text_size(px(13.)).line_height(px(18.)).text_color(theme::text_primary()).child(text))
                .children(ok),
        )
    }

    /// Wine / Proton: the version, the prefix and the shared prefix switch;
    /// closed it sums up the version and the prefix.
    fn wine_section(&mut self, cx: &mut Context<Self>) -> Div {
        let open = self.show_wine;
        let shared = self.shared_prefix;
        let list = self.wine_list.clone();
        let loading = list.is_none();
        let current = self.wine.clone().unwrap_or_else(default_wine);
        let none_found = list.as_ref().is_some_and(|l| l.is_empty()) && self.wine.is_none();
        let crossover = self.wine.as_ref().is_some_and(|w| w.kind == "crossover");
        let shown = if loading {
            tr!("Loading\u{2026}").to_string()
        } else if self.wine.is_none() {
            tr!("No Wine version found").to_string()
        } else {
            display_wine_name(&current.name)
        };
        let mut header = div()
            .id("ag-wine-toggle")
            .flex()
            .items_center()
            .gap(px(10.))
            .py(px(4.))
            .cursor_pointer()
            .on_click(cx.listener(|this, _, _, cx| {
                this.show_wine = !this.show_wine;
                this.open_select = None;
                cx.notify();
            }))
            .child(icon(Icon::WineBar, px(16.), theme::text_muted()))
            .child(div().flex_1().min_w_0().text_size(px(14.)).text_color(theme::text_primary()).child(tr!("Wine / Proton")));
        if !open {
            header = header.child(if none_found { status_pill(shown.clone(), theme::danger_text()) } else { tag_pill(shown.clone()) });
            if !crossover {
                header = header.child(tag_pill(if shared { tr!("Shared prefix") } else { tr!("Own prefix") }));
            }
        }
        header = header.child(icon(if open { Icon::ExpandMore } else { Icon::ChevronRight }, px(18.), theme::text_muted()));
        let mut col = card_box().gap(px(12.)).py(px(12.)).child(header);
        if !open {
            return col;
        }

        let options = wine_options(list.as_deref().unwrap_or(&[]), &current);
        let items: Vec<(SharedString, bool)> = options
            .iter()
            .map(|w| (SharedString::from(display_wine_name(&w.name)), self.wine.as_ref().is_some_and(|c| c.name == w.name)))
            .collect();
        col = col.child(select_field(
            "ag-wine",
            Some(tr!("Wine version")),
            shown,
            shared || loading || options.is_empty() || none_found,
            self.open_select == Some(Select::Wine),
            items,
            cx,
            |this, _, cx| {
                this.open_select = if this.open_select == Some(Select::Wine) { None } else { Some(Select::Wine) };
                cx.notify();
            },
            move |this, i, _, cx| {
                let list = this.wine_list.clone().unwrap_or_default();
                let cur = this.wine.clone().unwrap_or_else(default_wine);
                if let Some(w) = wine_options(&list, &cur).get(i) {
                    this.wine = Some(w.clone());
                }
                this.open_select = None;
                cx.notify();
            },
        ));
        if none_found {
            col = col.child(
                div()
                    .text_size(px(13.33))
                    .text_color(theme::danger_text())
                    .child(tr!("No Wine version was found, download one from the Wine Manager")),
            );
        }
        if crossover {
            col = col.child(meta_text(tr!("CrossOver bottles are chosen in Game settings after the game is added.")));
        } else {
            let has = !text_of(&self.prefix, cx).is_empty();
            col = col.child(path_field(
                "ag-prefix-pick",
                Some(tr!("Wine prefix")),
                &self.prefix,
                has,
                shared,
                cx.listener(move |this, _, window, cx| {
                    if has {
                        this.prefix.update(cx, |s, cx| s.set_value("", window, cx));
                    } else {
                        choose_path(window, cx, true, tr!("Select WinePrefix Folder"), |this: &mut Self, path, window, cx| {
                            this.prefix.update(cx, |s, cx| s.set_value(path, window, cx));
                        });
                    }
                }),
            ));
        }
        let share_label = match self.global.get("wineVersion").map(|_| wine_of(&self.global)) {
            Some(w) => tr!("Use shared Wine prefix ({name})", name = display_wine_name(&w.name)),
            None => tr!("Use shared Wine prefix").to_string(),
        };
        col.child(
            toggle_row(
                "ag-shared",
                share_label,
                Some(tr!("Only if you know what you are doing: games sharing a prefix can break each other's dependencies.")),
                shared,
            )
            .py(px(4.))
            .on_click(cx.listener(|this, _, window, cx| {
                let on = !this.shared_prefix;
                this.set_shared_prefix(on, window, cx);
            })),
        )
    }

    /// Under a Windows game's executable: running its setup program first,
    /// and the folder the executable is in.
    fn executable_notes(&self, exe: &str, title: &str, cx: &mut Context<Self>) -> Div {
        let mut col = div().flex().flex_col().gap(px(6.)).mt(px(-6.));
        if !self.is_edit() {
            let has_title = !title.trim().is_empty();
            let can_run = !self.saving && !self.installer_running && has_title;
            let label = if self.installer_running { tr!("Running setup\u{2026}") } else { tr!("Run installer first") };
            let tip = if has_title {
                tr!("Runs the game's setup program in its Wine prefix. When it is done, select the installed game's executable.")
            } else {
                tr!("Add a title first: the game's Wine prefix is named after it.")
            };
            col = col.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .child(div().text_size(px(13.)).text_color(theme::text_muted()).child(tr!("Not installed yet?")))
                    .child(
                        icon_button_sm(
                            "ag-run-installer",
                            ButtonKind::Glass,
                            Icon::PlayArrow,
                            label,
                            can_run,
                            cx.listener(|_, _, window, cx| {
                                choose_path(window, cx, false, tr!("Select EXE to Run"), |this: &mut Self, path, _, cx| {
                                    this.run_installer(path, cx)
                                });
                            }),
                        )
                        .tooltip(tooltip(tip)),
                    ),
            );
        }
        if let Some(folder) = install_folder(exe) {
            col = col.child(meta_text(tr!("Install folder: {folder}", folder)));
        }
        col
    }

    /// A small switch: the sandbox is for apps from sources one does not
    /// trust, its tooltip says what it does.
    fn sandbox_switch(&self, cx: &mut Context<Self>) -> Stateful<Div> {
        div()
            .id("ag-sandbox")
            .flex()
            .items_center()
            .gap(px(8.))
            .cursor_pointer()
            .text_size(px(13.))
            .text_color(theme::text_muted())
            .hover(|s| s.text_color(theme::text_primary()))
            .tooltip(tooltip(tr!(
                "An isolated environment: the game and its installer start without network and without your home folder. For apps from sources you do not trust. Allow what it needs later in the game's settings, Sandbox tab."
            )))
            .on_click(cx.listener(|this, _, _, cx| {
                this.sandbox = !this.sandbox;
                cx.notify();
            }))
            .child(mini_checkbox("ag-sandbox-sw", self.sandbox, false).mt(px(0.)))
            .child(tr!("Run in a sandbox"))
            .child(icon(Icon::InfoOutlined, px(14.), theme::text_dim()))
    }

    pub(super) fn form(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let _ = window;
        let (square_url, hero_url) = (text_of(&self.square, cx), text_of(&self.hero, cx));
        let title = text_of(&self.title, cx);
        let preview_hero = if hero_url.trim().is_empty() { square_url.clone() } else { hero_url.clone() };
        let left = self.art_column(&square_url, &preview_hero, cx);

        let mut right = div().flex().flex_col().flex_1().min_w_0().gap(px(16.));
        if let Some(note) = self.note(cx) {
            right = right.child(note);
        }
        let finding = self.finding;
        let find = icon_button_sm(
            "ag-find-art",
            ButtonKind::Glass,
            Icon::AutoAwesome,
            if finding { tr!("Searching\u{2026}") } else { tr!("Find art") },
            !finding && !title.trim().is_empty(),
            cx.listener(|this, _, window, cx| this.find_art(window, cx)),
        );
        right = right.child(labeled(tr!("Title"), input_box_with(&self.title, find.into_any_element()))).child(self.platform_tabs(cx));

        if self.platform == SideloadPlatform::Browser {
            right = right
                .child(labeled(tr!("BrowserURL"), input_box(&self.url, false)))
                .child(labeled(tr!("Custom User Agent"), input_box(&self.user_agent, false)))
                .child(card([checkbox("ag-fullscreen", self.fullscreen, tr!("Launch Fullscreen (F11 to exit)"))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.fullscreen = !this.fullscreen;
                        cx.notify();
                    }))
                    .into_any_element()]));
        } else if self.platform == SideloadPlatform::Linux {
            right = right.child(self.linux_start(cx));
        } else {
            let exe = text_of(&self.exe, cx);
            let has = !exe.is_empty();
            right = right
                .child(path_field(
                    "ag-exe-pick",
                    Some(tr!("Executable")),
                    &self.exe,
                    has,
                    false,
                    cx.listener(move |this, _, window, cx| {
                        if has {
                            this.exe.update(cx, |s, cx| s.set_value("", window, cx));
                        } else {
                            choose_path(window, cx, false, tr!("Select Executable"), |this: &mut Self, path, window, cx| {
                                this.exe.update(cx, |s, cx| s.set_value(path, window, cx));
                            });
                        }
                    }),
                ))
                .child(self.executable_notes(&exe, &title, cx));
            if !self.is_edit() {
                right = right.child(self.wine_section(cx));
            }
        }

        if self.offers_sandbox() {
            right = right.child(div().flex().child(self.sandbox_switch(cx)));
        }

        if !self.error.is_empty() {
            right = right.child(div().text_size(px(14.)).line_height(px(20.)).text_color(theme::danger_text()).child(self.error.clone()));
        } else if !self.notice.is_empty() {
            right = right.child(div().text_size(px(14.)).line_height(px(20.)).text_color(theme::success()).child(self.notice.clone()));
        }

        div().flex().gap(px(24.)).child(left).child(right)
    }

    /// Linux: the game's folder, then how it starts (an executable, or a
    /// command run inside the folder).
    fn linux_start(&mut self, cx: &mut Context<Self>) -> Div {
        let folder = text_of(&self.folder, cx);
        let has_folder = !folder.trim().is_empty();
        let mut col = div().flex().flex_col().gap(px(16.)).child(path_field(
            "ag-folder-pick",
            Some(tr!("Game folder")),
            &self.folder,
            has_folder,
            false,
            cx.listener(move |this, _, window, cx| {
                if has_folder {
                    this.folder.update(cx, |s, cx| s.set_value("", window, cx));
                } else {
                    choose_path(window, cx, true, tr!("Select the game folder"), |this: &mut Self, path, window, cx| {
                        this.folder.update(cx, |s, cx| s.set_value(path, window, cx));
                    });
                }
            }),
        ));
        let weak = cx.entity().downgrade();
        col = col.child(div().flex().flex_col().child(field_label(tr!("Start with"))).child(crate::ui::controls::tabs(
            "ag-start",
            &[(false, tr!("An executable")), (true, tr!("A command"))],
            self.by_command,
            move |by_command, _, cx| {
                weak.update(cx, |this, cx| {
                    this.by_command = by_command;
                    this.error.clear();
                    cx.notify();
                })
                .ok();
            },
        )));
        if self.by_command {
            col = col
                .child(labeled(tr!("Command"), input_box(&self.command, false)))
                .child(meta_text(tr!("Runs inside the game folder, like a terminal opened there. Example: ./start.sh --fullscreen")));
        } else {
            let exe = text_of(&self.exe, cx);
            let has = !exe.is_empty();
            col = col.child(path_field(
                "ag-exe-pick",
                Some(tr!("Executable")),
                &self.exe,
                has,
                false,
                cx.listener(move |this, _, window, cx| {
                    if has {
                        this.exe.update(cx, |s, cx| s.set_value("", window, cx));
                    } else {
                        choose_path(window, cx, false, tr!("Select Executable"), |this: &mut Self, path, window, cx| {
                            this.exe.update(cx, |s, cx| s.set_value(path.clone(), window, cx));
                            this.fill_folder_from(&path, window, cx);
                        });
                    }
                }),
            ));
        }
        col
    }

    pub(super) fn remove_confirmation(&self, cx: &mut Context<Self>) -> Div {
        let title = text_of(&self.title, cx);
        let mut col = div().flex().flex_col().gap(px(16.)).child(div().text_size(px(14.)).line_height(px(20.)).text_color(crate::theme::text_primary()).child(tr!(
            "Remove {title} from your library? The game's files are not deleted. Its playtime, status, notes, images and Collection data are removed, and so are its shortcuts.",
            title = if title.trim().is_empty() { tr!("this game") } else { title.trim() }
        )));
        if let (Some(prefix), Some(note)) = (&self.removable_prefix, &self.prefix_shared) {
            col = col.child(meta_text(tr!("Wine prefix {prefix}: {note}", prefix, note)));
        } else if let Some(prefix) = &self.removable_prefix {
            col = col
                .child(checkbox("ag-remove-prefix", self.remove_prefix_too, tr!("Also delete the Wine prefix")).on_click(cx.listener(
                    |this, _, _, cx| {
                        this.remove_prefix_too = !this.remove_prefix_too;
                        cx.notify();
                    },
                )))
                .child(meta_text(tr!("{prefix}  (saves and settings inside it will be lost)", prefix)));
        }
        if !self.error.is_empty() {
            col = col.child(div().text_size(px(14.)).text_color(theme::danger_text()).child(self.error.clone()));
        }
        col
    }
}
