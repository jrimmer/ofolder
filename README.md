# ofolder

Chrome for the save and open dialogs on Omarchy: a strip docked beside the
dialog with favorites, "new folder" and go-to-folder. It decorates Flea, the GTK portal dialog and Strata without changing
any of them.

Wayland cannot embed another process's window, and none of these dialogs has a
plugin API, so ofolder reads and drives each dialog from outside through an
adapter that says honestly what it can do.

- `docs/adapters.md`: what was measured about each dialog, and the hazards.
- `crates/core`: the adapter contract and the pure, tested logic (no UI, no D-Bus).
- `crates/strip`: the gpui strip.
- `crates/app`: the `ofolder` command.
- `spike/`: the throwaway Python spike behind the findings, with its evidence in `spike/runs/`.

```
bin/check              # fmt, clippy -D warnings, tests, architecture guard
cargo run -p ofolder -- adapters     # which installed file choosers can be decorated
cargo run -p ofolder -- strip-demo   # open a sample strip window
```

Status: the Flea adapter and the portal proxy are ported to Rust and proven against
the real Flea and the real GTK dialog (see `docs/adapters.md`). The GTK and Strata
adapters and the strip wiring are still the Python spike.
