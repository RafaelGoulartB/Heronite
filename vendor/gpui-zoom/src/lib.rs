//! An interface scale ("zoom") the app picks for every GPUI window, on top of
//! the scale the system gives the screen (a 4K TV at 300%, a laptop at 125%).
//!
//! The patched platform crates (`vendor/gpui-pre-linux`, `vendor/gpui-pre-windows`)
//! open every window through [`open`] and list the displays through
//! [`displays`]. GPUI then sees a screen `zoom` times denser and `zoom` times
//! smaller: it lays the interface out in fewer (or more) logical pixels and
//! still draws it at the window's real resolution, so the text stays sharp at
//! any zoom, like a web browser's. The window's size on screen does not change
//! when the zoom does.
//!
//! Every coordinate crossing this layer is converted: into GPUI,
//! `GPUI px = system px / zoom`; out of it, `system px = GPUI px × zoom`. The
//! text-input candidate box the backends ask the focused field for on their
//! own is not: they multiply it by [`factor`]. Mobile insets (always zero on
//! desktop) are not converted either.
//!
//! Pop-ups and layer-shell surfaces (the in-game overlay) are not zoomed: they
//! size themselves to the screen.

use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use futures::channel::oneshot;
use gpui::{
    px, A11yCallbacks, AnyWindowHandle, Bounds, Capslock, Decorations, DispatchEventResult, DisplayId, ExternalDragPayload, FileDropEvent,
    GpuSpecs, Modifiers, Pixels, PlatformAtlas, PlatformDisplay, PlatformInput, PlatformInputHandler, PlatformWindow, Point, PromptButton,
    PromptLevel, RequestFrameOptions, ResizeEdge, Scene, ScrollDelta, Size, SystemWindowTab, TextInputConfiguration, TextInputStateChange,
    WindowAppearance, WindowBackgroundAppearance, WindowBounds, WindowControlArea, WindowControls, WindowDecorations, WindowId,
    WindowInsets, WindowKind, WindowParams, WindowVisibility,
};
use raw_window_handle as rwh;

/// The zooms [`set_zoom`] accepts (1.0 = 100%).
pub const RANGE: (f32, f32) = (0.5, 3.0);

/// `f32` bits; 1.0 until the app sets it.
static ZOOM: AtomicU32 = AtomicU32::new(0x3f80_0000);

thread_local! {
    /// The windows open through a [`ZoomedWindow`] (windows live on the main thread).
    static ZOOMED: RefCell<HashSet<WindowId>> = RefCell::new(HashSet::new());
}

/// What a backend multiplies GPUI px it reads by itself with (the text
/// input's candidate box): the zoom for a zoomed window, 1 for the others.
pub fn factor(handle: AnyWindowHandle) -> f32 {
    if ZOOMED.with(|z| z.borrow().contains(&handle.window_id())) {
        zoom()
    } else {
        1.
    }
}

/// The interface scale in use (1.0 = 100%).
pub fn zoom() -> f32 {
    f32::from_bits(ZOOM.load(Ordering::Relaxed))
}

/// Sets the interface scale, clamped to [`RANGE`] (anything not finite is
/// 100%). Returns whether it changed. Windows already open take it at their
/// next `Window::bounds_changed`; their minimum size stays the one they opened
/// with.
pub fn set_zoom(zoom: f32) -> bool {
    let zoom = if zoom.is_finite() { zoom.clamp(RANGE.0, RANGE.1) } else { 1. };
    ZOOM.swap(zoom.to_bits(), Ordering::Relaxed) != zoom.to_bits()
}

fn mul(v: Pixels, f: f32) -> Pixels {
    px(f32::from(v) * f)
}

fn point_by(p: Point<Pixels>, f: f32) -> Point<Pixels> {
    Point { x: mul(p.x, f), y: mul(p.y, f) }
}

fn size_by(s: Size<Pixels>, f: f32) -> Size<Pixels> {
    Size { width: mul(s.width, f), height: mul(s.height, f) }
}

fn bounds_by(b: Bounds<Pixels>, f: f32) -> Bounds<Pixels> {
    Bounds { origin: point_by(b.origin, f), size: size_by(b.size, f) }
}

fn window_bounds_by(b: WindowBounds, f: f32) -> WindowBounds {
    match b {
        WindowBounds::Windowed(b) => WindowBounds::Windowed(bounds_by(b, f)),
        WindowBounds::Maximized(b) => WindowBounds::Maximized(bounds_by(b, f)),
        WindowBounds::Fullscreen(b) => WindowBounds::Fullscreen(bounds_by(b, f)),
    }
}

/// System px to GPUI px.
fn down(z: f32) -> f32 {
    1. / z
}

/// An input event's positions (and pixel scroll) from system px to GPUI px.
fn event_to_gpui(event: PlatformInput, z: f32) -> PlatformInput {
    let f = down(z);
    match event {
        PlatformInput::MouseDown(mut e) => {
            e.position = point_by(e.position, f);
            PlatformInput::MouseDown(e)
        }
        PlatformInput::MouseUp(mut e) => {
            e.position = point_by(e.position, f);
            PlatformInput::MouseUp(e)
        }
        PlatformInput::MousePressure(mut e) => {
            e.position = point_by(e.position, f);
            PlatformInput::MousePressure(e)
        }
        PlatformInput::MouseMove(mut e) => {
            e.position = point_by(e.position, f);
            PlatformInput::MouseMove(e)
        }
        PlatformInput::MouseExited(mut e) => {
            e.position = point_by(e.position, f);
            PlatformInput::MouseExited(e)
        }
        PlatformInput::ScrollWheel(mut e) => {
            e.position = point_by(e.position, f);
            // A touchpad scrolls by the distance the fingers moved, so the
            // page keeps following them at any zoom. Lines stay lines.
            if let ScrollDelta::Pixels(d) = e.delta {
                e.delta = ScrollDelta::Pixels(point_by(d, f));
            }
            PlatformInput::ScrollWheel(e)
        }
        PlatformInput::Pinch(mut e) => {
            e.position = point_by(e.position, f);
            PlatformInput::Pinch(e)
        }
        PlatformInput::LongPress(mut e) => {
            e.start_position = point_by(e.start_position, f);
            e.position = point_by(e.position, f);
            PlatformInput::LongPress(e)
        }
        PlatformInput::TouchDrag(mut e) => {
            e.start_position = point_by(e.start_position, f);
            e.position = point_by(e.position, f);
            PlatformInput::TouchDrag(e)
        }
        PlatformInput::Touch(mut e) => {
            e.position = point_by(e.position, f);
            e.predicted_position = e.predicted_position.map(|p| point_by(p, f));
            PlatformInput::Touch(e)
        }
        PlatformInput::FileDrop(e) => PlatformInput::FileDrop(match e {
            FileDropEvent::Entered { position, paths } => FileDropEvent::Entered { position: point_by(position, f), paths },
            FileDropEvent::Pending { position } => FileDropEvent::Pending { position: point_by(position, f) },
            FileDropEvent::Submit { position } => FileDropEvent::Submit { position: point_by(position, f) },
            e @ (FileDropEvent::Exited | FileDropEvent::Ended) => e,
        }),
        e @ (PlatformInput::KeyDown(_) | PlatformInput::KeyUp(_) | PlatformInput::ModifiersChanged(_)) => e,
    }
}

/// Whether windows of `kind` are zoomed: not pop-ups or layer-shell surfaces.
fn zoomed_kind(kind: &WindowKind) -> bool {
    match kind {
        WindowKind::PopUp => false,
        #[cfg(all(target_os = "linux", feature = "wayland"))]
        WindowKind::LayerShell(_) => false,
        _ => true,
    }
}

/// Opens window `handle` at the current zoom: `params` are GPUI px, `open` is
/// the backend's own `open_window`.
pub fn open(
    handle: AnyWindowHandle,
    mut params: WindowParams,
    open: impl FnOnce(WindowParams) -> gpui::Result<Box<dyn PlatformWindow>>,
) -> gpui::Result<Box<dyn PlatformWindow>> {
    if !zoomed_kind(&params.kind) {
        return open(params);
    }
    let z = zoom();
    params.bounds = bounds_by(params.bounds, z);
    params.window_min_size = params.window_min_size.map(|s| size_by(s, z));
    if let WindowKind::AnchoredPopup(popup) = &mut params.kind {
        popup.anchor_rect = bounds_by(popup.anchor_rect, z);
        popup.offset = point_by(popup.offset, z);
    }
    let window = open(params)?;
    let id = handle.window_id();
    ZOOMED.with(|z| z.borrow_mut().insert(id));
    Ok(Box::new(ZoomedWindow(window, id)))
}

/// The displays in GPUI px.
pub fn displays(displays: Vec<Rc<dyn PlatformDisplay>>) -> Vec<Rc<dyn PlatformDisplay>> {
    displays.into_iter().map(display).collect()
}

/// A display in GPUI px.
pub fn display(display: Rc<dyn PlatformDisplay>) -> Rc<dyn PlatformDisplay> {
    Rc::new(ZoomedDisplay(display))
}

#[derive(Debug)]
struct ZoomedDisplay(Rc<dyn PlatformDisplay>);

impl PlatformDisplay for ZoomedDisplay {
    fn id(&self) -> DisplayId {
        self.0.id()
    }

    fn uuid(&self) -> gpui::Result<uuid::Uuid> {
        self.0.uuid()
    }

    fn bounds(&self) -> Bounds<Pixels> {
        bounds_by(self.0.bounds(), down(zoom()))
    }

    fn visible_bounds(&self) -> Bounds<Pixels> {
        bounds_by(self.0.visible_bounds(), down(zoom()))
    }
}

/// A backend's window seen by GPUI at the current zoom.
struct ZoomedWindow(Box<dyn PlatformWindow>, WindowId);

impl Drop for ZoomedWindow {
    fn drop(&mut self) {
        ZOOMED.with(|z| z.borrow_mut().remove(&self.1));
    }
}

impl rwh::HasWindowHandle for ZoomedWindow {
    fn window_handle(&self) -> Result<rwh::WindowHandle<'_>, rwh::HandleError> {
        self.0.window_handle()
    }
}

impl rwh::HasDisplayHandle for ZoomedWindow {
    fn display_handle(&self) -> Result<rwh::DisplayHandle<'_>, rwh::HandleError> {
        self.0.display_handle()
    }
}

impl PlatformWindow for ZoomedWindow {
    fn bounds(&self) -> Bounds<Pixels> {
        bounds_by(self.0.bounds(), down(zoom()))
    }

    fn is_maximized(&self) -> bool {
        self.0.is_maximized()
    }

    fn window_bounds(&self) -> WindowBounds {
        window_bounds_by(self.0.window_bounds(), down(zoom()))
    }

    fn content_size(&self) -> Size<Pixels> {
        size_by(self.0.content_size(), down(zoom()))
    }

    fn visual_viewport_bounds(&self) -> Bounds<Pixels> {
        bounds_by(self.0.visual_viewport_bounds(), down(zoom()))
    }

    fn on_visual_viewport_changed(&self, callback: Box<dyn FnMut()>) {
        self.0.on_visual_viewport_changed(callback)
    }

    fn prepare_frame(&self) -> bool {
        self.0.prepare_frame()
    }

    fn resize(&mut self, size: Size<Pixels>) {
        self.0.resize(size_by(size, zoom()))
    }

    fn scale_factor(&self) -> f32 {
        self.0.scale_factor() * zoom()
    }

    fn appearance(&self) -> WindowAppearance {
        self.0.appearance()
    }

    fn display(&self) -> Option<Rc<dyn PlatformDisplay>> {
        self.0.display().map(display)
    }

    fn mouse_position(&self) -> Point<Pixels> {
        point_by(self.0.mouse_position(), down(zoom()))
    }

    fn modifiers(&self) -> Modifiers {
        self.0.modifiers()
    }

    fn capslock(&self) -> Capslock {
        self.0.capslock()
    }

    fn set_input_handler(&mut self, input_handler: PlatformInputHandler) {
        self.0.set_input_handler(input_handler)
    }

    fn take_input_handler(&mut self) -> Option<PlatformInputHandler> {
        self.0.take_input_handler()
    }

    fn set_text_input_configuration(&mut self, configuration: TextInputConfiguration) {
        self.0.set_text_input_configuration(configuration)
    }

    fn prompt(&self, level: PromptLevel, msg: &str, detail: Option<&str>, answers: &[PromptButton]) -> Option<oneshot::Receiver<usize>> {
        self.0.prompt(level, msg, detail, answers)
    }

    fn activate(&self) {
        self.0.activate()
    }

    fn request_attention(&self) {
        self.0.request_attention()
    }

    fn is_active(&self) -> bool {
        self.0.is_active()
    }

    fn visibility(&self) -> WindowVisibility {
        self.0.visibility()
    }

    fn is_hovered(&self) -> bool {
        self.0.is_hovered()
    }

    fn background_appearance(&self) -> WindowBackgroundAppearance {
        self.0.background_appearance()
    }

    fn set_title(&mut self, title: &str) {
        self.0.set_title(title)
    }

    fn set_background_appearance(&self, background_appearance: WindowBackgroundAppearance) {
        self.0.set_background_appearance(background_appearance)
    }

    fn minimize(&self) {
        self.0.minimize()
    }

    fn zoom(&self) {
        self.0.zoom()
    }

    fn toggle_fullscreen(&self) {
        self.0.toggle_fullscreen()
    }

    fn is_fullscreen(&self) -> bool {
        self.0.is_fullscreen()
    }

    fn frame_waker(&self) -> Option<Rc<dyn Fn()>> {
        self.0.frame_waker()
    }

    fn on_request_frame(&self, callback: Box<dyn FnMut(RequestFrameOptions)>) {
        self.0.on_request_frame(callback)
    }

    fn on_input(&self, mut callback: Box<dyn FnMut(PlatformInput) -> DispatchEventResult>) {
        self.0.on_input(Box::new(move |event| callback(event_to_gpui(event, zoom()))))
    }

    fn on_active_status_change(&self, callback: Box<dyn FnMut(bool)>) {
        self.0.on_active_status_change(callback)
    }

    fn on_visibility_change(&self, callback: Box<dyn FnMut(WindowVisibility)>) {
        self.0.on_visibility_change(callback)
    }

    fn on_hover_status_change(&self, callback: Box<dyn FnMut(bool)>) {
        self.0.on_hover_status_change(callback)
    }

    fn on_resize(&self, mut callback: Box<dyn FnMut(Size<Pixels>, f32)>) {
        self.0.on_resize(Box::new(move |size, scale| {
            let z = zoom();
            callback(size_by(size, down(z)), scale * z)
        }))
    }

    fn on_moved(&self, callback: Box<dyn FnMut()>) {
        self.0.on_moved(callback)
    }

    fn on_should_close(&self, callback: Box<dyn FnMut() -> bool>) {
        self.0.on_should_close(callback)
    }

    fn on_hit_test_window_control(&self, callback: Box<dyn FnMut() -> Option<WindowControlArea>>) {
        self.0.on_hit_test_window_control(callback)
    }

    fn on_close(&self, callback: Box<dyn FnOnce()>) {
        self.0.on_close(callback)
    }

    fn on_appearance_changed(&self, callback: Box<dyn FnMut()>) {
        self.0.on_appearance_changed(callback)
    }

    fn on_button_layout_changed(&self, callback: Box<dyn FnMut()>) {
        self.0.on_button_layout_changed(callback)
    }

    fn draw(&self, scene: &Scene) {
        self.0.draw(scene)
    }

    fn schedule_frame(&self) {
        self.0.schedule_frame()
    }

    fn sprite_atlas(&self) -> Arc<dyn PlatformAtlas> {
        self.0.sprite_atlas()
    }

    fn is_subpixel_rendering_supported(&self) -> bool {
        self.0.is_subpixel_rendering_supported()
    }

    fn get_title(&self) -> String {
        self.0.get_title()
    }

    fn tabbed_windows(&self) -> Option<Vec<SystemWindowTab>> {
        self.0.tabbed_windows()
    }

    fn tab_bar_visible(&self) -> bool {
        self.0.tab_bar_visible()
    }

    fn set_edited(&mut self, edited: bool) {
        self.0.set_edited(edited)
    }

    fn set_document_path(&self, path: Option<&std::path::Path>) {
        self.0.set_document_path(path)
    }

    fn toggle_simple_fullscreen(&self) {
        self.0.toggle_simple_fullscreen()
    }

    fn is_simple_fullscreen(&self) -> bool {
        self.0.is_simple_fullscreen()
    }

    fn show_character_palette(&self) {
        self.0.show_character_palette()
    }

    fn titlebar_double_click(&self, is_resizable: bool, is_minimizable: bool) {
        self.0.titlebar_double_click(is_resizable, is_minimizable)
    }

    fn native_window_state(&self) -> Option<Vec<u8>> {
        self.0.native_window_state()
    }

    fn restore_native_window_state(&self, state: &[u8]) {
        self.0.restore_native_window_state(state)
    }

    #[cfg(target_os = "windows")]
    fn get_raw_handle(&self) -> windows::Win32::Foundation::HWND {
        self.0.get_raw_handle()
    }

    fn inner_window_bounds(&self) -> WindowBounds {
        window_bounds_by(self.0.inner_window_bounds(), down(zoom()))
    }

    fn request_decorations(&self, decorations: WindowDecorations) {
        self.0.request_decorations(decorations)
    }

    fn show_window_menu(&self, position: Point<Pixels>) {
        self.0.show_window_menu(point_by(position, zoom()))
    }

    fn start_window_move(&self) {
        self.0.start_window_move()
    }

    fn can_start_external_drag(&self) -> bool {
        self.0.can_start_external_drag()
    }

    fn start_external_drag(&self, payload: &ExternalDragPayload) -> bool {
        self.0.start_external_drag(payload)
    }

    fn start_window_resize(&self, edge: ResizeEdge) {
        self.0.start_window_resize(edge)
    }

    fn set_exclusive_zone(&self, zone: Pixels) {
        self.0.set_exclusive_zone(mul(zone, zoom()))
    }

    fn set_input_region(&self, region: Option<&[Bounds<Pixels>]>) {
        let z = zoom();
        let region = region.map(|r| r.iter().map(|b| bounds_by(*b, z)).collect::<Vec<_>>());
        self.0.set_input_region(region.as_deref())
    }

    fn window_decorations(&self) -> Decorations {
        self.0.window_decorations()
    }

    fn set_app_id(&mut self, app_id: &str) {
        self.0.set_app_id(app_id)
    }

    fn map_window(&mut self) -> gpui::Result<()> {
        self.0.map_window()
    }

    fn window_controls(&self) -> WindowControls {
        self.0.window_controls()
    }

    fn set_client_inset(&self, inset: Pixels) {
        self.0.set_client_inset(mul(inset, zoom()))
    }

    fn gpu_specs(&self) -> Option<GpuSpecs> {
        self.0.gpu_specs()
    }

    fn update_ime_position(&self, bounds: Bounds<Pixels>) {
        self.0.update_ime_position(bounds_by(bounds, zoom()))
    }

    fn insets(&self) -> WindowInsets {
        self.0.insets()
    }

    fn on_insets_changed(&self, callback: Box<dyn FnMut(WindowInsets)>) {
        self.0.on_insets_changed(callback)
    }

    fn set_back_handler(&self, callback: Box<dyn FnMut()>) {
        self.0.set_back_handler(callback)
    }

    fn set_back_enabled(&self, enabled: bool) {
        self.0.set_back_enabled(enabled)
    }

    fn show_soft_keyboard(&self) {
        self.0.show_soft_keyboard()
    }

    fn hide_soft_keyboard(&self) {
        self.0.hide_soft_keyboard()
    }

    fn text_input_state_changed(&self, change: TextInputStateChange) {
        self.0.text_input_state_changed(change)
    }

    fn play_system_bell(&self) {
        self.0.play_system_bell()
    }

    fn a11y_init(&self, callbacks: A11yCallbacks) {
        self.0.a11y_init(callbacks)
    }

    fn a11y_tree_update(&self, tree_update: gpui::accesskit::TreeUpdate) {
        self.0.a11y_tree_update(tree_update)
    }

    fn a11y_update_window_bounds(&self) {
        self.0.a11y_update_window_bounds()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{point, size, MouseButton, MouseDownEvent};

    #[test]
    fn set_zoom_clamps_and_reports_changes() {
        assert!(set_zoom(0.8));
        assert!(!set_zoom(0.8));
        assert_eq!(zoom(), 0.8);
        set_zoom(10.);
        assert_eq!(zoom(), RANGE.1);
        set_zoom(f32::NAN);
        assert_eq!(zoom(), 1.);
    }

    #[test]
    fn events_reach_gpui_in_zoomed_pixels() {
        let down = PlatformInput::MouseDown(MouseDownEvent {
            button: MouseButton::Left,
            position: point(px(200.), px(100.)),
            modifiers: Modifiers::default(),
            click_count: 1,
            first_mouse: false,
        });
        let PlatformInput::MouseDown(e) = event_to_gpui(down, 2.) else { panic!("same kind of event") };
        assert_eq!(e.position, point(px(100.), px(50.)));
    }

    #[test]
    fn window_bounds_keep_their_kind() {
        let b = Bounds { origin: point(px(10.), px(20.)), size: size(px(800.), px(600.)) };
        let WindowBounds::Maximized(m) = window_bounds_by(WindowBounds::Maximized(b), 0.5) else { panic!("still maximized") };
        assert_eq!(m.size, size(px(400.), px(300.)));
        assert_eq!(m.origin, point(px(5.), px(10.)));
    }
}
