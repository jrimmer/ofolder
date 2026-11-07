# Local patch to gpui 0.2.2

This is gpui 0.2.2 from crates.io (Apache-2.0, https://github.com/zed-industries/zed, the licence is
`LICENSE-APACHE` in this directory). Changes, as the licence asks us to state them:

- `src/platform/linux/wayland/window.rs`: `WaylandWindowState` gets a `renderer_destroyed` flag. `Drop for
  WaylandWindow` sets it right after it destroys the renderer. `set_size_and_scale`, `draw` and
  `update_window` return early once it is set.
- `examples/` and `tests/` are left out, with their `[[example]]` and `[[test]]` tables in `Cargo.toml`
  and the inner `Cargo.lock`, to keep the copy small. Nothing the library builds is changed by that.

## Why

gpui destroys a window's renderer when the window is dropped, but the window stays registered with the
Wayland client until a task spawned by that `Drop` runs. A size or scale event that arrives in that gap
reaches the destroyed renderer. In ofolder every tooltip and menu is a window that is created and dropped,
so quick clicks on the rails hit it. Seen on the real desktop (Intel Vulkan):

- a panic in `blade-graphics` `reconfigure_surface` (`ERROR_EXTENSION_NOT_PRESENT`), and, with that call
  skipped, an abort in `Context::destroy_texture` from `BladeRenderer::update_drawable_size`, reached by
  `WaylandWindowStatePtr::set_size_and_scale` from `handle_xdg_surface_event`.

## Upstream

Read on 2026-11-07: current Zed (`crates/gpui_linux/src/linux/wayland/window.rs`) holds the renderer in an
`Option`, takes it out in `Drop`, and parks the frame loop first, and it has replaced blade with wgpu. So
the bug is already fixed there and there is nothing to send upstream. gpui 0.2.2 is the newest release on
crates.io (checked 2026-11-07).

## Remove this directory

and the `[patch.crates-io]` entry in the workspace `Cargo.toml` when ofolder moves to a gpui that holds the
renderer in an `Option` (a release after 0.2.2, or a pinned Zed commit).
