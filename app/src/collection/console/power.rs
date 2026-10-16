//! Power in the console menu: suspend, restart or shut down the computer
//! from the couch, or quit Heronite. Restart and shut down ask first, naming
//! the games that would close (see `system::power`).

use gpui_kit::*;
use heronite_i18n::tr;

use super::super::CollectionView;
use crate::console::list::Row;
use crate::console::sheet::{Sheet, SheetEvent};
use crate::icons::Icon;
use crate::system::power::{self, PowerAction};

impl CollectionView {
    pub(super) fn console_power_menu(&mut self, cx: &mut Context<Self>) {
        let rows = vec![
            Row::action("suspend", tr!("Suspend")).icon(Icon::Moon),
            Row::action("reboot", tr!("Restart")).icon(Icon::Replay),
            Row::action("poweroff", tr!("Shut down")).icon(Icon::PowerSettingsNew),
            Row::action("quit", tr!("Quit Heronite")).icon(Icon::Close),
        ];
        let sheet = cx.new(|_| Sheet::new(tr!("Power"), None, rows));
        let sub = cx.subscribe(&sheet, |this, _, ev: &SheetEvent, cx| {
            let SheetEvent::Activate(id) = ev;
            this.close_console_overlay(cx);
            match id.as_ref() {
                "suspend" => this.run_power(PowerAction::Suspend, cx),
                "reboot" => this.confirm_power(PowerAction::Reboot, cx),
                "poweroff" => this.confirm_power(PowerAction::PowerOff, cx),
                // Asks first when games run; not while this view is updating.
                "quit" => cx.defer(crate::system::request_quit),
                _ => {}
            }
        });
        self.open_console_overlay(&sheet, sub, cx);
    }

    /// `HERONITE_DEBUG_CONSOLE=power[:reboot|:poweroff]`: opens the menu or
    /// a question (`which` is what follows `power`).
    pub(super) fn debug_power(&mut self, which: &str, cx: &mut Context<Self>) {
        match which {
            ":reboot" => self.confirm_power(PowerAction::Reboot, cx),
            ":poweroff" => self.confirm_power(PowerAction::PowerOff, cx),
            _ => self.console_power_menu(cx),
        }
    }

    /// "Restart the computer?" / "Shut down the computer?". Cancel comes
    /// first and has the focus, so pressing A twice does not shut down.
    fn confirm_power(&mut self, action: PowerAction, cx: &mut Context<Self>) {
        let (title, yes) = match action {
            PowerAction::Reboot => (tr!("Restart the computer?"), tr!("Restart")),
            _ => (tr!("Shut down the computer?"), tr!("Shut down")),
        };
        let rows = vec![Row::action("no", tr!("Cancel")).icon(Icon::Close), Row::action("yes", yes).icon(Icon::PowerSettingsNew)];
        let sheet = cx.new(|_| Sheet::new(title, self.running_games_note(), rows));
        let sub = cx.subscribe(&sheet, move |this, _, ev: &SheetEvent, cx| {
            let SheetEvent::Activate(id) = ev;
            this.close_console_overlay(cx);
            if id.as_ref() == "yes" {
                this.run_power(action, cx);
            }
        });
        self.open_console_overlay(&sheet, sub, cx);
    }

    /// What closes with the computer, `None` when no game runs.
    fn running_games_note(&self) -> Option<String> {
        let lib = self.lib.as_ref();
        let mut titles: Vec<String> =
            self.playing.keys().map(|k| lib.and_then(|l| l.get(k)).map_or_else(|| k.clone(), |g| g.title.clone())).collect();
        titles.sort();
        let names = crate::dialogs::quit_confirm::name_list(&titles);
        match titles.len() {
            0 => None,
            1 => Some(tr!(
                "{names} is running and will close: progress the game has not saved is lost. Heronite saves the playtime so far.",
                names
            )),
            _ => Some(tr!(
                "{names} are running and will close: progress the games have not saved is lost. Heronite saves the playtime so far.",
                names
            )),
        }
    }

    /// Asks the system off the UI thread; a refusal becomes a toast.
    fn run_power(&mut self, action: PowerAction, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            let res = cx.background_spawn(async move { power::run(action) }).await;
            if let Err(err) = res {
                log::warn!("power {action:?}: {err}");
                this.update(cx, |this, cx| this.show_toast(err, true, cx)).ok();
            }
        })
        .detach();
    }
}
