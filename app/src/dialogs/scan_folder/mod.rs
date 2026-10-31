//! `ScanFolderDialog` ("Add games from a folder"): pick a folder, see the
//! games whose programs [`scan`] finds in it and add the chosen ones to the
//! Collection at once, as local games (what "Add game" adds one at a time).

mod actions;
mod render;
pub mod scan;

use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_i18n::{tr, trn};

use super::{text_of, DialogCtx, DialogEvent};
use crate::theme;
use crate::ui::controls::{action_button, button, ButtonKind};
use crate::ui::dialog::{dialog, DialogProps};
use crate::ui::fields::{meta_text, new_input, on_commit, paragraph};
use scan::Scan;

pub struct ScanFolderDialog {
    ctx: DialogCtx,
    directory: Entity<InputState>,
    /// The folder of the last scan, `scan` its result.
    scanned: String,
    scan: Option<Scan>,
    scanning: bool,
    /// Bumped by each scan: the result of an older one is dropped.
    generation: u64,
    importing: bool,
    error: String,
    /// Linux: each Windows game gets a Wine prefix named after it (else
    /// they share the default one).
    own_prefixes: bool,
    /// The game whose title is being edited, in `rename`.
    renaming: Option<usize>,
    rename: Entity<InputState>,
    /// The game whose list of programs is open.
    programs_open: Option<usize>,
    /// A folder to put in the input once a `Window` is at hand.
    pending_dir: Option<String>,
    _subs: Vec<Subscription>,
}

impl EventEmitter<DialogEvent> for ScanFolderDialog {}

impl ScanFolderDialog {
    pub fn new(ctx: DialogCtx, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let directory = new_input(window, cx, "", tr!("The folder your games are in"));
        // A path typed or pasted is searched once the field is left.
        on_commit(&directory, window, cx, |this: &mut Self, value, _, cx| {
            if value != this.scanned {
                this.scan(cx);
            }
        });
        let rename = new_input(window, cx, "", "");
        let subs = vec![cx.subscribe_in(&rename, window, |this, input, ev: &InputEvent, window, cx| match ev {
            InputEvent::Change => {
                let raw = input.read(cx).value().to_string();
                let clean = crate::dialogs::add_game::clean_title(&raw);
                if clean != raw {
                    input.update(cx, |s, cx| s.set_value(clean, window, cx));
                }
            }
            InputEvent::Blur | InputEvent::PressEnter { .. } => this.finish_rename(cx),
            _ => {}
        })];
        Self {
            ctx,
            directory,
            scanned: String::new(),
            scan: None,
            scanning: false,
            generation: 0,
            importing: false,
            error: String::new(),
            own_prefixes: true,
            renaming: None,
            rename,
            programs_open: None,
            pending_dir: None,
            _subs: subs,
        }
    }

    /// Esc, offered by the host before it closes the dialog: closes a list
    /// of programs or a title being edited first.
    pub fn on_escape(&mut self, cx: &mut Context<Self>) -> bool {
        if self.programs_open.take().is_some() || self.renaming.take().is_some() {
            cx.notify();
            return true;
        }
        false
    }

    /// Debug hook: `folder` searched at once, then the `+`-joined states:
    /// `programs` (the first game with several programs shows them),
    /// `rename` (the first game's title in its field), `none` (nothing
    /// selected), `import` (adds the selected games).
    pub fn debug_state(&mut self, folder: &str, states: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.scan_now(folder);
        for state in states.split('+') {
            match state {
                "programs" => self.programs_open = self.scan.as_ref().and_then(|s| s.games.iter().position(|g| g.programs.len() > 1)),
                "rename" => self.start_rename(0, window, cx),
                "none" => self.select_all(false),
                "import" => self.import(cx),
                _ => {}
            }
        }
        cx.notify();
    }

    fn selected(&self) -> usize {
        self.scan.as_ref().map_or(0, |s| s.games.iter().filter(|g| g.import).count())
    }

    /// The line above the list: how many games the last scan found, and how
    /// many of them the Collection has.
    fn summary(&self) -> Option<String> {
        let scan = self.scan.as_ref()?;
        let found = trn!(scan.games.len(), "{n} game found", "{n} games found");
        let known = scan.games.iter().filter(|g| g.known == scan::Known::Path).count();
        Some(if known > 0 {
            format!("{found} \u{b7} {}", trn!(known, "{n} already in Collection", "{n} already in Collection"))
        } else {
            found
        })
    }
}

impl Render for ScanFolderDialog {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if let Some(dir) = self.pending_dir.take() {
            self.directory.update(cx, |s, cx| s.set_value(dir, window, cx));
        }
        let dir_value = text_of(&self.directory, cx);
        let selected = self.selected();
        let busy = self.scanning || self.importing;

        let can_scan = !dir_value.is_empty() && !busy;
        let scan_button = action_button(
            "sf-scan",
            ButtonKind::Glass,
            if self.scanning {
                tr!("Looking for games\u{2026}")
            } else if self.scan.is_some() && dir_value == self.scanned {
                tr!("Scan again")
            } else {
                tr!("Scan folder")
            },
            can_scan,
            cx.listener(|this, _, _, cx| this.scan(cx)),
        );
        let body = div()
            .flex()
            .flex_col()
            .gap(px(12.))
            .child(paragraph(tr!(
                "Choose the folder your games are in. Heronite looks for the programs that start them, and you choose the games to add."
            )))
            .child(self.render_folder(&dir_value, cx))
            .child(
                div().flex().items_center().gap(px(12.)).child(scan_button).children(
                    self.scan
                        .as_ref()
                        .filter(|s| s.truncated)
                        .map(|_| meta_text(tr!("This folder is very big: only part of it was searched."))),
                ),
            )
            .children(self.render_results(cx))
            .children(self.render_prefix_option(cx))
            .children(
                (!self.error.is_empty()).then(|| div().text_color(theme::danger_text()).text_size(px(13.33)).child(self.error.clone())),
            );

        let footer = div()
            .flex()
            .gap(px(10.))
            .child(button("sf-close", ButtonKind::Ghost, tr!("Close")).on_click(cx.listener(|_, _, _, cx| cx.emit(DialogEvent::Close))))
            .child(action_button(
                "sf-import",
                ButtonKind::Primary,
                if self.importing { tr!("Adding\u{2026}").to_string() } else { tr!("Add {n} to Collection", n = selected) },
                selected > 0 && !busy,
                cx.listener(|this, _, _, cx| this.import(cx)),
            ))
            .into_any_element();
        let entity = cx.entity().downgrade();
        dialog(
            DialogProps {
                id: "scan-folder".into(),
                title: tr!("Add games from a folder").into(),
                width: 760f32.min(f32::from(window.viewport_size().width) * 0.9) + 64.,
                height: None,
            },
            body.into_any_element(),
            Some(footer),
            move |_, cx| {
                entity.update(cx, |_, cx| cx.emit(DialogEvent::Close)).ok();
            },
        )
    }
}
