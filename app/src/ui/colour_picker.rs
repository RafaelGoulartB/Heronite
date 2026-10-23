//! A colour picker the classic way: a saturation / brightness square and a
//! hue strip to click or drag on, and a hex field. An entity; its owner
//! subscribes to [`PickerEvent`] and moves it with [`ColourPicker::set`].

use std::cell::Cell;
use std::rc::Rc;

use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::appearance::{from_hsv, hex, parse_hex, to_hsv, Rgb};

use crate::theme;
use crate::ui::fields::input_box;
use crate::ui::NoDragView;

/// Height of the square and of the hue strip, px.
const SQUARE_H: f32 = 150.;
const STRIP_H: f32 = 14.;
/// The hue strip's colours, one band between two of them (GPUI gradients
/// take two stops).
const HUES: [f32; 7] = [0., 60., 120., 180., 240., 300., 360.];

pub enum PickerEvent {
    /// The colour moves (a drag).
    Change(Rgb),
    /// The colour is chosen: the drag ended, a click, a hex typed.
    Commit(Rgb),
}

impl EventEmitter<PickerEvent> for ColourPicker {}

/// What is being dragged; `owner` tells pickers apart.
struct SquareDrag {
    owner: EntityId,
}
struct HueDrag {
    owner: EntityId,
}

pub struct ColourPicker {
    /// Hue 0..360, saturation and value 0..=1: kept apart from the colour so
    /// the hue survives dragging to white or black.
    hsv: (f32, f32, f32),
    hex: Entity<InputState>,
    square: Rc<Cell<Bounds<Pixels>>>,
    strip: Rc<Cell<Bounds<Pixels>>>,
    /// A drag started in this picker: its mouse-up commits the colour.
    dragging: bool,
    /// Set while the hex field is moved to the colour, so its own event does
    /// not write it back.
    syncing: bool,
}

impl ColourPicker {
    pub fn new(c: Rgb, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input = crate::ui::fields::new_input(window, cx, &hex(c), "#rrggbb");
        cx.subscribe_in(&input, window, |this: &mut Self, input, ev: &InputEvent, _, cx| {
            if !matches!(ev, InputEvent::Change) || this.syncing {
                return;
            }
            let Some(c) = parse_hex(&input.read(cx).value()) else { return };
            if c == this.colour() {
                return;
            }
            this.hsv = to_hsv(c);
            cx.emit(PickerEvent::Commit(c));
            cx.notify();
        })
        .detach();
        Self {
            hsv: to_hsv(c),
            hex: input,
            square: Rc::new(Cell::new(Bounds::default())),
            strip: Rc::new(Cell::new(Bounds::default())),
            dragging: false,
            syncing: false,
        }
    }

    pub fn colour(&self) -> Rgb {
        from_hsv(self.hsv.0, self.hsv.1, self.hsv.2)
    }

    /// Moves the picker to `c` without an event (the owner changed it).
    pub fn set(&mut self, c: Rgb, window: &mut Window, cx: &mut Context<Self>) {
        if c != self.colour() {
            self.hsv = to_hsv(c);
        }
        self.sync_hex(window, cx);
        cx.notify();
    }

    fn sync_hex(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = hex(self.colour());
        if self.hex.read(cx).value() == text {
            return;
        }
        self.syncing = true;
        self.hex.update(cx, |s, cx| s.set_value(text, window, cx));
        self.syncing = false;
    }

    /// The pointer at `pos` over the square: saturation across, value up.
    fn pick_square(&mut self, pos: Point<Pixels>, window: &mut Window, cx: &mut Context<Self>) {
        let b = self.square.get();
        let x = f32::from(pos.x - b.origin.x) / f32::from(b.size.width).max(1.);
        let y = f32::from(pos.y - b.origin.y) / f32::from(b.size.height).max(1.);
        self.hsv.1 = x.clamp(0., 1.);
        self.hsv.2 = 1. - y.clamp(0., 1.);
        self.moved(window, cx);
    }

    /// The pointer at `pos` over the hue strip.
    fn pick_hue(&mut self, pos: Point<Pixels>, window: &mut Window, cx: &mut Context<Self>) {
        let b = self.strip.get();
        let x = f32::from(pos.x - b.origin.x) / f32::from(b.size.width).max(1.);
        self.hsv.0 = x.clamp(0., 1.) * 359.9;
        self.moved(window, cx);
    }

    fn moved(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.sync_hex(window, cx);
        cx.emit(PickerEvent::Change(self.colour()));
        cx.notify();
    }

    fn end_drag(&mut self, cx: &mut Context<Self>) {
        if std::mem::take(&mut self.dragging) {
            cx.emit(PickerEvent::Commit(self.colour()));
        }
    }
}

/// A canvas filling its parent that stores the parent's bounds in `cell`.
fn bounds_probe(cell: Rc<Cell<Bounds<Pixels>>>) -> impl IntoElement {
    canvas(move |b, _, _| cell.set(b), |_, _, _, _| {}).absolute().size_full()
}

fn hue_colour(h: f32) -> Hsla {
    let (r, g, b) = from_hsv(h, 1., 1.);
    theme::rgba_hsla(r, g, b, 1.)
}

impl Render for ColourPicker {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let owner = cx.entity_id();
        let (h, s, v) = self.hsv;
        let (r, g, b) = self.colour();
        let knob = |size: f32| {
            div().absolute().size(px(size)).rounded_full().border_2().border_color(white()).shadow(vec![BoxShadow {
                color: theme::black_a(0.45),
                offset: point(px(0.), px(1.)),
                blur_radius: px(3.),
                spread_radius: px(0.),
                inset: false,
            }])
        };

        let square = div()
            .id("cp-square")
            .relative()
            .w_full()
            .h(px(SQUARE_H))
            .rounded(px(8.))
            .cursor_crosshair()
            .bg(hue_colour(h))
            .child(div().absolute().inset_0().rounded(px(8.)).bg(linear_gradient(
                90.,
                linear_color_stop(theme::white_a(1.), 0.),
                linear_color_stop(theme::white_a(0.), 1.),
            )))
            .child(div().absolute().inset_0().rounded(px(8.)).bg(linear_gradient(
                180.,
                linear_color_stop(theme::black_a(0.), 0.),
                linear_color_stop(theme::black_a(1.), 1.),
            )))
            .child(bounds_probe(self.square.clone()))
            .child(knob(14.).left(relative(s)).top(relative(1. - v)).ml(px(-7.)).mt(px(-7.)).bg(theme::rgba_hsla(r, g, b, 1.)))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, ev: &MouseDownEvent, window, cx| {
                    this.dragging = true;
                    this.pick_square(ev.position, window, cx);
                }),
            )
            .on_drag(SquareDrag { owner }, |_, _, _, cx| cx.new(|_| NoDragView))
            .on_drag_move::<SquareDrag>(cx.listener(move |this, ev: &DragMoveEvent<SquareDrag>, window, cx| {
                if ev.drag(cx).owner == owner {
                    this.pick_square(ev.event.position, window, cx);
                }
            }));

        let mut bands = div().absolute().inset_0().flex().rounded_full().overflow_hidden();
        for w in HUES.windows(2) {
            bands = bands.child(div().flex_1().h_full().bg(linear_gradient(
                90.,
                linear_color_stop(hue_colour(w[0]), 0.),
                linear_color_stop(hue_colour(w[1]), 1.),
            )));
        }
        let strip = div()
            .id("cp-hue")
            .relative()
            .w_full()
            .h(px(STRIP_H))
            .cursor_pointer()
            .child(bands)
            .child(bounds_probe(self.strip.clone()))
            .child(knob(STRIP_H + 4.).left(relative(h / 360.)).top(px(-2.)).ml(px(-(STRIP_H + 4.) / 2.)).bg(hue_colour(h)))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, ev: &MouseDownEvent, window, cx| {
                    this.dragging = true;
                    this.pick_hue(ev.position, window, cx);
                }),
            )
            .on_drag(HueDrag { owner }, |_, _, _, cx| cx.new(|_| NoDragView))
            .on_drag_move::<HueDrag>(cx.listener(move |this, ev: &DragMoveEvent<HueDrag>, window, cx| {
                if ev.drag(cx).owner == owner {
                    this.pick_hue(ev.event.position, window, cx);
                }
            }));

        div()
            .flex()
            .flex_col()
            .gap(px(12.))
            .w_full()
            .on_mouse_up(MouseButton::Left, cx.listener(|this, _, _, cx| this.end_drag(cx)))
            .on_mouse_up_out(MouseButton::Left, cx.listener(|this, _, _, cx| this.end_drag(cx)))
            .child(square)
            .child(strip)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .child(
                        div()
                            .flex_none()
                            .size(px(32.))
                            .rounded(px(9.))
                            .bg(theme::rgba_hsla(r, g, b, 1.))
                            .border_1()
                            .border_color(theme::ink_a(0.18)),
                    )
                    .child(div().flex_1().child(input_box(&self.hex, false))),
            )
    }
}
