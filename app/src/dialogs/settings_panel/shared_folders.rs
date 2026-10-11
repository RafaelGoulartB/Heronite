//! "Folders visible to Proton" in the global Wine tab: the folders the Steam
//! Linux Runtime container mounts for games and installers, each also given
//! a drive letter in the prefix (see `heronite_launch::shared_folders`).

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_launch::shared_folders::{parse, KEY};
use serde_json::Value;

use super::parts::{column, heading, help, rows};
use super::SettingsPanel;
use crate::icons::Icon;
use crate::theme;
use crate::ui::fields::choose_path;
use crate::ui::icon;

impl SettingsPanel {
    pub(super) fn shared_folders_section(&mut self, cx: &mut Context<Self>) -> Div {
        let folders = parse(self.settings.get(KEY));
        let mut list = rows();
        for (i, folder) in folders.iter().enumerate() {
            list = list.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .py(px(8.))
                    .child(icon(Icon::FolderOpen, px(18.), theme::text_muted()))
                    .child(div().flex_1().min_w_0().truncate().text_size(px(14.)).text_color(theme::text_primary()).child(folder.clone()))
                    .child(
                        div()
                            .id(SharedString::from(format!("shared-folder-del-{i}")))
                            .flex()
                            .items_center()
                            .justify_center()
                            .size(px(28.))
                            .rounded_full()
                            .cursor_pointer()
                            .hover(|s| s.bg(theme::white_a(0.10)))
                            .child(icon(Icon::DeleteOutline, px(16.), theme::text_muted()))
                            .on_click(cx.listener(move |this, _, _, cx| this.remove_shared_folder(i, cx))),
                    ),
            );
        }
        list = list.child(div().py(px(10.)).child(self.outline_button("shared-folder-add", "Add folder", true, cx, |_, window, cx| {
            choose_path(window, cx, true, "Choose a folder for Proton to see", |this: &mut Self, path, _, cx| this.add_shared_folder(path, cx));
        })));
        column()
            .child(heading("Folders visible to Proton"))
            .child(help(
                "Games and installers run with Proton (umu) inside the Steam Linux Runtime, which only sees your home folder, /mnt, /media and /run/media. Add other folders here (for example a second drive mounted at /data) so file pickers and games can reach them. Each folder also gets a drive letter (D:, E:, ...) in the prefix the next time something runs in it.",
            ).px(px(6.)))
            .child(list.card())
    }

    fn add_shared_folder(&mut self, path: String, cx: &mut Context<Self>) {
        let mut folders = parse(self.settings.get(KEY));
        folders.push(path);
        let folders = parse(Some(&Value::from(folders)));
        self.set(KEY, Value::from(folders), cx);
    }

    fn remove_shared_folder(&mut self, index: usize, cx: &mut Context<Self>) {
        let mut folders = parse(self.settings.get(KEY));
        if index < folders.len() {
            folders.remove(index);
            self.set(KEY, Value::from(folders), cx);
        }
    }
}
