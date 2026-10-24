//! The layouts a console tab can use (`heronite_data::console_theme::Layout`).
//! Each lives in its own file with the same three entry points
//! (`render_<layout>`, `<layout>_pad`, `<layout>_hints`) and its own focus
//! state (`Nav`); a tab keeps the focus of every layout, so switching a tab's
//! layout and back finds its place. The shell (`super`) draws the top bar,
//! the hint bar and the background around them.

mod carousel;
mod cinema;
mod dashboard;
mod grid;
mod shelves;
mod spotlight;
mod wheel;
mod xbox;

use gpui_kit::*;
use heronite_data::console_theme::{Layout, TabSpec};

use super::super::CollectionView;
use super::backdrop::Scrim;
use crate::console::{Hint, Pad};

/// Focus of one tab in every layout.
#[derive(Default)]
pub(crate) struct TabNav {
    pub shelves: shelves::Nav,
    pub spotlight: spotlight::Nav,
    pub cinema: cinema::Nav,
    pub dashboard: dashboard::Nav,
    pub grid: grid::Nav,
    pub wheel: wheel::Nav,
    pub carousel: carousel::Nav,
    pub xbox: xbox::Nav,
}

/// The background shading a layout wants under it.
pub(super) fn scrim(layout: Layout) -> Scrim {
    match layout {
        Layout::Shelves => shelves::SCRIM,
        Layout::Spotlight => spotlight::SCRIM,
        Layout::Cinema => cinema::SCRIM,
        Layout::Dashboard => dashboard::SCRIM,
        Layout::Grid => grid::SCRIM,
        Layout::Wheel => wheel::SCRIM,
        Layout::Carousel => carousel::SCRIM,
        Layout::Xbox => xbox::SCRIM,
    }
}

/// `i` moved by `delta` in `0..n`, going round when `wrap`.
pub(crate) fn step(i: usize, n: usize, delta: isize, wrap: bool) -> usize {
    if n == 0 {
        return 0;
    }
    let j = i as isize + delta;
    if wrap {
        j.rem_euclid(n as isize) as usize
    } else {
        j.clamp(0, n as isize - 1) as usize
    }
}

impl CollectionView {
    /// The focus state of tab `tab` (created on first use).
    pub(super) fn tab_nav(&mut self, tab: usize) -> &mut TabNav {
        let navs = &mut self.console.tab_navs;
        if navs.len() <= tab {
            navs.resize_with(tab + 1, TabNav::default);
        }
        &mut navs[tab]
    }

    pub(super) fn render_layout(&mut self, tab: usize, spec: &TabSpec, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        match spec.layout {
            Layout::Shelves => self.render_shelves(tab, spec, window, cx),
            Layout::Spotlight => self.render_spotlight(tab, spec, window, cx),
            Layout::Cinema => self.render_cinema(tab, spec, window, cx),
            Layout::Dashboard => self.render_dashboard(tab, spec, window, cx),
            Layout::Grid => self.render_grid(tab, spec, window, cx),
            Layout::Wheel => self.render_wheel(tab, spec, window, cx),
            Layout::Carousel => self.render_carousel(tab, spec, window, cx),
            Layout::Xbox => self.render_xbox(tab, spec, window, cx),
        }
    }

    /// `false` lets the shell use the press (LB / RB switch tabs, B opens the menu).
    pub(super) fn layout_pad(&mut self, tab: usize, spec: &TabSpec, pad: Pad, window: &mut Window, cx: &mut Context<Self>) -> bool {
        match spec.layout {
            Layout::Shelves => self.shelves_pad(tab, spec, pad, window, cx),
            Layout::Spotlight => self.spotlight_pad(tab, spec, pad, window, cx),
            Layout::Cinema => self.cinema_pad(tab, spec, pad, window, cx),
            Layout::Dashboard => self.dashboard_pad(tab, spec, pad, window, cx),
            Layout::Grid => self.grid_pad(tab, spec, pad, window, cx),
            Layout::Wheel => self.wheel_pad(tab, spec, pad, window, cx),
            Layout::Carousel => self.carousel_pad(tab, spec, pad, window, cx),
            Layout::Xbox => self.xbox_pad(tab, spec, pad, window, cx),
        }
    }

    pub(super) fn layout_hints(&self, tab: usize, spec: &TabSpec) -> Vec<Hint> {
        match spec.layout {
            Layout::Shelves => self.shelves_hints(tab, spec),
            Layout::Spotlight => self.spotlight_hints(tab, spec),
            Layout::Cinema => self.cinema_hints(tab, spec),
            Layout::Dashboard => self.dashboard_hints(tab, spec),
            Layout::Grid => self.grid_hints(tab, spec),
            Layout::Wheel => self.wheel_hints(tab, spec),
            Layout::Carousel => self.carousel_hints(tab, spec),
            Layout::Xbox => self.xbox_hints(tab, spec),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::step;

    #[test]
    fn steps_clamp_or_wrap() {
        assert_eq!(step(0, 5, -1, false), 0);
        assert_eq!(step(0, 5, -1, true), 4);
        assert_eq!(step(4, 5, 1, true), 0);
        assert_eq!(step(2, 5, 7, false), 4);
        assert_eq!(step(0, 0, 1, true), 0);
    }
}
