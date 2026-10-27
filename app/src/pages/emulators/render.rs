//! Rendering of the Emulators page, laid out like the store pages
//! (`pages::side_nav`): the emulators listed on the left ("All emulators",
//! then each one, then "Add emulator"), and on the right the overview, the
//! picked emulator with its ROM folders, or the ways to add a first one.

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_emulation::manager::EmulatorSummary;
use heronite_emulation::RomScanner;
use heronite_i18n::{tr, trn};
use heronite_launch::emulators::UserEmulatorProfile;

use super::{EmulatorsPage, Menu};
use crate::dialogs::emulator_detect::DetectEmulatorsDialog;
use crate::dialogs::scan_roms::{profile_label, ScanInitial};
use crate::icons::Icon;
use crate::pages::side_nav::{self, Dot};
use crate::pages::{caption_with, columns, icon_action, status_pill, tag_pill, PageEvent};
use crate::theme;
use crate::ui::controls::{action_button, button, button_sm, caption, card, card_box, icon_button_sm, ButtonKind};
use crate::ui::dropdown::{menu_item, menu_panel, menu_separator};
use crate::ui::fields::{meta_text, paragraph, search_box};
use crate::ui::icon;

/// From this many emulators on, the list gets a search field.
const SEARCH_FROM: usize = 8;

pub(super) fn launch_label(kind: &str) -> &'static str {
    match kind {
        "flatpak" => "Flatpak",
        "wine" => "Wine",
        _ => tr!("Native"),
    }
}

pub(super) fn folder_flags(s: &RomScanner) -> String {
    let mut flags = vec![];
    flags.push(if s.scan_subfolders { tr!("Subfolders") } else { tr!("Top folder only") });
    if s.merge_related_files {
        flags.push(tr!("Discs merged"));
    }
    flags.push(if s.auto_scan_on_refresh { tr!("Scanned on refresh") } else { tr!("Manual scan") });
    flags.join(" \u{b7} ")
}

/// What the list says of an emulator: whether it starts, then whether it
/// has ROM folders and finds them, else its games and folders.
pub(super) fn glance(s: &EmulatorSummary) -> (Dot, String) {
    if !s.health.is_ok() {
        (Dot::Broken, s.health.label().to_string())
    } else if s.scanners.is_empty() {
        (Dot::Attention, tr!("No ROM folders").to_string())
    } else if !s.missing_folders.is_empty() {
        (Dot::Attention, tr!("Folder not found").to_string())
    } else {
        (Dot::On, format!("{} \u{b7} {}", trn!(s.games, "{n} game", "{n} games"), trn!(s.scanners.len(), "{n} folder", "{n} folders")))
    }
}

/// An action in a caption's line (`caption_with`): accent text with an
/// icon, as tall as the caption, so it does not reach into the card under
/// it as a button would.
fn caption_action(id: impl Into<ElementId>, ic: Icon, label: impl Into<SharedString>) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .items_center()
        .gap(px(4.))
        .mx(px(6.))
        .text_size(px(12.5))
        .text_color(theme::accent())
        .cursor_pointer()
        .hover(|s| s.text_color(theme::text_bright()))
        .child(icon(ic, px(14.), theme::accent()))
        .child(label.into())
}

/// Where it starts from: the executable, or the Flatpak's id.
fn location(s: &EmulatorSummary) -> String {
    let e = &s.emulator;
    if e.launch_kind == "flatpak" {
        format!("Flatpak \u{b7} {}", e.flatpak_id.clone().unwrap_or_default())
    } else if e.executable.is_empty() {
        tr!("No executable set").to_string()
    } else {
        e.executable.clone()
    }
}

/// The platforms of its profiles, once each.
fn platforms(s: &EmulatorSummary) -> Vec<String> {
    let mut platforms: Vec<String> = s
        .emulator
        .profiles
        .iter()
        .map(|p| p.platform_name.clone().filter(|n| !n.is_empty() && !n.eq_ignore_ascii_case("other")).unwrap_or_else(|| p.name.clone()))
        .collect();
    platforms.dedup();
    platforms
}

/// A profile in a line: its platform, the files it opens, and its core or
/// its arguments.
fn profile_line(p: &UserEmulatorProfile) -> String {
    let mut parts = vec![];
    if let Some(platform) =
        p.platform_name.as_deref().filter(|n| !n.is_empty() && !n.eq_ignore_ascii_case("other") && !n.eq_ignore_ascii_case(&p.name))
    {
        parts.push(platform.to_string());
    }
    if !p.extensions.is_empty() {
        parts.push(p.extensions.join(" "));
    }
    match p.core_path.as_deref().filter(|c| !c.is_empty()) {
        Some(core) => parts.push(std::path::Path::new(core).file_stem().map_or(core.to_string(), |s| s.to_string_lossy().into_owned())),
        None if !p.arguments.is_empty() => parts.push(p.arguments.clone()),
        None => {}
    }
    parts.join(" \u{b7} ")
}

impl EmulatorsPage {
    /// The desktop page.
    pub(super) fn render_desktop(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let nav = self.render_nav(cx);
        let wide = side_nav::content_width(window) >= 1000.;
        let selected = self.selected_summary();
        let (header, body) = match selected {
            Some(s) => (self.render_emulator_header(&s, cx), self.render_emulator(&s, wide, cx)),
            None if self.loaded && self.summaries.is_empty() => (side_nav::header(tr!("Emulators"), [], "", None), self.render_empty(cx)),
            None => (self.render_overview_header(cx), self.render_overview(wide, cx)),
        };
        side_nav::sheet("page-emulators", nav, header, cx, body)
    }

    /// The list: "All emulators", each emulator, "Add emulator", and the
    /// link to the stores.
    fn render_nav(&mut self, cx: &mut Context<Self>) -> Div {
        let folders = self.total_folders();
        let busy = self.busy.is_some();
        let scan = (folders > 0).then(|| {
            icon_action("emus-nav-scan", Icon::Refresh, !busy)
                .when(!busy, |b| b.on_click(cx.listener(|this, _, _, cx| this.scan_all(cx))))
                .into_any_element()
        });
        let heading = side_nav::heading(Icon::SportsEsports, tr!("Emulators"), scan);
        let mut items: Vec<AnyElement> = vec![];
        let n = self.summaries.len();
        items.push(side_nav::group(tr!("Emulators"), (n > 0).then_some(n)).into_any_element());
        if n >= SEARCH_FROM {
            items.push(div().px(px(2.)).pb(px(6.)).child(search_box(&self.search)).into_any_element());
        }
        if n > 0 {
            let attention = self.summaries.iter().any(|s| glance(s).0 != Dot::On);
            let line =
                format!("{} \u{b7} {}", trn!(n, "{n} emulator", "{n} emulators"), trn!(folders, "{n} ROM folder", "{n} ROM folders"));
            let dot = if attention { Dot::Attention } else { Dot::On };
            items.push(
                side_nav::item("emus-nav-all", Icon::GridView, tr!("All emulators"), line, dot, self.selected.is_none())
                    .on_click(cx.listener(|this, _, _, cx| this.select(None, cx)))
                    .into_any_element(),
            );
        }
        for s in self.summaries.iter().filter(|s| self.query.is_empty() || s.emulator.name.to_lowercase().contains(&self.query)) {
            let (dot, line) = glance(s);
            let id = s.emulator.id.clone();
            let active = self.selected.as_deref() == Some(id.as_str());
            items.push(
                side_nav::item(
                    SharedString::from(format!("emus-nav-{id}")),
                    super::icon::of(&s.emulator),
                    s.emulator.name.clone(),
                    line,
                    dot,
                    active,
                )
                .on_click(cx.listener(move |this, _, _, cx| this.select(Some(id.clone()), cx)))
                .into_any_element(),
            );
        }
        let add = side_nav::add_item("emus-nav-add", tr!("Add emulator"), tr!("From the catalog or this PC"))
            .on_click(cx.listener(|this, _, _, cx| this.toggle_menu(Menu::Add, cx)));
        items.push(div().relative().child(add).children(self.add_menu(cx)).into_any_element());
        // Names: not translated.
        let stores = side_nav::link("emus-nav-stores", Icon::Storefront, tr!("Stores"), "Epic Games, GOG, Steam, Battle.net\u{2026}")
            .on_click(cx.listener(|_, _, _, cx| cx.emit(PageEvent::OpenStores)));
        side_nav::nav("emus-nav", heading, items, Some(stores.into_any_element()), cx)
    }

    /// The ways to add an emulator, under "Add emulator".
    fn add_menu(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if self.menu != Some(Menu::Add) {
            return None;
        }
        let way = |id: &'static str, ic: Icon, title: &'static str, line: &'static str| {
            menu_item(id, false)
                .items_start()
                .child(div().pt(px(2.)).child(icon(ic, px(16.), theme::accent())))
                .child(div().flex().flex_col().child(title).child(div().text_size(px(12.)).text_color(theme::text_muted()).child(line)))
        };
        let menu = menu_panel(300.)
            .child(way("emus-add-new", Icon::Add, tr!("Add emulator\u{2026}"), tr!("From the catalog, or any program")).on_click(
                cx.listener(|this, _, window, cx| {
                    this.menu = None;
                    this.open_add_emulator(window, cx);
                }),
            ))
            .child(
                way(
                    "emus-add-detect",
                    Icon::TravelExplore,
                    tr!("Detect emulators\u{2026}"),
                    tr!("Find the emulators installed on this computer"),
                )
                .on_click(cx.listener(|this, _, window, cx| {
                    this.menu = None;
                    this.open_detect(window, cx);
                })),
            )
            .child(
                way(
                    "emus-add-folder",
                    Icon::FolderOpen,
                    tr!("Scan a folder for emulators\u{2026}"),
                    tr!("AppImages and programs kept in one folder"),
                )
                .on_click(cx.listener(|this, _, window, cx| {
                    this.menu = None;
                    this.open_scan_folder_for_emulators(window, cx);
                })),
            );
        Some(side_nav::popover("emus-add-menu", menu, 48., false, self.close_menu(cx)))
    }

    /// Closes the open menu (a click outside it).
    fn close_menu(&self, cx: &mut Context<Self>) -> impl Fn(&MouseDownEvent, &mut Window, &mut App) + 'static + use<> {
        let page = cx.entity().downgrade();
        move |_, _, cx| {
            page.update(cx, |this, cx| {
                this.menu = None;
                cx.notify();
            })
            .ok();
        }
    }

    /// The line shown while a background scan runs.
    fn render_busy(&self) -> Option<Div> {
        let label = self.busy.clone()?;
        Some(
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .mx(px(6.))
                .mb(px(14.))
                .text_size(px(13.))
                .text_color(theme::accent())
                .child(icon(Icon::Refresh, px(16.), theme::accent()))
                .child(label),
        )
    }

    /// The result of the last action with a "Dismiss" button.
    fn render_notice(&self, cx: &mut Context<Self>) -> Option<Div> {
        let n = self.notice.clone()?;
        let color = if n.error { theme::danger_text() } else { theme::success() };
        let mut lines = div().flex().flex_col().gap(px(2.)).flex_1().min_w_0().py(px(12.));
        for (i, l) in n.lines.iter().enumerate() {
            lines = lines.child(div().text_size(px(13.)).text_color(if i == 0 { color } else { theme::text_muted() }).child(l.clone()));
        }
        Some(card_box().mb(px(14.)).flex_row().items_center().gap(px(12.)).child(lines).child(
            button_sm("emus-notice-x", ButtonKind::Ghost, tr!("Dismiss")).on_click(cx.listener(|this, _, _, cx| {
                this.notice = None;
                cx.notify();
            })),
        ))
    }

    /// No emulator yet: the ways to add the first one.
    fn render_empty(&self, cx: &mut Context<Self>) -> Div {
        let way = |id: &'static str, ic: Icon, title: &'static str, line: &'static str, primary: bool| {
            div()
                .id(id)
                .flex()
                .flex_col()
                .gap(px(6.))
                .flex_1()
                .min_w_0()
                .p(px(16.))
                .rounded(px(theme::R_CARD))
                .bg(if primary { theme::accent_tint() } else { theme::card_fill() })
                .border_1()
                .border_color(if primary { theme::accent_tint_border() } else { theme::ink_a(0.08) })
                .cursor_pointer()
                .hover(|s| s.bg(theme::ink_a(0.08)))
                .child(icon(ic, px(20.), theme::accent()))
                .child(div().mt(px(4.)).text_size(px(14.)).font_weight(theme::W_BOLD).child(title))
                .child(div().text_size(px(12.5)).line_height(px(18.)).text_color(theme::text_muted()).child(line))
        };
        div()
            .flex()
            .flex_col()
            .max_w(px(720.))
            .gap(px(18.))
            .children(self.render_notice(cx))
            .child(paragraph(tr!(
                "No emulators configured yet. Detect the ones installed on this PC, or add one from the catalog or as a custom program."
            )))
            .child(
                div()
                    .flex()
                    .gap(px(12.))
                    .child(
                        way(
                            "emus-empty-detect",
                            Icon::TravelExplore,
                            tr!("Detect emulators\u{2026}"),
                            tr!("Find the emulators installed on this computer"),
                            true,
                        )
                        .on_click(cx.listener(|this, _, window, cx| this.open_detect(window, cx))),
                    )
                    .child(
                        way("emus-empty-add", Icon::Add, tr!("Add emulator\u{2026}"), tr!("From the catalog, or any program"), false)
                            .on_click(cx.listener(|this, _, window, cx| this.open_add_emulator(window, cx))),
                    )
                    .child(
                        way(
                            "emus-empty-folder",
                            Icon::FolderOpen,
                            tr!("Scan a folder for emulators\u{2026}"),
                            tr!("AppImages and programs kept in one folder"),
                            false,
                        )
                        .on_click(cx.listener(|this, _, window, cx| this.open_scan_folder_for_emulators(window, cx))),
                    ),
            )
    }

    fn render_overview_header(&self, cx: &mut Context<Self>) -> Div {
        let folders = self.total_folders();
        let games: usize = self.summaries.iter().map(|s| s.games).sum();
        let lead = if self.loaded {
            format!(
                "{} \u{b7} {} \u{b7} {}. {}",
                trn!(self.summaries.len(), "{n} emulator", "{n} emulators"),
                trn!(folders, "{n} ROM folder", "{n} ROM folders"),
                trn!(games, "{n} game", "{n} games"),
                tr!("Folders set to scan on refresh add new games whenever the Collection refreshes.")
            )
        } else {
            tr!("Loading\u{2026}").to_string()
        };
        let scan = action_button(
            "emus-scan-all",
            ButtonKind::Primary,
            tr!("Scan all folders now"),
            self.busy.is_none() && folders > 0,
            cx.listener(|this, _, _, cx| this.scan_all(cx)),
        );
        side_nav::header(tr!("All emulators"), [], lead, Some(scan.into_any_element()))
    }

    /// "All emulators": what needs attention, and every ROM folder.
    fn render_overview(&mut self, wide: bool, cx: &mut Context<Self>) -> Div {
        let summaries = self.summaries.clone();
        let mut attention: Vec<AnyElement> = vec![];
        for s in summaries.iter().filter(|s| glance(s).0 != Dot::On) {
            let (dot, line) = glance(s);
            let id = s.emulator.id.clone();
            let fix = match dot {
                Dot::Broken => Some(button_sm(SharedString::from(format!("emus-fix-{id}")), ButtonKind::Glass, tr!("Edit")).on_click({
                    let id = id.clone();
                    cx.listener(move |this, _, window, cx| this.open_edit_emulator(&id, window, cx))
                })),
                _ if s.scanners.is_empty() => {
                    Some(button_sm(SharedString::from(format!("emus-fix-{id}")), ButtonKind::Glass, tr!("Add ROM folder")).on_click({
                        let id = id.clone();
                        cx.listener(move |this, _, window, cx| this.open_folder_dialog(Some(id.clone()), None, window, cx))
                    }))
                }
                _ => None,
            };
            let color = if dot == Dot::Broken { theme::danger_text() } else { theme::warning() };
            attention.push(
                div()
                    .id(SharedString::from(format!("emus-attention-{id}")))
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .py(px(11.))
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _, _, cx| this.select(Some(id.clone()), cx)))
                    .child(icon(super::icon::of(&s.emulator), px(18.), theme::text_muted()))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .flex_1()
                            .min_w_0()
                            .gap(px(2.))
                            .child(div().text_size(px(14.)).child(s.emulator.name.clone()))
                            .child(div().text_size(px(12.5)).text_color(color).child(line)),
                    )
                    .children(fix)
                    .into_any_element(),
            );
        }
        if attention.is_empty() {
            attention.push(div().py(px(12.)).child(meta_text(tr!("Everything is ready."))).into_any_element());
        }
        let mut folders: Vec<AnyElement> = vec![];
        for s in &summaries {
            for sc in &s.scanners {
                folders.push(self.render_folder_row(s, sc, true, cx).into_any_element());
            }
        }
        if folders.is_empty() {
            folders.push(
                div()
                    .py(px(12.))
                    .child(meta_text(tr!("No ROM folders yet. Add one and its games are added to the Collection.")))
                    .into_any_element(),
            );
        }
        let add_folder = caption_action("emus-add-folder-all", Icon::Add, tr!("Add ROM folder"))
            .on_click(cx.listener(|this, _, window, cx| this.open_folder_dialog(None, None, window, cx)));
        let left = div().child(caption(tr!("Needs attention"))).child(card(attention));
        let right = div().child(caption_with(tr!("ROM folders"), add_folder)).child(card(folders));
        div().flex().flex_col().children(self.render_busy()).children(self.render_notice(cx)).child(columns(wide, [left, right]))
    }

    /// An emulator's name, kind and health, where it starts from, and Open,
    /// Edit and the menu with Duplicate and Remove.
    fn render_emulator_header(&self, s: &EmulatorSummary, cx: &mut Context<Self>) -> Div {
        let id = s.emulator.id.clone();
        let health = status_pill(s.health.label(), if s.health.is_ok() { theme::success() } else { theme::danger_text() });
        let tags = [tag_pill(launch_label(&s.emulator.launch_kind)).into_any_element(), health.into_any_element()];
        let open = {
            let id = id.clone();
            icon_button_sm(
                SharedString::from(format!("emu-open-{id}")),
                ButtonKind::Primary,
                Icon::PlayArrow,
                tr!("Open"),
                s.health.is_ok(),
                cx.listener(move |this, _, _, cx| this.open_emulator(&id, cx)),
            )
        };
        let edit = {
            let id = id.clone();
            button_sm(SharedString::from(format!("emu-edit-{id}")), ButtonKind::Glass, tr!("Edit"))
                .on_click(cx.listener(move |this, _, window, cx| this.open_edit_emulator(&id, window, cx)))
        };
        let more = icon_action(SharedString::from(format!("emu-more-{id}")), Icon::MoreHoriz, true)
            .on_click(cx.listener(|this, _, _, cx| this.toggle_menu(Menu::Emulator, cx)));
        let menu = (self.menu == Some(Menu::Emulator)).then(|| {
            let (dup, rm) = (id.clone(), id.clone());
            let panel = menu_panel(220.)
                .child(menu_item("emu-menu-dup", false).child(tr!("Duplicate")).on_click(cx.listener(move |this, _, _, cx| {
                    this.menu = None;
                    this.duplicate(&dup, cx);
                })))
                .child(menu_separator())
                .child(menu_item("emu-menu-rm", false).text_color(theme::danger_text()).child(tr!("Remove\u{2026}")).on_click(
                    cx.listener(move |this, _, window, cx| {
                        this.menu = None;
                        this.confirm_remove_emulator(&rm, window, cx);
                    }),
                ));
            side_nav::popover("emu-menu", panel, 38., true, self.close_menu(cx))
        });
        let actions = div().flex().items_center().gap(px(8.)).child(open).child(edit).child(div().relative().child(more).children(menu));
        side_nav::header(s.emulator.name.clone(), tags, location(s), Some(actions.into_any_element()))
    }

    /// An emulator: why it does not start (when it does not), its ROM
    /// folders on the left; its games, platforms and profiles on the right.
    fn render_emulator(&mut self, s: &EmulatorSummary, wide: bool, cx: &mut Context<Self>) -> Div {
        let id = s.emulator.id.clone();
        let broken = (!s.health.is_ok()).then(|| {
            let id = id.clone();
            card_box()
                .mb(px(14.))
                .flex_row()
                .items_center()
                .gap(px(12.))
                .bg(theme::danger_fill())
                .child(icon(Icon::Warning, px(18.), theme::danger_text()))
                .child(
                    div()
                        .flex_1()
                        .py(px(12.))
                        .text_size(px(13.))
                        .child(tr!("{problem}: its games cannot start until it is fixed.", problem = s.health.label())),
                )
                .child(
                    button_sm(SharedString::from(format!("emu-fix-{id}")), ButtonKind::Glass, tr!("Edit"))
                        .on_click(cx.listener(move |this, _, window, cx| this.open_edit_emulator(&id, window, cx))),
                )
        });
        let mut folders: Vec<AnyElement> =
            s.scanners.iter().map(|sc| self.render_folder_row(s, sc, false, cx).into_any_element()).collect();
        let add_folder = {
            let id = id.clone();
            caption_action(SharedString::from(format!("emu-addfolder-{id}")), Icon::Add, tr!("Add ROM folder"))
                .on_click(cx.listener(move |this, _, window, cx| this.open_folder_dialog(Some(id.clone()), None, window, cx)))
        };
        if folders.is_empty() {
            let id = id.clone();
            folders.push(
                div()
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .py(px(12.))
                    .child(div().flex_1().child(meta_text(tr!("No ROM folders yet. Add one and its games are added to the Collection."))))
                    .child(
                        button(SharedString::from(format!("emu-addfolder-first-{id}")), ButtonKind::Primary, tr!("Add ROM folder"))
                            .on_click(cx.listener(move |this, _, window, cx| this.open_folder_dialog(Some(id.clone()), None, window, cx))),
                    )
                    .into_any_element(),
            );
        }
        let left = div().child(caption_with(tr!("ROM folders"), add_folder)).child(card(folders));

        let platforms = div().flex().flex_wrap().gap(px(6.)).children(platforms(s).into_iter().map(tag_pill));
        let about = card([
            div().py(px(12.)).text_size(px(14.)).child(trn!(s.games, "{n} game using it", "{n} games using it")).into_any_element(),
            div()
                .flex()
                .flex_col()
                .gap(px(8.))
                .py(px(12.))
                .child(div().text_size(px(14.)).child(tr!("Platforms")))
                .child(platforms)
                .into_any_element(),
        ]);
        let profiles = card(s.emulator.profiles.iter().map(|p| {
            div()
                .flex()
                .flex_col()
                .gap(px(3.))
                .py(px(11.))
                .child(div().text_size(px(14.)).child(profile_label(p)))
                .child(meta_text(profile_line(p)))
                .into_any_element()
        }));
        let right = div()
            .child(caption(tr!("Emulator")))
            .child(about)
            .when(!s.emulator.profiles.is_empty(), |d| d.child(caption(tr!("Profiles"))).child(profiles));
        div()
            .flex()
            .flex_col()
            .children(broken)
            .children(self.render_busy())
            .children(self.render_notice(cx))
            .child(columns(wide, [left, right]))
    }

    /// One ROM folder: path, profile (and emulator, in the overview) and
    /// flags; Scan now and Review, and the menu with Edit and Remove.
    fn render_folder_row(&self, s: &EmulatorSummary, sc: &RomScanner, with_emulator: bool, cx: &mut Context<Self>) -> Div {
        let busy = self.busy.is_some();
        let sid = sc.id.clone();
        let missing = s.missing_folders.contains(&sc.directory);
        let profile = s.emulator.profile(&sc.profile_id).map(profile_label).unwrap_or_else(|| tr!("Unknown profile").into());
        let what = if with_emulator && !profile.eq_ignore_ascii_case(&s.emulator.name) {
            format!("{} \u{b7} {profile}", s.emulator.name)
        } else {
            profile
        };
        let init = ScanInitial {
            emulator_id: Some(sc.emulator_id.clone()),
            profile_id: Some(sc.profile_id.clone()),
            directory: Some(sc.directory.clone()),
        };
        let scan = {
            let sid = sid.clone();
            let b = button_sm(SharedString::from(format!("scan-now-{sid}")), ButtonKind::Glass, tr!("Scan now"));
            if !busy && !missing {
                b.on_click(cx.listener(move |this, _, _, cx| this.scan(vec![sid.clone()], tr!("Scanning\u{2026}"), cx)))
            } else {
                b.opacity(0.45).cursor_not_allowed()
            }
        };
        let review = button_sm(SharedString::from(format!("scan-review-{sid}")), ButtonKind::Glass, tr!("Review\u{2026}"))
            .on_click(cx.listener(move |_, _, _, cx| cx.emit(PageEvent::ScanRoms(init.clone()))));
        let more = {
            let sid = sid.clone();
            icon_action(SharedString::from(format!("scan-more-{sid}")), Icon::MoreHoriz, true)
                .on_click(cx.listener(move |this, _, _, cx| this.toggle_menu(Menu::Folder(sid.clone()), cx)))
        };
        let menu = (self.menu == Some(Menu::Folder(sid.clone()))).then(|| {
            let (edit, rm) = (sc.clone(), sc.clone());
            let panel = menu_panel(200.)
                .child(menu_item("scan-menu-edit", false).child(tr!("Edit")).on_click(cx.listener(move |this, _, window, cx| {
                    this.menu = None;
                    this.open_folder_dialog(None, Some(edit.clone()), window, cx);
                })))
                .child(menu_separator())
                .child(menu_item("scan-menu-rm", false).text_color(theme::danger_text()).child(tr!("Remove\u{2026}")).on_click(
                    cx.listener(move |this, _, window, cx| {
                        this.menu = None;
                        this.confirm_remove_folder(&rm, window, cx);
                    }),
                ));
            side_nav::popover("scan-menu", panel, 38., true, self.close_menu(cx))
        });
        div()
            .flex()
            .items_center()
            .gap(px(12.))
            .py(px(11.))
            .child(icon(Icon::Folder, px(18.), theme::text_muted()))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .gap(px(2.))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .child(
                                div().min_w_0().text_size(px(14.)).text_color(theme::text_primary()).truncate().child(sc.directory.clone()),
                            )
                            .children(missing.then(|| status_pill(tr!("Folder not found"), theme::danger_text()))),
                    )
                    .child(meta_text(format!("{what}  \u{b7}  {}", folder_flags(sc)))),
            )
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(px(6.))
                    .child(scan)
                    .child(review)
                    .child(div().relative().child(more).children(menu)),
            )
    }

    /// Detect emulators, starting with the folder picker.
    pub(super) fn open_scan_folder_for_emulators(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open_detect(window, cx);
        if let Some(d) = self.open_dialog_as::<DetectEmulatorsDialog>() {
            d.update(cx, |d, cx| d.pick_folder(window, cx));
        }
    }
}

#[cfg(test)]
mod tests {
    use heronite_emulation::manager::{EmulatorSummary, Health};
    use heronite_emulation::RomScanner;
    use heronite_launch::emulators::UserEmulator;

    use super::glance;
    use crate::pages::side_nav::Dot;

    fn summary(health: Health, folders: usize, missing: usize) -> EmulatorSummary {
        let scanners: Vec<RomScanner> =
            (0..folders).map(|i| RomScanner { id: i.to_string(), directory: format!("/roms/{i}"), ..Default::default() }).collect();
        EmulatorSummary {
            emulator: UserEmulator::default(),
            health,
            games: 10,
            missing_folders: scanners.iter().take(missing).map(|s| s.directory.clone()).collect(),
            scanners,
        }
    }

    #[test]
    fn an_emulator_says_first_whether_it_starts_then_what_it_lacks() {
        assert_eq!(glance(&summary(Health::MissingExecutable, 1, 0)).0, Dot::Broken);
        assert_eq!(glance(&summary(Health::Ok, 0, 0)).0, Dot::Attention);
        assert_eq!(glance(&summary(Health::Ok, 2, 1)).0, Dot::Attention);
        let (dot, line) = glance(&summary(Health::Ok, 2, 0));
        assert_eq!(dot, Dot::On);
        assert!(line.contains("10") && line.contains('2'));
    }
}
