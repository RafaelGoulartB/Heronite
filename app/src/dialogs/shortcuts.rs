//! `ShortcutsDialog` ("Keyboard shortcuts", the `?` key or the palette):
//! every shortcut, grouped by where it works. Keep it in step with the
//! handlers it describes (`collection/keys.rs`, `grid.rs`, `selection.rs`,
//! `palette/mod.rs`, `pages/settings/nav_keys.rs`).

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_i18n::tr;

use super::DialogEvent;
use crate::theme;
use crate::ui::controls::{caption, kbd};
use crate::ui::dialog::{dialog_scrolled, DialogProps};

/// One line: what it does and its keys (each a chip; `/` between options).
struct Line {
    what: &'static str,
    keys: &'static [&'static str],
}

const fn line(what: &'static str, keys: &'static [&'static str]) -> Line {
    Line { what, keys }
}

/// The groups: their caption and lines (English, translated when drawn).
fn groups() -> [(&'static str, Vec<Line>); 5] {
    [
        (
            tr!("Anywhere"),
            vec![
                line(tr!("Command palette: pages, actions and games"), &["Ctrl K"]),
                line(tr!("Settings"), &["Ctrl ,"]),
                line(tr!("Add a game"), &["Ctrl N"]),
                line(tr!("Collection, Downloads, Wine Manager, Stores, Emulators, Statistics"), &["Ctrl 1…6"]),
                line(tr!("Refresh the libraries"), &["F5"]),
                line(tr!("Fullscreen"), &["F11"]),
                line(tr!("Close a menu or dialog, then the page"), &["Esc"]),
                line(tr!("This list"), &["?"]),
            ],
        ),
        (
            tr!("Collection"),
            vec![
                line(tr!("Search"), &["/", "Ctrl F"]),
                line(tr!("Start a search by typing"), &["A–Z"]),
                line(tr!("Move between games"), &["↑ ↓ ← →", "Home", "End", "PgUp", "PgDn"]),
                line(tr!("Play or install the game"), &["Enter"]),
                line(tr!("Select or unselect the game"), &["Space"]),
                line(tr!("Select every game up to here"), &["Shift ↑ ↓ ← →"]),
                line(tr!("Select every game shown"), &["Ctrl A"]),
                line(tr!("Edit the game, or the selected games"), &["F3"]),
                line(tr!("Undo the last change to selected games"), &["Ctrl Z"]),
            ],
        ),
        (
            tr!("In a search field"),
            vec![
                line(tr!("To the results"), &["↓"]),
                line(tr!("Back to the search (from the first result)"), &["↑"]),
                line(tr!("Play or install the game"), &["Enter"]),
                line(tr!("Empty it, then leave it"), &["Esc"]),
            ],
        ),
        (
            tr!("Command palette"),
            vec![
                line(tr!("Move"), &["↑ ↓", "PgUp", "PgDn"]),
                line(tr!("Run the selected entry"), &["Enter"]),
                line(tr!("A game's actions"), &["→"]),
                line(tr!("Pages, actions or games only"), &["Tab"]),
            ],
        ),
        (
            tr!("Settings"),
            vec![
                line(tr!("Search the settings"), &["Ctrl F"]),
                line(tr!("Move in the sections and results"), &["↑ ↓", "Home", "End"]),
                line(tr!("Open the section or setting"), &["Enter", "Space"]),
            ],
        ),
    ]
}

pub struct ShortcutsDialog {
    scroll: ScrollHandle,
}

impl EventEmitter<DialogEvent> for ShortcutsDialog {}

impl ShortcutsDialog {
    pub fn new() -> Self {
        Self { scroll: ScrollHandle::new() }
    }
}

fn group(title: &'static str, lines: Vec<Line>) -> Div {
    let mut col = div().flex().flex_col().child(caption(title));
    let mut card = div()
        .flex()
        .flex_col()
        .px(px(14.))
        .py(px(4.))
        .rounded(px(theme::R_CARD))
        .bg(theme::card_fill())
        .border_1()
        .border_color(theme::card_border());
    for (i, l) in lines.into_iter().enumerate() {
        let mut keys = div().flex().flex_none().flex_wrap().justify_end().items_center().gap(px(5.)).max_w(px(190.));
        for (j, k) in l.keys.iter().enumerate() {
            if j > 0 {
                keys = keys.child(div().text_size(px(11.)).text_color(theme::text_dim()).child("/"));
            }
            keys = keys.child(kbd(*k));
        }
        card = card.child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .gap(px(14.))
                .py(px(8.))
                .when(i > 0, |d| d.border_t_1().border_color(theme::row_separator()))
                .child(div().flex_1().min_w_0().text_size(px(13.)).line_height(px(18.)).text_color(theme::text_primary()).child(l.what))
                .child(keys),
        );
    }
    col = col.child(card);
    col
}

impl Render for ShortcutsDialog {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Two columns: Anywhere and the palette on the left, the Collection's and Settings' keys on the right.
        let [anywhere, collection, field, palette, settings] = groups();
        let left =
            div().flex().flex_col().flex_1().min_w_0().gap(px(4.)).child(group(anywhere.0, anywhere.1)).child(group(palette.0, palette.1));
        let right = div()
            .flex()
            .flex_col()
            .flex_1()
            .min_w_0()
            .gap(px(4.))
            .child(group(collection.0, collection.1))
            .child(group(field.0, field.1))
            .child(group(settings.0, settings.1));
        let body = div().flex().items_start().gap(px(18.)).child(left).child(right);
        // The scroll's size is known after a first layout.
        if self.scroll.bounds().size.height <= px(0.) {
            let view = cx.entity_id();
            cx.defer(move |cx| cx.notify(view));
        }
        let entity = cx.entity().downgrade();
        let height = (f32::from(window.viewport_size().height) * 0.82).min(720.);
        dialog_scrolled(
            DialogProps {
                id: "shortcuts".into(),
                title: tr!("Keyboard shortcuts").into(),
                width: 960f32.min(f32::from(window.viewport_size().width) * 0.92),
                height: Some(height),
            },
            &self.scroll,
            body.into_any_element(),
            None,
            move |_, cx| {
                entity.update(cx, |_, cx| cx.emit(DialogEvent::Close)).ok();
            },
        )
    }
}
