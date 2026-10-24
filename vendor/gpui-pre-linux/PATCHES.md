# gpui-pre-linux 0.3.8, patched

The crates.io release of `gpui-pre-linux` 0.3.8 (Zed's `gpui_linux`), used
through `[patch.crates-io]` in the workspace `Cargo.toml`. Changes, in
`src/linux/wayland/client.rs` and `src/linux/x11/client.rs`:

- `drop_window` releases the shared `GpuContext` (wgpu instance, adapter,
  device and queue) when the last window is gone. Upstream keeps it for the
  life of the process, so an app closed to the tray kept the Vulkan device,
  its driver threads (the NVIDIA driver wakes about a hundred times a second)
  and its memory. The next window creates a new context.
- `open_window` copies `compositor_gpu` instead of taking it, so that new
  context picks the GPU the compositor runs on, like the first one.
- `prime_vulkan_context` (`src/linux/platform.rs`, called by the Wayland and
  X11 windows before `WgpuRenderer::new`) creates the context from Vulkan
  alone. Upstream's renderer brings up Vulkan and OpenGL together at every
  start (EGL and every driver's GL stack: ~45 ms and their libraries) even
  though a Vulkan adapter wins whenever one works. Software Vulkan is
  rejected there, and when no hardware Vulkan adapter works the context is
  left empty, so the renderer builds it with both backends as upstream does.

To update: copy the new release from `~/.cargo/registry/src/*/` over this
folder (without its `Cargo.lock`) and apply these changes again.
