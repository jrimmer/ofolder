# Local patch to blade-graphics 0.7.1

This is blade-graphics 0.7.1 from crates.io (MIT, https://github.com/kvark/blade), unchanged
except in `src/vulkan/surface.rs`, `reconfigure_surface`: the two Vulkan queries on the surface
(its capabilities and its present modes) no longer `unwrap`. When either fails the function logs
a line to stderr (`ofolder: skipped a surface reconfiguration ...`, so it shows in the journal) and returns.

Why (a reading of gpui's source, not yet confirmed by a reproduction; that stderr line firing is the confirmation): gpui destroys a window's renderer when the window is dropped but removes the window from its
table later. A Wayland scale or size event that arrives in between reaches `reconfigure_surface`
with a destroyed surface; the driver reports an error, and the `unwrap` killed the process. In
ofolder that took down the rails (see the journal line `ERROR_EXTENSION_NOT_PRESENT`).

Remove this directory and the `[patch.crates-io]` entry in the workspace `Cargo.toml` once gpui
or blade carries a fix.
