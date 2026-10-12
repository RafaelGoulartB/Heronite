//! Fullscreen screenshot viewer (zoom, pan, fit, delete) and the keyboard
//! handling of the gallery/viewer. Port of the viewer half of
//! `CollectionScreenshots.tsx`.

use std::sync::atomic::Ordering;
use std::time::Duration;

use gpui_kit::prelude::*;
use gpui_kit::*;

use super::screenshots::{icon_button, load_full, Drag, Msg};
use super::CollectionView;
use crate::icons::Icon;
use crate::theme;
use crate::ui::controls::{button, ButtonKind};
use heronite_i18n::tr;

const MIN_ZOOM: f32 = 1.;
const MAX_ZOOM: f32 = 5.;
const ZOOM_STEP: f32 = 1.12;
const POINTER_STOP: Duration = Duration::from_millis(500);

impl CollectionView {
    /// Keeps the viewer's full-resolution image loaded (decoded at the size
    /// it is shown, bigger once the user zooms).
    pub(super) fn sync_viewer(&mut self, window: &mut Window, _cx: &mut Context<Self>) {
        let Some(i) = self.shots.view else {
            if self.shots.full.is_some() {
                self.shots.discard_full();
            }
            return;
        };
        let Some(item) = self.shots.items.get(i) else { return };
        let path = item.path.clone();
        if self.shots.full.as_ref().is_some_and(|f| f.path != path) {
            self.shots.discard_full();
        }
        if self.shots.full_failed.as_ref() == Some(&path) {
            return;
        }
        let scale = window.scale_factor();
        let vp = window.viewport_size();
        let (vw, vh) = (f32::from(vp.width), f32::from(vp.height));
        let (bw, bh) = if self.shots.fit { (vw, vh) } else { ((0.92 * vw).min(1400.), 0.82 * vh) };
        let m = if self.shots.boost { 3. } else { 1. };
        let bx = ((bw * scale * m) as u32, (bh * scale * m) as u32);
        if let Some(f) = &self.shots.full {
            let s = (bx.0 as f32 / f.src.0 as f32).min(bx.1 as f32 / f.src.1 as f32).min(1.);
            if f.size.0 as f32 >= f.src.0 as f32 * s * 0.9 {
                return;
            }
        }
        if self.shots.full_pending.as_ref() == Some(&(path.clone(), bx)) {
            return;
        }
        let Some(tx) = self.shots.tx_clone() else { return };
        let req = self.shots.full_req.fetch_add(1, Ordering::SeqCst) + 1;
        let latest = self.shots.full_req.clone();
        self.shots.full_pending = Some((path.clone(), bx));
        let _ = std::thread::Builder::new().name("screenshot-full".into()).spawn(move || {
            let res = if latest.load(Ordering::SeqCst) != req { Err("superseded".to_string()) } else { load_full(&path, bx) };
            let _ = tx.unbounded_send(Msg::Full { req, path, res });
        });
    }

    fn shots_step(&mut self, delta: isize) {
        let n = self.shots.items.len() as isize;
        let Some(i) = self.shots.view else { return };
        if n == 0 {
            return;
        }
        self.shots.view = Some((i as isize + delta).rem_euclid(n) as usize);
        self.shots.reset_transform();
        self.shots.discard_full();
    }

    fn shots_toggle_fit(&mut self) {
        self.shots.reset_transform();
        self.shots.controls_task = None;
        self.shots.fit = !self.shots.fit;
        // Entering fit hides the controls after the pointer stops.
        self.shots.controls_visible = !self.shots.fit;
    }

    fn shots_close_view(&mut self) {
        self.shots.fit = false;
        self.shots.view = None;
        self.shots.confirm = None;
        self.shots.reset_transform();
        self.shots.discard_full();
    }

    fn shots_reveal_controls(&mut self, cx: &mut Context<Self>) {
        let was_hidden = !self.shots.controls_visible;
        self.shots.controls_visible = true;
        self.shots.controls_task = None;
        if self.shots.fit && self.shots.view.is_some() {
            self.shots.controls_task = Some(cx.spawn(async move |this, cx| {
                cx.background_executor().timer(POINTER_STOP).await;
                this.update(cx, |this, cx| {
                    this.shots.controls_visible = false;
                    cx.notify();
                })
                .ok();
            }));
        }
        if was_hidden {
            cx.notify();
        }
    }

    fn shots_wheel(&mut self, ev: &ScrollWheelEvent, window: &Window, cx: &mut Context<Self>) {
        let dy = match ev.delta {
            ScrollDelta::Pixels(p) => f32::from(p.y),
            ScrollDelta::Lines(l) => l.y,
        };
        if dy == 0. {
            return;
        }
        let current = self.shots.zoom;
        let factor = if dy > 0. { ZOOM_STEP } else { 1. / ZOOM_STEP };
        let next = (current * factor).clamp(MIN_ZOOM, MAX_ZOOM);
        if next == current {
            return;
        }
        let vp = window.viewport_size();
        let ox = f32::from(ev.position.x) - f32::from(vp.width) / 2.;
        let oy = f32::from(ev.position.y) - f32::from(vp.height) / 2.;
        let ratio = next / current;
        self.shots.pan = if next == MIN_ZOOM {
            (0., 0.)
        } else {
            let (px_, py_) = self.shots.pan;
            (ox - (ox - px_) * ratio, oy - (oy - py_) * ratio)
        };
        self.shots.zoom = next;
        if next > MIN_ZOOM {
            self.shots.boost = true;
        }
        cx.notify();
    }

    pub(super) fn shots_delete(&mut self, cx: &mut Context<Self>) {
        let Some(i) = self.shots.confirm.take() else { return };
        let Some(item) = self.shots.items.get(i).cloned() else { return };
        self.shots.deleting = true;
        let root = std::path::PathBuf::from(&self.shots.folder);
        let cache = self.shots.cache.clone();
        cx.notify();
        cx.spawn(async move |this, cx| {
            let path = item.path.clone();
            let res = cx
                .background_spawn(async move {
                    let r = heronite_media::trash_screenshot_in(Some(&root), &path);
                    if r.is_ok() {
                        if let Some(c) = cache {
                            c.maintain();
                        }
                    }
                    r.map_err(|e| e.to_string())
                })
                .await;
            this.update(cx, |this, cx| {
                this.shots.deleting = false;
                match res {
                    Ok(_) => {
                        this.shots.items.retain(|s| s.path != item.path);
                        this.shots.reset_transform();
                        this.shots.fit = false;
                        this.shots.discard_full();
                        let n = this.shots.items.len();
                        if n == 0 {
                            this.shots.view = None;
                            this.shots.gallery = false;
                        } else {
                            this.shots.view = Some(i.min(n - 1));
                        }
                        this.shots.error = None;
                    }
                    Err(err) => {
                        this.shots.error = Some(tr!("Could not delete the screenshot: {err}", err).into());
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Esc / arrows while the gallery or viewer is open. Returns whether the
    /// key was consumed.
    pub(super) fn shots_key(&mut self, ev: &KeyDownEvent, cx: &mut Context<Self>) -> bool {
        if !self.shots.is_open() {
            return false;
        }
        let key = ev.keystroke.key.as_str();
        if self.shots.confirm.is_some() {
            match key {
                "escape" => self.shots.confirm = None,
                "enter" => self.shots_delete(cx),
                _ => return false,
            }
            return true;
        }
        if self.shots.deleting {
            return true;
        }
        let viewing = self.shots.view.is_some();
        match key {
            "escape" => {
                if viewing && self.shots.zoom > MIN_ZOOM {
                    self.shots.reset_transform();
                } else if viewing && self.shots.fit {
                    self.shots.fit = false;
                    self.shots.controls_visible = true;
                } else if viewing && self.shots.gallery {
                    self.shots.view = None;
                    self.shots.discard_full();
                } else {
                    self.shots_close_gallery();
                }
            }
            "right" if viewing => self.shots_step(1),
            "left" if viewing => self.shots_step(-1),
            "up" if viewing => self.shots_toggle_fit(),
            "down" if viewing => self.shots_close_view(),
            _ => return false,
        }
        true
    }

    pub(super) fn render_viewer(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        let i = self.shots.view?;
        let item = self.shots.items.get(i)?.clone();
        let n = self.shots.items.len();
        let vp = window.viewport_size();
        let (vw, vh) = (f32::from(vp.width), f32::from(vp.height));
        let scale = window.scale_factor();
        let (fit, zoom) = (self.shots.fit, self.shots.zoom);
        let controls = !fit || self.shots.controls_visible;

        let image = self.viewer_image(&item.path, vw, vh, scale, cx);
        let tools = self.viewer_tools(i, cx);

        let mut caption = format!("{}/{}", i + 1, n);
        if zoom > MIN_ZOOM {
            caption.push_str(&format!(" · {}%", (zoom * 100.).round()));
        }

        let mut overlay = div()
            .id("shot-viewer")
            .absolute()
            .inset_0()
            .occlude()
            .overflow_hidden()
            .bg(theme::black_a(0.86))
            .font_family(theme::FONT_BODY)
            .on_mouse_move(cx.listener(|this, ev: &MouseMoveEvent, _, cx| {
                this.shots_reveal_controls(cx);
                if let (Some(d), Some(MouseButton::Left)) = (this.shots.drag, ev.pressed_button) {
                    this.shots.pan = (d.pan.0 + f32::from(ev.position.x - d.mouse.x), d.pan.1 + f32::from(ev.position.y - d.mouse.y));
                    cx.notify();
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, _| {
                    this.shots.drag = None;
                }),
            )
            .on_scroll_wheel(cx.listener(|this, ev: &ScrollWheelEvent, window, cx| this.shots_wheel(ev, window, cx)))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    cx.stop_propagation();
                    if this.shots.fit && !this.shots.controls_visible {
                        this.shots_reveal_controls(cx);
                    } else {
                        this.shots_close_view();
                    }
                    cx.notify();
                }),
            )
            .children(image);
        if controls {
            overlay = overlay.child(tools);
            if n > 1 {
                overlay = overlay.child(Self::viewer_nav("shot-prev", Icon::ChevronLeft, true, -1, cx)).child(Self::viewer_nav(
                    "shot-next",
                    Icon::ChevronRight,
                    false,
                    1,
                    cx,
                ));
            }
            overlay = overlay.child(
                div()
                    .absolute()
                    .bottom(px(17.6))
                    .left_0()
                    .right_0()
                    .flex()
                    .justify_center()
                    .text_size(px(13.6))
                    .text_color(theme::white_a(0.78))
                    .child(caption),
            );
        }
        if let Some(err) = self.shots.error.clone() {
            overlay = overlay.child(Self::viewer_error(err));
        }
        if let Some(ci) = self.shots.confirm {
            let name = self.shots.items.get(ci).map(|s| s.name.clone()).unwrap_or_default();
            overlay = overlay.child(Self::delete_confirm(&name, cx));
        }
        Some(deferred(overlay).with_priority(60).into_any_element())
    }

    /// The full-resolution image at its zoom and pan, once decoded.
    fn viewer_image(&self, path: &std::path::Path, vw: f32, vh: f32, scale: f32, cx: &mut Context<Self>) -> Option<Stateful<Div>> {
        let (fit, zoom, pan) = (self.shots.fit, self.shots.zoom, self.shots.pan);
        self.shots.full.as_ref().filter(|f| f.path.as_path() == path).map(|f| {
            let (nw, nh) = (f.src.0 as f32 / scale, f.src.1 as f32 / scale);
            let (bw, bh) = if fit { (vw, vh) } else { ((0.92 * vw).min(1400.), 0.82 * vh) };
            let mut s = (bw / nw).min(bh / nh);
            if !fit {
                s = s.min(1.);
            }
            let (w, h) = (nw * s * zoom, nh * s * zoom);
            let (left, top) = (vw / 2. + pan.0 - w / 2., vh / 2. + pan.1 - h / 2.);
            div()
                .id("shot-full")
                .absolute()
                .left(px(left))
                .top(px(top))
                .w(px(w))
                .h(px(h))
                .overflow_hidden()
                .when(!fit, |d| {
                    d.rounded(px(8. * zoom)).shadow(vec![BoxShadow {
                        color: theme::black_a(0.45),
                        offset: point(px(0.), px(18.)),
                        blur_radius: px(50.),
                        spread_radius: px(0.),
                        inset: false,
                    }])
                })
                .cursor(if zoom > MIN_ZOOM { CursorStyle::OpenHand } else { CursorStyle::Arrow })
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, ev: &MouseDownEvent, _, cx| {
                        cx.stop_propagation();
                        this.shots_reveal_controls(cx);
                        if ev.click_count >= 2 {
                            this.shots.reset_transform();
                        } else if this.shots.zoom > MIN_ZOOM {
                            this.shots.drag = Some(Drag { mouse: ev.position, pan: this.shots.pan });
                        }
                        cx.notify();
                    }),
                )
                .child(img(ImageSource::Render(f.img.clone())).size_full().object_fit(ObjectFit::Fill))
        })
    }

    /// Delete, fit and close buttons (top right).
    fn viewer_tools(&self, i: usize, cx: &mut Context<Self>) -> Div {
        let fit = self.shots.fit;
        div()
            .absolute()
            .top(px(17.6))
            .right(px(19.2))
            .flex()
            .gap(px(8.))
            .child({
                let busy = self.shots.deleting;
                let mut b = div()
                    .id("shot-delete")
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_center()
                    .size(px(36.))
                    .rounded_full()
                    .bg(theme::rgb_a(16, 18, 22, 0.72))
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .child(crate::ui::icon(Icon::DeleteOutline, px(24.), rgb(0xf4f6f8)));
                if busy {
                    b = b.opacity(0.55);
                } else {
                    b = b.cursor_pointer().hover(|s| s.bg(theme::rgb_a(190, 45, 45, 0.9))).on_click(cx.listener(move |this, _, _, cx| {
                        this.shots.confirm = Some(i);
                        cx.notify();
                    }));
                }
                b
            })
            .child(icon_button("shot-fit", if fit { Icon::FullscreenExit } else { Icon::Fullscreen }, 36., 0.72, 0.92).on_click(
                cx.listener(|this, _, _, cx| {
                    this.shots_toggle_fit();
                    cx.notify();
                }),
            ))
            .child(icon_button("shot-view-close", Icon::Close, 36., 0.72, 0.92).on_click(cx.listener(|this, _, _, cx| {
                this.shots_close_view();
                cx.notify();
            })))
    }

    /// Previous / next arrow, vertically centred.
    fn viewer_nav(id: &'static str, ic: Icon, left: bool, delta: isize, cx: &mut Context<Self>) -> Stateful<Div> {
        let b =
            icon_button(id, ic, 46., 0.72, 0.92).absolute().top(relative(0.5)).mt(px(-23.)).on_click(cx.listener(move |this, _, _, cx| {
                this.shots_step(delta);
                cx.notify();
            }));
        if left {
            b.left(px(19.2))
        } else {
            b.right(px(19.2))
        }
    }

    fn viewer_error(err: SharedString) -> Div {
        div().absolute().top(px(24.)).left_0().right_0().flex().justify_center().child(
            div()
                .px(px(14.))
                .py(px(8.))
                .rounded(px(8.))
                .bg(theme::rgb_a(18, 22, 28, 0.94))
                .border_1()
                .border_color(theme::danger())
                .text_size(px(13.33))
                .text_color(theme::white_a(0.92))
                .child(err),
        )
    }

    /// "Move to the trash?" confirmation over the viewer.
    fn delete_confirm(name: &str, cx: &mut Context<Self>) -> Div {
        div()
            .absolute()
            .inset_0()
            .flex()
            .items_center()
            .justify_center()
            .bg(theme::black_a(0.35))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(
                div()
                    .id("shot-confirm")
                    .occlude()
                    .flex()
                    .flex_col()
                    .gap(px(20.))
                    .w(px(420.))
                    .p(px(24.))
                    .rounded(px(10.))
                    .bg(theme::background())
                    .text_color(rgb(0xcaf3fd))
                    .text_size(px(16.))
                    .child(tr!("Move “{name}” to the trash?", name))
                    .child(
                        div()
                            .flex()
                            .justify_end()
                            .gap(px(12.))
                            .child(button("shot-confirm-cancel", ButtonKind::Glass, tr!("Cancel")).on_click(cx.listener(
                                |this, _, _, cx| {
                                    this.shots.confirm = None;
                                    cx.notify();
                                },
                            )))
                            .child(button("shot-confirm-ok", ButtonKind::Primary, tr!("Move to trash")).on_click(cx.listener(
                                |this, _, _, cx| {
                                    this.shots_delete(cx);
                                },
                            ))),
                    ),
            )
    }
}
