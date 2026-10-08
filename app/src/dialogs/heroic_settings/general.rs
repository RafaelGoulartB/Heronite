//! "General" tab of the global settings: the subset of Heroic's general and
//! advanced settings that matter to this app.

use gpui_kit::prelude::*;
use gpui_kit::*;
use serde_json::Value;

use super::parts::{column, heading, help};
use super::HeroicSettingsPanel;
use crate::icons::Icon;
use crate::theme;
use crate::ui::fields::choose_path;
use crate::ui::icon;

impl HeroicSettingsPanel {
    pub(super) fn general_tab(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> Div {
        let col = column()
            .child(self.path_row("defaultInstallPath", Some("Default Installation Path"), true, "Choose the default installation folder", false, cx))
            .child(self.path_row("defaultWinePrefixDir", Some("Set Folder for new Wine Prefixes"), true, "Select a Folder for new Wine Prefixes", false, cx))
            .child(self.path_row("defaultSteamPath", Some("Default Steam path"), true, "Steam path.", false, cx))
            .child(help("This path lets Heroic determine what version of Proton Steam uses, for adding non-Steam games to Steam."))
            .child(self.path_row("altLegendaryBin", Some("Choose an Alternative Legendary Binary"), false, "Select Legendary Binary", true, cx))
            .child(self.max_workers_field(cx))
            .child(self.toggle_row("minimize", "minimizeOnLaunch", false, "Minimize after game launch", None, false, cx))
            .child(self.system_rows(cx))
            .child(heading("Wine and Proton"))
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
                Some("When enabled, GE-Proton will be downloaded directly to the Steam compatibility tools directory instead of the default Heroic path. It will use the Steam path set in the 'Default Steam path' setting above."),
                false,
                cx,
            ));
        col.child(self.custom_wine_paths(cx))
    }

    /// Tray, protocol-launch and shortcut settings (`system/`).
    fn system_rows(&self, cx: &mut Context<Self>) -> Div {
        let no_tray = self.flag("noTrayIcon", false);
        let exit_to_tray = self.flag("exitToTray", false);
        column()
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
        div().w(px(220.)).child(self.select_row(
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
        let paths: Vec<String> = self
            .settings
            .get("customWinePaths")
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect())
            .unwrap_or_default();
        let mut list = div().flex().flex_col().gap(px(4.));
        for (i, p) in paths.iter().enumerate() {
            list = list.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .px(px(10.))
                    .py(px(4.))
                    .rounded(px(4.))
                    .bg(theme::white_a(0.04))
                    .child(div().flex_1().min_w_0().truncate().text_size(px(14.)).text_color(white()).child(p.clone()))
                    .child(
                        div()
                            .id(SharedString::from(format!("wine-path-del-{i}")))
                            .p(px(4.))
                            .rounded(px(4.))
                            .cursor_pointer()
                            .hover(|s| s.bg(theme::white_a(0.12)))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                let mut now = this.custom_paths();
                                if i < now.len() {
                                    now.remove(i);
                                    this.set("customWinePaths", serde_json::json!(now), cx);
                                }
                            }))
                            .child(icon(Icon::DeleteOutline, px(18.), rgb(0xcaf3fd))),
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
        div()
            .flex()
            .flex_col()
            .gap(px(8.))
            .child(heading("Custom Wine/Proton Paths"))
            .child(list)
            .child(div().flex().child(add))
    }

    fn custom_paths(&self) -> Vec<String> {
        self.settings
            .get("customWinePaths")
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect())
            .unwrap_or_default()
    }
}
