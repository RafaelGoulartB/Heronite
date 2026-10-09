//! Helpers to open the common overlays (sheet, on-screen keyboard, file
//! picker) in the [`Layer`] of a page or dialog. The owner implements
//! [`HasLayer`] so the overlay can close itself when it is done.

use std::path::{Path, PathBuf};

use gpui_kit::prelude::*;
use gpui_kit::*;

use super::browser::{Browser, BrowserEvent, PickKind};
use super::list::Row;
use super::osk::{Osk, OskEvent};
use super::sheet::{Sheet, SheetEvent};
use super::Layer;

/// An owner of a [`Layer`].
pub trait HasLayer {
    fn layer_mut(&mut self) -> &mut Layer;
}

/// Opens the keyboard on `initial`; `done` gets the typed text (cancelling
/// just closes it).
pub fn open_osk<T: HasLayer + 'static>(
    layer: &mut Layer,
    title: &str,
    initial: &str,
    window: &mut Window,
    cx: &mut Context<T>,
    done: impl Fn(&mut T, String, &mut Window, &mut Context<T>) + 'static,
) {
    let osk = cx.new(|_| Osk::new(title.to_string(), initial));
    let sub = cx.subscribe_in(&osk, window, move |this, _, ev: &OskEvent, window, cx| {
        this.layer_mut().close();
        if let OskEvent::Done(text) = ev {
            done(this, text.clone(), window, cx);
        }
        cx.notify();
    });
    layer.open(&osk, sub);
}

/// Opens the file / folder picker at `start`; `picked` gets the choice.
pub fn open_browser<T: HasLayer + 'static>(
    layer: &mut Layer,
    title: &str,
    kind: PickKind,
    start: Option<&Path>,
    window: &mut Window,
    cx: &mut Context<T>,
    picked: impl Fn(&mut T, PathBuf, &mut Window, &mut Context<T>) + 'static,
) {
    let browser = cx.new(|cx| Browser::new(title.to_string(), kind, start, cx));
    let sub = cx.subscribe_in(&browser, window, move |this, _, ev: &BrowserEvent, window, cx| {
        this.layer_mut().close();
        if let BrowserEvent::Picked(p) = ev {
            picked(this, p.clone(), window, cx);
        }
        cx.notify();
    });
    layer.open(&browser, sub);
}

/// Opens a sheet of action rows; `chosen` gets the id of the row activated
/// (the sheet closes first).
pub fn open_sheet<T: HasLayer + 'static>(
    layer: &mut Layer,
    title: impl Into<SharedString>,
    message: Option<String>,
    rows: Vec<Row>,
    window: &mut Window,
    cx: &mut Context<T>,
    chosen: impl Fn(&mut T, &str, &mut Window, &mut Context<T>) + 'static,
) {
    let sheet = cx.new(|_| Sheet::new(title, message, rows));
    let sub = cx.subscribe_in(&sheet, window, move |this, _, ev: &SheetEvent, window, cx| {
        let SheetEvent::Activate(id) = ev;
        this.layer_mut().close();
        chosen(this, id, window, cx);
        cx.notify();
    });
    layer.open(&sheet, sub);
}
