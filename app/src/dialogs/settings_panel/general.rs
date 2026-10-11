//! "General" tab of the global settings: the subset of Heroic's general and
//! advanced settings that matter to this app.

use gpui_kit::prelude::*;
use gpui_kit::*;
use serde_json::Value;

use super::parts::{column, heading, help, rows, Rows};
use super::{SettingsPanel, Section, Target};
use heronite_data::DataPaths;
use crate::icons::Icon;
use crate::theme;
use crate::ui::fields::choose_path;
use crate::ui::icon;

/// What the global ("General") panel shows. The Settings page hosts one
/// panel and switches between these; the Game defaults use the other tabs.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum GlobalView {
    #[default]
    General,
    Downloads,
    Advanced,
}

impl SettingsPanel {
    /// A panel bound to the global settings showing `view`.
    pub fn new_global(paths: DataPaths, view: GlobalView, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut this = Self::new(paths, Target::Defaults, Section::General, window, cx);
        this.global_view = view;
        this
    }

    /// Switches between the global views (closing open menus).
    pub fn set_global_view(&mut self, view: GlobalView, cx: &mut Context<Self>) {
        if self.global_view != view {
            self.global_view = view;
            self.open_select = None;
            self.notice = None;
            cx.notify();
        }
    }

    pub(super) fn general_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Div {
        match self.global_view {
            GlobalView::General => self.general_view(window, cx),
            GlobalView::Downloads => self.downloads_view(cx),
            GlobalView::Advanced => self.advanced_view(cx),
        }
    }

    /// Epic downloads and game updates.
    fn downloads_view(&mut self, cx: &mut Context<Self>) -> Div {
        let fields = rows()
            .field_help(
                self.max_workers_field(cx),
                vec![help("More workers download faster but use more CPU, memory and disk. \"Max\" uses every core.")],
            )
            .field_help(
                self.path_row("altLegendaryBin", Some("Choose an Alternative Legendary Binary"), false, "Select Legendary Binary", true, cx),
                vec![help("Leave it empty to use the one in the data folder (tools/bin/legendary).")],
            );
        let toggles = rows()
            .child(self.toggle_row(
                "download-no-https",
                "downloadNoHttps",
                false,
                "Download games without HTTPS (useful for CDNs e.g. LanCache)",
                None,
                false,
                cx,
            ))
            .child(self.toggle_row(
                "auto-update",
                "autoUpdateGames",
                false,
                "Automatically update games",
                Some("Updates are queued in the background when they are found, except offline, on a metered connection or while the game is running."),
                false,
                cx,
            ));
        column().child(heading("Downloads")).child(fields.card()).child(heading("Updates")).child(toggles.card())
    }

    /// Logging and advanced switches (folders and cache live in the page).
    fn advanced_view(&mut self, cx: &mut Context<Self>) -> Div {
        let r = rows()
            .child(self.toggle_row(
                "disable-logs",
                "disableLogs",
                false,
                "Disable Logs",
                Some("Toggle this switch ON to disable most of the writes to log files (critical information is always logged). Make sure to turn OFF this setting before reporting any issue."),
                false,
                cx,
            ));
        column().child(heading("Logging")).child(r.card())
    }

    fn general_view(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> Div {
        let folders = rows()
            .field(self.path_row("defaultInstallPath", Some("Default Installation Path"), true, "Choose the default installation folder", false, cx))
            .field(self.path_row("defaultWinePrefixDir", Some("Set Folder for new Wine Prefixes"), true, "Select a Folder for new Wine Prefixes", false, cx))
            .field_help(
                self.path_row("defaultSteamPath", Some("Default Steam path"), true, "Steam path.", false, cx),
                vec![help("This path lets Heronite determine what version of Proton Steam uses, for adding non-Steam games to Steam.")],
            );
        let launch = rows().child(self.toggle_row("minimize", "minimizeOnLaunch", false, "Minimize after game launch", None, false, cx));
        let wine = rows()
            .child(self.toggle_row(
                "valve-proton",
                "showValveProton",
                false,
                "Allow using Valve Proton builds to run games",
                Some("We recommend custom Proton forks (GE-Proton, Proton-CachyOS, etc.) to be used because Valve's Proton builds lack winetricks and protofixes. You can still use them if you want."),
                false,
                cx,
            ))
            .child(self.toggle_row(
                "proton-to-steam",
                "downloadProtonToSteam",
                false,
                "Download GE-Proton to Steam directory",
                Some("When enabled, GE-Proton will be downloaded directly to the Steam compatibility tools directory instead of the tools folder. It will use the Steam path set in the 'Default Steam path' setting above."),
                false,
                cx,
            ));
        column()
            .child(heading("Folders"))
            .child(folders.card())
            .child(heading("Launching"))
            .child(launch.card())
            .child(heading("System"))
            .child(self.system_rows(cx).card())
            .child(heading("Wine and Proton"))
            .child(wine.card())
            .child(heading("Custom Wine/Proton Paths"))
            .child(self.custom_wine_paths(cx))
    }

    /// Tray, protocol-launch and shortcut settings (`system/`).
    fn system_rows(&self, cx: &mut Context<Self>) -> Rows {
        let no_tray = self.flag("noTrayIcon", false);
        let exit_to_tray = self.flag("exitToTray", false);
        rows()
            .child(self.toggle_row("no-tray", "noTrayIcon", false, "Hide System Tray Icon (requires restart)", None, false, cx))
            .child(self.toggle_row("exit-to-tray", "exitToTray", false, "Exit to System Tray", None, no_tray, cx))
            .child(self.toggle_row("start-in-tray", "startInTray", false, "Start Minimized", None, !exit_to_tray || no_tray, cx))
            .child(self.toggle_row("dark-tray", "darkTrayIcon", false, "Use Dark Tray Icon (needs restart)", None, no_tray, cx))
            .child(self.toggle_row(
                "hide-on-protocol",
                "hideWindowOnProtocolLaunch",
                false,
                "Hide Heronite window when launching games from heronite-rs:// links",
                Some("Keeps the window hidden when a game is launched from an external shortcut, like the 'Add to Steam' feature."),
                false,
                cx,
            ))
            .child(self.toggle_row("shortcuts-desktop", "addDesktopShortcuts", false, "Add desktop shortcuts automatically", None, false, cx))
            .child(self.toggle_row("shortcuts-menu", "addStartMenuShortcuts", false, "Add games to start menu automatically", None, false, cx))
            .child(self.toggle_row("shortcuts-steam", "addSteamShortcuts", false, "Add games to Steam automatically", None, false, cx))
    }

    fn max_workers_field(&self, cx: &mut Context<Self>) -> Div {
        let cores = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1);
        let current = self.number("maxWorkers").unwrap_or(0).max(0) as usize;
        let mut items: Vec<(SharedString, bool)> = vec![("Max".into(), current == 0)];
        items.extend((1..=cores).map(|n| (SharedString::from(n.to_string()), n == current)));
        let shown = if current == 0 { "Max".to_string() } else { current.to_string() };
        div().child(self.select_row(
            "max-workers",
            Some("Maximum Number of Workers when downloading"),
            shown,
            items,
            false,
            cx,
            |this, i, _, cx| this.set("maxWorkers", serde_json::json!(i as i64), cx),
        ))
    }

    fn custom_wine_paths(&mut self, cx: &mut Context<Self>) -> Div {
        let paths: Vec<String> = self.custom_paths();
        let mut card_rows = rows();
        for (i, p) in paths.iter().enumerate() {
            card_rows = card_rows.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .py(px(8.))
                    .child(div().flex_1().min_w_0().truncate().text_size(px(13.)).text_color(theme::text_primary()).child(p.clone()))
                    .child(
                        div()
                            .id(SharedString::from(format!("wine-path-del-{i}")))
                            .flex()
                            .items_center()
                            .justify_center()
                            .size(px(28.))
                            .rounded_full()
                            .cursor_pointer()
                            .hover(|s| s.bg(theme::white_a(0.10)))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                let mut now = this.custom_paths();
                                if i < now.len() {
                                    now.remove(i);
                                    this.set("customWinePaths", serde_json::json!(now), cx);
                                }
                            }))
                            .child(icon(Icon::DeleteOutline, px(16.), theme::text_muted())),
                    ),
            );
        }
        let add = self.outline_button("wine-path-add", "Add path", true, cx, |_, window, cx| {
            choose_path(window, cx, false, "Select the Wine or Proton Binary", |this: &mut Self, path, _, cx| {
                let mut now = this.custom_paths();
                if !now.contains(&path) {
                    now.push(path);
                    this.set("customWinePaths", serde_json::json!(now), cx);
                }
            });
        });
        card_rows = card_rows.child(div().flex().py(px(10.)).child(add));
        card_rows.card()
    }

    pub(super) fn custom_paths(&self) -> Vec<String> {
        self.settings
            .get("customWinePaths")
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect())
            .unwrap_or_default()
    }
}
