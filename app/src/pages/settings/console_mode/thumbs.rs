//! Small schematic pictures of the eight layouts, for the tab editor.

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::console_theme::Layout;

use crate::theme;

const W: f32 = 84.;
const H: f32 = 50.;

/// `(x, y, w, h, strength)` of each block, on a W x H grid.
fn blocks(layout: Layout) -> Vec<(f32, f32, f32, f32, f32)> {
    match layout {
        // The focused game on top, rows of covers below.
        Layout::Shelves => {
            let mut v = vec![(6., 6., 34., 5., 0.9), (6., 14., 24., 3., 0.4)];
            for row in 0..2 {
                for i in 0..5 {
                    v.push((6. + i as f32 * 15., 25. + row as f32 * 13., 12., 11., 0.35));
                }
            }
            v
        }
        // Icons on top, the game at the bottom left, cards.
        Layout::Spotlight => {
            let mut v: Vec<_> = (0..5).map(|i| (6. + i as f32 * 9., 5., 7., 7., 0.45)).collect();
            v.extend([(6., 28., 30., 6., 0.9), (6., 37., 16., 6., 0.7), (50., 30., 12., 14., 0.3), (65., 30., 12., 14., 0.3)]);
            v
        }
        // One row of wide covers over the art.
        Layout::Cinema => vec![(6., 8., 30., 3., 0.7), (6., 28., 24., 14., 0.5), (33., 28., 24., 14., 0.35), (60., 28., 24., 14., 0.2)],
        // Tiles of different sizes.
        Layout::Dashboard => vec![
            (5., 5., 38., 24., 0.55),
            (46., 5., 15., 11., 0.35),
            (64., 5., 15., 11., 0.35),
            (46., 19., 33., 10., 0.3),
            (5., 32., 24., 13., 0.3),
            (32., 32., 24., 13., 0.35),
            (59., 32., 20., 13., 0.3),
        ],
        // A grid of covers.
        Layout::Grid => (0..12).map(|i| (6. + (i % 4) as f32 * 19., 5. + (i / 4) as f32 * 15., 16., 12., 0.35)).collect(),
        // Names in a column, the art beside.
        Layout::Wheel => {
            let mut v: Vec<_> = (0..6).map(|i| (6., 6. + i as f32 * 7., if i == 3 { 30. } else { 22. }, 3., if i == 3 { 0.95 } else { 0.35 })).collect();
            v.push((46., 6., 32., 38., 0.3));
            v
        }
        // A large cover in the middle, the neighbours smaller.
        Layout::Carousel => vec![(6., 14., 16., 24., 0.25), (24., 9., 14., 32., 0.35), (31., 6., 22., 38., 0.8), (49., 9., 14., 32., 0.35), (62., 14., 16., 24., 0.25)],
        // A row of tiles, the first one wide, groups below.
        Layout::Xbox => vec![
            (5., 6., 28., 14., 0.7),
            (36., 6., 14., 14., 0.4),
            (53., 6., 14., 14., 0.4),
            (70., 6., 10., 14., 0.3),
            (5., 26., 20., 7., 0.3),
            (28., 26., 20., 7., 0.3),
            (51., 26., 20., 7., 0.3),
            (5., 36., 20., 7., 0.25),
            (28., 36., 20., 7., 0.25),
        ],
    }
}

/// The picture of `layout`; `active` outlines it in the accent.
pub(super) fn thumb(layout: Layout, active: bool) -> Div {
    let mut d = div()
        .relative()
        .flex_none()
        .w(px(W))
        .h(px(H))
        .rounded(px(8.))
        .overflow_hidden()
        .bg(theme::recess_a(0.35))
        .border_1()
        .border_color(if active { theme::accent().into() } else { theme::card_border() });
    for (x, y, w, h, a) in blocks(layout) {
        d = d.child(div().absolute().left(px(x)).top(px(y)).w(px(w)).h(px(h)).rounded(px(2.)).bg(if active { theme::accent_alpha(a * 0.8).into() } else { theme::ink_a(a * 0.5) }));
    }
    d
}
