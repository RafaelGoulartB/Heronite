//! The Sandbox tab (`heroniteSandbox`, see `heronite_launch::sandbox`): the
//! switch, what the sandbox shows the game, and the exceptions, each one
//! saying what it fixes and what it lets through.

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_i18n::{tr, trn};
use heronite_launch::sandbox::{self, Exception, Owner, Sandbox};
use heronite_launch::settings::GameSettings;

use super::logic::RunKind;
use super::parts::{column, heading, help, hint, rows};
use super::SettingsPanel;
use crate::icons::Icon;
use crate::theme;
use crate::ui::controls::toggle;
use crate::ui::icon;

/// What an exception is for and what it costs.
pub(super) fn exception_text(e: Exception) -> (&'static str, &'static str, &'static str) {
    match e {
        Exception::Network => (
            tr!("Internet and local network"),
            tr!("For online games, activations and launchers that sign in."),
            tr!("A program could send out what it can read (its prefix and game folder) and reach other devices and services on your network."),
        ),
        Exception::GameFolder => (
            tr!("Write to the game's folder"),
            tr!("For games that save, update or keep settings in their own folder."),
            tr!("A program could change or delete the game's files."),
        ),
        Exception::SharedFolders => (
            tr!("Folders visible to Proton"),
            tr!("The folders chosen in Compatibility (mods, other drives), writable."),
            tr!("A program could read, change or delete what is in them."),
        ),
        Exception::Devices => (
            tr!("Every device"),
            tr!("For VR headsets, wheels, webcams and controllers plugged in while playing."),
            tr!("A program could use the webcam and, when your user is in the input group, read the keyboard."),
        ),
    }
}

impl SettingsPanel {
    /// Looks at bubblewrap off the UI thread (it runs it once).
    pub(super) fn load_sandbox_host(&mut self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            let state = cx.background_spawn(async move { sandbox::unavailable(&heronite_launch::SystemProbe) }).await;
            this.update(cx, |this, cx| {
                this.sandbox_host = Some(state);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub(super) fn sandbox(&self) -> Sandbox {
        Sandbox::from_map(&self.settings)
    }

    /// Flips the sandbox (`None`) or one of its exceptions.
    pub(super) fn toggle_sandbox(&mut self, exception: Option<Exception>, cx: &mut Context<Self>) {
        let s = self.sandbox();
        let s = match exception {
            None => Sandbox { enabled: !s.enabled, ..s },
            Some(e) => s.with(e, !s.allows(e)),
        };
        self.set(sandbox::KEY, s.to_value(), cx);
    }

    /// The folder the game sees as its home.
    pub(super) fn sandbox_home(&self) -> Option<String> {
        let f = self.facts()?;
        Some(match f.run_kind {
            RunKind::Native => sandbox::private_home(&self.paths, Owner::Game(&f.app_name)),
            _ => {
                let prefix = GameSettings::from_map(&self.settings, &self.home).wine_prefix;
                sandbox::private_home(&self.paths, Owner::Prefix(&prefix))
            }
        })
    }

    /// The other games of this game's prefix (Windows games).
    pub(super) fn prefix_neighbours(&self) -> usize {
        let Some(f) = self.facts().filter(|f| f.run_kind == RunKind::Wine) else { return 0 };
        let prefix = GameSettings::from_map(&self.settings, &self.home).wine_prefix;
        let prefix = prefix.trim_end_matches('/');
        self.prefix_list
            .iter()
            .flatten()
            .find(|p| p.path.to_string_lossy().trim_end_matches('/') == prefix)
            .map_or(0, |p| p.games.iter().filter(|g| **g != f.app_name).count())
    }

    pub(super) fn sandbox_tab(&mut self, cx: &mut Context<Self>) -> Div {
        let s = self.sandbox();
        let unavailable = self.sandbox_host.clone().flatten().filter(|_| s.enabled);
        let mut card = rows().child(self.sandbox_switch(
            "sandbox",
            tr!("Run in a sandbox"),
            tr!("What this game runs, and what runs in its prefix (installers, Winetricks, Run EXE), starts without network and without your home folder. It sees the system read-only, its prefix, its game folder (read-only), the graphics card, the display, the sound and the controllers."),
            s.enabled,
            false,
            None,
            cx,
        ));
        if let Some(why) = &unavailable {
            card = card.child(warning(why));
        }
        if let Some(home) = self.sandbox_home().filter(|_| s.enabled && unavailable.is_none()) {
            let open = home.clone();
            card = card.field_help(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap(px(12.))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .min_w_0()
                            .gap(px(2.))
                            .child(div().text_size(px(14.)).text_color(theme::text_primary()).child(tr!("Its home folder")))
                            .child(help(&home)),
                    )
                    .child(self.outline_button("sandbox-home", tr!("Open"), true, cx, move |_, _, _| {
                        let _ = std::fs::create_dir_all(&open);
                        crate::ui::open_external(&open);
                    })),
                vec![help(tr!(
                    "Saves, settings and caches the game writes to its home stay here between runs. The first runs build its shader cache again, so it may stutter for a while."
                ))],
            );
        }
        let mut col = column().child(heading(tr!("Sandbox"))).child(card.card());
        if unavailable.is_some() {
            col = col.child(hint(tr!(
                "Install bubblewrap from your distribution: sudo pacman -S bubblewrap · sudo apt install bubblewrap · sudo dnf install bubblewrap"
            )));
        }

        let mut exceptions = rows();
        for e in Exception::ALL {
            let (label, fixes, risk) = exception_text(e);
            let on = s.allows(e);
            exceptions = exceptions.child(self.sandbox_switch(exception_id(e), label, fixes, on, !s.enabled, Some((risk, on)), cx));
        }
        col =
            col.child(heading(tr!("Exceptions")))
                .child(
                    div().px(px(6.)).pb(px(4.)).child(help(tr!(
                        "Off until a game needs one. Each lets one thing through: turn on only what makes this game work."
                    ))),
                )
                .child(exceptions.card());
        let neighbours = self.prefix_neighbours();
        if s.enabled && neighbours > 0 {
            col = col.child(hint(&trn!(
                neighbours,
                "Another game uses this prefix. What this game installs in it reaches that game, which runs outside the sandbox unless it is on there too. A prefix of its own keeps it apart.",
                "{n} other games use this prefix. What this game installs in it reaches them, and they run outside the sandbox unless it is on for them too. A prefix of its own keeps it apart."
            )));
        }
        col.child(div().px(px(6.)).pt(px(8.)).child(help(tr!(
            "Outside the sandbox, as before: your scripts before and after launch, cloud saves and the store's own tools (legendary, gogdl, nile)."
        ))))
    }

    /// A switch row of the tab; `risk`: the exception's cost, stronger
    /// while it is on.
    fn sandbox_switch(
        &self,
        id: &'static str,
        label: &str,
        about: &str,
        on: bool,
        disabled: bool,
        risk: Option<(&str, bool)>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let mut text = div()
            .flex()
            .flex_col()
            .flex_1()
            .min_w_0()
            .gap(px(2.))
            .child(div().text_size(px(14.)).text_color(theme::text_primary()).child(label.to_string()))
            .child(div().text_size(px(12.)).line_height(px(16.)).text_color(theme::text_dim()).child(about.to_string()));
        if let Some((risk, active)) = risk {
            let color = if active { theme::accent() } else { theme::text_muted() };
            text = text.child(div().text_size(px(12.)).line_height(px(16.)).text_color(color).child(tr!("Risk: {risk}", risk).to_string()));
        }
        let row = div()
            .id(id)
            .flex()
            .items_center()
            .justify_between()
            .gap(px(16.))
            .py(px(11.))
            .child(text)
            .child(div().flex_none().child(toggle(SharedString::from(format!("{id}-sw")), on, disabled)));
        let exception = Exception::ALL.into_iter().find(|e| exception_id(*e) == id);
        match disabled {
            true => row.opacity(0.4).cursor_not_allowed().into_any_element(),
            false => {
                row.cursor_pointer().on_click(cx.listener(move |this, _, _, cx| this.toggle_sandbox(exception, cx))).into_any_element()
            }
        }
    }
}

pub(super) fn exception_id(e: Exception) -> &'static str {
    match e {
        Exception::Network => "sandbox-network",
        Exception::GameFolder => "sandbox-game-folder",
        Exception::SharedFolders => "sandbox-shared-folders",
        Exception::Devices => "sandbox-devices",
    }
}

/// The sandbox is on but cannot run: the game does not start.
fn warning(text: &str) -> Div {
    div()
        .flex()
        .items_start()
        .gap(px(8.))
        .py(px(11.))
        .child(icon(Icon::Warning, px(15.), theme::accent()))
        .child(div().text_size(px(12.)).line_height(px(16.)).text_color(theme::accent()).child(text.to_string()))
}
