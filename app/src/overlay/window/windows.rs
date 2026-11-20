use gpui_kit::*;
use heronite_data::overlay_prefs::Side;
use heronite_platform::overlay_window::{self, FocusReturn, Screen};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

#[allow(clippy::too_many_arguments)]
pub(super) fn open<V: Render>(
    cx: &mut App,
    screen: std::io::Result<Screen>,
    namespace: &str,
    size: Size<Pixels>,
    side: Side,
    bottom: bool,
    margin: f32,
    build: impl FnOnce(&mut Window, &mut App) -> Entity<V>,
) -> anyhow::Result<(WindowHandle<V>, Task<anyhow::Result<Option<FocusReturn>>>)> {
    let screen = screen?;
    // Notices never take the foreground nor the clicks.
    let notice = namespace.ends_with("notice");
    let handle = cx.open_window(super::options(namespace, size, side, bottom, px(margin), true, false), |window, cx| {
        window.set_window_title(if notice { "Heronite overlay notice" } else { "Heronite overlay" });
        build(window, cx)
    })?;
    let raw = handle.update(cx, |_, window, _| {
        let raw_handle = window.window_handle().map_err(|e| anyhow::anyhow!("could not get the overlay window handle: {e}"))?;
        let RawWindowHandle::Win32(raw) = raw_handle.as_raw() else {
            anyhow::bail!("the overlay has no Windows window handle");
        };
        Ok(raw.hwnd)
    });
    let hwnd = match raw.and_then(|r| r) {
        Ok(hwnd) => hwnd,
        Err(error) => {
            let _ = handle.update(cx, |_, window, _| window.remove_window());
            return Err(error);
        }
    };
    // Native placement sends synchronous size/DPI messages into GPUI. Its
    // foreground executor runs this after all App/Window borrows unwind.
    let shown = cx.foreground_executor().spawn(async move {
        Ok(overlay_window::show(hwnd, screen, (size.width.into(), size.height.into()), side == Side::Right, bottom, margin, notice)?)
    });
    Ok((handle, shown))
}
