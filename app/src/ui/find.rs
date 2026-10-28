//! The setting the Settings search jumped to: the row, field label or
//! caption whose text is the target ([`set_target`]) draws a short accent flash around itself
//! and records where it is, so the page can scroll it into view.
//!
//! The shared row helpers (`controls::row`, `controls::caption`,
//! `fields::field_label`) draw [`flash`]; a setting drawn through them is
//! found without code of its own.
//!
//! Debug: `HERONITE_DEBUG_FIND_LABELS=1` logs every text they draw once
//! (`find-label: <text>`), to list what a section shows.

use std::cell::RefCell;
use std::collections::HashSet;
use std::time::Duration;

use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::theme;

/// How long the flash lasts.
pub const FLASH: Duration = Duration::from_millis(1800);

struct Target {
    text: SharedString,
    /// Tells two jumps to the same setting apart, so the flash plays again.
    generation: u64,
    /// Window bounds of the element, from its last prepaint.
    bounds: Option<Bounds<Pixels>>,
}

thread_local! {
    static TARGET: RefCell<Option<Target>> = const { RefCell::new(None) };
    static GENERATION: RefCell<u64> = const { RefCell::new(0) };
    static LOGGED: RefCell<Option<HashSet<String>>> = const { RefCell::new(None) };
}

/// Flashes the element drawn with `text` from the next frame on.
pub fn set_target(text: impl Into<SharedString>) {
    let generation = GENERATION.with(|g| {
        *g.borrow_mut() += 1;
        *g.borrow()
    });
    TARGET.with(|t| *t.borrow_mut() = Some(Target { text: text.into(), generation, bounds: None }));
}

/// Ends the flash.
pub fn clear() {
    TARGET.with(|t| *t.borrow_mut() = None);
}

/// Where the flashed element was drawn, once it was.
pub fn target_bounds() -> Option<Bounds<Pixels>> {
    TARGET.with(|t| t.borrow().as_ref().and_then(|t| t.bounds))
}

fn log_label(text: &str) {
    if !crate::env::flag("HERONITE_DEBUG_FIND_LABELS") {
        return;
    }
    LOGGED.with(|l| {
        if l.borrow_mut().get_or_insert_with(HashSet::new).insert(text.to_string()) {
            log::info!("find-label: {text}");
        }
    });
}

/// The flash of an element drawn with `text`, when it is the target: put it
/// first in the element (painted under its content), which must be
/// `relative()`. `out` is how far it reaches past the element's sides.
pub fn flash(text: &str, out: f32) -> Option<AnyElement> {
    log_label(text);
    let generation = TARGET.with(|t| t.borrow().as_ref().filter(|t| t.text.as_ref() == text).map(|t| t.generation))?;
    let flash = div()
        .absolute()
        .top(px(-4.))
        .bottom(px(-4.))
        .left(px(-out))
        .right(px(-out))
        .rounded(px(10.))
        .border_1()
        // Records the bounds (the page scrolls to them); the first element
        // drawn with the text wins.
        .child(
            canvas(
                move |bounds, _, _| {
                    TARGET.with(|t| {
                        if let Some(t) = t.borrow_mut().as_mut().filter(|t| t.generation == generation && t.bounds.is_none()) {
                            t.bounds = Some(bounds);
                        }
                    })
                },
                |_, _, _, _| {},
            )
            .size_full(),
        );
    Some(if theme::animations() {
        flash
            .with_animation(ElementId::NamedInteger("find-flash".into(), generation), Animation::new(FLASH), |el, t| {
                // A quick rise, then a slow fade.
                let k = if t < 0.12 { t / 0.12 } else { 1. - (t - 0.12) / 0.88 };
                el.bg(theme::accent_alpha(0.18 * k)).border_color(theme::accent_alpha(0.75 * k))
            })
            .into_any_element()
    } else {
        flash.bg(theme::accent_alpha(0.14)).border_color(theme::accent_alpha(0.6)).into_any_element()
    })
}
