# gpui-pre-windows 0.3.8, patched

The crates.io release of `gpui-pre-windows` 0.3.8 (Zed's `gpui_windows`), used
through `[patch.crates-io]` in the workspace `Cargo.toml`. Zed's `main` has the
same code, so these changes can go once a release fixes it. Changes, for a
window closed by the user (its close button, Alt+F4, the taskbar), which
Windows destroys before GPUI drops its `WindowsWindow`:

- `handle_destroy_msg` (`src/events.rs`) revokes drag-and-drop and marks the
  window destroyed. Upstream revokes in a task queued by `WindowsWindow`'s
  `Drop`, which runs after the handle is gone, so `RevokeDragDrop` failed
  (`DRAGDROP_E_INVALIDHWND`) and OLE kept the drop target and the window it
  holds: the whole window (its DirectX renderer and swap chain) leaked at
  every close, ~50 MB each when closed to the tray.
- `WindowsWindow`'s `Drop` (`src/window.rs`) neither hides nor destroys a
  window marked destroyed (upstream logged `ERROR_INVALID_WINDOW_HANDLE` from
  `DestroyWindow`, and could reach a reused handle). A window closed by the
  app (`remove_window`) is not marked yet: it is hidden and destroyed as
  before, and its `WM_DESTROY` revokes.
- `handle_destroy_msg` sends `WM_GPUI_CLOSE_ONE_WINDOW` instead of posting it,
  so the platform stops tracking the window while its handle is valid. The
  VSync thread redraws every tracked window and logged
  `ERROR_INVALID_WINDOW_HANDLE` for one destroyed but not yet untracked.
- `WindowsPlatform`'s `Drop` (`src/platform.rs`) stops the dispatcher from
  posting wake-ups before destroying the platform window: tasks woken while
  the app is dropped (after the platform) posted to the destroyed window and
  logged the same error.

To update: copy the new release from `~/.cargo/registry/src/*/` over this
folder (without its `Cargo.lock`) and apply these changes again.
