//! The client pages in console mode: the client's state and actions as
//! console rows, its games below. Removing a client asks first (a sheet);
//! the prefix of an existing install is picked with the console browser.

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_i18n::{tr, trn};

use super::{logic, ClientKind, ClientPage};
use crate::console::browser::PickKind;
use crate::console::kit::{self, u};
use crate::console::layers::{open_browser, open_sheet, HasLayer};
use crate::console::list::{Outcome, Row};
use crate::console::{hint, Btn, Hint, Layer, Pad, PadTarget};
use crate::icons::Icon;
use crate::pages::{stores, PageEvent};
use crate::services::clients::{Activity, ClientState};

impl<K: ClientKind> HasLayer for ClientPage<K> {
    fn layer_mut(&mut self) -> &mut Layer {
        &mut self.layer
    }
}

impl<K: ClientKind> ClientPage<K> {
    fn console_rows(&self, st: &ClientState) -> Vec<Row> {
        let client = K::CLIENT;
        let pill = logic::pill(st);
        let main = logic::main_action(client, st);
        let mut rows = vec![Row::header(client.name()), Row::info("status", client.name(), pill.text()).icon(K::PAGE.icon())];
        let mut action = Row::action(main.id, main.label.clone()).enabled(main.enabled).icon(match main.id {
            "close" | "cancel" => Icon::Stop,
            "setup" => Icon::Download,
            _ => Icon::PlayArrow,
        });
        if let Activity::SettingUp(progress) = &st.activity {
            action = action.desc(logic::step_text(client, progress, st.cancelling()));
        } else if let Some(why) = main.why {
            action = action.desc(why);
        } else if !st.installed() {
            action =
                action.desc(tr!("Downloads the official installer and installs it in a Wine prefix of its own, with your default Proton."));
        }
        rows.push(action);
        if st.installed() && !st.busy() {
            rows.push(Row::action("settings", tr!("Wine settings")).desc(self.wine.clone().unwrap_or_default()).icon(Icon::Settings));
            if let Some(prefix) = st.prefix() {
                rows.push(
                    Row::action("folder", tr!("Open the Wine prefix folder")).desc(logic::tilde(prefix, &home())).icon(Icon::FolderOpen),
                );
            }
            rows.push(Row::action("forget", tr!("Remove from Heronite")).icon(Icon::Logout).danger());
        } else if !st.installed() && !st.busy() && st.status.is_some() {
            rows.push(Row::action("adopt", tr!("Use an existing install")).icon(Icon::FolderOpen));
        }
        if let Some(notice) = &st.notice {
            rows.push(Row::info("notice", notice.text.clone(), ""));
        }
        if st.installed() {
            let (title, hint) = logic::account_lines(client, &st.account());
            rows.push(Row::header(tr!("Account")));
            rows.push(Row::info("account", title, "").desc(hint.unwrap_or_default()).icon(Icon::AccountCircle));
            rows.push(Row::header(tr!("Installed games")));
            let games = st.status.as_ref().map(|s| s.games.as_slice()).unwrap_or(&[]);
            if games.is_empty() {
                rows.push(Row::info(
                    "none",
                    tr!("No games yet. Install them in {client}: they come into your Collection by themselves.", client = client.name()),
                    "",
                ));
            }
            for game in games {
                let shown = st.in_collection_game(&heronite_clients::collection::app_name(client, &game.id));
                let folder = game.folder.as_ref().map(|f| logic::tilde(f, &home())).unwrap_or_default();
                rows.push(
                    Row::action(format!("game:{}", game.id), game.title.clone()).desc(folder).icon(Icon::SportsEsports).enabled(shown),
                );
            }
            let when = st.last.as_ref().and_then(|l| logic::synced_text(&l.at, chrono::Local::now())).unwrap_or_default();
            rows.push(
                Row::action("sync", if st.syncing { tr!("Refreshing\u{2026}") } else { tr!("Refresh the games") })
                    .desc(when)
                    .icon(Icon::Refresh)
                    .enabled(!st.syncing && !st.busy()),
            );
        }
        rows
    }

    fn console_forget(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let client = K::CLIENT;
        let st = self.svc.read(cx).state(client);
        let path = st.prefix().map(|p| logic::tilde(p, &home())).unwrap_or_default();
        let mut message = tr!("Its Wine prefix and the games installed in it stay on disk at {path}.", path = path);
        if st.in_collection > 0 {
            message = format!(
                "{message}\n{}",
                trn!(st.in_collection, "Its {n} game leaves your Collection.", "Its {n} games leave your Collection.")
            );
        }
        let rows = vec![Row::action("no", tr!("Cancel")), Row::action("yes", tr!("Remove")).danger()];
        open_sheet(
            &mut self.layer,
            tr!("Remove {client} from Heronite?", client = client.name()),
            Some(message),
            rows,
            window,
            cx,
            |this, id, window, cx| {
                if id == "yes" {
                    this.run_action("forget-yes", window, cx);
                }
            },
        );
    }

    fn console_activate(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        match id {
            "forget" => self.console_forget(window, cx),
            "adopt" => {
                open_browser(&mut self.layer, tr!("Wine prefix"), PickKind::Folder, None, window, cx, |this: &mut Self, path, _, cx| {
                    this.use_existing(path, cx)
                })
            }
            id => match id.strip_prefix("game:") {
                Some(game) => self.show_game(game, cx),
                None => self.run_action(id, window, cx),
            },
        }
    }

    pub(super) fn render_console(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let st = self.svc.read(cx).state(K::CLIENT).clone();
        let rows = self.console_rows(&st);
        let list = self.console_list.render(self.id("console"), &rows, !self.layer.is_open() && self.dialog.is_none());
        div()
            .relative()
            .flex()
            .flex_col()
            .size_full()
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .h(u(76.))
                    .px(u(46.))
                    .gap(u(36.))
                    .child(kit::wordmark(&stores::title().to_uppercase()))
                    .child(stores::console_tabs(K::PAGE))
                    .child(div().flex_1())
                    .child(kit::clock()),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_h_0()
                    .px(u(46.))
                    .pb(u(16.))
                    .child(div().w(u(900.)).max_w_full().flex().flex_col().flex_1().min_h_0().child(list.flex_1().min_h_0())),
            )
            .children(self.layer.view())
            .children(self.dialog.clone())
            .into_any_element()
    }
}

fn home() -> String {
    std::env::var("HOME").unwrap_or_default()
}

impl<K: ClientKind> PadTarget for ClientPage<K> {
    fn on_pad(&mut self, pad: Pad, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if self.layer.pad(pad, window, cx) {
            cx.notify();
            return true;
        }
        if let Some(view) = self.dialog.clone() {
            let used = crate::console::registry::hook_for(&view).is_some_and(|h| h.pad(pad, window, cx));
            if !used && pad == Pad::B {
                self.dialog = None;
                self.dialog_sub = None;
            }
            cx.notify();
            return true;
        }
        if let Some(next) = stores::switch(K::PAGE, pad) {
            cx.emit(PageEvent::Navigate(next));
            return true;
        }
        let st = self.svc.read(cx).state(K::CLIENT).clone();
        let rows = self.console_rows(&st);
        let used = match self.console_list.on_pad(pad, &rows) {
            Outcome::Activate(id) => {
                self.console_activate(&id, window, cx);
                true
            }
            Outcome::Ignored => false,
            _ => true,
        };
        cx.notify();
        used
    }

    fn hints(&self, cx: &App) -> Vec<Hint> {
        if let Some(h) = self.layer.hints(cx) {
            return h;
        }
        if let Some(view) = &self.dialog {
            return crate::console::registry::hook_for(view).map(|h| h.hints(cx)).unwrap_or_else(|| vec![hint(Btn::B, tr!("Close"))]);
        }
        vec![hint(Btn::A, tr!("Select"))]
    }
}
