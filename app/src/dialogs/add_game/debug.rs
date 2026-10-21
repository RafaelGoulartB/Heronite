//! Debug hook for pre-filling the dialog.

use gpui_kit::component::input::InputState;
use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::sideload::SideloadPlatform;

use super::picker::Slot;
use super::{AddGameDialog, Select};

impl AddGameDialog {
    /// `HERONITE_DEBUG_DIALOG=add-game:<state>`: pre-fills the form like a user would.
    pub fn debug_state(&mut self, state: &str, window: &mut Window, cx: &mut Context<Self>) {
        let set = |input: &Entity<InputState>, v: &str, window: &mut Window, cx: &mut Context<Self>| {
            input.update(cx, |s, cx| s.set_value(v.to_string(), window, cx));
        };
        for part in state.split('+') {
            match part {
                "windows" => {
                    set(&self.title, "Sample Adventure", window, cx);
                    set(&self.exe, "/data/Games/Sample Adventure/sample.exe", window, cx);
                    self.platform = SideloadPlatform::Windows;
                }
                "linux" => {
                    set(&self.title, "Native Racer", window, cx);
                    set(&self.exe, "/data/Games/native-racer/start.sh", window, cx);
                    self.platform = SideloadPlatform::Linux;
                    self.platform_manual = true;
                }
                "command" => {
                    set(&self.title, "Native Port", window, cx);
                    set(&self.folder, "/tmp/hr-port", window, cx);
                    set(&self.command, "./start.sh --fullscreen", window, cx);
                    self.platform = SideloadPlatform::Linux;
                    self.platform_manual = true;
                    self.by_command = true;
                }
                "browser" => {
                    set(&self.title, "Browser Quest", window, cx);
                    set(&self.url, "https://browserquest.example.com/play", window, cx);
                    self.platform = SideloadPlatform::Browser;
                    self.platform_manual = true;
                }
                "shared" => self.set_shared_prefix(true, window, cx),
                "wine" => self.show_wine = true,
                "wine-open" => {
                    self.show_wine = true;
                    self.open_select = Some(Select::Wine);
                }
                "art-menu" => self.art_menu = Some(Slot::Square),
                "address" => self.art_address = Some(Slot::Cover),
                "no-note" => self.store_note = false,
                "remove" => self.confirm_remove = true,
                // Exercises the save path (fixture data only).
                "save" => self.save(cx),
                "remove-now" => self.remove(cx),
                "noprefix" => self.removable_prefix = None,
                "picker" => self.open_picker(Slot::Square, window, cx),
                art if art.starts_with("art=") => {
                    // art=<cover url>[,<hero url>]
                    let urls: Vec<&str> = art[4..].split(',').collect();
                    set(&self.square, urls[0], window, cx);
                    if let Some(h) = urls.get(1) {
                        set(&self.hero, h, window, cx);
                    }
                }
                _ => {}
            }
        }
        // As if typed: the prefix follows the title.
        self.title_changed(window, cx);
        cx.notify();
    }
}
